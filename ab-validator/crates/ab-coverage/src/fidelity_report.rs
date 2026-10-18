//! Evaluation of registered oracle fixtures against fidelity requirements and
//! the corpus source inventory, assembled into `soranoha-fidelity-baseline/2`.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};

pub const REPORT_SCHEMA: &str = "soranoha-fidelity-baseline/2";
const PARSE_CACHE_SCHEMA: &str = "soranoha-fidelity-parse-cache/1";
const ASPECTS: [&str; 5] = ["semantic-role", "source-span", "target", "scope", "value"];

/// Parse a TOML document into the JSON data model.
///
/// # Errors
///
/// Returns an error when `bytes` are not UTF-8 TOML.
pub fn toml_value(bytes: &[u8]) -> Result<Value> {
    let table: toml::Value = toml::from_str(std::str::from_utf8(bytes)?).context("parse TOML")?;
    serde_json::to_value(table).context("convert TOML")
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn strings(value: &Value, key: &str) -> Vec<String> {
    array(value, key)
        .iter()
        .filter_map(|item| item.as_str().map(str::to_owned))
        .collect()
}

fn required_array<'a>(value: &'a Value, key: &str) -> Result<&'a [Value]> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .with_context(|| format!("missing array {key}"))
}

/// The matrix fields the source inventory reads: each row's id and patterns.
///
/// # Errors
///
/// Returns an error when the matrix does not parse or has no rows.
pub fn inventory_matrix_projection(matrix_bytes: &[u8]) -> Result<Value> {
    let matrix = toml_value(matrix_bytes)?;
    Ok(Value::Array(
        required_array(&matrix, "syntax")?
            .iter()
            .map(|row| {
                json!({
                    "id": row["id"],
                    "source_patterns": row.get("source_patterns").cloned().unwrap_or_else(|| json!([])),
                })
            })
            .collect(),
    ))
}

/// # Errors
///
/// Returns an error when two records share an id.
pub fn unique_by_id(records: &[Value]) -> Result<BTreeMap<String, &Value>> {
    let result = records
        .iter()
        .map(|record| (text(record, "id").to_owned(), record))
        .collect::<BTreeMap<_, _>>();
    if result.len() != records.len() {
        bail!("duplicate input identifier");
    }
    Ok(result)
}

#[must_use]
pub fn linked_family_recognition_missing(case: &Value, row_id: &str) -> bool {
    let recognition = case.get("recognition").unwrap_or(&Value::Null);
    let expectation = recognition
        .get("expectation")
        .and_then(Value::as_str)
        .unwrap_or("linked");
    expectation == "linked"
        && strings(recognition, "missing_linked_families")
            .iter()
            .any(|id| id == row_id)
}

#[must_use]
pub fn source_recognition_matches(
    recognition_mode: &str,
    has_occurrences: bool,
    found_families: &BTreeSet<String>,
    missing_families: &BTreeSet<String>,
) -> bool {
    match recognition_mode {
        "unknown" => has_occurrences && found_families.is_empty(),
        "absent" => !has_occurrences,
        _ => missing_families.is_empty(),
    }
}

/// Every assertion of a projection except its visible-text expectation.
pub fn projection_assertions(projection: &Value) -> impl Iterator<Item = &Value> {
    projection
        .get("assertions")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(axis, _)| axis.as_str() != "visible_text")
        .flat_map(|(_, assertions)| assertions.as_array().into_iter().flatten())
}

