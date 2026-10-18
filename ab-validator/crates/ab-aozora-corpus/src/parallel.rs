//! Parallel I/O + decode helpers.
//!
//! The default [`crate::CorpusSource::iter`] impl on [`crate::FilesystemCorpus`]
//! returns a streaming iterator
//! that combines walkdir and the read; this module provides a
//! path-first variant that separates the two, so the reads are
//! distributed across rayon workers while walkdir runs once on the
//! calling thread.
//!
//! ## Physical-core thread pool
//!
//! [`par_load_decoded`] runs its parallel pass on a dedicated rayon
//! pool sized to physical cores (`num_cpus::get_physical()`) rather
//! than rayon's default logical-core (hyperthreaded) count. On an
//! 8-core, 8-hyperthread WSL2 host, a load of this corpus took 0.92 s
//! with 8 threads and 1.38 s with 16 threads: the decode work is
//! memory-bound, and two SMT siblings on one core contend for the same
//! L1/L2 cache and per-core memory bandwidth. On systems without SMT,
//! the pool size equals rayon's default.
//!
//! ## Paths are collected before the parallel read
//!
//! `walkdir::WalkDir` is not parallel-safe: it owns the kernel `DIR*`
//! cursor, and its iterator state is not `Sync`. Splitting the
//! traversal across workers would require either a per-worker walker
//! over disjoint subtrees, which needs a sharding rule the corpus
//! layout does not supply, or a custom parallel-aware walker. The
//! walkdir phase takes ~0.3 s on the 17 k-file corpus, well under any
//! rayon coordination overhead, so the paths are collected
//! sequentially and rayon's `par_iter` distributes the per-file read
//! and caller closure.
//!
//! ## Per-item closure (`F: Fn(CorpusItem) -> T`)
//!
//! Callers apply different work to each decoded item: bench harnesses
//! bucket by post-decode size, sweep tests run invariants, cache
//! builders write derived files. The closure runs inside the parallel
//! pass, on the rayon worker that read the item, so it can use
//! per-worker state such as a [`thread_local!`] decode buffer. A
//! returned `Vec<CorpusItem>` would require a second pass over the
//! vector outside the pool, with a cold buffer on each call.

use std::path::PathBuf;
use std::sync::OnceLock;

use rayon::prelude::*;
use rayon::{ThreadPool, ThreadPoolBuilder};

use crate::{CorpusError, CorpusItem, FilesystemCorpus};

/// Process-wide rayon pool sized to physical cores. Lazily constructed
/// the first time it is requested; subsequent calls reuse the same
/// pool, so its worker threads persist across corpus sweeps.
/// `OnceLock` is the standard-library lazy-init primitive and requires
/// no `unsafe` in this crate.
fn physical_core_pool() -> &'static ThreadPool {
    static POOL: OnceLock<ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        let n = num_cpus::get_physical().max(1);
        ThreadPoolBuilder::new()
            .num_threads(n)
            .thread_name(|i| format!("aozora-corpus-load-{i}"))
            .build()
            .expect("rayon pool construction must not fail under reasonable settings")
    })
}

/// Run `f` on the physical-core load pool.
///
/// A caller uses this to run on the pool that [`par_load_decoded`] uses,
/// so the decode-buffer thread-locals persist across consecutive corpus
/// sweeps. Rayon work started outside this function runs on a second
/// pool of the default (logical-core) size, which over-subscribes SMT
/// cores as described in the module documentation.
///
/// `f` runs synchronously; the function returns when `f` returns.
pub fn with_load_pool<R, F>(f: F) -> R
where
    F: FnOnce() -> R + Send,
    R: Send,
{
    physical_core_pool().install(f)
}

