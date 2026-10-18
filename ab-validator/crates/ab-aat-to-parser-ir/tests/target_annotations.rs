use std::path::Path;

use ab_aat_to_parser_ir::{ConversionOptions, ConversionRequest, MappingDocument, SchemaSet};
use serde_json::{Value, json};

fn convert(body: &str) -> Value {
    let source = format!("題\n作者\n\n{body}\n\n底本：本\n");
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mapping =
        MappingDocument::from_path(&repo.join("data/aat-to-parser-ir-mapping-v2.json")).unwrap();
    let schemas = SchemaSet::for_aat_version(2).unwrap();
    ab_aat_to_parser_ir::convert(ConversionRequest {
        aat: serde_json::from_slice(&ab_aat::aat_json_from_bytes(source.as_bytes()).unwrap())
            .unwrap(),
        mapping,
        schemas,
        options: ConversionOptions::default(),
    })
    .unwrap()
    .parser_ir
}

fn nodes(value: &Value) -> Vec<&Value> {
    let mut result = Vec::new();
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            Value::Object(map) => {
                result.push(value);
                pending.extend(map.values());
            }
            Value::Array(values) => pending.extend(values),
            _ => {}
        }
    }
    result
}

#[test]
fn ruby_retarget_keeps_the_repaired_base_and_both_source_relations() {
    let ir = convert("これは白い《エステ・エ・ブランコ》［＃ルビは「これは白い」にかかる］");
    let all = nodes(&ir);
    let ruby = all.iter().find(|node| node["type"] == "ruby").unwrap();
    let note = all
        .iter()
        .find(|node| node["type"] == "editor-note" && node["note_kind"] == "ruby-retarget")
        .unwrap();

    assert_eq!(ruby["ruby"]["base"], "これは白い");
    assert_eq!(ruby["ruby"]["reading"], "エステ・エ・ブランコ");
    assert_eq!(note["target_text"], "これは白い");
    let target = &note["target_source_spans"][0];
    assert_eq!(
        target["end"].as_u64().unwrap() - target["start"].as_u64().unwrap(),
        15
    );
    assert_eq!(
        note["subject_source_span"]["end"].as_u64().unwrap()
            - note["subject_source_span"]["start"].as_u64().unwrap(),
        51
    );
}

#[test]
fn ruby_presence_note_keeps_the_existing_ruby_relation() {
    let ir = convert("起上り《レカバリー》［＃「起上り」にルビ］");
    let note = nodes(&ir)
        .into_iter()
        .find(|node| node["type"] == "editor-note" && node["note_kind"] == "ruby-attached")
        .unwrap();

    assert_eq!(note["target_text"], "起上り");
    assert_eq!(
        note["target_source_spans"][0]["end"].as_u64().unwrap()
            - note["target_source_spans"][0]["start"].as_u64().unwrap(),
        9
    );
    assert_eq!(
        note["subject_source_span"]["end"].as_u64().unwrap()
            - note["subject_source_span"]["start"].as_u64().unwrap(),
        30
    );
    assert_eq!(ir["interpretation_problems"], json!([]));
}

#[test]
fn inputter_note_reference_keeps_its_number() {
    let ir = convert("語［＃入力者注(12)］後");
    let note = nodes(&ir)
        .into_iter()
        .find(|node| {
            node["type"] == "editor-note" && node["note_kind"] == "inputter-note-reference"
        })
        .unwrap();

    assert_eq!(note["note_number"], 12);
    assert_eq!(note["text"], "入力者注(12)");
    assert_eq!(ir["interpretation_problems"], json!([]));
}

#[test]
fn named_apparatus_keeps_witness_groups_and_readings() {
    let ir = convert(
        "、［＃「、」は、『全集　第十九巻』（出版社2015年）では「。」、『全集　第十五卷』（出版社1968年）では「、」］",
    );
    let apparatus = nodes(&ir)
        .into_iter()
        .find(|node| node["type"] == "text-apparatus")
        .unwrap();

    assert_eq!(apparatus["text"], "、");
    assert_eq!(apparatus["readings"][0]["text"], "。");
    assert_eq!(apparatus["readings"][1]["text"], "、");
    assert_eq!(
        apparatus["readings"][0]["witnesses"][0]["label"],
        "『全集　第十九巻』（出版社2015年）"
    );
    assert_eq!(
        apparatus["readings"][1]["witnesses"][0]["label"],
        "『全集　第十五卷』（出版社1968年）"
    );
    assert_eq!(ir["interpretation_problems"], json!([]));
}

#[test]
fn critical_note_keeps_editorial_role_and_margin_placement() {
    let ir = convert("仰願寺［＃「仰願寺」に欄外に校注、「小さきろうそく」］");
    let note = nodes(&ir)
        .into_iter()
        .find(|node| node["type"] == "annotated-text")
        .unwrap();

    assert_eq!(note["note_kind"], "critical-note");
    assert_eq!(note["position"], "margin");
    assert_eq!(note["text"], "仰願寺");
    assert_eq!(note["annotation_children"][0]["text"], "小さきろうそく");
    assert_eq!(ir["interpretation_problems"], json!([]));
}

