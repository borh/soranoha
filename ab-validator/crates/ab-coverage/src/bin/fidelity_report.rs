//! Evaluate every registered oracle fixture through the parser, checker,
//! parser-IR converter, source inventory and TEI probe, and report per-row
//! qualification against the fidelity requirements and the corpus inventory.

use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use ab_coverage::{
    fidelity_report::{
        Assembled, REPORT_SCHEMA, ReuseInputs, assemble, cached_parse, corpus_inventory_record,
        evaluate_requirements, evaluate_semantic_partition_coverage, evaluate_variant_coverage,
        reuse_corpus_inventory, source_recognition_matches, toml_value, unique_by_id,
        validate_links, validate_requirements,
    },
    parser_exec::{ToolFailed, run_checked, run_tool, write_atomically},
};
use ab_encoding::hex_sha256;
use anyhow::{Context, Result, bail};
use clap::Parser;
use rayon::prelude::*;
use serde_json::{Map, Value, json};

const TOOL_TIMEOUT: Duration = Duration::from_secs(60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Parser)]
struct Cli {
    #[arg(long)]
    parser: PathBuf,
    #[arg(long)]
    checker: PathBuf,
    #[arg(long)]
    inventory: PathBuf,
    #[arg(long)]
    converter: PathBuf,
    #[arg(long)]
    tei_probe: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[arg(long, default_value = "ab-validator/data/aozora-syntax-coverage.toml")]
    matrix: PathBuf,
    #[arg(long, default_value = "ab-validator/data/aat-oracle-cases.toml")]
    oracle: PathBuf,
    #[arg(long, default_value = "ab-validator/data/fidelity-requirements.toml")]
    requirements: PathBuf,
    #[arg(
        long,
        default_value = "ab-validator/schemas/records/fidelity-requirements.schema.json"
    )]
    requirements_schema: PathBuf,
    #[arg(
        long,
        default_value = "ab-validator/data/aat-to-parser-ir-mapping-v2.json"
    )]
    mapping: PathBuf,
    #[arg(
        long,
        default_value = "ab-validator/data/aozora-source-inventory-allowlist.toml"
    )]
    allowlist: PathBuf,
    #[arg(long, requires = "corpus")]
    index: Option<PathBuf>,
    #[arg(long, requires = "index")]
    corpus: Option<PathBuf>,
    /// Reuse the sibling corpus-inventory.json of this prior report after
    /// verifying its provenance.
    #[arg(long, requires = "index")]
    reuse_corpus_report: Option<PathBuf>,
    /// Reuse content-verified per-edition source inventory from this directory.
    #[arg(long, requires = "index")]
    inventory_cache: Option<PathBuf>,
    /// Include normalized corpus forms and samples for a syntax row.
    #[arg(long = "variant-row")]
    variant_rows: Vec<String>,
}

fn read(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).with_context(|| format!("read {}", path.display()))
}

fn read_json(path: &Path) -> Result<Value> {
    serde_json::from_slice(&read(path)?).with_context(|| format!("parse {}", path.display()))
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    write_atomically(path, &serde_json::to_vec_pretty(value)?)
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

struct Run<'a> {
    cli: &'a Cli,
    parser_sha256: String,
    matrix_path: PathBuf,
    matrix_sha256: String,
}

/// The case's projections of `kind` that apply at `version`.
fn matching_projections<'a>(
    case: &'a Value,
    kind: &str,
    version: Option<&Value>,
) -> Vec<&'a Value> {
    case["projection"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|projection| {
            text(projection, "kind") == kind
                && projection.get("version") == version
                && text(projection, "status") != "retired"
        })
        .collect()
}

