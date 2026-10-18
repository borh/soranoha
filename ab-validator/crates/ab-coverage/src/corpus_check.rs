use std::collections::BTreeMap;

use ab_aozora_encoding::gaiji;
use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::occurrence_alignment::LocatedOccurrence;

/// An obligation's predicate over the AAT at every corpus occurrence of its
/// semantic partitions, as declared in `fidelity-requirements.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorpusCheck {
    /// Occurrences carrying every one of these partitions are checked; an
    /// empty list checks every occurrence of the row.
    pub partitions: Vec<String>,
    /// Pattern over the occurrence's raw marker whose named groups supply
    /// `{name}` values to `fields` and `text`.
    pub operands: String,
    /// Kind, or `|`-separated kinds, of the single node anchored at the
    /// marker: the node of such a kind overlapping the marker or listing it
    /// among its interpretation marker spans, or of several, the one whose
    /// interpretation marker spans include the marker.
    pub kind: String,
    /// Scalar fields of the anchored node; a dotted name such as
    /// `formatting.style_type` reads a field of a nested object. A numeric
    /// field reads fullwidth digits in its expected value as ASCII digits.
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    /// Text of the anchored node's child-node arrays. An expected value reads
    /// as printed text: a gaiji marker as its resolved character, or as
    /// nothing when it does not resolve, and other bracketed notation and
    /// ruby readings as nothing.
    #[serde(default)]
    pub text: BTreeMap<String, String>,
    /// Child-node array, or array of source spans, whose source extent must
    /// be the nearest occurrence of its text before the marker, ignoring
    /// bracketed notation.
    #[serde(default)]
    pub target_before_marker: Option<String>,
    /// Whether the anchored node's span must equal the marker's extent.
    #[serde(default)]
    pub marker_span: bool,
    /// Whether the text nodes overlapping the marker must hold its first and
    /// last characters and reproduce its source where they overlap it, line
    /// terminators aside. Such a check anchors no node, so `kind` and the
    /// other options do not apply.
    #[serde(default)]
    pub marker_text: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum CheckOutcome {
    Satisfied,
    /// The marker does not match `operands`, so the check does not apply to it.
    OutsideOperands,
    /// The marker is quoted inside another marker, whose interpretation covers it.
    NestedInMarker,
    NoNode,
    /// No node is anchored, and the text that the `target_before_marker`
    /// array quotes, ruby readings included, occurs nowhere in the source
    /// before the marker, so the source supplies no target to check.
    TargetAbsentFromSource {
        array: String,
    },
    AmbiguousNode {
        nodes: usize,
    },
    FieldMismatch {
        field: String,
        expected: String,
        actual: String,
    },
    TextMismatch {
        array: String,
        expected: String,
        actual: String,
    },
    TargetNotBeforeMarker {
        array: String,
    },
    SpanNotMarker {
        span: Option<(usize, usize)>,
    },
    TextNotMarker,
}

impl CheckOutcome {
    /// The outcome without its values, for counting.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Satisfied => "satisfied".to_owned(),
            Self::OutsideOperands => "outside-operands".to_owned(),
            Self::NestedInMarker => "nested-in-marker".to_owned(),
            Self::NoNode => "no-node".to_owned(),
            Self::TargetAbsentFromSource { array } => format!("target-absent-from-source:{array}"),
            Self::AmbiguousNode { .. } => "ambiguous-node".to_owned(),
            Self::FieldMismatch { field, .. } => format!("field-mismatch:{field}"),
            Self::TextMismatch { array, .. } => format!("text-mismatch:{array}"),
            Self::TargetNotBeforeMarker { array } => format!("target-not-before-marker:{array}"),
            Self::SpanNotMarker { .. } => "span-not-marker".to_owned(),
            Self::TextNotMarker => "text-not-marker".to_owned(),
        }
    }

    #[must_use]
    pub fn failed(&self) -> bool {
        !matches!(
            self,
            Self::Satisfied
                | Self::OutsideOperands
                | Self::NestedInMarker
                | Self::TargetAbsentFromSource { .. }
        )
    }
}

/// An occurrence's check outcome under a baseline parser against the
/// outcome under the candidate parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BaselineComparison {
    BothSatisfied,
    BothFailed,
    /// Failed under the baseline, satisfied under the candidate.
    Gained,
    /// Satisfied under the baseline, failed under the candidate.
    Lost,
}

