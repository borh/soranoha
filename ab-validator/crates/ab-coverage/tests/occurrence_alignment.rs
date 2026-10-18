use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    process::Command,
};

use ab_coverage::{
    CoverageMatrix,
    occurrence_alignment::{
        AlignmentNode, AlignmentStatus, FormClass, alignment_index, alignment_status, form_class,
        located_occurrences, retains_uncertain_source,
    },
    source_inventory::{
        inventory_document_with_all_variant_occurrences, inventory_document_with_variants,
        patterns_from_rows,
    },
};
use serde_json::{Value, json};

const MATRIX: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../data/aozora-syntax-coverage.toml"
);
const SIC_SOURCE: &str = "本文一［＃「一」はママ］\n本文二［＃「二」はママ］\n本文三［＃「三」はママ］\n本文四［＃「四」はママ］\n";

fn chuuki_rows() -> Vec<String> {
    vec!["annotation.chuuki".to_owned()]
}

fn matrix_patterns() -> Vec<ab_coverage::source_inventory::SourceInventoryPattern> {
    let matrix = CoverageMatrix::parse(&fs::read_to_string(MATRIX).unwrap()).unwrap();
    patterns_from_rows(matrix.rows())
}

fn node(
    kind: &str,
    span: Option<(usize, usize)>,
    marker_spans: &[(usize, usize)],
) -> AlignmentNode {
    AlignmentNode {
        kind: kind.to_owned(),
        span,
        marker_spans: marker_spans.to_vec(),
        uncertain_source: None,
        literal: false,
    }
}

#[test]
fn full_occurrence_inventory_locates_every_occurrence_the_sampled_inventory_counts() {
    let patterns = matrix_patterns();
    let sampled = inventory_document_with_variants("w1", SIC_SOURCE, &patterns, &chuuki_rows());
    let full = inventory_document_with_all_variant_occurrences(
        "w1",
        SIC_SOURCE,
        &patterns,
        &chuuki_rows(),
    );

    assert_eq!(sampled.row_counts, full.row_counts);
    for (form, variant) in &full.row_variants["annotation.chuuki"] {
        assert_eq!(variant.occurrences, 4, "{form}");
        assert_eq!(variant.samples.len(), 4, "{form}");
        assert_eq!(
            sampled.row_variants["annotation.chuuki"][form]
                .samples
                .len(),
            3
        );
        for sample in &variant.samples {
            let start = sample.byte_start.unwrap();
            assert_eq!(&SIC_SOURCE[start..start + sample.raw.len()], sample.raw);
        }
    }
}

#[test]
fn located_occurrences_join_each_form_with_its_partitions() {
    let patterns = matrix_patterns();
    let full = inventory_document_with_all_variant_occurrences(
        "w1",
        SIC_SOURCE,
        &patterns,
        &chuuki_rows(),
    );
    let located = located_occurrences(&full, &chuuki_rows());

    assert_eq!(located.len(), 4);
    let partitions = &full.row_semantic_partitions["annotation.chuuki"];
    for occurrence in &located {
        assert_eq!(occurrence.form.as_deref(), Some("［＃「○○」はママ］"));
        let expected = partitions
            .iter()
            .filter(|(_, summary)| {
                summary
                    .samples
                    .iter()
                    .any(|sample| sample.byte_start == Some(occurrence.byte_start))
            })
            .map(|(partition, _)| partition.clone())
            .collect::<BTreeSet<_>>();
        assert!(!expected.is_empty());
        assert_eq!(occurrence.partitions, expected);
    }
}

#[test]
fn alignment_index_flattens_nested_kinds_and_raw_sidecars() {
    let aat = json!({"kind": "document", "children": [
        {"kind": "raw", "span": {"byte_start": 0, "byte_end": 4},
         "text_apparatus": {"note": "x"}},
        {"kind": "annotated_text", "span": {"byte_start": 4, "byte_end": 9},
         "interpretation_marker_spans": [{"byte_start": 6, "byte_end": 9}],
         "children": [{"kind": "text", "span": {"byte_start": 4, "byte_end": 6}}]}
    ]});
    let mut kinds = alignment_index(&aat, "")
        .into_iter()
        .map(|node| (node.kind, node.span, node.marker_spans))
        .collect::<Vec<_>>();
    kinds.sort();

    assert_eq!(
        kinds,
        vec![
            ("annotated_text".to_owned(), Some((4, 9)), vec![(6, 9)]),
            ("document".to_owned(), None, vec![]),
            ("raw".to_owned(), Some((0, 4)), vec![]),
            ("text".to_owned(), Some((4, 6)), vec![]),
            ("text_apparatus".to_owned(), Some((0, 4)), vec![]),
        ]
    );
}