#[test]
fn target_notes_preserve_principal_and_annotation_axes_with_only_supplied_side() {
    for (body, position) in [
        ("刺［＃「刺」に「テフダ」の注記］後", None),
        ("刺［＃「刺」の左に「テフダ」の注記］後", Some("left")),
        ("刺［＃「刺」の右に「テフダ」の注記］後", Some("right")),
        ("刺［＃「刺」に「ママ」注記］後", None),
    ] {
        let ir = convert(body);
        let all = nodes(&ir);
        let annotation = all
            .iter()
            .find(|node| node["type"] == "annotated-text")
            .unwrap();
        assert_eq!(annotation["text"], "刺");
        assert_eq!(annotation["position"].as_str(), position);
        assert_eq!(
            annotation["annotation_children"][0]["span"]["coordinate_system"],
            "annotation_utf8"
        );
        assert_eq!(annotation["annotation_children"][0]["span"]["start"], 0);
        assert_eq!(
            annotation["inline_children"][0]["span"]["coordinate_system"],
            "parser_text_utf8"
        );
        assert_eq!(ir["interpretation_problems"], json!([]));
        let source = format!("題\n作者\n\n{body}\n\n底本：本\n");
        let note_child = &annotation["annotation_children"][0];
        let start = usize::try_from(note_child["source_span"]["start"].as_u64().unwrap()).unwrap();
        let end = usize::try_from(note_child["source_span"]["end"].as_u64().unwrap()).unwrap();
        assert_eq!(&source[start..end], note_child["text"].as_str().unwrap());
        let fact = ir["interpretation_facts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|fact| fact["kind"] == "annotated-text")
            .unwrap();
        assert_eq!(fact["source_span"], annotation["source_span"]);
    }
}

#[test]
fn paired_annotations_retain_both_source_boundaries() {
    let open = "［＃左に注記付き］";
    let target = "病起《びょうき》";
    let close = "［＃左に「（ママ）」の注記付き終わり］";
    let body = format!("{open}{target}{close}");
    let ir = convert(&body);
    let annotation = nodes(&ir)
        .into_iter()
        .find(|node| node["type"] == "annotated-text")
        .unwrap();
    let markers = annotation["interpretation_marker_spans"]
        .as_array()
        .unwrap();
    assert_eq!(markers.len(), 2);
    let source = format!("題\n作者\n\n{body}\n\n底本：本\n");
    for (marker, expected) in markers.iter().zip([open, close]) {
        let start = usize::try_from(marker["start"].as_u64().unwrap()).unwrap();
        let end = usize::try_from(marker["end"].as_u64().unwrap()).unwrap();
        assert_eq!(&source[start..end], expected);
    }
}

#[test]
fn annotations_reuse_rich_source_fragments_without_turning_ruby_into_principal_text() {
    let ir = convert("漢《かん》［＃「漢」に「字《じ》」の注記］後");
    let all = nodes(&ir);
    let annotation = all
        .iter()
        .find(|node| node["type"] == "annotated-text")
        .unwrap();
    assert_eq!(annotation["text"], "漢");
    assert_eq!(annotation["inline_children"][0]["type"], "ruby");
    assert_eq!(annotation["inline_children"][0]["ruby"]["reading"], "かん");
    assert_eq!(annotation["annotation_children"][0]["type"], "ruby");
    assert_eq!(
        annotation["annotation_children"][0]["ruby"]["reading"],
        "じ"
    );
    assert_eq!(
        annotation["annotation_children"][0]["span"]["coordinate_system"],
        "annotation_utf8"
    );
    assert_eq!(ir["interpretation_problems"], json!([]));
}

#[test]
fn unsupported_annotation_content_retains_principal_text_and_located_uncertainty() {
    let ir = convert("刺［＃「刺」に「字［＃未知の注記］」の注記］後");
    let all = nodes(&ir);
    assert!(!all.iter().any(|node| node["type"] == "annotated-text"));
    assert!(all.iter().any(|node| node["text"] == "刺"));
    assert!(!ir["interpretation_problems"].as_array().unwrap().is_empty());
}

#[test]
fn native_interior_target_preserves_surrounding_okurigana() {
    let ir = convert("土を堀り［＃「堀」に「ママ」注記］かへす");
    let annotation = nodes(&ir)
        .into_iter()
        .find(|node| node["type"] == "annotated-text")
        .unwrap();
    assert_eq!(annotation["text"], "堀");
    assert_eq!(annotation["annotation_children"][0]["text"], "ママ");
    let text: String = ir["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|node| node["type"] != "source-note")
        .filter_map(|node| node["text"].as_str())
        .collect();
    assert_eq!(text, "土を堀りかへす");
    assert_eq!(ir["interpretation_problems"], json!([]));
}

#[test]
fn mismatched_annotation_target_does_not_invent_principal_content() {
    let ir = convert("刺［＃「別」に「ママ」注記］後");
    assert!(
        !nodes(&ir)
            .iter()
            .any(|node| node["type"] == "annotated-text")
    );
    let text: String = ir["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|node| node["type"] != "source-note")
        .filter_map(|node| node["text"].as_str())
        .collect();
    assert_eq!(text, "刺後");
    assert!(!ir["interpretation_problems"].as_array().unwrap().is_empty());
}
