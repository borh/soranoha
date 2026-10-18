//! Ruby-binding apparatus selects the supplied base and keeps adjacent text visible.

use ab_aat::aat_json_from_bytes;
use serde_json::{Value, json};

fn aat(source: &str) -> Value {
    serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap()
}

fn content(value: &Value) -> &[Value] {
    value["blocks"][0]["content"].as_array().unwrap()
}

fn span(start: usize, end: usize) -> Value {
    json!({"byte_start":start, "byte_end":end, "line_start":1, "line_end":1})
}

#[test]
fn retarget_extends_an_implicit_ruby_over_the_supplied_phrase() {
    let source = "これは白い《エステ・エ・ブランコ》［＃ルビは「これは白い」にかかる］";
    let value = aat(source);
    let nodes = content(&value);

    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0]["kind"], "ruby");
    assert_eq!(nodes[0]["base"], "これは白い");
    assert_eq!(nodes[0]["reading"], "エステ・エ・ブランコ");
    assert_eq!(nodes[0]["span"], span(0, 51));
    assert_eq!(nodes[1]["kind"], "editorial_note");
    assert_eq!(nodes[1]["note_kind"], "ruby-retarget");
    assert_eq!(nodes[1]["target_text"], "これは白い");
    assert_eq!(nodes[1]["target_source_spans"], json!([span(0, 15)]));
    assert_eq!(nodes[1]["subject_source_span"], span(0, 51));
}

#[test]
fn punctuation_between_target_and_reading_stays_visible_after_the_ruby() {
    let source = "さあ、出ろ！《ヴアツ・セ・エンポーラ》［＃ルビは「さあ、出ろ」にかかる］";
    let value = aat(source);
    let nodes = content(&value);

    assert_eq!(nodes[0]["kind"], "ruby");
    assert_eq!(nodes[0]["base"], "さあ、出ろ");
    assert_eq!(nodes[0]["reading"], "ヴアツ・セ・エンポーラ");
    assert_eq!(
        nodes[1],
        json!({"kind":"text", "value":"！", "span":span(15, 18)})
    );
    assert_eq!(nodes[2]["note_kind"], "ruby-retarget");
    assert_eq!(nodes[2]["target_source_spans"], json!([span(0, 15)]));
    assert_eq!(nodes[2]["subject_source_span"], span(0, 57));
}

#[test]
fn all_corpus_binding_verbs_retain_the_same_relation() {
    for verb in ["かかる", "掛かる", "付く"] {
        let source = format!("金口煙草《きんくち》［＃ルビは「金口煙草」に{verb}］");
        let value = aat(&source);
        let note = &content(&value)[1];

        assert_eq!(note["note_kind"], "ruby-retarget", "{verb}");
        assert_eq!(note["target_text"], "金口煙草", "{verb}");
        assert_eq!(note["target_source_spans"], json!([span(0, 12)]), "{verb}");
    }
}

#[test]
fn an_unbound_statement_does_not_invent_source_relations() {
    let value = aat("本文［＃ルビは「別文」にかかる］");
    let note = &content(&value)[1];

    assert_eq!(note["note_kind"], "ruby-retarget");
    assert_eq!(note["target_text"], "別文");
    assert!(note.get("target_source_spans").is_none());
    assert!(note.get("subject_source_span").is_none());
}

#[test]
fn a_quoted_ruby_note_extends_the_ruby_over_the_quoted_base() {
    let value = aat("お伽話《フェヤリー》［＃「お伽話」のルビ］");
    let nodes = content(&value);

    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0]["base"], "お伽話");
    assert_eq!(nodes[0]["reading"], "フェヤリー");
    assert_eq!(nodes[1]["note_kind"], "ruby-retarget");
    assert_eq!(nodes[1]["target_text"], "お伽話");
    assert_eq!(nodes[1]["target_source_spans"], json!([span(0, 9)]));
    assert_eq!(nodes[1]["subject_source_span"], span(0, 30));
}
