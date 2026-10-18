//! Formatting selectors remain source apparatus through AAT and parser IR.

use std::path::Path;

use ab_aat_to_parser_ir::{ConversionOptions, ConversionRequest, MappingDocument, SchemaSet};
use serde_json::{Value, json};

fn convert(source: &str) -> (Value, Value) {
    let aat: Value =
        serde_json::from_slice(&ab_aat::aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let result = ab_aat_to_parser_ir::convert(ConversionRequest {
        aat: aat.clone(),
        mapping: MappingDocument::from_path(&repo.join("data/aat-to-parser-ir-mapping-v2.json"))
            .unwrap(),
        schemas: SchemaSet::for_aat_version(2).unwrap(),
        options: ConversionOptions::default(),
    })
    .unwrap();
    (aat, result.parser_ir)
}

fn objects(value: &Value) -> Vec<&Value> {
    let mut pending = vec![value];
    let mut result = Vec::new();
    while let Some(value) = pending.pop() {
        match value {
            Value::Object(map) => {
                result.push(value);
                pending.extend(map.values());
            }
            Value::Array(values) => pending.extend(values),
            _ => {}
        }
    }
    result
}

fn source_text<'a>(source: &'a str, span: &Value) -> &'a str {
    assert_eq!(
        span["coordinate_system"], "decoded_utf8",
        "instruction provenance uses original decoded bytes"
    );
    let start = usize::try_from(span["start"].as_u64().unwrap()).unwrap();
    let end = usize::try_from(span["end"].as_u64().unwrap()).unwrap();
    &source[start..end]
}

#[test]
fn heading_typography_remains_named_rendition_in_parser_ir() {
    for (marker, variant) in [("見出し文字", "heading"), ("小見出し文字", "small-heading")]
    {
        let source = format!("題\n作者\n\n新聞標題［＃{marker}］\n");
        let (_, ir) = convert(&source);
        let instruction = objects(&ir)
            .into_iter()
            .find(|node| node["type"] == "formatting-instruction")
            .unwrap();
        let operation = &instruction["operations"][0];
        assert_eq!(operation["effect"]["kind"], "heading-typography");
        assert_eq!(operation["effect"]["variant"], variant);
        assert_eq!(
            source_text(&source, &operation["selection"]["targets"][0]),
            "新聞標題"
        );
        assert!(!objects(&ir).iter().any(|node| node["type"] == "heading"));
    }
}

