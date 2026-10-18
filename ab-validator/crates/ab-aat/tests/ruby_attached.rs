//! Ruby-presence notes preserve the supplied base and its source relation.

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
fn ruby_presence_note_links_the_existing_ruby_and_its_base() {
    let value = aat("起上り《レカバリー》［＃「起上り」にルビ］");
    let nodes = content(&value);

    assert_eq!(nodes[0]["kind"], "ruby");
    assert_eq!(nodes[1]["kind"], "editorial_note");
    assert_eq!(nodes[1]["note_kind"], "ruby-attached");
    assert_eq!(nodes[1]["target_text"], "起上り");
    assert_eq!(nodes[1]["target_source_spans"], json!([span(0, 9)]));
    assert_eq!(nodes[1]["subject_source_span"], span(0, 30));
}

#[test]
fn unbound_ruby_presence_note_does_not_invent_source_relations() {
    let value = aat("本文［＃「別文」にルビ］");
    let note = &content(&value)[1];

    assert_eq!(note["note_kind"], "ruby-attached");
    assert_eq!(note["target_text"], "別文");
    assert!(note.get("target_source_spans").is_none());
    assert!(note.get("subject_source_span").is_none());
}
