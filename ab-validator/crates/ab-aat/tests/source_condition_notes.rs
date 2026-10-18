//! Source-condition notes retain proposed corrections and printed-page locations.

use ab_aat::aat_json_from_bytes;
use serde_json::Value;

fn first_note<'a>(value: &'a Value, note_kind: &str) -> &'a Value {
    fn find<'a>(nodes: &'a [Value], note_kind: &str) -> Option<&'a Value> {
        nodes.iter().find_map(|node| {
            if node["kind"] == "editorial_note" && node["note_kind"] == note_kind {
                return Some(node);
            }
            ["content", "children"]
                .into_iter()
                .filter_map(|key| node[key].as_array())
                .find_map(|children| find(children, note_kind))
        })
    }
    find(value["blocks"].as_array().unwrap(), note_kind)
        .unwrap_or_else(|| panic!("missing {note_kind}: {value}"))
}

#[test]
fn questioned_correction_targets_the_preceding_ruby_base() {
    let marker = "［＃「霙」の誤り？、400-7］";
    let source = format!("題\n作者\n\n霰《みぞれ》{marker}の音\n\n底本：本\n");
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = first_note(&aat, "uncertain-correction");
    let target = source.find('霰').unwrap();
    let proposed = source.find('霙').unwrap();
    assert_eq!(note["proposed_text"], "霙");
    assert_eq!(note["certainty"], "uncertain");
    assert_eq!(note["source_locator"], "400-7");
    assert_eq!(
        note["target_source_spans"][0]["byte_start"], target,
        "{aat}"
    );
    assert_eq!(
        note["target_source_spans"][0]["byte_end"],
        target + '霰'.len_utf8()
    );
    assert_eq!(note["proposed_source_span"]["byte_start"], proposed);
    assert_eq!(
        note["proposed_source_span"]["byte_end"],
        proposed + '霙'.len_utf8()
    );
    let ruby = &aat["blocks"][0]["content"][0];
    assert_eq!(ruby["kind"], "ruby");
    assert_eq!(ruby["base"], "霰");
    assert_eq!(ruby["reading"], "みぞれ");
    assert!(!aat.to_string().contains("unresolved-gaiji"), "{aat}");
}

#[test]
fn named_questioned_correction_preserves_both_structured_readings() {
    let marker = "［＃「勧《つと》めた」は「勤《つと》めた」の誤記か］";
    let current = "勧《つと》めた";
    let proposed = "勤《つと》めた";
    let source = format!("題\n作者\n\n{current}{marker}後\n\n底本：本\n");
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = first_note(&aat, "uncertain-correction");
    let target_start = source.find(current).unwrap();
    let proposed_start = source.find(proposed).unwrap();

    assert_eq!(note["target_text"], "勧めた");
    assert_eq!(note["proposed_text"], "勤めた");
    assert_eq!(note["certainty"], "uncertain");
    assert!(note.get("source_locator").is_none());
    assert_eq!(note["target_source_spans"][0]["byte_start"], target_start);
    assert_eq!(
        note["target_source_spans"][0]["byte_end"],
        target_start + current.len()
    );
    assert_eq!(note["proposed_source_span"]["byte_start"], proposed_start);
    assert_eq!(
        note["proposed_source_span"]["byte_end"],
        proposed_start + proposed.len()
    );
    assert_eq!(note["proposed_content"][0]["kind"], "ruby");
    assert_eq!(note["proposed_content"][0]["base"], "勤");
    assert_eq!(note["proposed_content"][0]["reading"], "つと");
    assert_eq!(note["proposed_content"][1]["value"], "めた");
}

#[test]
fn unclear_printing_is_a_located_source_note() {
    let source = "題\n作者\n\nあたしまつぴらだわ［＃印刷不鮮明、87-14］。\n\n底本：本\n";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = first_note(&aat, "source-legibility");
    assert_eq!(note["text"], "印刷不鮮明、87-14");
    assert_eq!(note["source_locator"], "87-14");
    assert!(note.get("target_source_spans").is_none());
    assert!(!aat.to_string().contains("unresolved-gaiji"), "{aat}");
}

#[test]
fn quoted_sic_note_targets_the_nearest_preceding_text() {
    let source = "題\n作者\n\nein Erganen［＃「Erganen」はママ］ und Erganen\n\n底本：本\n";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = first_note(&aat, "sic");
    let target = source.find("Erganen").unwrap();
    assert_eq!(note["target_text"], "Erganen");
    assert_eq!(
        note["target_source_spans"],
        serde_json::json!([{"byte_start": target, "byte_end": target + 7, "line_start": 4, "line_end": 4}]),
        "{aat}"
    );
}

#[test]
fn ruby_reading_sic_note_targets_the_reading() {
    let source = "題\n作者\n\n窓《まど》［＃ルビの「まど」はママ］。\n\n底本：本\n";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = first_note(&aat, "sic");
    let reading = source.find("まど").unwrap();
    assert_eq!(note["target_text"], "まど");
    assert_eq!(
        note["target_source_spans"][0]["byte_start"], reading,
        "{aat}"
    );
    assert_eq!(
        note["target_source_spans"][0]["byte_end"],
        reading + "まど".len()
    );
}

