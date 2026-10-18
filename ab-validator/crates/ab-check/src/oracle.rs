use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

fn select<'a>(root: &'a Value, path: &str) -> Result<Vec<&'a Value>> {
    if path == "**" {
        let mut pending = vec![root];
        let mut found = Vec::new();
        while let Some(value) = pending.pop() {
            match value {
                Value::Object(fields) => {
                    found.push(value);
                    pending.extend(fields.values());
                }
                Value::Array(values) => pending.extend(values),
                _ => {}
            }
        }
        return Ok(found);
    }
    let mut selected = vec![root];
    for part in path.split('.') {
        ensure!(!part.is_empty() && part != "**", "invalid selector {path}");
        let mut next = Vec::new();
        for value in selected {
            if part == "*" {
                next.extend(value.as_array().context("wildcard requires an array")?);
            } else if let Some(value) = value.get(part) {
                next.push(value);
            }
        }
        selected = next;
    }
    ensure!(
        selected.iter().all(|v| v.is_object() || v.is_array()),
        "scalar selectors are unsupported"
    );
    Ok(selected)
}

fn matches_fields(node: &Value, expected: &Value, nullable_gaiji: bool) -> bool {
    expected.as_object().is_some_and(|fields| {
        fields.iter().all(|(key, value)| {
            node.get(key).is_some_and(|actual| {
                actual == value
                    || (nullable_gaiji
                        && matches!(key.as_str(), "resolved" | "jis_code" | "unresolved_reason")
                        && (actual.is_null() || actual == "")
                        && (value.is_null() || value == ""))
            })
        })
    })
}

fn nested_field<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .try_fold(value, |current, part| current.get(part))
}

fn sorted_rows(mut rows: Vec<Value>) -> Vec<Value> {
    rows.sort_unstable_by_key(Value::to_string);
    rows
}

fn assertion(document: &Value, axis: &str, expected: &Value) -> Result<Value> {
    if axis == "visible_text" {
        let actual = ab_plaintext::visible_text_projection(document);
        return Ok(json!({"passed": expected == &actual, "actual": actual}));
    }
    let path = expected["selector"].as_str().context("missing selector")?;
    let values = select(document, path)?;
    match axis {
        "sequence" => {
            let actual: Vec<Value> = values
                .iter()
                .map(|v| {
                    v.as_array()
                        .map(|a| json!(a.iter().map(|n| &n["kind"]).collect::<Vec<_>>()))
                        .context("sequence selector must select arrays")
                })
                .collect::<Result<_>>()?;
            Ok(
                json!({"passed": !actual.is_empty() && actual.iter().all(|a| a == &expected["kinds"]), "actual": actual}),
            )
        }
        "nodes" | "gaiji" | "diagnostics" => {
            ensure!(
                values.iter().all(|v| v.is_object()),
                "node selector must select objects"
            );
            let kind = match axis {
                "gaiji" => Some("gaiji"),
                "nodes" => Some(expected["kind"].as_str().context("missing kind")?),
                "diagnostics" => None,
                _ => unreachable!(),
            };
            let mut fields = expected.get("fields").cloned().unwrap_or_else(|| json!({}));
            if axis == "gaiji" {
                for key in ["description", "resolved", "jis_code", "unresolved_reason"] {
                    if let Some(value) = expected.get(key) {
                        fields[key] = value.clone();
                    }
                }
            }
            let matching: Vec<_> = values
                .into_iter()
                .filter(|node| {
                    kind.is_none_or(|kind| node["kind"] == kind)
                        && matches_fields(node, &fields, axis == "gaiji")
                        && expected
                            .get("field_absent")
                            .and_then(Value::as_array)
                            .is_none_or(|absent| {
                                absent.iter().all(|name| {
                                    name.as_str().is_some_and(|name| node.get(name).is_none())
                                })
                            })
                })
                .collect();
            let count = matching.len() as u64;
            let exact = expected.get("count").and_then(Value::as_u64);
            let minimum = expected.get("min_count").and_then(Value::as_u64);
            let absent = expected["absent"] == true;
            let passed = exact.is_none_or(|exact| count == exact)
                && minimum.is_none_or(|minimum| count >= minimum)
                && (!absent || count == 0)
                && (absent || exact.is_some() || minimum.is_some() || count >= 1);
            Ok(json!({"passed": passed, "matching_count": count, "matching_nodes": matching}))
        }
        "field_multiset" => {
            ensure!(
                values.iter().all(|v| v.is_object()),
                "field multiset selector must select objects"
            );
            let kind = expected.get("kind").and_then(Value::as_str);
            let fields = expected["fields"]
                .as_array()
                .context("field multiset requires fields")?;
            let actual = sorted_rows(
                values
                    .into_iter()
                    .filter(|node| kind.is_none_or(|kind| node["kind"] == kind))
                    .map(|node| {
                        fields
                            .iter()
                            .map(|field| {
                                field
                                    .as_str()
                                    .and_then(|field| nested_field(node, field))
                                    .cloned()
                                    .unwrap_or(Value::Null)
                            })
                            .collect::<Vec<_>>()
                    })
                    .map(Value::Array)
                    .collect(),
            );
            let expected_rows = sorted_rows(
                expected["values"]
                    .as_array()
                    .context("field multiset requires values")?
                    .clone(),
            );
            Ok(json!({"passed": actual == expected_rows, "actual": actual}))
        }
        _ => bail!("unsupported assertion axis {axis}"),
    }
}

