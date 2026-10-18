//! Ruby association is retained through realization of source-encoded Latin letters.
use ab_aat::aat_json_from_bytes;
use ab_aozora_facade::{Document, syntax::accent::ACCENT_TABLE};
use serde_json::Value;

#[test]
fn source_accents_preserve_the_whole_adjacent_latin_base() {
    for (source_base, expected) in [
        ("〔e'galite'〕", "égalité"),
        ("〔Charite'〕", "Charité"),
        ("〔Ske^ne^〕", "Skênê"),
        ("〔Orche^stra〕", "Orchêstra"),
        ("〔Proske^nion〕", "Proskênion"),
    ] {
        let source = format!("前{source_base}《よみ》後");
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let nodes = aat["blocks"][0]["content"].as_array().unwrap();
        let ruby = nodes
            .iter()
            .find(|n| n["kind"] == "ruby")
            .unwrap_or_else(|| panic!("{aat}"));
        assert_eq!(ruby["base"], expected);
        assert_eq!(ruby["reading"], "よみ");
        assert!(nodes.iter().all(|n| n["kind"] != "raw"), "{aat}");
    }
}

#[test]
fn accent_letters_share_the_latin_class_without_crossing_other_boundaries() {
    for (source, expected) in [
        ("前〔e'galite'〕《よみ》", "égalité"),
        ("〔cafe'〕｜〔e'〕《よみ》", "é"),
        ("〔e'〕Я《よみ》", "Я"),
        ("Я〔e'〕《よみ》", "é"),
        ("〔e'〕 《よみ》", "é "),
        ("〔e'〕.《よみ》", "é."),
        ("word 〔e'〕 《よみ》", "word é "),
        ("×《よみ》", "×"),
        ("÷《よみ》", "÷"),
    ] {
        let canonical = Document::new(source).parse().to_source();
        for spelling in [source, canonical.as_str()] {
            let aat: Value =
                serde_json::from_slice(&aat_json_from_bytes(spelling.as_bytes()).unwrap()).unwrap();
            let nodes = aat["blocks"][0]["content"].as_array().unwrap();
            let ruby = nodes.iter().find(|n| n["kind"] == "ruby").unwrap();
            assert_eq!(ruby["base"], expected, "{spelling}");
        }
    }
    for source in ["〔e'〕。《よみ》", "前「×」《よみ》"] {
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        assert!(
            aat["blocks"][0]["content"]
                .as_array()
                .unwrap()
                .iter()
                .all(|n| n["kind"] != "ruby"),
            "{source}"
        );
    }
}

#[test]
fn accent_note_inside_a_latin_word_keeps_the_whole_word_as_base() {
    for (source, base) in [
        (
            "軍艦Venus［＃「e」はアクサン（´）付き］《ヴェニュス》号",
            "Vénus",
        ),
        (
            "公使Leon［＃「e」はアクサン（´）付き］《レオン》 Roche",
            "Léon",
        ),
        ("の｜Rene［＃「e」はアクサン（´）付き］《ルネ》", "René"),
    ] {
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let nodes = aat["blocks"][0]["content"].as_array().unwrap();
        let rubies: Vec<_> = nodes.iter().filter(|n| n["kind"] == "ruby").collect();
        assert_eq!(rubies.len(), 1, "{source}: {aat}");
        assert_eq!(rubies[0]["base"], base, "{source}");
        assert!(nodes.iter().all(|n| n["kind"] != "raw"), "{source}: {aat}");
    }
}

#[test]
fn all_composed_accent_letters_are_retained_through_ruby_association() {
    let mut source = String::new();
    for (_, letter) in ACCENT_TABLE {
        source.push('x');
        source.push(*letter);
        source.push_str("《よみ》 ");
    }
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let bases: Vec<_> = aat["blocks"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["kind"] == "ruby")
        .map(|n| n["base"].as_str().unwrap())
        .collect();
    let expected: Vec<_> = ACCENT_TABLE
        .iter()
        .map(|(_, letter)| format!("x{letter}"))
        .collect();
    assert_eq!(bases, expected);
}

fn span_source<'a>(source: &'a str, span: &Value) -> &'a str {
    let start = usize::try_from(span["byte_start"].as_u64().unwrap()).unwrap();
    let end = usize::try_from(span["byte_end"].as_u64().unwrap()).unwrap();
    &source[start..end]
}

#[test]
fn ruby_base_accents_keep_their_spelling_and_scope_delimiters() {
    for (source, base, spelling) in [
        ("前〔Ku:ss〕《キュッス》後", "Küss", "u:"),
        ("前｜〔Ku:ss〕《キュッス》後", "Küss", "u:"),
        ("前 s'op〔e`〕re《ソペエル》 後", "s'opère", "e`"),
    ] {
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let nodes = aat["blocks"][0]["content"].as_array().unwrap();
        let ruby = nodes.iter().find(|n| n["kind"] == "ruby").unwrap();
        assert_eq!(ruby["base"], base, "{source}");
        let content = ruby["base_content"].as_array().unwrap();
        let accents: Vec<_> = content.iter().filter(|n| n["kind"] == "accent").collect();
        assert_eq!(accents.len(), 1, "{source}: {aat}");
        assert_eq!(span_source(source, &accents[0]["span"]), spelling);
        let delimiters: Vec<_> = accents[0]["interpretation_marker_spans"]
            .as_array()
            .unwrap()
            .iter()
            .map(|span| span_source(source, span))
            .collect();
        assert_eq!(delimiters, ["〔", "〕"], "{source}");
        let text: String = content
            .iter()
            .map(|n| {
                n["value"]
                    .as_str()
                    .or_else(|| n["resolved"].as_str())
                    .unwrap()
            })
            .collect();
        assert_eq!(text, base, "{source}");
    }
}
