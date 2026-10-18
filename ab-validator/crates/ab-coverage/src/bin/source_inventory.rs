use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use ab_coverage::{
    matrix::{CoverageMatrix, RepresentabilityStatus, Row},
    source_corpus::{SourceIndexEntry, load_index_entries, read_source_work},
    source_inventory::{
        MarkerSemanticPartitionSummary, MarkerVariantSummary, SourceMarkerOccurrence,
        inventory_document_with_variants, patterns_from_rows,
    },
};
use ab_encoding::{
    emendation::{Emendation, emendation_table},
    hex_sha256,
};
use anyhow::{Context, Result, bail};
use clap::Parser;
use rayon::prelude::*;
use regex::Regex;
use serde::{Deserialize, Serialize};

const UNKNOWN_CLASS_OUTPUT_LIMIT: usize = 1000;
const UNKNOWN_CLASS_REPORT_LIMIT: usize = 50;
const SOURCE_REGION_SCHEMA_VERSION: &str = "aozora-source-region-coverage-v1";
const EDITION_CACHE_SCHEMA_VERSION: &str = "aozora-source-inventory-edition-cache-v3";

#[derive(Parser, Debug)]
#[command(version, about = "Source-authority Aozora marker inventory.")]
struct Cli {
    #[arg(long, default_value = "data/aozora-syntax-coverage.toml")]
    matrix: PathBuf,
    /// Emit independent lexical accountability for one extracted source file.
    #[arg(long, conflicts_with_all = ["index", "corpus", "report_md", "work_ids", "allowlist", "unknown_workset", "edition_results", "fail_on_unknown", "strict_representability", "jobs", "variant_row"])]
    source: Option<PathBuf>,
    #[arg(long, required_unless_present = "source")]
    index: Option<PathBuf>,
    #[arg(long, required_unless_present = "source")]
    corpus: Option<PathBuf>,
    #[arg(long)]
    work_ids: Option<PathBuf>,
    #[arg(long)]
    allowlist: Option<PathBuf>,
    #[arg(long)]
    output_json: PathBuf,
    #[arg(long, required_unless_present = "source")]
    report_md: Option<PathBuf>,
    #[arg(long)]
    unknown_workset: Option<PathBuf>,
    #[arg(long)]
    edition_results: Option<PathBuf>,
    /// Reuse content-verified per-edition inventory outside the source tree.
    #[arg(long, conflicts_with = "source")]
    cache_dir: Option<PathBuf>,
    #[arg(long)]
    fail_on_unknown: bool,
    #[arg(long)]
    strict_representability: bool,
    #[arg(long)]
    jobs: Option<usize>,
    /// Retain normalized forms and source samples for this matrix row.
    #[arg(long)]
    variant_row: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
struct InventoryOutput {
    schema_version: String,
    gate_status: String,
    works_scanned: u64,
    works_failed: u64,
    markers_total: u64,
    unknown_markers_total: u64,
    unallowlisted_unknown_markers_total: u64,
    allowlisted_unknown_markers_total: u64,
    rows: BTreeMap<String, RowOutput>,
    unknown_classes_total: u64,
    unknown_classes_truncated: bool,
    unknown_classes: Vec<UnknownClassOutput>,
    unknown_examples: Vec<SourceMarkerOccurrence>,
    decode_failures: Vec<DecodeFailure>,
    representability: RepresentabilityOutput,
    source_region_coverage: SourceRegionCoverageOutput,
    emendations: EmendationsOutput,
    strict_errors: Vec<String>,
    inputs: Inputs,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache: Option<CacheProvenance>,
}

#[derive(Debug, Default, Serialize)]
struct RowOutput {
    works_with_marker: u64,
    occurrences: u64,
    sample_works: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    variants: BTreeMap<String, MarkerVariantSummary>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    semantic_partitions: BTreeMap<String, MarkerSemanticPartitionSummary>,
}

/// Verification of the source emendation table against the scanned editions.
///
/// An entry is unapplied when its declared edition was scanned and the entry
/// did not apply to it; misapplied when it applied to another edition or to
/// more than one.
#[derive(Debug, Default, Serialize)]
struct EmendationsOutput {
    entries: BTreeMap<String, EmendationOutput>,
    unapplied: Vec<String>,
    misapplied: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
struct EmendationOutput {
    declared: Edition,
    applied_to: Vec<Edition>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct Edition {
    work_id: String,
    indexed_path: String,
}

#[derive(Debug, Serialize)]
struct DecodeFailure {
    work_id: String,
    indexed_path: String,
    error: String,
}

#[derive(Debug, Default, Serialize)]
struct UnknownClassOutput {
    kind: String,
    raw: String,
    body: String,
    occurrences: u64,
    allowlisted_occurrences: u64,
    unallowlisted_occurrences: u64,
    sample_works: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
struct Inputs {
    matrix: String,
    index: String,
    corpus: String,
    allowlist: Option<String>,
    work_ids: Option<String>,
    variant_rows: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
struct CacheProvenance {
    schema_version: String,
    executable_sha256: String,
    matrix_sha256: String,
    classifier_sha256: String,
    variant_rows: Vec<String>,
    entries_reused: u64,
    entries_computed: u64,
}

struct CacheConfig {
    root: PathBuf,
    executable_sha256: String,
    matrix_sha256: String,
    classifier_sha256: String,
    variant_rows: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
struct CacheIdentity {
    schema_version: String,
    executable_sha256: String,
    classifier_sha256: String,
    variant_rows: Vec<String>,
    work_id: String,
    indexed_path: String,
    source_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct CacheEntry {
    identity: CacheIdentity,
    summary: ab_coverage::source_inventory::SourceInventorySummary,
    source_region_text: SourceRegionTextSummary,
    emendation_ids: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
struct RepresentabilityOutput {
    typed_occurrences: u64,
    raw_preserved_occurrences: u64,
    out_of_body_occurrences: u64,
    malformed_noise_occurrences: u64,
    unsupported_occurrences: u64,
    needs_research_occurrences: u64,
}

#[derive(Debug, Default, Serialize)]
struct SourceRegionCoverageOutput {
    body_typed_occurrences: u64,
    body_raw_preserved_occurrences: u64,
    source_apparatus_occurrences: u64,
    front_matter_occurrences: u64,
    back_matter_occurrences: u64,
    body_end_boundary_occurrences: u64,
    terminal_provenance_occurrences: u64,
    colophon_metadata_occurrences: u64,
    letter_address_origin_occurrences: u64,
    malformed_source_occurrences: u64,
    unsupported_body_markup_occurrences: u64,
    unknown_region_occurrences: u64,
    unknown_unreviewed_occurrences: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct SourceRegionTextSummary {
    colophon_metadata_occurrences: u64,
    letter_address_origin_occurrences: u64,
}

#[derive(Debug, Deserialize)]
struct AllowlistFile {
    #[serde(default)]
    allow: Vec<AllowRule>,
}

#[derive(Debug, Deserialize)]
struct AllowRule {
    id: String,
    kind: String,
    body_pattern: Option<String>,
    raw_pattern: Option<String>,
    scope: String,
    reason: String,
    evidence: String,
}

struct CompiledAllowRule {
    rule: AllowRule,
    body_pattern: Option<Regex>,
    raw_pattern: Option<Regex>,
}

enum WorkInventoryResult {
    Scanned {
        work_id: String,
        summary: ab_coverage::source_inventory::SourceInventorySummary,
        source_region_text: SourceRegionTextSummary,
        emendation_ids: Vec<String>,
        cache_reused: bool,
    },
    Failed {
        work_id: String,
        indexed_path: String,
        error: String,
    },
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
    let patterns = patterns_from_rows(matrix.rows());
    if let Some(source) = &cli.source {
        let report = ab_coverage::source_accountability::source_accountability(
            &fs::read(source).with_context(|| format!("read source {}", source.display()))?,
            &matrix_bytes,
            &patterns,
        );
        fs::write(&cli.output_json, serde_json::to_vec_pretty(&report)?)?;
        return Ok(());
    }
    let index = cli
        .index
        .as_ref()
        .context("corpus inventory requires --index")?;
    let corpus = cli
        .corpus
        .as_ref()
        .context("corpus inventory requires --corpus")?;
    let report_md = cli
        .report_md
        .as_ref()
        .context("corpus inventory requires --report-md")?;
    let rows_by_id = matrix
        .rows()
        .iter()
        .map(|row| (row.id.as_str(), row))
        .collect::<BTreeMap<_, _>>();
    for row_id in &cli.variant_row {
        if !rows_by_id.contains_key(row_id.as_str()) {
            bail!("unknown --variant-row {row_id:?}");
        }
    }
    let mut cache_variant_rows = cli.variant_row.clone();
    cache_variant_rows.sort();
    cache_variant_rows.dedup();
    let classifier_bytes = serde_json::to_vec(
        &patterns
            .iter()
            .map(|pattern| (&pattern.row_id, &pattern.source_patterns))
            .collect::<Vec<_>>(),
    )?;
    let cache_config = cli
        .cache_dir
        .as_ref()
        .map(|root| -> Result<CacheConfig> {
            let executable = std::env::current_exe().context("locate inventory executable")?;
            let executable_bytes = fs::read(&executable)
                .with_context(|| format!("read inventory executable {}", executable.display()))?;
            Ok(CacheConfig {
                root: root.clone(),
                executable_sha256: hex_sha256(&executable_bytes),
                matrix_sha256: hex_sha256(&matrix_bytes),
                classifier_sha256: hex_sha256(&classifier_bytes),
                variant_rows: cache_variant_rows.clone(),
            })
        })
        .transpose()?;
    let index_entries = load_index_entries(index)?;
    let allowlist = load_allowlist(cli.allowlist.as_deref())?;
    let work_entries = load_work_entries(cli.work_ids.as_deref(), &index_entries)?;
    let emendations = emendation_table()?;

    let mut output = InventoryOutput {
        schema_version: SOURCE_REGION_SCHEMA_VERSION.to_owned(),
        inputs: Inputs {
            matrix: cli.matrix.display().to_string(),
            index: index.display().to_string(),
            corpus: corpus.display().to_string(),
            allowlist: cli
                .allowlist
                .as_ref()
                .map(|path| path.display().to_string()),
            work_ids: cli.work_ids.as_ref().map(|path| path.display().to_string()),
            variant_rows: cache_variant_rows.clone(),
        },
        cache: cache_config.as_ref().map(|cache| CacheProvenance {
            schema_version: EDITION_CACHE_SCHEMA_VERSION.to_owned(),
            executable_sha256: cache.executable_sha256.clone(),
            matrix_sha256: cache.matrix_sha256.clone(),
            classifier_sha256: cache.classifier_sha256.clone(),
            variant_rows: cache.variant_rows.clone(),
            ..CacheProvenance::default()
        }),
        ..InventoryOutput::default()
    };
    let mut unknown_work_ids = BTreeSet::new();
    let mut unknown_classes = BTreeMap::new();
    let mut strict_errors = Vec::new();
    let mut scanned_editions = BTreeSet::new();
    let mut emendation_editions = BTreeMap::<String, Vec<Edition>>::new();

    let work_results = work_entries
        .par_iter()
        .map(|entry| {
            scan_work(
                entry,
                corpus,
                &patterns,
                &cli.variant_row,
                cache_config.as_ref(),
            )
        })
        .collect::<Result<Vec<_>>>()?;

    let mut edition_results = cli
        .edition_results
        .as_ref()
        .map(|path| fs::File::create(path).map(BufWriter::new))
        .transpose()?;
    for (entry, result) in work_entries.iter().zip(work_results) {
        if let Some(writer) = &mut edition_results {
            let record = match &result {
                WorkInventoryResult::Scanned { summary, .. } => serde_json::json!({
                    "indexed_path": entry.indexed_path, "summary": summary
                }),
                WorkInventoryResult::Failed { error, .. } => serde_json::json!({
                    "indexed_path": entry.indexed_path, "work_id": entry.work_id, "error": error
                }),
            };
            serde_json::to_writer(&mut *writer, &record)?;
            writeln!(writer)?;
        }
        match result {
            WorkInventoryResult::Scanned {
                work_id,
                summary,
                source_region_text,
                emendation_ids,
                cache_reused,
            } => {
                let edition = Edition {
                    work_id: work_id.clone(),
                    indexed_path: entry.indexed_path.clone(),
                };
                for id in emendation_ids {
                    emendation_editions
                        .entry(id)
                        .or_default()
                        .push(edition.clone());
                }
                scanned_editions.insert(edition);
                if let Some(cache) = &mut output.cache {
                    if cache_reused {
                        cache.entries_reused += 1;
                    } else {
                        cache.entries_computed += 1;
                    }
                }
                output.works_scanned += 1;
                output.markers_total += summary.markers_total;
                output.representability.out_of_body_occurrences +=
                    summary.source_region_events.front_matter_occurrences
                        + summary.source_region_events.body_end_boundary_occurrences
                        + summary.source_region_events.back_matter_occurrences;
                observe_source_region_text(&mut output.source_region_coverage, source_region_text);
                observe_source_region_events(
                    &mut output.source_region_coverage,
                    &summary.source_region_events,
                );
                for (row_id, count) in summary.row_counts {
                    let row = output.rows.entry(row_id).or_default();
                    row.occurrences += count.occurrences;
                    row.works_with_marker += 1;
                    if row.sample_works.len() < 5 && !row.sample_works.contains(&work_id) {
                        row.sample_works.push(work_id.clone());
                    }
                }
                for (row_id, variants) in summary.row_variants {
                    let row = output.rows.entry(row_id).or_default();
                    for (form, observed) in variants {
                        let aggregate = row.variants.entry(form).or_default();
                        aggregate.occurrences += observed.occurrences;
                        for (partition, occurrences) in observed.semantic_partitions {
                            *aggregate.semantic_partitions.entry(partition).or_default() +=
                                occurrences;
                        }
                        for sample in observed.samples {
                            if aggregate.samples.len() == 3 {
                                break;
                            }
                            aggregate.samples.push(sample);
                        }
                    }
                }
                for (row_id, partitions) in summary.row_semantic_partitions {
                    let row = output.rows.entry(row_id).or_default();
                    for (partition, observed) in partitions {
                        let aggregate = row.semantic_partitions.entry(partition).or_default();
                        aggregate.occurrences += observed.occurrences;
                        for sample in observed.samples {
                            if aggregate.samples.len() == 3 {
                                break;
                            }
                            aggregate.samples.push(sample);
                        }
                    }
                }

                for example in summary.unknown_examples {
                    output.unknown_markers_total += 1;
                    unknown_work_ids.insert(work_id.clone());
                    if let Some(rule) = matching_allowlist_rule(&example, &allowlist) {
                        output.allowlisted_unknown_markers_total += 1;
                        observe_unknown_class(&mut unknown_classes, &work_id, &example, true);
                        observe_allowlisted_representability(
                            &mut output.representability,
                            &mut output.source_region_coverage,
                            &rule.rule,
                        );
                    } else {
                        output.unallowlisted_unknown_markers_total += 1;
                        output.source_region_coverage.unknown_region_occurrences += 1;
                        output.source_region_coverage.unknown_unreviewed_occurrences += 1;
                        observe_unknown_class(&mut unknown_classes, &work_id, &example, false);
                        if output.unknown_examples.len() < 100 {
                            output.unknown_examples.push(example);
                        }
                    }
                }
            }
            WorkInventoryResult::Failed {
                work_id,
                indexed_path,
                error,
            } => {
                output.works_failed += 1;
                output.decode_failures.push(DecodeFailure {
                    work_id,
                    indexed_path,
                    error,
                });
            }
        }
    }

    observe_row_representability(&mut output, &rows_by_id, &mut strict_errors);
    output.emendations = verify_emendations(emendations, emendation_editions, &scanned_editions);
    let emendation_errors = emendation_errors(&output.emendations);
    strict_errors.extend(emendation_errors.iter().cloned());
    if let Some(writer) = &mut edition_results {
        writer.flush()?;
    }
    if output.works_failed > 0 {
        strict_errors.push(format!(
            "{} works failed source inventory read/decode",
            output.works_failed
        ));
    }
    if output.unallowlisted_unknown_markers_total > 0 {
        strict_errors.push(format!(
            "{} unallowlisted source markers",
            output.unallowlisted_unknown_markers_total
        ));
    }
    let unknown_classes = sorted_unknown_classes(unknown_classes);
    output.unknown_classes_total = unknown_classes.len() as u64;
    output.unknown_classes_truncated = unknown_classes.len() > UNKNOWN_CLASS_OUTPUT_LIMIT;
    output.unknown_classes = unknown_classes
        .into_iter()
        .take(UNKNOWN_CLASS_OUTPUT_LIMIT)
        .collect();
    output.strict_errors = strict_errors.clone();
    output.gate_status = source_authority_gate_status(&output).to_owned();

    write_json(&cli.output_json, &output)?;
    write_report(report_md, &output)?;
    if let Some(path) = &cli.unknown_workset {
        let ids = unknown_work_ids.into_iter().collect::<Vec<_>>();
        write_json(path, &ids)?;
    }

    if !emendation_errors.is_empty() {
        bail!(
            "source emendation verification failed:\n{}",
            emendation_errors.join("\n")
        );
    }
    if cli.fail_on_unknown && output.unallowlisted_unknown_markers_total > 0 {
        bail!(
            "{} unallowlisted source markers",
            output.unallowlisted_unknown_markers_total
        );
    }
    if cli.strict_representability && !strict_errors.is_empty() {
        bail!(
            "strict representability failed:\n{}",
            strict_errors.join("\n")
        );
    }

    Ok(())
}

fn scan_work(
    entry: &SourceIndexEntry,
    corpus: &Path,
    patterns: &[ab_coverage::source_inventory::SourceInventoryPattern],
    variant_rows: &[String],
    cache: Option<&CacheConfig>,
) -> Result<WorkInventoryResult> {
    match read_source_work(corpus, &entry.work_id, &entry.indexed_path) {
        Ok(work) => {
            let identity = cache.map(|cache| CacheIdentity {
                schema_version: EDITION_CACHE_SCHEMA_VERSION.to_owned(),
                executable_sha256: cache.executable_sha256.clone(),
                classifier_sha256: cache.classifier_sha256.clone(),
                variant_rows: cache.variant_rows.clone(),
                work_id: entry.work_id.clone(),
                indexed_path: entry.indexed_path.clone(),
                source_sha256: hex_sha256(&work.bytes),
            });
            if let (Some(cache), Some(identity)) = (cache, identity.as_ref()) {
                let path = cache_entry_path(cache, identity)?;
                if path.exists() {
                    let cached = read_cache_entry(&path, identity)?;
                    return Ok(WorkInventoryResult::Scanned {
                        work_id: entry.work_id.clone(),
                        summary: cached.summary,
                        source_region_text: cached.source_region_text,
                        emendation_ids: cached.emendation_ids,
                        cache_reused: true,
                    });
                }
            }
            let mut summary = inventory_document_with_variants(
                &entry.work_id,
                &work.decoded.text,
                patterns,
                variant_rows,
            );
            attach_indexed_path(&mut summary, &entry.indexed_path);
            let source_region_text = source_region_text_summary(&work.decoded.text);
            let emendation_ids = work
                .decoded
                .emendations
                .iter()
                .map(|item| item.id.clone())
                .collect::<Vec<_>>();
            if let (Some(cache), Some(identity)) = (cache, identity) {
                let path = cache_entry_path(cache, &identity)?;
                write_cache_entry(
                    &path,
                    &CacheEntry {
                        identity,
                        summary: summary.clone(),
                        source_region_text: source_region_text.clone(),
                        emendation_ids: emendation_ids.clone(),
                    },
                )?;
            }
            Ok(WorkInventoryResult::Scanned {
                work_id: entry.work_id.clone(),
                summary,
                source_region_text,
                emendation_ids,
                cache_reused: false,
            })
        }
        Err(err) => Ok(WorkInventoryResult::Failed {
            work_id: entry.work_id.clone(),
            indexed_path: entry.indexed_path.clone(),
            error: format!("{err:#}"),
        }),
    }
}

fn verify_emendations(
    table: &[Emendation],
    mut applied: BTreeMap<String, Vec<Edition>>,
    scanned: &BTreeSet<Edition>,
) -> EmendationsOutput {
    let mut output = EmendationsOutput::default();
    for entry in table {
        let declared = Edition {
            work_id: entry.work_id.clone(),
            indexed_path: entry.indexed_path.clone(),
        };
        let mut applied_to = applied.remove(&entry.id).unwrap_or_default();
        applied_to.sort();
        if applied_to.is_empty() && scanned.contains(&declared) {
            output.unapplied.push(entry.id.clone());
        }
        if applied_to.len() > 1 || applied_to.iter().any(|edition| *edition != declared) {
            output.misapplied.push(entry.id.clone());
        }
        output.entries.insert(
            entry.id.clone(),
            EmendationOutput {
                declared,
                applied_to,
            },
        );
    }
    output
}

fn emendation_errors(output: &EmendationsOutput) -> Vec<String> {
    let describe = |id: &String| {
        let entry = &output.entries[id];
        let applied = entry
            .applied_to
            .iter()
            .map(|edition| format!("{} {}", edition.work_id, edition.indexed_path))
            .collect::<Vec<_>>();
        format!(
            "declared {} {}, applied to [{}]",
            entry.declared.work_id,
            entry.declared.indexed_path,
            applied.join(", ")
        )
    };
    output
        .unapplied
        .iter()
        .map(|id| format!("source emendation {id} unapplied: {}", describe(id)))
        .chain(
            output
                .misapplied
                .iter()
                .map(|id| format!("source emendation {id} misapplied: {}", describe(id))),
        )
        .collect()
}

fn cache_entry_path(cache: &CacheConfig, identity: &CacheIdentity) -> Result<PathBuf> {
    let identity_bytes = serde_json::to_vec(identity)?;
    let key = hex_sha256(&identity_bytes);
    Ok(cache.root.join(&key[..2]).join(format!("{key}.json")))
}

fn read_cache_entry(path: &Path, identity: &CacheIdentity) -> Result<CacheEntry> {
    let bytes = fs::read(path).with_context(|| format!("read cache entry {}", path.display()))?;
    let entry: CacheEntry = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse cache entry {}", path.display()))?;
    if entry.identity != *identity {
        bail!("cache identity mismatch in {}", path.display());
    }
    Ok(entry)
}

fn write_cache_entry(path: &Path, entry: &CacheEntry) -> Result<()> {
    let parent = path.parent().context("cache entry has no parent")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("create cache directory {}", parent.display()))?;
    let thread = rayon::current_thread_index().unwrap_or(0);
    let temporary = path.with_extension(format!("tmp-{}-{thread}", std::process::id()));
    fs::write(&temporary, serde_json::to_vec(entry)?)
        .with_context(|| format!("write cache entry {}", temporary.display()))?;
    fs::rename(&temporary, path)
        .with_context(|| format!("publish cache entry {}", path.display()))?;
    Ok(())
}

fn attach_indexed_path(
    summary: &mut ab_coverage::source_inventory::SourceInventorySummary,
    indexed_path: &str,
) {
    let attach = |occurrence: &mut SourceMarkerOccurrence| {
        occurrence.indexed_path = Some(indexed_path.to_owned());
    };
    for variants in summary.row_variants.values_mut() {
        for variant in variants.values_mut() {
            variant.samples.iter_mut().for_each(&attach);
        }
    }
    for partitions in summary.row_semantic_partitions.values_mut() {
        for partition in partitions.values_mut() {
            partition.samples.iter_mut().for_each(&attach);
        }
    }
    summary.unknown_examples.iter_mut().for_each(&attach);
    for marker in &mut summary.classified_region_markers {
        attach(&mut marker.occurrence);
    }
}

fn load_work_entries(
    path: Option<&Path>,
    entries: &[SourceIndexEntry],
) -> Result<Vec<SourceIndexEntry>> {
    if let Some(path) = path {
        let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let ids: Vec<String> = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse work id array {}", path.display()))?;
        let requested = ids.into_iter().collect::<BTreeSet<_>>();
        let selected = entries
            .iter()
            .filter(|entry| requested.contains(&entry.work_id))
            .cloned()
            .collect::<Vec<_>>();
        let selected_ids = selected
            .iter()
            .map(|entry| entry.work_id.as_str())
            .collect::<BTreeSet<_>>();
        let missing = requested
            .iter()
            .filter(|id| !selected_ids.contains(id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            bail!(
                "--work-ids contains ids missing from index: {}",
                missing.join(", ")
            );
        }
        return Ok(selected);
    }
    Ok(entries.to_vec())
}

fn load_allowlist(path: Option<&Path>) -> Result<Vec<CompiledAllowRule>> {
    let Some(path) = path else {
        return Ok(Vec::new());
    };
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let parsed: AllowlistFile =
        toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    parsed
        .allow
        .into_iter()
        .map(|rule| {
            if rule.body_pattern.is_none() && rule.raw_pattern.is_none() {
                bail!(
                    "allowlist rule {} needs body_pattern or raw_pattern",
                    rule.id
                );
            }
            if !matches!(
                rule.scope.as_str(),
                "out_of_body"
                    | "malformed_noise"
                    | "unsupported_v1"
                    | "front_matter_legend"
                    | "notation_placeholder"
                    | "body_end_boundary"
                    | "terminal_provenance"
                    | "colophon_metadata"
                    | "back_matter_provenance"
                    | "malformed_source"
            ) {
                bail!(
                    "allowlist rule {} has invalid scope {}",
                    rule.id,
                    rule.scope
                );
            }
            if rule.reason.trim().is_empty() || rule.evidence.trim().is_empty() {
                bail!("allowlist rule {} needs reason and evidence", rule.id);
            }
            let body_pattern =
                compile_optional_regex(&rule.id, "body_pattern", &rule.body_pattern)?;
            let raw_pattern = compile_optional_regex(&rule.id, "raw_pattern", &rule.raw_pattern)?;
            Ok(CompiledAllowRule {
                rule,
                body_pattern,
                raw_pattern,
            })
        })
        .collect()
}

fn source_region_text_summary(text: &str) -> SourceRegionTextSummary {
    SourceRegionTextSummary {
        colophon_metadata_occurrences: text
            .lines()
            .filter(|line| is_colophon_metadata_line(line.trim_start()))
            .count() as u64,
        letter_address_origin_occurrences: text
            .lines()
            .filter(|line| is_letter_address_origin_line(line.trim_start()))
            .count() as u64,
    }
}

fn is_colophon_metadata_line(line: &str) -> bool {
    const COLOPHON_PREFIXES: &[&str] = &[
        "底本：",
        "底本:",
        "底本の親本：",
        "底本の親本:",
        "親本：",
        "親本:",
        "初出：",
        "初出:",
        "入力：",
        "入力:",
        "校正：",
        "校正:",
        "校閲：",
        "校閲:",
        "作成日：",
        "作成日:",
        "修正：",
        "修正:",
        "ファイル作成：",
        "ファイル作成:",
        "青空文庫作成ファイル：",
        "青空文庫作成ファイル:",
    ];
    COLOPHON_PREFIXES
        .iter()
        .any(|prefix| line.starts_with(prefix))
}

fn is_letter_address_origin_line(line: &str) -> bool {
    line.starts_with("宛先") || line.starts_with("発信地")
}

fn compile_optional_regex(
    id: &str,
    field: &str,
    pattern: &Option<String>,
) -> Result<Option<Regex>> {
    pattern
        .as_ref()
        .map(|pattern| Regex::new(pattern).with_context(|| format!("allowlist {id} {field}")))
        .transpose()
}

fn matching_allowlist_rule<'a>(
    example: &SourceMarkerOccurrence,
    allowlist: &'a [CompiledAllowRule],
) -> Option<&'a CompiledAllowRule> {
    allowlist.iter().find(|entry| {
        entry.rule.kind == example.kind
            && (entry
                .body_pattern
                .as_ref()
                .is_some_and(|pattern| pattern.is_match(&example.body))
                || entry
                    .raw_pattern
                    .as_ref()
                    .is_some_and(|pattern| pattern.is_match(&example.raw)))
    })
}

fn observe_allowlisted_representability(
    representability: &mut RepresentabilityOutput,
    source_region_coverage: &mut SourceRegionCoverageOutput,
    rule: &AllowRule,
) {
    match rule.scope.as_str() {
        "out_of_body" => {
            representability.out_of_body_occurrences += 1;
        }
        "unsupported_v1" => {
            representability.raw_preserved_occurrences += 1;
            source_region_coverage.body_raw_preserved_occurrences += 1;
        }
        "malformed_noise" | "front_matter_legend" => {
            representability.malformed_noise_occurrences += 1;
            source_region_coverage.source_apparatus_occurrences += 1;
            source_region_coverage.front_matter_occurrences += 1;
        }
        "notation_placeholder" => {
            representability.out_of_body_occurrences += 1;
            source_region_coverage.front_matter_occurrences += 1;
        }
        "body_end_boundary" => {
            representability.out_of_body_occurrences += 1;
            source_region_coverage.body_end_boundary_occurrences += 1;
            source_region_coverage.back_matter_occurrences += 1;
        }
        "terminal_provenance" => {
            representability.out_of_body_occurrences += 1;
            source_region_coverage.back_matter_occurrences += 1;
            source_region_coverage.terminal_provenance_occurrences += 1;
        }
        "back_matter_provenance" => {
            representability.out_of_body_occurrences += 1;
            source_region_coverage.back_matter_occurrences += 1;
        }
        "colophon_metadata" => {
            representability.out_of_body_occurrences += 1;
            source_region_coverage.back_matter_occurrences += 1;
            source_region_coverage.colophon_metadata_occurrences += 1;
        }
        "malformed_source" => {
            representability.malformed_noise_occurrences += 1;
            source_region_coverage.malformed_source_occurrences += 1;
        }
        _ => {}
    }
}

fn observe_source_region_text(
    source_region_coverage: &mut SourceRegionCoverageOutput,
    summary: SourceRegionTextSummary,
) {
    source_region_coverage.colophon_metadata_occurrences += summary.colophon_metadata_occurrences;
    source_region_coverage.back_matter_occurrences += summary.colophon_metadata_occurrences;
    source_region_coverage.letter_address_origin_occurrences +=
        summary.letter_address_origin_occurrences;
    source_region_coverage.back_matter_occurrences += summary.letter_address_origin_occurrences;
}

fn observe_source_region_events(
    source_region_coverage: &mut SourceRegionCoverageOutput,
    summary: &ab_coverage::source_inventory::SourceRegionEventSummary,
) {
    source_region_coverage.front_matter_occurrences += summary.front_matter_occurrences;
    source_region_coverage.body_end_boundary_occurrences += summary.body_end_boundary_occurrences;
    source_region_coverage.back_matter_occurrences += summary.back_matter_occurrences;
    source_region_coverage.terminal_provenance_occurrences +=
        summary.terminal_provenance_occurrences;
    source_region_coverage.back_matter_occurrences += summary.terminal_provenance_occurrences;
}

fn observe_unknown_class(
    classes: &mut BTreeMap<String, UnknownClassOutput>,
    work_id: &str,
    example: &SourceMarkerOccurrence,
    allowlisted: bool,
) {
    let key = format!(
        "{}\u{1f}{}\u{1f}{}",
        example.kind, example.raw, example.body
    );
    let class = classes.entry(key).or_insert_with(|| UnknownClassOutput {
        kind: example.kind.clone(),
        raw: example.raw.clone(),
        body: example.body.clone(),
        ..UnknownClassOutput::default()
    });
    class.occurrences += 1;
    if allowlisted {
        class.allowlisted_occurrences += 1;
    } else {
        class.unallowlisted_occurrences += 1;
    }
    if class.sample_works.len() < 5 && !class.sample_works.iter().any(|sample| sample == work_id) {
        class.sample_works.push(work_id.to_owned());
    }
}

fn sorted_unknown_classes(
    classes: BTreeMap<String, UnknownClassOutput>,
) -> Vec<UnknownClassOutput> {
    let mut classes = classes.into_values().collect::<Vec<_>>();
    classes.sort_by_key(|class| {
        (
            Reverse(class.unallowlisted_occurrences),
            Reverse(class.occurrences),
            class.kind.clone(),
            class.raw.clone(),
        )
    });
    classes
}

fn observe_row_representability(
    output: &mut InventoryOutput,
    rows_by_id: &BTreeMap<&str, &Row>,
    strict_errors: &mut Vec<String>,
) {
    for (row_id, row_output) in &output.rows {
        if row_output.occurrences == 0 {
            continue;
        }
        let Some(row) = rows_by_id.get(row_id.as_str()) else {
            strict_errors.push(format!(
                "source inventory row {row_id} is not present in matrix"
            ));
            continue;
        };
        let Some(cell) = &row.representability else {
            strict_errors.push(format!(
                "source inventory row {row_id} has occurrences but no representability table"
            ));
            continue;
        };
        if representability_requires_tei_projection(cell.status) && !has_tei_projection(row) {
            strict_errors.push(format!(
                "source inventory row {row_id} has representability.status = {} but no tei_projection",
                cell.status.as_str()
            ));
        }
        match cell.status {
            RepresentabilityStatus::Typed => {
                if cell.aat_nodes.is_empty() {
                    strict_errors.push(format!(
                        "source inventory row {row_id} is typed but has no representability.aat_nodes"
                    ));
                }
                output.representability.typed_occurrences += row_output.occurrences;
                output.source_region_coverage.body_typed_occurrences += row_output.occurrences;
            }
            RepresentabilityStatus::RawPreserved => {
                output.representability.raw_preserved_occurrences += row_output.occurrences;
                output.source_region_coverage.body_raw_preserved_occurrences +=
                    row_output.occurrences;
            }
            RepresentabilityStatus::OutOfBody => {
                output.representability.out_of_body_occurrences += row_output.occurrences;
            }
            RepresentabilityStatus::Unsupported => {
                output.representability.unsupported_occurrences += row_output.occurrences;
                output
                    .source_region_coverage
                    .unsupported_body_markup_occurrences += row_output.occurrences;
            }
            RepresentabilityStatus::NeedsResearch => {
                output.representability.needs_research_occurrences += row_output.occurrences;
                output.source_region_coverage.unknown_unreviewed_occurrences +=
                    row_output.occurrences;
                strict_errors.push(format!(
                    "source inventory row {row_id} has representability.status = needs_research"
                ));
            }
        }
    }
}

fn representability_requires_tei_projection(status: RepresentabilityStatus) -> bool {
    matches!(
        status,
        RepresentabilityStatus::Typed
            | RepresentabilityStatus::RawPreserved
            | RepresentabilityStatus::OutOfBody
    )
}

fn has_tei_projection(row: &Row) -> bool {
    let projection = row.tei_projection.trim();
    !projection.is_empty() && !projection.eq_ignore_ascii_case("n/a")
}

fn source_authority_gate_status(output: &InventoryOutput) -> &'static str {
    if output.works_failed == 0
        && output.unallowlisted_unknown_markers_total == 0
        && output.representability.needs_research_occurrences == 0
        && output.source_region_coverage.unknown_region_occurrences == 0
        && output.source_region_coverage.unknown_unreviewed_occurrences == 0
        && output
            .source_region_coverage
            .unsupported_body_markup_occurrences
            == 0
        && output.strict_errors.is_empty()
    {
        "SOURCE_AUTHORITY_GATE_PASS"
    } else {
        "SOURCE_AUTHORITY_GATE_FAILING_REVIEW_REQUIRED"
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)
        .with_context(|| format!("write {}", path.display()))
}

fn write_report(path: &Path, output: &InventoryOutput) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let mut report = String::new();
    report.push_str("# Source Authority Representability Inventory\n\n");
    report.push_str("## Verdict\n\n");
    report.push_str(&format!(
        "- source_authority_gate: `{}`\n",
        output.gate_status
    ));
    if output.gate_status == "SOURCE_AUTHORITY_GATE_FAILING_REVIEW_REQUIRED" {
        report.push_str(
            "- note: this is not a passing representability gate; durable representability claims remain blocked until strict_errors is empty.\n",
        );
    }
    if output.strict_errors.is_empty() {
        report.push_str("- strict_errors: none\n");
    } else {
        report.push_str("- strict_errors:\n");
        for error in &output.strict_errors {
            report.push_str(&format!("  - {}\n", escape_md(error)));
        }
    }

