use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::source_inventory::{SourceInventorySummary, SourceMarkerOccurrence};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AlignmentNode {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<(usize, usize)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marker_spans: Vec<(usize, usize)>,
    /// The source a raw node retains when it records an interpretation problem.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uncertain_source: Option<String>,
    /// Whether a text node's value is the source of its span, line endings
    /// normalized.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub literal: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AlignmentStatus {
    RawPreserved,
    Unaligned,
    Unlocated,
    ParseFailed,
    Typed,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FormClass {
    ParserSemanticGap,
    ParserAlignmentReview,
    FixtureCovered,
    AliasOfCovered,
    TypedPartitionUncovered,
}

/// One selected-row source occurrence with the form and semantic partitions
/// the inventory assigned to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocatedOccurrence {
    pub row_id: String,
    pub form: Option<String>,
    pub partitions: BTreeSet<String>,
    pub occurrence: SourceMarkerOccurrence,
    pub byte_start: usize,
}

fn span_of(value: &Value) -> Option<(usize, usize)> {
    let start = value.get("byte_start")?.as_u64()?;
    let end = value.get("byte_end")?.as_u64()?;
    Some((usize::try_from(start).ok()?, usize::try_from(end).ok()?))
}

/// Source spans of the notation a text variant's quoted target establishes,
/// such as a warichu marker inside the quoted text of a correction note.
fn quoted_interpretation_spans(
    object: &serde_json::Map<String, Value>,
) -> impl Iterator<Item = (usize, usize)> + '_ {
    object
        .get("current_interpretation_facts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|fact| {
            let span = fact.get("source_span")?;
            let offset = |name| usize::try_from(span.get(name)?.as_u64()?).ok();
            Some((offset("start")?, offset("end")?))
        })
}

/// Whether a text node is the line break that a ［＃改行］ marker yields; its
/// span is the extent of the marker.
fn is_forced_line_break(kind: &str, object: &serde_json::Map<String, Value>) -> bool {
    kind == "text"
        && object.get("x-break-kind").and_then(Value::as_str) == Some("line")
        && object.get("x-break-marker").and_then(Value::as_str) == Some("forced")
}

/// Whether an annotated text or an editorial note without marker spans spans
/// only the note marker that follows its target, so its span is the extent of
/// that marker. The target is the annotated text's content or the note's
/// target source spans.
fn is_trailing_note_marker(kind: &str, object: &serde_json::Map<String, Value>) -> bool {
    let targets = match kind {
        "annotated_text" => object
            .get("content")
            .and_then(Value::as_array)
            .map(|nodes| {
                nodes
                    .iter()
                    .map(|node| node.get("span").and_then(span_of))
                    .collect::<Vec<_>>()
            }),
        "editorial_note" => object
            .get("target_source_spans")
            .and_then(Value::as_array)
            .map(|spans| spans.iter().map(span_of).collect()),
        _ => None,
    };
    let (Some(span), Some(targets)) = (object.get("span").and_then(span_of), targets) else {
        return false;
    };
    !object.contains_key("interpretation_marker_spans")
        && !targets.is_empty()
        && targets
            .iter()
            .all(|target| target.is_some_and(|(_, end)| end <= span.0))
}

/// Flatten every AAT object carrying a string `kind` into alignment nodes;
/// `source` is the decoded text `aat` was parsed from.
#[must_use]
pub fn alignment_index(aat: &Value, source: &str) -> Vec<AlignmentNode> {
    let mut nodes = Vec::new();
    let mut stack = vec![aat];
    while let Some(value) = stack.pop() {
        match value {
            Value::Object(object) => {
                if let Some(kind) = object.get("kind").and_then(Value::as_str) {
                    let span = object.get("span").and_then(span_of);
                    let marker_spans: Vec<_> = object
                        .get("interpretation_marker_spans")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(span_of)
                        .chain(quoted_interpretation_spans(object))
                        .chain(span.filter(|_| {
                            is_forced_line_break(kind, object)
                                || is_trailing_note_marker(kind, object)
                        }))
                        .collect();
                    let uncertain_source = (kind == "raw"
                        && object
                            .get("interpretation_problem")
                            .is_some_and(Value::is_object))
                    .then(|| object.get("source").and_then(Value::as_str))
                    .flatten()
                    .map(str::to_owned);
                    let literal = kind == "text"
                        && marker_spans.is_empty()
                        && span
                            .and_then(|(start, end)| source.get(start..end))
                            .zip(object.get("value").and_then(Value::as_str))
                            .is_some_and(|(text, value)| {
                                text.replace("\r\n", "\n").replace('\r', "\n") == value
                            });
                    nodes.push(AlignmentNode {
                        kind: kind.to_owned(),
                        span,
                        marker_spans,
                        uncertain_source,
                        literal,
                    });
                    if kind == "raw" {
                        for sidecar in ["text_apparatus", "text_variant"] {
                            if object.get(sidecar).is_some_and(Value::is_object) {
                                nodes.push(AlignmentNode {
                                    kind: sidecar.to_owned(),
                                    span,
                                    marker_spans: Vec::new(),
                                    uncertain_source: None,
                                    literal: false,
                                });
                            }
                        }
                    }
                }
                stack.extend(object.values());
            }
            Value::Array(values) => stack.extend(values),
            _ => {}
        }
    }
    nodes
}

