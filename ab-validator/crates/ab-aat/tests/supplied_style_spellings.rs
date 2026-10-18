//! Source spellings of established styles, frames and centring share their
//! canonical AAT nodes.

use ab_aat::aat_json_from_bytes;
use serde_json::{Value, json};

fn aat(source: &str) -> Value {
    serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap()
}

fn find<'a>(value: &'a Value, kind: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) if map.get("kind").and_then(Value::as_str) == Some(kind) => Some(value),
        Value::Object(map) => map.values().find_map(|child| find(child, kind)),
        Value::Array(items) => items.iter().find_map(|child| find(child, kind)),
        _ => None,
    }
}

fn text(value: &Value) -> String {
    match value {
        Value::Object(map) if map.get("kind").and_then(Value::as_str) == Some("text") => {
            map["value"].as_str().unwrap().to_owned()
        }
        Value::Object(map) => ["content", "children"]
            .iter()
            .filter_map(|key| map.get(*key))
            .map(text)
            .collect(),
        Value::Array(items) => items.iter().map(text).collect(),
        _ => String::new(),
    }
}

#[test]
fn quoted_spellings_type_their_target() {
    for (source, kind, fields, target) in [
        (
            "前字［＃「字」の部分はイタリック体］後",
            "style",
            json!({"style_type": "italic"}),
            "字",
        ),
        (
            "前字［＃「字」の部分は斜体］後",
            "style",
            json!({"style_type": "italic"}),
            "字",
        ),
        (
            "前字［＃「字」はゴシック体］後",
            "style",
            json!({"style_type": "gothic"}),
            "字",
        ),
        (
            "前拝観料［＃「拝観料」は罫で囲む］後",
            "keigakomi",
            json!({"border": "rule"}),
            "拝観料",
        ),
        (
            "前守レ！［＃「守レ！」に枠線］後",
            "keigakomi",
            json!({"border": "unspecified"}),
            "守レ！",
        ),
    ] {
        let aat = aat(source);
        let node = find(&aat, kind).unwrap_or_else(|| panic!("{source}: {aat}"));
        for (key, expected) in fields.as_object().unwrap() {
            assert_eq!(&node[key], expected, "{source}: {aat}");
        }
        assert_eq!(text(node), target, "{source}: {aat}");
        assert!(!aat.to_string().contains("interpretation_problem"), "{aat}");
    }
}

#[test]
fn line_gothic_markers_set_the_text_of_their_line() {
    for (source, target) in [
        ("前\n本文［＃この行はゴシック体］\n後", "本文"),
        ("前\n　見出し［＃ゴシック体］\n後", "見出し"),
        ("前\n見出し［＃ゴシック］\n後", "見出し"),
    ] {
        let aat = aat(source);
        let style = find(&aat, "style").unwrap_or_else(|| panic!("{source}: {aat}"));
        assert_eq!(style["style_type"], "gothic", "{aat}");
        assert_eq!(text(style), target, "{source}: {aat}");
        assert!(!aat.to_string().contains("interpretation_problem"), "{aat}");
    }
}

#[test]
fn a_bare_gothic_marker_followed_by_text_stays_raw() {
    let aat = aat("前\n携帯品［＃ゴシック体］――鞄\n後");
    let raw = find(&aat, "raw").unwrap_or_else(|| panic!("{aat}"));
    assert_eq!(raw["source"], "［＃ゴシック体］", "{aat}");
    assert!(raw.get("interpretation_problem").is_some(), "{aat}");
    assert_eq!(text(&aat["blocks"]), "前\n携帯品――鞄\n後", "{aat}");
}

#[test]
fn line_centring_spellings_centre_their_line() {
    for marker in ["［＃直線は中央に配置］", "［＃中央寄せ］"] {
        let source = format!("前\n――――{marker}\n後");
        let aat = aat(&source);
        let block = find(&aat, "layout_block").unwrap_or_else(|| panic!("{source}: {aat}"));
        assert_eq!(block["align"], "center", "{aat}");
        assert_eq!(text(block), "――――", "{aat}");
    }
}
