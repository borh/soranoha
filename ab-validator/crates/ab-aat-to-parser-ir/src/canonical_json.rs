//! Canonical (sorted-key) JSON output for `ab-aat-to-parser-ir`.
//!
//! `ab-aat-to-parser-ir`'s parser-IR / divergence-bundle output, and the
//! `schema_hash` / `document_hash` mechanism in [`crate::schema`], are built
//! as `serde_json::Value` trees (mostly via the `json!` macro) and need
//! sorted object keys to produce stable, canonical bytes (persisted files,
//! sha256 hashes, golden-fixture comparisons). `serde_json::Map` is a
//! `BTreeMap` by default, which is sorted. If any crate in the compiling
//! workspace's unified feature graph enables the `preserve_order` feature
//! (Cargo unifies features across all crates sharing a `Cargo.lock`), `Map`
//! becomes an `IndexMap` that preserves insertion order, and the same code
//! emits differently-ordered (but semantically identical) JSON.
//!
//! `ab-aozora-facade`'s `json` feature enables `serde_json/preserve_order`.
//! The functions here sort keys explicitly at the call sites that turn a
//! `Value` into canonical bytes, so the output does not depend on the map
//! backend `serde_json` uses.
use serde_json::Value;

/// Recursively rebuild `value` inserting object keys in sorted order.
///
/// Under default `serde_json` (`BTreeMap`-backed `Map`) this is a byte-level
/// no-op: `BTreeMap` already iterates in sorted order, so re-inserting into a
/// fresh `Map` in sorted order reproduces the same order. Under
/// `preserve_order` (`IndexMap`-backed `Map`) sorted insertion restores
/// canonical output regardless of insertion order. Applied at serialization
/// call sites so canonical bytes never depend on the workspace feature
/// graph.
pub fn sort_keys_deep(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut pairs: Vec<(String, Value)> = map.into_iter().collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            let mut sorted = serde_json::Map::new();
            for (key, val) in pairs {
                sorted.insert(key, sort_keys_deep(val));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_keys_deep).collect()),
        leaf => leaf,
    }
}

/// Serialize `value` as pretty-printed, canonical (sorted-key) JSON text.
///
/// This is the wrapper used at the `ab-aat-to-parser-ir` CLI's `convert`
/// call site (see `src/main.rs`) so the persisted `parser_ir_out` /
/// `divergence_out` files are canonical independent of the workspace
/// feature graph. The `preserve-order-canary` crate's end-to-end test calls
/// it directly with `serde_json/preserve_order` enabled, in its own excluded
/// workspace.
pub fn to_canonical_json_pretty(value: Value) -> serde_json::Result<String> {
    serde_json::to_string_pretty(&sort_keys_deep(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sorts_nested_objects_and_recurses_into_arrays() {
        let value = json!({
            "zeta": 1,
            "alpha": [{"b": 2, "a": 3}],
        });
        let sorted = sort_keys_deep(value);
        assert_eq!(
            serde_json::to_string(&sorted).unwrap(),
            r#"{"alpha":[{"a":3,"b":2}],"zeta":1}"#
        );
    }

    #[test]
    fn leaves_non_object_leaves_untouched() {
        assert_eq!(sort_keys_deep(json!(null)), json!(null));
        assert_eq!(sort_keys_deep(json!(1)), json!(1));
        assert_eq!(sort_keys_deep(json!("x")), json!("x"));
        assert_eq!(sort_keys_deep(json!([3, 1, 2])), json!([3, 1, 2]));
    }
}