fn overlaps(span: (usize, usize), start: usize, end: usize) -> bool {
    span.0 < end && span.1 > start
}

/// Classify the AAT evidence for the source bytes `start..end`.
///
/// A typed node whose span or marker span is exactly the occurrence types it.
/// Otherwise a raw node with exactly that span retains it, even inside a wider
/// typed marker such as a ruby reading; otherwise an overlapping formatting
/// instruction types it, an overlapping raw node that records an
/// interpretation problem and adjoins an overlapping typed marker span, as the
/// unresolved clause of a compound opener does, retains it, overlapping typed
/// markers type it and other overlapping raw nodes retain it.
#[must_use]
pub fn alignment_status(nodes: &[AlignmentNode], start: usize, end: usize) -> AlignmentStatus {
    let semantic =
        |node: &AlignmentNode| !matches!(node.kind.as_str(), "text" | "paragraph" | "raw");
    let exact = |span: &(usize, usize)| *span == (start, end);
    if nodes.iter().any(|node| {
        (node.kind != "raw" && node.marker_spans.iter().any(exact))
            || (semantic(node) && node.span.as_ref().is_some_and(exact))
    }) {
        return AlignmentStatus::Typed;
    }
    if nodes
        .iter()
        .any(|node| node.kind == "raw" && node.span.as_ref().is_some_and(exact))
    {
        return AlignmentStatus::RawPreserved;
    }
    let typed_markers: Vec<(usize, usize)> = nodes
        .iter()
        .filter(|node| node.kind != "raw")
        .flat_map(|node| node.marker_spans.iter().copied())
        .filter(|span| overlaps(*span, start, end))
        .collect();
    let (mut problem_raw, mut raw) = (false, false);
    for node in nodes {
        let marker = node
            .marker_spans
            .iter()
            .any(|span| overlaps(*span, start, end));
        let span_overlaps = node.span.is_some_and(|span| overlaps(span, start, end));
        if node.kind == "formatting_instruction" && span_overlaps {
            return AlignmentStatus::Typed;
        }
        if node.kind == "raw" && (marker || span_overlaps) {
            raw = true;
            problem_raw |= node.uncertain_source.is_some()
                && node.span.is_some_and(|(raw_start, raw_end)| {
                    typed_markers.iter().any(|&(marker_start, marker_end)| {
                        marker_end == raw_start || marker_start == raw_end
                    })
                });
        }
    }
    let typed_marker = !typed_markers.is_empty();
    if problem_raw {
        AlignmentStatus::RawPreserved
    } else if typed_marker {
        AlignmentStatus::Typed
    } else if raw {
        AlignmentStatus::RawPreserved
    } else {
        AlignmentStatus::Unaligned
    }
}

