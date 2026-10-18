//! Accent notation is decoded without removing literal bracket text.

use ab_aat::aat_json_from_bytes;
use proptest::prelude::*;
use serde_json::Value;

fn visible_text(value: &Value) -> String {
    match value {
        Value::Array(values) => values.iter().map(visible_text).collect(),
        Value::Object(_) if value["kind"] == "text" => {
            value["value"].as_str().unwrap_or_default().to_owned()
        }
        Value::Object(_) if value["kind"] == "accent" => {
            value["resolved"].as_str().unwrap_or_default().to_owned()
        }
        Value::Object(_) => visible_text(&value["content"]),
        _ => String::new(),
    }
}

#[test]
fn isolated_consonant_accents_preserve_styling_and_source_spans() {
    for (spelling, expected) in [
        ("c,", "ç"),
        ("C,", "Ç"),
        ("s,", "ş"),
        ("S,", "Ş"),
        ("t,", "ţ"),
        ("T,", "Ţ"),
        ("c'", "ć"),
        ("C'", "Ć"),
        ("l'", "ĺ"),
        ("L'", "Ĺ"),
        ("m'", "ḿ"),
        ("M'", "Ḿ"),
        ("n'", "ń"),
        ("N'", "Ń"),
        ("r'", "ŕ"),
        ("R'", "Ŕ"),
        ("s'", "ś"),
        ("S'", "Ś"),
        ("z'", "ź"),
        ("Z'", "Ź"),
    ] {
        for source in [
            format!("［＃傍点］〔{spelling}〕［＃傍点終わり］"),
            format!("〔{spelling}〕［＃「〔{spelling}〕」に傍点］"),
        ] {
            let aat: Value =
                serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
            let style = &aat["blocks"][0]["content"][0];
            assert_eq!(style["kind"], "style", "{source}: {aat}");
            assert_eq!(style["style_type"], "bouten", "{source}: {aat}");
            assert_eq!(style["span"]["byte_start"], 0, "{source}: {aat}");
            assert_eq!(style["span"]["byte_end"], source.len(), "{source}: {aat}");
            let accent = &style["content"][0];
            assert_eq!(accent["kind"], "accent", "{source}: {aat}");
            assert_eq!(accent["resolved"], expected, "{source}: {aat}");
            assert_eq!(accent["source_spelling"], spelling, "{source}: {aat}");
            let start = usize::try_from(accent["span"]["byte_start"].as_u64().unwrap()).unwrap();
            let end = usize::try_from(accent["span"]["byte_end"].as_u64().unwrap()).unwrap();
            assert_eq!(&source[start..end], spelling, "{source}: {aat}");
        }
    }
}

#[test]
fn ambiguous_accent_punctuation_retains_text_and_exact_body_evidence() {
    for (body, expected, raw) in [
        (
            "〔Passantes du chemin qu'on eut peut-e^tre aime'es,〕",
            "Passantes du chemin qúon eut peut-être aimées,",
            vec!["s,"],
        ),
        ("〔L'art de la cite'〕", "L'art de la cité", vec!["L'"]),
        ("〔text, cafe'〕", "text, café", vec!["t,"]),
        ("〔nicht,ihr Ma:chte〕", "nicht,ihr Mächte", vec!["t,"]),
        (
            "〔Sciences,etc. Me'moires〕",
            "Sciences,etc. Mémoires",
            vec!["s,"],
        ),
        ("〔garc,on〕", "garçon", vec![]),
        ("〔qu'on〕", "qúon", vec![]),
        ("〔m'〕〔qu'on cafe'〕", "ḿqúon café", vec![]),
    ] {
        let source =
            format!("\u{feff}題〔qu'on〕\r\n作者\r\n\r\n{body}\r\n\r\n底本：〔qu'on〕\r\n");
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let visible = visible_text(&aat["blocks"][0]["content"]);
        assert_eq!(visible, expected, "{aat}");
        let problems = aat["meta"]["interpretation_problems"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        assert_eq!(
            problems
                .iter()
                .map(|p| p["raw"].as_str().unwrap())
                .collect::<Vec<_>>(),
            raw
        );
        for problem in problems {
            assert_eq!(problem["aspects"], serde_json::json!(["content"]));
            let start = usize::try_from(problem["source_span"]["start"].as_u64().unwrap()).unwrap();
            let end = usize::try_from(problem["source_span"]["end"].as_u64().unwrap()).unwrap();
            let decoded = source.trim_start_matches('\u{feff}');
            assert_eq!(&decoded[start..end], problem["raw"].as_str().unwrap());
            assert_eq!(problem["source_span"]["line"], 4);
        }
    }
    for body in [
        "qu'on",
        "q〔u'〕on",
        "〔u'〕",
        "〔c,〕",
        "〔m'〕",
        "〔欄外 Emil Brunner, Erlebnis, Erkenntnis und Glaube, 1923.〕",
    ] {
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(body.as_bytes()).unwrap()).unwrap();
        assert!(
            aat["meta"].get("interpretation_problems").is_none(),
            "{body}: {aat}"
        );
    }
}