/// Check that rows, cases and evidence reference each other in both directions.
///
/// # Errors
///
/// Returns an error identifying the first broken link.
pub fn validate_links(matrix: &Value, oracle: &Value) -> Result<()> {
    let rows = unique_by_id(required_array(matrix, "syntax")?)?;
    let cases = unique_by_id(required_array(oracle, "case")?)?;
    let evidence = unique_by_id(required_array(oracle, "evidence")?)?;
    for case in cases.values() {
        let id = text(case, "id");
        if array(case, "review").is_empty() || array(case, "evidence_ids").is_empty() {
            bail!("missing review/evidence: {id}");
        }
        for row in strings(case, "syntax_row_ids") {
            if !rows.get(&row).is_some_and(|linked| {
                strings(linked, "oracle_cases")
                    .iter()
                    .any(|case_id| case_id == id)
            }) {
                bail!("missing reverse row link: {id} / {row}");
            }
        }
        let mut linked = strings(case, "evidence_ids");
        let projections = array(case, "projection");
        let identities = projections
            .iter()
            .map(|projection| {
                (
                    projection["kind"].to_string(),
                    projection["version"].to_string(),
                )
            })
            .collect::<BTreeSet<_>>();
        if identities.len() != projections.len() {
            bail!("duplicate projection identity: {id}");
        }
        for projection in projections {
            for assertion in projection_assertions(projection) {
                linked.extend(strings(assertion, "evidence_ids"));
            }
        }
        if linked
            .iter()
            .any(|evidence_id| !evidence.contains_key(evidence_id))
        {
            bail!("unknown evidence: {id}");
        }
    }
    for row in rows.values() {
        let row_id = text(row, "id");
        for case_id in strings(row, "oracle_cases") {
            if !cases.get(&case_id).is_some_and(|case| {
                strings(case, "syntax_row_ids")
                    .iter()
                    .any(|linked| linked == row_id)
            }) {
                bail!("missing reverse case link: {row_id} / {case_id}");
            }
        }
    }
    Ok(())
}

/// Check that requirement families, obligations, cases and assertions agree.
///
/// # Errors
///
/// Returns an error identifying the first inconsistency.
pub fn validate_requirements(matrix: &Value, oracle: &Value, requirements: &Value) -> Result<()> {
    if requirements.get("requirements_version") != Some(&json!(1)) {
        bail!("unsupported fidelity requirements version");
    }
    let rows = unique_by_id(required_array(matrix, "syntax")?)?;
    let cases = unique_by_id(required_array(oracle, "case")?)?;
    let evidence = unique_by_id(required_array(oracle, "evidence")?)?;
    let families = requirements
        .get("family")
        .and_then(Value::as_array)
        .context("fidelity requirements need a family array")?;
    let mut by_row = BTreeMap::new();
    for family in families {
        by_row.insert(text(family, "syntax_row_id").to_owned(), family);
    }
    if by_row.len() != families.len() {
        bail!("duplicate fidelity requirement family");
    }
    let mut obligations = BTreeMap::<String, (&str, &Value)>::new();
    for (row_id, family) in &by_row {
        if !rows.contains_key(row_id) {
            bail!("requirement family references missing row: {row_id}");
        }
        if !matches!(
            text(family, "review_status"),
            "draft" | "reviewed" | "disputed"
        ) {
            bail!("invalid requirement review status: {row_id}");
        }
        if !matches!(text(family, "target_policy"), "required" | "not_applicable") {
            bail!("invalid target policy: {row_id}");
        }
        let evidence_ids = strings(family, "evidence_ids");
        if evidence_ids.is_empty() || evidence_ids.iter().any(|id| !evidence.contains_key(id)) {
            bail!("unknown or missing requirement evidence: {row_id}");
        }
        if array(family, "obligation").is_empty() {
            bail!("requirement family has no obligations: {row_id}");
        }
        for obligation in array(family, "obligation") {
            let obligation_id = text(obligation, "id");
            if obligations.contains_key(obligation_id) {
                bail!("duplicate obligation identifier: {obligation_id}");
            }
            if !ASPECTS.contains(&text(obligation, "aspect")) {
                bail!("invalid obligation aspect: {obligation_id}");
            }
            if text(obligation, "statement").is_empty() {
                bail!("obligation has no statement: {obligation_id}");
            }
            let required_cases = strings(obligation, "required_cases");
            let required_projections = strings(obligation, "required_projections");
            if required_cases.is_empty() || required_projections.is_empty() {
                bail!("obligation has no executable coverage: {obligation_id}");
            }
            if required_cases.iter().collect::<BTreeSet<_>>().len() != required_cases.len()
                || required_projections.iter().collect::<BTreeSet<_>>().len()
                    != required_projections.len()
            {
                bail!("duplicate obligation coverage entry: {obligation_id}");
            }
            if required_projections
                .iter()
                .any(|projection| !matches!(projection.as_str(), "aat" | "tei"))
            {
                bail!("invalid obligation projection: {obligation_id}");
            }
            for case_id in &required_cases {
                let Some(case) = cases.get(case_id).filter(|case| {
                    strings(case, "syntax_row_ids")
                        .iter()
                        .any(|linked| linked == row_id)
                }) else {
                    bail!("obligation case is not linked to its row: {obligation_id}");
                };
                let active_kinds = array(case, "projection")
                    .iter()
                    .filter(|projection| text(projection, "status") == "active")
                    .map(|projection| text(projection, "kind"))
                    .collect::<BTreeSet<_>>();
                if required_projections
                    .iter()
                    .any(|kind| !active_kinds.contains(kind.as_str()))
                {
                    bail!("obligation case lacks an active projection: {obligation_id}");
                }
            }
            obligations.insert(obligation_id.to_owned(), (row_id.as_str(), obligation));
        }
    }
    for case in cases.values() {
        for projection in array(case, "projection") {
            for assertion in projection_assertions(projection) {
                for obligation_id in strings(assertion, "obligation_ids") {
                    let Some((row_id, obligation)) = obligations.get(&obligation_id) else {
                        bail!("assertion cites unknown obligation: {obligation_id}");
                    };
                    if !strings(case, "syntax_row_ids")
                        .iter()
                        .any(|linked| linked == row_id)
                        || !strings(obligation, "required_cases")
                            .iter()
                            .any(|case_id| case_id == text(case, "id"))
                        || !strings(obligation, "required_projections")
                            .iter()
                            .any(|kind| kind == text(projection, "kind"))
                    {
                        bail!("assertion cites obligation outside its coverage: {obligation_id}");
                    }
                    if !strings(assertion, "qualifies")
                        .iter()
                        .any(|aspect| aspect == text(obligation, "aspect"))
                    {
                        bail!("obligation aspect is not qualified: {obligation_id}");
                    }
                }
            }
        }
    }
    Ok(())
}

