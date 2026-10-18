//! Corpus spellings of glyph references, emphasis and line sizes reach their
//! typed AAT nodes.

use ab_aat::aat_json_from_bytes;
use serde_json::{Value, json};

/// The body blocks of `body` set after a title and an author line.
fn aat(body: &str) -> Value {
    let source = format!("題\n作者\n\n{body}\n\n底本：本\n");
    let tree: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    tree["blocks"].clone()
}

fn nodes<'a>(value: &'a Value, kind: &str, out: &mut Vec<&'a Value>) {
    match value {
        Value::Object(map) => {
            if map.get("kind").and_then(Value::as_str) == Some(kind) {
                out.push(value);
            }
            map.values().for_each(|child| nodes(child, kind, out));
        }
        Value::Array(items) => items.iter().for_each(|child| nodes(child, kind, out)),
        _ => {}
    }
}

fn all<'a>(value: &'a Value, kind: &str) -> Vec<&'a Value> {
    let mut out = Vec::new();
    nodes(value, kind, &mut out);
    out
}

fn no_raw(value: &Value, source: &str) {
    assert!(all(value, "raw").is_empty(), "{source}: {value}");
}

#[test]
fn a_refmark_before_an_uninterpreted_annotation_is_a_described_gaiji() {
    for (source, description) in [
        ("臨める※［＃「窗」の下に「心」］に", "「窗」の下に「心」"),
        ("一※［＃「※」は小書きの「ト」］筋", "「※」は小書きの「ト」"),
        ("※［＃「※」は、石へん＋乞］", "「※」は、石へん＋乞"),
    ] {
        let tree = aat(source);
        no_raw(&tree, source);
        let gaiji = all(&tree, "gaiji");
        assert_eq!(gaiji.len(), 1, "{source}");
        assert_eq!(gaiji[0]["description"], description);
        assert_eq!(gaiji[0]["unresolved_reason"], "unresolved");
    }
    // A refmark before an annotation that another notation reads stays text.
    let tree = aat("※［＃「※」に傍点］");
    assert!(all(&tree, "gaiji").is_empty());
    assert_eq!(all(&tree, "style")[0]["content"][0]["value"], "※");
}

#[test]
fn a_stray_opener_does_not_prevent_later_reference_resolution() {
    let tree = aat("［＃改丁」\n瞼※［＃「目＋匡」、第3水準1-88-81、28-8］");
    let gaiji = all(&tree, "gaiji");
    assert_eq!(gaiji.len(), 1);
    assert_eq!(gaiji[0]["resolved"], "眶");
}

#[test]
fn a_text_apparatus_quote_states_its_glyph_facts() {
    let source = "當麻の※［＃「口＋羊」、第3水準1-15-1］斐［＃「當麻の※［＃「口＋羊」、第3水準1-15-1］斐」は本文の書き下し文では「當摩の※［＃「口＋羊」、第3水準1-15-1］斐」］";
    let tree = aat(source);
    let apparatus = all(&tree, "text-apparatus");
    assert_eq!(apparatus.len(), 1);
    let facts = apparatus[0]["current_interpretation_facts"]
        .as_array()
        .unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0]["kind"], "gaiji");
    let start = "題\n作者\n\n".len() + source.find("」は本文").unwrap()
        - "※［＃「口＋羊」、第3水準1-15-1］斐".len();
    assert_eq!(facts[0]["source_span"]["start"], start);
}

#[test]
fn emphasis_spellings_type_their_mark() {
    for (source, mark, target) in [
        (
            "五月四日［＃「五月四日」に傍点（白丸）］",
            "白丸傍点",
            "五月四日",
        ),
        ("實體［＃「實體」に傍点◎］", "二重丸傍点", "實體"),
        ("［＃傍点◎］語［＃傍点◎終わり］", "二重丸傍点", "語"),
        (
            "私の生き方［＃「私の生き方」に白四角傍点］",
            "白四角傍点",
            "私の生き方",
        ),
        (
            "革命［＃「革命」に×傍点、伏字を起こした文字］",
            "ばつ傍点",
            "革命",
        ),
        (
            "遺書、もしくは発見されなかった［＃「遺書」から「発見されなかった」まで傍点］。",
            "傍点",
            "遺書、もしくは発見されなかった",
        ),
    ] {
        let tree = aat(source);
        no_raw(&tree, source);
        let style = all(&tree, "style");
        assert_eq!(style.len(), 1, "{source}");
        assert_eq!(style[0]["decoration"]["kind"], mark, "{source}");
        assert_eq!(style[0]["content"][0]["value"], target, "{source}");
    }
    let tree = aat("革命［＃「革命」に×傍点、伏字を起こした文字］");
    let note = all(&tree, "editorial_note");
    assert_eq!(note[0]["note_kind"], "base-edition");
    assert_eq!(note[0]["text"], "伏字を起こした文字");
}