    report.push_str("\n## Scope\n\n");
    report.push_str(
        "This is a source-markup authority gate: every reached explicit Aozora Bunko marker must have a reviewed representation and, for represented rows, a TEI P5 projection target. Semantic TEI enrichment such as named-entity, speech, role, or place annotation is outside this gate and remains a downstream editorial layer.\n",
    );

    report.push_str("\n## Summary\n\n");
    report.push_str(&format!("- works_scanned: {}\n", output.works_scanned));
    report.push_str(&format!("- works_failed: {}\n", output.works_failed));
    report.push_str(&format!("- markers_total: {}\n", output.markers_total));
    report.push_str(&format!(
        "- unknown_markers_total: {}\n",
        output.unknown_markers_total
    ));
    report.push_str(&format!(
        "- unallowlisted_unknown_markers_total: {}\n",
        output.unallowlisted_unknown_markers_total
    ));
    report.push_str(&format!(
        "- allowlisted_unknown_markers_total: {}\n",
        output.allowlisted_unknown_markers_total
    ));

    report.push_str("\n## Representability\n\n");
    report.push_str(&format!(
        "- typed_occurrences: {}\n",
        output.representability.typed_occurrences
    ));
    report.push_str(&format!(
        "- raw_preserved_occurrences: {}\n",
        output.representability.raw_preserved_occurrences
    ));
    report.push_str(&format!(
        "- out_of_body_occurrences: {}\n",
        output.representability.out_of_body_occurrences
    ));
    report.push_str(&format!(
        "- malformed_noise_occurrences: {}\n",
        output.representability.malformed_noise_occurrences
    ));
    report.push_str(&format!(
        "- unsupported_occurrences: {}\n",
        output.representability.unsupported_occurrences
    ));
    report.push_str(&format!(
        "- needs_research_occurrences: {}\n",
        output.representability.needs_research_occurrences
    ));