/// Obligations cited by passing assertions of a checker or TEI evaluation.
#[must_use]
pub fn passed_obligation_ids(evaluation: &Value) -> BTreeSet<String> {
    array(evaluation, "assertions")
        .iter()
        .filter(|assertion| assertion.get("expected").is_some_and(Value::is_object))
        .filter(|assertion| {
            assertion.get("result").unwrap_or(assertion).get("passed") == Some(&Value::Bool(true))
        })
        .flat_map(|assertion| strings(&assertion["expected"], "obligation_ids"))
        .collect()
}

/// Each family's obligations with the case projections that cover them.
#[must_use]
pub fn evaluate_requirements(requirements: &Value, cases: &[Value]) -> BTreeMap<String, Value> {
    let by_case = cases
        .iter()
        .map(|case| (text(case, "id"), case))
        .collect::<BTreeMap<_, _>>();
    let mut results = BTreeMap::new();
    for family in array(requirements, "family") {
        let mut obligation_results = Vec::new();
        for obligation in array(family, "obligation") {
            let mut coverage = Vec::new();
            for case_id in strings(obligation, "required_cases") {
                let case = by_case[case_id.as_str()];
                for projection in strings(obligation, "required_projections") {
                    let evaluation = case
                        .get(if projection == "aat" {
                            "evaluation"
                        } else {
                            "tei_evaluation"
                        })
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    let passed = evaluation.get("assertions_match") == Some(&Value::Bool(true))
                        && passed_obligation_ids(&evaluation).contains(text(obligation, "id"));
                    coverage.push(
                        json!({"case_id": case_id, "projection": projection, "passed": passed}),
                    );
                }
            }
            let passed = !coverage.is_empty() && coverage.iter().all(|item| item["passed"] == true);
            let mut result = obligation.as_object().cloned().unwrap_or_default();
            result.insert("coverage".to_owned(), Value::Array(coverage));
            result.insert(
                "status".to_owned(),
                json!(if passed { "passed" } else { "failed" }),
            );
            obligation_results.push(Value::Object(result));
        }
        let obligations_pass = !obligation_results.is_empty()
            && obligation_results
                .iter()
                .all(|item| item["status"] == "passed");
        let reviewed = text(family, "review_status") == "reviewed";
        let qualified = reviewed && obligations_pass;
        let targeted = text(family, "target_policy") == "not_applicable"
            || obligation_results
                .iter()
                .any(|item| item["aspect"] == "target");
        let mut result = family.as_object().cloned().unwrap_or_default();
        result.remove("obligation");
        result.insert("obligations".to_owned(), Value::Array(obligation_results));
        result.insert("obligations_pass".to_owned(), json!(obligations_pass));
        result.insert(
            "semantic_certification".to_owned(),
            json!(if qualified {
                "qualified"
            } else {
                "not-assessed"
            }),
        );
        result.insert(
            "target_fidelity".to_owned(),
            json!(if qualified && targeted {
                "qualified"
            } else {
                "not-assessed"
            }),
        );
        results.insert(
            text(family, "syntax_row_id").to_owned(),
            Value::Object(result),
        );
    }
    results
}

