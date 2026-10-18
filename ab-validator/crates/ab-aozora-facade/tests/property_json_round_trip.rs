//! JSON-format round-trip properties.
//!
//! The six example tests in `tests/json_format.rs` fix the JSON output for
//! hand-written inputs (envelope versions, severity and source axes, ruby node
//! and pair shapes), which cover only the shapes those inputs produce.
//!
//! The properties below project the four JSON envelopes ([`diagnostics`],
//! [`nodes`], [`pairs`], [`container_pairs`]) of arbitrary Aozora input from the
//! `ab_notation_strategies` generators and assert two properties on each:
//!
//! 1. **The output parses as JSON.** For every source the JSON output is a
//!    syntactically valid JSON document. An unescaped control byte or a
//!    trailing comma would break every driver (`aozora-ffi` / `aozora-wasm` /
//!    `aozora-py`); proptest shrinks the failing input.
//! 2. **The round trip is canonical.** Parsing the JSON output as
//!    `serde_json::Value` and re-serialising it with `serde_json::to_string`
//!    yields a byte-identical envelope, because `Value`'s `Serialize` impl
//!    preserves the source object key order. Non-canonical key ordering would
//!    break cross-driver byte parity even when each driver parses valid JSON.

#![cfg(feature = "json")]

use ab_aozora_facade::{Document, json};
use ab_notation_strategies::config::default_config;
use ab_notation_strategies::generators::*;
use proptest::prelude::*;

fn assert_envelope_is_well_formed_json(label: &str, expected: u32, source: &str, json: &str) {
    let parsed: serde_json::Value = serde_json::from_str(json).unwrap_or_else(|e| {
        panic!("{label} envelope is not valid JSON for source {source:?}\n{e}\n---\n{json}")
    });
    let obj = parsed.as_object().unwrap_or_else(|| {
        panic!("{label} envelope must be a JSON object for source {source:?}\n---\n{json}")
    });
    let version = obj
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_else(|| {
            panic!("{label} envelope missing schemaVersion for source {source:?}\n---\n{json}")
        });
    assert_eq!(
        u32::try_from(version).ok(),
        Some(expected),
        "{label} envelope schemaVersion drift for source {source:?}: got {version}, expected {expected}",
    );
    assert!(
        obj.get("data").is_some_and(serde_json::Value::is_array),
        "{label} envelope missing data array for source {source:?}\n---\n{json}"
    );
}

fn assert_envelope_round_trips(label: &str, source: &str, json: &str) {
    let parsed: serde_json::Value =
        serde_json::from_str(json).expect("envelope already validated by caller");
    let reserialised = serde_json::to_string(&parsed).unwrap_or_else(|e| {
        panic!("{label} envelope refused to re-serialise for source {source:?}\n{e}")
    });
    // `serde_json::Value` preserves insertion order in its `Map`, so output
    // whose key order is canonical round-trips byte-equal; dynamic key
    // ordering, such as from a `HashMap`, fails this assertion.
    assert_eq!(
        json, reserialised,
        "{label} envelope round-trip is not byte-canonical for source {source:?}\n\
         original: {json}\nre-serialised: {reserialised}"
    );
}

fn assert_json_round_trip(source: &str) {
    let doc = Document::new(source);
    let tree = doc.parse();

    // Properties (1) and (2) for each of the four envelopes.
    let diags = json::diagnostics(tree.diagnostics());
    assert_envelope_is_well_formed_json(
        "diagnostics",
        json::DIAGNOSTICS_SCHEMA_VERSION,
        source,
        &diags,
    );
    assert_envelope_round_trips("diagnostics", source, &diags);

    let nodes = json::nodes(&tree);
    assert_envelope_is_well_formed_json("nodes", json::NODES_SCHEMA_VERSION, source, &nodes);
    assert_envelope_round_trips("nodes", source, &nodes);

    let pairs = json::pairs(&tree);
    assert_envelope_is_well_formed_json("pairs", json::PAIRS_SCHEMA_VERSION, source, &pairs);
    assert_envelope_round_trips("pairs", source, &pairs);

    let containers = json::container_pairs(&tree);
    assert_envelope_is_well_formed_json(
        "container_pairs",
        json::CONTAINER_PAIRS_SCHEMA_VERSION,
        source,
        &containers,
    );
    assert_envelope_round_trips("container_pairs", source, &containers);
}

#[test]
fn empty_input_round_trips() {
    assert_json_round_trip("");
}

#[test]
fn plain_text_round_trips() {
    assert_json_round_trip("Hello, world.");
    assert_json_round_trip("こんにちは。\n\n本日は晴れ。");
}

#[test]
fn ruby_round_trips() {
    assert_json_round_trip("｜青梅《おうめ》");
    assert_json_round_trip("青梅《おうめ》");
}

#[test]
fn paired_container_round_trips() {
    assert_json_round_trip(
        "［＃ここから2字下げ］\n\
         body\n\
         ［＃ここで字下げ終わり］",
    );
}

#[test]
fn diagnostic_carrying_input_round_trips() {
    assert_json_round_trip("a［＃unclosed bracket");
}

proptest! {
    #![proptest_config(default_config())]

    /// Every Aozora-shaped fragment produces four envelopes that are valid JSON
    /// and byte-canonical after a `serde_json::Value` round trip.
    #[test]
    fn aozora_fragment_json_round_trips(s in aozora_fragment(120)) {
        assert_json_round_trip(&s);
    }

    /// Unbalanced bracket shapes produce the most diagnostics; every emitted
    /// diagnostic serialises into well-formed JSON regardless of their number.
    #[test]
    fn pathological_input_json_round_trips(s in pathological_aozora(120)) {
        assert_json_round_trip(&s);
    }

    /// Inputs with combining marks, RTL overrides, and PUA codepoints; PUA
    /// inputs exercise the diagnostics envelope's `codepoint` field, which must
    /// JSON-escape the payload.
    #[test]
    fn unicode_adversarial_json_round_trips(s in unicode_adversarial()) {
        assert_json_round_trip(&s);
    }
}