#[test]
fn a_text_variant_types_the_markers_its_quoted_target_establishes() {
    let aat = json!({"kind": "text-variant", "span": {"byte_start": 10, "byte_end": 90},
    "current_interpretation_facts": [
        {"kind": "warichu", "source_span": {"coordinate_system": "decoded_utf8", "start": 16, "end": 34}}
    ]});
    let nodes = alignment_index(&aat, "");

    assert_eq!(nodes[0].marker_spans, vec![(16, 34)]);
    assert_eq!(alignment_status(&nodes, 16, 34), AlignmentStatus::Typed);
    assert_eq!(alignment_status(&nodes, 40, 58), AlignmentStatus::Unaligned);
}

#[test]
fn a_forced_line_break_types_the_marker_it_spans() {
    let aat = json!({"kind": "paragraph", "content": [
        {"kind": "text", "value": "前", "span": {"byte_start": 0, "byte_end": 3}},
        {"kind": "text", "value": "\n", "x-break-kind": "line", "x-break-marker": "forced",
         "span": {"byte_start": 3, "byte_end": 18}},
        {"kind": "text", "value": "\n", "x-break-kind": "line",
         "span": {"byte_start": 21, "byte_end": 36}}
    ]});
    let nodes = alignment_index(&aat, "");

    assert_eq!(alignment_status(&nodes, 3, 18), AlignmentStatus::Typed);
    assert_eq!(alignment_status(&nodes, 0, 3), AlignmentStatus::Unaligned);
    assert_eq!(alignment_status(&nodes, 21, 36), AlignmentStatus::Unaligned);
}

#[test]
fn an_annotated_text_types_references_inside_the_note_that_follows_it() {
    let aat = json!({"kind": "paragraph", "content": [
        {"kind": "annotated_text", "note_kind": "gloss", "span": {"byte_start": 6, "byte_end": 51},
         "content": [{"kind": "iteration-mark", "span": {"byte_start": 0, "byte_end": 6}}],
         "annotation_content": [{"kind": "text", "span": {"byte_start": 30, "byte_end": 36}}]},
        {"kind": "annotated_text", "note_kind": "base-edition", "span": {"byte_start": 60, "byte_end": 90},
         "content": [{"kind": "gaiji", "span": {"byte_start": 60, "byte_end": 90}}]}
    ]});
    let nodes = alignment_index(&aat, "");

    assert_eq!(alignment_status(&nodes, 12, 18), AlignmentStatus::Typed);
    assert_eq!(alignment_status(&nodes, 70, 76), AlignmentStatus::Unaligned);
}

#[test]
fn an_editorial_note_types_references_inside_the_note_that_follows_its_target() {
    let aat = json!({"kind": "paragraph", "content": [
        {"kind": "editorial_note", "note_kind": "uncertain-correction", "span": {"byte_start": 24, "byte_end": 90},
         "target_source_spans": [{"byte_start": 9, "byte_end": 24}]},
        {"kind": "editorial_note", "note_kind": "sic", "span": {"byte_start": 100, "byte_end": 130},
         "target_source_spans": [{"byte_start": 100, "byte_end": 130}]}
    ]});
    let nodes = alignment_index(&aat, "");

    assert_eq!(alignment_status(&nodes, 33, 42), AlignmentStatus::Typed);
    assert_eq!(
        alignment_status(&nodes, 110, 116),
        AlignmentStatus::Unaligned
    );
}