    report.push_str("\n## Source Region Coverage\n\n");
    report.push_str(&format!("- schema_version: `{}`\n", output.schema_version));
    report.push_str(&format!(
        "- body_typed_occurrences: {}\n",
        output.source_region_coverage.body_typed_occurrences
    ));
    report.push_str(&format!(
        "- body_raw_preserved_occurrences: {}\n",
        output.source_region_coverage.body_raw_preserved_occurrences
    ));
    report.push_str(&format!(
        "- source_apparatus_occurrences: {}\n",
        output.source_region_coverage.source_apparatus_occurrences
    ));
    report.push_str(&format!(
        "- front_matter_occurrences: {}\n",
        output.source_region_coverage.front_matter_occurrences
    ));
    report.push_str(&format!(
        "- back_matter_occurrences: {}\n",
        output.source_region_coverage.back_matter_occurrences
    ));
    report.push_str(&format!(
        "- body_end_boundary_occurrences: {}\n",
        output.source_region_coverage.body_end_boundary_occurrences
    ));
    report.push_str(&format!(
        "- terminal_provenance_occurrences: {}\n",
        output
            .source_region_coverage
            .terminal_provenance_occurrences
    ));
    report.push_str(&format!(
        "- colophon_metadata_occurrences: {}\n",
        output.source_region_coverage.colophon_metadata_occurrences
    ));
    report.push_str(&format!(
        "- letter_address_origin_occurrences: {}\n",
        output
            .source_region_coverage
            .letter_address_origin_occurrences
    ));
    report.push_str(&format!(
        "- malformed_source_occurrences: {}\n",
        output.source_region_coverage.malformed_source_occurrences
    ));
    report.push_str(&format!(
        "- unsupported_body_markup_occurrences: {}\n",
        output
            .source_region_coverage
            .unsupported_body_markup_occurrences
    ));
    report.push_str(&format!(
        "- unknown_region_occurrences: {}\n",
        output.source_region_coverage.unknown_region_occurrences
    ));
    report.push_str(&format!(
        "- unknown_unreviewed_occurrences: {}\n",
        output.source_region_coverage.unknown_unreviewed_occurrences
    ));

