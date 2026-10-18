use std::{collections::BTreeSet, fs, path::Path, process::Command};

use ab_coverage::{
    CoverageMatrix,
    edition_diff::{Side, Targets, attribute, changed_nodes, content_sha256, region_occurrences},
    occurrence_alignment::LocatedOccurrence,
    source_inventory::{
        SourceMarkerOccurrence, inventory_document_with_all_variant_occurrences, patterns_from_rows,
    },
};
use serde_json::{Value, json};

const MATRIX: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../data/aozora-syntax-coverage.toml"
);

fn span(start: usize, end: usize) -> Value {
    json!({"byte_start": start, "byte_end": end})
}

#[test]
fn changed_nodes_report_the_changed_node_not_its_ancestors_or_build_identity() {
    let document = |version: &str, marker: Value| {
        json!({
            "meta": {"adapter_version": version, "interpretation_facts": []},
            "blocks": [{"kind": "paragraph", "span": span(0, 20), "content": [
                {"kind": "text", "span": span(0, 10), "value": "本文"},
                marker,
            ]}],
        })
    };
    let raw = json!({"kind": "raw", "span": span(10, 20), "source": "［＃注］"});
    let typed = json!({"kind": "editorial_note", "span": span(10, 20), "text": "注"});

    assert_eq!(
        content_sha256(&document("1", raw.clone())),
        content_sha256(&document("2", raw.clone()))
    );
    assert_eq!(
        changed_nodes(&document("1", raw.clone()), &document("2", raw.clone())),
        []
    );
    let changed = changed_nodes(&document("1", raw), &document("1", typed));
    let summary = changed
        .iter()
        .map(|node| (node.side, node.kind.as_str(), node.spans.clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        summary,
        [
            (Side::Baseline, "raw", vec![(10, 20)]),
            (Side::Candidate, "editorial_note", vec![(10, 20)]),
        ]
    );
}

#[test]
fn a_nested_node_without_spans_takes_its_nearest_spanned_ancestors() {
    let gaiji = |code: &str| {
        json!([{"kind": "gaiji", "span": span(10, 20),
                "code": {"kind": "jis-code", "value": code}}])
    };
    let changed = changed_nodes(&gaiji("1-1-1"), &gaiji("1-1-2"));
    assert_eq!(
        changed
            .iter()
            .map(|node| (node.side, node.kind.as_str(), node.spans.clone()))
            .collect::<Vec<_>>(),
        [
            (Side::Baseline, "jis-code", vec![(10, 20)]),
            (Side::Candidate, "jis-code", vec![(10, 20)]),
        ]
    );
}

fn occurrence(row_id: &str, form: &str, byte_start: usize, raw: &str) -> LocatedOccurrence {
    LocatedOccurrence {
        row_id: row_id.to_owned(),
        form: Some(form.to_owned()),
        partitions: BTreeSet::new(),
        occurrence: SourceMarkerOccurrence {
            work_id: "w".to_owned(),
            indexed_path: None,
            line: 1,
            kind: "Annotation".to_owned(),
            raw: raw.to_owned(),
            body: String::new(),
            byte_start: Some(byte_start),
        },
        byte_start,
    }
}

#[test]
fn attribution_explains_a_change_only_through_a_target_occurrence_it_touches() {
    // Two 10-byte markers: a target at 10..20 and a non-target at 40..50.
    let occurrences = [
        occurrence("target.row", "target-form", 10, "0123456789"),
        occurrence("other.row", "other-form", 40, "0123456789"),
    ];
    let targets = Targets {
        rows: BTreeSet::from(["target.row".to_owned()]),
        forms: BTreeSet::new(),
    };
    for (name, node_span, explained, forms) in [
        ("overlaps the target", (12, 15), true, vec!["target.row"]),
        ("abuts the target", (0, 10), true, vec!["target.row"]),
        ("touches no occurrence", (25, 35), false, vec![]),
        (
            "touches only a non-target",
            (45, 60),
            false,
            vec!["other.row"],
        ),
        (
            "touches both",
            (15, 45),
            true,
            vec!["other.row", "target.row"],
        ),
    ] {
        let baseline =
            json!({"kind": "text", "span": span(node_span.0, node_span.1), "value": "a"});
        let candidate =
            json!({"kind": "text", "span": span(node_span.0, node_span.1), "value": "b"});
        let changed = changed_nodes(&json!([baseline]), &json!([candidate]));
        let attribution = attribute(changed, &occurrences, &targets);
        assert_eq!(attribution.unexplained.is_empty(), explained, "{name}");
        assert_eq!(
            attribution
                .forms
                .keys()
                .map(|(row, _)| row.as_str())
                .collect::<Vec<_>>(),
            forms,
            "{name}"
        );
    }

    let by_form = Targets {
        rows: BTreeSet::new(),
        forms: BTreeSet::from([("target.row".to_owned(), "target-form".to_owned())]),
    };
    let changed = changed_nodes(
        &json!([{"kind": "raw", "span": span(10, 20)}]),
        &json!([{"kind": "note", "span": span(10, 20)}]),
    );
    assert!(
        attribute(changed, &occurrences, &by_form)
            .unexplained
            .is_empty()
    );
}

/// A typed marker at 30..40 takes 20..30 as its target: the baseline text
/// node 0..30 becomes 0..20 and a container whose own span is the marker and
/// whose content is 20..30. The container touches the marker; the old text
/// node overlaps the container's extent; the remainder overlaps the old text
/// node. A changed node away from all of them stays unexplained.
#[test]
fn attribution_explains_nodes_that_overlap_an_explained_node() {
    let occurrences = [occurrence("target.row", "form", 30, "0123456789")];
    let targets = Targets {
        rows: BTreeSet::from(["target.row".to_owned()]),
        forms: BTreeSet::new(),
    };
    let baseline = json!([
        {"kind": "text", "span": span(0, 30), "value": "abc"},
        {"kind": "raw", "span": span(30, 40)},
        {"kind": "text", "span": span(45, 50), "value": "d"},
    ]);
    let candidate = json!([
        {"kind": "text", "span": span(0, 20), "value": "ab"},
        {"kind": "note", "span": span(30, 40), "content": [
            {"kind": "text", "span": span(20, 30), "value": "c"},
        ]},
        {"kind": "text", "span": span(45, 50), "value": "e"},
    ]);
    let attribution = attribute(changed_nodes(&baseline, &candidate), &occurrences, &targets);
    assert_eq!(
        attribution
            .unexplained
            .iter()
            .map(|node| node.spans.clone())
            .collect::<Vec<_>>(),
        vec![vec![(45, 50)], vec![(45, 50)]],
        "{attribution:?}"
    );
}

#[test]
fn back_matter_markers_are_located_under_the_rows_they_match() {
    let source = "本文\n\n底本：「全集※［＃ローマ数字1、1-13-21］」角川書店\n";
    let matrix = CoverageMatrix::parse(&fs::read_to_string(MATRIX).unwrap()).unwrap();
    let patterns = patterns_from_rows(matrix.rows());
    let rows = matrix
        .rows()
        .iter()
        .map(|row| row.id.clone())
        .collect::<Vec<_>>();
    let summary = inventory_document_with_all_variant_occurrences("w", source, &patterns, &rows);
    let marker = "※［＃ローマ数字1、1-13-21］";
    let located = region_occurrences(&summary, &patterns);
    let marker_start = source.find(marker).unwrap();
    assert!(
        located
            .iter()
            .all(|occurrence| occurrence.byte_start == marker_start
                && occurrence.occurrence.raw == marker),
        "{located:?}"
    );
    let rows = located
        .iter()
        .map(|occurrence| occurrence.row_id.as_str())
        .collect::<BTreeSet<_>>();
    assert!(rows.contains("gaiji.jis_code"), "{rows:?}");
}

const CHANGED_SOURCE: &str = "本文一［＃「一」はママ］\n";
const STABLE_SOURCE: &str = "本文二\n";

/// A parser stub printing `changed` for the source holding `一` and `stable`
/// otherwise, and logging each run to `calls`.
fn write_stub_parser(
    dir: &Path,
    name: &str,
    changed: &Value,
    stable: &Value,
) -> std::path::PathBuf {
    let changed_path = dir.join(format!("{name}-changed.json"));
    let stable_path = dir.join(format!("{name}-stable.json"));
    fs::write(&changed_path, serde_json::to_vec(changed).unwrap()).unwrap();
    fs::write(&stable_path, serde_json::to_vec(stable).unwrap()).unwrap();
    let parser = dir.join(name);
    fs::write(
        &parser,
        format!(
            "#!/bin/sh\ninput=$(cat)\necho {name} >> '{calls}'\ncase \"$input\" in *一*) cat '{changed}';; *) cat '{stable}';; esac\n",
            calls = dir.join("calls").display(),
            changed = changed_path.display(),
            stable = stable_path.display(),
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
fn binary_fails_changes_outside_the_targets_and_reuses_cached_hashes() {
    let dir = std::env::temp_dir().join(format!("ab-edition-diff-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("corpus")).unwrap();
    fs::write(dir.join("corpus/w1.txt"), CHANGED_SOURCE).unwrap();
    fs::write(dir.join("corpus/w2.txt"), STABLE_SOURCE).unwrap();
    fs::write(
        dir.join("index.json"),
        serde_json::to_vec(&json!({"works": [
            {"id": "w1", "txt_path": "w1.txt"},
            {"id": "w2", "txt_path": "w2.txt"},
        ]}))
        .unwrap(),
    )
    .unwrap();
    let marker_start = "本文一".len();
    let marker_end = CHANGED_SOURCE.len() - 1;
    let document = |kind: &str, version: &str| {
        json!({"meta": {"adapter_version": version}, "blocks": [
            {"kind": kind, "span": span(marker_start, marker_end)}
        ]})
    };
    let stable = json!({"blocks": [{"kind": "text", "span": span(0, 9), "value": "本文二"}]});
    let baseline = write_stub_parser(&dir, "baseline", &document("raw", "1"), &stable);
    let candidate = write_stub_parser(&dir, "candidate", &document("note", "2"), &stable);

    let run = |output: &str, targets: &[&str]| {
        let status = Command::new(env!("CARGO_BIN_EXE_ab-edition-diff"))
            .args(["--index", dir.join("index.json").to_str().unwrap()])
            .args(["--corpus", dir.join("corpus").to_str().unwrap()])
            .args(["--matrix", MATRIX])
            .args(["--baseline-parser", baseline.to_str().unwrap()])
            .args(["--candidate-parser", candidate.to_str().unwrap()])
            .args(["--cache-dir", dir.join("cache").to_str().unwrap()])
            .args(targets)
            .args(["--output-json", dir.join(output).to_str().unwrap()])
            .status()
            .unwrap();
        let report = serde_json::from_slice::<Value>(&fs::read(dir.join(output)).unwrap()).unwrap();
        (status.success(), report)
    };
    let (passed, first) = run("first.json", &["--target-row", "annotation.chuuki"]);
    assert!(passed, "{first}");
    assert_eq!(first["editions"]["unchanged"], 1);
    assert_eq!(first["editions"]["changed"], 1);
    assert_eq!(
        first["editions"]["parses"],
        json!({"baseline": 2, "candidate": 2})
    );
    let edition = &first["changed"][0];
    assert_eq!(edition["work_id"], "w1");
    assert_eq!(edition["unexplained_nodes"], 0);
    assert_eq!(
        edition["changed_nodes"],
        json!({"baseline": 1, "candidate": 1})
    );
    assert_eq!(edition["forms"][0]["row"], "annotation.chuuki");
    assert_eq!(edition["forms"][0]["target"], true);

    // The second run reads both hash tables and parses only the changed edition again.
    let (passed, second) = run("second.json", &[]);
    assert!(!passed);
    assert_eq!(second["passed"], false);
    assert_eq!(
        second["editions"]["parses"],
        json!({"baseline": 0, "candidate": 0})
    );
    assert_eq!(second["editions"]["unexplained"], 1);
    assert_eq!(second["changed"][0]["forms"][0]["target"], false);
    assert_eq!(
        fs::read_to_string(dir.join("calls"))
            .unwrap()
            .lines()
            .count(),
        6
    );
    fs::remove_dir_all(&dir).unwrap();
}
