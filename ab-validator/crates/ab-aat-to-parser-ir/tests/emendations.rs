use std::path::Path;

use ab_aat_to_parser_ir::{ConversionOptions, ConversionRequest, MappingDocument, SchemaSet};
use serde_json::{Value, json};

fn convert(source: &str) -> Value {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let aat =
        serde_json::from_slice(&ab_aat::aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    ab_aat_to_parser_ir::convert(ConversionRequest {
        aat,
        mapping: MappingDocument::from_path(&repo.join("data/aat-to-parser-ir-mapping-v2.json"))
            .unwrap(),
        schemas: SchemaSet::for_aat_version(2).unwrap(),
        options: ConversionOptions::default(),
    })
    .unwrap()
    .parser_ir
}

#[test]
fn applied_emendations_pass_through_with_their_source_span() {
    let ir = convert("前\n「もう一杯｜《く》呉れ」\n");
    let start = "前\n".len();
    assert_eq!(
        ir["emendations"],
        json!([{"id": "genka-138-reading-before-base",
            "transcribed": "「もう一杯｜《く》呉れ」", "emended": "「もう一杯｜呉《く》れ」",
            "source_span": {"start": start, "end": start + "「もう一杯｜呉《く》れ」".len(),
                "line": 2, "coordinate_system": "decoded_utf8"}}])
    );
}

#[test]
fn unemended_source_has_no_emendations() {
    let ir = convert("前｜向《むか》う\n");
    assert!(ir.get("emendations").is_none(), "{ir}");
}