impl BaselineComparison {
    /// `None` when either outcome leaves the occurrence unchecked.
    #[must_use]
    pub fn between(baseline: &CheckOutcome, candidate: &CheckOutcome) -> Option<Self> {
        let satisfied = |outcome: &CheckOutcome| match outcome {
            CheckOutcome::Satisfied => Some(true),
            outcome if outcome.failed() => Some(false),
            _ => None,
        };
        match (satisfied(baseline)?, satisfied(candidate)?) {
            (true, true) => Some(Self::BothSatisfied),
            (false, false) => Some(Self::BothFailed),
            (false, true) => Some(Self::Gained),
            (true, false) => Some(Self::Lost),
        }
    }
}

/// Occurrence counts of an obligation's check by [`BaselineComparison`].
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct Discrimination {
    pub both_satisfied: u64,
    pub both_failed: u64,
    pub gained: u64,
    pub lost: u64,
}

impl Discrimination {
    pub fn add(&mut self, comparison: BaselineComparison) {
        *match comparison {
            BaselineComparison::BothSatisfied => &mut self.both_satisfied,
            BaselineComparison::BothFailed => &mut self.both_failed,
            BaselineComparison::Gained => &mut self.gained,
            BaselineComparison::Lost => &mut self.lost,
        } += 1;
    }

    /// Whether any occurrence changed outcome between the parsers.
    #[must_use]
    pub fn flipped(&self) -> bool {
        self.gained + self.lost > 0
    }
}

/// A [`CorpusCheck`] with its operand pattern compiled.
#[derive(Debug, Clone)]
pub struct CompiledCheck {
    pub check: CorpusCheck,
    operands: Regex,
}

impl CompiledCheck {
    /// # Errors
    ///
    /// Returns an error when `operands` is not a valid pattern.
    pub fn new(check: CorpusCheck) -> Result<Self, regex::Error> {
        let operands = Regex::new(&check.operands)?;
        Ok(Self { check, operands })
    }

    /// Evaluate at the marker `raw` starting at byte `start` of `source`,
    /// the decoded text `aat` was parsed from.
    #[must_use]
    pub fn evaluate(&self, aat: &Value, source: &str, start: usize, raw: &str) -> CheckOutcome {
        let Some(captures) = self.operands.captures(raw) else {
            return CheckOutcome::OutsideOperands;
        };
        if nested_in_marker(source, start) {
            return CheckOutcome::NestedInMarker;
        }
        let end = start + raw.len();
        if self.check.marker_text {
            return if retains_marker_text(aat, source, start, end) {
                CheckOutcome::Satisfied
            } else {
                CheckOutcome::TextNotMarker
            };
        }
        let fill = |template: &str| {
            let mut filled = String::new();
            captures.expand(&template.replace('{', "${"), &mut filled);
            filled
        };
        let mut anchored = Vec::new();
        collect_kind(aat, &self.check.kind, start, end, &mut anchored);
        let node = match anchored.as_slice() {
            [] => {
                return match &self.check.target_before_marker {
                    Some(array)
                        if self.check.text.get(array).is_some_and(|template| {
                            source.get(..start).is_some_and(|before| {
                                !source_notation(before).contains(&source_notation(&fill(template)))
                            })
                        }) =>
                    {
                        CheckOutcome::TargetAbsentFromSource {
                            array: array.clone(),
                        }
                    }
                    _ => CheckOutcome::NoNode,
                };
            }
            [node] => *node,
            nodes => {
                let mut interpreting = nodes
                    .iter()
                    .filter(|node| interprets_marker(node, start, end));
                match (interpreting.next(), interpreting.next()) {
                    (Some(node), None) => *node,
                    _ => return CheckOutcome::AmbiguousNode { nodes: nodes.len() },
                }
            }
        };
        for (field, template) in &self.check.fields {
            let mut expected = fill(template);
            let actual = match field
                .split('.')
                .try_fold(node, |value, name| value.get(name))
            {
                Some(Value::String(value)) => value.clone(),
                Some(Value::Null) | None => String::new(),
                Some(value @ Value::Number(_)) => {
                    expected = ascii_digits(&expected);
                    value.to_string()
                }
                Some(value) => value.to_string(),
            };
            if actual != expected {
                return CheckOutcome::FieldMismatch {
                    field: field.clone(),
                    expected,
                    actual,
                };
            }
        }
        for (array, template) in &self.check.text {
            let expected = layer_text(&resolved_gaiji(&fill(template)), false);
            let actual = node.get(array).map(visible_text).unwrap_or_default();
            if actual != expected {
                return CheckOutcome::TextMismatch {
                    array: array.clone(),
                    expected,
                    actual,
                };
            }
        }
        if let Some(array) = &self.check.target_before_marker {
            let extent = node
                .get(array)
                .and_then(|target| target_extent(target, source));
            let nearest = extent.is_some_and(|(target, target_start, end)| {
                let in_reading = source
                    .get(..target_start)
                    .is_some_and(|before| before.rfind('《') > before.rfind('》'));
                end <= start
                    && source
                        .get(end..start)
                        .is_some_and(|between| !layer_text(between, in_reading).contains(&target))
            });
            if !nearest {
                return CheckOutcome::TargetNotBeforeMarker {
                    array: array.clone(),
                };
            }
        }
        if self.check.marker_span && span(node) != Some((start, end)) {
            return CheckOutcome::SpanNotMarker { span: span(node) };
        }
        CheckOutcome::Satisfied
    }
}

