//! Source selectors decorate only their established target ranges.
use serde_json::{Value, json};

fn visible(value: &Value) -> String {
    if value["kind"] == "text" {
        value["value"].as_str().unwrap().to_owned()
    } else {
        value["content"]
            .as_array()
            .map(|children| children.iter().map(visible).collect())
            .unwrap_or_default()
    }
}

fn styles<'a>(value: &'a Value, selected: &mut Vec<&'a Value>) {
    match value {
        Value::Object(object) => {
            if value["kind"] == "style" && value["style_type"] == "bouten" {
                selected.push(value);
            }
            for child in object.values() {
                styles(child, selected);
            }
        }
        Value::Array(children) => {
            for child in children {
                styles(child, selected);
            }
        }
        _ => {}
    }
}

#[test]
fn source_relations_keep_unselected_text_and_exact_marker_facts() {
    for (body, expected, plain) in [
        (
            "地面の価格やのれんのねうち［＃「のれん」と「ねうち」に傍点］後",
            vec!["のれん", "ねうち"],
            "地面の価格やのれんのねうち後",
        ),
        (
            "法文・法律［＃「法律」の「法」に傍点］後",
            vec!["法"],
            "法文・法律後",
        ),
        (
            "生、息、行、意気［＃「生」「息」「行」「意気」に傍点］後",
            vec!["生", "息", "行", "意気"],
            "生、息、行、意気後",
        ),
        // The operand quotes what to exclude, and every occurrence of it is
        // excluded rather than the first alone.
        (
            "行・実践［＃「行・実践」の「・」を除く部分に傍点］後",
            vec!["行", "実践"],
            "行・実践後",
        ),
    ] {
        let source = format!("題\n著者\n\n{body}\n\n底本：本\n");
        let aat: Value =
            serde_json::from_slice(&ab_aat::aat_json_from_bytes(source.as_bytes()).unwrap())
                .unwrap();
        let mut selected = Vec::new();
        styles(&aat["blocks"], &mut selected);
        assert_eq!(
            selected
                .iter()
                .map(|node| visible(node))
                .collect::<Vec<_>>(),
            expected,
            "{aat:#}"
        );
        let paragraphs = aat["blocks"].as_array().unwrap();
        assert!(
            paragraphs.iter().any(|p| visible(p).trim() == plain),
            "{aat:#}"
        );
        let marker_start = source.find("［＃").unwrap();
        let marker_end =
            source[marker_start..].find('］').unwrap() + marker_start + '］'.len_utf8();
        for node in selected {
            let markers = node["interpretation_marker_spans"].as_array().unwrap();
            assert_eq!(markers.len(), 1);
            assert_eq!(markers[0]["byte_start"], marker_start);
            assert_eq!(markers[0]["byte_end"], marker_end);
        }
    }
}

#[test]
fn repeated_disjoint_operands_select_the_nearest_ordered_ranges() {
    let source = "生息行意気。生、息、行、意気［＃「生」「息」「行」「意気」に傍点］後";
    let aat: Value =
        serde_json::from_slice(&ab_aat::aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let mut selected = Vec::new();
    styles(&aat["blocks"], &mut selected);

    assert_eq!(
        selected
            .iter()
            .map(|node| visible(node))
            .collect::<Vec<_>>(),
        ["生", "息", "行", "意気"]
    );
    assert!(aat["meta"]["warnings"].as_array().unwrap().is_empty());
    assert_eq!(visible(&aat["blocks"][0]), "生息行意気。生、息、行、意気後");
}

#[test]
fn resolved_rich_targets_do_not_retain_decline_warnings() {
    for source in [
        "扨、私事《わたくしこと》、［＃「扨、私事、」は太字］",
        "に錯けば。に錯《お》けば［＃「に錯けば」は太字］",
        "全集の全集［＃「全集の全集」は２段階大きな文字］［＃「全集の全集」に傍線］",
    ] {
        let aat: Value =
            serde_json::from_slice(&ab_aat::aat_json_from_bytes(source.as_bytes()).unwrap())
                .unwrap();
        assert!(
            aat["meta"]["warnings"].as_array().unwrap().is_empty(),
            "{aat}"
        );
        let diagnostics: Value = serde_json::from_slice(
            &ab_aat::diagnostics_json_from_bytes(source.as_bytes()).unwrap(),
        )
        .unwrap();
        assert_eq!(diagnostics["data"], json!([]), "{diagnostics}");
        assert!(aat.to_string().contains("\"kind\":\"style\""), "{aat}");
    }
}

#[test]
fn unresolved_rich_target_retains_decline_diagnostic() {
    let source = "前［＃「別の語」は太字］";
    let diagnostics: Value =
        serde_json::from_slice(&ab_aat::diagnostics_json_from_bytes(source.as_bytes()).unwrap())
            .unwrap();

    assert_eq!(
        diagnostics["data"][0]["code"],
        "forward-referent-not-stylable"
    );
}

#[test]
fn plural_italic_operands_decorate_separate_source_ranges() {
    let marker = "［＃「animus」、「mens」の部分はイタリック体］";
    let source = format!("心（mind, animus or mens{marker}）後");
    let aat: Value =
        serde_json::from_slice(&ab_aat::aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let content = aat["blocks"][0]["content"].as_array().unwrap();
    let selected: Vec<_> = content.iter().filter(|n| n["kind"] == "style").collect();
    assert_eq!(
        selected.iter().map(|n| visible(n)).collect::<Vec<_>>(),
        ["animus", "mens"]
    );
    assert_eq!(visible(&aat["blocks"][0]), "心（mind, animus or mens）後");
    assert!(content.iter().all(|n| n["kind"] != "gaiji"));
    for (node, target) in selected.into_iter().zip(["animus", "mens"]) {
        assert_eq!(node["style_type"], "italic");
        assert_eq!(node["span"]["byte_start"], source.find(target).unwrap());
        assert_eq!(
            node["span"]["byte_end"],
            source.find(target).unwrap() + target.len()
        );
        let markers = node["interpretation_marker_spans"].as_array().unwrap();
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0]["byte_start"], source.find(marker).unwrap());
        assert_eq!(
            markers[0]["byte_end"],
            source.find(marker).unwrap() + marker.len()
        );
    }
    assert_eq!(
        ab_aozora_facade::Document::new(source.as_str())
            .parse()
            .to_source(),
        source
    );
}

