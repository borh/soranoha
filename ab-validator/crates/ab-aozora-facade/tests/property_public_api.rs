//! Totality and idempotence of [`Document::parse`] through the public API.
//!
//! 1. **Totality**: [`Document::parse`] does not panic on any input. The lex
//!    pipeline emits diagnostics rather than failing, and the facade is the
//!    only entry point for the FFI, WASM and Python drivers, which pass user
//!    input directly to the parser; a panic is a denial-of-service condition
//!    for every downstream binding.
//!
//! 2. **Parse → serialise → parse converges**: the second `Document::parse`
//!    over the serialised output of the first yields a tree whose
//!    `serialize()` output matches, that is, the renderer's `serialize` is a
//!    parser fixed point. A failure indicates a round-trip mutation such as
//!    whitespace drift or character substitution. The `ab-aozora-render`
//!    `property_serialize_idempotent` test checks the same property; this test
//!    runs through the public API only, which covers facade defects such as a
//!    `Document` that builds an inconsistent arena.

use ab_aozora_facade::Document;
use ab_notation_strategies::config::default_config;
use ab_notation_strategies::generators::*;
use proptest::prelude::*;

fn parse_serialise_parse(source: &str) -> (String, String) {
    let doc = Document::new(source.to_owned());
    let tree = doc.parse();
    let first = tree.to_source();
    let doc2 = Document::new(first.clone());
    let tree2 = doc2.parse();
    let second = tree2.to_source();
    (first, second)
}

fn assert_facade_round_trip_converges(source: &str) {
    let (first, second) = parse_serialise_parse(source);
    assert_eq!(
        first, second,
        "facade round-trip diverges on second pass for source {source:?}\n\
         after 1st: {first:?}\n\
         after 2nd: {second:?}"
    );
}

#[test]
fn empty_input_round_trips() {
    assert_facade_round_trip_converges("");
}

#[test]
fn plain_text_round_trips() {
    assert_facade_round_trip_converges("Hello, world.");
    assert_facade_round_trip_converges("こんにちは。\n\n本日は晴れ。");
}

#[test]
fn ruby_round_trips() {
    assert_facade_round_trip_converges("｜青梅《おうめ》");
    assert_facade_round_trip_converges("青梅《おうめ》");
}

#[test]
fn paired_container_round_trips() {
    assert_facade_round_trip_converges("［＃ここから2字下げ］\nbody\n［＃ここで字下げ終わり］");
}

proptest! {
    #![proptest_config(default_config())]

    /// `Document::parse` is total over the `aozora_fragment` distribution.
    #[test]
    fn aozora_fragment_parse_is_total(s in aozora_fragment(120)) {
        let doc = Document::new(s);
        let _tree = doc.parse();
    }

    #[test]
    fn aozora_fragment_round_trip_converges(s in aozora_fragment(120)) {
        assert_facade_round_trip_converges(&s);
    }

    /// Public parse is total on unbalanced inputs that produce diagnostics.
    #[test]
    fn pathological_input_parse_is_total(s in pathological_aozora(120)) {
        let doc = Document::new(s);
        let _tree = doc.parse();
    }

    /// Public parse is total on combining marks, RTL overrides, and the PUA
    /// codepoints that the lexer reserves for sentinels, and keeps them as
    /// literal characters.
    #[test]
    fn unicode_adversarial_parse_is_total(s in unicode_adversarial()) {
        let doc = Document::new(s);
        let _tree = doc.parse();
    }
}
