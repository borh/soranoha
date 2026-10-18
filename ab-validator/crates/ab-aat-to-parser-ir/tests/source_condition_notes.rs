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
fn correction_and_legibility_notes_are_typed_and_located() {
    let correction = convert("霰《みぞれ》［＃「霙」の誤り？、400-7］の音");
    let note = correction["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["note_kind"] == "uncertain-correction")
        .unwrap();
    assert_eq!(note["proposed_text"], "霙");
    assert_eq!(note["certainty"], "uncertain");
    assert_eq!(note["source_locator"], "400-7");
    assert_eq!(note["target_source_spans"].as_array().unwrap().len(), 1);
    assert_eq!(
        note["proposed_source_span"]["end"].as_u64().unwrap()
            - note["proposed_source_span"]["start"].as_u64().unwrap(),
        '霙'.len_utf8() as u64
    );
    let legibility = convert("あたしまつぴらだわ［＃印刷不鮮明、87-14］。");
    let note = legibility["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["note_kind"] == "source-legibility")
        .unwrap();
    assert_eq!(note["source_locator"], "87-14");
    assert_eq!(note["text"], "印刷不鮮明、87-14");
    assert_eq!(legibility["interpretation_problems"], serde_json::json!([]));
}

#[test]
fn questioned_omission_retains_its_semantic_kind_and_uncertainty() {
    let ir = convert("前［＃「人」が脱落か？］後");
    let note = ir["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["note_kind"] == "omission")
        .unwrap();

    assert_eq!(note["text"], "「人」が脱落か？");
    assert_eq!(note["certainty"], "uncertain");
    assert_eq!(ir["interpretation_problems"], serde_json::json!([]));
}

#[test]
fn supplied_translation_remains_distinct_from_an_explanation() {
    let ir = convert("前［＃現代語訳「松籟《しょうらい》を聞かせる。」］後");
    let note = ir["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["note_kind"] == "translation")
        .unwrap();

    assert_eq!(note["annotation_children"][0]["type"], "ruby");
    assert_eq!(note["annotation_children"][0]["ruby"]["base"], "松籟");
    assert_eq!(ir["interpretation_problems"], serde_json::json!([]));
}

#[test]
fn ruby_reading_annotation_keeps_its_axis_and_exact_target() {
    let ir = convert(
        "間人連老《ハシビトノムラジオユ》［＃ルビの「ハシビトノムラジオユ」に〈原〉の注記］",
    );
    let note = ir["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["note_kind"] == "ruby-reading-annotation")
        .unwrap();

    assert_eq!(note["text"], "〈原〉");
    assert_eq!(note["target_text"], "ハシビトノムラジオユ");
    assert_eq!(note["target_source_spans"][0]["start"], 27);
    assert_eq!(note["target_source_spans"][0]["end"], 57);
    assert_eq!(ir["interpretation_problems"], serde_json::json!([]));
}

#[test]
fn ruby_reading_sic_keeps_its_unique_subrange_target() {
    let ir = convert("東海美人《しうり》［＃ルビの「しう」にママの注記］");
    let note = ir["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["note_kind"] == "sic" && node["target_text"] == "しう")
        .unwrap();

    assert_eq!(note["text"], "ママ");
    assert_eq!(note["target_source_spans"][0]["start"], 27);
    assert_eq!(note["target_source_spans"][0]["end"], 33);
    assert_eq!(ir["interpretation_problems"], serde_json::json!([]));
}

#[test]
fn named_correction_keeps_structured_proposal_and_exact_target() {
    let ir = convert("勧《つと》めた［＃「勧《つと》めた」は「勤《つと》めた」の誤記か］後");
    let note = ir["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["note_kind"] == "uncertain-correction")
        .unwrap();

    assert_eq!(note["target_text"], "勧めた");
    assert_eq!(note["proposed_text"], "勤めた");
    assert_eq!(note["proposed_children"][0]["type"], "ruby");
    assert_eq!(note["proposed_children"][0]["ruby"]["base"], "勤");
    assert_eq!(
        note["target_source_spans"][0]["end"].as_u64().unwrap()
            - note["target_source_spans"][0]["start"].as_u64().unwrap(),
        "勧《つと》めた".len() as u64
    );
    assert_eq!(ir["interpretation_problems"], serde_json::json!([]));
}
