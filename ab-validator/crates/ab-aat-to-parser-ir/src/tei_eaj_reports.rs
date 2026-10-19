//! Generates Melos, all-work, and workset comparison reports from
//! [`crate::tei_eaj_compare`], executes alignment probes across the workset,
//! and attaches each probe result to its corresponding workset row.
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::tei_eaj_alignment_probe::{
    TeiEajAlignmentProbeConfig, run_tei_eaj_alignment_probe, write_tei_eaj_alignment_probe_reports,
};
use crate::tei_eaj_compare::{
    AllWorkReport, MelosReport, analyze_file, build_all_work_report, melos_report_from_records,
    render_all_work_markdown, render_melos_markdown, render_sorted_json, resolve_melos_abc_path,
    workset_export,
};

pub const MELOS_REPORT_NAME: &str = "tei-eaj-aozora-melos-comparison-report.md";
pub const ALL_WORK_REPORT_NAME: &str = "tei-eaj-aozora-all-work-comparison-report.md";
pub const WORKSET_EXPORT_NAME: &str = "tei-eaj-aozora-workset-export.json";
pub const ALIGNMENT_PROBE_JSON_NAME: &str = "tei-eaj-aozora-alignment-probe.json";
pub const ALIGNMENT_PROBE_MD_NAME: &str = "tei-eaj-aozora-alignment-probe.md";
const DEFAULT_REPORT_SUBDIR: &str = "out/reports/tei-eaj-aozora";

