//! Invariants for `DirectiveKind::Unknown`: the catch-all that
//! the classify stage emits when no recogniser claimed an `［＃…］` annotation.
//!
//! The catch-all places every bracket annotation in the AST, so the
//! Tier-A canary (no bare `［＃` reaches HTML) holds end-to-end. The
//! property tests here pin the four invariants the catch-all must
//! satisfy:
//!
//! 1. **panic-free**: lex must not panic on any input that exercises
//!    the catch-all path, including pathological / unbalanced bracket
//!    shapes.
//! 2. **non-empty raw bytes**: every emitted `Directive` node carries
//!    a `NonEmptyStr` raw payload (the type enforces it; the runtime
//!    check fails if a change to the type permits empty annotations).
//! 3. **lex → serialise → lex round-trip preserves Directive count**:
//!    re-parsing the serialised output must produce the same number of
//!    Directive nodes (any kind, including Unknown). A regression that
//!    drops or duplicates an Unknown annotation on the second pass
//!    would shift this count.
//! 4. **Tier-A canary holds on well-formed input**: when the lexer
//!    emits no diagnostics for the source, the serialised text must
//!    not contain a bare `［＃` (i.e. one not paired with a closing
//!    `］`). On malformed inputs the serialiser reproduces the source's
//!    unbalanced brackets: Tier-A is an HTML-side contract, and the
//!    serialised text satisfies it only when the lexer emits no
//!    diagnostics.

use ab_aozora_pipeline::lex;
use ab_aozora_render::serialize;
use ab_aozora_syntax::ast::{Node, NodeRef};
use ab_notation_strategies::config::default_config;
use ab_notation_strategies::generators::*;
use proptest::prelude::*;

/// Count the `Directive` nodes in the registry.
///
/// Annotations are emitted on `Sentinel::Inline` positions, so they
/// surface as `NodeRef::Inline(Node::Directive(_))`. Container
/// open/close hits never carry annotations; block-leaf hits in the
/// owned AST are reserved for non-annotation block primitives
/// (page break, section break, sashie, …).
fn count_annotations(out: &ab_aozora_pipeline::LexOutput) -> usize {
    out.registry
        .iter_sorted()
        .filter(|(_, nr)| matches!(nr, NodeRef::Inline(Node::Directive(_))))
        .count()
}

/// Whether the registry contains at least one annotation. Guards the
/// properties that apply only when the input reached the catch-all path.
fn has_annotation(out: &ab_aozora_pipeline::LexOutput) -> bool {
    out.registry
        .iter_sorted()
        .any(|(_, nr)| matches!(nr, NodeRef::Inline(Node::Directive(_))))
}

/// Tier-A canary on a string: no bare `［＃` may appear without a
/// corresponding `］` after it on the same line. The check is
/// approximate; it detects a serialiser that drops a closing bracket,
/// and a bare `［＃` in the output is a Tier-A violation in any context.
fn assert_no_bare_open_bracket(serialised: &str) {
    let mut cursor = 0_usize;
    while let Some(rel) = serialised[cursor..].find("［＃") {
        let open_at = cursor + rel;
        let after = &serialised[open_at + "［＃".len()..];
        assert!(
            after.contains('］'),
            "bare `［＃` at byte {open_at} has no following `］` in serialised output: {serialised:?}"
        );
        cursor = open_at + "［＃".len();
    }
}

