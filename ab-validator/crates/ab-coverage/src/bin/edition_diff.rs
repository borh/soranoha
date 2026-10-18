//! Compare every indexed edition's AAT under a baseline and a candidate parser,
//! attribute each changed node to the source-inventory forms it touches, and
//! carry changed editions through parser-IR, TEI and plaintext.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use ab_coverage::{
    CoverageMatrix,
    edition_diff::{
        ChangedNode, Side, Targets, attribute, changed_nodes, content_sha256, region_occurrences,
    },
    occurrence_alignment::located_occurrences,
    parser_exec::{run_parser, write_atomically},
    source_corpus::{SourceIndexEntry, load_index_entries, read_source_work},
    source_inventory::{
        SourceInventoryPattern, inventory_document_with_all_variant_occurrences, patterns_from_rows,
    },
};
use ab_encoding::hex_sha256;
use anyhow::{Context, Result, bail};
use clap::Parser;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const SCHEMA_VERSION: &str = "soranoha-edition-diff/1";
const HASH_CACHE_SCHEMA_VERSION: &str = "soranoha-edition-aat-hashes/1";

#[derive(Parser)]
struct Cli {
    #[arg(long)]
    index: PathBuf,
    #[arg(long)]
    corpus: PathBuf,
    #[arg(long, default_value = "data/aozora-syntax-coverage.toml")]
    matrix: PathBuf,
    /// Parser executables accepting `--mode aat` with source bytes on stdin.
    #[arg(long)]
    baseline_parser: PathBuf,
    #[arg(long)]
    candidate_parser: PathBuf,
    /// Holds one AAT hash table per parser executable.
    #[arg(long)]
    cache_dir: PathBuf,
    /// Syntax row whose occurrences may change; repeatable.
    #[arg(long = "target-row")]
    target_rows: Vec<String>,
    /// `ROW=FORM` pair whose occurrences may change; repeatable.
    #[arg(long = "target-form")]
    target_forms: Vec<String>,
    /// `ab-aat-to-parser-ir` executable; enables the parser-IR comparison.
    #[arg(long, requires_all = ["mapping", "artifacts_dir"])]
    converter: Option<PathBuf>,
    #[arg(long)]
    mapping: Option<PathBuf>,
    /// Converter and mapping from the baseline revision, whose AAT the
    /// candidate converter may reject; default to the candidate's.
    #[arg(long, requires = "converter")]
    baseline_converter: Option<PathBuf>,
    #[arg(long, requires = "converter")]
    baseline_mapping: Option<PathBuf>,
    /// Fidelity probe executable; enables the TEI and plaintext comparison.
    #[arg(long, requires = "converter")]
    tei_probe: Option<PathBuf>,
    /// Receives both sides' outputs for every changed edition.
    #[arg(long)]
    artifacts_dir: Option<PathBuf>,
    /// Unexplained changed nodes listed per edition.
    #[arg(long, default_value_t = 5)]
    samples: usize,
    #[arg(long)]
    jobs: Option<usize>,
    #[arg(long)]
    output_json: PathBuf,
}

#[derive(Default, Deserialize, Serialize)]
struct HashTable {
    schema_version: String,
    parser_sha256: String,
    /// Source SHA-256 to the build-independent AAT hash, or to
    /// `parse-failed` when the parser rejected the source.
    aat_sha256_by_source: BTreeMap<String, String>,
}

impl HashTable {
    fn path(cache_dir: &Path, parser_sha256: &str) -> PathBuf {
        cache_dir.join(format!("{parser_sha256}.json"))
    }

    fn load(cache_dir: &Path, parser_sha256: &str) -> Result<Self> {
        let path = Self::path(cache_dir, parser_sha256);
        let Ok(bytes) = fs::read(&path) else {
            return Ok(Self {
                schema_version: HASH_CACHE_SCHEMA_VERSION.to_owned(),
                parser_sha256: parser_sha256.to_owned(),
                aat_sha256_by_source: BTreeMap::new(),
            });
        };
        let table: Self =
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
        if table.schema_version != HASH_CACHE_SCHEMA_VERSION || table.parser_sha256 != parser_sha256
        {
            bail!("hash table identity mismatch in {}", path.display());
        }
        Ok(table)
    }

    fn store(&self, cache_dir: &Path) -> Result<()> {
        write_atomically(
            &Self::path(cache_dir, &self.parser_sha256),
            &serde_json::to_vec(self)?,
        )
    }
}