#[test]
fn a_forward_annotation_types_the_markers_its_quoted_target_contains() {
    let aat = json!({"kind": "paragraph", "content": [
        {"kind": "annotated_text", "note_kind": "gloss", "span": {"byte_start": 54, "byte_end": 167},
         "content": [{"kind": "gaiji", "span": {"byte_start": 0, "byte_end": 54}}]},
        {"kind": "annotated_text", "note_kind": "gloss", "span": {"byte_start": 170, "byte_end": 260},
         "interpretation_marker_spans": [{"byte_start": 170, "byte_end": 185},
                                         {"byte_start": 230, "byte_end": 260}]}
    ]});
    let nodes = alignment_index(&aat, "");

    assert_eq!(alignment_status(&nodes, 63, 117), AlignmentStatus::Typed);
    assert_eq!(
        alignment_status(&nodes, 195, 215),
        AlignmentStatus::Unaligned
    );
}

#[test]
fn alignment_status_ranks_typed_evidence_over_raw_preservation() {
    let cases = [
        (
            "marker span on a typed node",
            vec![node("annotated_text", Some((0, 20)), &[(10, 20)])],
            AlignmentStatus::Typed,
        ),
        (
            "exact typed node span",
            vec![node("note", Some((10, 20)), &[])],
            AlignmentStatus::Typed,
        ),
        (
            "formatting instruction overlap",
            vec![node("formatting_instruction", Some((12, 30)), &[])],
            AlignmentStatus::Typed,
        ),
        (
            "raw sidecar with the exact span",
            vec![
                node("raw", Some((10, 20)), &[]),
                node("text_apparatus", Some((10, 20)), &[]),
            ],
            AlignmentStatus::Typed,
        ),
        (
            "typed wins over overlapping raw",
            vec![
                node("raw", Some((0, 30)), &[]),
                node("note", Some((10, 20)), &[]),
            ],
            AlignmentStatus::Typed,
        ),
        (
            "raw span overlap",
            vec![node("raw", Some((5, 15)), &[])],
            AlignmentStatus::RawPreserved,
        ),
        (
            "exact raw node inside a wider typed marker",
            vec![
                node("ruby", Some((0, 40)), &[(0, 40)]),
                node("raw", Some((10, 20)), &[]),
            ],
            AlignmentStatus::RawPreserved,
        ),
        (
            "exact typed marker wins over an exact raw node",
            vec![
                node("raw", Some((10, 20)), &[]),
                node("ruby", Some((0, 40)), &[(10, 20)]),
            ],
            AlignmentStatus::Typed,
        ),
        (
            "typed node with a different span",
            vec![node("note", Some((10, 21)), &[])],
            AlignmentStatus::Unaligned,
        ),
        (
            "text and paragraph never type an occurrence",
            vec![
                node("text", Some((10, 20)), &[]),
                node("paragraph", Some((10, 20)), &[]),
            ],
            AlignmentStatus::Unaligned,
        ),
        (
            "adjacent raw span",
            vec![node("raw", Some((20, 30)), &[])],
            AlignmentStatus::Unaligned,
        ),
    ];
    for (name, nodes, expected) in cases {
        assert_eq!(alignment_status(&nodes, 10, 20), expected, "{name}");
    }
}

#[test]
fn a_raw_node_retains_an_occurrence_only_with_a_problem_and_the_exact_source() {
    // The marker occupies bytes 6..15 of the raw node's source "前文［＃」後".
    let raw = |problem: bool, source: &str, span: (usize, usize)| {
        let mut node = json!({"kind": "raw", "source": source,
            "span": {"byte_start": span.0, "byte_end": span.1}});
        if problem {
            node["interpretation_problem"] = json!({"code": "unknown-notation"});
        }
        node
    };
    let marker = "［＃」";
    let retained = |children: Vec<Value>| {
        let nodes = alignment_index(&json!({"kind": "document", "children": children}), "");
        retains_uncertain_source(&nodes, 16, 25, marker)
    };
    let source = "前文［＃」後";
    assert!(retained(vec![raw(true, source, (10, 28))]));
    for (name, children) in [
        (
            "no interpretation problem",
            vec![raw(false, source, (10, 28))],
        ),
        (
            "different source text",
            vec![raw(true, "前文［＃「後", (10, 28))],
        ),
        (
            "source shorter than its span",
            vec![raw(true, source, (10, 30))],
        ),
        (
            "raw node beside the marker",
            vec![raw(true, source, (25, 43))],
        ),
        (
            "a typed node interprets the marker",
            vec![
                raw(true, source, (10, 28)),
                json!({"kind": "note", "span": {"byte_start": 16, "byte_end": 25}}),
            ],
        ),
    ] {
        assert!(!retained(children), "{name}");
    }
}