#[test]
fn gothic_emphasis_is_two_operations_on_one_target() {
    let tree = aat("のは［＃「は」ゴシック体に傍点］と");
    let instruction = all(&tree, "formatting_instruction");
    assert_eq!(instruction.len(), 1);
    let attributes: Vec<_> = instruction[0]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|operation| operation["effect"]["attribute"]["style_type"].clone())
        .collect();
    assert_eq!(attributes, [json!("gothic"), json!("bouten")]);
    assert!(instruction[0].get("interpretation_problem").is_none());
}

#[test]
fn a_quoted_refmark_after_a_glyph_reference_refers_to_the_glyph() {
    let tree = aat("最後の※［＃より、1-2-25］［＃「※」に傍点］まで");
    let style = all(&tree, "style");
    assert_eq!(style.len(), 1);
    assert_eq!(style[0]["content"][0]["kind"], "gaiji");
    assert_eq!(style[0]["content"][0]["resolved"], "ゟ");
}

#[test]
fn a_leading_base_edition_attribution_is_a_text_variant() {
    let tree = aat("次［＃底本では「次」は「下」］図");
    let variant = all(&tree, "text-variant");
    assert_eq!(variant.len(), 1);
    assert_eq!(variant[0]["base_text"], "下");
    let tree = aat(
        "うん［＃「うん」に傍点］［＃底本では「うん［＃「うん」に傍点］」は「う［＃「う」に傍点］ん」］",
    );
    let variant = all(&tree, "text-variant");
    assert_eq!(variant.len(), 1);
    assert_eq!(
        variant[0]["current_interpretation_facts"][0]["kind"],
        "emphasis"
    );
}

#[test]
fn a_line_final_relative_size_sets_the_text_before_it() {
    let tree = aat("見出し［＃２回り大きな文字］\n本文");
    let size = all(&tree, "font_size");
    assert_eq!(size.len(), 1);
    assert_eq!(size[0]["level"], 2);
    assert_eq!(size[0]["content"][0]["value"], "見出し");
    // Text after the opener on its line leaves the opener raw.
    let tree = aat("見出し［＃２回り大きな文字］続き");
    let raw = all(&tree, "raw");
    assert_eq!(raw.len(), 1);
    assert_eq!(raw[0]["source"], "［＃２回り大きな文字］");
}

#[test]
fn listed_fraction_operands_select_their_occurrences_on_the_line() {
    let body = "R = 実際数m/偶然数n = mQ/N(VC)［＃「実際数m/偶然数n」「mQ/N(VC)」は分数］";
    let tree = aat(body);
    no_raw(&tree, body);
    let instruction = all(&tree, "formatting_instruction");
    assert_eq!(instruction.len(), 1);
    assert!(instruction[0].get("interpretation_problem").is_none());
    let operation = &instruction[0]["operations"][0];
    assert_eq!(operation["effect"]["attribute"]["kind"], "fraction");
    let line_start = "題\n作者\n\n".len();
    let target = |text: &str| {
        let start = line_start + body.find(text).unwrap();
        json!([start, start + text.len()])
    };
    let targets = operation["selection"]["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|span| json!([span["byte_start"], span["byte_end"]]))
        .collect::<Vec<_>>();
    assert_eq!(targets, [target("実際数m/偶然数n"), target("mQ/N(VC)")]);
    // An operand that occurs twice on the line leaves the operands unselected.
    let tree = aat("a/b = a/b + c/d［＃「a/b」「c/d」は分数］");
    let instruction = all(&tree, "formatting_instruction");
    assert_eq!(instruction.len(), 1);
    assert!(instruction[0]["operations"][0].get("selection").is_none());
}

#[test]
fn listed_word_endings_select_the_ending_of_each_nearest_word() {
    let note = "［＃「Novae」「discendae」および「docendae」のそれぞれの末尾「ae」は、「a」と「e」の合字］";
    let body = format!("discendaeque Novae discendae docendae.{note}");
    let tree = aat(&body);
    no_raw(&tree, &body);
    let instruction = all(&tree, "formatting_instruction");
    assert_eq!(instruction.len(), 1);
    assert!(instruction[0].get("interpretation_problem").is_none());
    let line_start = "題\n作者\n\n".len();
    let ending = |word: &str| {
        let end = line_start + body.find(word).unwrap() + word.len();
        json!([end - 2, end])
    };
    let operations = instruction[0]["operations"].as_array().unwrap();
    let targets = operations
        .iter()
        .map(|operation| {
            assert_eq!(operation["effect"]["kind"], "ligature");
            let target = &operation["selection"]["targets"][0];
            json!([target["byte_start"], target["byte_end"]])
        })
        .collect::<Vec<_>>();
    assert_eq!(
        targets,
        [ending(" Novae"), ending(" discendae"), ending(" docendae")]
    );
    // A nearest occurrence that does not end a word leaves the word unselected.
    let tree = aat(&format!("Novae discendaeque docendae{note}"));
    let instruction = all(&tree, "formatting_instruction");
    assert!(instruction[0]["operations"][1].get("selection").is_none());
    assert!(instruction[0].get("interpretation_problem").is_some());
}
