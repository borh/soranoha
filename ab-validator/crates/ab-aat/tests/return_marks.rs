//! Retrospective kunten declarations own the supplied target and exact marker.

use ab_aat::aat_json_from_bytes;
use serde_json::Value;

#[test]
fn scoped_and_inserted_return_mark_policies_are_typed() {
    for (source, scope, marks, exclusions, repeated, ruled) in [
        (
            "「撃レ非如レ鷹」［＃「」内の「レ」は返り点］",
            "quoted-region",
            "レ",
            0,
            false,
            false,
        ),
        (
            "［＃以下、「レ一二」は返り点］「準二拠一」",
            "following-text",
            "レ一二",
            0,
            false,
            false,
        ),
        (
            "令［＃返り点の「二」の右横に縦棒あり］二",
            "insertion-site",
            "二",
            0,
            false,
            true,
        ),
        (
            "「凡聴レ訟者必須二両辞一、偏信二一言一折レ獄者」［＃「一言」の「一」をのぞいて「レ一二三」は返り点］",
            "quoted-region",
            "レ一二三",
            1,
            false,
            false,
        ),
        (
            "「欲レ食二其肉一」［＃「」内の「レ一二」は返り点、以下同じ］",
            "quoted-region",
            "レ一二",
            0,
            true,
            false,
        ),
    ] {
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let instruction = aat["blocks"][0]["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["kind"] == "formatting_instruction")
            .unwrap_or_else(|| panic!("{source}: {aat}"));
        let operation = &instruction["operations"][0];
        let effect = &operation["effect"];
        assert_eq!(effect["kind"], "kunten-policy");
        assert_eq!(effect["scope"], scope);
        assert_eq!(effect["marks"]["text"], marks);
        assert_eq!(effect["exclusions"].as_array().unwrap().len(), exclusions);
        assert_eq!(effect["repeat_following"], repeated);
        assert_eq!(effect["vertical_rule_right"], ruled);
        if scope == "insertion-site" {
            assert_eq!(operation["selection"]["scope"], instruction["span"]);
            assert_eq!(
                operation["selection"]["targets"],
                serde_json::json!([effect["marks"]["span"]])
            );
        } else if source.starts_with("「撃") || source.starts_with("［＃以下") {
            assert_eq!(
                operation["selection"]["targets"].as_array().unwrap().len(),
                2
            );
        } else if source.starts_with("「凡聴") {
            assert_eq!(
                operation["selection"]["targets"].as_array().unwrap().len(),
                6
            );
            assert!(aat.to_string().contains("\"value\":\"一言\""));
        } else if source.starts_with("「欲") {
            assert_eq!(
                operation["selection"]["targets"].as_array().unwrap().len(),
                3
            );
        } else {
            assert!(operation.get("selection").is_none(), "{source}: {aat}");
        }
        assert!(aat["meta"]["parse_complete"].as_bool().unwrap());
    }
}

#[test]
fn quoted_return_mark_policy_can_precede_its_target() {
    let source = "［＃「」内の「一二」は返り点］「告二祖父母父母一者絞」";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let operation = &aat["blocks"][0]["content"][0]["operations"][0];

    assert_eq!(operation["effect"]["scope"], "quoted-region");
    assert_eq!(
        operation["selection"]["targets"].as_array().unwrap().len(),
        2
    );
    let paragraph = aat["blocks"][0]["content"].as_array().unwrap();
    assert_eq!(
        paragraph
            .iter()
            .filter(|node| node["kind"] == "kunten")
            .count(),
        2
    );
}

#[test]
fn closing_policy_can_reaffirm_marks_realized_by_the_opening_policy() {
    let source = "［＃ここから２字下げ、「一銭」の「一」をのぞいて「レ一二」は返り点］\n門前百姓、於二非法有一レ之者、可レ為二一銭切一事。\n［＃ここで字下げ終わり、「レ」は返り点］";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let block = &aat["blocks"][0];
    let close = &block["source_instructions"][1]["operations"][0];

    assert_eq!(close["effect"]["scope"], "preceding-block");
    assert_eq!(close["selection"]["targets"].as_array().unwrap().len(), 2);
    let close_span = &block["source_instructions"][1]["span"];
    let reaffirmed = block["children"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|node| node["kind"] == "kunten" && node["text"] == "レ")
        .collect::<Vec<_>>();
    assert_eq!(reaffirmed.len(), 2);
    assert!(reaffirmed.iter().all(|node| {
        node["interpretation_marker_spans"]
            .as_array()
            .unwrap()
            .contains(close_span)
    }));
}

#[test]
fn exact_marker_facts_are_retained_for_plain_and_rich_ruby_targets() {
    for source in [
        "前レ［＃「レ」は返り点］後",
        "｜漢レ［＃「レ」は返り点］字《かんじ》",
        "前一レ［＃「一レ」は返り点］後",
    ] {
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let facts = aat["meta"]["interpretation_facts"].as_array().unwrap();
        let fact = facts
            .iter()
            .find(|fact| fact["kind"] == "kunten")
            .unwrap_or_else(|| panic!("{source}: {aat}"));
        let start = usize::try_from(fact["source_span"]["start"].as_u64().unwrap()).unwrap();
        let end = usize::try_from(fact["source_span"]["end"].as_u64().unwrap()).unwrap();
        assert_eq!(
            &source[start..end],
            &source[source.find('［').unwrap()..source.find('］').unwrap() + "］".len()],
            "{aat}"
        );
        let mut pending = vec![&aat["blocks"]];
        let mut marks = Vec::new();
        while let Some(value) = pending.pop() {
            match value {
                Value::Object(fields) => {
                    if value["kind"] == "kunten" {
                        marks.push(value);
                    }
                    pending.extend(fields.values());
                }
                Value::Array(values) => pending.extend(values),
                _ => {}
            }
        }
        assert_eq!(marks.len(), 1, "{aat}");
        let mark = marks[0];
        assert_eq!(mark["interpretation_marker_spans"][0]["byte_start"], start);
        assert_eq!(mark["span"]["byte_end"], end);
        assert!(mark["span"]["byte_start"].as_u64().unwrap() < start as u64);
    }
}

#[test]
fn unestablished_reading_target_remains_located_source_notation() {
    let source = "｜漢字《かんレ［＃「レ」は返り点］じ》";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let reading = &aat["blocks"][0]["content"][0]["reading_content"];
    assert_eq!(reading[0]["value"], "かんレ");
    assert_eq!(reading[1]["source"], "［＃「レ」は返り点］");
    assert_eq!(
        reading[1]["interpretation_problem"]["kind"],
        "unknown-notation"
    );
    assert!(
        !aat["meta"]["interpretation_facts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|fact| fact["kind"] == "kunten")
    );
}