/// A case's recognized occurrences and, recursively, their components.
fn recognition_occurrences(case: &Value) -> Vec<&Value> {
    let mut pending = case
        .get("recognition")
        .map(|recognition| array(recognition, "occurrences").iter().collect::<Vec<_>>())
        .unwrap_or_default();
    let mut occurrences = Vec::new();
    while let Some(occurrence) = pending.pop() {
        occurrences.push(occurrence);
        pending.extend(array(occurrence, "components"));
    }
    occurrences
}

fn family_recognition<'a>(occurrence: &'a Value, row_id: &str) -> &'a Value {
    occurrence
        .get("family_evidence")
        .and_then(|evidence| evidence.get(row_id))
        .unwrap_or(occurrence)
}

/// Keys each passing case exercises per row, read by `select` from each
/// family's recognition evidence.
fn exercised(
    cases: &[Value],
    select: impl Fn(&Value) -> Vec<String>,
) -> BTreeMap<String, BTreeMap<String, BTreeSet<String>>> {
    let mut exercised = BTreeMap::<String, BTreeMap<String, BTreeSet<String>>>::new();
    for case in cases.iter().filter(|case| text(case, "status") == "passed") {
        for occurrence in recognition_occurrences(case) {
            for row_id in strings(occurrence, "families") {
                for key in select(family_recognition(occurrence, &row_id)) {
                    exercised
                        .entry(row_id.clone())
                        .or_default()
                        .entry(key)
                        .or_default()
                        .insert(text(case, "id").to_owned());
                }
            }
        }
    }
    exercised
}

/// Compare each row's corpus inventory entries in `section` with the keys
/// passing cases exercise.
fn coverage_section(
    corpus: &Value,
    section: &str,
    exercised: &BTreeMap<String, BTreeMap<String, BTreeSet<String>>>,
    [key_name, count_name, total_name, cases_name]: [&str; 4],
) -> Value {
    let mut result = Map::new();
    for (row_id, row) in corpus
        .get("rows")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        let Some(entries) = row
            .get(section)
            .and_then(Value::as_object)
            .filter(|entries| !entries.is_empty())
        else {
            continue;
        };
        let empty = BTreeMap::new();
        let covered = exercised.get(row_id).unwrap_or(&empty);
        let occurrences = |details: &Value| details["occurrences"].as_u64().unwrap_or(0);
        let mut uncovered = entries
            .iter()
            .filter(|(key, _)| !covered.contains_key(*key))
            .map(|(key, details)| {
                let mut item = details.as_object().cloned().unwrap_or_default();
                item.insert(key_name.to_owned(), json!(key));
                (occurrences(details), key.clone(), Value::Object(item))
            })
            .collect::<Vec<_>>();
        uncovered.sort_by(|left, right| (right.0, &left.1).cmp(&(left.0, &right.1)));
        result.insert(
            row_id.clone(),
            json!({
                total_name: entries.len(),
                count_name: entries.keys().filter(|key| covered.contains_key(*key)).count(),
                "covered_occurrences": entries
                    .iter()
                    .filter(|(key, _)| covered.contains_key(*key))
                    .map(|(_, details)| occurrences(details))
                    .sum::<u64>(),
                "corpus_occurrences": entries.values().map(occurrences).sum::<u64>(),
                cases_name: covered
                    .iter()
                    .filter(|(key, _)| entries.contains_key(*key))
                    .map(|(key, case_ids)| (key.clone(), json!(case_ids)))
                    .collect::<Map<_, _>>(),
                "uncovered": uncovered.into_iter().map(|(_, _, item)| item).collect::<Vec<_>>(),
            }),
        );
    }
    Value::Object(result)
}