#[test]
fn ruby_reading_sic_note_quoting_an_iteration_mark_targets_the_reading() {
    let source = "題\n作者\n\n時々《とき／＼》［＃ルビの「とき／＼」はママ］お癪\n\n底本：本\n";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = first_note(&aat, "sic");
    let reading = source.find("とき").unwrap();
    assert_eq!(note["target_text"], "とき〱");
    assert_eq!(
        note["target_source_spans"][0]["byte_start"], reading,
        "{aat}"
    );
    assert_eq!(
        note["target_source_spans"][0]["byte_end"],
        reading + "とき／＼".len()
    );
}

#[test]
fn a_translation_error_note_is_a_sic_note_with_the_stated_correction() {
    let source = "題\n作者\n\n弟［＃「弟」は誤訳で本当は「兄」］のミクロフト\n\n底本：本\n";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = first_note(&aat, "sic");
    let target = source.find('弟').unwrap();
    let proposed = source.find('兄').unwrap();
    assert_eq!(note["target_text"], "弟");
    assert_eq!(note["proposed_text"], "兄");
    assert_eq!(note["target_source_spans"][0]["byte_start"], target);
    assert_eq!(note["proposed_source_span"]["byte_start"], proposed);
    assert!(note.get("certainty").is_none());
}

#[test]
fn a_quoted_sic_note_without_a_preceding_match_stays_raw_as_an_unresolved_variant() {
    let source = "題\n作者\n\n別の語［＃「Erganen」はママ］\n\n底本：本\n";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let raw = aat["blocks"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["kind"] == "raw")
        .unwrap_or_else(|| panic!("missing raw note: {aat}"));
    assert_eq!(raw["source"], "［＃「Erganen」はママ］");
    assert_eq!(raw["interpretation_problem"]["code"], "unresolved-variant");
    assert!(
        aat["blocks"][0]["content"]
            .as_array()
            .unwrap()
            .iter()
            .all(|node| node["note_kind"] != "sic"),
        "{aat}"
    );
}

#[test]
fn a_quoted_sic_target_ending_in_a_ruby_base_excludes_the_reading() {
    let source = "題\n作者\n\n咳をして涕《はな》［＃「涕」はママ］を拭んだ\n\n底本：本\n";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = first_note(&aat, "sic");
    let target = source.find("涕").unwrap();
    assert_eq!(
        note["target_source_spans"],
        serde_json::json!([{"byte_start": target, "byte_end": target + 3, "line_start": 4, "line_end": 4}]),
        "{aat}"
    );
}

#[test]
fn a_quoted_sic_target_spanning_a_ruby_covers_the_ruby_source() {
    let source = "題\n作者\n\n布團で昂布卷《こぶまき》に［＃「昂布卷に」はママ］した\n\n底本：本\n";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = first_note(&aat, "sic");
    let start = source.find("昂布卷").unwrap();
    let end = source.find("［＃「昂布卷に」").unwrap();
    assert_eq!(note["target_text"], "昂布卷に");
    assert_eq!(note["target_source_spans"][0]["byte_start"], start, "{aat}");
    assert_eq!(note["target_source_spans"][0]["byte_end"], end);
}

#[test]
fn a_quoted_sic_target_with_ruby_notation_has_the_visible_target_text() {
    let source = "題\n作者\n\nここに餌《えは》あり［＃「餌《えは》あり」はママ］、チ\n\n底本：本\n";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = first_note(&aat, "sic");
    let start = source.find("餌《えは》あり").unwrap();
    assert_eq!(note["target_text"], "餌あり", "{aat}");
    assert_eq!(note["target_source_spans"][0]["byte_start"], start);
    assert_eq!(
        note["target_source_spans"][0]["byte_end"],
        start + "餌《えは》あり".len()
    );
}

#[test]
fn a_quoted_sic_target_that_is_an_explicit_ruby_base_excludes_the_delimiter() {
    let source = "題\n作者\n\n紀州｜名草群《なぐさぐん》［＃「名草群」はママ］\n\n底本：本\n";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = first_note(&aat, "sic");
    let start = source.find("名草群").unwrap();
    assert_eq!(
        note["target_source_spans"],
        serde_json::json!([{"byte_start": start, "byte_end": start + 9, "line_start": 4, "line_end": 4}]),
        "{aat}"
    );
}

#[test]
fn a_quoted_sic_target_that_is_a_suffix_of_an_implicit_ruby_base_is_the_base_tail() {
    let source = "題\n作者\n\n水いろの染裕衣《そめゆかた》［＃「裕衣」はママ］\n\n底本：本\n";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = first_note(&aat, "sic");
    let start = source.find("裕衣").unwrap();
    assert_eq!(
        note["target_source_spans"],
        serde_json::json!([{"byte_start": start, "byte_end": start + 6, "line_start": 4, "line_end": 4}]),
        "{aat}"
    );
}