struct ParserSide<'a> {
    executable: &'a Path,
    hashes: &'a HashTable,
}

const PARSE_FAILED: &str = "parse-failed";

/// The AAT and its build-independent hash, or the parser's error.
fn parse(executable: &Path, text: &str) -> (String, Result<Value>) {
    match run_parser(executable, text.as_bytes()) {
        Ok(aat) => (content_sha256(&aat), Ok(aat)),
        Err(error) => (PARSE_FAILED.to_owned(), Err(error)),
    }
}

#[derive(Serialize)]
struct FormChange {
    row: String,
    form: String,
    target: bool,
    changed_nodes: u64,
}

#[derive(Serialize)]
struct ChangedEdition {
    work_id: String,
    indexed_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    artifacts: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parse_failure: Option<BTreeMap<Side, String>>,
    changed_nodes: BTreeMap<Side, u64>,
    forms: Vec<FormChange>,
    unexplained_nodes: usize,
    unexplained_samples: Vec<ChangedNode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parser_ir_changed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tei: Option<Value>,
}

impl ChangedEdition {
    fn explained(&self) -> bool {
        self.parse_failure.is_none() && self.unexplained_nodes == 0
    }
}

/// One side's AAT hash for a source, and whether the parser ran to produce it.
struct SideHash {
    aat_sha256: String,
    parsed: bool,
}

enum Outcome {
    Unchanged {
        source_sha256: String,
        hashes: [SideHash; 2],
    },
    Changed {
        source_sha256: String,
        hashes: [SideHash; 2],
        edition: Box<ChangedEdition>,
    },
    Failed {
        work_id: String,
        indexed_path: String,
        error: String,
    },
}

