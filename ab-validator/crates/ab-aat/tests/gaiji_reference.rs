//! Whole-glyph identifiers and component descriptions across the source-to-AAT boundary.

use ab_aat::aat_json_from_bytes;
use serde_json::Value;

fn nodes_of_kind<'a>(value: &'a Value, kind: &str, found: &mut Vec<&'a Value>) {
    match value {
        Value::Object(object) => {
            if value["kind"] == kind {
                found.push(value);
            }
            for child in object.values() {
                nodes_of_kind(child, kind, found);
            }
        }
        Value::Array(values) => {
            for child in values {
                nodes_of_kind(child, kind, found);
            }
        }
        _ => {}
    }
}

#[test]
fn roman_numeral_declaration_separates_its_target_from_its_marker() {
    let marker = "［＃「1」はローマ数字、1-13-21］";
    let source = format!("題\n作者\n\n詩集1{marker}後\n\n底本：本\n");
    let document: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let gaiji = &document["blocks"][0]["content"][1];
    let target_start = source.find("1［").unwrap();
    let marker_start = target_start + 1;

    assert_eq!(gaiji["kind"], "gaiji");
    assert_eq!(gaiji["resolved"], "Ⅰ");
    assert_eq!(gaiji["span"]["byte_start"], target_start);
    assert_eq!(gaiji["span"]["byte_end"], marker_start);
    assert_eq!(
        gaiji["interpretation_marker_spans"][0]["byte_start"],
        marker_start
    );
    assert_eq!(
        gaiji["interpretation_marker_spans"][0]["byte_end"],
        marker_start + marker.len()
    );
    assert_eq!(document["blocks"][0]["content"][0]["value"], "詩集");
    assert_eq!(document["blocks"][0]["content"][2]["value"], "後");
}

#[test]
fn unresolved_combined_punctuation_keeps_target_text_and_marker_identity() {
    let marker = "［＃「!!!」は一文字、95-13］";
    let body = format!("詩集!!!{marker}後");
    let source = format!("題\n作者\n\n{body}\n\n底本：本\n");
    let document: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let gaiji = document["blocks"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["kind"] == "gaiji")
        .unwrap();
    let target_start = source.find("!!!").unwrap();
    let marker_start = target_start + 3;

    assert_eq!(gaiji["resolved"], Value::Null);
    assert_eq!(gaiji["source_fallback"], "!!!");
    assert_eq!(gaiji["span"]["byte_start"], target_start);
    assert_eq!(gaiji["span"]["byte_end"], marker_start);
    assert_eq!(
        gaiji["interpretation_marker_spans"][0]["byte_start"],
        marker_start
    );
    assert_eq!(
        gaiji["interpretation_marker_spans"][0]["byte_end"],
        marker_start + marker.len()
    );
}

