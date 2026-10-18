//! Supplied line geometry remains independent of its enclosed text.

use std::path::Path;

use ab_aat_to_parser_ir::{ConversionOptions, ConversionRequest, MappingDocument, SchemaSet};
use serde_json::Value;

fn convert(body: &str) -> Value {
    let source = format!("題\n作者\n\n{body}\n\n底本：本\n");
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    ab_aat_to_parser_ir::convert(ConversionRequest {
        aat: serde_json::from_slice(&ab_aat::aat_json_from_bytes(source.as_bytes()).unwrap())
            .unwrap(),
        mapping: MappingDocument::from_path(&repo.join("data/aat-to-parser-ir-mapping-v2.json"))
            .unwrap(),
        schemas: SchemaSet::for_aat_version(2).unwrap(),
        options: ConversionOptions::default(),
    })
    .unwrap()
    .parser_ir
}

#[test]
fn partial_layout_keeps_body_and_unresolved_source_separate() {
    let ir =
        convert("［＃ここから３字下げ、未対応指定、２０字詰め］\n本文\n［＃ここで字下げ終わり］");
    assert_eq!(ir["layout_blocks"][0]["indent"], 3);
    assert_eq!(ir["layout_blocks"][0]["width"], 20);
    let facts = ir["interpretation_facts"].as_array().unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0]["kind"], "line-layout");
    let problems = ir["interpretation_problems"].as_array().unwrap();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0]["raw"], "未対応指定");
    assert!(
        ir["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["type"] == "text" && node["text"] == "本文")
    );
    assert!(!ir["nodes"].as_array().unwrap().iter().any(|node| {
        node["type"] == "text"
            && node["text"]
                .as_str()
                .is_some_and(|text| text.contains("未対応指定"))
    }));
}

#[test]
fn return_mark_clause_stays_typed_on_its_layout_scope() {
    let marker = "［＃ここから２字下げ、「レ」は返り点］";
    let ir = convert(&format!("{marker}\n漢レ文\n［＃ここで字下げ終わり］"));
    let scope = &ir["layout_blocks"][0];
    assert_eq!(scope["indent"], 2);
    let instruction = &scope["source_instructions"][0];
    assert_eq!(instruction["type"], "formatting-instruction");
    assert_eq!(instruction["raw"], marker);
    assert_eq!(
        instruction["operations"][0]["effect"]["kind"],
        "kunten-policy"
    );
    assert_eq!(
        instruction["operations"][0]["effect"]["scope"],
        "following-block"
    );
    assert_eq!(
        instruction["operations"][0]["effect"]["marks"]["text"],
        "レ"
    );
    assert!(instruction["operations"][0]["selection"].is_object());
    assert!(
        ir["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["type"] == "kunten" && node["text"] == "レ")
    );
    assert!(!ir["nodes"].as_array().unwrap().iter().any(|node| {
        node["type"] == "text"
            && node["text"]
                .as_str()
                .is_some_and(|text| text.contains('レ'))
    }));
    assert!(ir["interpretation_problems"].as_array().unwrap().is_empty());
}

#[test]
fn return_mark_close_clause_stays_typed_on_its_layout_scope() {
    let marker = "［＃ここで字下げ終わり、「レ」は返り点］";
    let ir = convert(&format!("［＃ここから２字下げ］\n漢レ文\n{marker}"));
    let scope = &ir["layout_blocks"][0];
    assert_eq!(scope["indent"], 2);
    let instruction = &scope["source_instructions"][0];
    assert_eq!(instruction["type"], "formatting-instruction");
    assert_eq!(instruction["raw"], marker);
    assert_eq!(
        instruction["operations"][0]["effect"]["scope"],
        "preceding-block"
    );
    assert!(instruction["operations"][0]["selection"].is_object());
    assert!(
        ir["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["type"] == "kunten" && node["text"] == "レ")
    );
    assert!(ir["interpretation_problems"].as_array().unwrap().is_empty());
}