struct Run<'a> {
    cli: &'a Cli,
    patterns: &'a [SourceInventoryPattern],
    row_ids: &'a [String],
    targets: &'a Targets,
    sides: [ParserSide<'a>; 2],
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(jobs) = cli.jobs {
        rayon::ThreadPoolBuilder::new()
            .num_threads(jobs)
            .build_global()
            .context("rayon thread pool")?;
    }
    let matrix_bytes =
        fs::read(&cli.matrix).with_context(|| format!("read matrix {}", cli.matrix.display()))?;
    let matrix = CoverageMatrix::parse(std::str::from_utf8(&matrix_bytes)?)
        .with_context(|| format!("load matrix {}", cli.matrix.display()))?;
    let row_ids = matrix
        .rows()
        .iter()
        .map(|row| row.id.clone())
        .collect::<Vec<_>>();
    let targets = targets(&cli, &row_ids)?;
    let patterns = patterns_from_rows(matrix.rows());
    let parser_sha256 = |path: &Path| -> Result<String> {
        Ok(hex_sha256(&fs::read(path).with_context(|| {
            format!("read parser {}", path.display())
        })?))
    };
    let baseline_sha256 = parser_sha256(&cli.baseline_parser)?;
    let candidate_sha256 = parser_sha256(&cli.candidate_parser)?;
    let mut tables = [
        HashTable::load(&cli.cache_dir, &baseline_sha256)?,
        HashTable::load(&cli.cache_dir, &candidate_sha256)?,
    ];
    let index_bytes =
        fs::read(&cli.index).with_context(|| format!("read index {}", cli.index.display()))?;
    let entries = load_index_entries(&cli.index)?;

    let mut unchanged = 0_u64;
    let mut parses = [0_u64; 2];
    let mut changed = Vec::new();
    let mut failures = Vec::new();
    for chunk in entries.chunks(256) {
        let run = Run {
            cli: &cli,
            patterns: &patterns,
            row_ids: &row_ids,
            targets: &targets,
            sides: [
                ParserSide {
                    executable: &cli.baseline_parser,
                    hashes: &tables[0],
                },
                ParserSide {
                    executable: &cli.candidate_parser,
                    hashes: &tables[1],
                },
            ],
        };
        let outcomes = chunk
            .par_iter()
            .map(|entry| compare_edition(entry, &run))
            .collect::<Vec<_>>();
        for outcome in outcomes {
            let (source_sha256, hashes) = match outcome {
                Outcome::Failed {
                    work_id,
                    indexed_path,
                    error,
                } => {
                    failures.push(
                        json!({"work_id": work_id, "indexed_path": indexed_path, "error": error}),
                    );
                    continue;
                }
                Outcome::Unchanged {
                    source_sha256,
                    hashes,
                } => {
                    unchanged += 1;
                    (source_sha256, hashes)
                }
                Outcome::Changed {
                    source_sha256,
                    hashes,
                    edition,
                } => {
                    changed.push(*edition);
                    (source_sha256, hashes)
                }
            };
            for ((table, count), hash) in tables.iter_mut().zip(&mut parses).zip(hashes) {
                if hash.parsed {
                    *count += 1;
                    table
                        .aat_sha256_by_source
                        .insert(source_sha256.clone(), hash.aat_sha256);
                }
            }
        }
    }
    for table in &tables {
        table.store(&cli.cache_dir)?;
    }
    if let Some(probe) = &cli.tei_probe {
        compare_tei(probe, &cli, &mut changed)?;
    }

    let mut forms = BTreeMap::<String, BTreeMap<String, Value>>::new();
    for edition in &changed {
        for form in &edition.forms {
            let entry = forms
                .entry(form.row.clone())
                .or_default()
                .entry(form.form.clone())
                .or_insert_with(
                    || json!({"target": form.target, "editions": 0, "changed_nodes": 0}),
                );
            entry["editions"] = json!(entry["editions"].as_u64().unwrap_or(0) + 1);
            entry["changed_nodes"] =
                json!(entry["changed_nodes"].as_u64().unwrap_or(0) + form.changed_nodes);
        }
    }
    let unexplained_editions = changed
        .iter()
        .filter(|edition| !edition.explained())
        .count();
    let invalid_tei = changed
        .iter()
        .filter(|edition| {
            edition
                .tei
                .as_ref()
                .is_some_and(|tei| tei["candidate_valid"] != true)
        })
        .count();
    let passed = unexplained_editions == 0 && invalid_tei == 0 && failures.is_empty();
    let report = json!({
        "schema_version": SCHEMA_VERSION,
        "inputs": {
            "index_sha256": hex_sha256(&index_bytes),
            "matrix_sha256": hex_sha256(&matrix_bytes),
            "baseline_parser_sha256": baseline_sha256,
            "candidate_parser_sha256": candidate_sha256,
            "converter_sha256": file_sha256(cli.converter.as_ref())?,
            "baseline_converter_sha256": file_sha256(cli.baseline_converter.as_ref().or(cli.converter.as_ref()))?,
            "target_rows": targets.rows,
            "target_forms": targets.forms.iter().map(|(row, form)| json!({"row": row, "form": form})).collect::<Vec<_>>(),
        },
        "editions": {
            "indexed": entries.len(),
            "unchanged": unchanged,
            "changed": changed.len(),
            "unexplained": unexplained_editions,
            "invalid_candidate_tei": invalid_tei,
            "parses": {"baseline": parses[0], "candidate": parses[1]},
            "failures": failures,
        },
        "passed": passed,
        "forms": forms,
        "changed": changed,
    });
    fs::write(&cli.output_json, serde_json::to_vec_pretty(&report)?)
        .with_context(|| format!("write {}", cli.output_json.display()))?;
    if !passed {
        bail!(
            "{unexplained_editions} editions changed outside the target forms, {invalid_tei} candidate TEI documents are invalid, {} editions could not be read; see {}",
            report["editions"]["failures"]
                .as_array()
                .map_or(0, Vec::len),
            cli.output_json.display()
        );
    }
    Ok(())
}

fn file_sha256(path: Option<&PathBuf>) -> Result<Option<String>> {
    path.map(|path| {
        fs::read(path)
            .map(|bytes| hex_sha256(&bytes))
            .with_context(|| format!("read {}", path.display()))
    })
    .transpose()
}

fn targets(cli: &Cli, row_ids: &[String]) -> Result<Targets> {
    let mut targets = Targets::default();
    for row in &cli.target_rows {
        if !row_ids.contains(row) {
            bail!("unknown --target-row {row:?}");
        }
        targets.rows.insert(row.clone());
    }
    for pair in &cli.target_forms {
        let Some((row, form)) = pair.split_once('=') else {
            bail!("--target-form {pair:?} is not ROW=FORM");
        };
        if !row_ids.iter().any(|id| id == row) {
            bail!("unknown row in --target-form {pair:?}");
        }
        targets.forms.insert((row.to_owned(), form.to_owned()));
    }
    Ok(targets)
}