    report.push_str("\n## Rows\n\n");
    report.push_str("| row | works | occurrences | samples |\n");
    report.push_str("|---|---:|---:|---|\n");
    for (row_id, row) in &output.rows {
        report.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            row_id,
            row.works_with_marker,
            row.occurrences,
            row.sample_works.join(", ")
        ));
    }

    report.push_str("\n## Unknown Source Markers\n\n");
    if output.unknown_examples.is_empty() {
        report.push_str("None.\n");
    } else {
        report.push_str("| work_id | line | kind | raw | body |\n");
        report.push_str("|---|---:|---|---|---|\n");
        for example in &output.unknown_examples {
            report.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                example.work_id,
                example.line,
                example.kind,
                escape_md(&example.raw),
                escape_md(&example.body)
            ));
        }
    }

    report.push_str("\n## Unknown Source Marker Classes\n\n");
    if output.unknown_classes.is_empty() {
        report.push_str("None.\n");
    } else {
        let report_rows = output.unknown_classes.len().min(UNKNOWN_CLASS_REPORT_LIMIT);
        report.push_str(&format!(
            "Showing {} report rows of {} total classes. JSON carries {} top classes. truncated: {}\n\n",
            report_rows,
            output.unknown_classes_total,
            output.unknown_classes.len(),
            output.unknown_classes_truncated
        ));
        report.push_str("| kind | raw | occurrences | unallowlisted | allowlisted | samples |\n");
        report.push_str("|---|---|---:|---:|---:|---|\n");
        for class in output
            .unknown_classes
            .iter()
            .take(UNKNOWN_CLASS_REPORT_LIMIT)
        {
            report.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                class.kind,
                escape_md(&class.raw),
                class.occurrences,
                class.unallowlisted_occurrences,
                class.allowlisted_occurrences,
                class.sample_works.join(", ")
            ));
        }
    }

    report.push_str("\n## Decode Failures\n\n");
    if output.decode_failures.is_empty() {
        report.push_str("None.\n");
    } else {
        for failure in &output.decode_failures {
            report.push_str(&format!(
                "- {} `{}`: {}\n",
                failure.work_id, failure.indexed_path, failure.error
            ));
        }
    }

    report.push_str("\n## Inputs\n\n");
    report.push_str(&format!("- matrix: `{}`\n", output.inputs.matrix));
    report.push_str(&format!("- index: `{}`\n", output.inputs.index));
    report.push_str(&format!("- corpus: `{}`\n", output.inputs.corpus));
    if let Some(path) = &output.inputs.allowlist {
        report.push_str(&format!("- allowlist: `{path}`\n"));
    }
    if let Some(path) = &output.inputs.work_ids {
        report.push_str(&format!("- work_ids: `{path}`\n"));
    }

    fs::write(path, report).with_context(|| format!("write {}", path.display()))
}