/// A corpus check bound to the obligation and syntax row that declare it.
#[derive(Debug, Clone)]
pub struct ObligationCheck {
    pub obligation_id: String,
    pub row_id: String,
    pub check: CompiledCheck,
}

impl ObligationCheck {
    #[must_use]
    pub fn applies(&self, occurrence: &LocatedOccurrence) -> bool {
        occurrence.row_id == self.row_id
            && self
                .check
                .check
                .partitions
                .iter()
                .all(|partition| occurrence.partitions.contains(partition))
    }
}

#[derive(Deserialize)]
struct RequirementsFile {
    family: Vec<FamilyRecord>,
}

#[derive(Deserialize)]
struct FamilyRecord {
    syntax_row_id: String,
    obligation: Vec<ObligationRecord>,
}

#[derive(Deserialize)]
struct ObligationRecord {
    id: String,
    corpus_check: Option<CorpusCheck>,
}

/// Every corpus check declared in a `fidelity-requirements.toml` document.
///
/// # Errors
///
/// Returns an error when the document does not parse or an operand pattern
/// does not compile.
pub fn obligation_checks(requirements: &str) -> Result<Vec<ObligationCheck>> {
    let file: RequirementsFile = toml::from_str(requirements).context("parse requirements")?;
    let mut checks = Vec::new();
    for family in file.family {
        for obligation in family.obligation {
            if let Some(check) = obligation.corpus_check {
                checks.push(ObligationCheck {
                    check: CompiledCheck::new(check)
                        .with_context(|| format!("operands of {}", obligation.id))?,
                    obligation_id: obligation.id,
                    row_id: family.syntax_row_id.clone(),
                });
            }
        }
    }
    Ok(checks)
}

/// Whether an annotation bracket ［＃ opened earlier on the marker's line is
/// still open at `start`. A literal ［ opens no annotation.
fn nested_in_marker(source: &str, start: usize) -> bool {
    let line = source
        .get(..start)
        .map(|before| &before[before.rfind('\n').map_or(0, |newline| newline + 1)..])
        .unwrap_or_default();
    let mut open = Vec::new();
    for (offset, character) in line.char_indices() {
        match character {
            '［' => open.push(line[offset + character.len_utf8()..].starts_with('＃')),
            '］' => {
                open.pop();
            }
            _ => {}
        }
    }
    open.contains(&true)
}

/// `text` with each gaiji marker replaced by its resolved character.
fn resolved_gaiji(text: &str) -> String {
    let mut resolved = String::new();
    let mut cursor = 0;
    for marker in gaiji::gaiji_resolutions(text) {
        resolved.push_str(&text[cursor..marker.start]);
        resolved.push_str(marker.resolved.as_deref().unwrap_or_default());
        cursor = marker.end;
    }
    resolved.push_str(&text[cursor..]);
    resolved
}

fn ascii_digits(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '０'..='９' => {
                char::from_u32(u32::from(character) - u32::from('０') + u32::from('0'))
                    .unwrap_or(character)
            }
            _ => character,
        })
        .collect()
}

/// Text of one notation layer: the printed text outside brackets and
/// readings, or, with `in_reading`, the reading text, `text` then starting
/// inside a reading.
fn layer_text(text: &str, in_reading: bool) -> String {
    let wanted = usize::from(in_reading);
    let mut depth = wanted;
    text.chars()
        .filter(|character| match character {
            '［' | '《' => {
                depth += 1;
                false
            }
            '］' | '》' => {
                depth = depth.saturating_sub(1);
                false
            }
            _ => depth == wanted,
        })
        .collect()
}

/// `text` without its ［＃…］ annotations and ｜ ruby-base delimiters, so that
/// a quoted reading such as `漢字《かんじ》` matches the source `｜漢字《かんじ》`.
fn source_notation(text: &str) -> String {
    let mut depth = 0_usize;
    let mut kept = String::new();
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '［' if characters.peek() == Some(&'＃') => depth += 1,
            '］' if depth > 0 => depth -= 1,
            '｜' => {}
            _ if depth == 0 => kept.push(character),
            _ => {}
        }
    }
    kept
}

