//! Literal reference signs remain visible without resolving unknown gaiji.
use ab_aat::aat_json_from_bytes;
use ab_aozora_facade::Document;
use serde_json::Value;

fn has_explicit_reference(value: &Value, source: &str) -> bool {
    match value {
        Value::Object(fields) => {
            let retained_raw = value["kind"] == "raw"
                && value["source"]
                    .as_str()
                    .is_some_and(|raw| raw.contains('※'));
            let retained_gaiji = value["kind"] == "gaiji"
                && value["span"]["byte_start"]
                    .as_u64()
                    .zip(value["span"]["byte_end"].as_u64())
                    .and_then(|(start, end)| {
                        source.get(usize::try_from(start).ok()?..usize::try_from(end).ok()?)
                    })
                    .is_some_and(|raw| raw.contains('※'));
            retained_raw
                || retained_gaiji
                || fields
                    .values()
                    .any(|child| has_explicit_reference(child, source))
        }
        Value::Array(values) => values
            .iter()
            .any(|child| has_explicit_reference(child, source)),
        _ => false,
    }
}

#[test]
fn separators_numbered_references_and_symbol_mentions_remain_text() {
    for source in [
        "　※\n",
        "句　※１／書簡",
        "［※２］",
        "（※三）",
        "※137，※149",
        "※記号。この※番号。",
    ] {
        let canonical = Document::new(source).parse().to_source();
        assert_eq!(canonical, source);
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let nodes = aat["blocks"][0]["content"].as_array().unwrap();
        assert!(nodes.iter().all(|node| node["kind"] == "text"), "{aat}");
        let visible: String = nodes
            .iter()
            .map(|node| node["value"].as_str().unwrap())
            .collect();
        assert_eq!(visible, source);
    }
}

#[test]
fn unknown_glyphs_and_unexplained_prose_marks_remain_explicit() {
    for source in [
        "語※語",
        "※［＃判読不可］",
        "※※［＃「あ」の異体字］",
        "※[#unknown]",
        "※一部",
        "｜※1《しるし》",
        "｜甲※1《しるし》",
    ] {
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        assert!(has_explicit_reference(&aat["blocks"], source), "{aat}");
    }
}