/// Inputs shared by every report; accepted before or after the subcommand.
#[derive(Debug, Clone, clap::Args)]
pub struct ReportInputs {
    /// Pinned TEI-EAJ aozora_tei checkout root
    #[arg(long, global = true, required = false)]
    pub tei_eaj_root: Option<PathBuf>,
    /// Pinned TEI-EAJ source revision
    #[arg(long, global = true)]
    pub source_rev: Option<String>,
    /// ABC-generated Melos TEI path
    #[arg(long, global = true)]
    pub abc_melos: Option<String>,
    /// ABC TEI counterpart path or WORK_ID=PATH mapping
    #[arg(long = "abc-tei", global = true)]
    pub abc_tei: Vec<String>,
    /// Directory to scan recursively for ABC TEI counterparts; if neither
    /// --abc-tei nor --abc-tei-dir is specified, directories are read from
    /// ABC_TEI_EAJ_ABC_TEI_DIRS.
    #[arg(long = "abc-tei-dir", global = true)]
    pub abc_tei_dir: Vec<String>,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum ReportCommand {
    /// Melos, all-work and workset reports
    All {
        /// MELOS_MD ALL_WORK_MD WORKSET_JSON, or none for the report directory
        outputs: Vec<PathBuf>,
    },
    Melos {
        output: Option<PathBuf>,
    },
    AllWork {
        output: Option<PathBuf>,
    },
    WorksetJson {
        output: Option<PathBuf>,
    },
    /// Alignment probe over an existing workset, attached to its rows
    AlignmentProbe {
        workset: Option<PathBuf>,
        summary_json: Option<PathBuf>,
        report_md: Option<PathBuf>,
        #[arg(long)]
        max_rows: Option<usize>,
    },
    /// All three reports, then the alignment probe
    AllWithProbes {
        /// MELOS_MD ALL_WORK_MD WORKSET_JSON ALIGNMENT_JSON ALIGNMENT_MD, or
        /// none for the report directory
        outputs: Vec<PathBuf>,
        #[arg(long)]
        max_probe_rows: Option<usize>,
    },
}

/// `ABC_REPORT_DIR`, then `SORANOHA_REPORT_DIR`, then
/// `out/reports/tei-eaj-aozora` under the working directory.
pub fn default_report_dir() -> Result<PathBuf> {
    for name in ["ABC_REPORT_DIR", "SORANOHA_REPORT_DIR"] {
        if let Some(value) = std::env::var_os(name).filter(|value| !value.is_empty()) {
            return Ok(PathBuf::from(value));
        }
    }
    Ok(std::env::current_dir()
        .context("failed to resolve the working directory")?
        .join(DEFAULT_REPORT_SUBDIR))
}

/// Counterpart directories from a `:`-separated `ABC_TEI_EAJ_ABC_TEI_DIRS`
/// value, empty entries dropped.
pub fn abc_tei_dirs_from_env(value: Option<&str>) -> Vec<String> {
    value
        .map(|value| {
            value
                .split(':')
                .filter(|entry| !entry.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn output_or_default(output: Option<PathBuf>, name: &str) -> Result<PathBuf> {
    match output {
        Some(path) => Ok(path),
        None => Ok(default_report_dir()?.join(name)),
    }
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(path, text).with_context(|| format!("failed to write {}", path.display()))
}

struct Reports {
    melos: MelosReport,
    all_work: AllWorkReport,
}

impl ReportInputs {
    fn tei_eaj_root(&self) -> Result<&Path> {
        self.tei_eaj_root
            .as_deref()
            .context("--tei-eaj-root is required")
    }

    fn all_work(&self) -> Result<AllWorkReport> {
        build_all_work_report(
            &self.abc_tei,
            &self.abc_tei_dir,
            self.tei_eaj_root()?,
            self.source_rev.as_deref(),
        )
    }

    fn melos(&self) -> Result<MelosReport> {
        let all_work = self.all_work()?;
        Ok(self.reports_from(all_work)?.melos)
    }

    /// Both reports from one scan of the TEI-EAJ checkout.
    fn reports_from(&self, all_work: AllWorkReport) -> Result<Reports> {
        let abc_path =
            resolve_melos_abc_path(self.abc_melos.as_deref(), &self.abc_tei, &self.abc_tei_dir)?;
        let abc_features = analyze_file(Path::new(&abc_path))?;
        let melos = melos_report_from_records(
            abc_path,
            abc_features,
            self.tei_eaj_root()?,
            self.source_rev.as_deref(),
            all_work.records.clone(),
        );
        Ok(Reports { melos, all_work })
    }
}

fn write_melos(report: &MelosReport, output: Option<PathBuf>) -> Result<PathBuf> {
    let target = output_or_default(output, MELOS_REPORT_NAME)?;
    write_text(&target, &render_melos_markdown(report))?;
    Ok(target)
}

fn write_all_work(report: &AllWorkReport, output: Option<PathBuf>) -> Result<PathBuf> {
    let target = output_or_default(output, ALL_WORK_REPORT_NAME)?;
    write_text(&target, &render_all_work_markdown(report))?;
    Ok(target)
}

fn write_workset(report: &AllWorkReport, output: Option<PathBuf>) -> Result<PathBuf> {
    let target = output_or_default(output, WORKSET_EXPORT_NAME)?;
    write_text(&target, &render_sorted_json(workset_export(report))?)?;
    Ok(target)
}

/// Attaches each probe's `alignment_probe` object to the workset row matching its
/// `tei_eaj_file` and updates the workset file. Returns the count of attached rows.
pub fn attach_alignment_probes(workset_path: &Path, probe_report: &Value) -> Result<usize> {
    let workset_text = std::fs::read_to_string(workset_path)
        .with_context(|| format!("failed to read workset {}", workset_path.display()))?;
    let mut workset: Value = serde_json::from_str(&workset_text)
        .with_context(|| format!("failed to parse workset {}", workset_path.display()))?;
    let probes_by_file: std::collections::HashMap<&str, &Value> = probe_report["rows"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter(|row| !row["alignment_probe"].is_null())
                .filter_map(|row| {
                    row["tei_eaj_file"]
                        .as_str()
                        .map(|file| (file, &row["alignment_probe"]))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut attached = 0;
    if let Some(files) = workset.get_mut("files").and_then(Value::as_array_mut) {
        for row in files {
            let probe = row
                .get("tei_eaj_file")
                .and_then(Value::as_str)
                .and_then(|file| probes_by_file.get(file));
            if let (Some(probe), Some(object)) = (probe, row.as_object_mut()) {
                object.insert("alignment_probe".to_owned(), (*probe).clone());
                attached += 1;
            }
        }
    }
    write_text(workset_path, &render_sorted_json(workset)?)?;
    Ok(attached)
}

fn run_probe(
    workset: &Path,
    summary_json: &Path,
    report_md: &Path,
    max_rows: Option<usize>,
) -> Result<()> {
    let report = run_tei_eaj_alignment_probe(TeiEajAlignmentProbeConfig {
        workset_path: workset.to_path_buf(),
        max_rows,
    })?;
    write_tei_eaj_alignment_probe_reports(&report, summary_json, report_md)?;
    attach_alignment_probes(workset, &serde_json::to_value(&report)?)?;
    Ok(())
}

fn updated(path: &Path) {
    println!("Updated {}", path.display());
}

fn take<const N: usize>(outputs: Vec<PathBuf>, names: [&str; N]) -> Result<[Option<PathBuf>; N]> {
    if !outputs.is_empty() && outputs.len() != N {
        bail!(
            "expected either no output paths or exactly {N}: {}",
            names.join(" ")
        );
    }
    let mut taken: [Option<PathBuf>; N] = std::array::from_fn(|_| None);
    for (slot, output) in taken.iter_mut().zip(outputs) {
        *slot = Some(output);
    }
    Ok(taken)
}

pub fn run(inputs: &ReportInputs, command: ReportCommand) -> Result<()> {
    match command {
        ReportCommand::Melos { output } => {
            write_melos(&inputs.melos()?, output)?;
        }
        ReportCommand::AllWork { output } => {
            write_all_work(&inputs.all_work()?, output)?;
        }
        ReportCommand::WorksetJson { output } => {
            write_workset(&inputs.all_work()?, output)?;
        }
        ReportCommand::All { outputs } => {
            let [melos, all_work, workset] =
                take(outputs, ["MELOS_MD", "ALL_WORK_MD", "WORKSET_JSON"])?;
            let reports = inputs.reports_from(inputs.all_work()?)?;
            updated(&write_melos(&reports.melos, melos)?);
            updated(&write_all_work(&reports.all_work, all_work)?);
            updated(&write_workset(&reports.all_work, workset)?);
        }
        ReportCommand::AlignmentProbe {
            workset,
            summary_json,
            report_md,
            max_rows,
        } => {
            let workset = output_or_default(workset, WORKSET_EXPORT_NAME)?;
            let summary_json = output_or_default(summary_json, ALIGNMENT_PROBE_JSON_NAME)?;
            let report_md = output_or_default(report_md, ALIGNMENT_PROBE_MD_NAME)?;
            run_probe(&workset, &summary_json, &report_md, max_rows)?;
        }
        ReportCommand::AllWithProbes {
            outputs,
            max_probe_rows,
        } => {
            let [melos, all_work, workset, probe_json, probe_md] = take(
                outputs,
                [
                    "MELOS_MD",
                    "ALL_WORK_MD",
                    "WORKSET_JSON",
                    "ALIGNMENT_JSON",
                    "ALIGNMENT_MD",
                ],
            )?;
            let reports = inputs.reports_from(inputs.all_work()?)?;
            updated(&write_melos(&reports.melos, melos)?);
            updated(&write_all_work(&reports.all_work, all_work)?);
            let workset = write_workset(&reports.all_work, workset)?;
            updated(&workset);
            let probe_json = output_or_default(probe_json, ALIGNMENT_PROBE_JSON_NAME)?;
            let probe_md = output_or_default(probe_md, ALIGNMENT_PROBE_MD_NAME)?;
            run_probe(&workset, &probe_json, &probe_md, max_probe_rows)?;
            updated(&probe_json);
            updated(&probe_md);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counterpart_directories_come_only_from_an_explicit_environment_value() {
        assert_eq!(
            vec!["/first/tei", "/second/tei"],
            abc_tei_dirs_from_env(Some("/first/tei::/second/tei"))
        );
        assert!(abc_tei_dirs_from_env(None).is_empty());
        assert!(abc_tei_dirs_from_env(Some("")).is_empty());
    }

    #[test]
    fn output_lists_are_all_or_nothing() {
        let names = ["A", "B", "C"];
        assert_eq!([None, None, None], take(vec![], names).unwrap());
        assert_eq!(
            [
                Some(PathBuf::from("a")),
                Some(PathBuf::from("b")),
                Some(PathBuf::from("c"))
            ],
            take(vec!["a".into(), "b".into(), "c".into()], names).unwrap()
        );
        let error = take(vec!["a".into()], names).unwrap_err();
        assert_eq!(
            "expected either no output paths or exactly 3: A B C",
            error.to_string()
        );
    }

    #[test]
    fn probes_attach_to_workset_rows_by_tei_eaj_file() {
        let temp = tempfile::tempdir().unwrap();
        let workset_path = temp.path().join("workset.json");
        std::fs::write(
            &workset_path,
            json!({
                "schema_version": "tei-eaj-aozora-workset-export-v1",
                "files": [
                    {"tei_eaj_file": "data/complete/tei_lib_lv4/1567_tei.xml", "comparison_status": "compared"},
                    {"tei_eaj_file": "data/complete/tei_lib_lv4/86_tei.xml", "comparison_status": "compared"}
                ]
            })
            .to_string(),
        )
        .unwrap();
        let probe = json!({
            "schema_version": "tei-eaj-alignment-probe-report-v1",
            "rows": [
                {
                    "tei_eaj_file": "data/complete/tei_lib_lv4/1567_tei.xml",
                    "alignment_probe": {
                        "schema_version": "alignment-probe-v1",
                        "diagnosis_counts": {"tail_addition": 1}
                    }
                },
                {"tei_eaj_file": "data/complete/tei_lib_lv4/86_tei.xml", "alignment_probe": null}
            ]
        });

        assert_eq!(1, attach_alignment_probes(&workset_path, &probe).unwrap());
        let updated: Value =
            serde_json::from_str(&std::fs::read_to_string(&workset_path).unwrap()).unwrap();
        assert_eq!(
            json!({"tail_addition": 1}),
            updated["files"][0]["alignment_probe"]["diagnosis_counts"]
        );
        assert!(updated["files"][1].get("alignment_probe").is_none());
        let text = std::fs::read_to_string(&workset_path).unwrap();
        assert!(text.starts_with("{\n  \"files\": ["));
        assert!(text.ends_with("}\n"));
    }
}