fn escape_md(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edition(work_id: &str) -> Edition {
        Edition {
            work_id: work_id.to_owned(),
            indexed_path: format!("cards/{work_id}.zip::a.txt"),
        }
    }

    fn entry(id: &str, work_id: &str) -> Emendation {
        let declared = edition(work_id);
        Emendation {
            id: id.to_owned(),
            work_id: declared.work_id,
            indexed_path: declared.indexed_path,
            transcribed: "x".to_owned(),
            emended: "y".to_owned(),
        }
    }

    #[test]
    fn classifies_unapplied_and_misapplied_entries() {
        let table = [
            entry("applied", "w1"),
            entry("stale", "w2"),
            entry("not-scanned", "w3"),
            entry("elsewhere", "w4"),
            entry("repeated", "w5"),
        ];
        let scanned = ["w1", "w2", "w4", "w5", "w6"]
            .into_iter()
            .map(edition)
            .collect::<BTreeSet<_>>();
        let applied = BTreeMap::from([
            ("applied".to_owned(), vec![edition("w1")]),
            ("elsewhere".to_owned(), vec![edition("w6")]),
            ("repeated".to_owned(), vec![edition("w5"), edition("w6")]),
        ]);
        let output = verify_emendations(&table, applied, &scanned);
        assert_eq!(output.unapplied, ["stale"]);
        assert_eq!(output.misapplied, ["elsewhere", "repeated"]);
        assert_eq!(output.entries.len(), 5);
        assert_eq!(emendation_errors(&output).len(), 3);
    }

    #[test]
    fn application_to_another_file_of_the_declared_work_is_misapplied() {
        let table = [entry("a", "w1")];
        let other = Edition {
            work_id: "w1".to_owned(),
            indexed_path: "cards/w1-other.zip::a.txt".to_owned(),
        };
        let scanned = BTreeSet::from([other.clone()]);
        let applied = BTreeMap::from([("a".to_owned(), vec![other])]);
        let output = verify_emendations(&table, applied, &scanned);
        assert_eq!(output.misapplied, ["a"]);
        assert!(output.unapplied.is_empty(), "declared edition not scanned");
    }
}