#[test]
fn a_quoted_sic_target_ending_a_styled_run_is_the_tail_of_the_styled_text() {
    let source =
        "題\n作者\n\n若井善三部［＃「若井善三部」に傍点］［＃「部」はママ］と\n\n底本：本\n";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = first_note(&aat, "sic");
    let start = source.find("部［").unwrap();
    assert_eq!(
        note["target_source_spans"],
        serde_json::json!([{"byte_start": start, "byte_end": start + 3, "line_start": 4, "line_end": 4}]),
        "{aat}"
    );
}

#[test]
fn a_quoted_sic_target_starting_inside_a_ruby_base_spans_the_base_tail_and_the_text() {
    let source = "題\n作者\n\n木村｜長門《ながと》、薄田隼人生《すすきだはいとのしょう》ら［＃「隼人生ら」はママ］、\n\n底本：本\n";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = first_note(&aat, "sic");
    let start = source.find("隼人生").unwrap();
    let end = source.find("［＃「隼人生ら」").unwrap();
    assert_eq!(note["target_source_spans"][0]["byte_start"], start, "{aat}");
    assert_eq!(note["target_source_spans"][0]["byte_end"], end);
}

#[test]
fn a_ruby_reading_sic_note_after_trailing_text_targets_the_nearest_ruby_reading() {
    let source = "題\n作者\n\n機《しほ》に［＃ルビの「しほ」はママ］起《た》つ\n\n底本：本\n";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = first_note(&aat, "sic");
    let reading = source.find("しほ").unwrap();
    assert_eq!(
        note["target_source_spans"],
        serde_json::json!([{"byte_start": reading, "byte_end": reading + 6, "line_start": 4, "line_end": 4}]),
        "{aat}"
    );
}

#[test]
fn a_quoted_sic_target_inside_the_ruby_base_before_the_marker_is_that_part_of_the_base() {
    let source = "題\n作者\n\n泥壁に鋸屑《かんなくず》［＃「鋸」はママ］の散っている\n\n底本：本\n";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = first_note(&aat, "sic");
    let start = source.find("鋸屑").unwrap();
    assert_eq!(
        note["target_source_spans"],
        serde_json::json!([{"byte_start": start, "byte_end": start + 3, "line_start": 4, "line_end": 4}]),
        "{aat}"
    );
}

#[test]
fn an_unprefixed_quoted_sic_target_matching_only_the_preceding_reading_is_the_reading() {
    let source = "題\n作者\n\n愛玩《あいかん》［＃「あいかん」はママ］する\n\n底本：本\n";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = first_note(&aat, "sic");
    let start = source.find("あいかん").unwrap();
    assert_eq!(
        note["target_source_spans"],
        serde_json::json!([{"byte_start": start, "byte_end": start + 12, "line_start": 4, "line_end": 4}]),
        "{aat}"
    );
}

#[test]
fn a_quoted_sic_target_followed_by_a_particle_is_its_last_occurrence_before_the_marker() {
    let source =
        "題\n作者\n\n陸路の外に。然らば此陸運は［＃「陸運」はママ］どうであつた\n\n底本：本\n";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = first_note(&aat, "sic");
    let start = source.find("陸運は").unwrap();
    assert_eq!(
        note["target_source_spans"],
        serde_json::json!([{"byte_start": start, "byte_end": start + 6, "line_start": 4, "line_end": 4}]),
        "{aat}"
    );
}

#[test]
fn a_quoted_sic_target_resolves_inside_an_indented_block() {
    let source = "題\n作者\n\n［＃ここから２字下げ］\n水いろの絽の染裕衣《そめゆかた》［＃「裕衣」はママ］\n［＃ここで字下げ終わり］\n\n底本：本\n";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = first_note(&aat, "sic");
    let start = source.find("裕衣").unwrap();
    assert_eq!(
        note["target_source_spans"],
        serde_json::json!([{"byte_start": start, "byte_end": start + 6, "line_start": 5, "line_end": 5}]),
        "{aat}"
    );
}

#[test]
fn a_quoted_sic_target_spans_paired_style_markers() {
    let source = "題\n作者\n\n『［＃太字］即興曲［＃太字終わり］＝［＃太字］ヘ長調［＃太字終わり］［＃「即興曲＝ヘ長調」はママ］』\n\n底本：本\n";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = first_note(&aat, "sic");
    let start = source.find("即興曲").unwrap();
    let end = source.find("ヘ長調").unwrap() + "ヘ長調".len();
    assert_eq!(
        note["target_source_spans"],
        serde_json::json!([{"byte_start": start, "byte_end": end, "line_start": 4, "line_end": 4}]),
        "{aat}"
    );
}

#[test]
fn a_sic_note_whose_operands_cannot_be_read_stays_raw_as_uninterpreted_notation() {
    let source = "題\n作者\n\nここに餌《えは》［＃《えは》はママ］あり\n\n底本：本\n";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let raw = aat["blocks"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["kind"] == "raw")
        .unwrap_or_else(|| panic!("missing raw note: {aat}"));
    assert_eq!(raw["source"], "［＃《えは》はママ］");
    assert_eq!(
        raw["interpretation_problem"]["code"],
        "uninterpreted-notation"
    );
}
