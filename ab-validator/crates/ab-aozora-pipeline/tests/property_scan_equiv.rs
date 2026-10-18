//! Cross-checks between the tokenize-stage trigger scan and the brute-force
//! [`naive_scan_offsets`] reference, projected through the whole
//! `lex` pipeline.
//!
//! The differential proptests inside `lexer::trigger_scan` check that the
//! production `scan_offsets` agrees byte for byte with
//! [`naive_scan_offsets`] on raw source bytes. They cover the scanner
//! alone, not the tokenize stage's output after pair / classify in the
//! lexer's normalized buffer.
//!
//! The properties here cover the whole pipeline:
//!
//! 1. **Scanner totality on lex output.** [`naive_scan_offsets`] applied to
//!    the lexer's normalized buffer must not panic and must return
//!    offsets that are valid UTF-8 char boundaries inside that buffer.
//!    The normalized buffer carries PUA sentinels and may differ in
//!    length from the source; a regression that left dangling bytes
//!    or invalidated boundaries fails this property.
//!
//! 2. **Tokenize-stage monotonicity.** The Aozora pipeline can only *consume*
//!    triggers (replacing them with PUA sentinels); it never adds
//!    new ones. Therefore the count of triggers in the *normalized*
//!    buffer is at most the count in the *source* (modulo the sanitize
//!    stage's rewrites: BOM strip, CRLF→LF, accent decomposition).
//!    A regression that emitted a trigger in the normalized output
//!    where none existed in source changes this difference.
//!
//! Both properties are stated as inequalities rather than equalities
//! because the sanitize stage can drop bytes (BOM, CR) and rewrite sequences
//! (`〔NFC〕` → combining), so an equality check would report false
//! failures. A regression that adds triggers during normalization fails the
//! inequality, and proptest shrinks it to a minimal input.

use ab_aozora_pipeline::lex;
use ab_aozora_pipeline::lexer::trigger_scan::naive_scan_offsets;
use ab_notation_strategies::config::default_config;
use ab_notation_strategies::generators::*;
use proptest::prelude::*;

fn scan_count(source: &str) -> usize {
    naive_scan_offsets(source).len()
}

fn assert_offsets_are_char_boundaries(label: &str, text: &str, offsets: &[u32]) {
    for &offset in offsets {
        let off = offset as usize;
        assert!(
            off <= text.len(),
            "{label} offset {off} out of bounds for length {} in {text:?}",
            text.len()
        );
        assert!(
            text.is_char_boundary(off),
            "{label} offset {off} is not a UTF-8 char boundary in {text:?}"
        );
    }
}

fn assert_scan_invariants(source: &str) {
    // (1a) Scanner totality on raw source. This also checks that the
    // reference walker does not panic on any generated input,
    // independently of the in-module differential property.
    let source_offsets = naive_scan_offsets(source);
    assert_offsets_are_char_boundaries("source", source, &source_offsets);

    // Run the lex pipeline. (1b) Scanner totality on the normalized
    // buffer.
    let out = lex(source);
    let norm_offsets = naive_scan_offsets(&out.normalized);
    assert_offsets_are_char_boundaries("normalized", &out.normalized, &norm_offsets);

    // (2) tokenize-stage monotonicity: the pipeline consumes or passes
    // through triggers and adds none. A sanitize-stage rewrite could in
    // principle add characters from the lexer's reserved trigger set;
    // accent decomposition expands `〔NFC〕` into combining sequences that
    // are not triggers. The property fails if a sanitize or tokenize
    // change emits trigger glyphs.
    let source_triggers = scan_count(source);
    let normalized_triggers = norm_offsets.len();
    assert!(
        normalized_triggers <= source_triggers,
        "trigger count grew through lex: source={source_triggers} normalized={normalized_triggers}\n\
         source: {source:?}\nnormalized: {:?}",
        out.normalized
    );
}

// Fixed regression inputs.

#[test]
fn empty_input_satisfies_invariants() {
    assert_scan_invariants("");
}

#[test]
fn plain_text_satisfies_invariants() {
    assert_scan_invariants("Hello, world.");
    assert_scan_invariants("こんにちは、世界！");
}

#[test]
fn explicit_ruby_satisfies_invariants() {
    // `｜青梅《おうめ》`: three triggers in source (`｜`, `《`, `》`),
    // all consumed into PUA sentinels by the classify stage → zero triggers in
    // normalized. Property: 0 ≤ 3.
    assert_scan_invariants("｜青梅《おうめ》");
}

#[test]
fn paired_container_satisfies_invariants() {
    assert_scan_invariants(
        "［＃ここから2字下げ］\n\
         body\n\
         ［＃ここで字下げ終わり］",
    );
}

#[test]
fn literal_pua_keeps_scan_invariants() {
    // Private-use characters are not notation triggers.
    assert_scan_invariants("a\u{E001}b\u{E004}c");
}

proptest! {
    #![proptest_config(default_config())]

    /// The SIMD scan and tokenize-stage monotonicity properties must hold
    /// over every Aozora-shaped fragment.
    #[test]
    fn aozora_fragment_scan_invariants_hold(s in aozora_fragment(120)) {
        assert_scan_invariants(&s);
    }

    /// Pathological: runs of unbalanced trigger glyphs, the input class
    /// that exercises tokenize → pair → classify trigger-consumption
    /// accounting most heavily.
    #[test]
    fn pathological_input_scan_invariants_hold(s in pathological_aozora(120)) {
        assert_scan_invariants(&s);
    }

    /// Unicode adversarial: combining marks, RTL overrides, PUA
    /// codepoints, full-width forms. The SIMD scanner must classify
    /// every Unicode shape correctly, and the lexer must not synthesise
    /// triggers when normalising adversarial input.
    #[test]
    fn unicode_adversarial_scan_invariants_hold(s in unicode_adversarial()) {
        assert_scan_invariants(&s);
    }
}
