//! Replay raw fuzz regression inputs from `tests/fuzz_regressions/`.
//! Invalid UTF-8 is skipped, matching the fuzz target input contract.

use std::fs;
use std::panic;
use std::path::{Path, PathBuf};
use std::str;

use ab_aozora_pipeline::{ALL_SENTINELS, lex};
use ab_aozora_render::render_html;
use ab_aozora_render::serialize;
#[test]
fn render_html_regressions_replay_cleanly() {
    replay_each("render_html", |src| {
        // Apply the fuzz target's PUA-sentinel skip: source-supplied
        // U+E001..U+E004 is exempt from the check that no sentinel
        // appears in the HTML.
        if src.chars().any(|c| ALL_SENTINELS.contains(&c)) {
            return;
        }
        let lex_out = lex(src);
        let html = render_html(&lex_out);
        for sentinel in ALL_SENTINELS {
            assert!(
                !html.contains(sentinel),
                "PUA sentinel {sentinel:?} leaked into rendered HTML\n  html = {html:?}",
            );
        }
    });
}

#[test]
fn serialize_round_trip_regressions_replay_cleanly() {
    replay_each("serialize_round_trip", |src| {
        // Apply the fuzz target's PUA-sentinel skip: sources containing
        // U+E001..U+E004 are out of contract for I3 (the lexer
        // reads them as reserved markers).
        if src.chars().any(|c| ALL_SENTINELS.contains(&c)) {
            return;
        }
        let lex1 = lex(src);
        let first = serialize(&lex1);
        let lex2 = lex(&first);
        let second = serialize(&lex2);
        assert!(
            first == second,
            "I3 fixed-point broken\n  first  = {first:?}\n  second = {second:?}",
        );
    });
}

/// Pass the decoded UTF-8 source of every artifact under
/// `tests/fuzz_regressions/<target>/` to `assert_one`. Panics when the
/// directory is missing or holds no artifact.
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
