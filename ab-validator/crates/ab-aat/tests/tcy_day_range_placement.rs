//! Day-range TCY placement retains both source-owned schedule anchors.

use ab_aat::aat_json_from_bytes;
use serde_json::Value;

fn instruction(node: &Value) -> Option<&Value> {
    if node["kind"] == "formatting_instruction"
        && node["operations"].as_array().is_some_and(|operations| {
            operations
                .iter()
                .any(|operation| operation["effect"]["kind"] == "relative-placement")
        })
    {
        return Some(node);
    }
    match node {
        Value::Object(fields) => fields.values().find_map(instruction),
        Value::Array(values) => values.iter().find_map(instruction),
        _ => None,
    }
}

fn source_text<'a>(source: &'a str, span: &Value) -> &'a str {
    &source[usize::try_from(span["byte_start"].as_u64().unwrap()).unwrap()
        ..usize::try_from(span["byte_end"].as_u64().unwrap()).unwrap()]
}

#[test]
fn tcy_below_a_day_range_preserves_both_source_anchors() {
    let source = "１日\n２日\n３日\n４日\n５日\n６日\n７日\n８日\n９日\n10［＃「10」は縦中横］日\n注記［＃「注記」は２日〜10日の下に縦中横］";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let instruction = instruction(&aat).expect("day-range formatting instruction");
    let effect = &instruction["operations"][1]["effect"];

    assert_eq!(effect["kind"], "relative-placement");
    assert_eq!(effect["relation"], "below");
    assert_eq!(effect["anchor"]["kind"], "day-range");
    assert_eq!(effect["anchor"]["start"]["day"], 2);
    assert_eq!(effect["anchor"]["end"]["day"], 10);
    assert_eq!(effect["anchor"]["start"]["operand"]["text"], "２");
    assert_eq!(effect["anchor"]["end"]["operand"]["text"], "10");
    assert_eq!(
        source_text(source, &effect["anchor"]["start"]["target"]),
        "２"
    );
    assert_eq!(
        source_text(source, &effect["anchor"]["end"]["target"]),
        "10"
    );
    assert!(instruction.get("interpretation_problem").is_none(), "{aat}");
}

#[test]
fn incomplete_day_sequence_keeps_the_placement_unresolved() {
    let source = "２日\n４日\n注記［＃「注記」は２日〜４日の下に縦中横］";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let instruction = instruction(&aat).expect("day-range formatting instruction");
    let effect = &instruction["operations"][1]["effect"];

    assert!(effect["anchor"]["start"].get("target").is_none());
    assert!(effect["anchor"]["end"].get("target").is_none());
    assert_eq!(
        instruction["interpretation_problem"]["code"],
        "uninterpreted-notation"
    );
}
