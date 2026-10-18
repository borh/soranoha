//! Qualified break semantics are retained in both wire projections.

use std::path::Path;

use ab_aat_to_parser_ir::{ConversionOptions, ConversionRequest, MappingDocument, SchemaSet};
use serde_json::Value;

fn objects(value: &Value) -> Vec<&Value> {
    let mut found = Vec::new();
    if let Value::Object(map) = value {
        found.push(value);
        for child in map.values() {
            found.extend(objects(child));
        }
    } else if let Value::Array(items) = value {
        for child in items {
            found.extend(objects(child));
        }
    }
    found
}

fn convert(source: &str) -> (Value, Value) {
    let aat: Value =
        serde_json::from_slice(&ab_aat::aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = ab_aat_to_parser_ir::convert(ConversionRequest {
        aat: aat.clone(),
        mapping: MappingDocument::from_path(&repo.join("data/aat-to-parser-ir-mapping-v2.json"))
            .unwrap(),
        schemas: SchemaSet::for_aat_version(2).unwrap(),
        options: ConversionOptions::default(),
    })
    .unwrap();
    (aat, output.parser_ir)
}

#[test]
fn qualified_break_fields_are_retained_through_aat_and_parser_ir_projection() {
    for (source, break_kind, field, expected, ir_type) in [
        (
            "［＃改見開き右］",
            "kaimihiraki",
            "destination_side",
            Value::from("right"),
            "page-break",
        ),
        (
            "［＃改ページ、ページの左右中央に］",
            "page",
            "following_placement",
            Value::from("horizontal-center"),
            "page-break",
        ),
        (
            "［＃改丁、左寄せで］",
            "kaicho",
            "following_placement",
            Value::from("left"),
            "page-break",
        ),
        (
            "［＃改ページ、２字下げで］",
            "page",
            "following_indent",
            Value::from(2),
            "page-break",
        ),
    ] {
        let (aat, ir) = convert(source);
        let aat_break = objects(&aat)
            .into_iter()
            .find(|node| node["kind"] == "layout_break")
            .expect("AAT layout break");
        assert_eq!(aat_break["break_kind"], break_kind, "{source}");
        assert_eq!(aat_break[field], expected, "{source}");

        let ir_break = objects(&ir)
            .into_iter()
            .find(|node| node["type"] == ir_type)
            .expect("parser-IR layout break");
        assert_eq!(ir_break["marker"], break_kind, "{source}");
        assert_eq!(ir_break[field], expected, "{source}");
    }
}
