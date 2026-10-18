//! Corpus check of incremental re-parse equivalence.
//!
//! For every document in `AOZORA_CORPUS_ROOT`, the diagnostics-only incremental
//! engine [`ab_aozora_facade::reparse_incremental_diagnostics_only`], which the
//! LSP calls on each keystroke, must splice diagnostics byte-for-byte equal to a
//! from-scratch parse of the edited text. One deterministic plain-character
//! insertion near each document's midpoint exercises the splice fast path on
//! global-free documents and the full-parse fallback elsewhere; the test asserts
//! a non-zero fast-path count. An edit the splice cannot prove byte-identical
//! returns `None` and falls back to a full parse.
//!
//! The test is skipped when `AOZORA_CORPUS_ROOT` is unset and does not fail on a
//! missing corpus, as in `corpus_sweep`.

use ab_aozora_encoding::decode_auto;
use ab_aozora_facade::{
    DiagBaseRef, Diagnostic, Document, PieceSeq, reparse_incremental_diagnostics_only,
};

/// The harness takes each document's sanitized fixed-point buffer as the cached
/// baseline, applies one midpoint plain insertion, and reports all divergences
/// together. Documents whose midpoint insertion is not a sanitize fixed point
/// are skipped.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one corpus differential harness: per-doc baseline + diagnostics-only splice, all over the same loop"
)]
fn reparse_diagnostics_only_equals_full_parse() {
    let Some(source) = ab_aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set; skipping diagnostics-only incremental gate");
        return;
    };

    let mut count: usize = 0;
    let mut fast_path: usize = 0;
    let mut fallback: usize = 0;
    let mut diverged: Vec<String> = Vec::new();

    for item in source.iter() {
        let item = item.expect("corpus iteration must not error");
        let Ok(text) = decode_auto(&item.bytes) else {
            continue;
        };
        let text = text.as_ref();
        if text.is_empty() {
            continue;
        }

        // The incremental engine operates in sanitized space, so `cached` is the
        // parse of sanitized text, not of the raw corpus bytes. A parse of the
        // raw text would leave sanitize-stage diagnostics
        // (`AccentDecompositionApplied`) in `cached` that a re-parse of the
        // sanitized buffer does not reproduce.
        let san = Document::new(text).lex().sanitized;
        if san.is_empty() {
            continue;
        }
        let cached = Document::new(san.as_str()).lex();
        // The incremental contract requires a sanitized baseline that is a
        // sanitize fixed point; for other documents production runs a full parse.
        if cached.sanitized != san {
            continue;
        }

        let mut mid = san.len() / 2;
        while mid < san.len() && !san.is_char_boundary(mid) {
            mid += 1;
        }
        let new_san = format!("{}x{}", &san[..mid], &san[mid..]);

        // Production runs a full parse for an edit whose result is not a
        // sanitize fixed point, so such edits are skipped.
        let full = Document::new(new_san.as_str()).lex();
        if full.sanitized != new_san {
            continue;
        }

        let pieces = PieceSeq::from_contiguous(
            &cached.source_nodes,
            &cached.pairs,
            &cached.diagnostics,
            cached.sanitized_len,
        );
        let Some(diag) = reparse_incremental_diagnostics_only(
            DiagBaseRef::from_cached(&cached, &pieces),
            &new_san,
            mid..mid,
        ) else {
            fallback += 1;
            count += 1;
            continue;
        };
        fast_path += 1;
        count += 1;
        // The diagnostics-only splice returns a `PieceSeq`; the comparison uses
        // its flattened diagnostics.
        let diag_diags = diag.pieces.collect_diagnostics();

        // Diagnostics byte-identical to a full parse (the production contract).
        if sorted_debug(diag_diags) != sorted_debug(full.diagnostics.clone()) {
            diverged.push(format!(
                "{}: diagnostics-only multiset != full parse",
                item.label
            ));
        }
    }

    eprintln!(
        "diagnostics-only incremental gate: {count} docs edited, {fast_path} fast-path, \
         {fallback} fallback, {} diverged",
        diverged.len(),
    );
    assert!(
        diverged.is_empty(),
        "{} document(s) where the diagnostics-only splice diverged:\n  {}",
        diverged.len(),
        diverged.join("\n  "),
    );
    assert!(
        fast_path > 0,
        "the diagnostics-only gate must exercise the fast path at least once \
         (got {fast_path}); a perpetual fallback proves nothing",
    );
}

/// Returns the debug strings of `diags` sorted by span, then by debug string, so
/// that two diagnostic multisets compare equal regardless of order.
fn sorted_debug(mut diags: Vec<Diagnostic>) -> Vec<String> {
    diags.sort_by(|a, b| {
        let (sa, sb) = (a.span(), b.span());
        (sa.start, sa.end)
            .cmp(&(sb.start, sb.end))
            .then_with(|| format!("{a:?}").cmp(&format!("{b:?}")))
    });
    diags.iter().map(|d| format!("{d:?}")).collect()
}