/// Walk the corpus, read every file, and apply `per_item` to each
/// `CorpusItem` in parallel via rayon.
///
/// The walkdir traversal runs once on the calling thread (it is not
/// `Sync`); the read + closure runs on rayon's work-stealing pool.
/// Per-item I/O failures are `Err` entries in the returned `Vec`; as
/// with the sequential `iter()` path, the caller decides whether to
/// skip or fail.
///
/// Output order is rayon's collect order (matches the input
/// `Vec<PathBuf>` order, which is walkdir's directory-entry order:
/// defined by `readdir(2)`, OS- and filesystem-dependent, not
/// lexicographic). Per-thread accumulator merging happens inside
/// rayon's parallel-iterator collect — the caller does not see
/// intermediate per-thread shards. If the caller needs deterministic
/// output ordering, sort the returned `Vec` by `CorpusItem.label`
/// before consuming it.
///
/// # Errors
///
/// Per-item errors are returned inline as `Err` entries; this function
/// itself never returns `Err`. Walkdir-level errors (permission denied
/// on a subdirectory, broken symlink) likewise surface as `Err`
/// entries in their walkdir position.
pub fn par_load_decoded<F, T>(corpus: &FilesystemCorpus, per_item: F) -> Vec<Result<T, CorpusError>>
where
    F: Fn(CorpusItem) -> T + Sync + Send,
    T: Send,
{
    // The walk runs serially because walkdir is `!Sync`; it takes
    // ~0.3 s on 17 k files because `is_text_dir_entry` issues no
    // extra `stat`.
    let paths: Vec<Result<PathBuf, CorpusError>> = corpus.walk_paths().collect();

    physical_core_pool().install(|| {
        paths
            .into_par_iter()
            .map(|path_result| {
                let path = path_result?;
                let item = corpus.read_path(&path)?;
                Ok(per_item(item))
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn fresh_root_with(files: &[(&str, &[u8])]) -> TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        for (rel, bytes) in files {
            let path = dir.path().join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("mkdir");
            }
            fs::write(&path, bytes).expect("write");
        }
        dir
    }

    #[test]
    fn par_load_decoded_visits_every_file() {
        let dir = fresh_root_with(&[
            ("a.txt", b"alpha"),
            ("b.txt", b"beta"),
            ("c.txt", b"gamma"),
            ("nested/d.txt", b"delta"),
        ]);
        let corpus = FilesystemCorpus::new(dir.path()).expect("corpus");

        let mut results: Vec<usize> = par_load_decoded(&corpus, |item| item.bytes.len())
            .into_iter()
            .filter_map(Result::ok)
            .collect();
        results.sort_unstable();
        // 5, 4, 5, 5 sorted = 4, 5, 5, 5
        assert_eq!(results, vec![4, 5, 5, 5]);
    }

    #[test]
    fn par_load_decoded_returns_empty_for_empty_corpus() {
        let dir = tempfile::tempdir().expect("tempdir");
        let corpus = FilesystemCorpus::new(dir.path()).expect("corpus");
        let results = par_load_decoded(&corpus, |item| item.bytes.len());
        assert!(results.is_empty());
    }

    #[test]
    fn par_load_decoded_skips_non_txt_files() {
        let dir = fresh_root_with(&[
            ("keep.txt", b"yes"),
            ("skip.md", b"no"),
            ("skip.bin", &[0xFF, 0xFE]),
        ]);
        let corpus = FilesystemCorpus::new(dir.path()).expect("corpus");
        let results = par_load_decoded(&corpus, |item| (item.label, item.bytes.len()));
        let kept: Vec<_> = results.into_iter().filter_map(Result::ok).collect();
        assert_eq!(kept, vec![(String::from("keep.txt"), 3)]);
    }

    #[test]
    fn par_load_decoded_visits_every_file_at_scale() {
        // Fifty files, so the reads span several rayon workers. The
        // assertion compares label sets after the parallel collect,
        // not sequences, because walkdir's directory-entry order is
        // OS-dependent.
        let files: Vec<(String, Vec<u8>)> = (0..50)
            .map(|i| (format!("doc-{i:03}.txt"), format!("body-{i}").into_bytes()))
            .collect();
        let file_refs: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(p, b)| (p.as_str(), b.as_slice()))
            .collect();
        let dir = fresh_root_with(&file_refs);
        let corpus = FilesystemCorpus::new(dir.path()).expect("corpus");

        let mut labels: Vec<String> = par_load_decoded(&corpus, |item| item.label)
            .into_iter()
            .filter_map(Result::ok)
            .collect();
        labels.sort();
        let mut expected: Vec<String> = files.iter().map(|(p, _)| p.clone()).collect();
        expected.sort();
        assert_eq!(labels, expected);
    }
}
