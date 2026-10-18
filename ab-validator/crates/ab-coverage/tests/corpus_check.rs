use std::collections::BTreeSet;

use ab_coverage::{
    corpus_check::{
        BaselineComparison, CheckOutcome, CompiledCheck, CorpusCheck, Discrimination,
        obligation_checks,
    },
    occurrence_alignment::LocatedOccurrence,
    source_inventory::SourceMarkerOccurrence,
};
use serde_json::{Value, json};

const REQUIREMENTS: &str = include_str!("../../../data/fidelity-requirements.toml");

/// Parser output for `其の喋［＃「喋」に「ママ」の注記］る`.
fn reviewed_mama() -> Value {
    json!({"blocks": [{"kind": "paragraph", "content": [
        {"kind": "text", "span": {"byte_start": 0, "byte_end": 6}, "value": "其の"},
        {"kind": "annotated_text", "note_kind": "gloss",
         "span": {"byte_start": 9, "byte_end": 51},
         "content": [{"kind": "text", "span": {"byte_start": 6, "byte_end": 9}, "value": "喋"}],
         "annotation_content": [{"kind": "text", "span": {"byte_start": 30, "byte_end": 36}, "value": "ママ"}]},
        {"kind": "text", "span": {"byte_start": 51, "byte_end": 55}, "value": "る\n"},
    ]}]})
}

const SOURCE: &str = "其の喋［＃「喋」に「ママ」の注記］る\n";
const MARKER: &str = "［＃「喋」に「ママ」の注記］";

fn check() -> CompiledCheck {
    CompiledCheck::new(CorpusCheck {
        partitions: vec![],
        operands: "^［＃「(?P<target>[^「」]+)」に「(?P<note>[^「」]+)」の注記］$".to_owned(),
        kind: "annotated_text".to_owned(),
        fields: [("note_kind".to_owned(), "gloss".to_owned())].into(),
        text: [
            ("content".to_owned(), "{target}".to_owned()),
            ("annotation_content".to_owned(), "{note}".to_owned()),
        ]
        .into(),
        target_before_marker: Some("content".to_owned()),
        marker_span: true,
        marker_text: false,
    })
    .unwrap()
}

#[test]
fn every_mutation_of_the_checked_projection_fails_the_check() {
    type Mutation = fn(&mut Value);
    let cases: [(&str, Mutation, &str); 8] = [
        ("unchanged", |_| {}, "satisfied"),
        (
            "note left as a raw node",
            |aat| aat["blocks"][0]["content"][1]["kind"] = json!("raw"),
            "no-node",
        ),
        (
            "wrong note kind",
            |aat| aat["blocks"][0]["content"][1]["note_kind"] = json!("sic"),
            "field-mismatch:note_kind",
        ),
        (
            "note value lost",
            |aat| aat["blocks"][0]["content"][1]["annotation_content"] = json!([]),
            "text-mismatch:annotation_content",
        ),
        (
            "target text changed",
            |aat| aat["blocks"][0]["content"][1]["content"][0]["value"] = json!("其の喋"),
            "text-mismatch:content",
        ),
        (
            "target detached from the marker",
            |aat| {
                aat["blocks"][0]["content"][1]["content"][0]["span"] =
                    json!({"byte_start": 0, "byte_end": 3});
            },
            "target-not-before-marker:content",
        ),
        (
            "node span not the marker",
            |aat| {
                aat["blocks"][0]["content"][1]["span"] = json!({"byte_start": 6, "byte_end": 51});
            },
            "span-not-marker",
        ),
        (
            "duplicated node",
            |aat| {
                let copy = aat["blocks"][0]["content"][1].clone();
                aat["blocks"][0]["content"]
                    .as_array_mut()
                    .unwrap()
                    .push(copy);
            },
            "ambiguous-node",
        ),
    ];
    let check = check();
    for (name, mutate, expected) in cases {
        let mut aat = reviewed_mama();
        mutate(&mut aat);
        assert_eq!(
            check.evaluate(&aat, SOURCE, 9, MARKER).label(),
            expected,
            "{name}"
        );
    }
    assert_eq!(
        check
            .evaluate(
                &reviewed_mama(),
                "其の喋［＃「喋」はママ］る\n",
                9,
                "［＃「喋」はママ］"
            )
            .label(),
        "outside-operands"
    );
}