#[test]
fn outer_literal_brackets_do_not_claim_nested_accent_uncertainty() {
    let source = "〔未閉\n〔L'art〕\n後〕";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let problems = aat["meta"]["interpretation_problems"].as_array().unwrap();
    assert_eq!(problems.len(), 1, "{aat}");
    assert_eq!(problems[0]["raw"], "L'", "{aat}");
    assert_eq!(problems[0]["aspects"], serde_json::json!(["content"]));
    let source = "〔ピース・ポリッジ・ホットとは\nPease porridge hot,\n意味のないことをいって遊ぶ子供の遊戯の一である。〕";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    assert!(
        aat["meta"].get("interpretation_problems").is_none(),
        "{aat}"
    );
}

fn source_notes(value: &Value) -> Vec<&Value> {
    let mut pending = vec![value];
    let mut raw = Vec::new();
    while let Some(value) = pending.pop() {
        match value {
            Value::Object(fields) => {
                if value.get("source").is_some() {
                    raw.push(value);
                }
                pending.extend(fields.values());
            }
            Value::Array(values) => pending.extend(values),
            _ => {}
        }
    }
    raw
}

#[test]
fn nested_accent_scopes_do_not_shift_later_source_notes() {
    let note = "［＃「平和」は底本では「価格」、正誤表による訂正］";
    for source in [
        format!("〔a'〔e'〕o'〕\n平和{note}"),
        format!("≪〔a'〔e'〕o'〕平和{note}≫"),
    ] {
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let raw = source_notes(&aat);
        assert!(raw.iter().any(|node| node["source"] == note), "{aat}");
        for node in raw {
            let start = usize::try_from(node["span"]["byte_start"].as_u64().unwrap()).unwrap();
            let end = usize::try_from(node["span"]["byte_end"].as_u64().unwrap()).unwrap();
            assert_eq!(node["source"], &source[start..end], "{node}");
        }
    }
}

#[test]
fn resolved_variant_spelling_preserves_original_accent_notation() {
    let source = "〔sorgfa:ltig［＃「sorgfa:ltig」は底本では「sorgfa:tig」］〕";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let raw = source_notes(&aat);
    let note = raw
        .iter()
        .find(|node| node["kind"] == "text-variant")
        .unwrap();
    assert_eq!(visible_text(&note["content"]), "sorgfältig");
    assert_eq!(note["base_text"], "sorgfätig");
    assert_eq!(
        note["source"],
        "［＃「sorgfa:ltig」は底本では「sorgfa:tig」］"
    );
    let start = usize::try_from(note["span"]["byte_start"].as_u64().unwrap()).unwrap();
    let end = usize::try_from(note["span"]["byte_end"].as_u64().unwrap()).unwrap();
    assert_eq!(note["source"], &source[start..end]);
}

#[test]
fn unparsed_source_gaps_do_not_own_the_visible_accent_prefix() {
    let source = "前〔cafe'〕［＃tail\r\n";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let raw = source_notes(&aat);
    assert_eq!(raw.len(), 1);
    assert_eq!(raw[0]["source"], "［＃tail");
    let start = usize::try_from(raw[0]["span"]["byte_start"].as_u64().unwrap()).unwrap();
    let end = usize::try_from(raw[0]["span"]["byte_end"].as_u64().unwrap()).unwrap();
    assert_eq!(&source[start..end], "［＃tail");
}

proptest! {
    #[test]
    fn retained_raw_notes_match_their_decoded_source_slices(
        prefix in "[a-zあ-ん]{0,30}",
        accents in prop::collection::vec(prop::sample::select(vec!["〔a'〕", "〔a'〔e'〕o'〕", "〔本全集〕"]), 0..5),
        quote in any::<bool>(),
        crlf in any::<bool>(),
    ) {
        let note = "［＃「平和」は底本では「価格」、正誤表による訂正］";
        let body = format!("{prefix}{}{}平和{note}", accents.join(""), if crlf { "\r\n" } else { "\n" });
        let source = if quote { format!("≪{body}≫") } else { body };
        let aat: Value = serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let raw = source_notes(&aat);
        prop_assert!(raw.iter().any(|node| node["source"] == note));
        for node in raw {
            let start = usize::try_from(node["span"]["byte_start"].as_u64().unwrap()).unwrap();
            let end = usize::try_from(node["span"]["byte_end"].as_u64().unwrap()).unwrap();
            prop_assert_eq!(node["source"].as_str(), source.get(start..end));
        }
    }
}

#[test]
fn accent_scope_delimiters_are_syntax_but_literal_brackets_are_text() {
    for (source, expected) in [
        ("〔a` la Huysmans〕", "à la Huysmans"),
        ("〔本全集〕", "〔本全集〕"),
        ("前〔a`〕中〔本全集〕後", "前à中〔本全集〕後"),
    ] {
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let actual = visible_text(&aat["blocks"][0]["content"]);
        assert_eq!(actual, expected, "{source}");
    }
}
