//! Implicit ruby preserves script and explicit boundaries.

use ab_aat::aat_json_from_bytes;
use ab_aozora_facade::Document;
use serde_json::Value;

fn nodes(source: &str) -> Vec<Value> {
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    aat["blocks"][0]["content"].as_array().unwrap().clone()
}

#[test]
fn cyrillic_and_decimal_runs_keep_their_supplied_readings() {
    for (source, base, reading) in [
        ("現代のСССР《エスエスエスエル》", "СССР", "エスエスエスエル"),
        ("М《ム》", "М", "ム"),
        ("Ёё《よ》", "Ёё", "よ"),
        ("８《エイト》", "８", "エイト"),
        ("12３《すうじ》", "12３", "すうじ"),
    ] {
        let content = nodes(source);
        let ruby = content.iter().find(|node| node["kind"] == "ruby").unwrap();
        assert_eq!(ruby["base"], base);
        assert_eq!(ruby["reading"], reading);
        let canonical = Document::new(source).parse().to_source();
        let reparsed = nodes(&canonical);
        let reparsed_ruby = reparsed.iter().find(|node| node["kind"] == "ruby").unwrap();
        assert_eq!(reparsed_ruby["base"], base);
        assert_eq!(reparsed_ruby["reading"], reading);
    }
}

#[test]
fn classes_and_explicit_bars_bound_the_base() {
    for (source, base) in [
        ("ABCЯ《ヤ》", "Я"),
        ("Яa《エー》", "a"),
        ("９｜８《はち》", "８"),
        ("｜Я８《やはち》", "Я８"),
    ] {
        let content = nodes(source);
        let ruby = content.iter().find(|node| node["kind"] == "ruby").unwrap();
        assert_eq!(ruby["base"], base);
        let canonical = Document::new(source).parse().to_source();
        let reparsed = nodes(&canonical);
        assert_eq!(
            reparsed.iter().find(|node| node["kind"] == "ruby").unwrap()["base"],
            base
        );
    }
    for source in ["Я 《や》", "８.《はち》"] {
        assert!(nodes(source).iter().all(|node| node["kind"] != "ruby"));
    }
}

#[test]
fn internal_latin_punctuation_preserves_ruby_targets_and_source_boundaries() {
    for (source, base, raw) in [
        ("J'avoue《ジャヴウ》", "J'avoue", "J'avoue《ジャヴウ》"),
        ("J'ai《ジェ》", "J'ai", "J'ai《ジェ》"),
        ("a-t-il《ザテイル》", "a-t-il", "a-t-il《ザテイル》"),
        ("Woman's《ウーマンス》", "Woman's", "Woman's《ウーマンス》"),
        (
            "Je-m'en-foutiste《ドウデモイイニスト》",
            "Je-m'en-foutiste",
            "Je-m'en-foutiste《ドウデモイイニスト》",
        ),
        ("'J'avoue《ジャヴウ》'", "J'avoue", "J'avoue《ジャヴウ》"),
        ("mot 'ai《よみ》", "ai", "ai《よみ》"),
        ("a--il《よみ》", "il", "il《よみ》"),
        ("a-'il《よみ》", "il", "il《よみ》"),
        ("Я-ai《よみ》", "ai", "ai《よみ》"),
        ("9-ai《よみ》", "ai", "ai《よみ》"),
        ("a-｜il《よみ》", "il", "｜il《よみ》"),
        (
            "｜Mr. O'Grie《ミスター・オーグリー》",
            "Mr. O'Grie",
            "｜Mr. O'Grie《ミスター・オーグリー》",
        ),
    ] {
        let content = nodes(source);
        let ruby = content.iter().find(|node| node["kind"] == "ruby").unwrap();
        assert_eq!(ruby["base"], base, "{source}");
        let start = usize::try_from(ruby["span"]["byte_start"].as_u64().unwrap()).unwrap();
        let end = usize::try_from(ruby["span"]["byte_end"].as_u64().unwrap()).unwrap();
        assert_eq!(&source[start..end], raw, "{source}");
        let canonical = Document::new(source).parse().to_source();
        let reparsed = nodes(&canonical);
        let restored = reparsed.iter().find(|node| node["kind"] == "ruby").unwrap();
        assert_eq!(restored["base"], ruby["base"], "{source}");
        assert_eq!(restored["reading"], ruby["reading"], "{source}");
        assert_eq!(
            Document::new(canonical.as_str()).parse().to_source(),
            canonical
        );
    }
}

#[test]
fn source_observed_ruby_reading_keeps_nested_emphasis() {
    let source = "前｜違い《チ［＃「チ」に傍点］ガエ》後";
    let content = nodes(source);
    let ruby = content.iter().find(|node| node["kind"] == "ruby").unwrap();

    assert_eq!(ruby["base"], "違い");
    assert_eq!(ruby["reading"], "チガエ");
    let reading = ruby["reading_content"].as_array().unwrap();
    assert_eq!(reading.len(), 2);
    assert_eq!(reading[0]["kind"], "style");
    assert_eq!(reading[0]["style_type"], "bouten");
    assert_eq!(reading[0]["content"][0]["value"], "チ");
    assert_eq!(reading[1]["kind"], "text");
    assert_eq!(reading[1]["value"], "ガエ");
}
