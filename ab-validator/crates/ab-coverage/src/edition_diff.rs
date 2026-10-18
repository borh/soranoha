use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{Map, Value};

use ab_encoding::hex_sha256;

use crate::{
    occurrence_alignment::LocatedOccurrence,
    source_inventory::{
        SourceInventoryPattern, SourceInventorySummary, compile_patterns, matching_rows,
        normalize_marker_form,
    },
};

/// SHA-256 of the AAT without `meta.adapter_version`, which identifies the
/// parser build rather than describing the source.
#[must_use]
pub fn content_sha256(aat: &Value) -> String {
    let mut content = aat.clone();
    if let Some(meta) = content.get_mut("meta").and_then(Value::as_object_mut) {
        meta.remove("adapter_version");
    }
    hex_sha256(&serde_json::to_vec(&content).expect("JSON values serialize"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Side {
    Baseline,
    Candidate,
}

/// An AAT node present on one side only, compared without its child nodes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct ChangedNode {
    pub side: Side,
    pub kind: String,
    pub spans: Vec<(usize, usize)>,
    pub node: String,
    /// Source extent of the node and every node under it.
    pub extent: Option<(usize, usize)>,
}

fn is_node(value: &Value) -> bool {
    value.get("kind").is_some_and(Value::is_string)
}

fn holds_nodes(value: &Value) -> bool {
    match value {
        Value::Object(_) => is_node(value),
        Value::Array(values) => values.iter().any(is_node),
        _ => false,
    }
}

fn source_span(value: &Value) -> Option<(usize, usize)> {
    let field = |name: &str| {
        value
            .get(name)
            .and_then(Value::as_u64)
            .and_then(|offset| usize::try_from(offset).ok())
    };
    field("byte_start")
        .zip(field("byte_end"))
        .or_else(|| field("start").zip(field("end")))
}

fn node_spans(object: &Map<String, Value>) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    for (key, value) in object {
        if key.ends_with("span") {
            spans.extend(source_span(value));
        } else if key.ends_with("spans") {
            spans.extend(
                value
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(source_span),
            );
        }
    }
    spans.sort_unstable();
    spans.dedup();
    spans
}

struct ShallowNode {
    kind: String,
    spans: Vec<(usize, usize)>,
    node: String,
    extent: Option<(usize, usize)>,
}

fn union(a: Option<(usize, usize)>, b: Option<(usize, usize)>) -> Option<(usize, usize)> {
    match (a, b) {
        (Some(a), Some(b)) => Some((a.0.min(b.0), a.1.max(b.1))),
        (a, b) => a.or(b),
    }
}

/// Every AAT object with a string `kind`, each serialized without the fields
/// that hold further nodes, so a change is reported once, at the node that
/// changed. A node without source spans takes those of its nearest spanned ancestor.
fn shallow_nodes(aat: &Value) -> Vec<ShallowNode> {
    let mut nodes = Vec::new();
    collect_shallow(aat, &[], &mut nodes);
    nodes
}

/// Collect the shallow nodes under `value`; the extent returned covers every
/// span of every node under it.
fn collect_shallow(
    value: &Value,
    inherited: &[(usize, usize)],
    nodes: &mut Vec<ShallowNode>,
) -> Option<(usize, usize)> {
    match value {
        Value::Object(object) => {
            let kind = object.get("kind").and_then(Value::as_str);
            let own = kind.map(|_| node_spans(object)).unwrap_or_default();
            let spans = if own.is_empty() {
                inherited.to_vec()
            } else {
                own
            };
            let index = kind.map(|kind| {
                let shallow = object
                    .iter()
                    .filter(|(_, value)| !holds_nodes(value))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect::<Map<_, _>>();
                nodes.push(ShallowNode {
                    kind: kind.to_owned(),
                    spans: spans.clone(),
                    node: Value::Object(shallow).to_string(),
                    extent: None,
                });
                nodes.len() - 1
            });
            let mut extent = kind.and_then(|_| {
                spans
                    .iter()
                    .copied()
                    .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
            });
            for child in object.values() {
                extent = union(extent, collect_shallow(child, &spans, nodes));
            }
            if let Some(index) = index {
                nodes[index].extent = extent;
            }
            extent
        }
        Value::Array(values) => values
            .iter()
            .map(|child| collect_shallow(child, inherited, nodes))
            .fold(None, union),
        _ => None,
    }
}

/// The multiset difference between the two documents' shallow nodes.
#[must_use]
pub fn changed_nodes(baseline: &Value, candidate: &Value) -> Vec<ChangedNode> {
    let mut counts =
        BTreeMap::<(String, Vec<(usize, usize)>), (i64, String, Option<(usize, usize)>)>::new();
    for (sign, document) in [(1, baseline), (-1, candidate)] {
        for ShallowNode {
            kind,
            spans,
            node,
            extent,
        } in shallow_nodes(document)
        {
            let entry = counts.entry((node, spans)).or_insert((0, kind, extent));
            entry.0 += sign;
            entry.2 = union(entry.2, extent);
        }
    }
    let mut changed = Vec::new();
    for ((node, spans), (count, kind, extent)) in counts {
        let side = if count > 0 {
            Side::Baseline
        } else {
            Side::Candidate
        };
        for _ in 0..count.unsigned_abs() {
            changed.push(ChangedNode {
                side,
                kind: kind.clone(),
                spans: spans.clone(),
                node: node.clone(),
                extent,
            });
        }
    }
    changed.sort();
    changed
}

/// Rows and row forms a change is expected to touch.
#[derive(Debug, Clone, Default)]
pub struct Targets {
    pub rows: BTreeSet<String>,
    pub forms: BTreeSet<(String, String)>,
}

impl Targets {
    #[must_use]
    pub fn admits(&self, occurrence: &LocatedOccurrence) -> bool {
        self.rows.contains(&occurrence.row_id)
            || occurrence.form.as_ref().is_some_and(|form| {
                self.forms
                    .contains(&(occurrence.row_id.clone(), form.clone()))
            })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attribution {
    /// Changed nodes touching each (row, form); a form-less occurrence has an empty form.
    pub forms: BTreeMap<(String, String), u64>,
    /// Changed nodes touching no target occurrence.
    pub unexplained: Vec<ChangedNode>,
}

fn touches(span: (usize, usize), start: usize, end: usize) -> bool {
    span.0 <= end && span.1 >= start
}

/// Spans of an explained changed node and the (row, form) keys that explain it.
type ExplainedNode = (Vec<(usize, usize)>, Vec<(String, String)>);

/// The spans a changed node reaches: its subtree extent, or its own spans.
fn reach(node: &ChangedNode) -> Vec<(usize, usize)> {
    node.extent
        .map_or_else(|| node.spans.clone(), |extent| vec![extent])
}

fn overlaps(span: (usize, usize), other: (usize, usize)) -> bool {
    span.0 < other.1 && span.1 > other.0
}

/// Attribute each changed node to the source occurrences its spans overlap or
/// abut: a marker that becomes typed also moves the boundaries of the text next
/// to it. A node whose spans overlap the extent of an explained node is
/// explained the same way: a typed marker that takes earlier text as its
/// target splits the text node that held it.
#[must_use]
pub fn attribute(
    changed: Vec<ChangedNode>,
    occurrences: &[LocatedOccurrence],
    targets: &Targets,
) -> Attribution {
    let mut attribution = Attribution::default();
    let mut explained: Vec<ExplainedNode> = Vec::new();
    let mut pending = Vec::new();
    for node in changed {
        let mut keys = Vec::new();
        let mut admitted = false;
        for occurrence in occurrences {
            let (start, end) = (
                occurrence.byte_start,
                occurrence.byte_start + occurrence.occurrence.raw.len(),
            );
            if node.spans.iter().any(|span| touches(*span, start, end)) {
                let key = (
                    occurrence.row_id.clone(),
                    occurrence.form.clone().unwrap_or_default(),
                );
                *attribution.forms.entry(key.clone()).or_default() += 1;
                if targets.admits(occurrence) {
                    admitted = true;
                    keys.push(key);
                }
            }
        }
        if admitted {
            explained.push((reach(&node), keys));
        } else {
            pending.push(node);
        }
    }
    loop {
        let mut still_pending = Vec::new();
        let mut newly = Vec::new();
        for node in pending {
            let keys = explained.iter().find_map(|(spans, keys)| {
                node.spans
                    .iter()
                    .any(|span| spans.iter().any(|other| overlaps(*span, *other)))
                    .then(|| keys.clone())
            });
            match keys {
                Some(keys) => {
                    for key in &keys {
                        *attribution.forms.entry(key.clone()).or_default() += 1;
                    }
                    newly.push((reach(&node), keys));
                }
                None => still_pending.push(node),
            }
        }
        pending = still_pending;
        if newly.is_empty() {
            break;
        }
        explained.extend(newly);
    }
    attribution.unexplained = pending;
    attribution
}

/// Front- and back-matter markers located under the syntax rows their text
/// matches; the inventory counts them apart from body syntax, but a parser
/// change reaches them all the same.
#[must_use]
pub fn region_occurrences(
    summary: &SourceInventorySummary,
    patterns: &[SourceInventoryPattern],
) -> Vec<LocatedOccurrence> {
    let compiled = compile_patterns(patterns);
    summary
        .classified_region_markers
        .iter()
        .filter_map(|marker| {
            let byte_start = marker.occurrence.byte_start?;
            Some(
                matching_rows(&marker.occurrence.raw, &compiled)
                    .into_iter()
                    .map(move |row_id| LocatedOccurrence {
                        row_id,
                        form: Some(normalize_marker_form(&marker.occurrence.raw)),
                        partitions: BTreeSet::new(),
                        occurrence: marker.occurrence.clone(),
                        byte_start,
                    }),
            )
        })
        .flatten()
        .collect()
}
