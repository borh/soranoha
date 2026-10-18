//! Source-selected annotation targets retain rich transcription independently of spelling.

use ab_aat::aat_json_from_bytes;
use serde_json::Value;

fn aat(source: &str) -> Value {
    serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap()
}

fn first_annotated_text(aat: &Value) -> &Value {
    aat["blocks"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["kind"] == "annotated_text")
        .unwrap()
}

fn span(byte_start: usize, byte_end: usize) -> Value {
    serde_json::json!({
        "byte_start": byte_start,
        "byte_end": byte_end,
        "line_start": 1,
        "line_end": 1,
    })
}

#[test]
fn unquoted_mama_note_is_an_attached_gloss() {
    let prefix = "いり";
    let marker = "［＃「り」にママの注記］";
    let source = format!("{prefix}{marker}かねる");
    let aat = aat(&source);
    let note = first_annotated_text(&aat);

    assert_eq!(note["note_kind"], "gloss");
    assert_eq!(note["content"][0]["value"], "り");
    assert_eq!(note["annotation_content"][0]["value"], "ママ");
    assert_eq!(
        note["span"],
        span(prefix.len(), prefix.len() + marker.len())
    );
    assert_eq!(note["content"][0]["span"], span("い".len(), prefix.len()));
    assert_eq!(
        note["annotation_content"][0]["span"],
        span(
            prefix.len() + "［＃「り」に".len(),
            prefix.len() + "［＃「り」にママ".len()
        )
    );
    assert_eq!(aat["meta"]["warnings"], serde_json::json!([]));
    assert_eq!(
        ab_aozora_facade::Document::new(source.as_str())
            .parse()
            .to_source(),
        "いり［＃「り」に「ママ」の注記］かねる"
    );
}

#[test]
fn attested_note_keyword_spellings_are_attached_glosses() {
    for (marker, note) in [
        ("［＃「し」に「本ノマヽ」と注記］", "本ノマヽ"),
        ("［＃「し」に「（ママ）」の注記がある］", "（ママ）"),
        ("［＃「し」に「ママ」の注］", "ママ"),
        ("［＃「し」に「ママ」の表記］", "ママ"),
        ("［＃「し」に〔ママ〕と傍書］", "〔ママ〕"),
        ("［＃「し」に〔ママ〕の注記］", "〔ママ〕"),
    ] {
        let aat = aat(&format!("かし{marker}ら"));
        let annotated = first_annotated_text(&aat);

        assert_eq!(annotated["note_kind"], "gloss", "{marker}");
        assert_eq!(annotated["content"][0]["value"], "し", "{marker}");
        assert_eq!(
            annotated["annotation_content"][0]["value"], note,
            "{marker}"
        );
    }
}

#[test]
fn named_witness_reading_preserves_gaiji_markup() {
    let source = "海蓴（こも）の柄［＃「海蓴（こも）の柄」は本文の書き下し文では「海※［＃「くさかんむり／溥のつくり」、U+84AA、59-本文-4］（こも）の柄」］";
    let aat = aat(source);
    let apparatus = aat["blocks"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["kind"] == "text-apparatus")
        .unwrap();

    assert_eq!(apparatus["content"][0]["value"], "海蓴（こも）の柄");
    assert_eq!(
        apparatus["readings"][0]["witnesses"][0]["label"],
        "本文の書き下し文"
    );
    assert_eq!(apparatus["readings"][0]["content"][1]["kind"], "gaiji");
    assert_eq!(apparatus["readings"][0]["content"][1]["resolved"], "蒪");
    assert_eq!(aat["meta"]["warnings"], serde_json::json!([]));
}

#[test]
fn marginal_critical_note_keeps_role_and_placement() {
    let source = "仰願寺［＃「仰願寺」に欄外に校注、「小さき一種のろうそく江戸山谷の仰願寺にて用いはじめしより云う」］";
    let aat = aat(source);
    let note = first_annotated_text(&aat);

    assert_eq!(note["note_kind"], "critical-note");
    assert_eq!(note["position"], "margin");
    assert_eq!(note["content"][0]["value"], "仰願寺");
    assert_eq!(
        note["annotation_content"][0]["value"],
        "小さき一種のろうそく江戸山谷の仰願寺にて用いはじめしより云う"
    );
    assert_eq!(
        ab_aozora_facade::Document::new(source).parse().to_source(),
        source
    );
}