/// See [`CorpusCheck::marker_text`].
fn retains_marker_text(aat: &Value, source: &str, start: usize, end: usize) -> bool {
    let mut texts = Vec::new();
    collect_kind(aat, "text", start, end, &mut texts);
    let last = source
        .get(start..end)
        .and_then(|marker| marker.chars().next_back())
        .map_or(end, |character| end - character.len_utf8());
    let holds = |at: usize| {
        texts.iter().any(|node| {
            span(node).is_some_and(|(node_start, node_end)| node_start <= at && at < node_end)
        })
    };
    holds(start)
        && holds(last)
        && texts.iter().all(|node| {
            let (Some((node_start, node_end)), Some(value)) =
                (span(node), node.get("value").and_then(Value::as_str))
            else {
                return false;
            };
            let from = start.max(node_start);
            let printed = |text: &str| -> String {
                text.chars()
                    .filter(|character| !matches!(character, '\r' | '\n'))
                    .collect()
            };
            let expected = printed(source.get(from..end.min(node_end)).unwrap_or_default());
            value
                .get(from - node_start..)
                .is_some_and(|rest| printed(rest).starts_with(&expected))
        })
}

fn interprets_marker(node: &Value, start: usize, end: usize) -> bool {
    node.get("interpretation_marker_spans")
        .and_then(Value::as_array)
        .is_some_and(|spans| {
            spans
                .iter()
                .any(|marker| offsets(marker) == Some((start, end)))
        })
}

fn span(value: &Value) -> Option<(usize, usize)> {
    offsets(value.get("span")?)
}

fn offsets(span: &Value) -> Option<(usize, usize)> {
    let offset = |name: &str| usize::try_from(span.get(name)?.as_u64()?).ok();
    Some((offset("byte_start")?, offset("byte_end")?))
}

/// The text, source start and source end of a target: the source covered by
/// an array of spans, or the visible text and extent of a node array.
fn target_extent(target: &Value, source: &str) -> Option<(String, usize, usize)> {
    let items = target.as_array()?;
    if !items.is_empty() && items.iter().all(|item| item.get("byte_start").is_some()) {
        let mut text = String::new();
        let mut start = usize::MAX;
        let mut end = 0;
        for item in items {
            let (item_start, item_end) = offsets(item)?;
            text.push_str(source.get(item_start..item_end)?);
            start = start.min(item_start);
            end = end.max(item_end);
        }
        return Some((text, start, end));
    }
    Some((
        visible_text(target),
        source_start(target)?,
        source_end(target)?,
    ))
}

/// Start of the source extent covered by a node array.
fn source_start(value: &Value) -> Option<usize> {
    value
        .as_array()?
        .iter()
        .filter_map(|node| {
            source_start(&node["content"]).or_else(|| span(node).map(|(start, _)| start))
        })
        .min()
}

fn collect_kind<'a>(
    value: &'a Value,
    kind: &str,
    start: usize,
    end: usize,
    out: &mut Vec<&'a Value>,
) {
    match value {
        Value::Object(object) => {
            if object
                .get("kind")
                .and_then(Value::as_str)
                .is_some_and(|node_kind| kind.split('|').any(|wanted| wanted == node_kind))
                && (span(value)
                    .is_some_and(|(node_start, node_end)| node_start < end && node_end > start)
                    || interprets_marker(value, start, end))
            {
                out.push(value);
            }
            for child in object.values() {
                collect_kind(child, kind, start, end, out);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_kind(child, kind, start, end, out);
            }
        }
        _ => {}
    }
}

/// Principal text of a node or node array: text values, ruby base nodes, resolved
/// gaiji, and the principal `content` of any other node.
fn visible_text(value: &Value) -> String {
    match value {
        Value::Array(values) => values.iter().map(visible_text).collect(),
        Value::Object(object) => match object.get("kind").and_then(Value::as_str) {
            Some("text") => object
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            Some("gaiji") => object
                .get("resolved")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            Some("ruby") => object
                .get("base_content")
                .or_else(|| object.get("base"))
                .map(visible_text)
                .unwrap_or_default(),
            _ => object.get("content").map(visible_text).unwrap_or_default(),
        },
        Value::String(text) => text.clone(),
        _ => String::new(),
    }
}

/// End of the source extent covered by a node array. A node with `content`
/// ends where its content ends: a text variant spans only its marker, which
/// can follow the enclosing range.
fn source_end(value: &Value) -> Option<usize> {
    value
        .as_array()?
        .iter()
        .filter_map(|node| source_end(&node["content"]).or_else(|| span(node).map(|(_, end)| end)))
        .max()
}