fn compare_edition(entry: &SourceIndexEntry, run: &Run<'_>) -> Outcome {
    let work = match read_source_work(&run.cli.corpus, &entry.work_id, &entry.indexed_path) {
        Ok(work) => work,
        Err(error) => {
            return Outcome::Failed {
                work_id: entry.work_id.clone(),
                indexed_path: entry.indexed_path.clone(),
                error: format!("{error:#}"),
            };
        }
    };
    // Both parsers receive the transcribed text, so each side applies the
    // source emendation table of its own revision.
    let text = match ab_encoding::decode_text(&work.bytes) {
        Ok((text, _)) => text,
        Err(error) => {
            return Outcome::Failed {
                work_id: entry.work_id.clone(),
                indexed_path: entry.indexed_path.clone(),
                error: format!("{error:#}"),
            };
        }
    };
    let text = &text;
    let source_sha256 = hex_sha256(text.as_bytes());
    let mut documents = [None, None];
    let hashes = [0, 1].map(|index| {
        let side = &run.sides[index];
        if let Some(cached) = side.hashes.aat_sha256_by_source.get(&source_sha256) {
            SideHash {
                aat_sha256: cached.clone(),
                parsed: false,
            }
        } else {
            let (aat_sha256, aat) = parse(side.executable, text);
            documents[index] = Some(aat);
            SideHash {
                aat_sha256,
                parsed: true,
            }
        }
    });
    if hashes[0].aat_sha256 == hashes[1].aat_sha256 {
        return Outcome::Unchanged {
            source_sha256,
            hashes,
        };
    }
    let [baseline, candidate] = documents;
    let baseline = baseline.unwrap_or_else(|| parse(run.sides[0].executable, text).1);
    let candidate = candidate.unwrap_or_else(|| parse(run.sides[1].executable, text).1);
    let edition = describe_change(entry, run, text, &source_sha256, baseline, candidate);
    Outcome::Changed {
        source_sha256,
        hashes,
        edition: Box::new(edition),
    }
}

fn describe_change(
    entry: &SourceIndexEntry,
    run: &Run<'_>,
    text: &str,
    source_sha256: &str,
    baseline: Result<Value>,
    candidate: Result<Value>,
) -> ChangedEdition {
    let mut edition = ChangedEdition {
        work_id: entry.work_id.clone(),
        indexed_path: entry.indexed_path.clone(),
        artifacts: None,
        parse_failure: None,
        changed_nodes: BTreeMap::new(),
        forms: Vec::new(),
        unexplained_nodes: 0,
        unexplained_samples: Vec::new(),
        parser_ir_changed: None,
        tei: None,
    };
    let (baseline, candidate) = match (baseline, candidate) {
        (Ok(baseline), Ok(candidate)) => (baseline, candidate),
        (baseline, candidate) => {
            edition.parse_failure = Some(
                [(Side::Baseline, baseline), (Side::Candidate, candidate)]
                    .into_iter()
                    .filter_map(|(side, result)| {
                        result.err().map(|error| (side, format!("{error:#}")))
                    })
                    .collect(),
            );
            return edition;
        }
    };
    let changed = changed_nodes(&baseline, &candidate);
    for node in &changed {
        *edition.changed_nodes.entry(node.side).or_default() += 1;
    }
    let summary = inventory_document_with_all_variant_occurrences(
        &entry.work_id,
        text,
        run.patterns,
        run.row_ids,
    );
    let mut occurrences = located_occurrences(&summary, run.row_ids);
    occurrences.extend(region_occurrences(&summary, run.patterns));
    let attribution = attribute(changed, &occurrences, run.targets);
    edition.forms = attribution
        .forms
        .into_iter()
        .map(|((row, form), changed_nodes)| FormChange {
            target: run.targets.rows.contains(&row)
                || run.targets.forms.contains(&(row.clone(), form.clone())),
            row,
            form,
            changed_nodes,
        })
        .collect();
    edition.unexplained_nodes = attribution.unexplained.len();
    edition.unexplained_samples = attribution
        .unexplained
        .into_iter()
        .take(run.cli.samples)
        .collect();
    if let Some(artifacts_dir) = &run.cli.artifacts_dir {
        let relative = hex_sha256(entry.indexed_path.as_bytes())[..16].to_owned();
        let dir = artifacts_dir.join(&relative);
        edition.artifacts = Some(relative);
        match write_outputs(run.cli, &dir, source_sha256, &baseline, &candidate) {
            Ok(parser_ir_changed) => edition.parser_ir_changed = parser_ir_changed,
            Err(error) => {
                edition.tei =
                    Some(json!({"error": format!("{error:#}"), "candidate_valid": false}));
            }
        }
    }
    edition
}

