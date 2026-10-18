//! Source-edition statements retain their wording and annotation boundaries.
use ab_aat::aat_json_from_bytes;
use serde_json::Value;

#[test]
fn character_descriptions_are_not_edition_statements() {
    for marker in [
        "［＃「浄」の「さんずい」の代わりに「めへん」、37-3］",
        "［＃「※」は「題の頁の代わりに韋」、第4水準2-92-15、182-13］",
        "※［＃「底本は」、1-13-21］",
    ] {
        let source = format!("前{marker}後");
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let content = aat["blocks"][0]["content"].as_array().unwrap();
        assert_eq!(
            content.iter().filter(|n| n["kind"] == "gaiji").count(),
            1,
            "{aat}"
        );
        assert!(content.iter().all(|n| n["kind"] != "editorial_note"));
        assert_eq!(
            ab_aozora_facade::Document::new(source.as_str())
                .parse()
                .to_source(),
            source
        );
    }
}

#[test]
fn edition_statements_with_locations_are_notes_without_invented_glyphs() {
    for (principal, statement) in [
        ("一抱え", "底本は「一抱へ」、32-10"),
        ("せいたか", "底本は「せいだか」、34-9"),
        ("で", "「で」は底本どおり、岩波文庫版では「ど」、151-11"),
        (
            "セプルクルム",
            "底本7-10、63-13、69-6ではセブルクルムと表記。ここ30-10での表記は誤りか",
        ),
    ] {
        for newline in ["\n", "\r\n"] {
            let marker = format!("［＃{statement}］");
            let source = format!("{principal}{marker}後{newline}");
            let aat: Value =
                serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
            let content = aat["blocks"][0]["content"].as_array().unwrap();
            let note = content
                .iter()
                .find(|n| n["kind"] == "editorial_note")
                .expect("edition note");
            assert_eq!(note["note_kind"], "base-edition");
            assert_eq!(note["text"], statement);
            assert_eq!(note["span"]["byte_start"], principal.len());
            assert_eq!(note["span"]["byte_end"], principal.len() + marker.len());
            assert!(content.iter().all(|n| n["kind"] != "gaiji"));
            let visible: String = content
                .iter()
                .filter(|n| n["kind"] == "text")
                .filter_map(|n| n["value"].as_str())
                .collect();
            assert_eq!(visible.trim_end(), format!("{principal}後"));
            assert_eq!(
                ab_aozora_facade::Document::new(source.as_str())
                    .parse()
                    .to_source(),
                source.replace("\r\n", "\n")
            );
        }
    }
}

#[test]
fn quoted_closing_bracket_does_not_move_annotation_tail_into_prose() {
    let marker = "［＃「〕」は底本では「］」］";
    let source = format!("前〕{marker}後");
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    assert_eq!(aat["blocks"][0]["content"][2]["value"], "後", "{aat}");
    let variant = &aat["blocks"][0]["content"][1];
    assert_eq!(variant["kind"], "text-variant", "{aat}");
    assert_eq!(variant["source"], marker);
    assert_eq!(variant["base_text"], "］");
    assert_eq!(variant["content"][0]["value"], "〕");
    assert_eq!(
        variant["span"]["byte_end"],
        source.len() - "後".len(),
        "{aat}"
    );
}

#[test]
fn explicit_base_correction_binds_the_supplied_current_reading() {
    for source in [
        "生まれでた［＃底本「生まれてた」を「生まれでた」に訂正］のではない",
        "底流［＃底本の「低流」を「底流」に訂正］になっている",
    ] {
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let content = aat["blocks"][0]["content"].as_array().unwrap();
        let variant = content
            .iter()
            .find(|node| node["kind"] == "text-variant")
            .expect("base-edition correction apparatus");
        let expected_current = if source.starts_with("底流") {
            "底流"
        } else {
            "生まれでた"
        };
        let expected_base = if source.starts_with("底流") {
            "低流"
        } else {
            "生まれてた"
        };
        assert_eq!(variant["witness_kind"], "base-edition", "{aat}");
        assert_eq!(variant["base_text"], expected_base, "{aat}");
        assert_eq!(variant["content"][0]["value"], expected_current, "{aat}");
        assert!(content.iter().any(|node| {
            node["kind"] == "editorial_note" && node["note_kind"] == "base-edition"
        }));
        assert_eq!(
            ab_aozora_facade::Document::new(source).parse().to_source(),
            source
        );
    }
}

#[test]
fn implicit_base_correction_retains_only_the_supplied_base_reading() {
    for (source, base) in [
        ("こゝろを［＃底本「お」を訂正］どらす", "お"),
        ("底流［＃底本の「低流」を訂正］になっている", "低流"),
    ] {
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let content = aat["blocks"][0]["content"].as_array().unwrap();
        let note = content
            .iter()
            .find(|node| node["kind"] == "editorial_note")
            .expect("base-edition correction note");
        assert_eq!(note["note_kind"], "base-edition", "{aat}");
        assert_eq!(note["base_text"], base, "{aat}");
        let start =
            usize::try_from(note["base_source_span"]["byte_start"].as_u64().unwrap()).unwrap();
        let end = usize::try_from(note["base_source_span"]["byte_end"].as_u64().unwrap()).unwrap();
        assert_eq!(&source[start..end], base);
        assert!(content.iter().all(|node| node["kind"] != "text-variant"));
        assert_eq!(
            ab_aozora_facade::Document::new(source).parse().to_source(),
            source
        );
    }
}

#[test]
fn original_text_errors_are_apparatus_with_explicit_witness_authority() {
    for (source, current, witness, locator) in [
        (
            "音がしました。［＃「しました。」は原文では「しました、」と誤記、136-2］後",
            "しました。",
            "しました、",
            "136-2",
        ),
        (
            "驚きました。「［＃【「】は原文では【」】と誤記、43-10］なあんだ",
            "「",
            "」",
            "43-10",
        ),
    ] {
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let content = aat["blocks"][0]["content"].as_array().unwrap();
        let variant = content
            .iter()
            .find(|node| node["kind"] == "text-variant")
            .expect("original-text apparatus");
        assert_eq!(variant["witness_kind"], "original-text", "{aat}");
        assert_eq!(variant["base_text"], witness);
        assert_eq!(variant["content"][0]["value"], current);
        let note = content
            .iter()
            .find(|node| node["kind"] == "editorial_note")
            .expect("source-error statement");
        assert_eq!(note["note_kind"], "original-text");
        assert_eq!(note["source_locator"], locator);
        assert!(content.iter().all(|node| node["kind"] != "gaiji"));
        assert_eq!(
            ab_aozora_facade::Document::new(source).parse().to_source(),
            source
        );
    }
}

#[test]
fn base_edition_register_locators_remain_apparatus() {
    for locator in ["172-上段-8", "172-下段-2"] {
        let source = format!("松ヶ谷学士［＃「学士」は底本では「博士」、{locator}］は下を向いた");
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let content = aat["blocks"][0]["content"].as_array().unwrap();
        let variant = content
            .iter()
            .find(|node| node["kind"] == "text-variant")
            .expect("base-edition apparatus");
        assert_eq!(variant["content"][0]["value"], "学士");
        assert_eq!(variant["base_text"], "博士");
        let note = content
            .iter()
            .find(|node| node["kind"] == "editorial_note")
            .expect("base-edition statement");
        assert_eq!(note["note_kind"], "base-edition");
        assert_eq!(note["source_locator"], locator);
    }
}