/// Compare opt-in corpus forms with the forms passing active cases exercise.
#[must_use]
pub fn evaluate_variant_coverage(cases: &[Value], corpus: &Value) -> Value {
    let exercised = exercised(cases, |recognition| {
        recognition
            .get("normalized_form")
            .and_then(Value::as_str)
            .filter(|form| !form.is_empty())
            .map(str::to_owned)
            .into_iter()
            .collect()
    });
    coverage_section(
        corpus,
        "variants",
        &exercised,
        [
            "normalized_form",
            "covered_forms",
            "corpus_forms",
            "cases_by_form",
        ],
    )
}

/// Compare corpus source-context partitions with passing active cases.
#[must_use]
pub fn evaluate_semantic_partition_coverage(cases: &[Value], corpus: &Value) -> Value {
    let exercised = exercised(cases, |recognition| {
        strings(recognition, "semantic_partitions")
    });
    coverage_section(
        corpus,
        "semantic_partitions",
        &exercised,
        [
            "partition",
            "covered_partitions",
            "corpus_partitions",
            "cases_by_partition",
        ],
    )
}

/// The report's record of a corpus inventory run.
///
/// # Errors
///
/// Returns an error when the inventory report lacks a summary field.
pub fn corpus_inventory_record(
    inventory_report: &Value,
    inventory_sha256: &str,
    revision: &str,
    index_sha256: &str,
) -> Result<Value> {
    let mut corpus = Map::new();
    corpus.insert("scope".to_owned(), json!("provided-source-index"));
    corpus.insert("revision".to_owned(), json!(revision));
    corpus.insert("index_sha256".to_owned(), json!(index_sha256));
    corpus.insert("inventory_sha256".to_owned(), json!(inventory_sha256));
    corpus.insert("semantic_certification".to_owned(), json!("not-assessed"));
    for key in [
        "works_scanned",
        "works_failed",
        "markers_total",
        "unknown_markers_total",
        "unallowlisted_unknown_markers_total",
        "allowlisted_unknown_markers_total",
        "unknown_classes_total",
        "unknown_classes_truncated",
        "decode_failures",
        "rows",
    ] {
        let value = inventory_report
            .get(key)
            .with_context(|| format!("inventory report lacks {key}"))?;
        corpus.insert(key.to_owned(), value.clone());
    }
    if let Some(cache) = inventory_report.get("cache") {
        corpus.insert("cache".to_owned(), cache.clone());
    }
    Ok(Value::Object(corpus))
}

/// Everything a prior report must match before its corpus inventory is reused.
pub struct ReuseInputs<'a> {
    pub prior_report: &'a Value,
    /// Bytes of the matrix the prior report recorded, when it differs.
    pub prior_matrix: Option<&'a [u8]>,
    pub prior_inventory: &'a [u8],
    pub matrix: &'a [u8],
    pub allowlist_sha256: &'a str,
    pub inventory_tool_sha256: &'a str,
    pub revision: &'a str,
    pub index_sha256: &'a str,
    pub variant_rows: &'a [String],
}