pub fn evaluate(request: &Value, document: &Value) -> Result<Value> {
    let schema: Value = serde_json::from_str(include_str!(
        "../../../schemas/records/aat-oracle-cases.schema.json"
    ))?;
    let validator = jsonschema::validator_for(&json!({
        "$ref": "#/$defs/Projection", "$defs": schema["$defs"]
    }))?;
    validator
        .validate(request)
        .map_err(|error| anyhow::anyhow!("invalid projection: {error}"))?;
    let expected_version = request["version"]
        .as_u64()
        .context("missing projection version")?;
    let expected_assertions = request["assertions"]
        .as_object()
        .context("missing projection assertions")?;
    let mut assertions = Vec::new();
    for (axis, value) in expected_assertions {
        let expectations = if axis == "visible_text" {
            vec![value]
        } else {
            value
                .as_array()
                .context("assertions must be an array")?
                .iter()
                .collect()
        };
        for (index, expected) in expectations.into_iter().enumerate() {
            let result = match assertion(document, axis, expected) {
                Ok(result) => result,
                Err(error) => json!({"passed": false, "error": error.to_string()}),
            };
            assertions.push(
                json!({"axis": axis, "index": index, "expected": expected, "result": result}),
            );
        }
    }
    let schema_error = crate::check::validate_aat_value(document)
        .err()
        .map(|e| e.to_string());
    let compatible = document["version"] == expected_version;
    let matched =
        !assertions.is_empty() && assertions.iter().all(|a| a["result"]["passed"] == true);
    let mut matching_aspects = assertions
        .iter()
        .filter(|assertion| assertion["result"]["passed"] == true)
        .flat_map(|assertion| {
            assertion["expected"]["qualifies"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
        })
        .collect::<Vec<_>>();
    matching_aspects.sort_unstable();
    matching_aspects.dedup();
    Ok(json!({
        "expected_aat_version": expected_version,
        "actual_aat_version": document.get("version"),
        "version_compatible": compatible,
        "schema_error": schema_error,
        "assertions_match": matched,
        "oracle_status": if !compatible { "version-mismatch" } else if schema_error.is_some() { "invalid-aat" } else if matched { "passed" } else { "failed" },
        "assertions": assertions,
        "matching_aspects": matching_aspects,
        "semantic_certification": "not-assessed",
        "target_fidelity": "not-assessed",
        "xml_fidelity": "not-assessed"
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_fidelity_requirements_match_their_schema() {
        let schema: Value = serde_json::from_str(include_str!(
            "../../../schemas/records/fidelity-requirements.schema.json"
        ))
        .unwrap();
        let requirements: toml::Value =
            toml::from_str(include_str!("../../../data/fidelity-requirements.toml")).unwrap();
        let requirements = serde_json::to_value(requirements).unwrap();
        jsonschema::validator_for(&schema)
            .unwrap()
            .validate(&requirements)
            .unwrap();
    }

    #[test]
    fn selectors_reject_wrong_shapes_and_do_not_pass_empty_sequences() {
        let doc = json!({"blocks": [{"kind": "paragraph", "content": []}]});
        assert!(select(&doc, "blocks.*.kind").is_err());
        assert!(select(&doc, "blocks.*.*").is_err());
        let result = assertion(
            &doc,
            "sequence",
            &json!({"selector":"blocks.*.children", "kinds":[]}),
        )
        .unwrap();
        assert_eq!(result["passed"], false);
    }

    #[test]
    fn diagnostic_assertions_match_fields_without_requiring_a_node_kind() {
        let doc = json!({"meta":{"warnings":[{
            "code":"empty-ruby-reading",
            "severity":"error",
            "span":{"byte_start":3,"byte_end":18}
        }]}});
        let result = assertion(
            &doc,
            "diagnostics",
            &json!({
                "selector":"meta.warnings.*",
                "count":1,
                "fields":{"code":"empty-ruby-reading","severity":"error"}
            }),
        )
        .unwrap();
        assert_eq!(result["passed"], true);
    }

    #[test]
    fn field_mutations_and_duplicate_matches_fail_exact_assertions() {
        let good = json!({"kind":"ruby", "base":"漢", "reading":"かん"});
        let expected = json!({"selector":"blocks.*", "kind":"ruby", "count":1, "fields":{"base":"漢", "reading":"かん"}});
        for (nodes, passed) in [
            (json!([good]), true),
            (json!([good, good]), false),
            (
                json!([{"kind":"ruby","base":"漢","reading":"wrong"}]),
                false,
            ),
        ] {
            assert_eq!(
                assertion(&json!({"blocks":nodes}), "nodes", &expected).unwrap()["passed"],
                passed
            );
        }
    }

    #[test]
    fn field_multisets_compare_nested_values_and_cardinality() {
        let expected = json!({
            "selector":"nodes.*",
            "kind":"style",
            "fields":["style_type", "decoration.kind"],
            "values":[["bouten", "傍点"], ["bosen", "傍線"]]
        });
        let good = json!({"nodes":[
            {"kind":"style", "style_type":"bosen", "decoration":{"kind":"傍線"}},
            {"kind":"text", "value":"ignored"},
            {"kind":"style", "style_type":"bouten", "decoration":{"kind":"傍点"}}
        ]});
        assert_eq!(
            assertion(&good, "field_multiset", &expected).unwrap()["passed"],
            true
        );

        let duplicate = json!({"nodes":[
            {"kind":"style", "style_type":"bouten", "decoration":{"kind":"傍点"}},
            {"kind":"style", "style_type":"bouten", "decoration":{"kind":"傍点"}}
        ]});
        assert_eq!(
            assertion(&duplicate, "field_multiset", &expected).unwrap()["passed"],
            false
        );
    }

    #[test]
    fn null_compatibility_does_not_allow_missing_fields_or_other_field_types() {
        assert!(matches_fields(
            &json!({"resolved":null}),
            &json!({"resolved":""}),
            true
        ));
        assert!(!matches_fields(&json!({}), &json!({"resolved":""}), true));
        assert!(!matches_fields(
            &json!({"base":null}),
            &json!({"base":""}),
            true
        ));
        assert!(!matches_fields(
            &json!({"resolved":null}),
            &json!({"resolved":""}),
            false
        ));
    }

    #[test]
    fn matching_assertions_report_version_mismatch_and_unassessed_semantics() {
        let result = evaluate(
            &json!({"kind":"aat","version":1,"status":"active","assertions":{"visible_text":""}}),
            &json!({"version":2,"blocks":[]}),
        )
        .unwrap();
        assert_eq!(result["assertions_match"], true);
        assert_eq!(result["oracle_status"], "version-mismatch");
        assert_eq!(result["semantic_certification"], "not-assessed");
        assert!(result["schema_error"].is_string());
    }

    #[test]
    fn only_passing_assertions_qualify_fidelity_aspects() {
        let result = evaluate(
            &json!({
                "kind":"aat",
                "version":2,
                "status":"active",
                "assertions":{
                    "nodes":[
                        {
                            "selector":"blocks.*.content.*",
                            "kind":"text",
                            "count":1,
                            "qualifies":["semantic-role"]
                        },
                        {
                            "selector":"blocks.*.content.*",
                            "kind":"ruby",
                            "count":1,
                            "qualifies":["target"]
                        }
                    ]
                }
            }),
            &json!({
                "version":2,
                "meta":{
                    "encoding":"utf-8",
                    "source_hash":"sha256:0000000000000000000000000000000000000000000000000000000000000000"
                },
                "blocks":[{
                    "kind":"paragraph",
                    "source_span":{"start":0,"end":1},
                    "content":[{"kind":"text","text":"x","source_span":{"start":0,"end":1}}]
                }]
            }),
        )
        .unwrap();
        assert_eq!(result["assertions_match"], false);
        assert_eq!(result["matching_aspects"], json!(["semantic-role"]));
    }

    #[test]
    fn invalid_assertion_fields_are_rejected() {
        for oracle in [
            json!({"nodes":[{"selector":"**", "kind":"ruby", "count":-1}]}),
            json!({"unsupported":[]}),
        ] {
            assert!(
                evaluate(
                    &json!({"kind":"aat","version":1,"status":"active","assertions":oracle}),
                    &json!({})
                )
                .is_err()
            );
        }
    }

    #[test]
    fn every_supplied_cardinality_constraint_applies() {
        let doc = json!({"blocks":[{"kind":"ruby"}]});
        for constraints in [
            json!({"count":1,"min_count":2}),
            json!({"count":1,"absent":true}),
        ] {
            let mut expected = json!({"selector":"blocks.*", "kind":"ruby"});
            expected
                .as_object_mut()
                .unwrap()
                .extend(constraints.as_object().unwrap().clone());
            assert_eq!(
                assertion(&doc, "nodes", &expected).unwrap()["passed"],
                false
            );
        }
    }
}