#[test]
fn unresolved_plural_italic_targets_remain_annotation_without_partial_styling() {
    for principal in [
        "animus only",
        "animus animus or mens",
        "mens or animus",
        "animus\nor mens",
    ] {
        let source = format!("{principal}［＃「animus」、「mens」の部分はイタリック体］");
        let aat: Value =
            serde_json::from_slice(&ab_aat::aat_json_from_bytes(source.as_bytes()).unwrap())
                .unwrap();
        for block in aat["blocks"].as_array().unwrap() {
            for node in block["content"].as_array().unwrap() {
                assert_ne!(node["kind"], "style", "{source}: {aat}");
                assert_ne!(node["kind"], "gaiji", "{source}: {aat}");
            }
        }
        assert_eq!(
            ab_aozora_facade::Document::new(source.as_str())
                .parse()
                .to_source(),
            source
        );
    }
}

#[test]
fn inconsistent_compound_instructions_remain_apparatus_with_exact_marker_span() {
    let marker = "［＃「x2」、「y2」、「x2」はそれぞれ縦中横、すべての「2」は上付き小書き］";
    let source = format!("x2＋y2＝r2{marker}");
    let aat: Value =
        serde_json::from_slice(&ab_aat::aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let content = aat["blocks"][0]["content"].as_array().unwrap();
    assert_eq!(visible(&aat["blocks"][0]), "x2＋y2＝r2");
    assert!(content.iter().all(|node| node["kind"] != "gaiji"));
    let annotation = content
        .iter()
        .find(|node| node["source"] == marker)
        .unwrap();
    assert_eq!(annotation["kind"], "formatting_instruction");
    assert!(annotation.get("interpretation_problem").is_some());
    assert_eq!(
        annotation["span"]["byte_start"],
        source.find(marker).unwrap()
    );
    assert_eq!(annotation["span"]["byte_end"], source.len());
    assert!(aat["meta"].get("interpretation_facts").is_none());
}

#[test]
fn native_typography_roles_do_not_establish_unresolved_interpretations() {
    for source in [
        "missing［＃「B1」は縦中横、「1」は下付き小文字］",
        "B11［＃「B11」は縦中横、「1」は下付き小文字］",
        "q-r2/4Dt［＃「2」は上付き小文字、「r2/4Dt」は分数、「-r2/4Dt」は「e」の上付き］",
    ] {
        let marker_start = source.find("［＃").unwrap();
        let native = ab_aozora_pipeline::lex(source);
        let fact = native
            .classified_source_facts
            .iter()
            .find(|fact| fact.source_span.start as usize == marker_start)
            .unwrap();
        assert_eq!(
            fact.source_role,
            ab_aozora_pipeline::ClassifiedSourceRole::Typography
        );
        assert!(fact.is_semantic());
        let aat: Value =
            serde_json::from_slice(&ab_aat::aat_json_from_bytes(source.as_bytes()).unwrap())
                .unwrap();
        let instruction = aat["blocks"][0]["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["kind"] == "formatting_instruction")
            .unwrap();
        assert_eq!(
            instruction["interpretation_problem"]["kind"],
            "uninterpreted-notation"
        );
        assert_eq!(instruction["span"]["byte_start"], marker_start);
        assert_eq!(instruction["span"]["byte_end"], source.len());
        assert!(
            aat["meta"]["interpretation_facts"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|fact| fact["source_span"]["start"] != marker_start)
        );
    }
}
