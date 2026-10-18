use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::occurrence_alignment::AlignmentStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UndecidableReason {
    AmbiguousTarget,
    SourceConflict,
    MalformedBoundary,
    InsufficientSourceAuthority,
}

/// One source occurrence a ruling identifies.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuledOccurrence {
    pub work_id: String,
    pub indexed_path: String,
    pub byte_start: usize,
    pub raw: String,
}

impl RuledOccurrence {
    fn names(&self, locator: &Locator) -> bool {
        self.work_id == locator.work_id
            && self.indexed_path == locator.indexed_path
            && self.byte_start == locator.byte_start
            && self.raw == locator.raw
    }
}

/// A reviewer's ruling that the listed occurrences of a normalized form, or
/// every occurrence when `locators` is empty, have no single defensible reading.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub row: String,
    pub form: String,
    #[serde(default)]
    pub locators: Vec<RuledOccurrence>,
    pub reason: UndecidableReason,
    pub evidence: Vec<String>,
    pub reviewer: String,
    pub reviewed_at: String,
    pub notes: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionsFile {
    decisions_version: u32,
    #[serde(default)]
    decision: Vec<Decision>,
}

/// # Errors
///
/// Returns an error when the document does not parse or has another version.
pub fn load_decisions(text: &str) -> Result<Vec<Decision>> {
    let file: DecisionsFile = toml::from_str(text).context("parse residue decisions")?;
    if file.decisions_version != 1 {
        bail!("unsupported decisions_version {}", file.decisions_version);
    }
    Ok(file.decision)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OccurrenceClass {
    FixtureCovered,
    PredicateCovered,
    AliasOfCovered,
    /// A ruled occurrence that meets the obligation of
    /// `occurrence_alignment::retains_uncertain_source`.
    Undecidable,
    /// A ruled occurrence that fails that obligation.
    UndecidableConflict,
    SemanticGap,
    AlignmentReview,
    CheckFailed,
    Unqualified,
}

impl OccurrenceClass {
    #[must_use]
    pub fn is_residue(self) -> bool {
        !matches!(
            self,
            Self::FixtureCovered | Self::PredicateCovered | Self::AliasOfCovered
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Locator {
    pub work_id: String,
    pub indexed_path: String,
    pub line: usize,
    pub byte_start: usize,
    pub raw: String,
}

/// The evidence the ledger needs about one located occurrence.
#[derive(Debug, Clone)]
pub struct LedgerOccurrence {
    pub form: String,
    pub partitions: BTreeSet<String>,
    pub status: AlignmentStatus,
    /// See `occurrence_alignment::retains_uncertain_source`.
    pub uncertain_source_retained: bool,
    pub checks_satisfied: bool,
    pub checks_failed: bool,
    /// A `marker_text` check found the marker's source in text nodes.
    pub text_retained: bool,
    pub locator: Locator,
}

/// What a row's fixtures, predicates and reviewers already account for.
#[derive(Debug, Default)]
pub struct Coverage {
    pub fixture_forms: BTreeSet<String>,
    /// Partitions credited by passing fixtures or by credited corpus checks.
    pub covered_partitions: BTreeSet<String>,
    pub rulings: Vec<Decision>,
}

impl Coverage {
    fn ruling(&self, occurrence: &LedgerOccurrence) -> Option<UndecidableReason> {
        self.rulings
            .iter()
            .find(|decision| {
                decision.form == occurrence.form
                    && (decision.locators.is_empty()
                        || decision
                            .locators
                            .iter()
                            .any(|ruled| ruled.names(&occurrence.locator)))
            })
            .map(|decision| decision.reason)
    }
}

/// The rulings for `row_id`, or their locators, that identify no occurrence of
/// the row's form, as when the corpus or the form's normalization changed.
#[must_use]
pub fn unmatched_rulings(
    decisions: &[Decision],
    row_id: &str,
    occurrences: &[LedgerOccurrence],
) -> Vec<String> {
    let mut unmatched = Vec::new();
    for decision in decisions.iter().filter(|decision| decision.row == row_id) {
        let forms = occurrences
            .iter()
            .filter(|occurrence| occurrence.form == decision.form)
            .collect::<Vec<_>>();
        if decision.locators.is_empty() && forms.is_empty() {
            unmatched.push(format!("{row_id} {}", decision.form));
        }
        for ruled in &decision.locators {
            if !forms
                .iter()
                .any(|occurrence| ruled.names(&occurrence.locator))
            {
                unmatched.push(format!(
                    "{row_id} {} at {} byte {}",
                    decision.form, ruled.indexed_path, ruled.byte_start
                ));
            }
        }
    }
    unmatched
}

#[must_use]
pub fn occurrence_class(occurrence: &LedgerOccurrence, coverage: &Coverage) -> OccurrenceClass {
    if coverage.ruling(occurrence).is_some() {
        return if occurrence.uncertain_source_retained {
            OccurrenceClass::Undecidable
        } else {
            OccurrenceClass::UndecidableConflict
        };
    }
    match occurrence.status {
        AlignmentStatus::RawPreserved => OccurrenceClass::SemanticGap,
        AlignmentStatus::Unaligned if occurrence.text_retained && !occurrence.checks_failed => {
            OccurrenceClass::PredicateCovered
        }
        AlignmentStatus::Unaligned | AlignmentStatus::Unlocated | AlignmentStatus::ParseFailed => {
            OccurrenceClass::AlignmentReview
        }
        AlignmentStatus::Typed if occurrence.checks_failed => OccurrenceClass::CheckFailed,
        AlignmentStatus::Typed if occurrence.checks_satisfied => OccurrenceClass::PredicateCovered,
        AlignmentStatus::Typed if coverage.fixture_forms.contains(&occurrence.form) => {
            OccurrenceClass::FixtureCovered
        }
        AlignmentStatus::Typed
            if !occurrence.partitions.is_empty()
                && occurrence
                    .partitions
                    .is_subset(&coverage.covered_partitions) =>
        {
            OccurrenceClass::AliasOfCovered
        }
        AlignmentStatus::Typed => OccurrenceClass::Unqualified,
    }
}

#[derive(Debug, Default, Serialize)]
pub struct ClassTotal {
    pub forms: u64,
    pub occurrences: u64,
}

#[derive(Debug, Serialize)]
pub struct ResidueForm {
    pub form: String,
    pub occurrences: u64,
    pub partitions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<UndecidableReason>,
    /// Every locator for undecidable forms; the first few otherwise.
    pub locators: Vec<Locator>,
}

/// Semantic-gap occurrences sharing one partition set, so a parser pass can
/// address them together regardless of spelling.
#[derive(Debug, Serialize)]
pub struct GapBatch {
    pub partitions: Vec<String>,
    pub forms: u64,
    pub occurrences: u64,
}

#[derive(Debug, Serialize)]
pub struct RowLedger {
    pub occurrences: u64,
    pub residue_occurrences: u64,
    pub classes: BTreeMap<OccurrenceClass, ClassTotal>,
    pub residue: BTreeMap<OccurrenceClass, Vec<ResidueForm>>,
    pub gap_batches: Vec<GapBatch>,
}

#[must_use]
pub fn row_ledger(
    occurrences: &[LedgerOccurrence],
    coverage: &Coverage,
    sample_limit: usize,
) -> RowLedger {
    let mut by_class = BTreeMap::<OccurrenceClass, BTreeMap<&str, Vec<&LedgerOccurrence>>>::new();
    for occurrence in occurrences {
        by_class
            .entry(occurrence_class(occurrence, coverage))
            .or_default()
            .entry(occurrence.form.as_str())
            .or_default()
            .push(occurrence);
    }
    let mut classes = BTreeMap::new();
    let mut residue = BTreeMap::new();
    let mut batches = BTreeMap::<Vec<String>, (BTreeSet<&str>, u64)>::new();
    for (class, forms) in &by_class {
        classes.insert(
            *class,
            ClassTotal {
                forms: forms.len() as u64,
                occurrences: forms.values().map(|members| members.len() as u64).sum(),
            },
        );
        if !class.is_residue() {
            continue;
        }
        let mut entries = forms
            .iter()
            .map(|(form, members)| {
                let partitions = members
                    .iter()
                    .flat_map(|member| member.partitions.iter().cloned())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                let reason = members.iter().find_map(|member| coverage.ruling(member));
                let mut locators = members
                    .iter()
                    .map(|member| member.locator.clone())
                    .collect::<Vec<_>>();
                locators.sort();
                if reason.is_none() {
                    locators.truncate(sample_limit);
                }
                ResidueForm {
                    form: (*form).to_owned(),
                    occurrences: members.len() as u64,
                    partitions,
                    reason,
                    locators,
                }
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            (right.occurrences, &left.form).cmp(&(left.occurrences, &right.form))
        });
        if *class == OccurrenceClass::SemanticGap {
            for (form, members) in forms {
                for member in members {
                    let batch = batches
                        .entry(member.partitions.iter().cloned().collect())
                        .or_default();
                    batch.0.insert(form);
                    batch.1 += 1;
                }
            }
        }
        residue.insert(*class, entries);
    }
    let mut gap_batches = batches
        .into_iter()
        .map(|(partitions, (forms, occurrences))| GapBatch {
            partitions,
            forms: forms.len() as u64,
            occurrences,
        })
        .collect::<Vec<_>>();
    gap_batches.sort_by(|left, right| {
        (right.occurrences, &left.partitions).cmp(&(left.occurrences, &right.partitions))
    });
    RowLedger {
        occurrences: occurrences.len() as u64,
        residue_occurrences: classes
            .iter()
            .filter(|(class, _)| class.is_residue())
            .map(|(_, total)| total.occurrences)
            .sum(),
        classes,
        residue,
        gap_batches,
    }
}

/// A form or partition whose ledger occurrence count differs from the
/// source inventory's count for the row.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct InventoryMismatch {
    pub row: String,
    pub section: &'static str,
    pub key: String,
    pub inventory: u64,
    pub ledger: u64,
}

/// Compare the ledger occurrences of `row_id` with the `variants` and
/// `semantic_partitions` occurrence counts of a source-inventory report.
#[must_use]
pub fn inventory_mismatches(
    inventory: &serde_json::Value,
    row_id: &str,
    occurrences: &[LedgerOccurrence],
) -> Vec<InventoryMismatch> {
    let mut ledger = BTreeMap::<(&'static str, &str), u64>::new();
    for occurrence in occurrences {
        if !occurrence.form.is_empty() {
            *ledger.entry(("variants", &occurrence.form)).or_default() += 1;
        }
        for partition in &occurrence.partitions {
            *ledger
                .entry(("semantic_partitions", partition))
                .or_default() += 1;
        }
    }
    let row = inventory.get("rows").and_then(|rows| rows.get(row_id));
    let mut expected = BTreeMap::<(&'static str, &str), u64>::new();
    for section in ["variants", "semantic_partitions"] {
        for (key, value) in row
            .and_then(|row| row.get(section))
            .and_then(serde_json::Value::as_object)
            .into_iter()
            .flatten()
        {
            expected.insert(
                (section, key),
                value
                    .get("occurrences")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
            );
        }
    }
    expected
        .keys()
        .chain(ledger.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|&(section, key)| {
            let inventory = expected.get(&(section, key)).copied().unwrap_or(0);
            let ledger = ledger.get(&(section, key)).copied().unwrap_or(0);
            (inventory != ledger).then(|| InventoryMismatch {
                row: row_id.to_owned(),
                section,
                key: key.to_owned(),
                inventory,
                ledger,
            })
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub struct RatchetRow {
    pub previous_residue: u64,
    pub current_residue: u64,
    /// Occurrence count change per class, omitting unchanged classes.
    pub class_deltas: BTreeMap<String, i64>,
    pub regressed: bool,
}

/// A row is qualified when the fidelity report lists no gap for it and no
/// occurrence in its ledger is residue other than a ruled undecidable.
#[derive(Debug, Serialize)]
pub struct RowQualification {
    pub qualified: bool,
    pub report_gaps: Vec<String>,
    /// Residue occurrences per class, omitting empty classes and ruled
    /// undecidables.
    pub residue: BTreeMap<OccurrenceClass, u64>,
}

#[must_use]
pub fn row_qualification(report_gaps: &[String], ledger: &RowLedger) -> RowQualification {
    let residue = ledger
        .classes
        .iter()
        .filter(|(class, total)| {
            class.is_residue() && **class != OccurrenceClass::Undecidable && total.occurrences > 0
        })
        .map(|(class, total)| (*class, total.occurrences))
        .collect::<BTreeMap<_, _>>();
    RowQualification {
        qualified: report_gaps.is_empty() && residue.is_empty(),
        report_gaps: report_gaps.to_vec(),
        residue,
    }
}

/// Compare each row's ledger with the `residue_ledger` of a previous
/// alignment report; a row regresses when its residue grows.
#[must_use]
pub fn ratchet(
    previous: &serde_json::Value,
    current: &BTreeMap<String, RowLedger>,
) -> BTreeMap<String, RatchetRow> {
    let mut rows = BTreeMap::new();
    for (row_id, ledger) in current {
        let Some(before) = previous
            .get("residue_ledger")
            .and_then(|ledgers| ledgers.get(row_id))
        else {
            continue;
        };
        let previous_residue = before
            .get("residue_occurrences")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let mut counts = BTreeMap::<String, (i64, i64)>::new();
        for (class, total) in before
            .get("classes")
            .and_then(serde_json::Value::as_object)
            .into_iter()
            .flatten()
        {
            let occurrences = total
                .get("occurrences")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0);
            counts.entry(class.clone()).or_default().0 = occurrences;
        }
        for (class, total) in &ledger.classes {
            let name = serde_json::to_value(class)
                .ok()
                .and_then(|name| name.as_str().map(str::to_owned))
                .unwrap_or_default();
            counts.entry(name).or_default().1 =
                i64::try_from(total.occurrences).unwrap_or(i64::MAX);
        }
        rows.insert(
            row_id.clone(),
            RatchetRow {
                previous_residue,
                current_residue: ledger.residue_occurrences,
                class_deltas: counts
                    .into_iter()
                    .filter(|(_, (before, after))| before != after)
                    .map(|(class, (before, after))| (class, after - before))
                    .collect(),
                regressed: ledger.residue_occurrences > previous_residue,
            },
        );
    }
    rows
}