/// Write both AAT documents and, with a converter, both parser-IR documents;
/// returns whether the parser-IR differs.
fn write_outputs(
    cli: &Cli,
    dir: &Path,
    source_sha256: &str,
    baseline: &Value,
    candidate: &Value,
) -> Result<Option<bool>> {
    let mut parser_ir = Vec::new();
    for (name, aat) in [("baseline", baseline), ("candidate", candidate)] {
        let aat_path = dir.join(format!("{name}.aat.json"));
        write_atomically(&aat_path, &serde_json::to_vec(aat)?)?;
        let (Some(candidate_converter), Some(candidate_mapping)) = (&cli.converter, &cli.mapping)
        else {
            continue;
        };
        let (converter, mapping) = if name == "baseline" {
            (
                cli.baseline_converter
                    .as_ref()
                    .unwrap_or(candidate_converter),
                cli.baseline_mapping.as_ref().unwrap_or(candidate_mapping),
            )
        } else {
            (candidate_converter, candidate_mapping)
        };
        let parser_ir_path = dir.join(format!("{name}.parser-ir.json"));
        let output = Command::new(converter)
            .arg("convert")
            .arg("--aat")
            .arg(&aat_path)
            .arg("--mapping")
            .arg(mapping)
            .arg("--work-content-hash")
            .arg(format!("sha256:{source_sha256}"))
            .arg("--parser-ir-out")
            .arg(&parser_ir_path)
            .arg("--divergence-out")
            .arg(dir.join(format!("{name}.divergence.json")))
            .output()
            .with_context(|| format!("spawn {}", converter.display()))?;
        if !output.status.success() {
            bail!(
                "{name} parser-IR conversion exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let bytes = fs::read(&parser_ir_path)
            .with_context(|| format!("read {}", parser_ir_path.display()))?;
        parser_ir.push(serde_json::from_slice::<Value>(&bytes)?);
    }
    Ok((parser_ir.len() == 2).then(|| parser_ir[0] != parser_ir[1]))
}

fn compare_tei(probe: &Path, cli: &Cli, changed: &mut [ChangedEdition]) -> Result<()> {
    let artifacts_dir = cli
        .artifacts_dir
        .as_ref()
        .context("--tei-probe needs --artifacts-dir")?;
    let manifest = changed
        .iter()
        .filter(|edition| edition.parser_ir_changed.is_some())
        .filter_map(|edition| edition.artifacts.as_ref())
        .map(|relative| {
            let dir = artifacts_dir.join(relative);
            json!({
                "id": relative,
                "baseline_parser_ir": dir.join("baseline.parser-ir.json"),
                "candidate_parser_ir": dir.join("candidate.parser-ir.json"),
                "out_dir": dir,
            })
        })
        .collect::<Vec<_>>();
    if manifest.is_empty() {
        return Ok(());
    }
    let manifest_path = artifacts_dir.join("tei-diff-manifest.json");
    write_atomically(&manifest_path, &serde_json::to_vec_pretty(&manifest)?)?;
    let output = Command::new(probe)
        .arg("--diff-manifest")
        .arg(&manifest_path)
        .output()
        .with_context(|| format!("spawn {}", probe.display()))?;
    let results: Value = serde_json::from_slice(&output.stdout).with_context(|| {
        format!(
            "TEI probe exited with {} and wrote no JSON: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )
    })?;
    let mut by_id = results["results"]
        .as_array()
        .context("TEI probe results")?
        .iter()
        .map(|result| {
            (
                result["id"].as_str().unwrap_or_default().to_owned(),
                result.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let expected = manifest
        .iter()
        .filter_map(|entry| entry["id"].as_str())
        .collect::<BTreeSet<_>>();
    for edition in changed {
        let Some(relative) = &edition.artifacts else {
            continue;
        };
        if expected.contains(relative.as_str()) {
            edition.tei = Some(by_id.remove(relative).unwrap_or_else(
                || json!({"error": "TEI probe returned no result", "candidate_valid": false}),
            ));
        }
    }
    Ok(())
}