#[test]
fn literal_text_between_raw_nodes_with_problems_retains_an_occurrence() {
    let source = "前｜※［＃」然《ぜん》後\r\n";
    let occurrence = "｜※［＃」然《ぜん》";
    let start = source.find(occurrence).unwrap();
    let end = start + occurrence.len();
    let extent = |part: &str| {
        let at = source.find(part).unwrap();
        (at, at + part.len())
    };
    let (prefix, base, reading) = (extent("｜※［＃」"), extent("然"), extent("《ぜん》"));
    let span = |(from, to): (usize, usize)| json!({"byte_start": from, "byte_end": to});
    let raw = |extent: (usize, usize)| {
        json!({"kind": "raw", "source": &source[extent.0..extent.1], "span": span(extent),
            "interpretation_problem": {"code": "unparsed-source-gap"}})
    };
    let text = |extent: (usize, usize), value: &str| json!({"kind": "text", "value": value, "span": span(extent)});
    let retained = |children: Vec<Value>| {
        let aat =
            json!({"kind": "paragraph", "span": span((0, source.len())), "content": children});
        retains_uncertain_source(&alignment_index(&aat, source), start, end, occurrence)
    };
    assert!(retained(vec![
        text(extent("前"), "前"),
        raw(prefix),
        text(base, "然"),
        raw(reading),
        text(extent("後\r\n"), "後\n"),
    ]));
    for (name, children) in [
        (
            "a typed node interprets part of the occurrence",
            vec![
                raw(prefix),
                json!({"kind": "ruby", "span": span((base.0, reading.1)),
                    "content": [text(base, "然")]}),
                raw(reading),
            ],
        ),
        (
            "a byte is neither in text nor in a raw node",
            vec![raw(prefix), raw(reading)],
        ),
        (
            "a text value differs from its source",
            vec![raw(prefix), text(base, "燃"), raw(reading)],
        ),
    ] {
        assert!(!retained(children), "{name}");
    }
}

#[test]
fn a_raw_node_with_a_problem_outranks_a_typed_marker_it_adjoins() {
    let problem = |span: (usize, usize)| AlignmentNode {
        uncertain_source: Some("x".repeat(span.1 - span.0)),
        ..node("raw", Some(span), &[])
    };
    let cases = [
        (
            "problem raw node inside the occurrence",
            vec![
                node("layout_block", Some((0, 40)), &[(10, 14), (18, 20)]),
                problem((14, 18)),
            ],
            AlignmentStatus::RawPreserved,
        ),
        (
            "problem raw node apart from the typed markers",
            vec![
                node("accent", Some((12, 14)), &[(10, 11), (19, 20)]),
                problem((14, 18)),
            ],
            AlignmentStatus::Typed,
        ),
        (
            "raw node without a problem",
            vec![
                node("layout_block", Some((0, 40)), &[(10, 14), (18, 20)]),
                node("raw", Some((14, 18)), &[]),
            ],
            AlignmentStatus::Typed,
        ),
        (
            "exact typed marker",
            vec![
                node("layout_block", Some((0, 40)), &[(10, 20)]),
                problem((14, 18)),
            ],
            AlignmentStatus::Typed,
        ),
        (
            "formatting instruction overlap",
            vec![
                node("formatting_instruction", Some((12, 30)), &[]),
                problem((14, 18)),
            ],
            AlignmentStatus::Typed,
        ),
    ];
    for (name, nodes, expected) in cases {
        assert_eq!(alignment_status(&nodes, 10, 20), expected, "{name}");
    }
}

