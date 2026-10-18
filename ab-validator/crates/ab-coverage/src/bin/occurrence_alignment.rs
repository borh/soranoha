//! Align every selected-row source-inventory occurrence in a corpus with the
//! AAT emitted by a parser executable, and classify each normalized form.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use ab_coverage::{
    CoverageMatrix,
    corpus_check::{
        BaselineComparison, CheckOutcome, Discrimination, ObligationCheck, obligation_checks,
    },
    occurrence_alignment::{
        AlignmentNode, AlignmentStatus, FormClass, LocatedOccurrence, alignment_index,
        alignment_status, form_class, located_occurrences, retains_uncertain_source,
    },
    parser_exec::{run_parser, write_atomically},
    residue::{
        Coverage, LedgerOccurrence, Locator, inventory_mismatches, load_decisions, ratchet,
        row_ledger, row_qualification, unmatched_rulings,
    },
    source_corpus::{SourceIndexEntry, load_index_entries, read_source_work},
    source_inventory::{inventory_document_with_all_variant_occurrences, patterns_from_rows},
};
use ab_encoding::hex_sha256;
use anyhow::{Context, Result, bail};
use clap::Parser;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const SCHEMA_VERSION: &str = "soranoha-occurrence-alignment/1";
const CACHE_SCHEMA_VERSION: &str = "soranoha-occurrence-alignment-cache/7";
const CHECK_CACHE_SCHEMA_VERSION: &str = "soranoha-corpus-check-cache/1";

#[derive(Parser)]
struct Cli {
    #[arg(long)]
    index: PathBuf,
    #[arg(long)]
    corpus: PathBuf,
    #[arg(long, default_value = "data/aozora-syntax-coverage.toml")]
    matrix: PathBuf,
    /// Syntax row to align; repeatable.
    #[arg(long = "row", required = true)]
    rows: Vec<String>,
    /// Parser executable accepting `--mode aat` with source bytes on stdin.
    #[arg(long)]
    parser: PathBuf,
    /// Parser whose corpus check outcomes are compared with `--parser`'s.
    #[arg(long, requires = "requirements")]
    baseline_parser: Option<PathBuf>,
    /// Obligation whose check must change outcome at some occurrence between
    /// the baseline and candidate parser; repeatable.
    #[arg(long = "expect-flip", requires = "baseline_parser")]
    expect_flips: Vec<String>,
    #[arg(long)]
    cache_dir: PathBuf,
    /// Fidelity report whose passing cases define fixture-covered forms and
    /// covered semantic partitions.
    #[arg(long)]
    fidelity_report: Option<PathBuf>,
    /// Corpus inventory report whose per-form and per-partition occurrence
    /// counts must equal the aligned totals.
    #[arg(long)]
    inventory_json: Option<PathBuf>,
    /// Fidelity requirements whose corpus checks are evaluated at every
    /// occurrence of their partitions.
    #[arg(long)]
    requirements: Option<PathBuf>,
    /// Reviewer rulings that forms are undecidable.
    #[arg(long)]
    decisions: Option<PathBuf>,
    /// Previous alignment report; the run fails when a row's residue grows.
    #[arg(long)]
    ratchet: Option<PathBuf>,
    #[arg(long, default_value_t = 3)]
    samples: usize,
    #[arg(long)]
    jobs: Option<usize>,
    #[arg(long)]
    output_json: PathBuf,
}

#[derive(Deserialize, Serialize)]
struct CacheEntry {
    identity: CacheIdentity,
    nodes: Vec<AlignmentNode>,
}

/// Check outcomes keyed by obligation, marker start and raw marker.
#[derive(Deserialize, Serialize)]
struct CheckCacheEntry {
    identity: CheckCacheIdentity,
    outcomes: Vec<(String, usize, String, CheckOutcome)>,
}