/// Verify a prior report's provenance and return its corpus inventory record.
///
/// # Errors
///
/// Returns an error identifying the first provenance mismatch.
pub fn reuse_corpus_inventory(inputs: &ReuseInputs<'_>) -> Result<Value> {
    let prior_inputs = inputs.prior_report.get("inputs").unwrap_or(&Value::Null);
    for (name, expected) in [
        ("allowlist_sha256", inputs.allowlist_sha256),
        ("inventory_sha256", inputs.inventory_tool_sha256),
    ] {
        if prior_inputs.get(name).and_then(Value::as_str) != Some(expected) {
            bail!("reusable corpus inventory {name} mismatch");
        }
    }
    let matrix_sha256 = ab_encoding::hex_sha256(inputs.matrix);
    let matrix_matches =
        prior_inputs.get("matrix_sha256").and_then(Value::as_str) == Some(&matrix_sha256);
    if !matrix_matches {
        let same_projection = match inputs.prior_matrix {
            Some(prior) => {
                inventory_matrix_projection(prior)? == inventory_matrix_projection(inputs.matrix)?
            }
            None => false,
        };
        if !same_projection {
            bail!("reusable corpus inventory matrix mismatch");
        }
    }
    let Some(prior_corpus) = inputs
        .prior_report
        .get("corpus_inventory")
        .filter(|corpus| corpus.is_object())
    else {
        bail!("reusable report has no corpus inventory");
    };
    if prior_corpus.get("revision").and_then(Value::as_str) != Some(inputs.revision) {
        bail!("reusable corpus revision mismatch");
    }
    if prior_corpus.get("index_sha256").and_then(Value::as_str) != Some(inputs.index_sha256) {
        bail!("reusable source index mismatch");
    }
    let inventory_sha256 = ab_encoding::hex_sha256(inputs.prior_inventory);
    if prior_corpus.get("inventory_sha256").and_then(Value::as_str) != Some(&inventory_sha256) {
        bail!("reusable corpus inventory digest mismatch");
    }
    let inventory_report: Value = serde_json::from_slice(inputs.prior_inventory)
        .context("parse reusable corpus inventory")?;
    let recorded_rows = inventory_report
        .pointer("/inputs/variant_rows")
        .or_else(|| inventory_report.pointer("/cache/variant_rows"))
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.as_str().map(str::to_owned))
                .collect::<BTreeSet<_>>()
        });
    let expected_rows = inputs.variant_rows.iter().cloned().collect::<BTreeSet<_>>();
    if recorded_rows.as_ref() != Some(&expected_rows) {
        bail!("reusable corpus variant-row selection mismatch");
    }
    if let Some(cache) = inventory_report.get("cache") {
        if cache.get("executable_sha256").and_then(Value::as_str)
            != Some(inputs.inventory_tool_sha256)
        {
            bail!("reusable corpus cache tool mismatch");
        }
        if matrix_matches
            && cache.get("matrix_sha256").and_then(Value::as_str) != Some(&matrix_sha256)
        {
            bail!("reusable corpus cache matrix mismatch");
        }
    }
    corpus_inventory_record(
        &inventory_report,
        &inventory_sha256,
        inputs.revision,
        inputs.index_sha256,
    )
}

fn counter<'a>(values: impl Iterator<Item = &'a str>) -> Value {
    let mut counts = BTreeMap::<&str, u64>::new();
    for value in values {
        *counts.entry(value).or_default() += 1;
    }
    json!(counts)
}

/// The report's per-row qualification, ranked gap worklist and summary.
pub struct Assembled {
    pub rows: Vec<Value>,
    pub worklist: Vec<Value>,
    pub summary: Map<String, Value>,
}