/// Whether the source bytes `start..end`, whose text is `raw`, meet the
/// obligation of a form ruled undecidable: no typed node interprets any part
/// of them, and each byte is kept as literal text, in the source of a raw node
/// recording an interpretation problem, or in a typed node's marker span that
/// no such raw node overlaps.
#[must_use]
pub fn retains_uncertain_source(
    nodes: &[AlignmentNode],
    start: usize,
    end: usize,
    raw: &str,
) -> bool {
    let interprets_part = |node: &AlignmentNode| {
        !matches!(node.kind.as_str(), "text" | "paragraph" | "raw")
            && node.span.is_some_and(|span| {
                overlaps(span, start, end) && !(span.0 <= start && end <= span.1)
            })
    };
    if alignment_status(nodes, start, end) != AlignmentStatus::RawPreserved
        || nodes.iter().any(interprets_part)
    {
        return false;
    }
    let mut kept = vec![false; end - start];
    for node in nodes {
        let Some((node_start, node_end)) = node.span else {
            continue;
        };
        let (from, to) = (node_start.max(start), node_end.min(end));
        if from >= to {
            continue;
        }
        let retained = node.literal
            || node.uncertain_source.as_ref().is_some_and(|source| {
                source.len() == node_end - node_start
                    && source.get(from - node_start..to - node_start)
                        == raw.get(from - start..to - start)
            });
        if retained {
            kept[from - start..to - start].fill(true);
        }
    }
    let problem_spans: Vec<_> = nodes
        .iter()
        .filter(|node| node.uncertain_source.is_some())
        .filter_map(|node| node.span)
        .collect();
    for (marker_start, marker_end) in nodes
        .iter()
        .filter(|node| node.kind != "raw")
        .flat_map(|node| &node.marker_spans)
    {
        for byte in (*marker_start).max(start)..(*marker_end).min(end) {
            if !problem_spans
                .iter()
                .any(|&(raw_start, raw_end)| raw_start <= byte && byte < raw_end)
            {
                kept[byte - start] = true;
            }
        }
    }
    kept.into_iter().all(|byte_kept| byte_kept)
}

/// Join a full-occurrence inventory's variant and partition samples into one
/// record per located occurrence of each selected row.
///
/// The summary must come from `inventory_document_with_all_variant_occurrences`.
#[must_use]
pub fn located_occurrences(
    summary: &SourceInventorySummary,
    rows: &[String],
) -> Vec<LocatedOccurrence> {
    type Located = BTreeMap<(String, usize, String), LocatedOccurrence>;
    fn entry<'a>(
        located: &'a mut Located,
        row_id: &str,
        occurrence: &SourceMarkerOccurrence,
    ) -> &'a mut LocatedOccurrence {
        let byte_start = occurrence
            .byte_start
            .expect("full-occurrence inventory samples carry byte_start");
        located
            .entry((row_id.to_owned(), byte_start, occurrence.raw.clone()))
            .or_insert_with(|| LocatedOccurrence {
                row_id: row_id.to_owned(),
                form: None,
                partitions: BTreeSet::new(),
                occurrence: occurrence.clone(),
                byte_start,
            })
    }
    let mut located = Located::new();
    for row_id in rows {
        for (form, variant) in summary.row_variants.get(row_id).into_iter().flatten() {
            for occurrence in &variant.samples {
                entry(&mut located, row_id, occurrence).form = Some(form.clone());
            }
        }
        for (partition, partition_summary) in summary
            .row_semantic_partitions
            .get(row_id)
            .into_iter()
            .flatten()
        {
            for occurrence in &partition_summary.samples {
                entry(&mut located, row_id, occurrence)
                    .partitions
                    .insert(partition.clone());
            }
        }
    }
    located.into_values().collect()
}

/// Classify a normalized form from the alignment statuses of all its corpus
/// occurrences. A raw-preserved occurrence outranks every other outcome.
#[must_use]
pub fn form_class(
    statuses: &BTreeMap<AlignmentStatus, u64>,
    form_partitions: &BTreeSet<String>,
    fixture_covered: bool,
    covered_partitions: &BTreeSet<String>,
) -> FormClass {
    let seen = |status| statuses.get(&status).is_some_and(|count| *count > 0);
    if seen(AlignmentStatus::RawPreserved) {
        FormClass::ParserSemanticGap
    } else if seen(AlignmentStatus::Unaligned)
        || seen(AlignmentStatus::Unlocated)
        || seen(AlignmentStatus::ParseFailed)
    {
        FormClass::ParserAlignmentReview
    } else if fixture_covered {
        FormClass::FixtureCovered
    } else if !form_partitions.is_empty() && form_partitions.is_subset(covered_partitions) {
        FormClass::AliasOfCovered
    } else {
        FormClass::TypedPartitionUncovered
    }
}