#[derive(Deserialize, Serialize, PartialEq, Eq)]
struct CheckCacheIdentity {
    schema_version: String,
    parser_sha256: String,
    source_sha256: String,
    requirements_sha256: String,
    checker_sha256: String,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
struct CacheIdentity {
    schema_version: String,
    parser_sha256: String,
    source_sha256: String,
}

#[derive(Clone, Serialize)]
struct Sample {
    work_id: String,
    indexed_path: String,
    line: usize,
    byte_start: usize,
    raw: String,
    status: AlignmentStatus,
}

#[derive(Default, Serialize)]
struct StatusCounts {
    occurrences: u64,
    statuses: BTreeMap<AlignmentStatus, u64>,
}

impl StatusCounts {
    fn add(&mut self, status: AlignmentStatus) {
        self.occurrences += 1;
        *self.statuses.entry(status).or_default() += 1;
    }
}

#[derive(Default, Serialize)]
struct FormReport {
    #[serde(flatten)]
    counts: StatusCounts,
    partitions: BTreeMap<String, u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    class: Option<FormClass>,
    samples: Vec<Sample>,
}

#[derive(Default, Serialize)]
struct RowReport {
    #[serde(flatten)]
    counts: StatusCounts,
    occurrences_without_form: u64,
    forms: BTreeMap<String, FormReport>,
    partitions: BTreeMap<String, StatusCounts>,
    classes: BTreeMap<String, ClassTotals>,
}

#[derive(Default, Serialize)]
struct ClassTotals {
    forms: u64,
    occurrences: u64,
}

#[derive(Default, Serialize)]
struct EditionTotals {
    indexed: u64,
    with_occurrences: u64,
    parse_cache_hits: u64,
    parses: u64,
    failures: Vec<EditionFailure>,
}

#[derive(Serialize)]
struct EditionFailure {
    work_id: String,
    indexed_path: String,
    stage: &'static str,
    error: String,
}

struct AlignedOccurrence {
    located: LocatedOccurrence,
    indexed_path: String,
    status: AlignmentStatus,
    uncertain_source_retained: bool,
    /// Candidate and baseline outcome of each applicable corpus check, by
    /// index into the checks.
    checks: Vec<(usize, CheckOutcome, Option<CheckOutcome>)>,
}

#[derive(Serialize)]
struct CheckSample {
    work_id: String,
    indexed_path: String,
    line: usize,
    byte_start: usize,
    raw: String,
    #[serde(flatten)]
    outcome: CheckOutcome,
}

#[derive(Serialize)]
struct ObligationReport {
    row: String,
    partitions: Vec<String>,
    occurrences: u64,
    satisfied: u64,
    /// Occurrences outside the operands or quoted inside another marker.
    unchecked: u64,
    failed: u64,
    /// Every in-operand occurrence satisfies the check and at least one does.
    credited: bool,
    outcomes: BTreeMap<String, u64>,
    samples: BTreeMap<String, Vec<CheckSample>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    discrimination: Option<DiscriminationReport>,
}

#[derive(Default, Serialize)]
struct DiscriminationReport {
    #[serde(flatten)]
    counts: Discrimination,
    /// Gained and lost occurrences.
    samples: BTreeMap<BaselineComparison, Vec<FlipSample>>,
}

#[derive(Serialize)]
struct FlipSample {
    work_id: String,
    indexed_path: String,
    line: usize,
    byte_start: usize,
    raw: String,
    baseline: CheckOutcome,
    candidate: CheckOutcome,
}

#[derive(Default, Serialize)]
struct PartitionCredit {
    fixture: bool,
    /// Satisfied occurrences of each credited obligation's check.
    predicates: BTreeMap<String, u64>,
}

enum EditionOutcome {
    Aligned {
        occurrences: Vec<AlignedOccurrence>,
        parsed: Option<bool>,
    },
    Failed(EditionFailure),
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
    for row_id in &cli.rows {
        if !matrix.rows().iter().any(|row| &row.id == row_id) {
            bail!("unknown --row {row_id:?}");
        }
    }
    let patterns = patterns_from_rows(matrix.rows());
    let parser_bytes =
        fs::read(&cli.parser).with_context(|| format!("read parser {}", cli.parser.display()))?;
    let parser_sha256 = hex_sha256(&parser_bytes);
    let baseline = cli
        .baseline_parser
        .as_ref()
        .map(|path| {
            fs::read(path)
                .map(|bytes| (path.as_path(), hex_sha256(&bytes)))
                .with_context(|| format!("read baseline parser {}", path.display()))
        })
        .transpose()?;
    let index_bytes =
        fs::read(&cli.index).with_context(|| format!("read index {}", cli.index.display()))?;
    let entries = load_index_entries(&cli.index)?;
    let (checks, check_identity) = match &cli.requirements {
        Some(path) => {
            let bytes =
                fs::read(path).with_context(|| format!("read requirements {}", path.display()))?;
            let checks = obligation_checks(std::str::from_utf8(&bytes)?)?
                .into_iter()
                .filter(|check| cli.rows.contains(&check.row_id))
                .collect::<Vec<_>>();
            let checker = std::env::current_exe().context("locate checker executable")?;
            let checker = fs::read(&checker)
                .with_context(|| format!("read checker {}", checker.display()))?;
            (checks, Some((hex_sha256(&bytes), hex_sha256(&checker))))
        }
        None => (Vec::new(), None),
    };
    for obligation_id in &cli.expect_flips {
        if !checks
            .iter()
            .any(|check| &check.obligation_id == obligation_id)
        {
            bail!("--expect-flip {obligation_id:?} has no corpus check in the selected rows");
        }
    }
    let mut obligations = checks
        .iter()
        .map(|check| ObligationReport {
            row: check.row_id.clone(),
            partitions: check.check.check.partitions.clone(),
            occurrences: 0,
            satisfied: 0,
            unchecked: 0,
            failed: 0,
            credited: false,
            outcomes: BTreeMap::new(),
            samples: BTreeMap::new(),
            discrimination: baseline.as_ref().map(|_| DiscriminationReport::default()),
        })
        .collect::<Vec<_>>();

