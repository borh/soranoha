//! A supplied table role and a supplied absence of table rules stay on the
//! scope the source supplies them for, beside its geometry.

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

fn fact_kinds(ir: &Value) -> Vec<String> {
    ir["interpretation_facts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|fact| fact["kind"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_table_clause_states_the_role_of_the_scope_its_indentation_measures() {
    let ir = convert(
        "［＃ここから１字下げ、表組み］\n王朝　年代\n西漢　元始二年\n［＃ここで字下げ終わり］",
    );
    let scope = &ir["layout_blocks"][0];
    assert_eq!(scope["role"], "table");
    assert_eq!(scope["indent"], 1);
    assert!(scope.get("table_rules").is_none(), "no rule assertion made");
    assert_eq!(ir["interpretation_problems"], serde_json::json!([]));
    // The lines are the source's own; nothing splits them into cells.
    let text = serde_json::to_string(&ir["nodes"]).unwrap();
    assert!(text.contains("王朝　年代"), "source spacing kept: {text}");
}

#[test]
fn a_supplied_absence_of_rules_is_recorded_and_a_silence_is_not() {
    let ruleless =
        convert("［＃ここから１字下げ、表組み、罫無し］\n甲　乙\n［＃ここで字下げ終わり］");
    assert_eq!(ruleless["layout_blocks"][0]["role"], "table");
    assert_eq!(ruleless["layout_blocks"][0]["table_rules"], false);

    let silent = convert("［＃ここから１字下げ、表組み］\n甲　乙\n［＃ここで字下げ終わり］");
    assert!(silent["layout_blocks"][0].get("table_rules").is_none());
}

#[test]
fn a_figure_or_table_clause_does_not_become_a_table_claim_about_the_lines() {
    let ir =
        convert("［＃ここから２字下げ、横組み、図表］\n太陽年　365日\n［＃ここで字下げ終わり］");
    let scope = &ir["layout_blocks"][0];
    assert_eq!(scope["role"], "figure-table");
    assert_eq!(scope["indent"], 2);
    assert_eq!(scope["direction"], "horizontal");
    assert_eq!(ir["interpretation_problems"], serde_json::json!([]));
}

#[test]
fn a_fused_frame_and_table_clause_keeps_both_axes_separate() {
    for body in [
        "［＃ここから２字下げ、罫囲みの表］\n甲　乙\n［＃ここで字下げ終わり］",
        "［＃ここから２字下げ、表罫囲み］\n甲　乙\n［＃ここで字下げ、表罫囲み終わり］",
    ] {
        let ir = convert(body);
        let scope = &ir["layout_blocks"][0];
        assert_eq!(scope["role"], "table", "{body}");
        assert_eq!(scope["indent"], 2, "{body}");
        assert_eq!(scope["typography"]["kind"], "keigakomi", "{body}");
        assert_eq!(scope["typography"]["border"], "rule", "{body}");
        assert_eq!(
            ir["interpretation_problems"],
            serde_json::json!([]),
            "{body}"
        );
    }
}

#[test]
fn a_role_supplied_with_indentation_claims_the_geometry_as_well_as_the_role() {
    let ir = convert("［＃ここから１字下げ、表組み］\n甲　乙\n［＃ここで字下げ終わり］");
    let kinds = fact_kinds(&ir);
    assert!(kinds.iter().any(|kind| kind == "table"), "{kinds:?}");
    assert!(kinds.iter().any(|kind| kind == "line-layout"), "{kinds:?}");

    // A role supplied alone asserts only the role.
    let bare = convert("［＃ここから表］\n甲　乙\n［＃ここで表終わり］");
    let kinds = fact_kinds(&bare);
    assert!(kinds.iter().any(|kind| kind == "table"), "{kinds:?}");
    assert!(!kinds.iter().any(|kind| kind == "line-layout"), "{kinds:?}");
}

#[test]
fn a_base_edition_table_description_stays_apparatus() {
    let ir = convert("［＃ここから２字下げ、底本では表組み］\n甲\n［＃ここで字下げ終わり］");
    let scope = &ir["layout_blocks"][0];
    assert!(scope.get("role").is_none(), "an edition note is not a role");
    assert_eq!(scope["indent"], 2);
    let kinds = fact_kinds(&ir);
    assert!(
        kinds.iter().any(|kind| kind == "editorial-note"),
        "{kinds:?}"
    );
    assert!(!kinds.iter().any(|kind| kind == "table"), "{kinds:?}");
}

#[test]
fn a_rule_absence_without_a_supplied_table_stays_an_unresolved_clause() {
    let ir = convert("［＃ここから１字下げ、罫無し］\n甲\n［＃ここで字下げ終わり］");
    let scope = &ir["layout_blocks"][0];
    assert!(scope.get("table_rules").is_none());
    assert!(scope.get("role").is_none());
    assert!(
        !ir["interpretation_problems"].as_array().unwrap().is_empty(),
        "the clause is retained as unresolved, not silently dropped"
    );
}

#[test]
fn a_standalone_table_opener_keeps_the_axes_it_states_beside_the_role() {
    for (body, expected) in [
        (
            "［＃ここから表罫囲み］\n甲　乙\n［＃ここで表罫囲み終わり］",
            serde_json::json!({"kind":"keigakomi","border":"rule"}),
        ),
        (
            "［＃ここからプログラム、表罫囲み］\n甲　乙\n［＃ここでプログラム（表罫囲み）終わり］",
            serde_json::json!({"kind":"keigakomi","border":"rule"}),
        ),
    ] {
        let ir = convert(body);
        let scope = &ir["layout_blocks"][0];
        assert_eq!(scope["role"], "table", "{body}");
        assert!(
            scope.get("indent").is_none(),
            "no measure is invented: {body}"
        );
        assert_eq!(scope["typography"]["kind"], expected["kind"], "{body}");
        assert_eq!(scope["typography"]["border"], expected["border"], "{body}");
    }
}

#[test]
fn a_horizontal_table_keeps_the_direction_the_source_fused_into_the_role_word() {
    let ir = convert("［＃ここから横組みの表］\n甲　乙\n［＃ここで横組みの表終わり］");
    let scope = &ir["layout_blocks"][0];
    assert_eq!(scope["role"], "table");
    assert_eq!(scope["direction"], "horizontal");
    assert_eq!(ir["interpretation_problems"], serde_json::json!([]));
}

fn explanation_notes(ir: &serde_json::Value) -> Vec<&str> {
    ir["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|node| node["type"] == "editor-note" && node["note_kind"] == "explanation")
        .map(|node| node["text"].as_str().unwrap())
        .collect()
}

#[test]
fn a_clause_the_table_reading_does_not_cover_is_an_explanation() {
    // `プログラム` states what the enclosed lines are. It is not a role, an
    // enclosure or a direction, so it stays an explanation beside the table
    // scope rather than being folded into one of them.
    let ir = convert(
        "［＃ここからプログラム、表罫囲み］\n甲　乙\n［＃ここでプログラム（表罫囲み）終わり］",
    );
    assert_eq!(ir["layout_blocks"][0]["role"], "table");
    assert_eq!(explanation_notes(&ir), ["プログラム"]);
}

#[test]
fn a_table_close_stating_an_enclosure_does_not_end_a_scope_without_one() {
    let ir = convert("［＃ここから表］\n甲　乙\n［＃ここで表罫囲み終わり］");
    assert_eq!(ir["layout_blocks"], serde_json::json!([]));
    assert!(!ir["interpretation_problems"].as_array().unwrap().is_empty());
}

#[test]
fn a_ruled_partition_is_a_ruled_table_whose_partitions_stay_text() {
    // `罫仕切り` delimits ruled partitions, and the clause states that a run
    // of `----` bounds one partition. The `----` lines stay literal text, so
    // the partitions are not derived as scopes.
    let ir = convert(
        "［＃ここから罫仕切り、----で挾まれた部分が一つの仕切り内］\n----\n甲\n----\n［＃ここで罫仕切り終わり］",
    );
    let scope = &ir["layout_blocks"][0];
    assert_eq!(scope["role"], "table");
    assert_eq!(scope["typography"]["border"], "rule");
    assert_eq!(
        explanation_notes(&ir),
        ["----で挾まれた部分が一つの仕切り内"]
    );
    let text = serde_json::to_string(&ir["nodes"]).unwrap();
    assert!(
        text.contains("\"----\\n\""),
        "the partition lines stay text: {text}"
    );
}