#[test]
fn paired_annotation_records_both_markers_and_exact_target() {
    let open = "［＃注記付き］";
    let target = "病起";
    let close = "［＃「（ママ）」の注記付き終わり］";
    let source = format!("{open}{target}{close}");
    let aat = aat(&source);
    let note = first_annotated_text(&aat);

    assert_eq!(note["span"], span(0, source.len()));
    assert_eq!(
        note["interpretation_marker_spans"],
        serde_json::json!([
            span(0, open.len()),
            span(open.len() + target.len(), source.len()),
        ])
    );
    assert_eq!(note["content"][0]["kind"], "text");
    assert_eq!(note["content"][0]["value"], "病起");
    assert_eq!(
        note["content"][0]["span"],
        span(open.len(), open.len() + target.len())
    );
    assert_eq!(note["annotation_content"][0]["value"], "（ママ）");
    assert!(
        aat["blocks"][0]["content"]
            .as_array()
            .unwrap()
            .iter()
            .all(|node| node["kind"] != "raw")
    );

    let document = ab_aozora_facade::Document::new(source.as_str());
    let tree = document.parse();
    assert_eq!(tree.to_source(), source);
}

#[test]
fn paired_annotations_preserve_structured_targets_and_notes() {
    for (source, target_kind, note_kind, position) in [
        (
            "［＃注記付き］碧梧桐［＃「碧梧桐」は罫囲み］［＃「何とかいう俳画家」の注記付き終わり］",
            "keigakomi",
            "text",
            None,
        ),
        (
            "［＃注記付き］12［＃「12」は縦中横］月［＃「日付」の注記付き終わり］",
            "tcy",
            "text",
            None,
        ),
        (
            "［＃左に注記付き］病起《びょうき》［＃左に「（ママ）」の注記付き終わり］",
            "ruby",
            "text",
            Some("left"),
        ),
        (
            "［＃注記付き］愈尼剌太喇立［＃「ユニラテ※［＃半濁点付き片仮名ラ、169-5］ル」の注記付き終わり］",
            "text",
            "gaiji",
            None,
        ),
    ] {
        let aat = aat(source);
        let note = first_annotated_text(&aat);
        assert!(
            note["content"]
                .as_array()
                .unwrap()
                .iter()
                .any(|node| node["kind"] == target_kind),
            "{source}: {note:#}"
        );
        assert!(
            note["annotation_content"]
                .as_array()
                .unwrap()
                .iter()
                .any(|node| node["kind"] == note_kind),
            "{source}: {note:#}"
        );
        assert_eq!(note.get("position").and_then(Value::as_str), position);
        assert_eq!(
            note["interpretation_marker_spans"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(
            aat["blocks"][0]["content"]
                .as_array()
                .unwrap()
                .iter()
                .all(|node| node["kind"] != "raw"),
            "{source}"
        );
        let document = ab_aozora_facade::Document::new(source);
        let tree = document.parse();
        let canonical = tree.to_source();
        let close_start = source
            .rfind("［＃左に「")
            .or_else(|| source.rfind("［＃「"))
            .unwrap();
        assert!(canonical.ends_with(&source[close_start..]), "{canonical}");
        assert_eq!(
            tree.to_html(),
            ab_aozora_facade::Document::new(canonical.as_str())
                .parse()
                .to_html()
        );
    }
}

#[test]
fn incomplete_or_ambiguous_annotation_pairs_stay_raw() {
    for source in [
        "前［＃注記付き］後",
        "前［＃「注」の注記付き終わり］後",
        "［＃左に注記付き］本文［＃「注」の注記付き終わり］",
        "［＃注記付き］前\n後［＃「注」の注記付き終わり］",
        "［＃注記付き］外［＃左に注記付き］内［＃「右」の注記付き終わり］［＃左に「左」の注記付き終わり］",
    ] {
        let aat = aat(source);
        let content = aat["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|block| block["content"].as_array().unwrap())
            .collect::<Vec<_>>();
        assert!(
            content.iter().all(|node| node["kind"] != "annotated_text"),
            "{source}"
        );
        assert!(content.iter().any(|node| node["kind"] == "raw"), "{source}");
        assert_eq!(
            ab_aozora_facade::Document::new(source).parse().to_source(),
            source
        );
    }
}

#[test]
fn editorial_gloss_keeps_edition_provenance_and_original_ruby() {
    let statement =
        "校注、「枕橋の架してある堀の奥のところ」、ただし底本では校注が脱落、底本の親本にて確認";
    let source = format!("本所｜〆切《しめきり》［＃「〆切」に{statement}］後");
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let content = aat["blocks"][0]["content"].as_array().unwrap();
    let note = content
        .iter()
        .find(|node| node["kind"] == "annotated_text")
        .unwrap();
    let ruby = &note["content"][0];
    assert_eq!(ruby["kind"], "ruby");
    assert_eq!(ruby["base"], "〆切");
    assert_eq!(ruby["reading"], "しめきり");
    assert_eq!(note["note_kind"], "gloss");
    assert_eq!(note["annotation_content"][0]["value"], statement);
    assert!(note.get("position").is_none());
    let fact = aat["meta"]["interpretation_facts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|fact| fact["kind"] == "annotated-text")
        .unwrap();
    assert_eq!(fact["aspects"], serde_json::json!(["content", "structure"]));
    assert!(content.iter().all(|node| node["kind"] != "raw"));
    let document = ab_aozora_facade::Document::new(source.as_str());
    let tree = document.parse();
    assert_eq!(tree.source(), source);
    let canonical = tree.to_source();
    assert_eq!(
        tree.to_html(),
        ab_aozora_facade::Document::new(canonical.as_str())
            .parse()
            .to_html()
    );
}

#[test]
fn editorial_gloss_does_not_guess_target_or_provenance() {
    for (base, statement) in [
        (
            "締切",
            "校注、「説明」、ただし底本では校注が脱落、底本の親本にて確認",
        ),
        ("〆切", "校注、「説明」、ただし底本の状態は不明"),
        (
            "〆切",
            "校注、「」、ただし底本では校注が脱落、底本の親本にて確認",
        ),
    ] {
        let source = format!("{base}《しめきり》［＃「〆切」に{statement}］");
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        assert!(
            aat["blocks"][0]["content"]
                .as_array()
                .unwrap()
                .iter()
                .any(|node| node["kind"] == "raw")
        );
    }
}

#[test]
fn adjacent_source_targets_retain_rich_principal_content() {
    for (target, child_kind) in [
        ("」", "text"),
        ("どふ／＼", "iteration-mark"),
        ("※［＃濁点付き片仮名ヱ、1-7-84］", "gaiji"),
        ("※［＃「飮のへん＋旨」、341-5］", "gaiji"),
        ("白《タク》衾", "ruby"),
    ] {
        let marker = format!("［＃「{target}」に「マヽ」の注記］");
        let source = format!("前、{target}{marker}後");
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let content = aat["blocks"][0]["content"].as_array().unwrap();
        let note = content
            .iter()
            .find(|node| node["kind"] == "annotated_text")
            .unwrap_or_else(|| panic!("source target was not attached: {source}"));
        assert!(
            note["content"]
                .as_array()
                .unwrap()
                .iter()
                .any(|node| node["kind"] == child_kind)
        );
        assert_eq!(note["span"]["byte_start"], "前、".len() + target.len());
        assert_eq!(
            note["span"]["byte_end"],
            "前、".len() + target.len() + marker.len()
        );
        assert_eq!(content.first().unwrap()["value"], "前、");
        assert_eq!(content.last().unwrap()["value"], "後");
    }
}

#[test]
fn different_source_target_is_not_replaced_by_a_visible_match() {
    for (base, marker) in [
        ("剌", "［＃「刺」の左に「テフダ」の注記］"),
        ("）", "［＃「」」に「ママ」の注記］"),
        ("」", "［＃「」に「ママ」の注記］"),
    ] {
        let source = format!("前{base}{marker}後");
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let content = aat["blocks"][0]["content"].as_array().unwrap();
        assert!(content.iter().all(|node| node["kind"] != "annotated_text"));
        assert!(
            content
                .iter()
                .any(|node| node["kind"] == "raw" && node["source"] == marker)
        );
    }
}

#[test]
fn annotations_inside_ruby_bases_keep_the_full_reading_association() {
    for (base, target, reading) in [
        ("菌毒", "菌", "きんどく"),
        ("其駁雑", "駁雑", "そのはくざつ"),
        ("安然", "然", "あんぜん"),
    ] {
        let source = format!("前、{base}《{reading}》［＃「{target}」の左に「注」の注記］後");
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let content = aat["blocks"][0]["content"].as_array().unwrap();
        let ruby = content.iter().find(|node| node["kind"] == "ruby").unwrap();
        assert_eq!(ruby["base"], base);
        assert_eq!(ruby["reading"], reading);
        let note = ruby["base_content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["kind"] == "annotated_text")
            .unwrap();
        assert_eq!(note["content"][0]["value"], target);
        assert_eq!(note["position"], "left");
        let fact = aat["meta"]["interpretation_facts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|fact| fact["kind"] == "annotated-text")
            .unwrap();
        assert_eq!(
            fact["aspects"],
            serde_json::json!(["content", "structure", "layout"])
        );
        assert!(content.iter().all(|node| node["kind"] != "raw"));
    }
}

#[test]
fn annotation_target_can_join_plain_prefix_to_explicit_ruby_base() {
    let source = "天の配｜剤《ザイ》［＃「天の配剤」の左に「いかにもむずかしいハイザイね」の注記］はよろしきを得て居ります。";
    let aat = aat(source);
    let note = first_annotated_text(&aat);
    assert_eq!(note["position"], "left");
    assert_eq!(note["content"][0]["value"], "天の配");
    assert_eq!(note["content"][1]["kind"], "ruby");
    assert_eq!(note["content"][1]["base"], "剤");
    assert_eq!(note["content"][1]["reading"], "ザイ");
    assert_eq!(
        note["annotation_content"][0]["value"],
        "いかにもむずかしいハイザイね"
    );
    assert!(
        aat["blocks"][0]["content"]
            .as_array()
            .unwrap()
            .iter()
            .all(|node| node["kind"] != "raw")
    );
}

#[test]
fn annotation_target_can_cross_a_tcy_marker() {
    let source = "日ソ文化協会 広順社12［＃「12」は縦中横］時［＃「広順社12時」に「（欠席メッセージ）」の注記］より";
    let aat = aat(source);
    let note = first_annotated_text(&aat);
    assert_eq!(note["note_kind"], "gloss");
    assert_eq!(note["content"][0]["value"], "広順社");
    assert_eq!(note["content"][1]["kind"], "tcy");
    assert_eq!(note["content"][1]["content"][0]["value"], "12");
    assert_eq!(note["content"][2]["value"], "時");
    assert_eq!(note["annotation_content"][0]["value"], "（欠席メッセージ）");
    let content = aat["blocks"][0]["content"].as_array().unwrap();
    assert_eq!(content[0]["value"], "日ソ文化協会 ");
    assert!(content.iter().all(|node| node["kind"] != "raw"));
}

#[test]
fn ambiguous_ruby_base_target_stays_unresolved() {
    let source = "菌菌《きんきん》［＃「菌」の左に「注」の注記］";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let content = aat["blocks"][0]["content"].as_array().unwrap();
    assert!(content.iter().any(|node| node["kind"] == "raw"));
    assert!(content.iter().all(|node| node["kind"] != "annotated_text"));
}

#[test]
fn supplied_note_roles_keep_each_operand_occurrence_independent() {
    for (target, marker, role, word) in [
        (
            "（１）",
            "［＃「（１）」は注釈番号］",
            "annotation-number",
            "注釈番号",
        ),
        ("（一）", "［＃（一）は自注］", "author-note", "自注"),
        ("（一）", "［＃「（一）」は自注］", "author-note", "自注"),
    ] {
        let source = format!("前{target}{marker}中{target}{marker}後");
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let content = aat["blocks"][0]["content"].as_array().unwrap();
        let notes = content
            .iter()
            .filter(|node| node["kind"] == "annotated_text")
            .collect::<Vec<_>>();
        assert_eq!(notes.len(), 2, "{source}");
        for note in &notes {
            assert_eq!(note["note_kind"], role);
            assert_eq!(note["content"][0]["value"], target);
            assert_eq!(note["annotation_content"][0]["value"], word);
            assert!(note.get("position").is_none());
            let span = &note["span"];
            let start = usize::try_from(span["byte_start"].as_u64().unwrap()).unwrap();
            let end = usize::try_from(span["byte_end"].as_u64().unwrap()).unwrap();
            assert_eq!(&source[start..end], marker);
        }
        assert_ne!(notes[0]["span"], notes[1]["span"]);
        for fact in aat["meta"]["interpretation_facts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|fact| fact["kind"] == "annotated-text")
        {
            assert_eq!(fact["aspects"], serde_json::json!(["content", "structure"]));
        }
    }
}

#[test]
fn attached_author_notes_preserve_spacing_and_rich_targets() {
    let source =
        "Ongo ［＃「Ongo」に「＊」の著者註］と御｜正忌《しやうき》［＃「御正忌」に「＊」の著者註］";
    let aat = aat(source);
    let content = aat["blocks"][0]["content"].as_array().unwrap();
    let notes = content
        .iter()
        .filter(|node| node["kind"] == "annotated_text")
        .collect::<Vec<_>>();
    assert_eq!(notes.len(), 2, "{aat}");
    assert_eq!(notes[0]["note_kind"], "author-note");
    assert_eq!(notes[0]["content"][0]["value"], "Ongo");
    assert_eq!(notes[0]["annotation_content"][0]["value"], "＊");
    assert_eq!(notes[1]["content"][0]["value"], "御");
    assert_eq!(notes[1]["content"][1]["kind"], "ruby");
    assert_eq!(notes[1]["content"][1]["base"], "正忌");
    assert_eq!(notes[1]["content"][1]["reading"], "しやうき");
    assert_eq!(content[1]["value"], " ");
    assert_eq!(content[2]["value"], "と");
}
