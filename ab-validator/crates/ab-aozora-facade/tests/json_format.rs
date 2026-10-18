//! Tests of the JSON format shared by the drivers. Every driver (`aozora-ffi`,
//! `aozora-wasm`, `aozora-py`) calls into [`ab_aozora_facade::json`] for JSON
//! projection; these tests fix the byte shape of that projection.

#![cfg(feature = "json")]

use ab_aozora_facade::{Document, json};

/// Every projection function serialises an empty parse as the canonical empty
/// envelope.
#[test]
fn empty_parse_serialises_to_canonical_envelope() {
    let doc = Document::new("plain");
    let tree = doc.parse();
    let canonical = r#"{"schemaVersion":3,"data":[]}"#;
    assert_eq!(json::diagnostics(tree.diagnostics()), canonical);
    assert_eq!(json::nodes(&tree), canonical);
    assert_eq!(json::pairs(&tree), canonical);
}

/// Each envelope has its own schema version; the container-pairs version
/// differs from the diagnostics, nodes and pairs versions.
#[test]
fn envelope_versions_change_independently() {
    assert_eq!(json::CONTAINER_PAIRS_SCHEMA_VERSION, 7);
    assert_eq!(json::DIAGNOSTICS_SCHEMA_VERSION, 3);
    assert_eq!(json::NODES_SCHEMA_VERSION, 3);
    assert_eq!(json::PAIRS_SCHEMA_VERSION, 3);
}

#[test]
fn diagnostic_json_has_severity_and_source_axes() {
    let doc = Document::new("a［＃unclosed");
    let tree = doc.parse();
    let json = json::diagnostics(tree.diagnostics());
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    let entry = parsed
        .get("data")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .expect("at least one diagnostic");
    assert_eq!(
        entry.get("severity").and_then(|v| v.as_str()),
        Some("error"),
        "unclosed delimiter is an error: {entry}"
    );
    assert_eq!(
        entry.get("source").and_then(|v| v.as_str()),
        Some("source"),
        "unclosed delimiter is source-side: {entry}"
    );
}

#[test]
fn ruby_node_byte_shape() {
    let doc = Document::new("｜青梅《おうめ》");
    let tree = doc.parse();
    let json = json::nodes(&tree);
    assert!(json.starts_with(r#"{"schemaVersion":3,"data":["#));
    assert!(json.contains(r#""kind":"ruby""#));
    assert!(json.contains(r#""span":{"start":"#));
}

#[test]
fn ruby_pair_byte_shape() {
    let doc = Document::new("｜青梅《おうめ》");
    let tree = doc.parse();
    let json = json::pairs(&tree);
    assert!(json.starts_with(r#"{"schemaVersion":3,"data":["#));
    assert!(json.contains(r#""kind":"ruby""#));
    assert!(json.contains(r#""open":{"start":"#));
    assert!(json.contains(r#""close":{"start":"#));
}

#[test]
fn all_three_channels_emit_valid_json() {
    let doc = Document::new("｜青梅《おうめ》abc\u{E001}def");
    let tree = doc.parse();
    for json in [
        json::diagnostics(tree.diagnostics()),
        json::nodes(&tree),
        json::pairs(&tree),
    ] {
        let value: serde_json::Value =
            serde_json::from_str(&json).expect("JSON output must be valid JSON");
        assert!(value.is_object(), "envelope must be JSON object");
        assert_eq!(
            value
                .get("schemaVersion")
                .and_then(serde_json::Value::as_u64),
            Some(3)
        );
        assert!(value.get("data").is_some_and(serde_json::Value::is_array));
    }
}