#[test]
fn a_target_is_the_nearest_preceding_occurrence_of_its_text() {
    let source = "其の漢字《かんじ》［＃「字」に「ママ」の注記］\n";
    let marker = "［＃「字」に「ママ」の注記］";
    let start = source.find(marker).unwrap();
    let annotated = |start: usize, target: Value| {
        json!([{"kind": "annotated_text", "note_kind": "gloss",
            "span": {"byte_start": start, "byte_end": start + marker.len()},
            "content": [target],
            "annotation_content": [{"kind": "text", "value": "ママ"}]}])
    };
    let ruby = annotated(
        start,
        json!({"kind": "ruby", "base": "字", "reading": "かんじ",
                                "span": {"byte_start": 9, "byte_end": 12}}),
    );
    assert_eq!(
        check().evaluate(&ruby, source, start, marker).label(),
        "satisfied"
    );

    let emphasised = "字［＃「字」に傍線］［＃「字」に「ママ」の注記］\n";
    let start = emphasised.find(marker).unwrap();
    let target = annotated(
        start,
        json!({"kind": "text", "value": "字",
                                  "span": {"byte_start": 0, "byte_end": 3}}),
    );
    assert_eq!(
        check().evaluate(&target, emphasised, start, marker).label(),
        "satisfied"
    );

    let repeated = "字と字［＃「字」に「ママ」の注記］\n";
    let start = repeated.find(marker).unwrap();
    let first = annotated(
        start,
        json!({"kind": "text", "value": "字",
                                 "span": {"byte_start": 0, "byte_end": 3}}),
    );
    assert_eq!(
        check().evaluate(&first, repeated, start, marker).label(),
        "target-not-before-marker:content"
    );
}

#[test]
fn a_marker_quoted_inside_another_marker_is_left_to_that_marker() {
    let marker = "［＃「四」に「マヽ」の注記］";
    let source = format!("十四{marker}日［＃「十四{marker}日」に二重傍線］\n");
    let nested = source.rfind(marker).unwrap();
    assert_eq!(
        check()
            .evaluate(&json!([]), &source, nested, marker)
            .label(),
        "nested-in-marker"
    );
    assert_eq!(
        check()
            .evaluate(&json!([]), &source, source.find(marker).unwrap(), marker)
            .label(),
        "no-node"
    );
}

#[test]
fn checked_in_corpus_checks_compile_and_select_occurrences_by_partition() {
    let checks = obligation_checks(REQUIREMENTS).unwrap();
    assert!(!checks.is_empty());
    let check = &checks[0];
    let occurrence = |row_id: &str, partitions: &[String]| LocatedOccurrence {
        row_id: row_id.to_owned(),
        form: None,
        partitions: partitions.iter().cloned().collect::<BTreeSet<_>>(),
        occurrence: SourceMarkerOccurrence {
            work_id: "w".to_owned(),
            indexed_path: None,
            line: 1,
            kind: "Annotation".to_owned(),
            raw: String::new(),
            body: String::new(),
            byte_start: Some(0),
        },
        byte_start: 0,
    };
    let partitions = &check.check.check.partitions;
    assert!(check.applies(&occurrence(&check.row_id, partitions)));
    assert!(!check.applies(&occurrence("other.row", partitions)));
    assert!(!check.applies(&occurrence(&check.row_id, &partitions[1..])));
    let unpartitioned = checks
        .iter()
        .find(|check| check.check.check.partitions.is_empty())
        .unwrap();
    assert!(unpartitioned.applies(&occurrence(&unpartitioned.row_id, &[])));
    assert!(!unpartitioned.applies(&occurrence("other.row", &[])));
}