#[test]
fn marker_spans_around_a_raw_clause_retain_a_compound_opener() {
    let clause = "ただし冒頭の歌記号のみは１字下げ";
    let opener = format!("［＃ここから２字下げ、{clause}］");
    let source = format!("{opener}\n本文\n");
    let clause_start = opener.find(clause).unwrap();
    let clause_end = clause_start + clause.len();
    let span = |(from, to): (usize, usize)| json!({"byte_start": from, "byte_end": to});
    let retained = |markers: &[(usize, usize)], raw_source: &str| {
        let block = json!({"kind": "layout_block", "span": span((0, source.len())),
            "interpretation_marker_spans": markers.iter().copied().map(span).collect::<Vec<_>>(),
            "children": [{"kind": "paragraph", "content": [{"kind": "raw", "source": raw_source,
                "span": span((clause_start, clause_end)),
                "interpretation_problem": {"code": "uninterpreted-notation"}}]}]});
        retains_uncertain_source(&alignment_index(&block, &source), 0, opener.len(), &opener)
    };
    let around = [(0, clause_start), (clause_end, opener.len())];
    assert!(retained(&around, clause));
    for (name, markers, raw_source) in [
        (
            "a byte outside every marker and raw node",
            &around[..1],
            clause,
        ),
        (
            "a marker overlapping a raw node whose source differs",
            &[(0, opener.len())][..],
            &"x".repeat(clause.len())[..],
        ),
    ] {
        assert!(!retained(markers, raw_source), "{name}");
    }
}

#[test]
fn form_class_ranks_raw_then_alignment_problems_then_coverage() {
    let statuses =
        |pairs: &[(AlignmentStatus, u64)]| pairs.iter().copied().collect::<BTreeMap<_, _>>();
    let set = |items: &[&str]| {
        items
            .iter()
            .map(|item| (*item).to_owned())
            .collect::<BTreeSet<_>>()
    };
    let covered = set(&["relation:sic", "form:sic-note"]);
    let cases = [
        (
            "any raw occurrence",
            statuses(&[
                (AlignmentStatus::Typed, 9),
                (AlignmentStatus::RawPreserved, 1),
            ]),
            set(&["relation:sic"]),
            true,
            FormClass::ParserSemanticGap,
        ),
        (
            "unlocated occurrence",
            statuses(&[(AlignmentStatus::Typed, 9), (AlignmentStatus::Unlocated, 1)]),
            set(&["relation:sic"]),
            true,
            FormClass::ParserAlignmentReview,
        ),
        (
            "fixture-covered typed form",
            statuses(&[(AlignmentStatus::Typed, 3)]),
            set(&["relation:other"]),
            true,
            FormClass::FixtureCovered,
        ),
        (
            "typed form in covered partitions",
            statuses(&[(AlignmentStatus::Typed, 3)]),
            set(&["relation:sic", "form:sic-note"]),
            false,
            FormClass::AliasOfCovered,
        ),
        (
            "typed form with an uncovered partition",
            statuses(&[(AlignmentStatus::Typed, 3)]),
            set(&["relation:sic", "relation:other"]),
            false,
            FormClass::TypedPartitionUncovered,
        ),
        (
            "typed form without partitions",
            statuses(&[(AlignmentStatus::Typed, 3)]),
            set(&[]),
            false,
            FormClass::TypedPartitionUncovered,
        ),
    ];
    for (name, statuses, partitions, fixture, expected) in cases {
        assert_eq!(
            form_class(&statuses, &partitions, fixture, &covered),
            expected,
            "{name}"
        );
    }
}

fn write_stub_parser(dir: &Path, aat: &Value) -> std::path::PathBuf {
    let aat_path = dir.join("aat.json");
    fs::write(&aat_path, serde_json::to_vec(aat).unwrap()).unwrap();
    let calls = dir.join("calls");
    let parser = dir.join("parser");
    fs::write(
        &parser,
        format!(
            "#!/bin/sh\ncat >/dev/null\necho run >> '{}'\ncat '{}'\n",
            calls.display(),
            aat_path.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&parser, fs::Permissions::from_mode(0o755)).unwrap();
    }
    parser
}