    let mut rows = cli
        .rows
        .iter()
        .map(|row_id| (row_id.clone(), RowReport::default()))
        .collect::<BTreeMap<_, _>>();
    let mut editions = EditionTotals {
        indexed: entries.len() as u64,
        ..EditionTotals::default()
    };
    let mut form_sample_pool = BTreeMap::<(String, String), Vec<Sample>>::new();
    let mut ledger_occurrences = BTreeMap::<String, Vec<LedgerOccurrence>>::new();
    for chunk in entries.chunks(256) {
        let outcomes = chunk
            .par_iter()
            .map(|entry| {
                align_edition(
                    entry,
                    &cli,
                    &patterns,
                    &parser_sha256,
                    baseline
                        .as_ref()
                        .map(|(path, sha256)| (*path, sha256.as_str())),
                    &checks,
                    check_identity.as_ref(),
                )
            })
            .collect::<Vec<_>>();
        for outcome in outcomes {
            match outcome {
                EditionOutcome::Failed(failure) => editions.failures.push(failure),
                EditionOutcome::Aligned {
                    occurrences,
                    parsed,
                } => {
                    if occurrences.is_empty() {
                        continue;
                    }
                    editions.with_occurrences += 1;
                    match parsed {
                        Some(true) => editions.parses += 1,
                        Some(false) => editions.parse_cache_hits += 1,
                        None => {}
                    }
                    for mut aligned in occurrences {
                        ledger_occurrences
                            .entry(aligned.located.row_id.clone())
                            .or_default()
                            .push(ledger_occurrence(&aligned, &checks));
                        for (index, outcome, baseline) in std::mem::take(&mut aligned.checks) {
                            record_check(
                                &mut obligations[index],
                                &aligned,
                                outcome,
                                baseline,
                                cli.samples,
                            );
                        }
                        record(&mut rows, &mut form_sample_pool, aligned, cli.samples);
                    }
                }
            }
        }
    }

