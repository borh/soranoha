//! Numbered inputter-note markers retain their reference identity.

use ab_aat::aat_json_from_bytes;
use serde_json::Value;

fn aat(source: &str) -> Value {
    serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap()
}

#[test]
fn numbered_inputter_note_is_a_typed_reference() {
    let value = aat("語［＃入力者注(12)］後");
    let note = &value["blocks"][0]["content"][1];

    assert_eq!(note["kind"], "editorial_note");
    assert_eq!(note["note_kind"], "inputter-note-reference");
    assert_eq!(note["note_number"], 12);
    assert_eq!(note["text"], "入力者注(12)");
    assert_eq!(note["span"]["byte_start"], 3);
    assert_eq!(note["span"]["byte_end"], 28);
}
