//! Diagnostic ordering invariants for the fused lex pipeline.
//!
//! Pins the order documented at [`Pipeline::build`]:
//!
//!   sanitize → pair → classify
//!
//! Downstream tooling (IDE diagnostics, the CLI's miette renderer,
//! property tests that grep for diagnostic positions) depends on a stable
//! order; a pipeline change that reorders diagnostic emission fails here.
//!
//! The `Diagnostic` enum (`crates/ab-aozora-spec/src/diagnostic.rs`)
//! has no classify-stage variant for "unrecognised annotation keyword"; an
//! unknown body is folded into `DirectiveKind::Unknown` without a
//! diagnostic. The tests therefore pin the sanitize ↔ pair ordering only.
//! The insta snapshot records the complete multi-diagnostic output.

use ab_aozora_pipeline::lex;
use ab_aozora_spec::{Diagnostic, DiagnosticSource, codes};
/// Ordinal position of a diagnostic in the documented pipeline order
/// (sanitize → pair → classify → "later").
///
/// The `Registry*`, `Unregistered*` and `ResidualAnnotationMarker`
/// diagnostics are [`Diagnostic::Internal`] values with a stable `code`
/// payload; they come from post-classify validators and sort last.
fn phase_ordinal(d: &Diagnostic) -> u8 {
    match d.source() {
        // Source-side diagnostics: match by stable code.
        DiagnosticSource::Source => match d.code() {
            // sanitize stage.
            codes::ACCENT_DECOMPOSITION_APPLIED => 0,
            // pair stage.
            codes::UNCLOSED_BRACKET | codes::UNMATCHED_CLOSE => 2,
            // classify stage.
            codes::UNRESOLVED_GAIJI
            | codes::EMPTY_RUBY_READING
            | codes::NESTED_RUBY
            | codes::UNRECOGNISED_CONTAINER_DIRECTIVE
            | codes::TCY_TARGET_NOT_FOUND
            | codes::BOUTEN_TARGET_AMBIGUOUS
            | codes::BRACKETED_KAERITEN_NO_PAIR
            | codes::KAERITEN_OUTSIDE_KANBUN => 3,
            // Post-classify normalizer fold (same slot as the Internal
            // validators below, which also run after classify).
            codes::MISMATCHED_CONTAINER_CLOSE
            | codes::BREAK_IN_SINGLE_LINE_CONTAINER
            | codes::MISMATCHED_BOUTEN_CONTAINER => 4,
            _ => 99,
        },
        // Pipeline-internal validators run after the classify stage.
        DiagnosticSource::Internal => 4,
        // `DiagnosticSource` is `#[non_exhaustive]`. A category added
        // later maps here until it is classified explicitly.
        _ => 99,
    }
}

#[test]
fn phase0_then_phase2_diagnostics_are_emitted_in_pipeline_order() {
    // Accent normalization precedes unmatched-delimiter reporting.
    let src = "〔cafe'〕［＃unclosed";
    let out = lex(src);

    let ordinals: Vec<u8> = out.diagnostics.iter().map(phase_ordinal).collect();

    // Must contain at least one sanitize-stage and one pair-stage diagnostic.
    assert!(
        ordinals.contains(&0),
        "expected a sanitize-stage diagnostic in {:?}",
        out.diagnostics
    );
    assert!(
        ordinals.contains(&2),
        "expected a pair-stage diagnostic in {:?}",
        out.diagnostics
    );

    // Ordinals must be monotonically non-decreasing (sanitize-stage entries
    // come first, then pair stage, then classify stage onward if any).
    let mut sorted = ordinals.clone();
    sorted.sort_unstable();
    assert_eq!(
        ordinals, sorted,
        "diagnostics must come back in pipeline order, got ordinals={ordinals:?} for {:?}",
        out.diagnostics
    );
}

/// Insta snapshot of the diagnostic vector for a hand-curated
/// multi-diagnostic input. Records the variants, kinds and span payloads
/// byte for byte; a reorder, a payload change, or a missing or extra
/// diagnostic produces a snapshot diff.
///
/// The input combines:
///   * sanitize-stage accent decomposition at position 0.
///   * pair-stage unmatched close (`］` in mid-text without an open).
///   * pair-stage unclosed bracket (`［＃...` at end of input).
#[test]
fn multi_diagnostic_snapshot_freezes_pipeline_order() {
    let src = "〔cafe'〕stray］then［＃tail";
    let out = lex(src);
    insta::assert_snapshot!(format!("{:#?}", out.diagnostics));
}