#[test]
fn an_unanchored_note_whose_quoted_text_the_source_lacks_is_not_checked() {
    let checks = obligation_checks(REQUIREMENTS).unwrap();
    let check = &checks
        .iter()
        .find(|check| check.obligation_id == "ruby.basic.base-edition-reading")
        .unwrap()
        .check;
    let outcome = |source: &str, marker: &str| {
        let start = source.find(marker).unwrap();
        let aat = json!({"blocks": [{"kind": "paragraph", "content": [
            {"kind": "raw", "source": marker,
             "span": {"byte_start": start, "byte_end": start + marker.len()}},
        ]}]});
        check.evaluate(&aat, source, start, marker)
    };
    let absent = CheckOutcome::TargetAbsentFromSource {
        array: "content".to_owned(),
    };
    assert!(!absent.failed());
    let reading = "［＃「乾燥無味《かんそうむみ》」は底本では「乾燦無味《かんさうむみ》」］";
    let extent = "［＃「瑤泉院《ようぜいいん》の」は底本では「冷泉院《れいぜいいん》の」］";
    for (source, marker, expected) in [
        (
            format!("極めて乾燥無味《かんさうむみ》{reading}の"),
            reading,
            &absent,
        ),
        (
            format!("内室｜瑤泉院《ようぜいいん》{extent}化粧料"),
            extent,
            &absent,
        ),
        (
            format!("極めて乾燥無味《かんそうむみ》{reading}の"),
            reading,
            &CheckOutcome::NoNode,
        ),
        (
            format!("内室｜瑤泉院《ようぜいいん》の{extent}化粧料"),
            extent,
            &CheckOutcome::NoNode,
        ),
    ] {
        assert_eq!(&outcome(&source, marker), expected, "{source}");
    }
}

/// A sic note whose target is an array of source spans rather than child
/// nodes: the nearest preceding 喋 satisfies the check, an earlier one does
/// not.
#[test]
fn a_span_array_target_is_the_nearest_preceding_occurrence_of_its_source() {
    let check = CompiledCheck::new(CorpusCheck {
        partitions: vec![],
        operands: "^［＃「(?P<target>[^「」]+)」はママ］$".to_owned(),
        kind: "editorial_note".to_owned(),
        fields: [
            ("note_kind".to_owned(), "sic".to_owned()),
            ("target_text".to_owned(), "{target}".to_owned()),
        ]
        .into(),
        text: [].into(),
        target_before_marker: Some("target_source_spans".to_owned()),
        marker_span: true,
        marker_text: false,
    })
    .unwrap();
    let marker = "［＃「喋」はママ］";
    let source = format!("喋の喋{marker}る\n");
    let aat = |target_start: usize| {
        json!({"blocks": [{"kind": "paragraph", "content": [
            {"kind": "text", "span": {"byte_start": 0, "byte_end": 9}, "value": "喋の喋"},
            {"kind": "editorial_note", "note_kind": "sic", "text": "「喋」はママ",
             "target_text": "喋",
             "target_source_spans": [{"byte_start": target_start, "byte_end": target_start + 3}],
             "span": {"byte_start": 9, "byte_end": 9 + marker.len()}},
        ]}]})
    };
    assert_eq!(
        check.evaluate(&aat(6), &source, 9, marker).label(),
        "satisfied"
    );
    assert_eq!(
        check.evaluate(&aat(0), &source, 9, marker).label(),
        "target-not-before-marker:target_source_spans"
    );
}

/// A paired heading close: the content ends at the close whether it is a
/// resolved gaiji or a text variant whose marker follows the close.
#[test]
fn a_node_array_target_ends_where_its_principal_content_ends() {
    let check = CompiledCheck::new(CorpusCheck {
        partitions: vec![],
        operands: "^［＃同行中見出し終わり］$".to_owned(),
        kind: "heading".to_owned(),
        fields: [].into(),
        text: [].into(),
        target_before_marker: Some("content".to_owned()),
        marker_span: false,
        marker_text: false,
    })
    .unwrap();
    let close = "［＃同行中見出し終わり］";
    let gaiji_source = format!("［＃同行中見出し］※［＃ローマ数字1、1-13-21］{close}");
    let variant_source =
        format!("［＃同行中見出し］P.202{close}［＃「P.202」は底本では「P.250」］");
    let heading = |source: &str, content: Value| {
        json!({"blocks": [{"kind": "paragraph", "content": [
            {"kind": "heading", "span": {"byte_start": 0, "byte_end": source.find(close).unwrap() + close.len()},
             "content": content},
        ]}]})
    };
    let gaiji_end = gaiji_source.find(close).unwrap();
    let gaiji = |resolved: Value| {
        json!([{"kind": "gaiji", "resolved": resolved,
                "span": {"byte_start": 27, "byte_end": gaiji_end}}])
    };
    let variant_end = variant_source.len();
    let variant = json!([{"kind": "text-variant",
        "span": {"byte_start": 68, "byte_end": variant_end},
        "content": [{"kind": "text", "value": "P.202", "span": {"byte_start": 27, "byte_end": 32}}]}]);
    let cases = [
        (
            "resolved gaiji",
            &gaiji_source,
            gaiji(json!("Ⅰ")),
            "satisfied",
        ),
        (
            "unresolved gaiji",
            &gaiji_source,
            gaiji(Value::Null),
            "target-not-before-marker:content",
        ),
        ("text variant", &variant_source, variant, "satisfied"),
    ];
    for (name, source, content, expected) in cases {
        assert_eq!(
            check
                .evaluate(
                    &heading(source, content),
                    source,
                    source.find(close).unwrap(),
                    close
                )
                .label(),
            expected,
            "{name}"
        );
    }
}

