//! Compound following-line instructions retain both effects and exact physical-line targets.

use ab_aat::aat_json_from_bytes;
use serde_json::Value;

fn instruction(node: &Value) -> Option<&Value> {
    if node["kind"] == "formatting_instruction"
        && node["operations"].as_array().is_some_and(|operations| {
            operations
                .iter()
                .any(|operation| operation["effect"]["kind"] == "rule-style")
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
fn following_line_typeface_and_rule_style_keep_exact_line_targets() {
    let source = concat!(
        "［＃次３行は、文字はゴシック体、罫線は全て波線］\n",
        "　　　　　　　┌〜〜〜〜〜〜〜〜〜〜┐\n",
        "　　　　　　　│　秘密を孕む怪悲劇　│\n",
        "　　　　　　　└〜〜〜〜〜〜〜〜〜〜┘\n",
        "市内大森区"
    );
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let instruction = instruction(&aat).expect("following-line formatting instruction");
    let operations = instruction["operations"].as_array().unwrap();
    assert_eq!(operations.len(), 2);
    assert_eq!(
        operations[0]["selector"],
        serde_json::json!({"kind":"following-lines", "lines":3})
    );
    assert_eq!(operations[0]["effect"]["kind"], "attribute");
    assert_eq!(operations[0]["effect"]["attribute"]["style_type"], "gothic");
    assert_eq!(
        operations[1]["effect"],
        serde_json::json!({"kind":"rule-style", "style":"wavy"})
    );
    for operation in operations {
        let selection = &operation["selection"];
        assert_eq!(
            selection["targets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|span| source_text(source, span))
                .collect::<Vec<_>>(),
            [
                "　　　　　　　┌〜〜〜〜〜〜〜〜〜〜┐",
                "　　　　　　　│　秘密を孕む怪悲劇　│",
                "　　　　　　　└〜〜〜〜〜〜〜〜〜〜┘",
            ]
        );
    }
    assert!(instruction.get("interpretation_problem").is_none(), "{aat}");
}
