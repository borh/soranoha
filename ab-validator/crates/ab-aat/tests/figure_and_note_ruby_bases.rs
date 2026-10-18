//! A reading that follows a figure note or a target-associated note directly
//! takes the figure or the noted target as its base.

use ab_aat::aat_json_from_bytes;
use serde_json::Value;

fn inline(source: &str) -> Vec<Value> {
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    aat["blocks"][0]["content"].as_array().unwrap().clone()
}

#[test]
fn figure_and_noted_target_form_the_base_of_a_following_reading() {
    for (source, base, reading, kinds) in [
        (
            "白く［＃梵字（fig1317_15.png、横18×縦23）入る］《ラン》形",
            "",
            "ラン",
            &["figure"][..],
        ),
        (
            "の｜［＃梵字（fig1317_17.png、横23×縦22）入る］《オム》の",
            "",
            "オム",
            &["figure"][..],
        ),
        (
            "女の起誓［＃「起誓」に「ママ」の注記］《きしょう》を",
            "起誓",
            "きしょう",
            &["annotated_text"][..],
        ),
        (
            "た。√３［＃「３」は「√」の記号の中に入っている］《ルートサン》と",
            "√３",
            "ルートサン",
            &["text", "annotated_text"][..],
        ),
    ] {
        let nodes = inline(source);
        let ruby = nodes
            .iter()
            .find(|node| node["kind"] == "ruby")
            .unwrap_or_else(|| panic!("{source}: {nodes:?}"));
        assert_eq!(ruby["base"], base, "{source}");
        assert_eq!(ruby["reading"], reading, "{source}");
        let base_kinds: Vec<_> = ruby["base_content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["kind"].as_str().unwrap())
            .collect();
        assert_eq!(base_kinds, kinds, "{source}");
        assert!(
            nodes.iter().all(|node| node["kind"] != "raw"),
            "{source}: {nodes:?}"
        );
    }
}

#[test]
fn figure_or_note_without_a_directly_following_reading_stays_outside_ruby() {
    for source in [
        "白く［＃梵字（fig1317_15.png、横18×縦23）入る］形《かた》",
        "女の起誓［＃「起誓」に「ママ」の注記］を",
        "女の起誓［＃「起誓」に「ママ」の注記］《》を",
    ] {
        let nodes = inline(source);
        assert!(
            nodes
                .iter()
                .any(|node| matches!(node["kind"].as_str(), Some("figure" | "annotated_text"))),
            "{source}: {nodes:?}"
        );
        assert!(
            nodes.iter().all(|node| node["kind"] != "ruby"
                || node["base_content"]
                    .as_array()
                    .is_none_or(|content| content.iter().all(|child| child["kind"] == "text"))),
            "{source}: {nodes:?}"
        );
    }
}

#[test]
fn noted_target_that_continues_a_base_run_leaves_the_base_open() {
    let nodes = inline("大起誓［＃「起誓」に「ママ」の注記］《きしょう》を");
    assert!(nodes.iter().all(|node| node["kind"] != "ruby"), "{nodes:?}");
}

#[test]
fn latin_phrase_forms_one_base_only_where_no_single_word_does() {
    for (source, base) in [
        (
            "の Hsifan territory  《シフアン・テリトリー》",
            "Hsifan territory  ",
        ),
        (
            "が、Der Teufel hole sie!《デル トイフェル ホオレ ジイ》",
            "Der Teufel hole sie!",
        ),
        ("の Zwagri 《ツワグリ》", "Zwagri "),
        ("Mr. Smith《スミス》", "Smith"),
        ("Pestalozzi Str.《シユトラセ》", "Str."),
    ] {
        let nodes = inline(source);
        let ruby = nodes
            .iter()
            .find(|node| node["kind"] == "ruby")
            .unwrap_or_else(|| panic!("{source}: {nodes:?}"));
        assert_eq!(ruby["base"], base, "{source}");
    }
}