    let (fixture_forms, covered_partitions, report_gaps) = match &cli.fidelity_report {
        Some(path) => coverage_from_report(path, &cli.rows)?,
        None => (BTreeMap::new(), BTreeMap::new(), BTreeMap::new()),
    };
    for (row_id, row) in &mut rows {
        let fixture = fixture_forms.get(row_id);
        let covered = covered_partitions.get(row_id);
        for (form, report) in &mut row.forms {
            if let Some(samples) = form_sample_pool.remove(&(row_id.clone(), form.clone())) {
                report.samples = samples;
            }
            if cli.fidelity_report.is_some() {
                let partitions = report.partitions.keys().cloned().collect();
                let class = form_class(
                    &report.counts.statuses,
                    &partitions,
                    fixture.is_some_and(|forms| forms.contains(form)),
                    covered.unwrap_or(&BTreeSet::new()),
                );
                let totals = row
                    .classes
                    .entry(
                        serde_json::to_value(&class)?
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                    )
                    .or_default();
                totals.forms += 1;
                totals.occurrences += report.counts.occurrences;
                report.class = Some(class);
            }
        }
    }
    for obligation in &mut obligations {
        obligation.credited = obligation.failed == 0 && obligation.satisfied > 0;
    }
    let mut partition_credit = BTreeMap::<String, BTreeMap<String, PartitionCredit>>::new();
    for (row_id, partitions) in &covered_partitions {
        for partition in partitions {
            partition_credit
                .entry(row_id.clone())
                .or_default()
                .entry(partition.clone())
                .or_default()
                .fixture = true;
        }
    }
    for (check, obligation) in checks.iter().zip(&obligations) {
        if obligation.credited {
            for partition in &obligation.partitions {
                partition_credit
                    .entry(check.row_id.clone())
                    .or_default()
                    .entry(partition.clone())
                    .or_default()
                    .predicates
                    .insert(check.obligation_id.clone(), obligation.satisfied);
            }
        }
    }
    let decisions = match &cli.decisions {
        Some(path) => load_decisions(
            &fs::read_to_string(path)
                .with_context(|| format!("read decisions {}", path.display()))?,
        )?,
        None => Vec::new(),
    };
    let inventory_mismatches = match &cli.inventory_json {
        Some(path) => {
            let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
            let inventory: Value = serde_json::from_slice(&bytes)
                .with_context(|| format!("parse {}", path.display()))?;
            cli.rows
                .iter()
                .flat_map(|row_id| {
                    inventory_mismatches(
                        &inventory,
                        row_id,
                        ledger_occurrences
                            .get(row_id)
                            .map_or(&[][..], Vec::as_slice),
                    )
                })
                .collect()
        }
        None => Vec::new(),
    };
    let unmatched = cli
        .rows
        .iter()
        .flat_map(|row_id| {
            unmatched_rulings(
                &decisions,
                row_id,
                ledger_occurrences
                    .get(row_id)
                    .map_or(&[][..], Vec::as_slice),
            )
        })
        .collect::<Vec<_>>();
    if !unmatched.is_empty() {
        bail!("rulings name no occurrence: {}", unmatched.join("; "));
    }
    let residue_ledger = cli
        .rows
        .iter()
        .map(|row_id| {
            let mut covered = covered_partitions.get(row_id).cloned().unwrap_or_default();
            for (check, obligation) in checks.iter().zip(&obligations) {
                if &check.row_id == row_id && obligation.credited {
                    covered.extend(obligation.partitions.iter().cloned());
                }
            }
            let coverage = Coverage {
                fixture_forms: fixture_forms.get(row_id).cloned().unwrap_or_default(),
                covered_partitions: covered,
                rulings: decisions
                    .iter()
                    .filter(|decision| &decision.row == row_id)
                    .cloned()
                    .collect(),
            };
            let occurrences = ledger_occurrences.remove(row_id).unwrap_or_default();
            (
                row_id.clone(),
                row_ledger(&occurrences, &coverage, cli.samples),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let ratchet_rows = match &cli.ratchet {
        Some(path) => {
            let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
            let previous: Value = serde_json::from_slice(&bytes)
                .with_context(|| format!("parse {}", path.display()))?;
            Some(ratchet(&previous, &residue_ledger))
        }
        None => None,
    };
    let obligation_checks = checks
        .iter()
        .map(|check| check.obligation_id.clone())
        .zip(obligations)
        .collect::<BTreeMap<_, _>>();
    let unflipped = cli
        .expect_flips
        .iter()
        .filter(|obligation_id| {
            obligation_checks[obligation_id.as_str()]
                .discrimination
                .as_ref()
                .is_none_or(|discrimination| !discrimination.counts.flipped())
        })
        .map(String::as_str)
        .collect::<Vec<_>>();
    let qualification = cli.fidelity_report.is_some().then(|| {
        residue_ledger
            .iter()
            .map(|(row_id, ledger)| {
                let gaps = report_gaps
                    .get(row_id)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                (row_id.clone(), row_qualification(gaps, ledger))
            })
            .collect::<BTreeMap<_, _>>()
    });
    let report = serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "inputs": {
            "index_sha256": hex_sha256(&index_bytes),
            "matrix_sha256": hex_sha256(&matrix_bytes),
            "parser_sha256": parser_sha256,
            "baseline_parser_sha256": baseline.as_ref().map(|(_, sha256)| sha256),
            "fidelity_report_sha256": cli.fidelity_report.as_ref().map(|path| fs::read(path).map(|bytes| hex_sha256(&bytes))).transpose()?,
            "requirements_sha256": check_identity.as_ref().map(|(requirements, _)| requirements),
            "rows": cli.rows,
        },
        "editions": editions,
        "rows": rows,
        "obligation_checks": obligation_checks,
        "partition_credit": partition_credit,
        "residue_ledger": residue_ledger,
        "qualification": qualification,
        "ratchet": ratchet_rows,
        "inventory_mismatches": inventory_mismatches,
    });
    fs::write(&cli.output_json, serde_json::to_vec_pretty(&report)?)
        .with_context(|| format!("write {}", cli.output_json.display()))?;
    let regressed = ratchet_rows
        .iter()
        .flatten()
        .filter(|(_, row)| row.regressed)
        .map(|(row_id, _)| row_id.as_str())
        .collect::<Vec<_>>();
    if !regressed.is_empty() {
        bail!("residue grew in {}", regressed.join(", "));
    }
    if !unflipped.is_empty() {
        bail!(
            "no occurrence changed check outcome between the baseline and candidate parser for {}",
            unflipped.join(", ")
        );
    }
    if !inventory_mismatches.is_empty() {
        bail!(
            "{} ledger totals differ from {}",
            inventory_mismatches.len(),
            cli.inventory_json
                .as_ref()
                .map_or_else(String::new, |path| path.display().to_string())
        );
    }
    Ok(())
}

fn align_edition(
    entry: &SourceIndexEntry,
    cli: &Cli,
    patterns: &[ab_coverage::source_inventory::SourceInventoryPattern],
    parser_sha256: &str,
    baseline: Option<(&Path, &str)>,
    checks: &[ObligationCheck],
    check_identity: Option<&(String, String)>,
) -> EditionOutcome {
    let failure = |stage, error: anyhow::Error| {
        EditionOutcome::Failed(EditionFailure {
            work_id: entry.work_id.clone(),
            indexed_path: entry.indexed_path.clone(),
            stage,
            error: format!("{error:#}"),
        })
    };
    let work = match read_source_work(&cli.corpus, &entry.work_id, &entry.indexed_path) {
        Ok(work) => work,
        Err(error) => return failure("read", error),
    };
    let text = &work.decoded.text;
    let summary =
        inventory_document_with_all_variant_occurrences(&entry.work_id, text, patterns, &cli.rows);
    let located = located_occurrences(&summary, &cli.rows);
    if located.is_empty() {
        return EditionOutcome::Aligned {
            occurrences: Vec::new(),
            parsed: None,
        };
    }
    let (nodes, aat) = match cached_alignment_index(text, cli, parser_sha256) {
        Ok((nodes, aat)) => (Some(nodes), aat),
        Err(error) => {
            eprintln!(
                "parse failed for {} ({}): {error:#}",
                entry.work_id, entry.indexed_path
            );
            (None, None)
        }
    };
    let parsed = nodes.as_ref().map(|_| aat.is_some());
    let requests = located
        .iter()
        .flat_map(|located| {
            checks
                .iter()
                .filter(|check| check.applies(located))
                .map(|check| {
                    (
                        check.obligation_id.clone(),
                        located.byte_start,
                        located.occurrence.raw.clone(),
                    )
                })
        })
        .collect::<BTreeSet<_>>();
    let check_identity = |parser_sha256: &str| {
        check_identity.map(|(requirements_sha256, checker_sha256)| CheckCacheIdentity {
            schema_version: CHECK_CACHE_SCHEMA_VERSION.to_owned(),
            parser_sha256: parser_sha256.to_owned(),
            source_sha256: hex_sha256(text.as_bytes()),
            requirements_sha256: requirements_sha256.clone(),
            checker_sha256: checker_sha256.clone(),
        })
    };
    let check_outcomes = match (&nodes, check_identity(parser_sha256)) {
        (Some(_), Some(identity)) if !requests.is_empty() => {
            match cached_check_outcomes(text, cli, &cli.parser, identity, checks, &requests, aat) {
                Ok(outcomes) => outcomes,
                Err(error) => return failure("check", error),
            }
        }
        _ => BTreeMap::new(),
    };
    let baseline_outcomes =
        baseline.map(
            |(parser, parser_sha256)| match check_identity(parser_sha256) {
                Some(identity) if !requests.is_empty() => {
                    cached_check_outcomes(text, cli, parser, identity, checks, &requests, None)
                        .unwrap_or_else(|error| {
                            eprintln!(
                                "baseline check failed for {} ({}): {error:#}",
                                entry.work_id, entry.indexed_path
                            );
                            BTreeMap::new()
                        })
                }
                _ => BTreeMap::new(),
            },
        );
    let occurrences = located
        .into_iter()
        .map(|located| {
            let end = located.byte_start + located.occurrence.raw.len();
            let status = match &nodes {
                None => AlignmentStatus::ParseFailed,
                Some(_)
                    if text.get(located.byte_start..end)
                        != Some(located.occurrence.raw.as_str()) =>
                {
                    AlignmentStatus::Unlocated
                }
                Some(nodes) => alignment_status(nodes, located.byte_start, end),
            };
            let uncertain_source_retained = status == AlignmentStatus::RawPreserved
                && nodes.as_ref().is_some_and(|nodes| {
                    retains_uncertain_source(
                        nodes,
                        located.byte_start,
                        end,
                        &located.occurrence.raw,
                    )
                });
            let checks = checks
                .iter()
                .enumerate()
                .filter(|(_, check)| check.applies(&located))
                .map(|(index, check)| {
                    let key = (
                        check.obligation_id.clone(),
                        located.byte_start,
                        located.occurrence.raw.clone(),
                    );
                    let outcome = match status {
                        AlignmentStatus::ParseFailed | AlignmentStatus::Unlocated => {
                            CheckOutcome::NoNode
                        }
                        _ => check_outcomes
                            .get(&key)
                            .cloned()
                            .unwrap_or(CheckOutcome::NoNode),
                    };
                    let baseline = baseline_outcomes.as_ref().map(|outcomes| {
                        outcomes.get(&key).cloned().unwrap_or(CheckOutcome::NoNode)
                    });
                    (index, outcome, baseline)
                })
                .collect();
            AlignedOccurrence {
                located,
                indexed_path: entry.indexed_path.clone(),
                status,
                uncertain_source_retained,
                checks,
            }
        })
        .collect();
    EditionOutcome::Aligned {
        occurrences,
        parsed,
    }
}

/// Returns the alignment index and, when the parser had to run, its AAT.
fn cached_alignment_index(
    text: &str,
    cli: &Cli,
    parser_sha256: &str,
) -> Result<(Vec<AlignmentNode>, Option<Value>)> {
    let identity = CacheIdentity {
        schema_version: CACHE_SCHEMA_VERSION.to_owned(),
        parser_sha256: parser_sha256.to_owned(),
        source_sha256: hex_sha256(text.as_bytes()),
    };
    let key = hex_sha256(&serde_json::to_vec(&identity)?);
    let path = cli.cache_dir.join(&key[..2]).join(format!("{key}.json"));
    if let Ok(bytes) = fs::read(&path) {
        let entry: CacheEntry = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse cache entry {}", path.display()))?;
        if entry.identity != identity {
            bail!("cache identity mismatch in {}", path.display());
        }
        return Ok((entry.nodes, None));
    }
    let aat = run_parser(&cli.parser, text.as_bytes())?;
    let nodes = alignment_index(&aat, text);
    write_atomically(
        &path,
        &serde_json::to_vec(&CacheEntry {
            identity,
            nodes: nodes.clone(),
        })?,
    )?;
    Ok((nodes, Some(aat)))
}

type CheckKey = (String, usize, String);

/// Returns the outcome of every requested check, parsing the source again
/// only when the cache lacks one of them and `aat` is absent.
fn cached_check_outcomes(
    text: &str,
    cli: &Cli,
    parser: &Path,
    identity: CheckCacheIdentity,
    checks: &[ObligationCheck],
    requests: &BTreeSet<CheckKey>,
    aat: Option<Value>,
) -> Result<BTreeMap<CheckKey, CheckOutcome>> {
    let key = hex_sha256(&serde_json::to_vec(&identity)?);
    let path = cli.cache_dir.join(&key[..2]).join(format!("{key}.json"));
    let mut outcomes = BTreeMap::new();
    if let Ok(bytes) = fs::read(&path) {
        let entry: CheckCacheEntry = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse check cache entry {}", path.display()))?;
        if entry.identity != identity {
            bail!("cache identity mismatch in {}", path.display());
        }
        outcomes = entry
            .outcomes
            .into_iter()
            .map(|(obligation, start, raw, outcome)| ((obligation, start, raw), outcome))
            .collect();
    }
    if requests
        .iter()
        .all(|request| outcomes.contains_key(request))
    {
        return Ok(outcomes);
    }
    let aat = match aat {
        Some(aat) => aat,
        None => run_parser(parser, text.as_bytes())?,
    };
    for request in requests {
        let (obligation_id, start, raw) = request;
        let check = checks
            .iter()
            .find(|check| &check.obligation_id == obligation_id)
            .expect("requests name loaded checks");
        outcomes.insert(
            request.clone(),
            check.check.evaluate(&aat, text, *start, raw),
        );
    }
    write_atomically(
        &path,
        &serde_json::to_vec(&CheckCacheEntry {
            identity,
            outcomes: outcomes
                .iter()
                .map(|((obligation, start, raw), outcome)| {
                    (obligation.clone(), *start, raw.clone(), outcome.clone())
                })
                .collect(),
        })?,
    )?;
    Ok(outcomes)
}

fn record_check(
    report: &mut ObligationReport,
    aligned: &AlignedOccurrence,
    outcome: CheckOutcome,
    baseline: Option<CheckOutcome>,
    sample_limit: usize,
) {
    report.occurrences += 1;
    if let (Some(discrimination), Some(baseline)) = (&mut report.discrimination, baseline)
        && let Some(comparison) = BaselineComparison::between(&baseline, &outcome)
    {
        discrimination.counts.add(comparison);
        if matches!(
            comparison,
            BaselineComparison::Gained | BaselineComparison::Lost
        ) {
            let samples = discrimination.samples.entry(comparison).or_default();
            samples.push(FlipSample {
                work_id: aligned.located.occurrence.work_id.clone(),
                indexed_path: aligned.indexed_path.clone(),
                line: aligned.located.occurrence.line,
                byte_start: aligned.located.byte_start,
                raw: aligned.located.occurrence.raw.clone(),
                baseline,
                candidate: outcome.clone(),
            });
            samples.sort_by(|left, right| {
                (&left.work_id, left.byte_start).cmp(&(&right.work_id, right.byte_start))
            });
            samples.truncate(sample_limit);
        }
    }
    match &outcome {
        CheckOutcome::Satisfied => report.satisfied += 1,
        outcome if outcome.failed() => report.failed += 1,
        _ => report.unchecked += 1,
    }
    let label = outcome.label();
    *report.outcomes.entry(label.clone()).or_default() += 1;
    if !outcome.failed() {
        return;
    }
    let samples = report.samples.entry(label).or_default();
    samples.push(CheckSample {
        work_id: aligned.located.occurrence.work_id.clone(),
        indexed_path: aligned.indexed_path.clone(),
        line: aligned.located.occurrence.line,
        byte_start: aligned.located.byte_start,
        raw: aligned.located.occurrence.raw.clone(),
        outcome,
    });
    samples.sort_by(|left, right| {
        (&left.work_id, left.byte_start).cmp(&(&right.work_id, right.byte_start))
    });
    samples.truncate(sample_limit);
}

fn ledger_occurrence(aligned: &AlignedOccurrence, checks: &[ObligationCheck]) -> LedgerOccurrence {
    let located = &aligned.located;
    LedgerOccurrence {
        form: located.form.clone().unwrap_or_default(),
        partitions: located.partitions.clone(),
        status: aligned.status,
        uncertain_source_retained: aligned.uncertain_source_retained,
        checks_satisfied: aligned
            .checks
            .iter()
            .any(|(_, outcome, _)| *outcome == CheckOutcome::Satisfied),
        checks_failed: aligned
            .checks
            .iter()
            .any(|(_, outcome, _)| outcome.failed()),
        text_retained: aligned.checks.iter().any(|(index, outcome, _)| {
            *outcome == CheckOutcome::Satisfied && checks[*index].check.check.marker_text
        }),
        locator: Locator {
            work_id: located.occurrence.work_id.clone(),
            indexed_path: aligned.indexed_path.clone(),
            line: located.occurrence.line,
            byte_start: located.byte_start,
            raw: located.occurrence.raw.clone(),
        },
    }
}

fn record(
    rows: &mut BTreeMap<String, RowReport>,
    sample_pool: &mut BTreeMap<(String, String), Vec<Sample>>,
    aligned: AlignedOccurrence,
    sample_limit: usize,
) {
    let AlignedOccurrence {
        located,
        indexed_path,
        status,
        ..
    } = aligned;
    let row = rows
        .get_mut(&located.row_id)
        .expect("located occurrences belong to selected rows");
    row.counts.add(status);
    for partition in &located.partitions {
        row.partitions
            .entry(partition.clone())
            .or_default()
            .add(status);
    }
    let Some(form) = located.form else {
        row.occurrences_without_form += 1;
        return;
    };
    let report = row.forms.entry(form.clone()).or_default();
    report.counts.add(status);
    for partition in &located.partitions {
        *report.partitions.entry(partition.clone()).or_default() += 1;
    }
    let pool = sample_pool.entry((located.row_id, form)).or_default();
    pool.push(Sample {
        work_id: located.occurrence.work_id,
        indexed_path,
        line: located.occurrence.line,
        byte_start: located.byte_start,
        raw: located.occurrence.raw,
        status,
    });
    // Keep the lowest-ranked statuses: non-typed evidence is what review needs.
    pool.sort_by(|left, right| {
        (left.status, &left.work_id, left.byte_start).cmp(&(
            right.status,
            &right.work_id,
            right.byte_start,
        ))
    });
    pool.truncate(sample_limit);
}

type RowSets = BTreeMap<String, BTreeSet<String>>;

/// The fixture forms, covered partitions and report gaps of each row.
fn coverage_from_report(
    path: &Path,
    rows: &[String],
) -> Result<(RowSets, RowSets, BTreeMap<String, Vec<String>>)> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let report: Value =
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
    let keys = |section: &str, field: &str| -> RowSets {
        rows.iter()
            .map(|row_id| {
                let keys = report
                    .get(section)
                    .and_then(|section| section.get(row_id))
                    .and_then(|row| row.get(field))
                    .and_then(Value::as_object)
                    .map(|object| object.keys().cloned().collect())
                    .unwrap_or_default();
                (row_id.clone(), keys)
            })
            .collect()
    };
    let gaps = report
        .get("rows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let id = row.get("id")?.as_str()?;
            rows.contains(&id.to_owned()).then(|| {
                let gaps = row
                    .get("gaps")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                (id.to_owned(), gaps)
            })
        })
        .collect();
    Ok((
        keys("variant_coverage", "cases_by_form"),
        keys("semantic_partition_coverage", "cases_by_partition"),
        gaps,
    ))
}
