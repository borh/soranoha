use std::{collections::BTreeSet, fs, path::Path, time::Duration};

use ab_coverage::fidelity_report::{
    ReuseInputs, cached_parse, evaluate_requirements, evaluate_semantic_partition_coverage,
    evaluate_variant_coverage, inventory_matrix_projection, linked_family_recognition_missing,
    reuse_corpus_inventory, source_recognition_matches, unique_by_id, validate_links,
    validate_requirements,
};
use ab_encoding::hex_sha256;
use serde_json::{Value, json};

#[test]
fn inventory_matrix_projection_keeps_only_detector_inputs() {
    let original = r#"[[syntax]]
id = "row.one"
source_patterns = ["one"]
oracle_cases = ["case.one"]
notes = "draft"
[syntax.representability]
status = "typed"
"#;
    let reviewed = original
        .replace(r#"["case.one"]"#, r#"["case.one", "case.two"]"#)
        .replace("draft", "reviewed");
    let projection = |text: &str| inventory_matrix_projection(text.as_bytes()).unwrap();
    assert_eq!(projection(original), projection(&reviewed));
    assert_ne!(
        projection(original),
        projection(&reviewed.replacen(r#"["one"]"#, r#"["two"]"#, 1))
    );
    assert_eq!(
        projection(original),
        projection(&reviewed.replace(r#"status = "typed""#, r#"status = "raw_preserved""#))
    );
}

#[test]
fn corpus_reuse_requires_matching_provenance() {
    let matrix =
        "[[syntax]]\nid = \"row.one\"\nsource_patterns = [\"one\"]\nstatus = \"needs_research\"\n";
    let raw_inventory = json!({
        "inputs": {"variant_rows": ["row.one"]},
        "works_scanned": 1,
        "works_failed": 0,
        "markers_total": 1,
        "unknown_markers_total": 0,
        "unallowlisted_unknown_markers_total": 0,
        "allowlisted_unknown_markers_total": 0,
        "unknown_classes_total": 0,
        "unknown_classes_truncated": false,
        "decode_failures": [],
        "rows": {"row.one": {"occurrences": 1}},
    });
    let inventory = serde_json::to_vec(&raw_inventory).unwrap();
    let prior = json!({
        "inputs": {
            "matrix_sha256": hex_sha256(matrix.as_bytes()),
            "allowlist_sha256": "allowlist-v1",
            "inventory_sha256": "inventory-v1",
        },
        "corpus_inventory": {
            "revision": "revision-one",
            "index_sha256": "index-v1",
            "inventory_sha256": hex_sha256(&inventory),
        },
    });
    let rows = ["row.one".to_owned()];
    let inputs = || ReuseInputs {
        prior_report: &prior,
        prior_matrix: Some(matrix.as_bytes()),
        prior_inventory: &inventory,
        matrix: matrix.as_bytes(),
        allowlist_sha256: "allowlist-v1",
        inventory_tool_sha256: "inventory-v1",
        revision: "revision-one",
        index_sha256: "index-v1",
        variant_rows: &rows,
    };
    let corpus = reuse_corpus_inventory(&inputs()).unwrap();
    assert_eq!(corpus["rows"], raw_inventory["rows"]);
    let reviewed = matrix.replace("needs_research", "partial");
    assert!(
        reuse_corpus_inventory(&ReuseInputs {
            matrix: reviewed.as_bytes(),
            ..inputs()
        })
        .is_ok(),
        "matrix metadata the inventory does not read may change"
    );

    let other_rows = ["row.two".to_owned()];
    for (name, mismatched) in [
        (
            "revision",
            ReuseInputs {
                revision: "revision-two",
                ..inputs()
            },
        ),
        (
            "matrix",
            ReuseInputs {
                matrix: b"[[syntax]]\nid = \"row.one\"\n",
                ..inputs()
            },
        ),
        (
            "tool",
            ReuseInputs {
                inventory_tool_sha256: "inventory-v2",
                ..inputs()
            },
        ),
        (
            "rows",
            ReuseInputs {
                variant_rows: &other_rows,
                ..inputs()
            },
        ),
        (
            "index",
            ReuseInputs {
                index_sha256: "index-v2",
                ..inputs()
            },
        ),
        (
            "inventory",
            ReuseInputs {
                prior_inventory: b"{}",
                ..inputs()
            },
        ),
    ] {
        assert!(reuse_corpus_inventory(&mismatched).is_err(), "{name}");
    }
}

#[test]
fn source_recognition_follows_the_case_expectation() {
    let none = BTreeSet::new();
    let kunoji = BTreeSet::from(["iteration.kunoji".to_owned()]);
    assert!(source_recognition_matches("absent", false, &none, &kunoji));
    assert!(!source_recognition_matches("absent", true, &none, &kunoji));
    assert!(source_recognition_matches("unknown", true, &none, &kunoji));
    assert!(!source_recognition_matches("unknown", true, &kunoji, &none));
    assert!(!source_recognition_matches("linked", true, &none, &kunoji));
    assert!(source_recognition_matches("linked", true, &kunoji, &none));

    let mut malformed = json!({"recognition": {
        "expectation": "unknown",
        "missing_linked_families": ["ruby.basic"],
        "matches": true,
    }});
    assert!(!linked_family_recognition_missing(&malformed, "ruby.basic"));
    malformed["recognition"]["expectation"] = json!("linked");
    assert!(linked_family_recognition_missing(&malformed, "ruby.basic"));
}

#[test]
fn coverage_ranks_unexercised_corpus_forms_and_partitions() {
    let cases = [json!({
        "id": "adjacent",
        "status": "passed",
        "recognition": {"occurrences": [{
            "normalized_form": "basic-form",
            "families": ["emphasis.basic"],
            "semantic_partitions": ["referent:adjacent-unique"],
        }]},
    })];
    let corpus = json!({"rows": {"emphasis.basic": {
        "variants": {
            "basic-form": {"occurrences": 10, "samples": []},
            "rare-form": {"occurrences": 2, "samples": []},
            "common-gap": {"occurrences": 7, "samples": []},
        },
        "semantic_partitions": {
            "referent:adjacent-unique": {"occurrences": 10, "samples": []},
            "referent:missing": {"occurrences": 2, "samples": []},
            "referent:detached-repeated": {"occurrences": 7, "samples": []},
        },
    }}});
    let keys = |section: &Value, key: &str| -> Vec<String> {
        section["uncovered"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item[key].as_str().unwrap().to_owned())
            .collect()
    };

    let variants = &evaluate_variant_coverage(&cases, &corpus)["emphasis.basic"];
    assert_eq!(variants["covered_forms"], 1);
    assert_eq!(variants["covered_occurrences"], 10);
    assert_eq!(
        variants["cases_by_form"],
        json!({"basic-form": ["adjacent"]})
    );
    assert_eq!(
        keys(variants, "normalized_form"),
        ["common-gap", "rare-form"]
    );

    let partitions = &evaluate_semantic_partition_coverage(&cases, &corpus)["emphasis.basic"];
    assert_eq!(partitions["covered_partitions"], 1);
    assert_eq!(partitions["covered_occurrences"], 10);
    assert_eq!(
        keys(partitions, "partition"),
        ["referent:detached-repeated", "referent:missing"]
    );
}

#[test]
fn family_evidence_separates_shared_occurrence_forms_and_partitions() {
    let cases = [json!({
        "id": "shared",
        "status": "passed",
        "recognition": {"occurrences": [{
            "normalized_form": "generic-form",
            "families": ["generic.row", "specific.row"],
            "semantic_partitions": ["role:body"],
            "family_evidence": {"specific.row": {
                "normalized_form": "specific-form",
                "semantic_partitions": ["plane:2"],
            }},
        }]},
    })];
    let corpus = json!({"rows": {
        "generic.row": {
            "variants": {"generic-form": {"occurrences": 1}},
            "semantic_partitions": {"role:body": {"occurrences": 1}},
        },
        "specific.row": {
            "variants": {"specific-form": {"occurrences": 1}},
            "semantic_partitions": {"plane:2": {"occurrences": 1}},
        },
    }});
    let variants = evaluate_variant_coverage(&cases, &corpus);
    let partitions = evaluate_semantic_partition_coverage(&cases, &corpus);
    for row in ["generic.row", "specific.row"] {
        assert_eq!(variants[row]["covered_forms"], 1, "{row}");
        assert_eq!(partitions[row]["covered_partitions"], 1, "{row}");
    }
}

fn write_stub_parser(dir: &Path) -> std::path::PathBuf {
    let parser = dir.join("parser");
    fs::write(
        &parser,
        format!(
            "#!/bin/sh\ncat >/dev/null\necho run >> '{}'\necho '{{\"version\": 2, \"blocks\": []}}'\n",
            dir.join("calls").display()
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
fn parse_cache_tracks_source_and_executable_and_rejects_corruption() {
    let dir = std::env::temp_dir().join(format!("ab-fidelity-cache-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let parser = write_stub_parser(&dir);
    let cache = dir.join("cache");
    let parse = |parser_sha256: &str, source: &[u8]| {
        cached_parse(
            &parser,
            parser_sha256,
            source,
            &cache,
            Duration::from_secs(10),
        )
        .unwrap()
    };
    let calls = || {
        fs::read_to_string(dir.join("calls"))
            .unwrap()
            .lines()
            .count()
    };
    let output = json!({"version": 2, "blocks": []});

    assert_eq!(parse("parser-one", b"source-one"), (output.clone(), false));
    assert_eq!(parse("parser-one", b"source-one"), (output.clone(), true));
    assert_eq!(calls(), 1);

    let entry_path = fs::read_dir(&cache)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut entry: Value = serde_json::from_slice(&fs::read(&entry_path).unwrap()).unwrap();
    entry["aat"]["blocks"] = json!(["corruption"]);
    fs::write(&entry_path, serde_json::to_vec(&entry).unwrap()).unwrap();
    assert_eq!(parse("parser-one", b"source-one"), (output, false));

    parse("parser-one", b"source-two");
    parse("parser-two", b"source-one");
    assert_eq!(calls(), 4);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn oracle_links_and_assertion_evidence_must_resolve_in_both_directions() {
    let matrix = json!({"syntax": [{"id": "ruby", "oracle_cases": ["one"]}]});
    let oracle = json!({
        "evidence": [{"id": "source"}],
        "case": [{
            "id": "one",
            "syntax_row_ids": ["ruby"],
            "evidence_ids": ["source"],
            "review": [{"status": "reviewed"}],
            "projection": [{
                "kind": "aat",
                "version": 1,
                "status": "historical",
                "assertions": {"visible_text": "body"},
            }],
        }],
    });
    validate_links(&matrix, &oracle).unwrap();
    type Mutation = fn(&mut Value, &mut Value);
    let mutations: [(&str, Mutation); 4] = [
        ("row forgets the case", |matrix, _| {
            matrix["syntax"][0]["oracle_cases"] = json!([]);
        }),
        ("case forgets the row", |_, oracle| {
            oracle["case"][0]["syntax_row_ids"] = json!([]);
        }),
        ("duplicate projection", |_, oracle| {
            let projection = oracle["case"][0]["projection"][0].clone();
            oracle["case"][0]["projection"]
                .as_array_mut()
                .unwrap()
                .push(projection);
        }),
        ("unknown assertion evidence", |_, oracle| {
            oracle["case"][0]["projection"][0]["assertions"]["nodes"] =
                json!([{"evidence_ids": ["missing"]}]);
        }),
    ];
    for (name, mutate) in mutations {
        let (mut matrix, mut oracle) = (matrix.clone(), oracle.clone());
        mutate(&mut matrix, &mut oracle);
        assert!(validate_links(&matrix, &oracle).is_err(), "{name}");
    }
    assert!(unique_by_id(&[json!({"id": "one"}), json!({"id": "one"})]).is_err());
}

#[test]
fn requirement_links_bind_assertions_to_row_case_layer_and_aspect() {
    let matrix = json!({"syntax": [{"id": "ruby", "oracle_cases": ["one"]}]});
    let mut oracle = json!({
        "evidence": [{"id": "rule"}],
        "case": [{
            "id": "one",
            "syntax_row_ids": ["ruby"],
            "projection": [{
                "kind": "aat",
                "version": 2,
                "status": "active",
                "assertions": {"nodes": [{
                    "selector": "blocks.*",
                    "kind": "ruby",
                    "qualifies": ["target"],
                    "obligation_ids": ["ruby.target"],
                }]},
            }],
        }],
    });
    let requirements = json!({
        "requirements_version": 1,
        "family": [{
            "syntax_row_id": "ruby",
            "review_status": "draft",
            "target_policy": "required",
            "evidence_ids": ["rule"],
            "obligation": [{
                "id": "ruby.target",
                "aspect": "target",
                "statement": "The reading owns the selected base.",
                "required_cases": ["one"],
                "required_projections": ["aat"],
            }],
        }],
    });
    validate_requirements(&matrix, &oracle, &requirements).unwrap();
    oracle["case"][0]["projection"][0]["assertions"]["nodes"][0]["qualifies"] = json!(["value"]);
    let error = validate_requirements(&matrix, &oracle, &requirements).unwrap_err();
    assert!(
        error.to_string().contains("aspect is not qualified"),
        "{error}"
    );
}

#[test]
fn requirement_evaluation_needs_every_declared_case_and_layer() {
    let requirements = json!({"family": [{
        "syntax_row_id": "ruby",
        "review_status": "reviewed",
        "target_policy": "required",
        "evidence_ids": ["rule"],
        "obligation": [{
            "id": "ruby.target",
            "aspect": "target",
            "statement": "The reading owns the selected base.",
            "required_cases": ["one"],
            "required_projections": ["aat", "tei"],
        }],
    }]});
    let passed = json!({
        "assertions_match": true,
        "assertions": [
            {"expected": "visible text", "result": {"passed": true}},
            {"expected": {"obligation_ids": ["ruby.target"]}, "result": {"passed": true}},
        ],
    });
    let mut cases = [json!({"id": "one", "evaluation": passed, "tei_evaluation": passed})];
    let result = &evaluate_requirements(&requirements, &cases)["ruby"];
    assert_eq!(result["semantic_certification"], "qualified");
    assert_eq!(result["target_fidelity"], "qualified");

    cases[0]["tei_evaluation"]["assertions_match"] = json!(false);
    let result = &evaluate_requirements(&requirements, &cases)["ruby"];
    assert_eq!(result["obligations"][0]["status"], "failed");

    cases[0]["tei_evaluation"]["assertions_match"] = json!(true);
    cases[0]["tei_evaluation"]["assertions"][1]["result"]["passed"] = json!(false);
    let result = &evaluate_requirements(&requirements, &cases)["ruby"];
    assert_eq!(result["obligations"][0]["status"], "failed");
    assert_eq!(result["semantic_certification"], "not-assessed");
}
