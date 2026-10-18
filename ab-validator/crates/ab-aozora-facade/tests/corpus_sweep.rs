//! Walks `AOZORA_CORPUS_ROOT` and checks that every document parses and
//! round-trips through `parse ∘ serialize` (the I3 fixed-point invariant).
//!
//! The test is skipped when `AOZORA_CORPUS_ROOT` is unset and does not fail on a
//! missing corpus.
//!
//! The shrink-only allowlist `corpus/roundtrip-allowlist.json` lists the
//! documents whose canonical serialization is not a fixed point. The test fails
//! if an unlisted document diverges or a listed document round-trips; in the
//! second case the entry must be removed.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fs::read_to_string;

use ab_aozora_encoding::decode_auto;
use ab_aozora_facade::Document;

#[test]
fn corpus_round_trip_is_a_fixed_point() {
    let Some(source) = ab_aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set; skipping corpus sweep");
        return;
    };

    let allow = load_allowlist();

    let mut count: usize = 0;
    let mut sjis_decoded: usize = 0;
    // All divergences are collected, so one run reports the complete set for
    // comparison with the allowlist.
    let mut diverged: Vec<(String, String)> = Vec::new();
    let mut diverged_labels: BTreeSet<String> = BTreeSet::new();

    for item in source.iter() {
        let item = item.expect("corpus iteration must not error");

        let Ok(utf8) = decode_auto(&item.bytes) else {
            eprintln!("skip (neither UTF-8 nor Shift_JIS): {}", item.label);
            continue;
        };
        if matches!(utf8, Cow::Owned(_)) {
            sjis_decoded += 1;
        }

        let doc = Document::new(utf8);
        let serialized = doc.parse().to_source();

        let doc2 = Document::new(serialized.clone());
        let serialized2 = doc2.parse().to_source();

        if serialized != serialized2 {
            diverged.push((
                item.label.clone(),
                first_diff_hint(&serialized, &serialized2),
            ));
            diverged_labels.insert(item.label.clone());
        }

        count += 1;
    }

    eprintln!(
        "corpus sweep: {count} docs walked ({sjis_decoded} decoded from Shift_JIS, the rest already UTF-8)"
    );

    // Unlisted divergences fail the test.
    let mut unexpected: Vec<&(String, String)> = diverged
        .iter()
        .filter(|(label, _)| !allow.contains(label))
        .collect();
    unexpected.sort_by(|a, b| a.0.cmp(&b.0));

    // A listed document that round-trips is a stale entry, reported for removal
    // because the list is shrink-only.
    let stale: Vec<&String> = allow
        .iter()
        .filter(|label| !diverged_labels.contains(*label))
        .collect();

    let mut problems: Vec<String> = Vec::new();
    if !unexpected.is_empty() {
        let list = unexpected
            .iter()
            .map(|(l, h)| format!("{l}{h}"))
            .collect::<Vec<_>>()
            .join("\n  ");
        problems.push(format!(
            "{} un-allowlisted round-trip divergence(s):\n  {list}",
            unexpected.len(),
        ));
    }
    if !stale.is_empty() {
        let list = stale
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  ");
        problems.push(format!(
            "{} allowlist entr(y/ies) now round-trip — remove from corpus/roundtrip-allowlist.json:\n  {list}",
            stale.len(),
        ));
    }

    assert!(problems.is_empty(), "\n{}", problems.join("\n"));

    eprintln!(
        "corpus sweep: round-trip fixed point holds ({} allowlisted divergence(s) tracked)",
        allow.len()
    );
}

/// Returns the labels of documents permitted to diverge, from the shrink-only
/// allowlist. A missing or empty file yields an empty set, so every document
/// must round-trip.
fn load_allowlist() -> BTreeSet<String> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../corpus/roundtrip-allowlist.json"
    );
    let Ok(text) = read_to_string(path) else {
        return BTreeSet::new();
    };
    // A hand parse, which avoids a serde dependency in this test, collects every
    // JSON string inside the `"allow"` array.
    let Some(start) = text.find("\"allow\"") else {
        return BTreeSet::new();
    };
    let Some(open) = text[start..].find('[') else {
        return BTreeSet::new();
    };
    let body_start = start + open + 1;
    let Some(close_rel) = text[body_start..].find(']') else {
        return BTreeSet::new();
    };
    let body = &text[body_start..body_start + close_rel];
    body.split('"')
        .enumerate()
        // Odd-indexed splits are the quoted contents.
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, s)| s.to_owned())
        .collect()
}

/// Returns a `…before│after…` window around the first byte at which two
/// serializations differ, which identifies the diverging construct.
fn first_diff_hint(a: &str, b: &str) -> String {
    let at = a
        .bytes()
        .zip(b.bytes())
        .position(|(x, y)| x != y)
        .unwrap_or_else(|| a.len().min(b.len()));
    let lo = floor_boundary(a, at.saturating_sub(24));
    let a_hi = ceil_boundary(a, (at + 24).min(a.len()));
    let b_hi = ceil_boundary(b, (at + 24).min(b.len()));
    format!(
        "  @{at}: …{}│ vs │{}…",
        &a[lo..a_hi],
        &b[floor_boundary(b, lo)..b_hi]
    )
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_boundary(s: &str, mut i: usize) -> usize {
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}