/// Run one active case through every tool, filling `result`.
fn evaluate_case(
    run: &Run<'_>,
    case: &Value,
    case_dir: &Path,
    source: &[u8],
    result: &mut Map<String, Value>,
) -> Result<bool> {
    let cli = run.cli;
    let source_hash = hex_sha256(source);
    let source_path = case_dir.join("source.txt");
    let (aat, hit) = cached_parse(
        &cli.parser,
        &run.parser_sha256,
        source,
        &cli.out.join("cache"),
        TOOL_TIMEOUT,
    )?;
    let recorded = aat.pointer("/meta/source_hash").and_then(Value::as_str);
    if recorded != Some(source_hash.as_str()) && recorded != Some(&format!("sha256:{source_hash}"))
    {
        bail!("parser source hash mismatch");
    }
    let aat_path = case_dir.join("aat.json");
    write_json(&aat_path, &aat)?;
    let parser_ir_path = case_dir.join("parser-ir.json");
    let divergence_path = case_dir.join("divergence.json");
    run_checked(
        &cli.converter,
        [
            "convert".as_ref(),
            "--aat".as_ref(),
            aat_path.as_os_str(),
            "--mapping".as_ref(),
            cli.mapping.as_os_str(),
            "--work-content-hash".as_ref(),
            format!("sha256:{source_hash}").as_ref(),
            "--parser-ir-out".as_ref(),
            parser_ir_path.as_os_str(),
            "--divergence-out".as_ref(),
            divergence_path.as_os_str(),
        ],
        None,
        TOOL_TIMEOUT,
    )?;
    result.insert("parser_ir".to_owned(), read_json(&parser_ir_path)?);
    result.insert("divergence".to_owned(), read_json(&divergence_path)?);

    let matching = matching_projections(case, "aat", aat.get("version"));
    if matching.len() > 1 {
        bail!("multiple projections match parser output");
    }
    if let Some(projection) = matching.first() {
        result.insert("current_projection_registered".to_owned(), json!(true));
        let request_path = case_dir.join("projection.json");
        write_json(&request_path, projection)?;
        let evaluation = run_checked(
            &cli.checker,
            [
                "--aat".as_ref(),
                aat_path.as_os_str(),
                "--oracle".as_ref(),
                request_path.as_os_str(),
            ],
            None,
            TOOL_TIMEOUT,
        )?;
        result.insert(
            "evaluation".to_owned(),
            serde_json::from_slice(&evaluation).context("parse checker output")?,
        );
    } else {
        result.insert("current_projection_registered".to_owned(), json!(false));
        result.insert(
            "evaluation".to_owned(),
            json!({
                "actual_aat_version": aat.get("version"),
                "oracle_status": "projection-not-registered",
                "assertions_match": false,
                "semantic_certification": "not-assessed",
                "target_fidelity": "not-assessed",
                "xml_fidelity": "not-assessed",
            }),
        );
    }

    let inventory_path = case_dir.join("inventory.json");
    run_checked(
        &cli.inventory,
        [
            "--source".as_ref(),
            source_path.as_os_str(),
            "--matrix".as_ref(),
            run.matrix_path.as_os_str(),
            "--output-json".as_ref(),
            inventory_path.as_os_str(),
        ],
        None,
        TOOL_TIMEOUT,
    )?;
    let inventory = read_json(&inventory_path)?;
    if text(&inventory, "source_sha256") != format!("sha256:{source_hash}") {
        bail!("inventory source hash mismatch");
    }
    if text(&inventory, "matrix_sha256") != format!("sha256:{}", run.matrix_sha256) {
        bail!("inventory matrix hash mismatch");
    }
    let occurrences = inventory
        .get("occurrences")
        .and_then(Value::as_array)
        .context("inventory has no occurrences")?;
    let found = occurrences
        .iter()
        .flat_map(|item| item["families"].as_array().into_iter().flatten())
        .filter_map(|family| family.as_str().map(str::to_owned))
        .collect::<std::collections::BTreeSet<_>>();
    let missing = case["syntax_row_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row.as_str().map(str::to_owned))
        .filter(|row| !found.contains(row))
        .collect::<std::collections::BTreeSet<_>>();
    let expectation = case
        .get("source_recognition")
        .and_then(Value::as_str)
        .unwrap_or("linked");
    let matches =
        source_recognition_matches(expectation, !occurrences.is_empty(), &found, &missing);
    result.insert(
        "recognition".to_owned(),
        json!({
            "found_families": found,
            "missing_linked_families": missing,
            "occurrences": occurrences,
            "decode_outcome": inventory.get("decode_outcome").context("inventory has no decode_outcome")?,
            "expectation": expectation,
            "matches": matches,
        }),
    );
    let status = if matches {
        result["evaluation"]["oracle_status"].clone()
    } else {
        json!("failed-source-recognition")
    };
    result.insert("status".to_owned(), status);
    Ok(hit)
}

/// The case's report entry and whether its parse came from the cache.
fn case_result(run: &Run<'_>, case: &Value) -> Result<(Value, bool)> {
    let review = case["review"]
        .as_array()
        .and_then(|reviews| reviews.last())
        .context("case has no review")?;
    let mut result = Map::new();
    result.insert("id".to_owned(), case["id"].clone());
    result.insert("syntax_row_ids".to_owned(), case["syntax_row_ids"].clone());
    result.insert("review".to_owned(), review.clone());
    result.insert("evidence_ids".to_owned(), case["evidence_ids"].clone());
    result.insert("source_utf8".to_owned(), case["source_utf8"].clone());
    result.insert(
        "projections".to_owned(),
        case["projection"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|projection| {
                json!({"kind": projection["kind"], "version": projection["version"], "status": projection["status"]})
            })
            .collect(),
    );
    if text(review, "status") == "retired" {
        result.insert("status".to_owned(), json!("retired"));
        return Ok((Value::Object(result), false));
    }
    let source = text(case, "source_utf8").as_bytes();
    let case_dir = run
        .cli
        .out
        .join("cases")
        .join(hex_sha256(text(case, "id").as_bytes()));
    fs::create_dir_all(&case_dir).with_context(|| format!("create {}", case_dir.display()))?;
    result.insert(
        "artifacts".to_owned(),
        json!(format!("cases/{}", hex_sha256(text(case, "id").as_bytes()))),
    );
    fs::write(case_dir.join("source.txt"), source)?;
    result.insert("source_sha256".to_owned(), json!(hex_sha256(source)));
    let mut hit = false;
    match evaluate_case(run, case, &case_dir, source, &mut result) {
        Ok(cached) => hit = cached,
        Err(error) => {
            result.insert("status".to_owned(), json!("execution-error"));
            result.insert("error".to_owned(), json!(format!("{error:#}")));
            if let Some(failed) = error.downcast_ref::<ToolFailed>() {
                result.insert("stderr".to_owned(), json!(failed.stderr));
            }
        }
    }
    Ok((Value::Object(result), hit))
}

/// Render, validate and assert the TEI of every case that reached parser IR.
fn probe_tei(cli: &Cli, oracle_cases: &[Value], cases: &mut [Value]) -> Result<()> {
    let sources = unique_by_id(oracle_cases)?;
    let mut manifest = Vec::new();
    for case in cases.iter() {
        if case.get("parser_ir").is_none() {
            continue;
        }
        let id = text(case, "id");
        let case_dir = cli.out.join(text(case, "artifacts"));
        let matching = matching_projections(sources[id], "tei", Some(&json!(3)));
        if matching.len() > 1 {
            bail!("multiple TEI projections match {id}");
        }
        let mut entry = json!({
            "id": id,
            "parser_ir": case_dir.join("parser-ir.json"),
            "xml_out": case_dir.join("tei.xml"),
        });
        if let Some(projection) = matching.first() {
            entry["tei_projection"] = (*projection).clone();
        }
        manifest.push(entry);
    }
    if manifest.is_empty() {
        return Ok(());
    }
    let manifest_path = cli.out.join("tei-manifest.json");
    write_json(&manifest_path, &Value::Array(manifest))?;
    let probe = run_tool(
        &cli.tei_probe,
        ["--manifest".as_ref(), manifest_path.as_os_str()],
        None,
        PROBE_TIMEOUT,
    )?;
    let output: Value = serde_json::from_slice(&probe.stdout).with_context(|| {
        format!(
            "TEI probe returned invalid output; stderr={}",
            String::from_utf8_lossy(&probe.stderr)
        )
    })?;
    let results = output
        .get("results")
        .and_then(Value::as_array)
        .context("TEI probe output has no results")?;
    let validations = unique_by_id(results)?;
    for case in cases.iter_mut() {
        let Some(validation) = validations.get(text(case, "id")) else {
            continue;
        };
        let mut validation = (*validation).clone();
        let evaluation = validation
            .as_object_mut()
            .and_then(|validation| validation.remove("tei_evaluation"));
        let valid = validation["valid"] == true;
        case["tei_validation"] = validation;
        if !valid {
            case["status"] = json!("invalid-tei");
            continue;
        }
        case["tei_projection_registered"] = json!(evaluation.is_some());
        if let Some(evaluation) = evaluation {
            let matched = evaluation["assertions_match"] == true;
            case["tei_evaluation"] = evaluation;
            if !matched {
                case["status"] = json!("failed-tei-projection");
            }
        }
    }
    Ok(())
}

fn git(corpus: &Path, args: &[&str]) -> Result<std::process::Output> {
    std::process::Command::new("git")
        .arg("-C")
        .arg(corpus)
        .args(args)
        .output()
        .context("run git")
}

/// Inventory the corpus, or reuse a verified prior inventory.
fn corpus_inventory(
    cli: &Cli,
    index: &Path,
    corpus: &Path,
    matrix_bytes: &[u8],
    matrix_path: &Path,
) -> Result<Value> {
    let revision = git(corpus, &["rev-parse", "HEAD"])?;
    if !revision.status.success() {
        bail!("corpus revision unavailable");
    }
    let revision = String::from_utf8(revision.stdout)?.trim().to_owned();
    if !git(corpus, &["diff", "--quiet", "HEAD"])?.status.success() {
        bail!("corpus checkout has local changes");
    }
    let index_sha256 = hex_sha256(&read(index)?);
    let inventory_path = cli.out.join("corpus-inventory.json");
    if let Some(prior_path) = &cli.reuse_corpus_report {
        let prior_report = read_json(prior_path)?;
        let prior_dir = prior_path
            .parent()
            .context("prior report has no directory")?;
        let prior_matrix = prior_report
            .pointer("/inputs/matrix_sha256")
            .and_then(Value::as_str)
            .map(|digest| prior_dir.join("inputs").join(format!("{digest}.toml")))
            .and_then(|path| fs::read(path).ok());
        let prior_inventory = read(&prior_dir.join("corpus-inventory.json"))?;
        let corpus = reuse_corpus_inventory(&ReuseInputs {
            prior_report: &prior_report,
            prior_matrix: prior_matrix.as_deref(),
            prior_inventory: &prior_inventory,
            matrix: matrix_bytes,
            allowlist_sha256: &hex_sha256(&read(&cli.allowlist)?),
            inventory_tool_sha256: &hex_sha256(&read(&cli.inventory)?),
            revision: &revision,
            index_sha256: &index_sha256,
            variant_rows: &cli.variant_rows,
        })?;
        fs::write(&inventory_path, prior_inventory)?;
        return Ok(corpus);
    }
    let mut command = std::process::Command::new(&cli.inventory);
    command
        .arg("--index")
        .arg(index)
        .arg("--corpus")
        .arg(corpus)
        .arg("--matrix")
        .arg(matrix_path)
        .arg("--allowlist")
        .arg(&cli.allowlist)
        .args(["--jobs", "6"])
        .arg("--output-json")
        .arg(&inventory_path)
        .arg("--report-md")
        .arg(cli.out.join("corpus-inventory.md"))
        .arg("--edition-results")
        .arg(cli.out.join("editions.jsonl"))
        .arg("--unknown-workset")
        .arg(cli.out.join("unknown-workset.json"));
    for row in &cli.variant_rows {
        command.arg("--variant-row").arg(row);
    }
    if let Some(cache) = &cli.inventory_cache {
        command.arg("--cache-dir").arg(cache);
    }
    if !command.status().context("run corpus inventory")?.success() {
        bail!("corpus inventory failed");
    }
    let inventory_bytes = read(&inventory_path)?;
    corpus_inventory_record(
        &serde_json::from_slice(&inventory_bytes).context("parse corpus inventory")?,
        &hex_sha256(&inventory_bytes),
        &revision,
        &index_sha256,
    )
}

fn main() -> Result<()> {
    let mut cli = Cli::parse();
    // The TEI probe resolves manifest paths from its own working directory.
    cli.out = std::path::absolute(&cli.out).context("resolve --out")?;
    let matrix_bytes = read(&cli.matrix)?;
    let oracle_bytes = read(&cli.oracle)?;
    let requirements_bytes = read(&cli.requirements)?;
    let matrix = toml_value(&matrix_bytes).context("matrix")?;
    let oracle = toml_value(&oracle_bytes).context("oracle")?;
    let requirements = toml_value(&requirements_bytes).context("requirements")?;
    validate_links(&matrix, &oracle)?;
    validate_requirements(&matrix, &oracle, &requirements)?;
    let matrix_sha256 = hex_sha256(&matrix_bytes);
    let matrix_path = cli.out.join("inputs").join(format!("{matrix_sha256}.toml"));
    fs::create_dir_all(cli.out.join("inputs"))?;
    fs::write(&matrix_path, &matrix_bytes)?;
    fs::write(
        cli.out
            .join("inputs")
            .join(format!("{}.toml", hex_sha256(&oracle_bytes))),
        &oracle_bytes,
    )?;
    let run = Run {
        cli: &cli,
        parser_sha256: hex_sha256(&read(&cli.parser)?),
        matrix_path,
        matrix_sha256,
    };
    let oracle_cases = oracle["case"].as_array().context("oracle has no cases")?;
    let evaluated = oracle_cases
        .par_iter()
        .map(|case| case_result(&run, case))
        .collect::<Result<Vec<_>>>()?;
    let reused = evaluated.iter().filter(|(_, hit)| *hit).count();
    let mut cases = evaluated
        .into_iter()
        .map(|(case, _)| case)
        .collect::<Vec<_>>();
    probe_tei(&cli, oracle_cases, &mut cases)?;

    let corpus = match (&cli.index, &cli.corpus) {
        (Some(index), Some(corpus)) => Some(corpus_inventory(
            &cli,
            index,
            corpus,
            &matrix_bytes,
            &run.matrix_path,
        )?),
        _ => None,
    };
    let (variant_coverage, semantic_partition_coverage) = match &corpus {
        Some(corpus) => (
            evaluate_variant_coverage(&cases, corpus),
            evaluate_semantic_partition_coverage(&cases, corpus),
        ),
        None => (json!({}), json!({})),
    };
    let requirement_results = evaluate_requirements(&requirements, &cases);
    let Assembled {
        rows,
        worklist,
        mut summary,
    } = assemble(
        &matrix,
        &cases,
        &requirement_results,
        corpus.as_ref(),
        &variant_coverage,
        &semantic_partition_coverage,
    )?;
    summary.insert("parser_cache_hits".to_owned(), json!(reused));
    let digest_of = |path: &Path| read(path).map(|bytes| hex_sha256(&bytes));
    let runner = std::env::current_exe().context("locate runner")?;
    let report = json!({
        "schema": REPORT_SCHEMA,
        "scope": "registered-syntax-families-and-oracle-fixtures",
        "semantic_certification": "not-assessed",
        "corpus_coverage": "not-assessed",
        "corpus_inventory": corpus,
        "variant_coverage": variant_coverage,
        "semantic_partition_coverage": semantic_partition_coverage,
        "inputs": {
            "matrix_sha256": run.matrix_sha256,
            "oracle_sha256": hex_sha256(&oracle_bytes),
            "requirements_sha256": hex_sha256(&requirements_bytes),
            "requirements_schema_sha256": digest_of(&cli.requirements_schema)?,
            "runner_sha256": digest_of(&runner)?,
            "parser_sha256": run.parser_sha256,
            "checker_sha256": digest_of(&cli.checker)?,
            "inventory_sha256": digest_of(&cli.inventory)?,
            "converter_sha256": digest_of(&cli.converter)?,
            "tei_probe_sha256": digest_of(&cli.tei_probe)?,
            "mapping_sha256": digest_of(&cli.mapping)?,
            "allowlist_sha256": digest_of(&cli.allowlist)?,
        },
        "summary": summary,
        "evidence": oracle["evidence"],
        "cases": cases,
        "rows": rows,
        "worklist": worklist,
    });
    write_json(&cli.out.join("report.json"), &report)?;
    println!("{}", serde_json::to_string_pretty(&report["summary"])?);
    Ok(())
}