/// Assemble the per-row qualification, the ranked gap worklist and the summary.
///
/// # Errors
///
/// Returns an error when a row refers to a case that was not evaluated.
pub fn assemble(
    matrix: &Value,
    cases: &[Value],
    requirement_results: &BTreeMap<String, Value>,
    corpus: Option<&Value>,
    variant_coverage: &Value,
    semantic_partition_coverage: &Value,
) -> Result<Assembled> {
    let by_id = cases
        .iter()
        .map(|case| (text(case, "id"), case))
        .collect::<BTreeMap<_, _>>();
    let mut rows = Vec::new();
    let mut worklist = Vec::new();
    for row in required_array(matrix, "syntax")? {
        let row_id = text(row, "id");
        let requirement = requirement_results.get(row_id);
        let linked = strings(row, "oracle_cases")
            .iter()
            .map(|case_id| {
                by_id
                    .get(case_id.as_str())
                    .copied()
                    .with_context(|| format!("unknown case {case_id}"))
            })
            .collect::<Result<Vec<_>>>()?;
        let active = linked
            .iter()
            .copied()
            .filter(|case| text(case, "status") != "retired")
            .collect::<Vec<_>>();
        let matching_aspects = active
            .iter()
            .filter_map(|case| case.get("evaluation"))
            .filter(|evaluation| text(evaluation, "oracle_status") == "passed")
            .flat_map(|evaluation| strings(evaluation, "matching_aspects"))
            .collect::<BTreeSet<_>>();
        let xml_qualified = !active.is_empty()
            && active.iter().all(|case| {
                case.pointer("/tei_validation/valid") == Some(&Value::Bool(true))
                    && case.pointer("/tei_evaluation/assertions_match") == Some(&Value::Bool(true))
            });
        let mut gaps = BTreeSet::new();
        match requirement {
            None => {
                gaps.extend([
                    "semantic-obligations-not-declared",
                    "semantic-obligations-not-exhaustively-qualified",
                    "exact-targets-not-assessed",
                ]);
            }
            Some(requirement) => {
                if text(requirement, "review_status") != "reviewed" {
                    gaps.insert("semantic-obligations-review-incomplete");
                }
                if text(requirement, "semantic_certification") != "qualified" {
                    gaps.insert("semantic-obligations-not-exhaustively-qualified");
                }
                if text(requirement, "target_fidelity") != "qualified" {
                    gaps.insert("exact-targets-not-assessed");
                }
            }
        }
        if !xml_qualified {
            gaps.insert("xml-not-assessed");
        }
        if active.is_empty() {
            gaps.insert("no-active-oracle-case");
        }
        if semantic_partition_coverage
            .get(row_id)
            .is_some_and(|coverage| !array(coverage, "uncovered").is_empty())
        {
            gaps.insert("semantic-partitions-uncovered");
        }
        for case in &active {
            if text(case, "status") != "passed" {
                gaps.insert(text(case, "status"));
            }
            if case.get("current_projection_registered") == Some(&Value::Bool(true))
                && case.pointer("/evaluation/assertions_match") == Some(&Value::Bool(false))
            {
                gaps.insert("assertion-mismatch-needs-triage");
            }
            if case.pointer("/review/status").and_then(Value::as_str) != Some("reviewed") {
                gaps.insert("oracle-needs-review");
            }
            if linked_family_recognition_missing(case, row_id) {
                gaps.insert("source-family-not-detected");
            }
        }
        let mut entry = json!({
            "id": row_id,
            "category": row["category"],
            "reference_sources": row["reference_sources"],
            "validation_properties": row["validation_properties"],
            "adapter_expectations": row["adapter_expectations"],
            "oracle_cases": linked.iter().map(|case| text(case, "id")).collect::<Vec<_>>(),
            "case_results": counter(linked.iter().map(|case| text(case, "status"))),
            "matching_aspects": matching_aspects,
            "requirements": requirement,
            "semantic_certification": requirement.map_or("not-assessed", |requirement| text(requirement, "semantic_certification")),
            "target_fidelity": requirement.map_or("not-assessed", |requirement| text(requirement, "target_fidelity")),
            "xml_qualified": xml_qualified,
            "gaps": gaps,
        });
        if let Some(corpus) = corpus {
            entry["corpus_recognition"] = corpus
                .pointer(&format!(
                    "/rows/{}",
                    row_id.replace('~', "~0").replace('/', "~1")
                ))
                .cloned()
                .unwrap_or_else(
                    || json!({"works_with_marker": 0, "occurrences": 0, "sample_works": []}),
                );
            if let Some(coverage) = variant_coverage.get(row_id) {
                entry["variant_coverage"] = coverage.clone();
            }
            if let Some(coverage) = semantic_partition_coverage.get(row_id) {
                entry["semantic_partition_coverage"] = coverage.clone();
            }
        }
        let occurrences = entry
            .pointer("/corpus_recognition/occurrences")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        for gap in &gaps {
            worklist.push(json!({
                "syntax_row_id": row_id,
                "category": row["category"],
                "gap": gap,
                "corpus_occurrences": occurrences,
            }));
        }
        rows.push(entry);
    }
    worklist.sort_by(|left, right| {
        let key = |item: &Value| {
            (
                std::cmp::Reverse(item["corpus_occurrences"].as_u64().unwrap_or(0)),
                text(item, "syntax_row_id").to_owned(),
                text(item, "gap").to_owned(),
            )
        };
        key(left).cmp(&key(right))
    });

    let is_true = |value: Option<&Value>| value == Some(&Value::Bool(true));
    let mut summary = Map::new();
    summary.insert("syntax_rows".to_owned(), json!(rows.len()));
    summary.insert("cases".to_owned(), json!(cases.len()));
    summary.insert(
        "case_results".to_owned(),
        counter(cases.iter().map(|case| text(case, "status"))),
    );
    summary.insert(
        "assertions_matching_cases".to_owned(),
        json!(
            cases
                .iter()
                .filter(|case| is_true(case.pointer("/evaluation/assertions_match")))
                .count()
        ),
    );
    summary.insert(
        "current_projections_registered".to_owned(),
        json!(
            cases
                .iter()
                .filter(|case| is_true(case.get("current_projection_registered")))
                .count()
        ),
    );
    summary.insert(
        "current_projections_passed".to_owned(),
        json!(
            cases
                .iter()
                .filter(|case| text(case, "status") == "passed")
                .count()
        ),
    );
    summary.insert(
        "current_projections_missing".to_owned(),
        json!(
            cases
                .iter()
                .filter(
                    |case| case.get("current_projection_registered") == Some(&Value::Bool(false))
                )
                .count()
        ),
    );
    summary.insert(
        "rows_with_qualified_aspects".to_owned(),
        counter(rows.iter().flat_map(|row| {
            array(row, "matching_aspects")
                .iter()
                .filter_map(Value::as_str)
        })),
    );
    summary.insert(
        "rows_without_active_cases".to_owned(),
        json!(
            rows.iter()
                .filter(|row| array(row, "gaps")
                    .iter()
                    .any(|gap| gap == "no-active-oracle-case"))
                .count()
        ),
    );
    summary.insert(
        "rows_with_xml_qualification".to_owned(),
        json!(
            rows.iter()
                .filter(|row| row["xml_qualified"] == true)
                .count()
        ),
    );
    summary.insert(
        "families_with_declared_obligations".to_owned(),
        json!(requirement_results.len()),
    );
    summary.insert(
        "families_with_reviewed_obligations".to_owned(),
        json!(
            requirement_results
                .values()
                .filter(|family| text(family, "review_status") == "reviewed")
                .count()
        ),
    );
    summary.insert(
        "families_semantically_qualified".to_owned(),
        json!(
            requirement_results
                .values()
                .filter(|family| text(family, "semantic_certification") == "qualified")
                .count()
        ),
    );
    summary.insert(
        "declared_obligations".to_owned(),
        json!(
            requirement_results
                .values()
                .map(|family| array(family, "obligations").len())
                .sum::<usize>()
        ),
    );
    summary.insert(
        "passing_obligations".to_owned(),
        json!(
            requirement_results
                .values()
                .flat_map(|family| array(family, "obligations"))
                .filter(|obligation| text(obligation, "status") == "passed")
                .count()
        ),
    );
    Ok(Assembled {
        rows,
        worklist,
        summary,
    })
}