#[test]
fn baseline_comparison_pairs_checked_outcomes_and_skips_unchecked_ones() {
    use BaselineComparison::{BothFailed, BothSatisfied, Gained, Lost};
    use CheckOutcome::{NestedInMarker, NoNode, OutsideOperands, Satisfied};
    let span = CheckOutcome::SpanNotMarker { span: None };
    let cases = [
        (
            "satisfied on both",
            Satisfied,
            Satisfied,
            Some(BothSatisfied),
        ),
        (
            "failed on both with different outcomes",
            NoNode,
            span.clone(),
            Some(BothFailed),
        ),
        (
            "failed on the baseline only",
            NoNode,
            Satisfied,
            Some(Gained),
        ),
        ("failed on the candidate only", Satisfied, span, Some(Lost)),
        (
            "outside the operands on the baseline",
            OutsideOperands,
            NoNode,
            None,
        ),
        (
            "nested in a marker on the candidate",
            Satisfied,
            NestedInMarker,
            None,
        ),
    ];
    for (case, baseline, candidate, expected) in cases {
        assert_eq!(
            BaselineComparison::between(&baseline, &candidate),
            expected,
            "{case}"
        );
    }
}

#[test]
fn discrimination_counts_each_comparison_and_flips_only_on_gained_or_lost() {
    use BaselineComparison::{BothFailed, BothSatisfied, Gained, Lost};
    let cases: [(&str, &[BaselineComparison], bool); 4] = [
        (
            "unchanged outcomes",
            &[BothSatisfied, BothFailed, BothSatisfied],
            false,
        ),
        ("a gained occurrence", &[BothFailed, Gained], true),
        ("a lost occurrence", &[BothSatisfied, Lost], true),
        ("no occurrence", &[], false),
    ];
    for (case, comparisons, flipped) in cases {
        let mut discrimination = Discrimination::default();
        for comparison in comparisons {
            discrimination.add(*comparison);
        }
        let count = |wanted| comparisons.iter().filter(|c| **c == wanted).count() as u64;
        assert_eq!(
            discrimination,
            Discrimination {
                both_satisfied: count(BothSatisfied),
                both_failed: count(BothFailed),
                gained: count(Gained),
                lost: count(Lost),
            },
            "{case}"
        );
        assert_eq!(discrimination.flipped(), flipped, "{case}");
    }
}

/// An indentation block around a 字詰め block: both overlap the marker at
/// bytes 10..43, and only the inner one lists it among its interpretation
/// markers.
#[test]
fn of_overlapping_nodes_the_one_interpreting_the_marker_is_anchored() {
    let check = CompiledCheck::new(CorpusCheck {
        partitions: vec![],
        operands: "(?P<width>[0-9０-９]+)字詰め".to_owned(),
        kind: "layout_block".to_owned(),
        fields: [("width".to_owned(), "{width}".to_owned())].into(),
        text: Default::default(),
        target_before_marker: None,
        marker_span: false,
        marker_text: false,
    })
    .unwrap();
    let marker = "［＃ここから４字詰め］";
    let source = format!("{}{marker}{}", "x".repeat(10), "x".repeat(20));
    let outcome = |inner_markers: Value| {
        let aat = json!({"blocks": [{"kind": "layout_block", "indent": 2,
            "span": {"byte_start": 0, "byte_end": 63},
            "interpretation_marker_spans": [{"byte_start": 0, "byte_end": 10},
                                            {"byte_start": 53, "byte_end": 63}],
            "children": [{"kind": "layout_block", "width": 4,
                "span": {"byte_start": 10, "byte_end": 53},
                "interpretation_marker_spans": inner_markers}]}]});
        check.evaluate(&aat, &source, 10, marker)
    };
    assert_eq!(
        outcome(json!([{"byte_start": 10, "byte_end": 43}])),
        CheckOutcome::Satisfied
    );
    assert_eq!(outcome(json!([])), CheckOutcome::AmbiguousNode { nodes: 2 });
}