#[test]
fn binary_aligns_every_corpus_occurrence_and_reuses_cached_parses() {
    let dir = std::env::temp_dir().join(format!("ab-occurrence-alignment-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("corpus")).unwrap();
    fs::write(dir.join("corpus/w1.txt"), SIC_SOURCE).unwrap();
    fs::write(
        dir.join("index.json"),
        serde_json::to_vec(&json!({"works": [{"id": "w1", "txt_path": "w1.txt"}]})).unwrap(),
    )
    .unwrap();
    let patterns = matrix_patterns();
    let full = inventory_document_with_all_variant_occurrences(
        "w1",
        SIC_SOURCE,
        &patterns,
        &chuuki_rows(),
    );
    let mut starts = located_occurrences(&full, &chuuki_rows())
        .into_iter()
        .map(|occurrence| {
            (
                occurrence.byte_start,
                occurrence.byte_start + occurrence.occurrence.raw.len(),
            )
        })
        .collect::<Vec<_>>();
    starts.sort_unstable();
    // The stub types the first three notes and leaves the last one raw.
    let children = starts
        .iter()
        .enumerate()
        .map(|(position, (start, end))| {
            json!({"kind": if position < 3 { "note" } else { "raw" },
                   "span": {"byte_start": start, "byte_end": end}})
        })
        .collect::<Vec<_>>();
    let parser = write_stub_parser(&dir, &json!({"kind": "document", "children": children}));
    let partition_occurrences = full.row_semantic_partitions["annotation.chuuki"]
        .iter()
        .map(|(partition, summary)| {
            (
                partition.clone(),
                json!({"occurrences": summary.occurrences}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    fs::write(
        dir.join("inventory.json"),
        serde_json::to_vec(&json!({"rows": {"annotation.chuuki": {
            "variants": {"［＃「○○」はママ］": {"occurrences": 4}},
            "semantic_partitions": partition_occurrences,
        }}}))
        .unwrap(),
    )
    .unwrap();

    let run = |output: &str| {
        let status = Command::new(env!("CARGO_BIN_EXE_ab-occurrence-alignment"))
            .args(["--index", dir.join("index.json").to_str().unwrap()])
            .args(["--corpus", dir.join("corpus").to_str().unwrap()])
            .args(["--matrix", MATRIX, "--row", "annotation.chuuki"])
            .args(["--parser", parser.to_str().unwrap()])
            .args(["--cache-dir", dir.join("cache").to_str().unwrap()])
            .args([
                "--inventory-json",
                dir.join("inventory.json").to_str().unwrap(),
            ])
            .args(["--output-json", dir.join(output).to_str().unwrap()])
            .status()
            .unwrap();
        assert!(status.success());
        serde_json::from_slice::<Value>(&fs::read(dir.join(output)).unwrap()).unwrap()
    };
    let first = run("first.json");
    let second = run("second.json");

    let row = &first["rows"]["annotation.chuuki"];
    assert_eq!(row["occurrences"], 4);
    assert_eq!(row["statuses"], json!({"raw-preserved": 1, "typed": 3}));
    assert_eq!(
        row["forms"]["［＃「○○」はママ］"]["samples"][0]["status"],
        "raw-preserved"
    );
    assert_eq!(first["inventory_mismatches"], json!([]));
    assert_eq!(
        first["residue_ledger"]["annotation.chuuki"]["occurrences"],
        row["occurrences"]
    );
    assert_eq!(first["editions"]["parses"], 1);
    assert_eq!(second["editions"]["parses"], 0);
    assert_eq!(second["editions"]["parse_cache_hits"], 1);
    assert_eq!(second["rows"], first["rows"]);
    assert_eq!(
        fs::read_to_string(dir.join("calls"))
            .unwrap()
            .lines()
            .count(),
        1
    );

    let inventory = fs::read_to_string(dir.join("inventory.json")).unwrap();
    fs::write(
        dir.join("inventory.json"),
        inventory.replace(
            r#""［＃「○○」はママ］":{"occurrences":4}"#,
            r#""［＃「○○」はママ］":{"occurrences":5}"#,
        ),
    )
    .unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_ab-occurrence-alignment"))
        .args(["--index", dir.join("index.json").to_str().unwrap()])
        .args(["--corpus", dir.join("corpus").to_str().unwrap()])
        .args(["--matrix", MATRIX, "--row", "annotation.chuuki"])
        .args(["--parser", parser.to_str().unwrap()])
        .args(["--cache-dir", dir.join("cache").to_str().unwrap()])
        .args([
            "--inventory-json",
            dir.join("inventory.json").to_str().unwrap(),
        ])
        .args(["--output-json", dir.join("mismatch.json").to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        !status.success(),
        "a ledger total the inventory disagrees with fails the run"
    );
    let mismatch: Value =
        serde_json::from_slice(&fs::read(dir.join("mismatch.json")).unwrap()).unwrap();
    assert_eq!(
        mismatch["inventory_mismatches"],
        json!([{"row": "annotation.chuuki", "section": "variants",
                "key": "［＃「○○」はママ］", "inventory": 5, "ledger": 4}])
    );
    fs::remove_dir_all(&dir).unwrap();
}

const REQUIREMENTS: &str = r#"
[[family]]
syntax_row_id = "annotation.chuuki"

[[family.obligation]]
id = "annotation.chuuki.note-content"

[family.obligation.corpus_check]
partitions = ["form:attached-note"]
operands = '^［＃「(?P<target>[^「」]+)」に「(?P<note>[^「」]+)」の注記］$'
kind = "annotated_text"
text = { annotation_content = "{note}" }
"#;

#[test]
fn binary_reports_corpus_checks_per_obligation_and_reruns_them_when_requirements_change() {
    let dir = std::env::temp_dir().join(format!("ab-corpus-check-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("corpus")).unwrap();
    let source = "甲［＃「甲」に「注一」の注記］乙［＃「乙」に「注二」の注記］\n";
    fs::write(dir.join("corpus/w1.txt"), source).unwrap();
    fs::write(
        dir.join("index.json"),
        serde_json::to_vec(&json!({"works": [{"id": "w1", "txt_path": "w1.txt"}]})).unwrap(),
    )
    .unwrap();
    // The stub keeps the first note's value and drops the second one's.
    let annotated = |marker: &str, note: &str| {
        let start = source.find(marker).unwrap();
        json!({"kind": "annotated_text",
               "span": {"byte_start": start, "byte_end": start + marker.len()},
               "annotation_content": [{"kind": "text", "value": note}]})
    };
    let parser = write_stub_parser(
        &dir,
        &json!({"blocks": [
            annotated("［＃「甲」に「注一」の注記］", "注一"),
            annotated("［＃「乙」に「注二」の注記］", ""),
        ]}),
    );
    let run = |output: &str, requirements: &str| {
        fs::write(dir.join("requirements.toml"), requirements).unwrap();
        let status = Command::new(env!("CARGO_BIN_EXE_ab-occurrence-alignment"))
            .args(["--index", dir.join("index.json").to_str().unwrap()])
            .args(["--corpus", dir.join("corpus").to_str().unwrap()])
            .args(["--matrix", MATRIX, "--row", "annotation.chuuki"])
            .args(["--parser", parser.to_str().unwrap()])
            .args(["--cache-dir", dir.join("cache").to_str().unwrap()])
            .args([
                "--requirements",
                dir.join("requirements.toml").to_str().unwrap(),
            ])
            .args(["--output-json", dir.join(output).to_str().unwrap()])
            .status()
            .unwrap();
        assert!(status.success());
        serde_json::from_slice::<Value>(&fs::read(dir.join(output)).unwrap()).unwrap()
    };
    let calls = || {
        fs::read_to_string(dir.join("calls"))
            .unwrap()
            .lines()
            .count()
    };

    let first = run("first.json", REQUIREMENTS);
    let report = &first["obligation_checks"]["annotation.chuuki.note-content"];
    assert_eq!(report["occurrences"], 2);
    assert_eq!(report["satisfied"], 1);
    assert_eq!(report["failed"], 1);
    assert_eq!(report["credited"], false);
    let sample = &report["samples"]["text-mismatch:annotation_content"][0];
    assert_eq!(sample["raw"], "［＃「乙」に「注二」の注記］");
    assert_eq!(sample["expected"], "注二");
    assert_eq!(
        first["partition_credit"],
        json!({}),
        "an obligation with a failing occurrence credits no partition"
    );
    assert_eq!(
        first["residue_ledger"]["annotation.chuuki"]["classes"],
        json!({"predicate-covered": {"forms": 1, "occurrences": 1},
               "check-failed": {"forms": 1, "occurrences": 1}}),
        "each occurrence is classified by its own check outcome"
    );
    assert_eq!(calls(), 1);

    let second = run("second.json", REQUIREMENTS);
    assert_eq!(second["obligation_checks"], first["obligation_checks"]);
    assert_eq!(calls(), 1);

    // Restricting the operands to the first note re-evaluates from a new parse.
    let narrowed = REQUIREMENTS.replace("[^「」]+)」の注記", "注一)」の注記");
    let third = run("third.json", &narrowed);
    let report = &third["obligation_checks"]["annotation.chuuki.note-content"];
    assert_eq!(report["satisfied"], 1);
    assert_eq!(report["unchecked"], 1);
    assert_eq!(report["credited"], true);
    assert_eq!(
        third["partition_credit"]["annotation.chuuki"]["form:attached-note"]["predicates"],
        json!({"annotation.chuuki.note-content": 1})
    );
    assert_eq!(calls(), 2);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn binary_compares_check_outcomes_with_a_baseline_parser_and_fails_unflipped_expectations() {
    let dir = std::env::temp_dir().join(format!("ab-baseline-check-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("corpus")).unwrap();
    fs::create_dir_all(dir.join("baseline")).unwrap();
    let source = "甲［＃「甲」に「注一」の注記］乙［＃「乙」に「注二」の注記］\n";
    fs::write(dir.join("corpus/w1.txt"), source).unwrap();
    fs::write(
        dir.join("index.json"),
        serde_json::to_vec(&json!({"works": [{"id": "w1", "txt_path": "w1.txt"}]})).unwrap(),
    )
    .unwrap();
    fs::write(dir.join("requirements.toml"), REQUIREMENTS).unwrap();
    let annotated = |marker: &str, note: &str| {
        let start = source.find(marker).unwrap();
        json!({"kind": "annotated_text",
               "span": {"byte_start": start, "byte_end": start + marker.len()},
               "annotation_content": [{"kind": "text", "value": note}]})
    };
    // The baseline keeps both notes; the candidate drops the second one.
    let baseline = write_stub_parser(
        &dir.join("baseline"),
        &json!({"blocks": [
            annotated("［＃「甲」に「注一」の注記］", "注一"),
            annotated("［＃「乙」に「注二」の注記］", "注二"),
        ]}),
    );
    let candidate = write_stub_parser(
        &dir,
        &json!({"blocks": [
            annotated("［＃「甲」に「注一」の注記］", "注一"),
            annotated("［＃「乙」に「注二」の注記］", ""),
        ]}),
    );
    let run = |baseline: &Path, expect_flip: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ab-occurrence-alignment"));
        command
            .args(["--index", dir.join("index.json").to_str().unwrap()])
            .args(["--corpus", dir.join("corpus").to_str().unwrap()])
            .args(["--matrix", MATRIX, "--row", "annotation.chuuki"])
            .args(["--parser", candidate.to_str().unwrap()])
            .args(["--baseline-parser", baseline.to_str().unwrap()])
            .args(["--cache-dir", dir.join("cache").to_str().unwrap()])
            .args([
                "--requirements",
                dir.join("requirements.toml").to_str().unwrap(),
            ])
            .args(["--output-json", dir.join("out.json").to_str().unwrap()]);
        for obligation in expect_flip {
            command.args(["--expect-flip", obligation]);
        }
        let status = command.status().unwrap();
        let report =
            serde_json::from_slice::<Value>(&fs::read(dir.join("out.json")).unwrap()).unwrap();
        (status.success(), report)
    };
    let obligation = "annotation.chuuki.note-content";

    let (passed, report) = run(&baseline, &[obligation]);
    assert!(passed, "the expected obligation flipped");
    let discrimination = &report["obligation_checks"][obligation]["discrimination"];
    assert_eq!(
        [
            &discrimination["both_satisfied"],
            &discrimination["both_failed"],
            &discrimination["gained"],
            &discrimination["lost"],
        ],
        [&json!(1), &json!(0), &json!(0), &json!(1)]
    );
    let lost = &discrimination["samples"]["lost"][0];
    assert_eq!(lost["raw"], "［＃「乙」に「注二」の注記］");
    assert_eq!(lost["baseline"]["outcome"], "satisfied");
    assert_eq!(lost["candidate"]["outcome"], "text-mismatch");

    let (passed, report) = run(&candidate, &[obligation]);
    assert!(!passed, "a baseline equal to the candidate flips nothing");
    assert_eq!(
        report["obligation_checks"][obligation]["discrimination"]["both_failed"],
        1
    );
    fs::remove_dir_all(&dir).unwrap();
}