/// Parse `source` with `parser`, reusing an AAT cached under `cache` whose
/// parser, source and content digests still match.
///
/// # Errors
///
/// Returns an error when the parser fails or its output cannot be cached.
pub fn cached_parse(
    parser: &Path,
    parser_sha256: &str,
    source: &[u8],
    cache: &Path,
    timeout: Duration,
) -> Result<(Value, bool)> {
    let identity = json!({
        "schema": PARSE_CACHE_SCHEMA,
        "parser_sha256": parser_sha256,
        "source_sha256": ab_encoding::hex_sha256(source),
    });
    let key = ab_encoding::hex_sha256(&serde_json::to_vec(&identity)?);
    let path = cache.join(format!("{key}.json"));
    if let Some(entry) = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        && entry.get("identity") == Some(&identity)
        && let Some(aat) = entry.get("aat")
        && entry.get("aat_sha256").and_then(Value::as_str)
            == Some(&ab_encoding::hex_sha256(&serde_json::to_vec(aat)?))
    {
        return Ok((aat.clone(), true));
    }
    let output = crate::parser_exec::run_checked(parser, ["--mode", "aat"], Some(source), timeout)?;
    let aat: Value = serde_json::from_slice(&output).context("parse parser output")?;
    crate::parser_exec::write_atomically(
        &path,
        &serde_json::to_vec_pretty(&json!({
            "identity": identity,
            "aat_sha256": ab_encoding::hex_sha256(&serde_json::to_vec(&aat)?),
            "aat": aat,
        }))?,
    )?;
    Ok((aat, false))
}