#[test]
fn a_node_interpreting_a_marker_it_does_not_span_is_anchored() {
    let check = CompiledCheck::new(CorpusCheck {
        partitions: vec![],
        operands: "^［＃「(?P<target>[^「」]+)」は斜体］$".to_owned(),
        kind: "style".to_owned(),
        fields: [("style_type".to_owned(), "italic".to_owned())].into(),
        text: [("content".to_owned(), "{target}".to_owned())].into(),
        target_before_marker: Some("content".to_owned()),
        marker_span: false,
        marker_text: false,
    })
    .unwrap();
    let source = "「男竹」［＃「男竹」は斜体］";
    let marker = "［＃「男竹」は斜体］";
    let start = source.find(marker).unwrap();
    let outcome = |marker_spans: Value| {
        let aat = json!({"blocks": [{"kind": "paragraph", "content": [
            {"kind": "text", "span": {"byte_start": 0, "byte_end": 3}, "value": "「"},
            {"kind": "style", "style_type": "italic",
             "span": {"byte_start": 3, "byte_end": 9},
             "interpretation_marker_spans": marker_spans,
             "content": [{"kind": "text", "span": {"byte_start": 3, "byte_end": 9}, "value": "男竹"}]},
            {"kind": "text", "span": {"byte_start": 9, "byte_end": 12}, "value": "」"},
        ]}]});
        check.evaluate(&aat, source, start, marker)
    };
    assert_eq!(
        outcome(json!([{"byte_start": start, "byte_end": source.len()}])),
        CheckOutcome::Satisfied
    );
    assert_eq!(outcome(json!([])), CheckOutcome::NoNode);
}

#[test]
fn a_numeric_field_reads_fullwidth_operand_digits_as_ascii() {
    let marker = "［＃ここから１字下げ、折り返して１２字下げ］";
    let check = CompiledCheck::new(CorpusCheck {
        partitions: vec![],
        operands: "折り返して(?P<continuation>[0-9０-９]+)字下げ".to_owned(),
        kind: "layout_block".to_owned(),
        fields: [(
            "continuation_indent".to_owned(),
            "{continuation}".to_owned(),
        )]
        .into(),
        text: Default::default(),
        target_before_marker: None,
        marker_span: false,
        marker_text: false,
    })
    .unwrap();
    let outcome = |continuation_indent: Value| {
        let aat = json!({"blocks": [{"kind": "layout_block",
            "continuation_indent": continuation_indent,
            "span": {"byte_start": 0, "byte_end": marker.len()}}]});
        check.evaluate(&aat, marker, 0, marker)
    };
    assert_eq!(outcome(json!(12)), CheckOutcome::Satisfied);
    assert_eq!(
        outcome(json!(2)),
        CheckOutcome::FieldMismatch {
            field: "continuation_indent".to_owned(),
            expected: "12".to_owned(),
            actual: "2".to_owned(),
        }
    );
    assert_eq!(
        outcome(json!("１２")),
        CheckOutcome::Satisfied,
        "a string field compares the operand's own digits"
    );
}

/// A frame block encloses the handwritten block; without the inner block the
/// frame is the only node overlapping the marker.
#[test]
fn a_dotted_field_reads_a_nested_object() {
    let marker = "［＃ここから手書き文字］";
    let check = CompiledCheck::new(CorpusCheck {
        partitions: vec![],
        operands: "^［＃ここから手書き文字］$".to_owned(),
        kind: "typography_block".to_owned(),
        fields: [("formatting.style_type".to_owned(), "handwritten".to_owned())].into(),
        text: Default::default(),
        target_before_marker: None,
        marker_span: false,
        marker_text: false,
    })
    .unwrap();
    let frame = |children: Value| {
        json!({"blocks": [{"kind": "typography_block",
            "formatting": {"kind": "keigakomi", "border": "rule"},
            "span": {"byte_start": 0, "byte_end": marker.len()},
            "children": children}]})
    };
    let handwritten = json!([{"kind": "typography_block",
        "formatting": {"kind": "style", "style_type": "handwritten"},
        "span": {"byte_start": 0, "byte_end": marker.len()},
        "interpretation_marker_spans": [{"byte_start": 0, "byte_end": marker.len()}]}]);
    assert_eq!(
        check.evaluate(&frame(handwritten), marker, 0, marker),
        CheckOutcome::Satisfied
    );
    assert_eq!(
        check.evaluate(&frame(json!([])), marker, 0, marker),
        CheckOutcome::FieldMismatch {
            field: "formatting.style_type".to_owned(),
            expected: "handwritten".to_owned(),
            actual: String::new(),
        }
    );
}