fn assert_annotation_invariants(source: &str) {
    let out_a = lex(source);

    // (2) Every annotation's raw payload is non-empty. `NonEmptyStr`
    // enforces this; the runtime check fails if a change to the type
    // weakens the invariant. The raw bytes are valid UTF-8 by
    // construction (`&str`).
    for (_, nr) in out_a.registry.iter_sorted() {
        if let NodeRef::Inline(Node::Directive(a)) = nr {
            assert!(
                !out_a.store.resolve_str(a.raw).is_empty(),
                "annotation with empty raw bytes for source {source:?}"
            );
        }
    }

    // (3) lex → serialise → lex preserves the annotation count.
    let serialised = serialize(&out_a);
    let out_b = lex(&serialised);
    let count_first = count_annotations(&out_a);
    let count_second = count_annotations(&out_b);
    assert_eq!(
        count_first, count_second,
        "annotation count drifted on round-trip: first={count_first} second={count_second}\n\
         source: {source:?}\nserialised: {serialised:?}"
    );

    // (4) Tier-A canary on serialised text: no bare `［＃` appears when
    // the lexer emits no diagnostics. On malformed inputs (those that
    // emit diagnostics), the serialiser reproduces the source's
    // unbalanced brackets; Tier-A is an HTML-side contract and does not
    // apply to the serialise path on malformed source.
    if out_a.diagnostics.is_empty() {
        assert_no_bare_open_bracket(&serialised);
    }

    // (1) panic-free is implicit: reaching this point means lex did not
    // panic. The check below runs only for inputs that reached the
    // catch-all.
    if has_annotation(&out_a) {
        // Stability check: re-lexing the annotated output must still observe
        // at least one annotation (catch-all is stable across passes).
        assert!(
            has_annotation(&out_b),
            "annotation present in first pass but vanished in second for source {source:?}\n\
             serialised: {serialised:?}"
        );
    }
}

// Fixed regression inputs.

#[test]
fn empty_input_has_no_annotations() {
    assert_annotation_invariants("");
}

#[test]
fn known_annotations_round_trip() {
    assert_annotation_invariants("text［＃改ページ］more");
    assert_annotation_invariants("［＃ここから2字下げ］\nbody\n［＃ここで字下げ終わり］");
}

#[test]
fn unknown_annotations_round_trip() {
    // Absent from the recogniser table: falls through to
    // `DirectiveKind::Unknown`.
    assert_annotation_invariants("text［＃this is a wholly novel marker］more");
    assert_annotation_invariants("［＃random text 1234］");
}

#[test]
fn quoted_annotations_round_trip() {
    assert_annotation_invariants("text［＃「青空」に傍点］more");
    assert_annotation_invariants("text［＃「これは未知の指示」］more");
}

#[test]
fn directive_before_repeated_explicit_ruby_markers_round_trips() {
    let source = "｜｜［＃改］｜ABC《≫》》";
    assert_annotation_invariants(source);
    assert_eq!(serialize(&lex(source)), source);
}

proptest! {
    #![proptest_config(default_config())]

    /// Aozora-shaped fragments must satisfy all four invariants for every
    /// annotation kind they contain.
    #[test]
    fn aozora_fragment_annotations_are_well_formed(s in aozora_fragment(120)) {
        assert_annotation_invariants(&s);
    }

    /// Pathological: unbalanced and unterminated bracket shapes, which
    /// exercise the catch-all most heavily. If the catch-all panics or
    /// drops annotations on the round-trip, this property fails and
    /// proptest shrinks the input to a minimal case.
    #[test]
    fn pathological_input_annotations_are_well_formed(s in pathological_aozora(120)) {
        assert_annotation_invariants(&s);
    }

    /// Unicode adversarial: combining marks, RTL overrides, PUA
    /// codepoints, full-width bracket variants. Checks the four
    /// invariants when the annotation body contains adversarial Unicode.
    #[test]
    fn unicode_adversarial_annotations_are_well_formed(s in unicode_adversarial()) {
        assert_annotation_invariants(&s);
    }
}

/// An unpaired `［` before a block directive must not suppress a later
/// directive. The newline expiry leaves a directive with newline allowance
/// on the stack: reclaiming the stray bracket would pop the live multi-line
/// directive above it as well, because a stack pops only from the top. The
/// serializer pads the block directive with blank lines, which separates the
/// stray bracket from what follows, so a suppressed directive would reappear
/// in the second parse and the directive counts would differ. Both parses
/// agree with the parse of the same text without the stray bracket.
#[test]
fn unpaired_bracket_before_a_block_directive_round_trips() {
    assert_annotation_invariants("［［＃ここから字下げ］［＃\n］");
}

#[test]
fn opaque_multiline_directive_cannot_extend_a_later_ruby_base() {
    assert_annotation_invariants("｜［＃\n］［＃］｜改丁《］》");
}
