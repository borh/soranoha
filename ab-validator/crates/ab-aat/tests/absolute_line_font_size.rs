//! Absolute line-size directives retain their size and coapplied weight.

use ab_aat::aat_json_from_bytes;
use serde_json::Value;

#[test]
fn absolute_line_size_and_weight_apply_to_the_preceding_line() {
    for (marker, size, bold) in [
        ("大文字", "large", false),
        ("中文字", "medium", false),
        ("小文字", "small", false),
        ("大文字、太字", "large", true),
    ] {
        let source = format!("本文［＃{marker}］\n次行");
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let block = &aat["blocks"][0];
        assert_eq!(block["kind"], "layout_block", "{aat}");
        assert_eq!(block["children"][0]["content"][0]["value"], "本文", "{aat}");
        let formatting = block["formatting"].as_array().map_or_else(
            || vec![&block["formatting"]],
            |values| values.iter().collect(),
        );
        assert!(
            formatting.iter().any(|value| {
                value["kind"] == "font_size"
                    && value["size_type"] == "absolute"
                    && value["size"] == size
            }),
            "{aat}"
        );
        assert_eq!(
            formatting
                .iter()
                .any(|value| value["kind"] == "style" && value["style_type"] == "bold"),
            bold,
            "{aat}"
        );
        assert!(!aat.to_string().contains("\"kind\":\"raw\""), "{aat}");
    }
}