/// A reading target competes only with readings between it and the marker:
/// plain text repeating the kana does not displace it, a later reading does.
#[test]
fn a_reading_target_is_the_nearest_preceding_reading_occurrence() {
    let check = CompiledCheck::new(CorpusCheck {
        partitions: vec![],
        operands: "^［＃ルビの「(?P<target>[^「」]+)」はママ］$".to_owned(),
        kind: "editorial_note".to_owned(),
        fields: [
            ("note_kind".to_owned(), "sic".to_owned()),
            ("target_text".to_owned(), "{target}".to_owned()),
        ]
        .into(),
        text: [].into(),
        target_before_marker: Some("target_source_spans".to_owned()),
        marker_span: true,
        marker_text: false,
    })
    .unwrap();
    let marker = "［＃ルビの「な」はママ］";
    let evaluate = |source: &str| {
        let reading = source.find("《な》").unwrap() + 3;
        let start = source.find(marker).unwrap();
        let aat = json!({"blocks": [{"kind": "paragraph", "content": [
            {"kind": "editorial_note", "note_kind": "sic", "target_text": "な",
             "target_source_spans": [{"byte_start": reading, "byte_end": reading + 3}],
             "span": {"byte_start": start, "byte_end": start + marker.len()}},
        ]}]});
        check.evaluate(&aat, source, start, marker).label()
    };
    assert_eq!(
        evaluate(&format!("成《な》んなんど{marker}\n")),
        "satisfied"
    );
    assert_eq!(
        evaluate(&format!("成《な》ん名《な》{marker}\n")),
        "target-not-before-marker:target_source_spans"
    );
}

/// A caption range opened and closed on one line is a caption node; one that
/// spans lines is a caption block.
#[test]
fn a_check_anchors_a_node_of_any_listed_kind() {
    let check = CompiledCheck::new(CorpusCheck {
        partitions: vec![],
        operands: "^［＃ここからキャプション］$".to_owned(),
        kind: "caption_block|caption".to_owned(),
        fields: Default::default(),
        text: Default::default(),
        target_before_marker: None,
        marker_span: false,
        marker_text: false,
    })
    .unwrap();
    let marker = "［＃ここからキャプション］";
    let outcome = |kind: &str| {
        let aat = json!({"blocks": [{"kind": kind,
            "span": {"byte_start": 0, "byte_end": marker.len() + 10}}]});
        check.evaluate(&aat, marker, 0, marker)
    };
    assert_eq!(outcome("caption_block"), CheckOutcome::Satisfied);
    assert_eq!(outcome("caption"), CheckOutcome::Satisfied);
    assert_eq!(outcome("paragraph"), CheckOutcome::NoNode);
}