#[test]
fn operations_preserve_selector_meaning_order_and_source_coordinates() {
    let cases = [
        (
            "B1［＃「B1」は縦中横、「1」は下付き小文字］",
            vec!["quoted", "quoted"],
        ),
        (
            "biνi［＃２文字目の「i」は下付き小文字、４文字目の「i」は上付き小文字］",
            vec!["character-position", "character-position"],
        ),
        (
            "1/144［＃２つめの「4」は上付き小文字、「1/144」は分数］",
            vec!["occurrence", "quoted"],
        ),
        (
            "x2＋y2＝r2［＃「x2」、「y2」、「x2」はそれぞれ縦中横、すべての「2」は上付き小書き］",
            vec!["each-quoted", "all-quoted"],
        ),
        (
            "現金三万円［＃「現金三万円」は大文字、太字、「三万円」に二重丸傍点］",
            vec!["quoted", "quoted", "quoted"],
        ),
        (
            "三万円の懸賞［＃特大文字、二重丸傍点］",
            vec!["preceding-text", "preceding-text"],
        ),
        (
            "e-r2/4Dt［＃「2」は上付き小文字、「r2/4Dt」は分数、「-r2/4Dt」は「e」の上付き］",
            vec!["quoted", "quoted", "quoted"],
        ),
        (
            "s1a1b1ν + s2a2b2ν2［＃ s、a、b に続くアラビア数字はすべて下付き小文字、νに続くアラビア数字はすべて上付き小文字］",
            vec!["arabic-digits-after", "arabic-digits-after"],
        ),
        (
            "a2 = 4/10; a4 = 2/10［＃アルファベットに続くアラビア数字はすべて下付き小文字、「4/10」「2/10」は分数］",
            vec!["arabic-digits-after", "each-quoted"],
        ),
        (
            "Ａ２Ａ３Ａ４［＃「Ａｎ」はそれぞれ縦中横、数字は上付き小書き］",
            vec!["each-quoted", "digits"],
        ),
    ];
    for (body, selectors) in cases {
        let source = format!("題\r\n作者\r\n\r\n{body}\r\n\r\n底本：本\r\n");
        let (aat, ir) = convert(&source);
        let all = objects(&ir);
        let instructions: Vec<_> = all
            .iter()
            .filter(|node| node["type"] == "formatting-instruction")
            .collect();
        assert_eq!(instructions.len(), 1, "{body}");
        let instruction = instructions[0];
        assert_eq!(
            source_text(&source, &instruction["source_span"]),
            instruction["raw"].as_str().unwrap()
        );
        assert_eq!(instruction["span"]["start"], instruction["span"]["end"]);
        let operations = instruction["operations"].as_array().unwrap();
        assert_eq!(
            operations
                .iter()
                .map(|op| op["selector"]["kind"].as_str().unwrap())
                .collect::<Vec<_>>(),
            selectors
        );
        for value in objects(instruction) {
            if let Some(text) = value.get("text").and_then(Value::as_str) {
                assert_eq!(source_text(&source, &value["source_span"]), text);
            }
        }
        assert_eq!(
            objects(&aat)
                .iter()
                .filter(|node| node["kind"] == "formatting_instruction")
                .count(),
            1
        );
        if selectors == ["each-quoted", "all-quoted"] {
            assert_eq!(
                operations[0]["selector"]["operands"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|op| op["text"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                ["x2", "y2", "x2"]
            );
            assert_eq!(operations[0]["effect"]["layout"]["kind"], "tcy");
            assert_eq!(operations[1]["effect"]["layout"]["style"], "superscript");
        }
        if selectors[0] == "character-position" {
            assert_eq!(operations[0]["selector"]["ordinal"], 2);
            assert_eq!(operations[1]["selector"]["ordinal"], 4);
        }
        if body.starts_with("現金") {
            assert_eq!(operations[0]["selector"], operations[1]["selector"]);
            assert_eq!(operations[0]["effect"]["layout"]["size"], "large");
            assert_eq!(operations[1]["effect"]["layout"]["style"], "bold");
            assert_eq!(source_text(&source, &operations[1]["source_span"]), "太字");
        }
        if body.starts_with("e-") {
            assert_eq!(operations[2]["effect"]["kind"], "upper-relative");
            assert_eq!(operations[2]["effect"]["base"]["text"], "e");
        }
    }
}

#[test]
fn horizontal_center_preserves_line_count_and_target_in_parser_ir() {
    let source = "題\r\n作者\r\n\r\n20［＃「20」は縦中横、２行の左右中央］\r\n";
    let (_, ir) = convert(source);
    let instruction = objects(&ir)
        .into_iter()
        .find(|node| node["type"] == "formatting-instruction")
        .expect("typed compound layout instruction");
    let operations = instruction["operations"].as_array().unwrap();

    assert_eq!(operations.len(), 2);
    assert_eq!(operations[0]["effect"]["layout"]["kind"], "tcy");
    assert_eq!(operations[1]["effect"]["kind"], "horizontal-center");
    assert_eq!(operations[1]["effect"]["lines"], 2);
    assert_eq!(
        source_text(source, &operations[1]["selection"]["targets"][0]),
        "20"
    );
}

#[test]
fn day_range_relative_placement_preserves_operands_and_resolved_anchors() {
    let source = "題\r\n作者\r\n\r\n１日\r\n２日\r\n３日\r\n４日\r\n５日\r\n６日\r\n７日\r\n８日\r\n９日\r\n10［＃「10」は縦中横］日\r\n注記［＃「注記」は２日〜10日の下に縦中横］\r\n";
    let (_, ir) = convert(source);
    let instruction = objects(&ir)
        .into_iter()
        .find(|node| {
            node["type"] == "formatting-instruction"
                && node["operations"].as_array().is_some_and(|operations| {
                    operations
                        .iter()
                        .any(|operation| operation["effect"]["kind"] == "relative-placement")
                })
        })
        .expect("relative placement instruction");
    let effect = &instruction["operations"][1]["effect"];

    assert_eq!(effect["relation"], "below");
    assert_eq!(effect["anchor"]["kind"], "day-range");
    assert_eq!(effect["anchor"]["start"]["day"], 2);
    assert_eq!(effect["anchor"]["end"]["day"], 10);
    assert_eq!(effect["anchor"]["start"]["operand"]["text"], "２");
    assert_eq!(effect["anchor"]["end"]["operand"]["text"], "10");
    assert_eq!(
        source_text(source, &effect["anchor"]["start"]["target"]),
        "２"
    );
    assert_eq!(
        source_text(source, &effect["anchor"]["end"]["target"]),
        "10"
    );
}

#[test]
fn single_global_and_relative_operations_reach_parser_ir() {
    for (body, selector, effect) in [
        (
            "s1 a2［＃アラビア数字はすべて下付き小文字］",
            "digits",
            "attribute",
        ),
        (
            "e-λt［＃「-λt」は「e」の上付き］",
            "quoted",
            "upper-relative",
        ),
    ] {
        let source = format!("題\r\n作者\r\n\r\n{body}\r\n");
        let (_, ir) = convert(&source);
        let instruction = objects(&ir)
            .into_iter()
            .find(|node| node["type"] == "formatting-instruction")
            .unwrap_or_else(|| panic!("typed instruction for {body}"));
        let operations = instruction["operations"].as_array().unwrap();
        assert_eq!(operations.len(), 1, "{body}");
        assert_eq!(operations[0]["selector"]["kind"], selector, "{body}");
        assert_eq!(operations[0]["effect"]["kind"], effect, "{body}");
    }
}

#[test]
fn corpus_small_and_end_alignment_compounds_reach_parser_ir() {
    for (body, offset, second_selector) in [
        ("江［＃「江」はポイント小さく右寄せ］", 0, "quoted"),
        (
            "昔の武蔵野今は東京府下［＃この行はポイントを下げ、「昔の武蔵野今は東京府下」は地より１１字上げ］",
            11,
            "quoted",
        ),
        (
            "本文［＃この行はポイントを下げ、は地より７字上げ］",
            7,
            "unresolved",
        ),
    ] {
        let source = format!("題\n作者\n\n{body}\n");
        let (aat, ir) = convert(&source);
        let instruction = objects(&ir)
            .into_iter()
            .find(|node| node["type"] == "formatting-instruction")
            .unwrap_or_else(|| panic!("typed instruction for {body}"));
        let operations = instruction["operations"].as_array().unwrap();
        assert_eq!(operations.len(), 2, "{body}");
        assert_eq!(operations[0]["effect"]["layout"]["kind"], "font-size");
        assert_eq!(operations[0]["effect"]["layout"]["direction"], "smaller");
        assert_eq!(operations[1]["selector"]["kind"], second_selector);
        assert_eq!(operations[1]["effect"]["layout"]["kind"], "chitsuki");
        assert_eq!(operations[1]["effect"]["layout"]["offset_from_end"], offset);
        if second_selector == "unresolved" {
            assert!(operations[0].get("selection").is_some());
            assert!(operations[1].get("selection").is_none());
            let native = objects(&aat)
                .into_iter()
                .find(|node| node["kind"] == "formatting_instruction")
                .unwrap();
            assert!(native.get("interpretation_problem").is_some());
        }
    }
}

#[test]
fn spatial_and_subtarget_instructions_reach_parser_ir_without_invented_targets() {
    for (body, selector, effect) in [
        (
            "［＃「注記」は「本文」の下にポイントを下げて２行で］",
            "quoted",
            "reduced-point-lines",
        ),
        (
            "一ノ戸［＃「一ノ戸」の「ノ」は小書き］",
            "subtarget",
            "attribute",
        ),
        ("［＃「「注―巻五十」は行右小書き］", "quoted", "attribute"),
    ] {
        let source = format!("題\r\n作者\r\n\r\n{body}\r\n");
        let (_, ir) = convert(&source);
        let instruction = objects(&ir)
            .into_iter()
            .find(|node| node["type"] == "formatting-instruction")
            .unwrap_or_else(|| panic!("typed instruction for {body}"));
        let operation = &instruction["operations"][0];
        assert_eq!(operation["selector"]["kind"], selector, "{body}");
        assert_eq!(operation["effect"]["kind"], effect, "{body}");
        if effect == "reduced-point-lines" {
            assert_eq!(operation["effect"]["lines"], 2);
            assert_eq!(operation["effect"]["placement"]["kind"], "below");
            assert_eq!(operation["effect"]["placement"]["anchor"]["text"], "本文");
            assert!(operation.get("selection").is_none());
        }
        if selector == "subtarget" {
            assert_eq!(operation["selector"]["enclosing"]["text"], "一ノ戸");
            assert_eq!(operation["selector"]["operands"][0]["text"], "ノ");
            assert_eq!(
                source_text(&source, &operation["selection"]["targets"][0]),
                "ノ"
            );
        }
    }
}

#[test]
fn rich_content_instructions_keep_owned_operands_after_fragment_rebasing() {
    let marker = "［＃「B1」は縦中横、「1」は下付き小文字］";
    for body in [
        format!("｜B1{marker}《びー》"),
        format!("字《B1{marker}》"),
        format!("［＃太字］｜B1{marker}《びー》［＃太字終わり］"),
        format!("字［＃「字」は底本では「｜B1{marker}《びー》」］"),
    ] {
        let source = format!("題\r\n作者\r\n\r\n前{body}後\r\n\r\n底本：本\r\n");
        let (_, ir) = convert(&source);
        let all = objects(&ir);
        let instruction: Vec<_> = all
            .iter()
            .filter(|node| node["type"] == "formatting-instruction")
            .collect();
        assert_eq!(instruction.len(), 1, "{body}\n{ir:#}");
        let instruction = instruction[0];
        assert_eq!(source_text(&source, &instruction["source_span"]), marker);
        for operation in instruction["operations"].as_array().unwrap() {
            let selection = &operation["selection"];
            assert_eq!(source_text(&source, &selection["scope"]), "B1");
            assert_eq!(selection["targets"].as_array().unwrap().len(), 1);
            assert_eq!(
                source_text(&source, &selection["targets"][0]),
                operation["selector"]["operands"][0]["text"]
                    .as_str()
                    .unwrap()
            );
            assert!(
                selection["scope"]["end"].as_u64().unwrap()
                    <= instruction["source_span"]["start"].as_u64().unwrap()
            );
            for operand in operation["selector"]["operands"].as_array().unwrap() {
                assert_eq!(
                    source_text(&source, &operand["source_span"]),
                    operand["text"].as_str().unwrap()
                );
                assert!(
                    operand["source_span"]["start"].as_u64().unwrap()
                        > instruction["source_span"]["start"].as_u64().unwrap()
                );
            }
        }
    }
}

#[test]
fn selectors_reject_missing_operands_and_incompatible_qualifiers() {
    for (schema, definition, span_key, span) in [
        (
            include_str!("../../../schemas/outputs/aat-schema.json"),
            "formatting_selector",
            "span",
            json!({"byte_start":1,"byte_end":2,"line_start":1,"line_end":1}),
        ),
        (
            include_str!("../../../schemas/parser-ir.schema.json"),
            "formattingSelector",
            "source_span",
            json!({"start":1,"end":2,"coordinate_system":"decoded_utf8","line":1}),
        ),
    ] {
        let schema: Value = serde_json::from_str(schema).unwrap();
        let validator = jsonschema::validator_for(&json!({"$schema":schema["$schema"],"$defs":schema["$defs"],"$ref":format!("#/$defs/{definition}")})).unwrap();
        let operand = json!({"text":"x",span_key:span});
        let valid = json!({"kind":"occurrence","ordinal":2,"operands":[operand]});
        assert!(validator.is_valid(&valid));
        for invalid in [
            json!({"kind":"occurrence","ordinal":0,"operands":[operand]}),
            json!({"kind":"quoted","ordinal":2,"operands":[operand]}),
            json!({"kind":"each-quoted","operands":[]}),
            json!({"kind":"all-quoted","operands":[operand,operand]}),
            json!({"kind":"digits","operands":[operand]}),
            json!({"kind":"arabic-digits-after","predecessor":{"kind":"alphabetic","operands":[operand]}}),
        ] {
            assert!(!validator.is_valid(&invalid), "{invalid}");
        }
    }
}

#[test]
fn selections_preserve_nested_targets_and_unresolved_operands() {
    for (body, expected) in [
        (
            "B1［＃「B1」は縦中横、「1」は下付き小文字］",
            vec![Some("B1"), Some("1")],
        ),
        (
            "B11［＃「B11」は縦中横、「1」は下付き小文字］",
            vec![Some("B11"), None],
        ),
        (
            "1/144［＃２つめの「4」は上付き小文字、「1/144」は分数］",
            vec![Some("4"), Some("1/144")],
        ),
        (
            "B2［＃「B1」は縦中横、「1」は下付き小文字］",
            vec![None, None],
        ),
    ] {
        let source = format!("題\r\n作者\r\n\r\n{body}\r\n");
        let (_, ir) = convert(&source);
        let all = objects(&ir);
        let instruction = all
            .into_iter()
            .find(|n| n["type"] == "formatting-instruction")
            .unwrap();
        for (operation, target) in instruction["operations"]
            .as_array()
            .unwrap()
            .iter()
            .zip(expected)
        {
            if let Some(target) = target {
                let selection = &operation["selection"];
                assert_eq!(source_text(&source, &selection["targets"][0]), target);
                assert!(
                    selection["scope"]["start"].as_u64()
                        <= selection["targets"][0]["start"].as_u64()
                );
                assert!(
                    selection["scope"]["end"].as_u64() >= selection["targets"][0]["end"].as_u64()
                );
            } else {
                assert!(operation.get("selection").is_none());
            }
        }
    }
}

#[test]
fn conversion_rejects_targets_outside_the_declared_body_scope() {
    fn alter(value: &mut Value) -> bool {
        match value {
            Value::Object(map) => {
                if let Some(selection) = map.get_mut("selection") {
                    let end = selection["scope"]["byte_end"].as_u64().unwrap();
                    selection["targets"][0]["byte_end"] = (end + 1).into();
                    return true;
                }
                map.values_mut().any(alter)
            }
            Value::Array(values) => values.iter_mut().any(alter),
            _ => false,
        }
    }
    let (mut aat, _) = convert("題\n作者\n\nB1［＃「B1」は縦中横、「1」は下付き小文字］\n");
    assert!(alter(&mut aat));
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let error = ab_aat_to_parser_ir::convert(ConversionRequest {
        aat,
        mapping: MappingDocument::from_path(&repo.join("data/aat-to-parser-ir-mapping-v2.json"))
            .unwrap(),
        schemas: SchemaSet::for_aat_version(2).unwrap(),
        options: ConversionOptions::default(),
    })
    .unwrap_err();
    assert!(format!("{error:#}").contains("contained in their scope"));
}

#[test]
fn relative_base_body_reference_is_retained_through_rich_content_rebasing() {
    let marker = "［＃「x」は太字、「-x」は「e」の上付き］";
    for body in [
        format!("e-x{marker}"),
        format!("｜e-x{marker}《しき》"),
        format!("式《e-x{marker}》"),
        format!("字［＃「字」は底本では「｜e-x{marker}《しき》」］"),
    ] {
        let source = format!("題\r\n作者\r\n\r\n前{body}後\r\n");
        let (_, ir) = convert(&source);
        let all = objects(&ir);
        let effect = all
            .into_iter()
            .find(|node| node["kind"] == "upper-relative")
            .unwrap();
        let base = &effect["base_target"];
        assert_eq!(source_text(&source, base), "e");
        assert_eq!(source_text(&source, &effect["base"]["source_span"]), "e");
        assert!(base["end"].as_u64() < effect["base"]["source_span"]["start"].as_u64());
        assert_eq!(&source[base["end"].as_u64().unwrap() as usize..][..2], "-x");
        assert!(effect.get("layout").is_none());
    }
}

#[test]
fn character_positions_keep_distinct_targets_after_fragment_rebasing() {
    let marker = "［＃２文字目の「i」は下付き小文字、４文字目の「i」は上付き小文字］";
    for body in [
        format!("確率は biνi{marker}"),
        format!("｜biνi{marker}《しき》"),
        format!("式《biνi{marker}》"),
        format!("字［＃「字」は底本では「｜biνi{marker}《しき》」］"),
    ] {
        let source = format!("題\r\n作者\r\n\r\n{body}\r\n");
        let (_, ir) = convert(&source);
        let all = objects(&ir);
        let instruction = all
            .into_iter()
            .find(|node| node["type"] == "formatting-instruction")
            .unwrap();
        let operations = instruction["operations"].as_array().unwrap();
        assert_eq!(operations.len(), 2);
        for (operation, offset) in operations.iter().zip([1, 4]) {
            let selection = &operation["selection"];
            assert_eq!(source_text(&source, &selection["scope"]), "biνi");
            assert_eq!(selection["targets"].as_array().unwrap().len(), 1);
            let target = &selection["targets"][0];
            assert_eq!(source_text(&source, target), "i");
            assert_eq!(
                target["start"].as_u64().unwrap(),
                selection["scope"]["start"].as_u64().unwrap() + offset
            );
            assert_eq!(
                selection["scope"]["end"],
                instruction["source_span"]["start"]
            );
        }
    }
    for body in [
        format!("｜biνi{marker}"),
        format!("｜biνi{marker}《》"),
        format!("｜bi［＃改行］νi{marker}《しき》"),
    ] {
        let source = format!("題\r\n作者\r\n\r\n{body}\r\n");
        let (_, ir) = convert(&source);
        let all = objects(&ir);
        let instruction = all
            .into_iter()
            .find(|node| node["type"] == "formatting-instruction")
            .unwrap();
        assert!(
            instruction["operations"]
                .as_array()
                .unwrap()
                .iter()
                .all(|operation| operation.get("selection").is_none()),
            "{body}"
        );
    }
}

#[test]
fn indexed_template_instances_are_retained_in_rich_content_without_rewriting_the_operand() {
    let marker = "［＃「Ａｎ」はそれぞれ縦中横、数字は上付き小書き］";
    for body in [
        format!("もしＡ２Ａ１０{marker}"),
        format!("｜Ａ２Ａ１０{marker}《こうい》"),
        format!("式《Ａ２Ａ１０{marker}》"),
        format!("字［＃「字」は底本では「｜Ａ２Ａ１０{marker}《こうい》」］"),
    ] {
        let source = format!("題\r\n作者\r\n\r\n{body}\r\n");
        let (_, ir) = convert(&source);
        let all = objects(&ir);
        let instruction = all
            .into_iter()
            .find(|node| node["type"] == "formatting-instruction")
            .unwrap();
        let operations = instruction["operations"].as_array().unwrap();
        assert_eq!(operations[0]["selector"]["kind"], "each-quoted");
        let operand = &operations[0]["selector"]["operands"][0];
        assert_eq!(operand["text"], "Ａｎ");
        assert_eq!(source_text(&source, &operand["source_span"]), "Ａｎ");
        for (operation, expected) in operations.iter().zip([["Ａ２", "Ａ１０"], ["２", "１０"]])
        {
            let selection = &operation["selection"];
            assert_eq!(source_text(&source, &selection["scope"]), "Ａ２Ａ１０");
            assert_eq!(
                selection["targets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|span| source_text(&source, span))
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(
                selection["scope"]["end"],
                instruction["source_span"]["start"]
            );
        }
    }
}

#[test]
fn rotation_angle_and_selection_are_retained_through_parser_ir_conversion() {
    let source = "題\r\n作者\r\n\r\n月二［＃「月二」は１８０度回転］\r\n";
    let (_, ir) = convert(source);
    let instruction = objects(&ir)
        .into_iter()
        .find(|node| node["type"] == "formatting-instruction")
        .unwrap();
    let operation = &instruction["operations"][0];
    assert_eq!(
        operation["effect"],
        json!({"kind":"rotation", "degrees":180})
    );
    assert_eq!(
        source_text(source, &operation["selector"]["operands"][0]["source_span"]),
        "月二"
    );
    assert_eq!(
        source_text(source, &operation["selection"]["scope"]),
        "月二"
    );
    assert_eq!(
        source_text(source, &operation["selection"]["targets"][0]),
        "月二"
    );
}

#[test]
fn equation_indices_keep_owned_scopes_through_rich_content() {
    let marker =
        "［＃ s に続くアラビア数字はすべて下付き小文字、νに続くアラビア数字はすべて上付き小文字］";
    for body in [
        format!("であるから P = s10 + ν20{marker}"),
        format!("｜P = s10 + ν20{marker}《しき》"),
        format!("式《P = s10 + ν20{marker}》"),
        format!("字［＃「字」は底本では「｜P = s10 + ν20{marker}《しき》」］"),
    ] {
        let source = format!("題\r\n作者\r\n\r\n{body}\r\n");
        let (_, ir) = convert(&source);
        let all = objects(&ir);
        let instruction = all
            .into_iter()
            .find(|node| node["type"] == "formatting-instruction")
            .unwrap();
        for (operation, (text, offset)) in instruction["operations"]
            .as_array()
            .unwrap()
            .iter()
            .zip([("10", 5), ("20", 12)])
        {
            let selection = &operation["selection"];
            assert_eq!(source_text(&source, &selection["scope"]), "P = s10 + ν20");
            assert_eq!(selection["targets"].as_array().unwrap().len(), 1);
            assert_eq!(source_text(&source, &selection["targets"][0]), text);
            assert_eq!(
                selection["targets"][0]["start"].as_u64().unwrap(),
                selection["scope"]["start"].as_u64().unwrap() + offset
            );
            assert_eq!(
                selection["scope"]["end"],
                instruction["source_span"]["start"]
            );
        }
    }
}

#[test]
fn inconsistent_repeated_references_preserve_selector_without_partial_selection() {
    let marker = "［＃「x2」、「y2」、「x2」はそれぞれ縦中横、すべての「2」は上付き小書き］";
    for body in [
        format!("例えば x2＋y2＝r2{marker}"),
        format!("｜x2＋y2＝r2{marker}《しき》"),
        format!("式《x2＋y2＝r2{marker}》"),
        format!("字［＃「字」は底本では「｜x2＋y2＝r2{marker}《しき》」］"),
    ] {
        let source = format!("題\r\n作者\r\n\r\n{body}\r\n");
        let (_, ir) = convert(&source);
        let all = objects(&ir);
        let instruction = all
            .into_iter()
            .find(|node| node["type"] == "formatting-instruction")
            .unwrap();
        let operation = &instruction["operations"][0];
        assert!(operation.get("selection").is_none());
        assert_eq!(
            operation["selector"]["operands"]
                .as_array()
                .unwrap()
                .iter()
                .map(|operand| operand["text"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["x2", "y2", "x2"]
        );
        let digit_targets = instruction["operations"][1]["selection"]["targets"]
            .as_array()
            .unwrap();
        assert_eq!(digit_targets.len(), 3);
        assert_eq!(
            digit_targets
                .iter()
                .map(|span| source_text(&source, span))
                .collect::<Vec<_>>(),
            ["2", "2", "2"]
        );
    }
}

#[test]
fn marker_accountability_requires_every_target_and_relative_base() {
    let schema = SchemaSet::for_aat_version(2).unwrap().aat_schema;
    let validator = jsonschema::validator_for(&json!({"$schema":schema["$schema"], "$defs":schema["$defs"], "$ref":"#/$defs/formatting_instruction"})).unwrap();
    let cases = [
        (
            "B1",
            "［＃「B1」は縦中横、「1」は下付き小文字］",
            vec!["emphasis"],
            2,
        ),
        ("B1", "［＃「B1」は縦中横、「2」は下付き小文字］", vec![], 1),
        (
            "B11",
            "［＃「B11」は縦中横、「1」は下付き小文字］",
            vec![],
            1,
        ),
        (
            "abc",
            "［＃「B1」は縦中横、「1」は下付き小文字］",
            vec![],
            0,
        ),
        (
            "e-r2/4Dt",
            "［＃「2」は上付き小文字、「r2/4Dt」は分数、「-r2/4Dt」は「e」の上付き］",
            vec!["emphasis", "exponent", "layout"],
            3,
        ),
        (
            "q-r2/4Dt",
            "［＃「2」は上付き小文字、「r2/4Dt」は分数、「-r2/4Dt」は「e」の上付き］",
            vec![],
            3,
        ),
        (
            "B1",
            "［＃「B1」は横組み、罫囲み、分数、縦中横、大文字、太字、斜体、「1」は下付き小文字］",
            vec!["emphasis", "layout"],
            8,
        ),
    ];
    for (text, marker, kinds, selected) in cases {
        for body in [
            format!("{text}{marker}"),
            format!("｜{text}{marker}《しき》"),
            format!("式《{text}{marker}》"),
            format!("字［＃「字」は底本では「｜{text}{marker}《しき》」］"),
        ] {
            let source = format!("題\r\n作者\r\n\r\n{body}\r\n");
            let (aat, ir) = convert(&source);
            let all = objects(&ir);
            let instruction = all
                .iter()
                .find(|node| node["type"] == "formatting-instruction")
                .unwrap();
            assert_eq!(
                instruction["operations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|op| op.get("selection").is_some())
                    .count(),
                selected,
                "{body}"
            );
            let facts: Vec<_> = ir["interpretation_facts"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|fact| fact["source_span"] == instruction["source_span"])
                .collect();
            assert_eq!(
                facts
                    .iter()
                    .map(|fact| fact["kind"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                kinds,
                "{body}"
            );
            for fact in &facts {
                assert_eq!(source_text(&source, &fact["source_span"]), marker);
                assert_eq!(fact["outcome"], "established");
                assert_eq!(
                    fact["aspects"],
                    if fact["kind"] == "exponent" {
                        json!(["structure"])
                    } else {
                        json!(["layout"])
                    }
                );
            }
            let problems: Vec<_> = ir["interpretation_problems"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|problem| problem["raw"] == marker)
                .collect();
            assert_eq!(problems.len(), usize::from(kinds.is_empty()), "{body}");
            if let Some(problem) = problems.first() {
                assert_eq!(problem["source_span"], instruction["source_span"]);
            }
            let native = objects(&aat)
                .into_iter()
                .find(|node| node["kind"] == "formatting_instruction")
                .unwrap();
            assert_eq!(
                native.get("interpretation_problem").is_some(),
                kinds.is_empty(),
                "{body}"
            );
            let mut without_problem = native.clone();
            without_problem
                .as_object_mut()
                .unwrap()
                .remove("interpretation_problem");
            assert_eq!(
                validator.is_valid(&without_problem),
                !kinds.is_empty(),
                "{body}"
            );
        }
    }
}

#[test]
fn kunten_policy_preserves_scope_exclusions_and_source_coordinates() {
    let body = "「一言」の一レ［＃「一言」の「一」をのぞいて「レ一二三」は返り点、以下同じ］";
    let source = format!("題\r\n作者\r\n\r\n{body}\r\n");
    let (_, ir) = convert(&source);
    let instruction = objects(&ir)
        .into_iter()
        .find(|node| node["type"] == "formatting-instruction")
        .expect("typed return-mark policy");
    let operation = &instruction["operations"][0];
    let effect = &operation["effect"];

    assert_eq!(operation["selector"]["kind"], "unresolved");
    assert_eq!(effect["kind"], "kunten-policy");
    assert_eq!(effect["scope"], "quoted-region");
    assert_eq!(effect["repeat_following"], true);
    assert_eq!(effect["vertical_rule_right"], false);
    assert_eq!(
        source_text(&source, &effect["marks"]["source_span"]),
        "レ一二三"
    );
    assert_eq!(effect["marks"]["text"], "レ一二三");

    let exclusions = effect["exclusions"].as_array().unwrap();
    assert_eq!(exclusions.len(), 1);
    assert_eq!(
        source_text(&source, &exclusions[0]["enclosing"]["source_span"]),
        "一言"
    );
    assert_eq!(
        source_text(&source, &exclusions[0]["mark"]["source_span"]),
        "一"
    );
}
