//! Replay raw fuzz regression inputs from `tests/fuzz_regressions/`.
//! Invalid UTF-8 is skipped, matching the fuzz target input contract.

use std::fs;
use std::panic;
use std::path::{Path, PathBuf};
use std::str;

use ab_aozora_pipeline::lex;
#[test]
fn lex_into_arena_regressions_replay_cleanly() {
    replay_each("lex_into_arena", |src| {
        let out = lex(src);
        for diag in &out.diagnostics {
            let span = diag.span();
            assert!(
                span.start <= span.end,
                "diagnostic span {span:?} has start > end",
            );
        }
    });
}

/// Pass the decoded UTF-8 source of every artifact under
/// `tests/fuzz_regressions/<target>/` to `assert_one`. A panic from the
/// closure is caught and re-raised with the artifact path as prefix.
/// Panics when the directory is missing or holds no artifact.
fn replay_each(target: &str, assert_one: impl Fn(&str)) {
    let dir = regression_dir(target);
    let artifacts = collect_artifacts(&dir);
    assert!(
        !artifacts.is_empty(),
        "no regression artifacts under {}",
        dir.display(),
    );
    for path in artifacts {
        let path_display = path.display();
        let bytes = fs::read(&path)
            .unwrap_or_else(|e| panic!("failed to read regression artifact {path_display}: {e}"));
        let Ok(src) = str::from_utf8(&bytes) else {
            // The fuzzer supplies raw bytes and the lex target returns
            // early on invalid UTF-8; the replay skips the same inputs.
            continue;
        };
        let label = path.display().to_string();
        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| assert_one(src)));
        if let Err(payload) = result {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| {
                    payload
                        .downcast_ref::<&'static str>()
                        .map(|s| (*s).to_owned())
                })
                .unwrap_or_else(|| "<non-string panic payload>".to_owned());
            panic!("regression artifact {label} still crashes:\n{message}\n  bytes = {bytes:?}");
        }
    }
}

fn regression_dir(target: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fuzz_regressions")
        .join(target)
}

fn collect_artifacts(dir: &Path) -> Vec<PathBuf> {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("failed to read regression directory {}: {e}", dir.display()));
    let mut out: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_none_or(|ext| ext != "txt" && ext != "md")
        })
        .collect();
    out.sort();
    out
}