/// An expected value reads as printed text: a quoted gaiji marker matches
/// the gaiji node that resolves it but not the marker left as literal text,
/// and a quoted ruby reading or annotation contributes no text.
#[test]
fn an_expected_text_reads_as_printed_text() {
    let check = CompiledCheck::new(CorpusCheck {
        partitions: vec![],
        operands: "^［＃「(?P<target>.+)」は底本では「(?P<base>.+)」］$".to_owned(),
        kind: "text-variant".to_owned(),
        fields: [].into(),
        text: [("base_content".to_owned(), "{base}".to_owned())].into(),
        target_before_marker: None,
        marker_span: true,
        marker_text: false,
    })
    .unwrap();
    let evaluate = |quoted_base: &str, base: Value| {
        let marker = format!("［＃「骨壷」は底本では「{quoted_base}」］");
        let source = format!("骨壷{marker}\n");
        let start = "骨壷".len();
        let aat = json!({"blocks": [{"kind": "paragraph", "content": [
            {"kind": "text-variant", "base_content": base,
             "span": {"byte_start": start, "byte_end": start + marker.len()}},
        ]}]});
        check.evaluate(&aat, &source, start, &marker).label()
    };
    let gaiji = "骨※［＃「てへん＋兩」、U+639A］";
    assert_eq!(
        evaluate(
            gaiji,
            json!([{"kind": "text", "value": "骨"}, {"kind": "gaiji", "resolved": "掚"}])
        ),
        "satisfied"
    );
    assert_eq!(
        evaluate(gaiji, json!([{"kind": "text", "value": gaiji}])),
        "text-mismatch:base_content"
    );
    assert_eq!(
        evaluate(
            gaiji,
            json!([{"kind": "text", "value": "骨"}, {"kind": "gaiji", "resolved": null}])
        ),
        "text-mismatch:base_content"
    );
    assert_eq!(
        evaluate(
            "骨壺《こつつぼ》［＃「壺」に傍点］",
            json!([
                {"kind": "ruby", "base": "骨壺", "reading": "こつつぼ",
                 "base_content": [{"kind": "text", "value": "骨壺"}]},
            ])
        ),
        "satisfied"
    );
    let unresolved = "骨※［＃「壼」の「亞」に代えて「亜」、58-下-13］";
    assert_eq!(
        evaluate(
            &format!("{unresolved}《こつつぼ》"),
            json!([
                {"kind": "ruby", "base": unresolved, "reading": "こつつぼ",
                 "base_content": [{"kind": "text", "value": "骨"},
                                  {"kind": "gaiji", "resolved": null}]},
            ])
        ),
        "satisfied"
    );
}

#[test]
fn a_marker_text_check_requires_the_brackets_in_text_nodes_that_reproduce_the_source() {
    let check = CompiledCheck::new(CorpusCheck {
        partitions: vec![],
        operands: "^〔".to_owned(),
        kind: "text".to_owned(),
        fields: Default::default(),
        text: Default::default(),
        target_before_marker: None,
        marker_span: false,
        marker_text: true,
    })
    .unwrap();
    let text = |start: usize, end: usize, value: &str| json!({"kind": "text", "span": {"byte_start": start, "byte_end": end}, "value": value});
    let outcome = |source: &str, start: usize, marker: &str, content: Vec<Value>| {
        let aat = json!({"blocks": [{"kind": "paragraph", "content": content}]});
        check.evaluate(&aat, source, start, marker)
    };
    let one_line = "前〔発信〕後\n";
    assert_eq!(
        outcome(one_line, 3, "〔発信〕", vec![text(0, 19, "前〔発信〕後\n")]),
        CheckOutcome::Satisfied
    );
    assert_eq!(
        outcome(one_line, 3, "〔発信〕", vec![text(0, 19, "前発信後\n")]),
        CheckOutcome::TextNotMarker
    );
    let two_lines = "〔甲\r\n乙〕\r\n";
    assert_eq!(
        outcome(
            two_lines,
            0,
            "〔甲\r\n乙〕",
            vec![text(0, 8, "〔甲\n"), text(8, 16, "乙〕\n")]
        ),
        CheckOutcome::Satisfied,
        "one text node per line"
    );
    assert_eq!(
        outcome(
            two_lines,
            0,
            "〔甲\r\n乙〕",
            vec![text(0, 8, "〔甲"), text(8, 16, "乙〕\n")]
        ),
        CheckOutcome::Satisfied,
        "a line ending before a block close has no terminator in its text"
    );
    assert_eq!(
        outcome(
            two_lines,
            0,
            "〔甲\r\n乙〕",
            vec![text(0, 8, "〔乙\n"), text(8, 16, "乙〕\n")]
        ),
        CheckOutcome::TextNotMarker
    );
    let accent = "〔“e'〕";
    assert_eq!(
        outcome(accent, 0, accent, vec![text(3, 6, "“")]),
        CheckOutcome::TextNotMarker,
        "brackets read as an accent scope"
    );
    let literal = "［『書簡』〔号〕附録］";
    let start = literal.find('〔').unwrap();
    assert_eq!(
        outcome(
            literal,
            start,
            "〔号〕",
            vec![text(0, literal.len(), literal)]
        ),
        CheckOutcome::Satisfied,
        "a literal bracket opens no annotation"
    );
}