#[test]
fn persistent_double_parenthesis_declarations_relate_every_later_pair() {
    let open = "［＃この二重括弧は一文字、以下同様に使用、第3水準1-2-54、197-16］";
    let close = "［＃この二重括弧は一文字、以下同様に使用、第3水準1-2-55、197-16］";
    let body = format!("（（{open}前文略））{close}\n（（中略））\n（（下略））");
    let source = format!("題\n作者\n\n{body}\n\n底本：本\n");
    let document: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let gaiji = document["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|block| block["content"].as_array().into_iter().flatten())
        .filter(|node| node["kind"] == "gaiji")
        .collect::<Vec<_>>();
    let open_marker_start = source.find(open).unwrap();
    let close_marker_start = source.find(close).unwrap();

    assert_eq!(gaiji.len(), 6);
    assert_eq!(
        gaiji
            .iter()
            .map(|node| node["resolved"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["｟", "｠", "｟", "｠", "｟", "｠"]
    );
    for node in &gaiji {
        let (expected_marker, expected_source) = if node["resolved"] == "｟" {
            (open_marker_start, "（（")
        } else {
            (close_marker_start, "））")
        };
        assert_eq!(
            node["interpretation_marker_spans"][0]["byte_start"],
            expected_marker
        );
        let start = usize::try_from(node["span"]["byte_start"].as_u64().unwrap()).unwrap();
        let end = usize::try_from(node["span"]["byte_end"].as_u64().unwrap()).unwrap();
        assert!(start < end);
        assert_eq!(&source[start..end], expected_source);
    }
    assert!(document["meta"]["interpretation_problems"].is_null());
}

#[test]
fn encoded_character_declarations_separate_targets_from_markers() {
    for (prefix, target, marker, resolved, suffix) in [
        (
            "LACLART",
            "E",
            "［＃「E」はアキュートアクセント付きE、1-9-32］",
            "É",
            "　第二",
        ),
        (
            "タックネッ",
            "プ",
            "［＃「プ」は小書き半濁点付き片仮名フ、1-6-88］",
            "ㇷ゚",
            "ネ",
        ),
        (
            "シサ",
            "ム",
            "［＃「ム」は小書き片仮名ム、1-6-89］",
            "ㇺ",
            "達",
        ),
    ] {
        let body = format!("{prefix}{target}{marker}{suffix}");
        let source = format!("題\n作者\n\n{body}\n\n底本：本\n");
        let document: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let gaiji = document["blocks"][0]["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["kind"] == "gaiji")
            .unwrap();
        let target_start = source.find(&format!("{target}{marker}")).unwrap();
        let marker_start = target_start + target.len();

        assert_eq!(gaiji["resolved"], resolved, "{body}");
        assert_eq!(gaiji["span"]["byte_start"], target_start, "{body}");
        assert_eq!(gaiji["span"]["byte_end"], marker_start, "{body}");
        assert_eq!(
            gaiji["interpretation_marker_spans"][0]["byte_start"], marker_start,
            "{body}"
        );
        assert_eq!(
            gaiji["interpretation_marker_spans"][0]["byte_end"],
            marker_start + marker.len(),
            "{body}"
        );
    }
}

#[test]
fn standalone_encoded_glyphs_use_the_declaration_position_as_their_source() {
    for marker in [
        "［＃「※」は「題の頁の代わりに韋」、第4水準2-92-15、182-13］",
        "［＃「※」は「題の頁の代わりに韋」、第4水準2-92-15、184-11］",
    ] {
        let body = format!("丁{marker}良");
        let source = format!("題\n作者\n\n{body}\n\n底本：本\n");
        let document: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let gaiji = document["blocks"][0]["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["kind"] == "gaiji")
            .unwrap();
        let marker_start = source.find(marker).unwrap();

        assert_eq!(gaiji["resolved"], "韙", "{body}");
        assert_eq!(gaiji["span"]["byte_start"], marker_start, "{body}");
        assert_eq!(
            gaiji["span"]["byte_end"],
            marker_start + marker.len(),
            "{body}"
        );
        assert!(gaiji.get("interpretation_marker_spans").is_none(), "{body}");
        assert_eq!(&source[marker_start..marker_start + marker.len()], marker);
        assert!(document["meta"]["interpretation_problems"].is_null());
    }
}

#[test]
fn ruby_reading_declarations_keep_the_target_on_the_reading_axis() {
    for (body, target, marker, reading) in [
        (
            "太陽《チュプ》［＃「プ」は小書き半濁点付き片仮名フ、1-6-88］",
            "プ",
            "［＃「プ」は小書き半濁点付き片仮名フ、1-6-88］",
            "チュㇷ゚",
        ),
        (
            "和人《シサム》［＃「ム」は小書き片仮名ム、1-6-89］達",
            "ム",
            "［＃「ム」は小書き片仮名ム、1-6-89］",
            "シサㇺ",
        ),
        (
            "和人《シサム［＃「ム」は小書き片仮名ム、1-6-89］》",
            "ム",
            "［＃「ム」は小書き片仮名ム、1-6-89］",
            "シサㇺ",
        ),
    ] {
        let source = format!("題\n作者\n\n{body}\n\n底本：本\n");
        let document: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let ruby = document["blocks"][0]["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["kind"] == "ruby")
            .unwrap();
        let gaiji = ruby["reading_content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["kind"] == "gaiji")
            .unwrap();
        let marker_start = source.find(marker).unwrap();
        let target_start = source[..marker_start].rfind(target).unwrap();

        assert_eq!(ruby["reading"], reading, "{body}");
        assert_eq!(gaiji["span"]["byte_start"], target_start, "{body}");
        assert_eq!(
            gaiji["span"]["byte_end"],
            target_start + target.len(),
            "{body}"
        );
        assert_eq!(
            gaiji["interpretation_marker_spans"][0]["byte_start"], marker_start,
            "{body}"
        );
        assert_eq!(
            gaiji["interpretation_marker_spans"][0]["byte_end"],
            marker_start + marker.len(),
            "{body}"
        );
        assert!(
            document["blocks"][0]["content"]
                .as_array()
                .unwrap()
                .iter()
                .all(|node| node["kind"] != "gaiji")
        );
    }
}

#[test]
fn detached_declarations_keep_the_placeholder_as_target() {
    for (body, marker, resolved, ruby) in [
        (
            "※々［＃「※」は「つつみがまえ（勹）」＋「夕」で、読みは「そうそう」67-6］",
            "［＃「※」は「つつみがまえ（勹）」＋「夕」で、読みは「そうそう」67-6］",
            None,
            false,
        ),
        (
            "琅※《らうかん》［＃「※」は「王へん」に「干」、第3水準1-87-83、P61-下段5首目］",
            "［＃「※」は「王へん」に「干」、第3水準1-87-83、P61-下段5首目］",
            Some("玕"),
            true,
        ),
        (
            "※《いとど》［＃「※」は「虫へん」に「車」、第3水準1-91-55、69-下段9首目］",
            "［＃「※」は「虫へん」に「車」、第3水準1-91-55、69-下段9首目］",
            Some("蛼"),
            true,
        ),
    ] {
        let source = format!("題\n作者\n\n{body}\n\n底本：本\n");
        let document: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let content = document["blocks"][0]["content"].as_array().unwrap();
        let owner = content
            .iter()
            .find(|node| node["kind"] == if ruby { "ruby" } else { "gaiji" })
            .unwrap();
        let gaiji = if ruby {
            owner["base_content"]
                .as_array()
                .unwrap()
                .iter()
                .find(|node| node["kind"] == "gaiji")
                .unwrap()
        } else {
            owner
        };
        let target_start = source.find('※').unwrap();
        let marker_start = source.find(marker).unwrap();
        assert_eq!(gaiji["resolved"].as_str(), resolved, "{body}");
        assert_eq!(gaiji["span"]["byte_start"], target_start, "{body}");
        assert_eq!(
            gaiji["span"]["byte_end"],
            target_start + '※'.len_utf8(),
            "{body}"
        );
        assert_eq!(
            gaiji["interpretation_marker_spans"][0]["byte_start"], marker_start,
            "{body}"
        );
        assert_eq!(
            gaiji["interpretation_marker_spans"][0]["byte_end"],
            marker_start + marker.len(),
            "{body}"
        );
        assert!(content.iter().all(|node| node["kind"] != "raw"), "{body}");
    }
}

#[test]
fn detached_declarations_compose_with_ruby_and_bouten() {
    let ruby_marker = "［＃「さんずい＋自」、第3水準1-86-66、XI-1］";
    let ruby_body = format!("幽かな※芙藍{ruby_marker}《さふらん》の凋れ");
    let ruby_source = format!("題\n作者\n\n{ruby_body}\n\n底本：本\n");
    let ruby_document: Value =
        serde_json::from_slice(&aat_json_from_bytes(ruby_source.as_bytes()).unwrap()).unwrap();
    let mut ruby_nodes = Vec::new();
    nodes_of_kind(&ruby_document["blocks"], "ruby", &mut ruby_nodes);
    let ruby = ruby_nodes
        .into_iter()
        .find(|node| node["reading"] == "さふらん")
        .unwrap();
    assert_eq!(ruby["base"], "洎芙藍");
    let gaiji = ruby["base_content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["kind"] == "gaiji")
        .unwrap();
    let target_start = ruby_source.find('※').unwrap();
    let marker_start = ruby_source.find(ruby_marker).unwrap();
    assert_eq!(gaiji["resolved"], "洎");
    assert_eq!(gaiji["span"]["byte_start"], target_start);
    assert_eq!(gaiji["span"]["byte_end"], target_start + '※'.len_utf8());
    assert_eq!(
        gaiji["interpretation_marker_spans"][0]["byte_start"],
        marker_start
    );
    assert_eq!(
        ruby_document["meta"]["interpretation_problems"],
        Value::Null
    );

    let bouten_marker = "［＃「大※」に傍点］";
    let gaiji_marker = "［＃「郷－即のへん」、232-1］";
    let bouten_body = format!("郵便局の上の字は大※{bouten_marker}{gaiji_marker}だけ見えている");
    let bouten_source = format!("題\n作者\n\n{bouten_body}\n\n底本：本\n");
    let bouten_document: Value =
        serde_json::from_slice(&aat_json_from_bytes(bouten_source.as_bytes()).unwrap()).unwrap();
    let mut styles = Vec::new();
    nodes_of_kind(&bouten_document["blocks"], "style", &mut styles);
    let style = styles
        .into_iter()
        .find(|node| node["style_type"] == "bouten")
        .unwrap();
    let style_content = style["content"].as_array().unwrap();
    assert_eq!(style_content[0]["value"], "大");
    let gaiji = style_content
        .iter()
        .find(|node| node["kind"] == "gaiji")
        .unwrap();
    let target_start = bouten_source.find('※').unwrap();
    let gaiji_marker_start = bouten_source.find(gaiji_marker).unwrap();
    assert_eq!(gaiji["resolved"], "\u{28675}");
    assert_eq!(gaiji["span"]["byte_start"], target_start);
    assert_eq!(gaiji["span"]["byte_end"], target_start + '※'.len_utf8());
    assert_eq!(
        gaiji["interpretation_marker_spans"][0]["byte_start"],
        gaiji_marker_start
    );
    assert_eq!(
        style["interpretation_marker_spans"][0]["byte_start"],
        bouten_source.find(bouten_marker).unwrap()
    );
    assert_eq!(
        bouten_document["meta"]["interpretation_problems"],
        Value::Null
    );
}

#[test]
fn explicit_gaiji_identifiers_establish_content_without_rewriting_references() {
    for (marker, reference, expected) in [
        (
            "※［＃「えんにょう＋囘」、第４水準2-12-11］",
            "第４水準2-12-11",
            "𢌞",
        ),
        (
            "※［＃「つつみがまえ＋夕」、読みは「そう」、第3水準1-14-76、30-8］",
            "読みは「そう」、第3水準1-14-76、30-8",
            "匇",
        ),
        (
            "※［＃「滷－さんずい」、第3水準1-83-35］",
            "第3水準1-83-35",
            "鹵",
        ),
    ] {
        let source = format!("題\n作者\n\n前{marker}後。\n\n底本：本\n");
        let document: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let gaiji = &document["blocks"][0]["content"][1];
        assert_eq!(gaiji["kind"], "gaiji");
        assert_eq!(gaiji["resolved"], expected);
        assert_eq!(gaiji["jis_code"], reference);
        let start = usize::try_from(gaiji["span"]["byte_start"].as_u64().unwrap()).unwrap();
        let end = usize::try_from(gaiji["span"]["byte_end"].as_u64().unwrap()).unwrap();
        assert_eq!(&source[start..end], marker);
        let fact = &document["meta"]["interpretation_facts"][0];
        assert_eq!(fact["kind"], "gaiji");
        assert_eq!(fact["outcome"], "established");
        assert_eq!(fact["source_span"]["start"], start);
        assert_eq!(fact["source_span"]["end"], end);
    }
}

#[test]
fn a_component_identifier_does_not_establish_the_containing_glyph() {
    let marker = "※［＃非0213外字：「厂＋菫」、ただし「菫」は第3水準1-92-16のつくりの形、読みは「わづ」、289-上-12］";
    let source = format!("題\n作者\n\n前{marker}後。\n\n底本：本\n");
    let document: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let gaiji = &document["blocks"][0]["content"][1];
    assert_eq!(gaiji["kind"], "gaiji");
    assert!(gaiji["resolved"].is_null());
    assert_eq!(gaiji["jis_code"], "289-上-12");
    assert!(document["meta"]["interpretation_facts"].is_null());
}

#[test]
fn ruby_association_requires_a_resolved_base_and_supplied_reading() {
    let marker = "※［＃「需＋頁」、第3水準1-94-6］";
    for (body, expected) in [
        (format!("{marker}《じゅ》"), 1),
        (format!("｜前{marker}《ぜんじゅ》"), 1),
        (marker.to_owned(), 0),
        (format!("{marker}字《じ》"), 0),
        (format!("{marker}《》"), 0),
        ("※［＃未知の字］《じゅ》".to_owned(), 0),
    ] {
        let source = format!("題\n作者\n\n{body}\n\n底本：本\n");
        let document: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let claims: Vec<_> = document["meta"]["interpretation_facts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|fact| fact["kind"] == "gaiji-ruby")
            .collect();
        assert_eq!(claims.len(), expected, "{body}");
        for claim in claims {
            let start = usize::try_from(claim["source_span"]["start"].as_u64().unwrap()).unwrap();
            let end = usize::try_from(claim["source_span"]["end"].as_u64().unwrap()).unwrap();
            assert!(&source[start..end] == marker || source[start..end] == body);
        }
    }
}

#[test]
fn gaiji_reference_parts_distinguish_codes_locations_and_readings() {
    for (marker, expected) in [
        (
            "※［＃「纏」の「里」に代えて「黨－尚－れんが」、「广」に代えて「厂」、3-12］",
            serde_json::json!([{"kind": "source-location", "source": "3-12", "page": "3", "line": "12"}]),
        ),
        (
            "※［＃濁点付き片仮名ワ、1-7-82］",
            serde_json::json!([{"kind": "jis-code", "source": "1-7-82", "plane": 1, "row": 7, "cell": 82}]),
        ),
        (
            "※［＃鋭アクセント付きο、U+1F79、17-3］",
            serde_json::json!([{"kind": "unicode-code", "source": "U+1F79", "codepoint": 8057}, {"kind": "source-location", "source": "17-3", "page": "17", "line": "3"}]),
        ),
        (
            "※［＃ローマ数字20、172-上-21］",
            serde_json::json!([{"kind": "source-location", "source": "172-上-21", "page": "172", "column": "上", "line": "21"}]),
        ),
        (
            "※［＃「てへん＋樗のつくり」、読みは「ちょ」、374-14］",
            serde_json::json!([{"kind": "reading", "source": "読みは「ちょ」", "text": "ちょ"}, {"kind": "source-location", "source": "374-14", "page": "374", "line": "14"}]),
        ),
        (
            "※［＃「囗＜欒」、U+571E、7-図キャプション］",
            serde_json::json!([{"kind": "unicode-code", "source": "U+571E", "codepoint": 22302}, {"kind": "source-location", "source": "7-図キャプション", "page": "7", "part": "図キャプション"}]),
        ),
        (
            "※［＃「竹かんむり／瓜」、U+7B1F、1巻-23-下-6］",
            serde_json::json!([{"kind": "unicode-code", "source": "U+7B1F", "codepoint": 31519}, {"kind": "source-location", "source": "1巻-23-下-6", "volume": "1巻", "page": "23", "column": "下", "line": "6"}]),
        ),
    ] {
        let source = format!("題\n作者\n\n前{marker}後。\n\n底本：本\n");
        let document: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let gaiji = &document["blocks"][0]["content"][1];
        assert_eq!(gaiji["reference_parts"], expected, "{marker}");
    }
}

#[test]
fn reference_parts_preserve_noncanonical_codes_and_uninterpreted_clauses() {
    for (reference, expected) in [
        (
            "第４水準 ２-１２-１１",
            serde_json::json!([{"kind":"jis-code","source":"第４水準 ２-１２-１１","plane":2,"row":12,"cell":11}]),
        ),
        (
            "U+D800、8-3",
            serde_json::json!([{"kind":"uninterpreted","source":"U+D800"},{"kind":"source-location","source":"8-3","page":"8","line":"3"}]),
        ),
        (
            "第3水準2-12-11",
            serde_json::json!([{"kind":"uninterpreted","source":"第3水準2-12-11"}]),
        ),
        (
            "1-95-1",
            serde_json::json!([{"kind":"uninterpreted","source":"1-95-1"}]),
        ),
        (
            "U+884E、225-図のキャプション",
            serde_json::json!([{"kind":"unicode-code","source":"U+884E","codepoint":34894},{"kind":"source-location","source":"225-図のキャプション","page":"225","part":"図のキャプション"}]),
        ),
        (
            "読みは「い、ろ」、第3水準1-87-64",
            serde_json::json!([{"kind":"reading","source":"読みは「い、ろ」","text":"い、ろ"},{"kind":"jis-code","source":"第3水準1-87-64","plane":1,"row":87,"cell":64}]),
        ),
        (
            "第三水準1-3-28、107-16",
            serde_json::json!([{"kind":"jis-code","source":"第三水準1-3-28","plane":1,"row":3,"cell":28},{"kind":"source-location","source":"107-16","page":"107","line":"16"}]),
        ),
        (
            "U+884E、場所不明",
            serde_json::json!([{"kind":"unicode-code","source":"U+884E","codepoint":34894},{"kind":"uninterpreted","source":"場所不明"}]),
        ),
    ] {
        let marker = format!("※［＃「仮」、{reference}］");
        let source = format!("題\n作者\n\n前{marker}後。\n\n底本：本\n");
        let document: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let gaiji = &document["blocks"][0]["content"][1];
        assert_eq!(gaiji["kind"], "gaiji", "{marker}");
        assert_eq!(gaiji["reference_parts"], expected, "{marker}");
    }
}

#[test]
fn conflicting_whole_glyph_identifiers_remain_typed_and_unresolved() {
    for (marker, expected_parts) in [
        (
            "※［＃「りっしんべん＋粟」、U+619F、2-12-34］",
            serde_json::json!([{"kind":"unicode-code","source":"U+619F","codepoint":24991},{"kind":"jis-code","source":"2-12-34","plane":2,"row":12,"cell":34}]),
        ),
        (
            "※［＃「くさかんむり／間」、U+8573、第4水準2-86-80］",
            serde_json::json!([{"kind":"unicode-code","source":"U+8573","codepoint":34163},{"kind":"jis-code","source":"第4水準2-86-80","plane":2,"row":86,"cell":80}]),
        ),
    ] {
        let source = format!("題\n作者\n\n{marker}\n\n底本：本\n");
        let document: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let gaiji = &document["blocks"][0]["content"][0];

        assert_eq!(gaiji["kind"], "gaiji", "{marker}");
        assert!(gaiji["resolved"].is_null(), "{marker}");
        assert_eq!(gaiji["unresolved_reason"], "unresolved", "{marker}");
        assert_eq!(gaiji["reference_parts"], expected_parts, "{marker}");
    }
}

#[test]
fn okurigana_gaiji_is_kunten_content() {
    let reference = "※［＃「低のつくり」、第3水準1-86-47］";
    let source = format!("題\n作者\n\n頸根衝抜［＃（{reference}）］……\n\n底本：本\n");
    let document: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let mut kunten = Vec::new();
    nodes_of_kind(&document["blocks"], "kunten", &mut kunten);
    let [okurigana] = kunten.as_slice() else {
        panic!("one kunten node expected: {kunten:?}");
    };
    let reference_start = source.find(reference).unwrap();

    assert_eq!(okurigana["kunten_kind"], "okurigana");
    assert_eq!(okurigana["text"], "氐");
    let gaiji = &okurigana["annotation_content"][0];
    assert_eq!(gaiji["kind"], "gaiji");
    assert_eq!(gaiji["resolved"], "氐");
    assert_eq!(gaiji["span"]["byte_start"], reference_start);
    assert_eq!(gaiji["span"]["byte_end"], reference_start + reference.len());
}

#[test]
fn okurigana_with_an_unresolved_gaiji_remains_raw() {
    let source = "題\n作者\n\n以［＃（※［＃「未詳」］）］告\n\n底本：本\n";
    let document: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let mut kunten = Vec::new();
    nodes_of_kind(&document["blocks"], "kunten", &mut kunten);

    assert!(kunten.is_empty(), "{kunten:?}");
}

#[test]
fn ruled_box_note_declares_the_one_refmark_in_the_box() {
    let note = "［＃罫内の「※」は「※［＃感嘆符二つ、1-8-75］」］";
    let source =
        format!("題\n作者\n\n　┌──────┐\n　│　万歳※　│\n　└──────┘\n　　{note}\n\n底本：本\n");
    let document: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let mut gaiji = Vec::new();
    nodes_of_kind(&document["blocks"], "gaiji", &mut gaiji);
    let [glyph] = gaiji.as_slice() else {
        panic!("one gaiji node expected: {gaiji:?}");
    };
    let target_start = source.find("※　│").unwrap();
    let note_start = source.find(note).unwrap();

    assert_eq!(glyph["resolved"], "‼");
    assert_eq!(glyph["jis_code"], "1-8-75");
    assert_eq!(glyph["span"]["byte_start"], target_start);
    assert_eq!(glyph["span"]["byte_end"], target_start + '※'.len_utf8());
    assert_eq!(
        glyph["interpretation_marker_spans"][0]["byte_start"],
        note_start
    );
    assert_eq!(
        glyph["interpretation_marker_spans"][0]["byte_end"],
        note_start + note.len()
    );
    let mut raw = Vec::new();
    nodes_of_kind(&document["blocks"], "raw", &mut raw);
    assert!(raw.is_empty(), "{raw:?}");
}

#[test]
fn ruled_box_note_without_one_boxed_refmark_remains_raw() {
    let note = "［＃罫内の「※」は「※［＃感嘆符二つ、1-8-75］」］";
    for body in [
        format!("　┌────┐\n　│※※│\n　└────┘\n{note}"),
        format!("前※\n{note}"),
    ] {
        let source = format!("題\n作者\n\n{body}\n\n底本：本\n");
        let document: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let mut gaiji = Vec::new();
        nodes_of_kind(&document["blocks"], "gaiji", &mut gaiji);
        let mut raw = Vec::new();
        nodes_of_kind(&document["blocks"], "raw", &mut raw);

        assert!(gaiji.is_empty(), "{body}: {gaiji:?}");
        assert!(raw.iter().any(|node| node["source"] == note), "{body}");
    }
}

#[test]
fn a_quoted_unresolved_gaiji_establishes_its_structure_alone() {
    let marker = "※［＃濁点付き井、379-1］";
    let body = format!(
        "ダ・{marker}ンチ等の［＃「ダ・{marker}ンチ等の」は底本では「ダ{marker}ンチ等の」］"
    );
    let source = format!("題\n作者\n\n{body}\n\n底本：本\n");
    let document: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let mut variants = Vec::new();
    nodes_of_kind(&document["blocks"], "text-variant", &mut variants);
    assert_eq!(variants.len(), 1);
    let quoted = source.match_indices(marker).nth(1).unwrap().0;
    let fact = serde_json::json!({"kind":"gaiji", "outcome":"established", "aspects":["structure"],
        "source_span":{"start":quoted, "end":quoted + marker.len(), "line":4, "coordinate_system":"decoded_utf8"}});
    assert_eq!(
        variants[0]["current_interpretation_facts"],
        serde_json::json!([fact])
    );
    // The body reference and the base-edition reading establish no gaiji fact.
    let gaiji_facts = document["meta"]["interpretation_facts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|fact| fact["kind"] == "gaiji")
        .collect::<Vec<_>>();
    assert_eq!(gaiji_facts, [&fact]);
}
