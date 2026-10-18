use std::collections::BTreeMap;
use std::sync::OnceLock;

use ab_aozora_syntax::accent::{decompose_fragment_sites, unresolved_accent_sites};
use ab_source_syntax::{SourceMarker, SourceMarkerKind, comparison_lossy_body, source_markers};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::matrix::Row;

/// [`ab_source_syntax::aozora_body_range`] over `source` read as the parser's
/// sanitized text, in which a bare CR ends a line. Replacing a bare CR by a
/// line feed keeps every byte offset.
fn sanitized_body_range(source: &str) -> (std::ops::Range<usize>, usize) {
    let bare_cr =
        |bytes: &[u8], index: usize| bytes[index] == b'\r' && bytes.get(index + 1) != Some(&b'\n');
    let bytes = source.as_bytes();
    if !(0..bytes.len()).any(|index| bare_cr(bytes, index)) {
        return ab_source_syntax::aozora_body_range(source);
    }
    let mut lines = bytes.to_vec();
    for index in 0..lines.len() {
        if bare_cr(&lines, index) {
            lines[index] = b'\n';
        }
    }
    let lines = String::from_utf8(lines).expect("replacing an ASCII byte keeps UTF-8");
    ab_source_syntax::aozora_body_range(&lines)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceInventoryPattern {
    pub row_id: String,
    pub source_patterns: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SourceInventorySummary {
    pub work_id: String,
    /// All lexical source occurrences, including editorial examples.
    pub markers_total: u64,
    pub row_counts: BTreeMap<String, MarkerClassSummary>,
    /// Normalized lexical forms for rows explicitly selected by the caller.
    pub row_variants: BTreeMap<String, BTreeMap<String, MarkerVariantSummary>>,
    /// Source-context partitions for rows explicitly selected by the caller.
    pub row_semantic_partitions: BTreeMap<String, BTreeMap<String, MarkerSemanticPartitionSummary>>,
    pub unknown_examples: Vec<SourceMarkerOccurrence>,
    pub source_region_events: SourceRegionEventSummary,
    /// Full source occurrences assigned to packaging or explicit boundaries,
    /// excluded from body-family and unreviewed-marker counts.
    pub classified_region_markers: Vec<ClassifiedRegionMarker>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct MarkerClassSummary {
    pub occurrences: u64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct MarkerVariantSummary {
    pub occurrences: u64,
    pub samples: Vec<SourceMarkerOccurrence>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub semantic_partitions: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct MarkerSemanticPartitionSummary {
    pub occurrences: u64,
    pub samples: Vec<SourceMarkerOccurrence>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SourceRegionEventSummary {
    pub terminal_provenance_occurrences: u64,
    pub front_matter_occurrences: u64,
    pub body_end_boundary_occurrences: u64,
    pub back_matter_occurrences: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SourceMarkerRegion {
    FrontMatter,
    BodyEndBoundary,
    BackMatter,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ClassifiedRegionMarker {
    pub region: SourceMarkerRegion,
    pub occurrence: SourceMarkerOccurrence,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SourceMarkerOccurrence {
    pub work_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexed_path: Option<String>,
    pub line: usize,
    pub kind: String,
    pub raw: String,
    pub body: String,
    /// Byte offset of `raw` in the decoded text that was scanned; set only on
    /// recorded variant and semantic-partition samples and region markers.
    #[serde(skip)]
    pub byte_start: Option<usize>,
}

#[must_use]
pub fn patterns_from_rows(rows: &[Row]) -> Vec<SourceInventoryPattern> {
    rows.iter()
        .map(|row| SourceInventoryPattern {
            row_id: row.id.clone(),
            source_patterns: row.source_patterns.clone(),
        })
        .collect()
}

#[must_use]
pub fn inventory_document(
    work_id: &str,
    text: &str,
    patterns: &[SourceInventoryPattern],
) -> SourceInventorySummary {
    inventory_document_with_variants(work_id, text, patterns, &[])
}

#[must_use]
pub fn inventory_document_with_variants(
    work_id: &str,
    text: &str,
    patterns: &[SourceInventoryPattern],
    variant_rows: &[String],
) -> SourceInventorySummary {
    inventory_document_sampled(work_id, text, patterns, variant_rows, 3)
}

/// Like [`inventory_document_with_variants`], but every selected-row variant
/// and semantic-partition occurrence is retained as a sample with its
/// `byte_start`.
#[must_use]
pub fn inventory_document_with_all_variant_occurrences(
    work_id: &str,
    text: &str,
    patterns: &[SourceInventoryPattern],
    variant_rows: &[String],
) -> SourceInventorySummary {
    inventory_document_sampled(work_id, text, patterns, variant_rows, usize::MAX)
}

fn push_sample(
    samples: &mut Vec<SourceMarkerOccurrence>,
    occurrence: &SourceMarkerOccurrence,
    byte_start: usize,
    sample_limit: usize,
) {
    if samples.len() < sample_limit {
        samples.push(SourceMarkerOccurrence {
            byte_start: Some(byte_start),
            ..occurrence.clone()
        });
    }
}

fn inventory_document_sampled(
    work_id: &str,
    text: &str,
    patterns: &[SourceInventoryPattern],
    variant_rows: &[String],
    sample_limit: usize,
) -> SourceInventorySummary {
    let mut variants = BTreeMap::<String, BTreeMap<String, MarkerVariantSummary>>::new();
    let mut semantic_partitions =
        BTreeMap::<String, BTreeMap<String, MarkerSemanticPartitionSummary>>::new();
    let compiled_patterns = compile_patterns(patterns);
    let mut summary =
        inventory_document_observed(work_id, text, patterns, |marker, region, rows| {
            if region.is_some() {
                return;
            }
            for row_id in rows {
                if !variant_rows.contains(row_id) {
                    continue;
                }
                if matches!(
                    row_id.as_str(),
                    "iteration.kunoji" | "gaiji.marker" | "gaiji.jis_code"
                ) {
                    continue;
                }
                let matched = if matches!(
                    row_id.as_str(),
                    "indentation.jisage_block" | "source.bracket_note"
                ) {
                    vec![MatchedVariantOccurrence {
                        occurrence: SourceMarkerOccurrence {
                            work_id: work_id.to_owned(),
                            indexed_path: None,
                            line: marker.span.line,
                            kind: format!("{:?}", marker.kind),
                            raw: marker.raw.to_owned(),
                            body: marker.body.to_owned(),
                            byte_start: None,
                        },
                        start: marker.span.start,
                    }]
                } else {
                    matching_variant_occurrences(marker, work_id, row_id, &compiled_patterns, 0, 0)
                };
                for matched in matched {
                    let start = matched.start;
                    let occurrence = matched.occurrence;
                    let form = normalize_marker_form(&occurrence.raw);
                    let variant = variants
                        .entry(row_id.clone())
                        .or_default()
                        .entry(form)
                        .or_default();
                    variant.occurrences += 1;
                    push_sample(&mut variant.samples, &occurrence, start, sample_limit);
                }
            }
        });
    for row_id in variant_rows {
        if matches!(
            row_id.as_str(),
            "iteration.kunoji" | "gaiji.marker" | "gaiji.jis_code" | "ruby.nested_forbidden"
        ) {
            continue;
        }
        let Some(occurrences) =
            semantic_partition_occurrences(row_id, text, work_id, &compiled_patterns)
        else {
            continue;
        };
        for (start, partition, occurrence) in occurrences {
            if row_id == "ruby.basic"
                && let Some(form) = ruby_marker_form(&partition)
            {
                let variant = variants
                    .entry(row_id.clone())
                    .or_default()
                    .entry(form.to_owned())
                    .or_default();
                variant.occurrences += 1;
                push_sample(&mut variant.samples, &occurrence, start, sample_limit);
            }
            if matches!(
                row_id.as_str(),
                "kunten.kaeriten"
                    | "layout.tcy"
                    | "layout.rotation"
                    | "decoration.bousen"
                    | "decoration.font_size"
                    | "annotation.chuuki"
                    | "indentation.chitsuki"
                    | "layout.cross_line_head_alignment"
            ) {
                let form = normalize_marker_form(&occurrence.raw);
                if let Some(variant) = variants
                    .get_mut(row_id)
                    .and_then(|variants| variants.get_mut(&form))
                {
                    *variant
                        .semantic_partitions
                        .entry(partition.clone())
                        .or_default() += 1;
                }
            }
            let summary = semantic_partitions
                .entry(row_id.clone())
                .or_default()
                .entry(partition)
                .or_default();
            summary.occurrences += 1;
            push_sample(&mut summary.samples, &occurrence, start, sample_limit);
        }
    }
    if variant_rows.iter().any(|row| row == "iteration.kunoji") {
        let iteration_partitions = iteration_semantic_partitions(text, work_id);
        let mut occurrence_count = 0_u64;
        for (start, partition, occurrence) in iteration_partitions {
            if partition.starts_with("form:") {
                occurrence_count += 1;
                let form = occurrence.raw.clone();
                let variant = variants
                    .entry("iteration.kunoji".to_owned())
                    .or_default()
                    .entry(form)
                    .or_default();
                variant.occurrences += 1;
                push_sample(&mut variant.samples, &occurrence, start, sample_limit);
            }
            let summary = semantic_partitions
                .entry("iteration.kunoji".to_owned())
                .or_default()
                .entry(partition)
                .or_default();
            summary.occurrences += 1;
            push_sample(&mut summary.samples, &occurrence, start, sample_limit);
        }
        if occurrence_count == 0 {
            summary.row_counts.remove("iteration.kunoji");
        } else {
            summary
                .row_counts
                .entry("iteration.kunoji".to_owned())
                .or_default()
                .occurrences = occurrence_count;
        }
    }
    if variant_rows.iter().any(|row| row == "gaiji.marker") {
        let gaiji_partitions = gaiji_semantic_partitions(text, work_id);
        let mut occurrence_count = 0_u64;
        for (start, partition, occurrence) in gaiji_partitions {
            if let Some(form) = partition.strip_prefix("marker-syntax:") {
                occurrence_count += 1;
                let form = match form {
                    "fullwidth" => "※［＃…］",
                    "ascii" => "※[#…]",
                    _ => unreachable!("gaiji marker syntax is closed"),
                };
                let variant = variants
                    .entry("gaiji.marker".to_owned())
                    .or_default()
                    .entry(form.to_owned())
                    .or_default();
                variant.occurrences += 1;
                push_sample(&mut variant.samples, &occurrence, start, sample_limit);
            }
            let partition_summary = semantic_partitions
                .entry("gaiji.marker".to_owned())
                .or_default()
                .entry(partition)
                .or_default();
            partition_summary.occurrences += 1;
            push_sample(
                &mut partition_summary.samples,
                &occurrence,
                start,
                sample_limit,
            );
        }
        if occurrence_count == 0 {
            summary.row_counts.remove("gaiji.marker");
        } else {
            summary
                .row_counts
                .entry("gaiji.marker".to_owned())
                .or_default()
                .occurrences = occurrence_count;
        }
    }
    if variant_rows.iter().any(|row| row == "gaiji.jis_code") {
        let mut occurrence_count = 0_u64;
        for (start, form, partitions, occurrence) in jis_gaiji_semantic_occurrences(text, work_id) {
            occurrence_count += 1;
            let variant = variants
                .entry("gaiji.jis_code".to_owned())
                .or_default()
                .entry(form)
                .or_default();
            variant.occurrences += 1;
            push_sample(&mut variant.samples, &occurrence, start, sample_limit);
            for partition in partitions {
                *variant
                    .semantic_partitions
                    .entry(partition.clone())
                    .or_default() += 1;
                let partition_summary = semantic_partitions
                    .entry("gaiji.jis_code".to_owned())
                    .or_default()
                    .entry(partition)
                    .or_default();
                partition_summary.occurrences += 1;
                push_sample(
                    &mut partition_summary.samples,
                    &occurrence,
                    start,
                    sample_limit,
                );
            }
        }
        if occurrence_count == 0 {
            summary.row_counts.remove("gaiji.jis_code");
        } else {
            summary
                .row_counts
                .entry("gaiji.jis_code".to_owned())
                .or_default()
                .occurrences = occurrence_count;
        }
    }
    for (start, _, occurrence) in nested_reading_ruby_occurrences(text, work_id, &compiled_patterns)
    {
        summary
            .row_counts
            .entry("ruby.nested_forbidden".to_owned())
            .or_default()
            .occurrences += 1;
        if variant_rows
            .iter()
            .any(|row| row == "ruby.nested_forbidden")
        {
            let form = normalize_marker_form(&occurrence.raw);
            let variant = variants
                .entry("ruby.nested_forbidden".to_owned())
                .or_default()
                .entry(form)
                .or_default();
            variant.occurrences += 1;
            push_sample(&mut variant.samples, &occurrence, start, sample_limit);
            for partition in [
                "form:nested-reading",
                "boundary:matched",
                "reading-content:ruby-delimiters",
            ] {
                let partition = semantic_partitions
                    .entry("ruby.nested_forbidden".to_owned())
                    .or_default()
                    .entry(partition.to_owned())
                    .or_default();
                partition.occurrences += 1;
                push_sample(&mut partition.samples, &occurrence, start, sample_limit);
            }
        }
    }
    for row_id in variant_rows {
        let occurrence_count = variants
            .get(row_id)
            .into_iter()
            .flat_map(|row_variants| row_variants.values())
            .map(|variant| variant.occurrences)
            .sum();
        if occurrence_count == 0 {
            summary.row_counts.remove(row_id);
        } else {
            summary
                .row_counts
                .entry(row_id.clone())
                .or_default()
                .occurrences = occurrence_count;
        }
    }
    summary.row_variants = variants;
    summary.row_semantic_partitions = semantic_partitions;
    summary
}

fn break_page_line_semantic_partitions(
    source: &str,
    work_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    let (body, _) = sanitized_body_range(source);
    source_markers(source)
        .into_iter()
        .flat_map(|marker| {
            matching_variant_occurrences(&marker, work_id, "break.page_line", patterns, 0, 0)
        })
        .filter(|marker| body.start <= marker.start && marker.start < body.end)
        .flat_map(|marker| {
            break_semantic_partitions(&marker.occurrence.body)
                .into_iter()
                .map(move |partition| (marker.start, partition, marker.occurrence.clone()))
        })
        .collect()
}

fn break_semantic_partitions(body: &str) -> Vec<String> {
    let (head, operand) = body
        .split_once('、')
        .map_or((body, None), |(head, operand)| (head, Some(operand)));
    let (head, side) = head
        .strip_suffix('右')
        .map(|head| (head, Some("right")))
        .or_else(|| head.strip_suffix('左').map(|head| (head, Some("left"))))
        .unwrap_or((head, None));
    let kind = match head {
        "改ページ" | "改頁" => "page",
        "改丁" => "recto",
        "改段" => "column",
        "改見開き" => "spread",
        "改行" => "line",
        _ => return Vec::new(),
    };
    let mut partitions = vec![format!("kind:{kind}"), format!("spelling:{head}")];
    if let Some(side) = side {
        partitions.push(format!("side:{side}"));
    }
    if let Some(operand) = operand {
        let partition = if matches!(operand, "ページの左右中央" | "ページの左右中央に")
        {
            "placement:horizontal-center".to_owned()
        } else if operand == "左寄せで" {
            "placement:left".to_owned()
        } else if let Some(value) = operand.strip_suffix("字下げで") {
            format!("indent:{value}")
        } else {
            format!("operand:{operand}")
        };
        partitions.push(partition);
    }
    partitions
}

pub(crate) fn font_size_semantic_partitions(
    source: &str,
    work_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    let (body_range, _) = sanitized_body_range(source);
    let mut markers = source_markers(source)
        .into_iter()
        .flat_map(|marker| {
            matching_variant_occurrences(&marker, work_id, "decoration.font_size", patterns, 0, 0)
        })
        .filter(|marker| body_range.start <= marker.start && marker.start < body_range.end)
        .collect::<Vec<_>>();
    markers.sort_by_key(|marker| marker.start);

    let mut partitions = Vec::new();
    let mut ranges = Vec::<(usize, FontSizeRange, SourceMarkerOccurrence)>::new();
    for marker in markers {
        let body = marker.occurrence.body.as_str();
        for fact in font_size_facts(body) {
            partitions.push((marker.start, fact, marker.occurrence.clone()));
        }
        if body.contains('「')
            && let Some(target) =
                emphasis_target_partition(source, marker.start, &marker.occurrence)
        {
            partitions.push((marker.start, target, marker.occurrence.clone()));
        }
        if let Some(range) = font_size_range(body) {
            ranges.push((marker.start, range, marker.occurrence));
        }
    }

    let mut stack = Vec::<(usize, FontSizeRange, SourceMarkerOccurrence)>::new();
    for (start, range, occurrence) in ranges {
        if !range.closing {
            stack.push((start, range, occurrence));
            continue;
        }
        match stack.last().map(|(_, open, _)| open) {
            Some(open) if open.family == range.family && range.matches(open) => {
                stack.pop();
                partitions.push((
                    start,
                    format!("boundary:matched;family:{}", range.family),
                    occurrence,
                ));
            }
            Some(open) if open.family == range.family => partitions.push((
                start,
                format!("boundary:mismatched;family:{}", range.family),
                occurrence,
            )),
            Some(open) => partitions.push((
                start,
                format!(
                    "boundary:mismatched;opened:{};closed:{}",
                    open.family, range.family
                ),
                occurrence,
            )),
            None => partitions.push((
                start,
                format!("boundary:orphan-close;family:{}", range.family),
                occurrence,
            )),
        }
    }
    partitions.extend(stack.into_iter().map(|(start, range, occurrence)| {
        (
            start,
            format!("boundary:orphan-open;family:{}", range.family),
            occurrence,
        )
    }));
    partitions
}

#[derive(Clone, Copy)]
struct FontSizeRange {
    family: &'static str,
    closing: bool,
    larger: Option<bool>,
    magnitude: Option<u64>,
}

impl FontSizeRange {
    fn matches(self, open: &Self) -> bool {
        self.larger.is_none_or(|value| open.larger == Some(value))
            && self
                .magnitude
                .is_none_or(|value| open.magnitude == Some(value))
    }
}

fn font_size_range(body: &str) -> Option<FontSizeRange> {
    if body.contains('「') || body.contains('、') {
        return None;
    }
    let core = body
        .strip_prefix("ここから")
        .or_else(|| body.strip_prefix("ここで"))
        .unwrap_or(body);
    if let Some(core) = core.strip_suffix("終わり") {
        if let Some((larger, magnitude)) = relative_font_size(core) {
            return Some(FontSizeRange {
                family: "relative-size",
                closing: true,
                larger: Some(larger),
                magnitude,
            });
        }
        if let Some(side) = core
            .strip_prefix("行")
            .and_then(|rest| rest.strip_suffix("小書き"))
            && matches!(side, "右" | "左")
        {
            return Some(FontSizeRange {
                family: if side == "右" {
                    "small-script-right"
                } else {
                    "small-script-left"
                },
                closing: true,
                larger: None,
                magnitude: None,
            });
        }
        return None;
    }
    if let Some((larger, magnitude)) = relative_font_size(core) {
        return Some(FontSizeRange {
            family: "relative-size",
            closing: false,
            larger: Some(larger),
            magnitude,
        });
    }
    let side = core
        .strip_prefix("行")
        .and_then(|rest| rest.strip_suffix("小書き"))?;
    Some(FontSizeRange {
        family: match side {
            "右" => "small-script-right",
            "左" => "small-script-left",
            _ => return None,
        },
        closing: false,
        larger: None,
        magnitude: None,
    })
}

fn relative_font_size(body: &str) -> Option<(bool, Option<u64>)> {
    let (larger, prefix) = body
        .strip_suffix("大きな文字")
        .map(|prefix| (true, prefix))
        .or_else(|| {
            body.strip_suffix("小さな文字")
                .map(|prefix| (false, prefix))
        })?;
    let magnitude = if prefix.is_empty() {
        None
    } else {
        let digits = prefix.strip_suffix("段階")?;
        Some(if digits == "一" {
            1
        } else {
            decimal_value(digits)?
        })
    };
    Some((larger, magnitude))
}

fn font_size_facts(body: &str) -> Vec<String> {
    let mut facts = Vec::new();
    let targeted = body.contains('「');
    if let Some(range) = font_size_range(body) {
        facts.push(format!(
            "form:{}-{}",
            if body.starts_with("ここ") {
                "block-range"
            } else {
                "bare-range"
            },
            if range.closing { "close" } else { "open" }
        ));
        facts.push(format!("effect:{}", range.family));
        if let Some(larger) = range.larger {
            facts.push(format!(
                "direction:{}",
                if larger { "larger" } else { "smaller" }
            ));
        }
        if let Some(magnitude) = range.magnitude {
            facts.push(format!("magnitude:{magnitude}"));
        }
        return facts;
    }

    if let Some((larger, magnitude)) = body
        .strip_suffix("回り大きな文字")
        .and_then(decimal_value)
        .map(|magnitude| (true, magnitude))
        .or_else(|| {
            body.strip_suffix("回り小さな文字")
                .and_then(decimal_value)
                .map(|magnitude| (false, magnitude))
        })
    {
        facts.push("form:legacy-range-open".to_owned());
        facts.push("effect:relative-size".to_owned());
        facts.push(format!(
            "direction:{}",
            if larger { "larger" } else { "smaller" }
        ));
        facts.push(format!("magnitude:{magnitude}"));
        return facts;
    }

    if let Some((larger, magnitude)) =
        relative_font_size(body.rsplit_once("は").map_or(body, |(_, effect)| effect))
    {
        facts.push("form:targeted".to_owned());
        facts.push("effect:relative-size".to_owned());
        facts.push(format!(
            "direction:{}",
            if larger { "larger" } else { "smaller" }
        ));
        if let Some(magnitude) = magnitude {
            facts.push(format!("magnitude:{magnitude}"));
        }
        return facts;
    }

    if body.contains("行右小書き") || body.contains("小書き右寄せ") {
        facts.push("form:targeted".to_owned());
        facts.push("effect:small-script".to_owned());
        facts.push("position:right".to_owned());
    } else if body.contains("行左小書き") {
        facts.push("form:targeted".to_owned());
        facts.push("effect:small-script".to_owned());
        facts.push("position:left".to_owned());
    }

    let baseline = if body.contains("下付き") || body.contains("ポイントを下げ") {
        Some("lowered")
    } else if body.contains("上付き")
        || body.contains("上部に出ている")
        || body.contains("右上がり")
    {
        Some("raised")
    } else {
        None
    };
    if let Some(position) = baseline {
        facts.push(format!(
            "form:{}",
            if targeted { "targeted" } else { "global" }
        ));
        facts.push("effect:baseline-position".to_owned());
        facts.push(format!("position:{position}"));
    }

    let target_effect = body
        .rsplit_once("は")
        .map(|(_, effect)| effect)
        .or_else(|| body.rsplit_once("の").map(|(_, effect)| effect));
    let absolute = if body.starts_with("特大文字") || target_effect == Some("特大文字") {
        Some("extra-large")
    } else if body.starts_with("大文字") || target_effect == Some("大文字") {
        Some("large")
    } else if body.starts_with("中文字") || target_effect == Some("中文字") {
        Some("medium")
    } else if body.starts_with("小文字") || matches!(target_effect, Some("小文字" | "小書き"))
    {
        Some("small")
    } else {
        None
    };
    if let Some(size) = absolute {
        facts.push(format!(
            "form:{}",
            if targeted { "targeted" } else { "line" }
        ));
        facts.push("effect:absolute-size".to_owned());
        facts.push(format!("size:{size}"));
        if body.contains("太字") {
            facts.push("compound:bold".to_owned());
        }
    }

    if body.contains("小さい活字")
        || body.contains("小さめの文字")
        || body.contains("小さいサイズの文字")
        || body.contains("小さい文字")
        || body.contains("小さい字")
        || body.contains("ポイント小さく")
        || body.starts_with("ここから小文字、")
        || body.starts_with("ここで小文字、")
    {
        facts.push(format!(
            "form:{}",
            if targeted { "targeted" } else { "compound" }
        ));
        facts.push("effect:qualitative-size".to_owned());
        facts.push("direction:smaller".to_owned());
    }
    if facts.is_empty() {
        facts.push("form:instruction".to_owned());
        facts.push("interpretation:unresolved".to_owned());
        facts.push("preservation:raw".to_owned());
    }
    facts.sort();
    facts.dedup();
    facts
}

pub(crate) fn emphasis_semantic_partitions(
    source: &str,
    work_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    targeted_style_semantic_partitions(source, work_id, patterns, "emphasis.basic")
}

pub(crate) fn targeted_style_semantic_partitions(
    source: &str,
    work_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
    row_id: &str,
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    let (body, _) = sanitized_body_range(source);
    let mut markers = source_markers(source)
        .into_iter()
        .flat_map(|marker| matching_variant_occurrences(&marker, work_id, row_id, patterns, 0, 0))
        .filter(|marker| body.start <= marker.start && marker.start < body.end)
        .collect::<Vec<_>>();
    markers.sort_by_key(|marker| marker.start);
    let mut partitions = markers
        .iter()
        .filter_map(|marker| {
            emphasis_target_partition(source, marker.start, &marker.occurrence)
                .map(|partition| (marker.start, partition, marker.occurrence.clone()))
        })
        .collect::<Vec<_>>();
    let mut stack = Vec::<(usize, String, SourceMarkerOccurrence)>::new();
    for marker in markers {
        let Some((family, closing)) = emphasis_boundary(&marker.occurrence.body) else {
            continue;
        };
        if !closing {
            stack.push((marker.start, family, marker.occurrence));
            continue;
        }
        match stack.last().map(|(_, opened, _)| opened.as_str()) {
            Some(opened) if opened == family => {
                stack.pop();
                partitions.push((
                    marker.start,
                    format!("boundary:matched;family:{family}"),
                    marker.occurrence,
                ));
            }
            Some(opened) if stack.iter().any(|(_, candidate, _)| candidate == &family) => {
                partitions.push((
                    marker.start,
                    format!("boundary:crossed;opened:{opened};closed:{family}"),
                    marker.occurrence,
                ));
            }
            Some(opened) => partitions.push((
                marker.start,
                format!("boundary:mismatched;opened:{opened};closed:{family}"),
                marker.occurrence,
            )),
            None => partitions.push((
                marker.start,
                format!("boundary:orphan-close;family:{family}"),
                marker.occurrence,
            )),
        }
    }
    partitions.extend(stack.into_iter().map(|(start, family, occurrence)| {
        (
            start,
            format!("boundary:orphan-open;family:{family}"),
            occurrence,
        )
    }));
    partitions
}

pub(crate) fn kaeriten_semantic_partitions(
    source: &str,
    work_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    let (body_range, _) = sanitized_body_range(source);
    source_markers(source)
        .into_iter()
        .flat_map(|marker| {
            matching_variant_occurrences(&marker, work_id, "kunten.kaeriten", patterns, 0, 0)
        })
        .filter(|marker| body_range.start <= marker.start && marker.start < body_range.end)
        .flat_map(|marker| {
            kaeriten_facts(&marker.occurrence.body)
                .into_iter()
                .map(move |fact| (marker.start, fact, marker.occurrence.clone()))
        })
        .collect()
}

fn kaeriten_facts(body: &str) -> Vec<String> {
    const MARKS: &str = "一二三四五六七八九十レ上中下甲乙丙丁天地人";
    let bare = !body.is_empty() && body.chars().all(|character| MARKS.contains(character));
    let marker = if bare {
        Some(body.to_owned())
    } else {
        quoted_operands(body)
            .into_iter()
            .rev()
            .find(|operand| {
                !operand.is_empty() && operand.chars().all(|character| MARKS.contains(character))
            })
            .filter(|operand| body.contains(operand))
            .or_else(|| {
                body.split_once("は返り点").and_then(|(prefix, _)| {
                    let start = prefix
                        .char_indices()
                        .rev()
                        .find(|(_, character)| !MARKS.contains(*character))
                        .map_or(0, |(index, character)| index + character.len_utf8());
                    (start < prefix.len()).then(|| prefix[start..].to_owned())
                })
            })
    };
    let Some(marker) = marker else {
        return Vec::new();
    };

    let (form, target, scope) = if bare {
        ("bare", "preceding-character", "single-position")
    } else if body.starts_with("返り点の") {
        ("insertion", "insertion-site", "single-position")
    } else if body.starts_with("ここから") {
        ("declaration", "following-source-range", "following-block")
    } else if body.starts_with("ここで") {
        ("declaration", "preceding-source-range", "preceding-block")
    } else if body.starts_with("以下") {
        ("declaration", "following-source-range", "following-text")
    } else if body.contains("内の") {
        ("declaration", "quoted-container", "quoted-region")
    } else {
        (
            "retrospective",
            "quoted-previous-occurrence",
            "single-position",
        )
    };
    let mut facts = vec![
        format!("form:{form}"),
        format!("target:{target}"),
        format!("scope:{scope}"),
        format!(
            "marker:{}",
            if marker.chars().count() == 1 {
                "single"
            } else {
                "compound"
            }
        ),
        format!(
            "exclusion:{}",
            if body.contains("のぞ") {
                "present"
            } else {
                "absent"
            }
        ),
        format!(
            "repetition:{}",
            if body.contains("以下同じ") {
                "following"
            } else {
                "none"
            }
        ),
    ];
    if body.starts_with("ここから") {
        facts.push("layout:indent-open".to_owned());
        if body.contains("折り返して") {
            facts.push("layout:folding-indent".to_owned());
        }
    } else if body.starts_with("ここで字下げ終わり") {
        facts.push("layout:indent-close".to_owned());
    }
    if body.contains("右横に縦棒") {
        facts.push("adjunct:vertical-rule-right".to_owned());
    }
    facts
}

pub(crate) fn tcy_semantic_partitions(
    source: &str,
    work_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    let (body_range, _) = sanitized_body_range(source);
    let mut markers = source_markers(source)
        .into_iter()
        .flat_map(|marker| {
            matching_variant_occurrences(&marker, work_id, "layout.tcy", patterns, 0, 0)
        })
        .filter(|marker| body_range.start <= marker.start && marker.start < body_range.end)
        .collect::<Vec<_>>();
    markers.sort_by_key(|marker| marker.start);

    let mut boundary = BTreeMap::<usize, &'static str>::new();
    let mut open = Vec::<usize>::new();
    for marker in &markers {
        match marker.occurrence.body.as_str() {
            "縦中横" | "ここから縦中横" => open.push(marker.start),
            "縦中横終わり" | "ここで縦中横終わり" => {
                if let Some(open_start) = open.pop() {
                    boundary.insert(open_start, "boundary:matched-open");
                    boundary.insert(marker.start, "boundary:matched-close");
                } else {
                    boundary.insert(marker.start, "boundary:orphan-close");
                }
            }
            _ => {}
        }
    }
    for start in open {
        boundary.insert(start, "boundary:orphan-open");
    }

    markers
        .into_iter()
        .flat_map(|marker| {
            let mut facts = tcy_facts(source, marker.start, &marker.occurrence);
            if let Some(fact) = boundary.get(&marker.start) {
                facts.push((*fact).to_owned());
            }
            facts
                .into_iter()
                .map(move |fact| (marker.start, fact, marker.occurrence.clone()))
        })
        .collect()
}

fn tcy_facts(
    source: &str,
    marker_start: usize,
    occurrence: &SourceMarkerOccurrence,
) -> Vec<String> {
    let body = occurrence.body.as_str();
    let range = matches!(
        body,
        "縦中横" | "縦中横終わり" | "ここから縦中横" | "ここで縦中横終わり"
    );
    if range {
        let opening = matches!(body, "縦中横" | "ここから縦中横");
        return vec![
            format!("form:range-{}", if opening { "open" } else { "close" }),
            "effect:tcy".to_owned(),
            "scope:paired-range".to_owned(),
            format!(
                "target:{}-source-range",
                if opening { "following" } else { "preceding" }
            ),
            "composition:none".to_owned(),
        ];
    }

    let operands = quoted_operands(body);
    let unquoted_parenthesized = parenthesized_tcy_target(body);
    let repeated_targets = body
        .split_once("はそれぞれ縦中横")
        .map(|(targets, _)| quoted_operands(targets))
        .unwrap_or_default();
    let form = if body.contains("日の下に縦中横") {
        "relative-placement"
    } else if body.contains("横一列") {
        "horizontal-line"
    } else if unquoted_parenthesized.is_some() {
        "retrospective-parenthesized"
    } else if body.contains("それぞれ縦中横") {
        "retrospective-plural"
    } else if body.contains("の縦中横") {
        "retrospective-no-particle"
    } else {
        "retrospective-quoted"
    };
    let target = if unquoted_parenthesized.is_some() {
        "unquoted-parenthesized-previous-occurrence"
    } else if repeated_targets.len() > 1 {
        "multiple-quoted-previous-occurrences"
    } else if repeated_targets.len() == 1 {
        "quoted-pattern-previous-occurrences"
    } else if operands.is_empty() {
        "unresolved"
    } else {
        "quoted-previous-occurrence"
    };
    let scope = if body.contains("それぞれ縦中横") {
        "multiple-targets"
    } else {
        "single-target"
    };
    let mut facts = vec![
        format!("form:{form}"),
        "effect:tcy".to_owned(),
        format!("scope:{scope}"),
        format!("target:{target}"),
    ];
    if let Some(target) = unquoted_parenthesized {
        facts.push(parenthesized_target_partition(source, marker_start, target));
    } else if let Some(relation) = emphasis_target_partition(source, marker_start, occurrence) {
        facts.push(relation);
    }

    let mut composed = false;
    for (needle, fact) in [
        ("行右小書き", "composition:small-script-right"),
        ("上付き小書き", "composition:superscript"),
        ("下付き小文字", "composition:subscript"),
        ("底本では", "composition:source-apparatus"),
        ("が底本では", "composition:source-apparatus"),
        ("左右中央", "composition:horizontal-centering"),
        ("縦組み", "composition:vertical-layout"),
    ] {
        if body.contains(needle) && !facts.iter().any(|value| value == fact) {
            facts.push(fact.to_owned());
            composed = true;
        }
    }
    if !composed {
        facts.push("composition:none".to_owned());
    }
    facts
}

pub(crate) fn rotation_semantic_partitions(
    source: &str,
    work_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    let (body_range, _) = sanitized_body_range(source);
    source_markers(source)
        .into_iter()
        .flat_map(|marker| {
            matching_variant_occurrences(&marker, work_id, "layout.rotation", patterns, 0, 0)
        })
        .filter(|marker| body_range.start <= marker.start && marker.start < body_range.end)
        .flat_map(|marker| {
            let body = &marker.occurrence.body;
            let degrees = body
                .split_once("」は")
                .and_then(|(_, effect)| effect.strip_suffix("度回転"))
                .and_then(decimal_value);
            let mut facts = vec![
                "form:retrospective-quoted".to_owned(),
                "effect:rotation".to_owned(),
                "direction:unspecified".to_owned(),
                "scope:single-target".to_owned(),
                "target:quoted-previous-occurrence".to_owned(),
            ];
            if let Some(degrees) = degrees {
                facts.push(format!("angle-degrees:{degrees}"));
            }
            if let Some(relation) =
                emphasis_target_partition(source, marker.start, &marker.occurrence)
            {
                facts.push(relation);
            }
            facts
                .into_iter()
                .map(move |fact| (marker.start, fact, marker.occurrence.clone()))
        })
        .collect()
}

fn parenthesized_tcy_target(body: &str) -> Option<&str> {
    let end = body.find("）は縦中横")? + '）'.len_utf8();
    let target = body.get(..end)?;
    (target.starts_with('（') && !target['（'.len_utf8()..].contains('（')).then_some(target)
}

fn parenthesized_target_partition(source: &str, marker_start: usize, target: &str) -> String {
    let prefix = source.get(..marker_start).unwrap_or_default();
    let count = prefix.matches(target).count();
    let referent = if prefix.ends_with(target) {
        if count > 1 {
            "adjacent-repeated"
        } else {
            "adjacent-unique"
        }
    } else if count > 1 {
        "detached-repeated"
    } else if count == 1 {
        "detached-unique"
    } else {
        "missing"
    };
    format!("selector:parenthesized;referent:{referent}")
}

pub(crate) fn indentation_semantic_partitions(
    source: &str,
    work_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    let (body_range, _) = sanitized_body_range(source);
    let markers = source_markers(source)
        .into_iter()
        .filter(|marker| {
            body_range.start <= marker.span.start && marker.span.start < body_range.end
        })
        .collect::<Vec<_>>();
    let mut selected = Vec::<(usize, String, Option<u64>, SourceMarkerOccurrence)>::new();
    for marker in &markers {
        for matched in
            matching_variant_occurrences(marker, work_id, "indentation.basic", patterns, 0, 0)
        {
            if let Some((form, amount)) = basic_indentation_form(&matched.occurrence.body) {
                selected.push((matched.start, form.to_owned(), amount, matched.occurrence));
            }
        }
    }
    selected.sort_by_key(|(start, _, _, _)| *start);

    let mut boundaries = BTreeMap::<usize, &'static str>::new();
    let mut open = None;
    for marker in &markers {
        if selected
            .iter()
            .any(|(start, form, _, _)| *start == marker.span.start && form == "block-indent")
        {
            if let Some(previous) = open.replace(marker.span.start) {
                boundaries.insert(previous, "next-block-open");
            }
        } else if is_indentation_close(marker.body)
            && let Some(previous) = open.take()
        {
            boundaries.insert(previous, "matched-close");
        }
    }
    if let Some(previous) = open {
        boundaries.insert(previous, "missing-close");
    }

    let mut partitions = Vec::new();
    for (start, form, amount, occurrence) in selected {
        let scope = if form == "block-indent" {
            "block"
        } else {
            "line"
        };
        for partition in [format!("form:{form}"), format!("scope:{scope}")] {
            partitions.push((start, partition, occurrence.clone()));
        }
        if let Some(amount) = amount {
            partitions.push((start, format!("indent:{amount}"), occurrence.clone()));
        } else {
            partitions.push((start, "alignment:end".to_owned(), occurrence.clone()));
        }
        partitions.push((
            start,
            format!(
                "placement:{}",
                marker_line_placement(source, start, occurrence.raw.len())
            ),
            occurrence.clone(),
        ));
        if form == "block-indent" {
            partitions.push((
                start,
                format!(
                    "boundary:{}",
                    boundaries.get(&start).copied().unwrap_or("missing-close")
                ),
                occurrence,
            ));
        }
    }
    partitions
}

pub(crate) fn chitsuki_semantic_partitions(
    source: &str,
    work_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    let (body_range, _) = sanitized_body_range(source);
    let mut unique = BTreeMap::<usize, MatchedVariantOccurrence>::new();
    for marker in source_markers(source) {
        if chitsuki_excluded_cross_line_alignment(marker.body) {
            continue;
        }
        for matched in
            matching_variant_occurrences(&marker, work_id, "indentation.chitsuki", patterns, 0, 0)
        {
            if body_range.start <= matched.start && matched.start < body_range.end {
                unique.entry(matched.start).or_insert(matched);
            }
        }
    }
    let markers = unique.into_values().collect::<Vec<_>>();

    let mut boundaries = BTreeMap::<usize, &'static str>::new();
    let mut open = Vec::<usize>::new();
    for marker in &markers {
        let body = marker.occurrence.body.as_str();
        if chitsuki_block_open(body) {
            open.push(marker.start);
        } else if chitsuki_block_close(body) {
            if let Some(open_start) = open.pop() {
                boundaries.insert(open_start, "boundary:matched-open");
                boundaries.insert(marker.start, "boundary:matched-close");
            } else {
                boundaries.insert(marker.start, "boundary:orphan-close");
            }
        }
    }
    for start in open {
        boundaries.insert(start, "boundary:orphan-open");
    }

    markers
        .into_iter()
        .flat_map(|marker| {
            let mut facts = chitsuki_facts(source, marker.start, &marker.occurrence);
            if let Some(boundary) = boundaries.get(&marker.start) {
                facts.push((*boundary).to_owned());
            }
            facts
                .into_iter()
                .map(move |fact| (marker.start, fact, marker.occurrence.clone()))
        })
        .collect()
}

fn chitsuki_excluded_cross_line_alignment(body: &str) -> bool {
    body.contains("頭揃え") || body.contains("頭は全て揃っている")
}

pub(crate) fn cross_line_head_alignment_semantic_partitions(
    source: &str,
    work_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    let (body_range, _) = sanitized_body_range(source);
    source_markers(source)
        .into_iter()
        .flat_map(|marker| {
            matching_variant_occurrences(
                &marker,
                work_id,
                "layout.cross_line_head_alignment",
                patterns,
                0,
                0,
            )
        })
        .filter(|matched| body_range.start <= matched.start && matched.start < body_range.end)
        .flat_map(|matched| {
            let facts: &[&str] = match matched.occurrence.body.as_str() {
                "この歌、二行前の歌に頭揃え。" => &[
                    "effect:line-start-alignment",
                    "relation:relative-line",
                    "lines-before:2",
                    "scope:current-line",
                    "target:two-lines-before",
                ],
                "で各歌の頭は全て揃っている。" => &[
                    "effect:line-start-alignment",
                    "relation:collective",
                    "operand:各歌",
                    "scope:named-group",
                    "target:unresolved-members",
                ],
                _ => &[],
            };
            facts.iter().map(move |fact| {
                (
                    matched.start,
                    (*fact).to_owned(),
                    matched.occurrence.clone(),
                )
            })
        })
        .collect()
}

fn chitsuki_block_open(body: &str) -> bool {
    body.starts_with("ここから")
        && !body.contains("字下げ")
        && (body.contains("地付き") || body.contains("字上げ"))
}

fn chitsuki_block_close(body: &str) -> bool {
    body.starts_with("ここで") && body.ends_with("終わり")
}

fn chitsuki_facts(
    source: &str,
    marker_start: usize,
    occurrence: &SourceMarkerOccurrence,
) -> Vec<String> {
    // A stray opening `「` that quotes nothing does not change the
    // instruction; the published XHTML applies `［＃「地付き］` as `［＃地付き］`.
    let body = occurrence.body.as_str();
    let body = body
        .strip_prefix('「')
        .filter(|rest| !rest.contains(['「', '」']))
        .unwrap_or(body);
    let quoted = body.contains('「') && body.contains('」');
    let block_open = chitsuki_block_open(body);
    let block_close = chitsuki_block_close(body);
    let following_declaration = body.starts_with("以下");
    let named_following_target = body.starts_with('（') && body.contains("）は下揃え");
    let compound_open = body.starts_with("ここから") && body.contains("字下げ");
    let compound_close = body.contains("地付きここまで");
    let offset = ["字上げ", "字上がり", "字あき", "字アキ"]
        .into_iter()
        .find_map(|suffix| number_before(body, suffix));
    let compound = body.contains("字下げ")
        || body.contains("ポイント")
        || body.contains("相対的に字が小さい")
        || body.contains("一字毎に空き")
        || body.contains("ゴシック体");
    let form = if named_following_target {
        "following-pattern-align-end"
    } else if following_declaration {
        "following-text-align-end"
    } else if compound_open {
        "compound-block-open"
    } else if compound_close {
        "compound-block-close"
    } else if block_close {
        "block-close"
    } else if block_open {
        if offset.is_some() {
            "block-open-offset"
        } else {
            "block-open-align-end"
        }
    } else if quoted {
        if offset.is_some() {
            "retrospective-offset"
        } else {
            "retrospective-align-end"
        }
    } else if compound {
        "compound-line-geometry"
    } else if offset.is_some() {
        "line-offset"
    } else {
        "line-align-end"
    };
    let scope = if block_open || block_close {
        "paired-block"
    } else if following_declaration || named_following_target {
        "following-text"
    } else if compound_open {
        "following-block"
    } else if compound_close {
        "preceding-block"
    } else if quoted {
        "single-target"
    } else {
        "line"
    };
    let placement = marker_line_placement(source, marker_start, occurrence.raw.len());
    let target = if block_open {
        "following-source-range"
    } else if block_close {
        "preceding-source-range"
    } else if following_declaration {
        "following-source-range"
    } else if named_following_target {
        "named-token-in-following-text"
    } else if compound_open {
        "following-source-range"
    } else if compound_close {
        "preceding-source-range"
    } else if quoted {
        "quoted-previous-occurrence"
    } else if body.starts_with("この行") || body.starts_with("この日付") {
        "current-line"
    } else if matches!(placement, "inline" | "line-prefix") {
        "same-line-remainder"
    } else {
        "adjacent-line"
    };
    let mut facts = vec![
        format!("form:{form}"),
        "effect:end-alignment".to_owned(),
        format!("scope:{scope}"),
        format!("target:{target}"),
        format!("placement:{placement}"),
    ];
    if let Some(offset) = offset {
        facts.push(format!("offset-from-end:{offset}"));
    } else if !block_close && !compound_close && !named_following_target {
        facts.push("offset-from-end:0".to_owned());
    }
    if quoted && let Some(relation) = emphasis_target_partition(source, marker_start, occurrence) {
        facts.push(relation);
    }
    for (present, fact) in [
        (body.contains("字下げ"), "composition:start-indent"),
        (
            body.contains("ポイント") || body.contains("相対的に字が小さい"),
            "composition:font-size",
        ),
        (body.contains("一字毎に空き"), "composition:letter-spacing"),
        (body.contains("ゴシック体"), "composition:bold"),
    ] {
        if present {
            facts.push(fact.to_owned());
        }
    }
    if !compound {
        facts.push("composition:none".to_owned());
    }
    facts
}

pub(crate) fn jisage_oneline_semantic_partitions(
    source: &str,
    work_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    let (body_range, _) = sanitized_body_range(source);
    let mut partitions = Vec::new();
    for marker in source_markers(source).into_iter().filter(|marker| {
        body_range.start <= marker.span.start && marker.span.start < body_range.end
    }) {
        for matched in matching_variant_occurrences(
            &marker,
            work_id,
            "indentation.jisage_oneline",
            patterns,
            0,
            0,
        ) {
            let Some((form, amount, digits, valid)) = oneline_jisage_form(&matched.occurrence.body)
            else {
                continue;
            };
            let placement =
                marker_line_placement(source, matched.start, matched.occurrence.raw.len());
            let digit_spelling = if digits.chars().all(|character| character.is_ascii_digit()) {
                "ascii"
            } else if digits
                .chars()
                .all(|character| ('０'..='９').contains(&character))
            {
                "fullwidth"
            } else {
                "mixed"
            };
            let facts = if valid {
                vec![
                    format!("form:{form}"),
                    "scope:line".to_owned(),
                    format!(
                        "target:{}",
                        if placement == "inline" {
                            "whole-line"
                        } else {
                            "same-line-remainder"
                        }
                    ),
                    format!("indent:{amount}"),
                    format!("digits:{digit_spelling}"),
                    format!("placement:{placement}"),
                    format!(
                        "content:{}",
                        if placement == "standalone-line" {
                            "empty"
                        } else {
                            "present"
                        }
                    ),
                ]
            } else {
                vec![
                    format!("form:{form}"),
                    "scope:unresolved".to_owned(),
                    "target:unresolved".to_owned(),
                    format!("indent:{amount}"),
                    format!("digits:{digit_spelling}"),
                    format!("placement:{placement}"),
                    "interpretation:unresolved".to_owned(),
                    "preservation:raw".to_owned(),
                ]
            };
            for fact in facts {
                partitions.push((matched.start, fact, matched.occurrence.clone()));
            }
        }
    }
    partitions
}

fn oneline_jisage_form(body: &str) -> Option<(&'static str, u64, &str, bool)> {
    let (form, quantified) = if let Some(rest) = body.strip_prefix("この行") {
        ("explicit-this-line", rest)
    } else if let Some(rest) = body.strip_prefix("天から") {
        ("top-margin", rest)
    } else if let Some(digits) = body.strip_suffix("字下がる") {
        let amount = decimal_value(digits)?;
        let valid = amount > 0 && u8::try_from(amount).is_ok();
        return Some((
            if valid { "legacy-sagaru" } else { "malformed" },
            amount,
            digits,
            valid,
        ));
    } else {
        ("bare", body)
    };
    let digits = quantified.strip_suffix("字下げ")?;
    let amount = decimal_value(digits)?;
    let valid = amount > 0 && u8::try_from(amount).is_ok();
    Some((
        if valid { form } else { "malformed" },
        amount,
        digits,
        valid,
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JisageSemanticForm {
    BlockOpen,
    BlockClose,
    LineLayout,
    TargetedLayout,
    DocumentInstruction,
    QuotedExample,
    Malformed,
}

impl JisageSemanticForm {
    const fn partition(self) -> &'static str {
        match self {
            Self::BlockOpen => "block-open",
            Self::BlockClose => "block-close",
            Self::LineLayout => "line-layout",
            Self::TargetedLayout => "targeted-layout",
            Self::DocumentInstruction => "document-instruction",
            Self::QuotedExample => "quoted-example",
            Self::Malformed => "malformed",
        }
    }
}

#[derive(Clone)]
struct SelectedJisageMarker {
    start: usize,
    form: JisageSemanticForm,
    indent: Option<u64>,
    width: Option<u64>,
    occurrence: SourceMarkerOccurrence,
}

pub(crate) fn jisage_block_semantic_partitions(
    source: &str,
    work_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    let (body_range, _) = sanitized_body_range(source);
    let markers = source_markers(source)
        .into_iter()
        .filter(|marker| {
            body_range.start <= marker.span.start && marker.span.start < body_range.end
        })
        .collect::<Vec<_>>();
    let mut selected = BTreeMap::<usize, SelectedJisageMarker>::new();
    for marker in &markers {
        let command = matches!(
            marker.kind,
            SourceMarkerKind::CommandFullwidth | SourceMarkerKind::CommandAscii
        );
        if command
            && matching_rows(marker.raw, patterns)
                .iter()
                .any(|row| row == "indentation.jisage_block")
        {
            let body = marker.body;
            selected.insert(
                marker.span.start,
                SelectedJisageMarker {
                    start: marker.span.start,
                    form: jisage_semantic_form(body),
                    indent: jisage_indent(body),
                    width: number_before(body, "字詰め").or_else(|| number_before(body, "字組み")),
                    occurrence: SourceMarkerOccurrence {
                        work_id: work_id.to_owned(),
                        indexed_path: None,
                        line: marker.span.line,
                        kind: format!("{:?}", marker.kind),
                        raw: marker.raw.to_owned(),
                        body: body.to_owned(),
                        byte_start: None,
                    },
                },
            );
        }
    }

    let mut boundaries = BTreeMap::<usize, &'static str>::new();
    let mut open = None::<Option<usize>>;
    for source_marker in &markers {
        let selected_marker = selected.get(&source_marker.span.start);
        if selected_marker.is_some_and(|marker| {
            marker.form == JisageSemanticForm::BlockOpen
                && marker.occurrence.body.contains("最後まで")
        }) {
            boundaries.insert(source_marker.span.start, "body-end");
        } else if is_any_indentation_block_open(source_marker.body) {
            if let Some(Some(previous)) = open {
                boundaries.insert(previous, "next-indentation-open");
            }
            open = Some(
                selected_marker
                    .filter(|marker| marker.form == JisageSemanticForm::BlockOpen)
                    .map(|marker| marker.start),
            );
        } else if is_indentation_close(source_marker.body)
            || source_marker.body.starts_with("ここで字下げ、")
        {
            let Some(marker) = selected_marker else {
                open = None;
                continue;
            };
            match open {
                Some(Some(previous_start)) => {
                    let previous = &selected[&previous_start];
                    let amount_matches =
                        marker.indent.is_none() || marker.indent == previous.indent;
                    let width_matches = marker.width.is_none() || marker.width == previous.width;
                    if amount_matches && width_matches {
                        boundaries.insert(previous.start, "matched-close");
                        boundaries.insert(marker.start, "matched-open");
                        open = None;
                    } else {
                        boundaries.insert(marker.start, "mismatched-close");
                    }
                }
                Some(None) => {
                    boundaries.insert(marker.start, "matched-other-indentation");
                    open = None;
                }
                None => {
                    boundaries.insert(marker.start, "orphan-close");
                }
            }
        }
    }
    if let Some(Some(previous)) = open {
        boundaries.insert(previous, "missing-close");
    }

    let mut partitions = Vec::new();
    for marker in selected.into_values() {
        let body = marker.occurrence.body.as_str();
        let mut facts = vec![
            format!("form:{}", marker.form.partition()),
            format!(
                "placement:{}",
                marker_line_placement(source, marker.start, marker.occurrence.raw.len())
            ),
        ];
        match marker.form {
            JisageSemanticForm::BlockOpen => {
                facts.extend([
                    "scope:block".to_owned(),
                    if body.contains("最後まで") {
                        "target:document-remainder".to_owned()
                    } else {
                        "target:following-block".to_owned()
                    },
                    format!(
                        "boundary:{}",
                        boundaries
                            .get(&marker.start)
                            .copied()
                            .unwrap_or("missing-close")
                    ),
                    format!(
                        "compound:{}",
                        if body.contains('、') { "yes" } else { "no" }
                    ),
                ]);
                append_jisage_geometry_partitions(body, &mut facts);
                append_jisage_clause_partitions(body, &mut facts);
                facts.push(format!("spelling:{}", jisage_open_spelling(body)));
            }
            JisageSemanticForm::BlockClose => {
                facts.extend([
                    "scope:block-boundary".to_owned(),
                    "target:active-block".to_owned(),
                    format!(
                        "boundary:{}",
                        boundaries
                            .get(&marker.start)
                            .copied()
                            .unwrap_or("orphan-close")
                    ),
                    format!("spelling:{}", jisage_close_spelling(body)),
                ]);
                append_jisage_geometry_partitions(body, &mut facts);
                append_jisage_clause_partitions(body, &mut facts);
            }
            JisageSemanticForm::LineLayout => {
                facts.extend(["scope:line".to_owned(), "target:following-line".to_owned()]);
                append_jisage_geometry_partitions(body, &mut facts);
            }
            JisageSemanticForm::TargetedLayout => {
                facts.extend([
                    "scope:explicit-target".to_owned(),
                    "target:quoted-source-text".to_owned(),
                ]);
                append_jisage_geometry_partitions(body, &mut facts);
                append_jisage_clause_partitions(body, &mut facts);
            }
            JisageSemanticForm::DocumentInstruction => {
                facts.extend([
                    "scope:document-instruction".to_owned(),
                    "target:described-lines".to_owned(),
                ]);
                append_jisage_geometry_partitions(body, &mut facts);
            }
            JisageSemanticForm::QuotedExample => facts.extend([
                "scope:documentary".to_owned(),
                "target:none".to_owned(),
                "interpretation:quoted-example".to_owned(),
            ]),
            JisageSemanticForm::Malformed => {
                facts.extend([
                    "scope:unresolved".to_owned(),
                    "target:unresolved".to_owned(),
                    "interpretation:unresolved".to_owned(),
                    "preservation:raw".to_owned(),
                ]);
                append_jisage_geometry_partitions(body, &mut facts);
            }
        }
        facts.sort();
        facts.dedup();
        partitions.extend(
            facts
                .into_iter()
                .map(|fact| (marker.start, fact, marker.occurrence.clone())),
        );
    }
    partitions
}

fn jisage_semantic_form(body: &str) -> JisageSemanticForm {
    if is_indentation_close(body) || body.starts_with("ここで字下げ、") {
        JisageSemanticForm::BlockClose
    } else if body.starts_with("ここから") && jisage_indent(body).is_some()
        || body.starts_with("ここより")
        || body.starts_with("以下")
    {
        JisageSemanticForm::BlockOpen
    } else if (body.starts_with("天より") || body.starts_with("天から"))
        && number_before(body, "字下げ").is_some()
    {
        JisageSemanticForm::LineLayout
    } else if body.starts_with('「') && body.contains("全体、天より")
        || body.starts_with('「') && body.contains("の文字の下から")
    {
        JisageSemanticForm::TargetedLayout
    } else if body.starts_with("改行ごとに")
        || body.starts_with("場面設定の表題")
        || body.starts_with("本文の台詞部分")
    {
        JisageSemanticForm::DocumentInstruction
    } else if body.starts_with('「') && body.ends_with('」') {
        JisageSemanticForm::QuotedExample
    } else {
        JisageSemanticForm::Malformed
    }
}

fn is_any_indentation_block_open(body: &str) -> bool {
    (body.starts_with("ここから") || body.starts_with("ここより") || body.starts_with("以下"))
        && jisage_indent(body).is_some()
}

fn append_jisage_geometry_partitions(body: &str, facts: &mut Vec<String>) {
    if let Some(value) = jisage_indent(body) {
        facts.push(format!("indent:{value}"));
    }
    if let Some(rest) = body.split_once("折り返して").map(|(_, rest)| rest)
        && let Some(value) = number_before(rest, "字下げ")
    {
        facts.push("geometry:hanging-indent".to_owned());
        facts.push(format!("continuation-indent:{value}"));
    }
    if let Some(value) = number_before(body, "字詰め") {
        facts.push("geometry:line-width".to_owned());
        facts.push(format!("width:{value}"));
    }
    if let Some(value) = number_before(body, "字組み") {
        facts.push("geometry:kumi".to_owned());
        facts.push(format!("width:{value}"));
        if let Some(lines) = number_before(body, "行") {
            facts.push(format!("line-count:{lines}"));
        }
    }
    if let Some(value) = number_before(body, "字上げ").or_else(|| {
        body.split_once("地から")
            .and_then(|(_, rest)| number_before(rest, "字下げ"))
    }) {
        facts.push("geometry:end-offset".to_owned());
        facts.push(format!("end-offset:{value}"));
    }
    if let Some(value) = number_before(body, "段組み") {
        facts.push("geometry:columns".to_owned());
        facts.push(format!("columns:{value}"));
    }
}

fn jisage_indent(body: &str) -> Option<u64> {
    number_before(body, "字下げ").or_else(|| (body == "ここから字下げ").then_some(1))
}

fn append_jisage_clause_partitions(body: &str, facts: &mut Vec<String>) {
    for (present, partition) in [
        (
            body.contains("小さい活字") || body.contains("ゴシック") || body.contains("ポイント"),
            "clause:typography",
        ),
        (
            body.contains("左右中央") || body.contains("中央揃え"),
            "clause:center",
        ),
        (
            body.contains('罫') || body.contains('枠') || body.contains("表組み"),
            "clause:frame-or-table",
        ),
        (body.contains("数式"), "clause:formula"),
        (
            body.contains("横書き") || body.contains("横組み"),
            "clause:horizontal",
        ),
        (
            body.contains("ただし") || body.contains("底本では"),
            "clause:exception",
        ),
    ] {
        if present {
            facts.push(partition.to_owned());
        }
    }
}

fn jisage_open_spelling(body: &str) -> &'static str {
    if body.starts_with("ここから最後まで") {
        "until-body-end"
    } else if body.starts_with("ここより") {
        "koko-yori"
    } else if body.starts_with("以下") {
        "ika"
    } else {
        "koko-kara"
    }
}

fn jisage_close_spelling(body: &str) -> &'static str {
    if body.contains("おわり") {
        "hiragana-owari"
    } else if body.contains("終り") {
        "abbreviated-owari"
    } else if body.contains("ここまで") {
        "koko-made"
    } else if body.contains("文字下げ") {
        "moji-sage"
    } else if body.contains("地下げ") {
        "chi-sage"
    } else if body.starts_with("ここで") {
        "koko-de"
    } else {
        "bare"
    }
}

fn number_before(body: &str, suffix: &str) -> Option<u64> {
    let end = body.find(suffix)?;
    let prefix = body[..end].trim_end();
    let start = prefix
        .char_indices()
        .rev()
        .find(|(_, character)| !is_japanese_number_character(*character))
        .map_or(0, |(offset, character)| offset + character.len_utf8());
    japanese_number(&prefix[start..])
}

fn is_japanese_number_character(character: char) -> bool {
    character.is_ascii_digit()
        || ('０'..='９').contains(&character)
        || matches!(
            character,
            '〇' | '一' | '二' | '三' | '四' | '五' | '六' | '七' | '八' | '九' | '十'
        )
}

fn japanese_number(value: &str) -> Option<u64> {
    if let Some(value) = decimal_value(value) {
        return Some(value);
    }
    let mut total = 0_u64;
    let mut digit = None;
    for character in value.chars() {
        match character {
            '〇' => digit = Some(0),
            '一' => digit = Some(1),
            '二' => digit = Some(2),
            '三' => digit = Some(3),
            '四' => digit = Some(4),
            '五' => digit = Some(5),
            '六' => digit = Some(6),
            '七' => digit = Some(7),
            '八' => digit = Some(8),
            '九' => digit = Some(9),
            '十' => {
                total += digit.take().unwrap_or(1) * 10;
            }
            _ => return None,
        }
    }
    digit.map_or_else(|| (total > 0).then_some(total), |digit| Some(total + digit))
}

fn basic_indentation_form(body: &str) -> Option<(&'static str, Option<u64>)> {
    if body == "地付き" {
        return Some(("line-align-end", None));
    }
    if let Some(digits) = body
        .strip_prefix("ここから")
        .and_then(|rest| rest.strip_suffix("字下げ"))
    {
        return decimal_value(digits).map(|amount| ("block-indent", Some(amount)));
    }
    body.strip_suffix("字下げ")
        .and_then(decimal_value)
        .map(|amount| ("line-indent", Some(amount)))
}

fn decimal_value(digits: &str) -> Option<u64> {
    (!digits.is_empty())
        .then(|| {
            digits.chars().try_fold(0_u64, |value, character| {
                let digit = match character {
                    '0'..='9' => u64::from(character as u8 - b'0'),
                    '０'..='９' => u64::from(character as u32 - '０' as u32),
                    _ => return None,
                };
                value.checked_mul(10)?.checked_add(digit)
            })
        })
        .flatten()
}

fn is_indentation_close(body: &str) -> bool {
    !body.starts_with("ここから")
        && body.contains("字下げ")
        && (body.contains("終わり")
            || body.contains("終り")
            || body.contains("おわり")
            || body.contains("ここまで"))
}

fn marker_line_placement(source: &str, start: usize, marker_len: usize) -> &'static str {
    let line_start = source[..start].rfind('\n').map_or(0, |newline| newline + 1);
    let end = start + marker_len;
    let line_end = source[end..]
        .find('\n')
        .map_or(source.len(), |newline| end + newline);
    let before = source[line_start..start].trim_matches([' ', '　', '\r']);
    let after = source[end..line_end].trim_matches([' ', '　', '\r']);
    match (before.is_empty(), after.is_empty()) {
        (true, true) => "standalone-line",
        (true, false) => "line-prefix",
        (false, _) => "inline",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HeadingSemanticForm {
    Forward,
    PairedOpen,
    PairedClose,
    BlockOpen,
    BlockClose,
    LegacyTypography,
}

impl HeadingSemanticForm {
    const fn partition(self) -> &'static str {
        match self {
            Self::Forward => "forward",
            Self::PairedOpen => "paired-open",
            Self::PairedClose => "paired-close",
            Self::BlockOpen => "block-open",
            Self::BlockClose => "block-close",
            Self::LegacyTypography => "legacy-typography",
        }
    }
}

#[derive(Clone)]
struct ParsedHeadingMarker {
    form: HeadingSemanticForm,
    level: Option<u8>,
    relation: Option<&'static str>,
    target: Option<String>,
}

#[derive(Clone)]
struct SelectedHeadingMarker {
    start: usize,
    form: HeadingSemanticForm,
    level: Option<u8>,
    relation: Option<&'static str>,
    target: Option<String>,
    occurrence: SourceMarkerOccurrence,
}

pub(crate) fn heading_semantic_partitions(
    source: &str,
    work_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    let (body_range, _) = sanitized_body_range(source);
    let mut selected = source_markers(source)
        .into_iter()
        .filter(|marker| {
            body_range.start <= marker.span.start
                && marker.span.start < body_range.end
                && matching_rows(marker.raw, patterns)
                    .iter()
                    .any(|row| row == "heading.basic")
        })
        .filter_map(|marker| {
            let parsed = heading_semantic_form(marker.body)?;
            Some(SelectedHeadingMarker {
                start: marker.span.start,
                form: parsed.form,
                level: parsed.level,
                relation: parsed.relation,
                target: parsed.target,
                occurrence: SourceMarkerOccurrence {
                    work_id: work_id.to_owned(),
                    indexed_path: None,
                    line: marker.span.line,
                    kind: format!("{:?}", marker.kind),
                    raw: marker.raw.to_owned(),
                    body: marker.body.to_owned(),
                    byte_start: None,
                },
            })
        })
        .collect::<Vec<_>>();
    selected.sort_by_key(|marker| marker.start);

    let mut boundaries = BTreeMap::<usize, &'static str>::new();
    let mut target_structures = BTreeMap::<usize, &'static str>::new();
    let mut stack = Vec::<usize>::new();
    for (index, marker) in selected.iter().enumerate() {
        let scope = match marker.form {
            HeadingSemanticForm::PairedOpen | HeadingSemanticForm::PairedClose => Some("paired"),
            HeadingSemanticForm::BlockOpen | HeadingSemanticForm::BlockClose => Some("block"),
            _ => None,
        };
        match marker.form {
            HeadingSemanticForm::PairedOpen | HeadingSemanticForm::BlockOpen => stack.push(index),
            HeadingSemanticForm::PairedClose | HeadingSemanticForm::BlockClose => {
                let Some(&open_index) = stack.last() else {
                    boundaries.insert(marker.start, "orphan-close");
                    continue;
                };
                let open = &selected[open_index];
                let open_scope = match open.form {
                    HeadingSemanticForm::PairedOpen => "paired",
                    HeadingSemanticForm::BlockOpen => "block",
                    _ => unreachable!("heading stack contains only open markers"),
                };
                let level_matches = marker.level.is_none() || marker.level == open.level;
                if scope == Some(open_scope) && level_matches {
                    stack.pop();
                    boundaries.insert(open.start, "matched-close");
                    boundaries.insert(marker.start, "matched-open");
                    let content_start = open.start + open.occurrence.raw.len();
                    let target = source.get(content_start..marker.start).unwrap_or("");
                    let structure = ruby_content_class(target);
                    target_structures.insert(open.start, structure);
                    target_structures.insert(marker.start, structure);
                } else {
                    boundaries.insert(marker.start, "mismatched-close");
                }
            }
            _ => {}
        }
    }
    for index in stack {
        boundaries.insert(selected[index].start, "missing-close");
    }

    let mut partitions = Vec::new();
    for marker in selected {
        let mut facts = vec![
            format!("form:{}", marker.form.partition()),
            format!(
                "placement:{}",
                marker_line_placement(source, marker.start, marker.occurrence.raw.len())
            ),
        ];
        if let Some(level) = marker.level {
            facts.push(format!("level:{level}"));
        }
        match marker.form {
            HeadingSemanticForm::Forward => {
                let target = marker.target.as_deref().unwrap_or("");
                let prefix = source.get(..marker.start).unwrap_or("");
                let visible_target = comparison_lossy_body(target);
                let (referent, target_source) = if target.is_empty() {
                    ("missing", target)
                } else if prefix.ends_with(target) {
                    ("adjacent-plain", target)
                } else if let Some(rich) = shortest_visible_suffix(prefix, &visible_target) {
                    ("adjacent-rich", rich)
                } else if comparison_lossy_body(prefix).contains(visible_target.as_ref()) {
                    ("detached", target)
                } else {
                    ("missing", target)
                };
                facts.extend([
                    "scope:line".to_owned(),
                    format!("relation:{}", marker.relation.unwrap_or("unknown")),
                    format!("referent:{referent}"),
                    format!("target-structure:{}", ruby_content_class(target_source)),
                ]);
            }
            HeadingSemanticForm::PairedOpen | HeadingSemanticForm::BlockOpen => {
                facts.extend([
                    format!(
                        "scope:{}",
                        if marker.form == HeadingSemanticForm::PairedOpen {
                            "inline-range"
                        } else {
                            "block"
                        }
                    ),
                    format!(
                        "boundary:{}",
                        boundaries
                            .get(&marker.start)
                            .copied()
                            .unwrap_or("missing-close")
                    ),
                ]);
                if let Some(structure) = target_structures.get(&marker.start) {
                    facts.push(format!("target-structure:{structure}"));
                }
            }
            HeadingSemanticForm::PairedClose | HeadingSemanticForm::BlockClose => {
                facts.extend([
                    "scope:boundary".to_owned(),
                    "target:active-heading".to_owned(),
                    format!(
                        "boundary:{}",
                        boundaries
                            .get(&marker.start)
                            .copied()
                            .unwrap_or("orphan-close")
                    ),
                ]);
                if marker.occurrence.body.contains("見出終わり") {
                    facts.push("spelling:abbreviated-mi-dashi".to_owned());
                }
                if marker.level.is_none() {
                    facts.push("level:inherited".to_owned());
                }
                if let Some(structure) = target_structures.get(&marker.start) {
                    facts.push(format!("target-structure:{structure}"));
                }
            }
            HeadingSemanticForm::LegacyTypography => {
                facts.extend([
                    "scope:line".to_owned(),
                    "target:same-line-preceding".to_owned(),
                    format!(
                        "typography:{}",
                        if marker.occurrence.body.starts_with('小') {
                            "small-heading"
                        } else {
                            "heading"
                        }
                    ),
                ]);
            }
        }
        facts.sort();
        facts.dedup();
        partitions.extend(
            facts
                .into_iter()
                .map(|fact| (marker.start, fact, marker.occurrence.clone())),
        );
    }
    partitions
}

fn heading_semantic_form(body: &str) -> Option<ParsedHeadingMarker> {
    if matches!(body, "見出し文字" | "小見出し文字") {
        return Some(ParsedHeadingMarker {
            form: HeadingSemanticForm::LegacyTypography,
            level: None,
            relation: None,
            target: None,
        });
    }
    for (keyword, level) in [("大見出し", 1), ("中見出し", 2), ("小見出し", 3)] {
        for (suffix, relation) in [
            (format!("は{keyword}"), "wa"),
            (format!("に{keyword}"), "ni"),
        ] {
            if let Some(target) = body
                .strip_suffix(&suffix)
                .and_then(|value| value.strip_prefix('「'))
                .and_then(|value| value.strip_suffix('」'))
            {
                return Some(ParsedHeadingMarker {
                    form: HeadingSemanticForm::Forward,
                    level: Some(level),
                    relation: Some(relation),
                    target: Some(target.to_owned()),
                });
            }
        }
    }
    if let Some(rest) = body.strip_prefix("ここから") {
        return heading_level(rest).map(|level| ParsedHeadingMarker {
            form: HeadingSemanticForm::BlockOpen,
            level: Some(level),
            relation: None,
            target: None,
        });
    }
    if let Some(rest) = body
        .strip_prefix("ここで")
        .and_then(|value| value.strip_suffix("終わり"))
    {
        let level = if rest == "見出し" {
            None
        } else {
            heading_close_level(rest)
        };
        return (rest == "見出し" || level.is_some()).then_some(ParsedHeadingMarker {
            form: HeadingSemanticForm::BlockClose,
            level,
            relation: None,
            target: None,
        });
    }
    if let Some(rest) = body.strip_suffix("終わり") {
        return heading_close_level(rest).map(|level| ParsedHeadingMarker {
            form: HeadingSemanticForm::PairedClose,
            level: Some(level),
            relation: None,
            target: None,
        });
    }
    heading_level(body).map(|level| ParsedHeadingMarker {
        form: HeadingSemanticForm::PairedOpen,
        level: Some(level),
        relation: None,
        target: None,
    })
}

fn heading_level(body: &str) -> Option<u8> {
    match body {
        "大見出し" => Some(1),
        "中見出し" => Some(2),
        "小見出し" => Some(3),
        _ => None,
    }
}

fn heading_close_level(body: &str) -> Option<u8> {
    match body {
        "大見出し" | "大見出" => Some(1),
        "中見出し" | "中見出" => Some(2),
        "小見出し" | "小見出" => Some(3),
        _ => None,
    }
}

pub(crate) fn ruby_semantic_partitions(
    source: &str,
    work_id: &str,
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    let (body, _) = sanitized_body_range(source);
    let markers = source_markers(source);
    markers
        .iter()
        .enumerate()
        .filter(|(_, marker)| {
            body.start <= marker.span.start
                && marker.span.start < body.end
                && matches!(
                    marker.kind,
                    SourceMarkerKind::RubyExplicit | SourceMarkerKind::RubyImplicit
                )
        })
        .flat_map(|(index, marker)| {
            let partitions = match marker.kind {
                SourceMarkerKind::RubyExplicit => {
                    let base = marker
                        .body
                        .rsplit_once('《')
                        .map_or(marker.body, |(base, _)| base);
                    vec![
                        "form:explicit".to_owned(),
                        format!("base-content:{}", explicit_ruby_base_class(base)),
                        format!(
                            "reading-content:{}",
                            ruby_content_class(
                                marker.body.rsplit_once('《').map_or("", |(_, reading)| {
                                    reading.strip_suffix('》').unwrap_or(reading)
                                })
                            )
                        ),
                    ]
                }
                SourceMarkerKind::RubyImplicit => vec![
                    "form:implicit".to_owned(),
                    format!(
                        "preceding-class:{}",
                        implicit_ruby_preceding_class(
                            source,
                            marker.span.start,
                            index
                                .checked_sub(1)
                                .and_then(|previous| markers.get(previous)),
                        )
                    ),
                    format!("reading-content:{}", ruby_content_class(marker.body)),
                ],
                _ => unreachable!("filtered ruby marker"),
            };
            let occurrence = SourceMarkerOccurrence {
                work_id: work_id.to_owned(),
                indexed_path: None,
                line: marker.span.line,
                kind: format!("{:?}", marker.kind),
                raw: marker.raw.to_owned(),
                body: marker.body.to_owned(),
                byte_start: None,
            };
            partitions
                .into_iter()
                .map(move |partition| (marker.span.start, partition, occurrence.clone()))
        })
        .collect()
}

pub(crate) fn directional_ruby_semantic_partitions(
    source: &str,
    work_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    let (body, _) = sanitized_body_range(source);
    let mut markers = source_markers(source)
        .iter()
        .flat_map(|marker| {
            matching_variant_occurrences(
                marker,
                work_id,
                "ruby.placement_directional",
                patterns,
                0,
                0,
            )
        })
        .filter(|marker| body.start <= marker.start && marker.start < body.end)
        .collect::<Vec<_>>();
    markers.sort_by_key(|marker| marker.start);
    markers.dedup_by(|right, left| {
        right.start == left.start && right.occurrence.raw == left.occurrence.raw
    });

    let mut partitions = Vec::new();
    let mut paired_opens = Vec::<MatchedVariantOccurrence>::new();
    for marker in markers {
        let occurrence = marker.occurrence.clone();
        if occurrence.body == "左にルビ付き" {
            partitions.push((marker.start, "form:paired".to_owned(), occurrence.clone()));
            paired_opens.push(marker);
            continue;
        }
        if let Some(reading) = occurrence
            .body
            .strip_prefix("左に「")
            .and_then(|body| body.strip_suffix("」のルビ付き終わり"))
        {
            partitions.push((marker.start, "form:paired".to_owned(), occurrence.clone()));
            partitions.push((
                marker.start,
                format!("reading-content:{}", ruby_content_class(reading)),
                occurrence.clone(),
            ));
            partitions.push((
                marker.start,
                format!("reading-notation:{}", reading_notation(reading)),
                occurrence.clone(),
            ));
            if let Some(open) = paired_opens.pop() {
                let target_start = open.start + open.occurrence.raw.len();
                let target = source.get(target_start..marker.start).unwrap_or("");
                for (start, paired_occurrence) in [
                    (open.start, open.occurrence.clone()),
                    (marker.start, occurrence.clone()),
                ] {
                    partitions.push((
                        start,
                        "boundary:matched".to_owned(),
                        paired_occurrence.clone(),
                    ));
                    partitions.push((
                        start,
                        format!("target-structure:{}", ruby_content_class(target)),
                        paired_occurrence,
                    ));
                }
            } else {
                partitions.push((marker.start, "boundary:orphan-close".to_owned(), occurrence));
            }
            continue;
        }

        let operands = quoted_operands(&occurrence.body);
        if operands.len() < 2 || !occurrence.body.contains("の左に") {
            continue;
        }
        let base = &operands[0];
        let reading = operands.last().expect("two quoted operands");
        let prefix = source.get(..marker.start).unwrap_or("");
        let visible_base = comparison_lossy_body(base);
        let (referent, target_source) = if prefix.ends_with(base) {
            ("adjacent-plain", base.as_str())
        } else if let Some(target) = shortest_visible_suffix(prefix, &visible_base) {
            ("adjacent-rich", target)
        } else if comparison_lossy_body(prefix).contains(visible_base.as_ref()) {
            ("detached", base.as_str())
        } else {
            ("missing", base.as_str())
        };
        for partition in [
            "form:forward".to_owned(),
            format!("referent:{referent}"),
            format!("target-structure:{}", ruby_content_class(target_source)),
            format!("base-content:{}", ruby_content_class(base)),
            format!("reading-content:{}", ruby_content_class(reading)),
            format!("reading-notation:{}", reading_notation(reading)),
        ] {
            partitions.push((marker.start, partition, occurrence.clone()));
        }
    }
    partitions.extend(paired_opens.into_iter().flat_map(|open| {
        [
            (
                open.start,
                "boundary:orphan-open".to_owned(),
                open.occurrence.clone(),
            ),
            (
                open.start,
                "target-structure:unbounded".to_owned(),
                open.occurrence,
            ),
        ]
    }));
    partitions
}

pub(crate) fn double_ruby_semantic_partitions(
    source: &str,
    work_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    let (body, _) = sanitized_body_range(source);
    let Some(row) = patterns
        .iter()
        .find(|pattern| pattern.row_id == "ruby.double")
    else {
        return Vec::new();
    };
    let mut matches = row
        .source_patterns
        .iter()
        .flat_map(|pattern| pattern.find_iter(source))
        .filter(|matched| body.start <= matched.start() && matched.start() < body.end)
        .map(|matched| (matched.start(), matched.end()))
        .collect::<Vec<_>>();
    matches.sort_unstable();
    matches.dedup();

    matches
        .into_iter()
        .flat_map(|(start, end)| {
            let raw = &source[start..end];
            let markers = source_markers(raw);
            let occurrence = SourceMarkerOccurrence {
                work_id: work_id.to_owned(),
                indexed_path: None,
                line: source[..start]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count()
                    + 1,
                kind: "DoubleRubyComposite".to_owned(),
                raw: raw.to_owned(),
                body: raw.to_owned(),
                byte_start: None,
            };
            let mut partitions = vec!["boundary:matched".to_owned()];

            if raw.starts_with('｜') {
                let Some(first) = markers.first() else {
                    return Vec::new();
                };
                let Some((base, first_reading)) = first.body.rsplit_once('《') else {
                    return Vec::new();
                };
                let first_reading = first_reading.strip_suffix('》').unwrap_or(first_reading);
                let second_reading = markers.get(1).map_or("", |marker| marker.body);
                partitions.extend([
                    "form:consecutive".to_owned(),
                    "referent:explicit".to_owned(),
                    "placement:right-left".to_owned(),
                    format!("target-structure:{}", ruby_content_class(base)),
                    format!("base-content:{}", explicit_ruby_base_class(base)),
                    format!(
                        "right-reading-content:{}",
                        ruby_content_class(first_reading)
                    ),
                    format!(
                        "left-reading-content:{}",
                        ruby_content_class(second_reading)
                    ),
                    format!("right-reading-notation:{}", reading_notation(first_reading)),
                    format!("left-reading-notation:{}", reading_notation(second_reading)),
                ]);
            } else {
                let Some(reading_marker) = markers.iter().find(|marker| {
                    matches!(
                        marker.kind,
                        SourceMarkerKind::RubyExplicit | SourceMarkerKind::RubyImplicit
                    )
                }) else {
                    return Vec::new();
                };
                let Some(command) = markers.iter().find(|marker| {
                    matches!(
                        marker.kind,
                        SourceMarkerKind::CommandFullwidth | SourceMarkerKind::CommandAscii
                    )
                }) else {
                    return Vec::new();
                };
                let operands = quoted_operands(command.body);
                if operands.len() < 2 {
                    return Vec::new();
                }
                let base = &operands[0];
                let second_reading = operands.last().expect("two quoted operands");
                let visible_base = comparison_lossy_body(base);
                let prefix = source.get(..start).unwrap_or("");
                let (referent, target_source) = if prefix.ends_with(base) {
                    ("adjacent-plain", base.as_str())
                } else if let Some(target) = shortest_visible_suffix(prefix, &visible_base) {
                    ("adjacent-rich", target)
                } else if comparison_lossy_body(prefix).contains(visible_base.as_ref()) {
                    ("detached", base.as_str())
                } else {
                    ("missing", base.as_str())
                };
                let side = if command.body.contains("の左に") {
                    "left"
                } else {
                    "right"
                };
                partitions.extend([
                    "form:directional".to_owned(),
                    format!("referent:{referent}"),
                    format!("placement:right-{side}"),
                    format!("target-structure:{}", ruby_content_class(target_source)),
                    format!("base-content:{}", ruby_content_class(base)),
                    format!(
                        "right-reading-content:{}",
                        ruby_content_class(reading_marker.body)
                    ),
                    format!(
                        "{side}-reading-content:{}",
                        ruby_content_class(second_reading)
                    ),
                    format!(
                        "right-reading-notation:{}",
                        reading_notation(reading_marker.body)
                    ),
                    format!(
                        "{side}-reading-notation:{}",
                        reading_notation(second_reading)
                    ),
                ]);
            }

            partitions
                .into_iter()
                .map(|partition| (start, partition, occurrence.clone()))
                .collect::<Vec<_>>()
        })
        .collect()
}

pub(crate) fn nested_reading_ruby_occurrences(
    source: &str,
    work_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, usize, SourceMarkerOccurrence)> {
    let (body, _) = sanitized_body_range(source);
    let Some(row) = patterns
        .iter()
        .find(|pattern| pattern.row_id == "ruby.nested_forbidden")
    else {
        return Vec::new();
    };
    let lexically_recognized = source_markers(source)
        .into_iter()
        .filter(|marker| {
            matching_rows(marker.raw, patterns)
                .iter()
                .any(|family| family == "ruby.nested_forbidden")
        })
        .map(|marker| (marker.span.start, marker.span.end))
        .collect::<Vec<_>>();
    let mut matches = row
        .source_patterns
        .iter()
        .flat_map(|pattern| pattern.find_iter(source))
        .filter(|matched| {
            matched.as_str().starts_with('｜')
                && body.start <= matched.start()
                && matched.start() < body.end
                && !lexically_recognized.contains(&(matched.start(), matched.end()))
        })
        .map(|matched| (matched.start(), matched.end()))
        .collect::<Vec<_>>();
    matches.sort_unstable();
    matches.dedup();
    matches
        .into_iter()
        .map(|(start, end)| {
            let raw = &source[start..end];
            (
                start,
                end,
                SourceMarkerOccurrence {
                    work_id: work_id.to_owned(),
                    indexed_path: None,
                    line: source[..start]
                        .bytes()
                        .filter(|byte| *byte == b'\n')
                        .count()
                        + 1,
                    kind: "NestedRuby".to_owned(),
                    raw: raw.to_owned(),
                    body: raw.to_owned(),
                    byte_start: None,
                },
            )
        })
        .collect()
}

fn shortest_visible_suffix<'a>(prefix: &'a str, visible: &str) -> Option<&'a str> {
    prefix
        .char_indices()
        .rev()
        .map(|(start, _)| &prefix[start..])
        .find(|candidate| comparison_lossy_body(candidate) == visible)
}

fn reading_notation(reading: &str) -> &'static str {
    if reading.contains('？') || reading.contains('?') {
        "question-mark"
    } else {
        "plain"
    }
}

pub(crate) fn gaiji_ruby_semantic_partitions(
    source: &str,
    work_id: &str,
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    let (body, _) = sanitized_body_range(source);
    let markers = source_markers(source);
    markers
        .iter()
        .enumerate()
        .filter(|(_, marker)| {
            body.start <= marker.span.start
                && marker.span.start < body.end
                && matches!(
                    marker.kind,
                    SourceMarkerKind::RubyExplicit | SourceMarkerKind::RubyImplicit
                )
        })
        .filter_map(|(index, ruby)| {
            let (form, base, gaiji) = match ruby.kind {
                SourceMarkerKind::RubyExplicit => {
                    let base = ruby
                        .body
                        .rsplit_once('《')
                        .map_or(ruby.body, |(base, _)| base);
                    if !base.contains("※［") && !base.contains("※[") {
                        return None;
                    }
                    let gaiji = source_markers(base)
                        .into_iter()
                        .filter(|marker| {
                            matches!(
                                marker.kind,
                                SourceMarkerKind::GaijiFullwidth | SourceMarkerKind::GaijiAscii
                            )
                        })
                        .collect::<Vec<_>>();
                    ("explicit", base, gaiji)
                }
                SourceMarkerKind::RubyImplicit => {
                    let gaiji = index
                        .checked_sub(1)
                        .and_then(|previous| markers.get(previous))
                        .filter(|marker| {
                            marker.span.end == ruby.span.start
                                && matches!(
                                    marker.kind,
                                    SourceMarkerKind::GaijiFullwidth | SourceMarkerKind::GaijiAscii
                                )
                        })
                        .cloned()?;
                    let gaiji = vec![gaiji];
                    ("implicit", "", gaiji)
                }
                _ => unreachable!("filtered ruby marker"),
            };
            if gaiji.is_empty() {
                return None;
            }
            let partition_start = if form == "implicit" {
                gaiji[0].span.start
            } else {
                ruby.span.start
            };
            let raw = &source[partition_start..ruby.span.end];
            let mut partitions = vec![
                format!("form:{form}"),
                format!(
                    "gaiji-count:{}",
                    if gaiji.len() == 1 {
                        "single"
                    } else {
                        "multiple"
                    }
                ),
                format!(
                    "base-shape:{}",
                    if form == "implicit" || gaiji_cover_base(base, &gaiji) {
                        "gaiji-only"
                    } else {
                        "mixed"
                    }
                ),
            ];
            let syntaxes = gaiji
                .iter()
                .map(|marker| match marker.kind {
                    SourceMarkerKind::GaijiFullwidth => "fullwidth",
                    SourceMarkerKind::GaijiAscii => "ascii",
                    _ => unreachable!("filtered gaiji marker"),
                })
                .collect::<std::collections::BTreeSet<_>>();
            partitions.push(format!(
                "marker-syntax:{}",
                if syntaxes.len() == 1 {
                    syntaxes.iter().next().expect("one syntax")
                } else {
                    "mixed"
                }
            ));
            let identifiers = gaiji
                .iter()
                .flat_map(|marker| gaiji_identifier_classes(marker.body))
                .collect::<std::collections::BTreeSet<_>>();
            partitions.extend(
                identifiers
                    .into_iter()
                    .map(|identifier| format!("identifier:{identifier}")),
            );
            let occurrence = SourceMarkerOccurrence {
                work_id: work_id.to_owned(),
                indexed_path: None,
                line: ruby.span.line,
                kind: format!("{:?}", ruby.kind),
                raw: raw.to_owned(),
                body: ruby.body.to_owned(),
                byte_start: None,
            };
            Some(
                partitions
                    .into_iter()
                    .map(move |partition| (partition_start, partition, occurrence.clone())),
            )
        })
        .flatten()
        .collect()
}

pub(crate) fn accent_semantic_partitions(
    source: &str,
    work_id: &str,
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    let (body, _) = sanitized_body_range(source);
    source_markers(source)
        .into_iter()
        .filter(|marker| {
            body.start <= marker.span.start
                && marker.span.start < body.end
                && marker.kind == SourceMarkerKind::AccentNotation
        })
        .flat_map(|marker| {
            let sites = decompose_fragment_sites(marker.body);
            let unresolved = unresolved_accent_sites(marker.body);
            let mut partitions = vec![
                format!(
                    "substitutions:{}",
                    match sites.len() {
                        0 => "none",
                        1 => "single",
                        _ => "multiple",
                    }
                ),
                format!(
                    "resolution:{}",
                    match (sites.is_empty(), unresolved.is_empty()) {
                        (false, true) => "complete",
                        (false, false) => "partial",
                        (true, false) => "none",
                        (true, true) => "literal-scope",
                    }
                ),
            ];
            if !sites.is_empty() {
                let widths = sites
                    .iter()
                    .map(|(_, width, _)| if *width == 3 { "ligature" } else { "digraph" })
                    .collect::<std::collections::BTreeSet<_>>();
                partitions.push(format!(
                    "spelling-width:{}",
                    if widths.len() == 1 {
                        widths.iter().next().expect("one spelling width")
                    } else {
                        "mixed"
                    }
                ));
                let cases = sites
                    .iter()
                    .filter_map(|(offset, _, _)| marker.body.as_bytes().get(*offset))
                    .map(|base| {
                        if base.is_ascii_uppercase() {
                            "upper"
                        } else {
                            "lower"
                        }
                    })
                    .collect::<std::collections::BTreeSet<_>>();
                partitions.push(format!(
                    "letter-case:{}",
                    if cases.len() == 1 {
                        cases.iter().next().expect("one letter case")
                    } else {
                        "mixed"
                    }
                ));
                let byte_effects = sites
                    .iter()
                    .map(
                        |(_, width, resolved)| match resolved.len_utf8().cmp(width) {
                            std::cmp::Ordering::Less => "shrinks",
                            std::cmp::Ordering::Equal => "preserves",
                            std::cmp::Ordering::Greater => "grows",
                        },
                    )
                    .collect::<std::collections::BTreeSet<_>>();
                partitions.push(format!(
                    "byte-effect:{}",
                    if byte_effects.len() == 1 {
                        byte_effects.iter().next().expect("one byte effect")
                    } else {
                        "mixed"
                    }
                ));
                let markers = sites
                    .iter()
                    .filter_map(|(offset, width, _)| {
                        marker.body.as_bytes().get(offset + width - 1).copied()
                    })
                    .map(accent_marker_name)
                    .collect::<std::collections::BTreeSet<_>>();
                partitions.extend(markers.into_iter().map(|accent| format!("marker:{accent}")));
            }
            let occurrence = SourceMarkerOccurrence {
                work_id: work_id.to_owned(),
                indexed_path: None,
                line: marker.span.line,
                kind: format!("{:?}", marker.kind),
                raw: marker.raw.to_owned(),
                body: marker.body.to_owned(),
                byte_start: None,
            };
            partitions
                .into_iter()
                .map(move |partition| (marker.span.start, partition, occurrence.clone()))
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NestedSourceRole {
    Body,
    RubyBase,
    RubyReading,
    DirectiveReference,
    EditorialApparatus,
    BracketNotation,
    AccentNotation,
}

impl NestedSourceRole {
    const fn partition(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::RubyBase => "ruby-base",
            Self::RubyReading => "ruby-reading",
            Self::DirectiveReference => "directive-reference",
            Self::EditorialApparatus => "editorial-apparatus",
            Self::BracketNotation => "bracket-notation",
            Self::AccentNotation => "accent-notation",
        }
    }
}

#[derive(Clone, Copy)]
struct SourceContainer {
    kind: SourceMarkerKind,
    start: usize,
    end: usize,
    reading_start: Option<usize>,
}

pub(crate) fn iteration_semantic_partitions(
    source: &str,
    work_id: &str,
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    let (body, _) = sanitized_body_range(source);
    let mut containers = Vec::new();
    collect_source_containers(source, 0, &mut containers);
    let mut partitions = Vec::new();
    let mut offset = body.start;
    while offset < body.end {
        let rest = &source[offset..body.end];
        let (raw, form) = if rest.starts_with("／″＼") {
            ("／″＼", "voiced")
        } else if rest.starts_with("／＼") {
            ("／＼", "unvoiced")
        } else {
            offset += rest
                .chars()
                .next()
                .expect("body offset is in bounds")
                .len_utf8();
            continue;
        };
        let role = source_role_at(offset, &containers);
        let occurrence = SourceMarkerOccurrence {
            work_id: work_id.to_owned(),
            indexed_path: None,
            line: 1 + source[..offset]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count(),
            kind: "IterationNotation".to_owned(),
            raw: raw.to_owned(),
            body: raw.to_owned(),
            byte_start: None,
        };
        for partition in [format!("form:{form}"), format!("role:{}", role.partition())] {
            partitions.push((offset, partition, occurrence.clone()));
        }
        if note_target_splits_mark(source, offset, raw.len()) {
            partitions.push((
                offset,
                "boundary:split-by-note-target".to_owned(),
                occurrence,
            ));
        }
        offset += raw.len();
    }
    partitions
}

/// Whether the quoted target of the first annotation after the mark at
/// `start` begins with the mark's second half, as in ／＼ゐ［＃「＼ゐ」に「マヽ」の注記］.
fn note_target_splits_mark(source: &str, start: usize, len: usize) -> bool {
    let second = start + '／'.len_utf8();
    let after = &source[start + len..];
    let line = &after[..after.find('\n').unwrap_or(after.len())];
    let Some(note) = line.find("［＃「") else {
        return false;
    };
    let quoted = &line[note + "［＃「".len()..];
    let note = start + len + note;
    quoted
        .find('」')
        .is_some_and(|close| quoted[..close] == source[second..note])
}

pub(crate) fn gaiji_semantic_partitions(
    source: &str,
    work_id: &str,
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    let (body, _) = sanitized_body_range(source);
    let mut gaiji = Vec::new();
    collect_gaiji_occurrences(source, 0, NestedSourceRole::Body, &mut gaiji);
    gaiji
        .into_iter()
        .filter(|(start, _, _, _, _)| body.start <= *start && *start < body.end)
        .flat_map(|(start, kind, raw, marker_body, role)| {
            let syntax = match kind {
                SourceMarkerKind::GaijiFullwidth => "fullwidth",
                SourceMarkerKind::GaijiAscii => "ascii",
                _ => unreachable!("collector only retains gaiji markers"),
            };
            let occurrence = SourceMarkerOccurrence {
                work_id: work_id.to_owned(),
                indexed_path: None,
                line: 1 + source[..start]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count(),
                kind: format!("{kind:?}"),
                raw,
                body: marker_body.clone(),
                byte_start: None,
            };
            let mut partitions = vec![
                format!("marker-syntax:{syntax}"),
                format!("role:{}", role.partition()),
            ];
            partitions.extend(
                gaiji_identifier_classes(&marker_body)
                    .into_iter()
                    .map(|identifier| format!("identifier:{identifier}")),
            );
            partitions
                .into_iter()
                .map(move |partition| (start, partition, occurrence.clone()))
        })
        .collect()
}

pub(crate) fn jis_gaiji_semantic_occurrences(
    source: &str,
    work_id: &str,
) -> Vec<(usize, String, Vec<String>, SourceMarkerOccurrence)> {
    let (body, _) = sanitized_body_range(source);
    let mut gaiji = Vec::new();
    collect_gaiji_occurrences(source, 0, NestedSourceRole::Body, &mut gaiji);
    gaiji
        .into_iter()
        .filter(|(start, _, _, marker_body, _)| {
            body.start <= *start
                && *start < body.end
                && gaiji_identifier_classes(marker_body).contains("jis")
        })
        .map(|(start, kind, raw, marker_body, role)| {
            let form = normalize_jis_gaiji_form(&kind, &marker_body);
            let partitions = jis_gaiji_semantic_partitions(&kind, &marker_body, role);
            let occurrence = SourceMarkerOccurrence {
                work_id: work_id.to_owned(),
                indexed_path: None,
                line: 1 + source[..start]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count(),
                kind: format!("{kind:?}"),
                raw,
                body: marker_body,
                byte_start: None,
            };
            (start, form, partitions, occurrence)
        })
        .collect()
}

fn collect_gaiji_occurrences(
    source: &str,
    base: usize,
    inherited_role: NestedSourceRole,
    occurrences: &mut Vec<(usize, SourceMarkerKind, String, String, NestedSourceRole)>,
) {
    let markers = source_markers(source);
    for (index, marker) in markers.iter().enumerate() {
        let start = base + marker.span.start;
        if matches!(
            marker.kind,
            SourceMarkerKind::GaijiFullwidth | SourceMarkerKind::GaijiAscii
        ) {
            let role = if inherited_role == NestedSourceRole::Body
                && markers.get(index + 1).is_some_and(|next| {
                    marker.span.end == next.span.start
                        && next.kind == SourceMarkerKind::RubyImplicit
                }) {
                NestedSourceRole::RubyBase
            } else {
                inherited_role
            };
            occurrences.push((
                start,
                marker.kind,
                marker.raw.to_owned(),
                marker.body.to_owned(),
                role,
            ));
            continue;
        }
        if !matches!(
            marker.kind,
            SourceMarkerKind::RubyExplicit
                | SourceMarkerKind::RubyImplicit
                | SourceMarkerKind::CommandFullwidth
                | SourceMarkerKind::CommandAscii
                | SourceMarkerKind::AccentNotation
                | SourceMarkerKind::BracketNote
                | SourceMarkerKind::EditorialNoteRubyCorrection
                | SourceMarkerKind::EditorialNoteBottomTextCorrection
        ) || marker.body.len() >= source.len()
        {
            continue;
        }
        let body_offset = marker.raw.find(marker.body).unwrap_or(0);
        collect_gaiji_occurrences(
            marker.body,
            start + body_offset,
            nested_source_role(inherited_role, marker.kind),
            occurrences,
        );
    }
}

fn nested_source_role(parent: NestedSourceRole, kind: SourceMarkerKind) -> NestedSourceRole {
    if parent == NestedSourceRole::EditorialApparatus
        || matches!(
            kind,
            SourceMarkerKind::EditorialNoteRubyCorrection
                | SourceMarkerKind::EditorialNoteBottomTextCorrection
        )
    {
        return NestedSourceRole::EditorialApparatus;
    }
    if parent == NestedSourceRole::DirectiveReference
        || matches!(
            kind,
            SourceMarkerKind::CommandFullwidth | SourceMarkerKind::CommandAscii
        )
    {
        return NestedSourceRole::DirectiveReference;
    }
    match kind {
        SourceMarkerKind::RubyExplicit => NestedSourceRole::RubyBase,
        SourceMarkerKind::RubyImplicit => NestedSourceRole::RubyReading,
        SourceMarkerKind::BracketNote => NestedSourceRole::BracketNotation,
        SourceMarkerKind::AccentNotation => NestedSourceRole::AccentNotation,
        _ => parent,
    }
}

fn collect_source_containers(source: &str, base: usize, containers: &mut Vec<SourceContainer>) {
    for marker in source_markers(source) {
        if !matches!(
            marker.kind,
            SourceMarkerKind::RubyExplicit
                | SourceMarkerKind::RubyImplicit
                | SourceMarkerKind::CommandFullwidth
                | SourceMarkerKind::CommandAscii
                | SourceMarkerKind::AccentNotation
                | SourceMarkerKind::BracketNote
                | SourceMarkerKind::EditorialNoteRubyCorrection
                | SourceMarkerKind::EditorialNoteBottomTextCorrection
        ) {
            continue;
        }
        let start = base + marker.span.start;
        let body_offset = marker.raw.find(marker.body).unwrap_or(0);
        let reading_start = (marker.kind == SourceMarkerKind::RubyExplicit)
            .then(|| {
                marker
                    .raw
                    .find('《')
                    .map(|index| start + index + '《'.len_utf8())
            })
            .flatten();
        containers.push(SourceContainer {
            kind: marker.kind,
            start,
            end: base + marker.span.end,
            reading_start,
        });
        if marker.body.len() < source.len() {
            collect_source_containers(marker.body, start + body_offset, containers);
        }
    }
}

fn source_role_at(position: usize, containers: &[SourceContainer]) -> NestedSourceRole {
    let containing = containers
        .iter()
        .filter(|container| container.start <= position && position < container.end)
        .collect::<Vec<_>>();
    if containing.iter().any(|container| {
        matches!(
            container.kind,
            SourceMarkerKind::EditorialNoteRubyCorrection
                | SourceMarkerKind::EditorialNoteBottomTextCorrection
        )
    }) {
        return NestedSourceRole::EditorialApparatus;
    }
    if containing.iter().any(|container| {
        matches!(
            container.kind,
            SourceMarkerKind::CommandFullwidth | SourceMarkerKind::CommandAscii
        )
    }) {
        return NestedSourceRole::DirectiveReference;
    }
    for container in containing.iter().rev() {
        match container.kind {
            SourceMarkerKind::RubyImplicit => return NestedSourceRole::RubyReading,
            SourceMarkerKind::RubyExplicit => {
                return if container
                    .reading_start
                    .is_some_and(|reading_start| position >= reading_start)
                {
                    NestedSourceRole::RubyReading
                } else {
                    NestedSourceRole::RubyBase
                };
            }
            SourceMarkerKind::BracketNote => return NestedSourceRole::BracketNotation,
            SourceMarkerKind::AccentNotation => return NestedSourceRole::AccentNotation,
            _ => {}
        }
    }
    NestedSourceRole::Body
}

fn accent_marker_name(marker: u8) -> &'static str {
    match marker {
        b'\'' => "acute",
        b'`' => "grave",
        b'^' => "circumflex",
        b':' => "diaeresis",
        b'~' => "tilde",
        b'&' => "ring-or-ligature",
        b',' => "cedilla",
        b'/' => "stroke",
        b'_' => "macron",
        _ => "unknown",
    }
}

fn gaiji_cover_base(base: &str, gaiji: &[SourceMarker<'_>]) -> bool {
    let mut cursor = 0;
    for marker in gaiji {
        if !base[cursor..marker.span.start].trim().is_empty() {
            return false;
        }
        cursor = marker.span.end;
    }
    base[cursor..].trim().is_empty()
}

fn gaiji_identifier_classes(body: &str) -> std::collections::BTreeSet<&'static str> {
    let mut classes = std::collections::BTreeSet::new();
    for clause in body.split(['、', ',']) {
        let clause = clause.trim();
        if clause.contains("U+") || clause.contains("u+") {
            classes.insert("unicode");
        } else if clause.contains("水準") || is_jis_cell(clause) {
            classes.insert("jis");
        } else if clause.chars().any(|character| character.is_ascii_digit()) && clause.contains('-')
        {
            classes.insert("source-location");
        }
    }
    if classes.is_empty() {
        classes.insert("description-only");
    }
    classes
}

fn normalize_jis_gaiji_form(kind: &SourceMarkerKind, body: &str) -> String {
    let syntax = match kind {
        SourceMarkerKind::GaijiFullwidth => ("※［＃", "］"),
        SourceMarkerKind::GaijiAscii => ("※[#", "]"),
        _ => unreachable!("JIS gaiji forms come from gaiji markers"),
    };
    let code = if body.contains("水準") {
        "第N水準N-N-N"
    } else {
        "N-N-N"
    };
    let identifiers = gaiji_identifier_classes(body);
    let mut supplements = Vec::new();
    if identifiers.contains("unicode") {
        supplements.push("U+XXXX");
    }
    if identifiers.contains("source-location") {
        supplements.push("N-N");
    }
    let suffix = if supplements.is_empty() {
        String::new()
    } else {
        format!("、{}", supplements.join("、"))
    };
    format!("{}○○、{code}{suffix}{}", syntax.0, syntax.1)
}

fn jis_gaiji_semantic_partitions(
    kind: &SourceMarkerKind,
    body: &str,
    role: NestedSourceRole,
) -> Vec<String> {
    let mut partitions = vec![
        format!(
            "marker-syntax:{}",
            match kind {
                SourceMarkerKind::GaijiFullwidth => "fullwidth",
                SourceMarkerKind::GaijiAscii => "ascii",
                _ => unreachable!("JIS gaiji partitions come from gaiji markers"),
            }
        ),
        format!("role:{}", role.partition()),
        format!(
            "code-notation:{}",
            if body.contains("水準") {
                "level-prefixed"
            } else {
                "bare-cell"
            }
        ),
    ];
    let identifiers = gaiji_identifier_classes(body);
    let mut reference_kinds = vec!["jis"];
    if identifiers.contains("unicode") {
        reference_kinds.push("unicode");
    }
    if identifiers.contains("source-location") {
        reference_kinds.push("source-location");
    }
    partitions.push(format!("references:{}", reference_kinds.join("+")));

    static CELL_PATTERN: OnceLock<Regex> = OnceLock::new();
    let cell_pattern = CELL_PATTERN.get_or_init(|| {
        Regex::new(r"(?:第([1-4一二三四])水準)?\s*([12])-([0-9]{1,2})-([0-9]{1,2})")
            .expect("JIS cell pattern is valid")
    });
    let mut cell_count = 0_usize;
    let mut planes = std::collections::BTreeSet::new();
    let mut levels = std::collections::BTreeSet::new();
    for captures in cell_pattern.captures_iter(body) {
        cell_count += 1;
        if let Some(plane) = captures.get(2) {
            planes.insert(plane.as_str());
        }
        if let Some(level) = captures.get(1) {
            levels.insert(level.as_str());
        }
    }
    if cell_count == 0 {
        partitions.push("cell:unparsed".to_owned());
    } else {
        for plane in planes {
            partitions.push(format!("plane:{plane}"));
        }
        partitions.push(format!(
            "cell-count:{}",
            if cell_count == 1 { "one" } else { "multiple" }
        ));
        for level in levels {
            partitions.push(format!("level:{level}"));
        }
    }
    partitions
}

fn is_jis_cell(clause: &str) -> bool {
    let components = clause.split('-').map(str::trim).collect::<Vec<_>>();
    components.len() == 3
        && matches!(components[0], "1" | "2")
        && components[1]
            .parse::<u8>()
            .is_ok_and(|row| (1..=94).contains(&row))
        && components[2]
            .parse::<u8>()
            .is_ok_and(|cell| (1..=94).contains(&cell))
}

fn ruby_content_class(source: &str) -> &'static str {
    if source.is_empty() {
        return "empty";
    }
    if !source
        .chars()
        .any(|character| matches!(character, '※' | '［' | '[' | '〔' | '《' | '／'))
    {
        return "plain";
    }
    let kinds = source_markers(source)
        .into_iter()
        .map(|marker| match marker.kind {
            SourceMarkerKind::IterationNotation => "iteration",
            SourceMarkerKind::GaijiFullwidth | SourceMarkerKind::GaijiAscii => "gaiji",
            SourceMarkerKind::CommandFullwidth | SourceMarkerKind::CommandAscii => "command",
            SourceMarkerKind::AccentNotation => "accent",
            SourceMarkerKind::BracketNote => "bracket",
            SourceMarkerKind::EditorialNoteRubyCorrection
            | SourceMarkerKind::EditorialNoteBottomTextCorrection => "editorial-note",
            SourceMarkerKind::RubyExplicit | SourceMarkerKind::RubyImplicit => "ruby",
            _ => "malformed",
        })
        .collect::<std::collections::BTreeSet<_>>();
    if kinds.is_empty() {
        "plain"
    } else if kinds.len() > 1 {
        "mixed"
    } else {
        kinds.into_iter().next().expect("one content kind")
    }
}

fn explicit_ruby_base_class(source: &str) -> &'static str {
    let content_class = ruby_content_class(source);
    if content_class != "plain" {
        return content_class;
    }
    let visible = comparison_lossy_body(source);
    if visible.chars().any(char::is_whitespace) {
        return "spaced";
    }
    let classes = visible
        .chars()
        .map(ruby_character_class)
        .collect::<std::collections::BTreeSet<_>>();
    if classes.is_empty() {
        "empty"
    } else if classes.len() == 1 {
        classes.into_iter().next().expect("one character class")
    } else {
        "mixed"
    }
}

fn implicit_ruby_preceding_class(
    source: &str,
    marker_start: usize,
    previous_marker: Option<&SourceMarker<'_>>,
) -> &'static str {
    let prefix = &source[..marker_start];
    if previous_marker.is_some_and(|marker| {
        marker.span.end == marker_start
            && matches!(
                marker.kind,
                SourceMarkerKind::GaijiFullwidth | SourceMarkerKind::GaijiAscii
            )
    }) {
        return "gaiji";
    }
    prefix
        .chars()
        .next_back()
        .map_or("none", ruby_character_class)
}

fn ruby_character_class(character: char) -> &'static str {
    if matches!(character, '々' | '仝' | '〆' | '〇' | '〻' | 'ヶ') {
        "special-kanji"
    } else if ('\u{3400}'..='\u{9fff}').contains(&character)
        || ('\u{f900}'..='\u{faff}').contains(&character)
        || ('\u{20000}'..='\u{2ffff}').contains(&character)
    {
        "han"
    } else if ('\u{3041}'..='\u{3096}').contains(&character)
        || ('\u{309d}'..='\u{309e}').contains(&character)
    {
        "hiragana"
    } else if ('\u{30a1}'..='\u{30f4}').contains(&character)
        || ('\u{30f7}'..='\u{30fa}').contains(&character)
        || ('\u{30fc}'..='\u{30fe}').contains(&character)
    {
        "katakana"
    } else if character.is_ascii_alphabetic()
        || ('\u{ff21}'..='\u{ff3a}').contains(&character)
        || ('\u{ff41}'..='\u{ff5a}').contains(&character)
    {
        "latin"
    } else if ('\u{0391}'..='\u{03a9}').contains(&character)
        || ('\u{03b1}'..='\u{03c9}').contains(&character)
    {
        "greek"
    } else if ('А'..='я').contains(&character) || matches!(character, 'Ё' | 'ё') {
        "cyrillic"
    } else if character.is_ascii_digit() || ('\u{ff10}'..='\u{ff19}').contains(&character) {
        "number"
    } else {
        "other"
    }
}

fn emphasis_boundary(body: &str) -> Option<(String, bool)> {
    if body.contains('「') || body.contains('、') {
        return None;
    }
    let closing = body.ends_with("終わり");
    let family = if body.contains("傍点") {
        "boten"
    } else if ["傍線", "鎖線", "破線", "波線"]
        .iter()
        .any(|name| body.contains(name))
    {
        "bousen"
    } else if body.contains("太字") {
        "bold"
    } else if body.contains("斜体") {
        "italic"
    } else {
        return None;
    };
    Some((family.to_owned(), closing))
}

fn emphasis_target_partition(
    source: &str,
    marker_start: usize,
    occurrence: &SourceMarkerOccurrence,
) -> Option<String> {
    let operands = quoted_operands(&occurrence.body);
    if operands.is_empty() {
        return occurrence
            .body
            .contains('、')
            .then(|| "selector:preceding-text;referent:adjacent".to_owned());
    }
    let form = normalize_marker_form(&occurrence.raw);
    let selector = if form.contains("」の間に") {
        "between"
    } else if form.contains("」を除く部分") {
        "exclusion"
    } else if form.contains("」～「") {
        "range"
    } else if form.contains("」の「") {
        "subtarget"
    } else if operands.len() > 1 && form.contains('、') && form.contains("」は") {
        "compound"
    } else if operands.len() > 1 {
        "plural"
    } else {
        "quoted"
    };
    let prefix = source.get(..marker_start)?;
    let counts = operands
        .iter()
        .map(|operand| prefix.matches(operand).count())
        .collect::<Vec<_>>();
    let anchored_relation = emphasis_anchor(prefix, selector, &operands);
    let repeated = counts.iter().any(|count| *count > 1);
    let referent = if let Some(anchored) = anchored_relation {
        if matches!(selector, "between" | "exclusion" | "subtarget") {
            anchored
        } else if repeated {
            "adjacent-repeated"
        } else {
            "adjacent-unique"
        }
    } else {
        let visible_prefix = comparison_lossy_body(prefix);
        let visible_counts = operands
            .iter()
            .map(|operand| visible_prefix.matches(operand).count())
            .collect::<Vec<_>>();
        let visible_anchor = emphasis_anchor(&visible_prefix, selector, &operands);
        let visible_repeated = visible_counts.iter().any(|count| *count > 1);
        if let Some(anchored) = visible_anchor {
            match anchored {
                "adjacent-pair" => "adjacent-rich-pair",
                "adjacent-enclosing" => "adjacent-rich-enclosing",
                _ if visible_repeated => "adjacent-rich-repeated",
                _ => "adjacent-rich-unique",
            }
        } else if !counts.contains(&0) {
            if repeated {
                "detached-repeated"
            } else {
                "detached-unique"
            }
        } else if visible_counts.contains(&0) {
            "missing"
        } else if visible_repeated {
            "detached-rich-repeated"
        } else {
            "detached-rich-unique"
        }
    };
    Some(format!("selector:{selector};referent:{referent}"))
}

fn emphasis_anchor<'a>(prefix: &'a str, selector: &str, operands: &[String]) -> Option<&'a str> {
    match selector {
        "between" => prefix
            .strip_suffix(&operands.concat())
            .map(|_| "adjacent-pair"),
        "exclusion" | "subtarget" => prefix
            .strip_suffix(&operands[0])
            .map(|_| "adjacent-enclosing"),
        _ => operands
            .last()
            .and_then(|operand| prefix.strip_suffix(operand))
            .map(|_| "adjacent"),
    }
}

fn quoted_operands(body: &str) -> Vec<String> {
    let nested = source_markers(body)
        .into_iter()
        .map(|marker| marker.span)
        .collect::<Vec<_>>();
    let mut operands = Vec::new();
    let mut operand = None::<String>;
    let mut offset = 0;
    while offset < body.len() {
        if let Some(span) = nested.iter().find(|span| span.start == offset) {
            if let Some(value) = &mut operand {
                value.push_str(&body[span.start..span.end]);
            }
            offset = span.end;
            continue;
        }
        let character = body[offset..]
            .chars()
            .next()
            .expect("valid character boundary");
        offset += character.len_utf8();
        match (character, &mut operand) {
            ('「', None) => operand = Some(String::new()),
            ('」', Some(_)) => operands.push(operand.take().expect("matched operand")),
            (_, Some(value)) => value.push(character),
            _ => {}
        }
    }
    operands
}

struct MatchedVariantOccurrence {
    occurrence: SourceMarkerOccurrence,
    start: usize,
}

fn matching_variant_occurrences(
    marker: &SourceMarker<'_>,
    work_id: &str,
    row_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
    line_offset: usize,
    byte_offset: usize,
) -> Vec<MatchedVariantOccurrence> {
    let line = line_offset + marker.span.line;
    let start = byte_offset + marker.span.start;
    let mut nested = Vec::new();
    let body_offset = start + marker.raw.find(marker.body).unwrap_or(0);
    for child in source_markers(marker.body) {
        if child.raw.len() >= marker.raw.len() {
            continue;
        }
        nested.extend(matching_variant_occurrences(
            &child,
            work_id,
            row_id,
            patterns,
            line - 1,
            body_offset,
        ));
    }
    if !nested.is_empty() {
        return nested;
    }
    let command = matches!(
        marker.kind,
        SourceMarkerKind::CommandFullwidth | SourceMarkerKind::CommandAscii
    );
    if command
        && matching_rows(marker.raw, patterns)
            .iter()
            .any(|row| row == row_id)
    {
        return vec![MatchedVariantOccurrence {
            occurrence: SourceMarkerOccurrence {
                work_id: work_id.to_owned(),
                indexed_path: None,
                line,
                kind: format!("{:?}", marker.kind),
                raw: marker.raw.to_owned(),
                body: marker.body.to_owned(),
                byte_start: None,
            },
            start,
        }];
    }
    Vec::new()
}

/// The variant form of a ruby marker from its `form:` partition: the reading
/// and an explicit base are masked.
#[must_use]
pub fn ruby_marker_form(partition: &str) -> Option<&'static str> {
    match partition {
        "form:explicit" => Some("｜○○《○○》"),
        "form:implicit" => Some("《○○》"),
        _ => None,
    }
}

/// Collapse operand identity and numeric values while retaining marker grammar.
#[must_use]
pub fn normalize_marker_form(source: &str) -> String {
    let structured = mask_nested_gaiji(source.trim());
    let mut masked = String::with_capacity(structured.len());
    let mut chars = structured.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '「' {
            masked.push('「');
            masked.push_str("○○");
            for nested in chars.by_ref() {
                if nested == '」' {
                    masked.push('」');
                    break;
                }
            }
        } else if character == '"' {
            masked.push_str("「○○」");
            for nested in chars.by_ref() {
                if nested == '"' {
                    break;
                }
            }
        } else if character.is_ascii_digit() || ('０'..='９').contains(&character) {
            masked.push('N');
            while chars
                .peek()
                .is_some_and(|next| next.is_ascii_digit() || ('０'..='９').contains(next))
            {
                chars.next();
            }
        } else if character == 'Ｎ' {
            masked.push('N');
        } else {
            masked.push(character);
        }
    }
    masked
}

fn mask_nested_gaiji(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut offset = 0;
    while offset < source.len() {
        if source[offset..].starts_with("※［＃") {
            let mut cursor = offset + '※'.len_utf8();
            let mut depth = 0usize;
            while cursor < source.len() {
                if source[cursor..].starts_with("［＃") {
                    depth += 1;
                    cursor += "［＃".len();
                } else if source[cursor..].starts_with('］') {
                    cursor += '］'.len_utf8();
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                } else {
                    cursor += source[cursor..]
                        .chars()
                        .next()
                        .expect("cursor is in bounds")
                        .len_utf8();
                }
            }
            if depth == 0 {
                output.push('※');
                offset = cursor;
                continue;
            }
        }
        let character = source[offset..]
            .chars()
            .next()
            .expect("offset is in bounds");
        output.push(character);
        offset += character.len_utf8();
    }
    output
}

pub(crate) fn inventory_document_observed(
    work_id: &str,
    text: &str,
    patterns: &[SourceInventoryPattern],
    mut observe: impl FnMut(&SourceMarker<'_>, Option<&SourceMarkerRegion>, &[String]),
) -> SourceInventorySummary {
    let compiled_patterns = compile_patterns(patterns);
    let markers = source_markers(text);
    let (body, _) = sanitized_body_range(text);
    let mut summary = SourceInventorySummary {
        work_id: work_id.to_owned(),
        ..SourceInventorySummary::default()
    };

    for (index, marker) in markers.iter().enumerate() {
        summary.markers_total += 1;
        if marker.kind == SourceMarkerKind::SegmentBoundaryTerminalProvenance {
            summary.source_region_events.terminal_provenance_occurrences += 1;
        }
        // Source packaging examples remain occurrences but do not measure body syntax.
        let region = if marker.span.end <= body.start {
            summary.source_region_events.front_matter_occurrences += 1;
            Some(SourceMarkerRegion::FrontMatter)
        } else if marker.raw == "［＃本文終わり］"
            && text[..marker.span.start]
                .rsplit('\n')
                .next()
                .unwrap_or_default()
                .trim()
                .is_empty()
            && text[marker.span.end..]
                .split('\n')
                .next()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            summary.source_region_events.body_end_boundary_occurrences += 1;
            Some(SourceMarkerRegion::BodyEndBoundary)
        } else if marker.span.start >= body.end {
            summary.source_region_events.back_matter_occurrences += 1;
            Some(SourceMarkerRegion::BackMatter)
        } else {
            None
        };
        if let Some(region) = region {
            observe(marker, Some(&region), &[]);
            summary
                .classified_region_markers
                .push(ClassifiedRegionMarker {
                    region,
                    occurrence: SourceMarkerOccurrence {
                        work_id: work_id.to_owned(),
                        indexed_path: None,
                        line: marker.span.line,
                        kind: format!("{:?}", marker.kind),
                        raw: marker.raw.to_owned(),
                        body: marker.body.to_owned(),
                        byte_start: Some(marker.span.start),
                    },
                });
            continue;
        }
        let mut matched = matching_rows(marker.raw, &compiled_patterns);
        if let Some(next_marker) = markers.get(index + 1)
            && marker.span.end == next_marker.span.start
        {
            append_composite_matching_rows(
                &mut matched,
                &text[marker.span.start..next_marker.span.end],
                marker.raw,
                next_marker.raw,
                &compiled_patterns,
            );
        }
        if matched.is_empty() {
            match marker.kind {
                SourceMarkerKind::RubyExplicit | SourceMarkerKind::RubyImplicit => {
                    add_default_match(&mut matched, patterns, "ruby.basic");
                }
                SourceMarkerKind::GaijiFullwidth | SourceMarkerKind::GaijiAscii => {
                    add_default_match(&mut matched, patterns, "gaiji.marker");
                }
                SourceMarkerKind::EditorialNoteBottomTextCorrection => {
                    add_default_match(&mut matched, patterns, "annotation.chuuki");
                }
                SourceMarkerKind::AccentNotation => {
                    add_default_match(&mut matched, patterns, "accent.diacritic");
                }
                SourceMarkerKind::BracketNote => {
                    add_default_match(&mut matched, patterns, "source.bracket_note");
                }
                _ => {}
            }
        }

        observe(marker, None, &matched);
        if matched.is_empty() {
            summary.unknown_examples.push(SourceMarkerOccurrence {
                work_id: work_id.to_owned(),
                indexed_path: None,
                line: marker.span.line,
                kind: format!("{:?}", marker.kind),
                raw: marker.raw.to_owned(),
                body: marker.body.to_owned(),
                byte_start: None,
            });
        } else {
            for row_id in matched {
                summary.row_counts.entry(row_id).or_default().occurrences += 1;
            }
        }
    }

    summary
}

pub(crate) struct CompiledSourceInventoryPattern {
    row_id: String,
    source_patterns: Vec<Regex>,
}

pub(crate) fn compile_patterns(
    patterns: &[SourceInventoryPattern],
) -> Vec<CompiledSourceInventoryPattern> {
    patterns
        .iter()
        // This row documents a research bucket; using it as a classifier would
        // exclude otherwise-unreviewed Aozora commands from the source-authority
        // gate.
        .filter(|pattern| pattern.row_id != "editor_note.unmapped")
        .map(|pattern| CompiledSourceInventoryPattern {
            row_id: pattern.row_id.clone(),
            source_patterns: pattern
                .source_patterns
                .iter()
                .map(|source_pattern| {
                    Regex::new(source_pattern).unwrap_or_else(|err| {
                        panic!("invalid source inventory regex {source_pattern:?}: {err}")
                    })
                })
                .collect(),
        })
        .collect()
}

type SemanticPartitionExtractor = fn(
    &str,
    &str,
    &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)>;

const SEMANTIC_PARTITION_EXTRACTORS: &[(&str, SemanticPartitionExtractor)] = &[
    ("emphasis.basic", emphasis_semantic_partitions),
    ("reference.frontref", frontref_semantic_partitions),
    ("decoration.boten", boten_semantic_partitions),
    ("decoration.bousen", bousen_semantic_partitions),
    ("decoration.font_size", font_size_semantic_partitions),
    ("annotation.chuuki", annotation_semantic_partitions),
    ("layout.tcy", tcy_semantic_partitions),
    ("layout.rotation", rotation_semantic_partitions),
    ("indentation.basic", indentation_semantic_partitions),
    ("indentation.chitsuki", chitsuki_semantic_partitions),
    (
        "layout.cross_line_head_alignment",
        cross_line_head_alignment_semantic_partitions,
    ),
    (
        "indentation.jisage_oneline",
        jisage_oneline_semantic_partitions,
    ),
    ("indentation.jisage_block", jisage_block_semantic_partitions),
    ("heading.basic", heading_semantic_partitions),
    ("ruby.basic", registered_ruby_semantic_partitions),
    ("gaiji.marker", registered_gaiji_semantic_partitions),
    (
        "gaiji_ruby.inline_base",
        registered_gaiji_ruby_semantic_partitions,
    ),
    ("accent.diacritic", registered_accent_semantic_partitions),
    ("iteration.kunoji", registered_iteration_semantic_partitions),
    (
        "ruby.placement_directional",
        directional_ruby_semantic_partitions,
    ),
    ("ruby.double", double_ruby_semantic_partitions),
    ("kunten.kaeriten", kaeriten_semantic_partitions),
    ("break.page_line", break_page_line_semantic_partitions),
];

pub(crate) fn semantic_partition_rows() -> impl Iterator<Item = &'static str> {
    SEMANTIC_PARTITION_EXTRACTORS
        .iter()
        .map(|(row_id, _)| *row_id)
}

pub(crate) fn semantic_partition_occurrences(
    row_id: &str,
    source: &str,
    work_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
) -> Option<Vec<(usize, String, SourceMarkerOccurrence)>> {
    SEMANTIC_PARTITION_EXTRACTORS
        .iter()
        .find(|(registered_row, _)| *registered_row == row_id)
        .map(|(_, extract)| extract(source, work_id, patterns))
}

fn frontref_semantic_partitions(
    source: &str,
    work_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    targeted_style_semantic_partitions(source, work_id, patterns, "reference.frontref")
}

fn boten_semantic_partitions(
    source: &str,
    work_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    targeted_style_semantic_partitions(source, work_id, patterns, "decoration.boten")
}

fn bousen_semantic_partitions(
    source: &str,
    work_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    let mut partitions =
        targeted_style_semantic_partitions(source, work_id, patterns, "decoration.bousen");
    partitions.retain(|(_, partition, _)| !partition.starts_with("selector:preceding-text;"));
    let (body, _) = sanitized_body_range(source);
    let markers = source_markers(source)
        .into_iter()
        .flat_map(|marker| {
            matching_variant_occurrences(&marker, work_id, "decoration.bousen", patterns, 0, 0)
        })
        .filter(|marker| body.start <= marker.start && marker.start < body.end);
    for marker in markers {
        for fact in bousen_facts(&marker.occurrence.body) {
            partitions.push((marker.start, fact, marker.occurrence.clone()));
        }
    }
    partitions
}

fn annotation_semantic_partitions(
    source: &str,
    work_id: &str,
    patterns: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    let (body, _) = sanitized_body_range(source);
    let markers: Vec<_> = source_markers(source)
        .into_iter()
        .flat_map(|marker| {
            matching_variant_occurrences(&marker, work_id, "annotation.chuuki", patterns, 0, 0)
        })
        .filter(|marker| body.start <= marker.start && marker.start < body.end)
        .collect();
    let mut facts: Vec<Vec<String>> = markers
        .iter()
        .map(|marker| annotation_facts(&marker.occurrence.body))
        .collect();
    let mut open = BTreeMap::<&str, Vec<usize>>::new();
    for (index, marker) in markers.iter().enumerate() {
        let text = marker.occurrence.body.as_str();
        if !text.contains("注記付き") {
            continue;
        }
        let side = if text.contains("左に") {
            "left"
        } else {
            "default"
        };
        if text.contains("終わり") {
            if let Some(open_index) = open.get_mut(side).and_then(Vec::pop) {
                facts[open_index].push("boundary:matched".to_owned());
                facts[index].push("boundary:matched".to_owned());
            } else {
                facts[index].push("boundary:orphan-close".to_owned());
            }
        } else {
            open.entry(side).or_default().push(index);
        }
    }
    for indices in open.into_values() {
        for index in indices {
            facts[index].push("boundary:orphan-open".to_owned());
        }
    }
    markers
        .into_iter()
        .zip(facts)
        .flat_map(|(marker, facts)| {
            facts
                .into_iter()
                .map(move |fact| (marker.start, fact, marker.occurrence.clone()))
        })
        .collect()
}

fn annotation_facts(body: &str) -> Vec<String> {
    let operator = annotation_unquoted_text(body);
    let paired = operator.contains("注記付き");
    let form = if paired && operator.contains("終わり") {
        "paired-close"
    } else if paired {
        "paired-open"
    } else if operator.starts_with("入力者注") || operator.starts_with("入力者註") {
        "inputter-note"
    } else if operator.starts_with("現代語訳") {
        "translation-note"
    } else if operator == "未完" {
        "incompleteness-note"
    } else if operator.contains("の注記")
        || operator.contains("と注記")
        || operator.contains("の注釈")
        || operator.contains("の著者註")
    {
        "attached-note"
    } else if operator.starts_with("ルビ")
        || operator.starts_with("るび")
        || operator.contains("ルビの")
    {
        "ruby-apparatus"
    } else if [
        "底本",
        "初出",
        "原文",
        "定本",
        "全集",
        "文庫版",
        "筑摩版",
        "桃源社版",
        "創元社版",
        "岩波文庫",
    ]
    .iter()
    .any(|term| operator.contains(term))
    {
        "source-apparatus"
    } else if operator.contains("ママ") || operator.contains("原文まま") {
        "sic-note"
    } else if ["誤植", "誤記", "誤字", "誤り", "訂正", "修正"]
        .iter()
        .any(|term| operator.contains(term))
    {
        "correction-note"
    } else if ["欠落", "欠如", "脱落", "脱字", "抜け", "省略"]
        .iter()
        .any(|term| operator.contains(term))
    {
        "omission-note"
    } else {
        "editorial-statement"
    };
    let subject = if operator.starts_with("ルビ")
        || operator.starts_with("るび")
        || operator.contains("ルビの")
    {
        "ruby"
    } else if body.starts_with('「') || body.starts_with('《') || body.starts_with('【') {
        "quoted-text"
    } else if [
        "字下げ",
        "字あき",
        "字アキ",
        "改行",
        "改ページ",
        "段組み",
        "表組み",
        "横書き",
        "天付き",
        "行頭",
        "この行",
        "空白",
        "アキ",
    ]
    .iter()
    .any(|term| operator.contains(term))
    {
        "layout"
    } else if operator.contains("底本") || operator.contains("本文") || operator.contains("作品")
    {
        "source-document"
    } else {
        "implicit-context"
    };
    let scope = if paired {
        "paired-range"
    } else if body.starts_with('「') || body.starts_with('《') || body.starts_with('【') {
        "explicit-target"
    } else {
        "statement"
    };
    let content = if body.contains('「') || body.contains('《') || body.contains('【') {
        "quoted"
    } else if body.contains('（') || body.contains('(') || body.contains('〔') {
        "parenthesized"
    } else {
        "bare"
    };
    let position = if operator.contains("左に")
        || operator.contains("左側")
        || operator.contains("左横")
    {
        Some("left")
    } else if operator.contains("右に") || operator.contains("右側") || operator.contains("右横")
    {
        Some("right")
    } else if operator.contains("下に") {
        Some("below")
    } else if operator.contains("上に") {
        Some("above")
    } else {
        None
    };
    let mut facts = vec![
        format!("form:{form}"),
        format!("subject:{subject}"),
        format!("scope:{scope}"),
        format!("content:{content}"),
    ];
    if let Some(position) = position {
        facts.push(format!("position:{position}"));
    }
    for (relation, terms) in [
        ("annotation", &["注記", "注釈", "著者註", "校注"][..]),
        ("sic", &["ママ", "原文まま", "底本のまま"][..]),
        (
            "correction",
            &["誤植", "誤記", "誤字", "誤り", "訂正", "修正"][..],
        ),
        (
            "omission",
            &[
                "欠落", "欠如", "脱落", "脱字", "抜け", "なし", "無し", "省略",
            ][..],
        ),
        ("duplication", &["重複"][..]),
        ("possible-excess", &["余分"][..]),
        ("addition", &["挿入", "補った"][..]),
        ("supplied-translation", &["現代語訳"][..]),
        ("translation-error", &["誤訳"][..]),
        ("facsimile", &["図", "写真", "地図", "画像"][..]),
        (
            "formatting",
            &[
                "字下げ",
                "字あき",
                "改行",
                "横書き",
                "縦中横",
                "傍点",
                "傍線",
                "太字",
                "見出し",
            ][..],
        ),
    ] {
        if terms.iter().any(|term| operator.contains(term)) {
            facts.push(format!("relation:{relation}"));
        }
    }
    if operator.contains('?')
        || operator.contains('？')
        || operator.contains("と思われ")
        || operator.contains("と考えられ")
        || operator.ends_with('か')
    {
        facts.push("certainty:uncertain".to_owned());
    } else {
        facts.push("certainty:asserted".to_owned());
    }
    facts
}

fn annotation_unquoted_text(body: &str) -> String {
    let mut depth = 0_u32;
    body.chars()
        .filter(|character| match character {
            '「' | '《' | '【' => {
                depth += 1;
                false
            }
            '」' | '》' | '】' => {
                depth = depth.saturating_sub(1);
                false
            }
            _ => depth == 0,
        })
        .collect()
}

fn bousen_facts(body: &str) -> Vec<String> {
    let boundary = emphasis_boundary(body)
        .filter(|(family, _)| family == "bousen")
        .map(|(_, closing)| closing);
    let following_lines = body
        .strip_prefix("次")
        .and_then(|rest| rest.split_once("行"))
        .and_then(|(count, _)| decimal_value(count));
    let form = if following_lines.is_some() {
        "following-lines"
    } else if body.contains('「') {
        "retrospective"
    } else if boundary == Some(false) {
        "range-open"
    } else if boundary == Some(true) {
        "range-close"
    } else {
        "instruction"
    };
    let line_kind = if body.contains("二重傍線") {
        "double"
    } else if body.contains("鎖線") || body.contains("鎖傍線") {
        "chain"
    } else if body.contains("破線") {
        "dashed"
    } else if body.contains("波線") {
        "wavy"
    } else if body.contains("上線") || body.contains("下線") || body.contains("傍線") {
        "single"
    } else {
        "unresolved"
    };
    let position = if body.contains("両側") {
        "both"
    } else if body.contains("上線") {
        "above"
    } else if body.contains("下線") {
        "below"
    } else if body.contains("の左") || body.contains("左に") {
        "left"
    } else {
        "default"
    };
    let scope = if following_lines.is_some() {
        "line-count"
    } else if form.starts_with("range-") {
        "paired-range"
    } else if form == "retrospective" {
        "single-target"
    } else {
        "unresolved"
    };
    let mut facts = vec![
        "effect:bousen".to_owned(),
        format!("form:{form}"),
        format!("line-kind:{line_kind}"),
        format!("position:{position}"),
        format!("scope:{scope}"),
    ];
    if let Some(count) = following_lines {
        facts.push(format!("line-count:{count}"));
        facts.push("target:following-lines".to_owned());
        if body.contains("ゴシック体") {
            facts.push("compound:typeface-gothic".to_owned());
        }
    } else if form == "range-open" {
        facts.push("target:following-source-range".to_owned());
    } else if form == "range-close" {
        facts.push("target:preceding-source-range".to_owned());
    }
    facts
}

fn registered_ruby_semantic_partitions(
    source: &str,
    work_id: &str,
    _: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    ruby_semantic_partitions(source, work_id)
}

fn registered_gaiji_semantic_partitions(
    source: &str,
    work_id: &str,
    _: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    gaiji_semantic_partitions(source, work_id)
}

fn registered_gaiji_ruby_semantic_partitions(
    source: &str,
    work_id: &str,
    _: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    gaiji_ruby_semantic_partitions(source, work_id)
}

fn registered_accent_semantic_partitions(
    source: &str,
    work_id: &str,
    _: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    accent_semantic_partitions(source, work_id)
}

fn registered_iteration_semantic_partitions(
    source: &str,
    work_id: &str,
    _: &[CompiledSourceInventoryPattern],
) -> Vec<(usize, String, SourceMarkerOccurrence)> {
    iteration_semantic_partitions(source, work_id)
}

pub(crate) fn matching_rows(raw: &str, patterns: &[CompiledSourceInventoryPattern]) -> Vec<String> {
    let mut rows = Vec::new();
    append_matching_rows(&mut rows, raw, patterns);
    rows
}

fn append_matching_rows(
    rows: &mut Vec<String>,
    raw: &str,
    patterns: &[CompiledSourceInventoryPattern],
) {
    for pattern in patterns {
        if pattern
            .source_patterns
            .iter()
            .any(|source_pattern| source_pattern.is_match(raw))
            && !rows.contains(&pattern.row_id)
        {
            rows.push(pattern.row_id.clone());
        }
    }
}

pub(crate) fn append_composite_matching_rows(
    rows: &mut Vec<String>,
    raw: &str,
    left_raw: &str,
    right_raw: &str,
    patterns: &[CompiledSourceInventoryPattern],
) {
    for pattern in patterns {
        let matches_composite = pattern
            .source_patterns
            .iter()
            .any(|source_pattern| source_pattern.is_match(raw));
        let matches_part = pattern.source_patterns.iter().any(|source_pattern| {
            source_pattern.is_match(left_raw) || source_pattern.is_match(right_raw)
        });
        if matches_composite && !matches_part && !rows.contains(&pattern.row_id) {
            rows.push(pattern.row_id.clone());
        }
    }
}

fn add_default_match(rows: &mut Vec<String>, patterns: &[SourceInventoryPattern], row_id: &str) {
    if patterns.iter().any(|pattern| pattern.row_id == row_id)
        && !rows.iter().any(|row| row == row_id)
    {
        rows.push(row_id.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_rows_from_source_patterns() {
        let patterns = vec![
            pattern("layout.yokogumi", vec![r"［＃ここから横組み］"]),
            pattern("gaiji.marker", vec![r"※［＃[^］]+］"]),
        ];
        let summary = inventory_document(
            "w1",
            "※［＃「口＋世」、U+546D］\n［＃ここから横組み］",
            &patterns,
        );

        assert_eq!(summary.markers_total, 2);
        assert_eq!(summary.row_counts["gaiji.marker"].occurrences, 1);
        assert_eq!(summary.row_counts["layout.yokogumi"].occurrences, 1);
        assert!(summary.unknown_examples.is_empty());
    }

    #[test]
    fn selected_rows_retain_bounded_normalized_variant_samples() {
        let patterns = vec![pattern("emphasis.basic", vec![r"傍点］"])];
        let source = "語［＃「語」に傍点］、字［＃「字」に傍点］";
        let selected = vec!["emphasis.basic".to_owned()];

        let summary = inventory_document_with_variants("w1", source, &patterns, &selected);
        let variants = &summary.row_variants["emphasis.basic"];
        let variant = &variants["［＃「○○」に傍点］"];

        assert_eq!(variant.occurrences, 2);
        assert_eq!(variant.samples.len(), 2);
        assert_eq!(variant.samples[0].raw, "［＃「語」に傍点］");
        assert_eq!(normalize_marker_form("［＃１０字下げ］"), "［＃N字下げ］");
        assert_eq!(
            normalize_marker_form("［＃「胡※［＃「示＋夭」、第3水準1-89-21］祠」に白丸傍点］"),
            "［＃「○○」に白丸傍点］"
        );
    }

    #[test]
    fn selected_emphasis_rows_partition_source_target_relations() {
        let patterns = vec![pattern("emphasis.basic", vec![r"傍点］"])];
        let selected = vec!["emphasis.basic".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            concat!(
                "語［＃「語」に傍点］、",
                "語、語［＃「語」に丸傍点］、",
                "本文［＃「欠落」に白丸傍点］、",
                "前文［＃特大文字、二重丸傍点］",
            ),
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["emphasis.basic"];

        assert_eq!(
            partitions["selector:quoted;referent:adjacent-unique"].occurrences,
            1
        );
        assert_eq!(
            partitions["selector:quoted;referent:adjacent-repeated"].occurrences,
            1
        );
        assert_eq!(
            partitions["selector:quoted;referent:missing"].occurrences,
            1
        );
        assert_eq!(
            partitions["selector:preceding-text;referent:adjacent"].occurrences,
            1
        );
    }

    #[test]
    fn selected_tcy_row_partitions_targets_effects_composition_and_ranges() {
        let patterns = vec![pattern(
            "layout.tcy",
            vec![r"［＃[^］]*(?:縦中横|横一列)[^］]*］"],
        )];
        let selected = vec!["layout.tcy".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            concat!(
                "12［＃「12」は縦中横］、",
                "（一）［＃（一）は縦中横］、",
                "!?［＃「!?」は横一列］、",
                "B1［＃「B1」は縦中横、「1」は下付き小文字］、",
                "Ａ１、Ａ２［＃「Ａｎ」はそれぞれ縦中横、数字は上付き小書き］、",
                "（十一）［＃（十一）は縦中横、「十一」は縦組み］、",
                "［＃縦中横］10［＃縦中横終わり］",
            ),
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["layout.tcy"];

        assert_eq!(summary.row_counts["layout.tcy"].occurrences, 8);
        assert_eq!(partitions["effect:tcy"].occurrences, 8);
        assert!(!partitions.contains_key("effect:horizontal-line"));
        assert_eq!(partitions["scope:paired-range"].occurrences, 2);
        assert_eq!(partitions["boundary:matched-open"].occurrences, 1);
        assert_eq!(partitions["boundary:matched-close"].occurrences, 1);
        assert_eq!(
            partitions["selector:parenthesized;referent:adjacent-unique"].occurrences,
            2
        );
        assert_eq!(partitions["composition:subscript"].occurrences, 1);
        assert_eq!(
            partitions["target:quoted-pattern-previous-occurrences"].occurrences,
            1
        );
        assert_eq!(partitions["composition:vertical-layout"].occurrences, 1);
        assert!(
            summary.row_variants["layout.tcy"]
                .values()
                .all(|variant| !variant.semantic_partitions.is_empty())
        );
    }

    #[test]
    fn selected_tcy_row_exposes_orphan_ranges_and_attested_spellings() {
        let patterns = vec![pattern("layout.tcy", vec![r"［＃[^］]*縦中横[^］]*］"])];
        let selected = vec!["layout.tcy".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            "［＃縦中横終わり］語［＃縦中横］12［＃「12」は縦中横組み］",
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["layout.tcy"];

        assert_eq!(partitions["boundary:orphan-close"].occurrences, 1);
        assert_eq!(partitions["boundary:orphan-open"].occurrences, 1);
        assert_eq!(partitions["form:retrospective-quoted"].occurrences, 1);
        assert!(!partitions.contains_key("form:malformed"));
    }

    #[test]
    fn selected_rotation_row_preserves_angle_target_and_unspecified_direction() {
        let patterns = vec![pattern(
            "layout.rotation",
            vec![r"［＃「[^」]+」は[０-９0-9]+度回転］"],
        )];
        let selected = vec!["layout.rotation".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            "月二［＃「月二」は１８０度回転］",
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["layout.rotation"];

        assert_eq!(summary.row_counts["layout.rotation"].occurrences, 1);
        for expected in [
            "form:retrospective-quoted",
            "effect:rotation",
            "angle-degrees:180",
            "direction:unspecified",
            "scope:single-target",
            "target:quoted-previous-occurrence",
            "selector:quoted;referent:adjacent-unique",
        ] {
            assert_eq!(partitions[expected].occurrences, 1, "missing {expected}");
        }
        assert_eq!(
            summary.row_variants["layout.rotation"]["［＃「○○」はN度回転］"]
                .semantic_partitions
                .len(),
            7
        );
    }

    #[test]
    fn selected_frontref_row_has_its_own_target_relation_queue() {
        let patterns = vec![
            pattern("emphasis.basic", vec![r"(?:傍点|傍線)］"]),
            pattern("reference.frontref", vec![r"「[^」]+」に(?:傍点|傍線)］"]),
        ];
        let selected = vec!["reference.frontref".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            concat!(
                "語［＃「語」に傍点］、",
                "対象間隔［＃「対象」に傍線］、",
                "語、語［＃「語」に傍点］、",
                "本文［＃「欠落」に傍線］、",
                "前文［＃特大文字、二重丸傍点］",
            ),
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["reference.frontref"];

        assert_eq!(
            partitions["selector:quoted;referent:adjacent-unique"].occurrences,
            1
        );
        assert_eq!(
            partitions["selector:quoted;referent:detached-unique"].occurrences,
            1
        );
        assert_eq!(
            partitions["selector:quoted;referent:adjacent-repeated"].occurrences,
            1
        );
        assert_eq!(
            partitions["selector:quoted;referent:missing"].occurrences,
            1
        );
        assert_eq!(
            partitions
                .values()
                .map(|partition| partition.occurrences)
                .sum::<u64>(),
            4
        );
    }

    #[test]
    fn selected_boten_row_has_its_own_target_relation_queue() {
        let patterns = vec![
            pattern("emphasis.basic", vec![r"(?:傍点|傍線)］"]),
            pattern("decoration.boten", vec![r"傍点］"]),
        ];
        let selected = vec!["decoration.boten".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            "語［＃「語」に傍点］線［＃「線」に傍線］",
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["decoration.boten"];

        assert_eq!(
            partitions["selector:quoted;referent:adjacent-unique"].occurrences,
            1
        );
        assert_eq!(
            partitions
                .values()
                .map(|partition| partition.occurrences)
                .sum::<u64>(),
            1
        );
    }

    #[test]
    fn selected_bousen_row_partitions_forms_kinds_positions_and_boundaries() {
        let patterns = vec![pattern(
            "decoration.bousen",
            vec![r"［＃[^］]*(?:傍線|鎖線|破線|波線|上線|下線)[^］]*］"],
        )];
        let selected = vec!["decoration.bousen".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            concat!(
                "線［＃「線」に二重傍線］［＃左に波線］波［＃左に波線終わり］",
                "［＃次３行は、文字はゴシック体、罫線は全て波線］",
            ),
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["decoration.bousen"];

        assert_eq!(summary.row_counts["decoration.bousen"].occurrences, 4);
        assert_eq!(partitions["effect:bousen"].occurrences, 4);
        assert_eq!(partitions["form:retrospective"].occurrences, 1);
        assert_eq!(partitions["form:range-open"].occurrences, 1);
        assert_eq!(partitions["form:range-close"].occurrences, 1);
        assert_eq!(partitions["line-kind:double"].occurrences, 1);
        assert_eq!(partitions["line-kind:wavy"].occurrences, 3);
        assert_eq!(partitions["position:default"].occurrences, 2);
        assert_eq!(partitions["position:left"].occurrences, 2);
        assert_eq!(partitions["boundary:matched;family:bousen"].occurrences, 1);
        assert_eq!(partitions["form:following-lines"].occurrences, 1);
        assert_eq!(partitions["line-count:3"].occurrences, 1);
        assert_eq!(partitions["target:following-lines"].occurrences, 1);
        assert_eq!(partitions["compound:typeface-gothic"].occurrences, 1);
        assert_eq!(
            partitions["selector:quoted;referent:adjacent-unique"].occurrences,
            1
        );
        assert!(
            summary.row_variants["decoration.bousen"]
                .values()
                .all(|variant| !variant.semantic_partitions.is_empty())
        );
    }

    #[test]
    fn selected_indentation_row_partitions_layout_scope_geometry_and_boundaries() {
        let patterns = vec![pattern(
            "indentation.basic",
            vec![
                r"［＃[０-９0-9]+字下げ］",
                r"［＃ここから[０-９0-9]+字下げ］",
                r"［＃地付き］",
            ],
        )];
        let selected = vec!["indentation.basic".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            concat!(
                "［＃２字下げ］行\n",
                "［＃ここから３字下げ］\n甲\n",
                "［＃ここから４字下げ］\n乙\n［＃ここで字下げ終わり］\n",
                "前［＃地付き］署名",
            ),
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["indentation.basic"];

        for partition in [
            "form:line-indent",
            "form:block-indent",
            "form:line-align-end",
            "scope:line",
            "scope:block",
            "indent:2",
            "indent:3",
            "indent:4",
            "alignment:end",
            "placement:line-prefix",
            "placement:standalone-line",
            "placement:inline",
            "boundary:next-block-open",
            "boundary:matched-close",
        ] {
            assert!(
                partitions.contains_key(partition),
                "{partition}: {partitions:?}"
            );
        }
    }

    #[test]
    fn selected_chitsuki_row_partitions_targets_offsets_compounds_and_boundaries() {
        let patterns = vec![pattern(
            "indentation.chitsuki",
            vec![r"［＃[^］]*(?:地付き|右寄せ|字上げ|字上がり|下揃え|頭揃え|揃っている)[^］]*］"],
        )];
        let selected = vec!["indentation.chitsuki".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            concat!(
                "［＃地から２字上げ］署名\n",
                "［＃ここから地付き］\n甲\n［＃ここで地付き終わり］\n",
                "幕。［＃「幕。」は地付き］\n",
                "［＃この行はポイントを下げて、地より３字上げ］注\n",
                "［＃「地付き］\n",
                "［＃こ地付き］日付\n",
                "歌［＃この歌、二行前の歌に頭揃え。］\n",
                "［＃（Ｆ・Ｏ）は下揃え］\nＦ・Ｏ\n",
                "［＃ここから３字下げ、地より１字上げ］\n本文\n",
            ),
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["indentation.chitsuki"];

        assert_eq!(summary.row_counts["indentation.chitsuki"].occurrences, 10);
        for expected in [
            "form:line-offset",
            "form:block-open-align-end",
            "form:block-close",
            "form:retrospective-align-end",
            "form:compound-line-geometry",
            "form:following-pattern-align-end",
            "form:compound-block-open",
            "effect:end-alignment",
            "scope:line",
            "scope:paired-block",
            "scope:single-target",
            "scope:following-text",
            "scope:following-block",
            "target:same-line-remainder",
            "target:following-source-range",
            "target:preceding-source-range",
            "target:quoted-previous-occurrence",
            "target:named-token-in-following-text",
            "offset-from-end:0",
            "offset-from-end:2",
            "offset-from-end:3",
            "composition:font-size",
            "boundary:matched-open",
            "boundary:matched-close",
            "selector:quoted;referent:adjacent-unique",
        ] {
            assert!(
                partitions.contains_key(expected),
                "{expected}: {partitions:?}"
            );
        }
        for excluded in [
            "form:cross-line-relative",
            "effect:cross-line-start-alignment",
            "scope:line-relation",
            "target:two-lines-prior",
            "composition:cross-line-alignment",
        ] {
            assert!(
                !partitions.contains_key(excluded),
                "{excluded}: {partitions:?}"
            );
        }
    }

    #[test]
    fn selected_oneline_jisage_row_partitions_spellings_values_and_empty_content() {
        let patterns = vec![pattern(
            "indentation.jisage_oneline",
            vec![
                r"［＃この行[０-９0-9]+字下げ］",
                r"［＃天から[０-９0-9]+字下げ］",
                r"［＃[０-９0-9]+字下がる］",
                r"［＃[０-９0-9]+字下げ］",
            ],
        )];
        let selected = vec!["indentation.jisage_oneline".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            concat!(
                "［＃２字下げ］甲\n",
                "［＃この行３字下げ］乙\n",
                "［＃天から4字下げ］丙\n",
                "［＃５字下がる］丁\n",
                "［＃６字下げ］\n",
                "［＃０字下げ］無効\n",
                "前［＃７字下げ］後\n",
            ),
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["indentation.jisage_oneline"];

        for partition in [
            "form:bare",
            "form:explicit-this-line",
            "form:top-margin",
            "form:legacy-sagaru",
            "scope:line",
            "target:same-line-remainder",
            "indent:2",
            "indent:3",
            "indent:4",
            "indent:5",
            "indent:6",
            "digits:ascii",
            "digits:fullwidth",
            "placement:line-prefix",
            "placement:standalone-line",
            "content:present",
            "content:empty",
            "form:malformed",
            "scope:unresolved",
            "target:unresolved",
            "interpretation:unresolved",
            "preservation:raw",
            "placement:inline",
            "target:whole-line",
        ] {
            assert!(
                partitions.contains_key(partition),
                "{partition}: {partitions:?}"
            );
        }
    }

    #[test]
    fn selected_block_jisage_row_partitions_forms_geometry_and_uncertainty() {
        let patterns = vec![pattern(
            "indentation.jisage_block",
            vec![r"［＃[^］]*字下げ[^］]*］"],
        )];
        let selected = vec!["indentation.jisage_block".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            concat!(
                "［＃ここから一字下げ、折り返して二字下げ、２１字詰め］\n甲\n",
                "［＃ここで字下げ終わり］\n",
                "［＃天より３２字下げて地より３字上げで］乙\n",
                "［＃「序」全体、天より２字下げ］\n",
                "［＃改行ごとに二字下げ］\n",
                "［＃「３字下げ」］\n",
                "［＃ここから字下げ］\n",
                "［＃ここから３　字下げ］\n",
                "［＃ここか三字下げ］\n",
            ),
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["indentation.jisage_block"];

        for partition in [
            "form:block-open",
            "form:block-close",
            "form:line-layout",
            "form:targeted-layout",
            "form:document-instruction",
            "form:quoted-example",
            "form:malformed",
            "geometry:hanging-indent",
            "geometry:line-width",
            "geometry:end-offset",
            "indent:1",
            "continuation-indent:2",
            "width:21",
            "end-offset:3",
            "boundary:matched-close",
            "boundary:matched-open",
            "interpretation:quoted-example",
            "interpretation:unresolved",
            "preservation:raw",
        ] {
            assert!(
                partitions.contains_key(partition),
                "{partition}: {partitions:?}"
            );
        }
        assert_eq!(partitions["form:block-open"].occurrences, 3);
        assert_eq!(partitions["form:malformed"].occurrences, 1);
    }

    #[test]
    fn selected_heading_row_partitions_forms_levels_targets_and_boundaries() {
        let patterns = vec![pattern(
            "heading.basic",
            vec![
                r#"［＃「[^\n]*」は[大中小]見出し］"#,
                r#"［＃「[^\n]*」に[大中小]見出し］"#,
                r"［＃(ここから|ここで)?[大中小]見出し?(終わり)?］",
                r"［＃ここで見出し終わり］",
                r"［＃[大中小]?見出し文字］",
            ],
        )];
        let selected = vec!["heading.basic".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            concat!(
                "大章［＃「大章」は大見出し］\n",
                "章《しょう》［＃「章」は中見出し］\n",
                "小章［＃「小章」は小見出し］\n",
                "旧章［＃「旧章」に大見出し］\n",
                "［＃中見出し］節《せつ》［＃中見出終わり］\n",
                "［＃ここから小見出し］\n項目\n［＃ここで見出し終わり］\n",
                "新聞見出し［＃見出し文字］\n",
                "小見出し［＃小見出し文字］\n",
                "［＃大見出し終わり］\n",
                "［＃大見出し］未完\n",
                "［＃中見出し終わり］",
            ),
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["heading.basic"];

        for partition in [
            "form:forward",
            "form:paired-open",
            "form:paired-close",
            "form:block-open",
            "form:block-close",
            "form:legacy-typography",
            "level:1",
            "level:2",
            "level:3",
            "level:inherited",
            "relation:wa",
            "relation:ni",
            "referent:adjacent-plain",
            "referent:adjacent-rich",
            "target-structure:plain",
            "target-structure:ruby",
            "boundary:matched-close",
            "boundary:matched-open",
            "boundary:orphan-close",
            "boundary:mismatched-close",
            "boundary:missing-close",
            "spelling:abbreviated-mi-dashi",
            "typography:heading",
            "typography:small-heading",
        ] {
            assert!(
                partitions.contains_key(partition),
                "{partition}: {partitions:?}"
            );
        }
    }

    #[test]
    fn selected_ruby_rows_partition_base_selection_and_structured_content() {
        let patterns = vec![pattern("ruby.basic", vec![])];
        let selected = vec!["ruby.basic".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            concat!(
                "火照《ほて》って、稍々《やや》、Fanatiker《ファナチイケル》、",
                "｜お伽話《フェヤリー・ストーリース》、",
                "｜前※［＃字］後《よみ》、｜音《※［＃字］》",
            ),
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["ruby.basic"];

        for partition in [
            "form:implicit",
            "form:explicit",
            "preceding-class:han",
            "preceding-class:special-kanji",
            "preceding-class:latin",
            "base-content:mixed",
            "base-content:gaiji",
            "base-content:han",
            "reading-content:plain",
            "reading-content:gaiji",
        ] {
            assert!(
                partitions.contains_key(partition),
                "{partition}: {:?}",
                partitions.keys().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn selected_directional_ruby_rows_partition_forms_targets_and_readings() {
        let patterns = vec![pattern(
            "ruby.placement_directional",
            vec![
                r"［＃「[^」]+」の左に「[^」]+」のルビ］",
                r"［＃左にルビ付き］",
                r"［＃左に「[^」]+」のルビ付き終わり］",
            ],
        )];
        let selected = vec!["ruby.placement_directional".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            concat!(
                "青空《あおぞら》［＃「青空」の左に「blue」のルビ］、",
                "黄梅［＃「黄梅」に傍点］［＃「黄梅」の左に「おうばい」のルビ］、",
                "［＃左にルビ付き］※［＃字、第3水準1-15-23］《じ》",
                "［＃左に「letter（？）」のルビ付き終わり］",
            ),
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["ruby.placement_directional"];

        for partition in [
            "form:forward",
            "form:paired",
            "boundary:matched",
            "referent:adjacent-rich",
            "target-structure:ruby",
            "target-structure:command",
            "target-structure:mixed",
            "base-content:plain",
            "reading-content:plain",
            "reading-notation:plain",
            "reading-notation:question-mark",
        ] {
            assert!(
                partitions.contains_key(partition),
                "{partition}: {:?}",
                partitions.keys().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn selected_double_ruby_rows_partition_forms_layers_and_targets() {
        let patterns = vec![pattern(
            "ruby.double",
            vec![
                r"｜[^《\n]+《[^》]+》《[^》]+》",
                r"《[^》]+》［＃「[^」]+」の(左|右)に「[^」]+」のルビ］",
            ],
        )];
        let selected = vec!["ruby.double".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            concat!(
                "｜text《upper》《lower》、",
                "青空《あおぞら》［＃「青空」の左に「blue」のルビ］",
            ),
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["ruby.double"];

        for partition in [
            "form:consecutive",
            "form:directional",
            "boundary:matched",
            "referent:explicit",
            "referent:adjacent-plain",
            "placement:right-left",
            "target-structure:plain",
            "base-content:latin",
            "base-content:plain",
            "right-reading-content:plain",
            "left-reading-content:plain",
            "right-reading-notation:plain",
            "left-reading-notation:plain",
        ] {
            assert!(
                partitions.contains_key(partition),
                "{partition}: {:?}",
                partitions.keys().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn selected_nested_ruby_rows_recover_the_complete_outer_source() {
        let patterns = vec![pattern(
            "ruby.nested_forbidden",
            vec![r"｜[^《\n]+《[^》\n]*《[^》\n]+》[^》\n]*》"],
        )];
        let selected = vec!["ruby.nested_forbidden".to_owned()];
        let summary =
            inventory_document_with_variants("w1", "｜漢《か《ん》じ》", &patterns, &selected);

        assert_eq!(summary.row_counts["ruby.nested_forbidden"].occurrences, 1);
        let partitions = &summary.row_semantic_partitions["ruby.nested_forbidden"];
        for partition in [
            "form:nested-reading",
            "boundary:matched",
            "reading-content:ruby-delimiters",
        ] {
            assert!(partitions.contains_key(partition));
        }
        assert_eq!(
            partitions["form:nested-reading"].samples[0].raw,
            "｜漢《か《ん》じ》"
        );
    }

    #[test]
    fn selected_gaiji_ruby_rows_partition_identity_and_base_shape() {
        let patterns = vec![pattern(
            "gaiji_ruby.inline_base",
            vec![r"※［＃[^］]+］《[^》]+》"],
        )];
        let selected = vec!["gaiji_ruby.inline_base".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            concat!(
                "※［＃字、第3水準1-15-23］《じ》、",
                "※［＃字、U+5B57、12-3］《じ》、",
                "｜前※［＃字］後《よみ》、",
                "｜※［＃字］※[#字]《よみ》",
            ),
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["gaiji_ruby.inline_base"];

        let implicit = partitions["form:implicit"]
            .samples
            .iter()
            .map(|sample| (sample.byte_start, sample.raw.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            implicit,
            [
                (Some(0), "※［＃字、第3水準1-15-23］《じ》"),
                (
                    Some("※［＃字、第3水準1-15-23］《じ》、".len()),
                    "※［＃字、U+5B57、12-3］《じ》"
                ),
            ]
        );
        for partition in [
            "form:implicit",
            "form:explicit",
            "gaiji-count:single",
            "gaiji-count:multiple",
            "base-shape:gaiji-only",
            "base-shape:mixed",
            "marker-syntax:fullwidth",
            "marker-syntax:mixed",
            "identifier:jis",
            "identifier:unicode",
            "identifier:source-location",
            "identifier:description-only",
        ] {
            assert!(
                partitions.contains_key(partition),
                "{partition}: {:?}",
                partitions.keys().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn selected_accent_rows_partition_resolution_and_spelling_shape() {
        let patterns = vec![pattern("accent.diacritic", vec![r"〔[A-Za-z][^〕]*〕"])];
        let selected = vec!["accent.diacritic".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            "〔AE& E' e~ L'art〕、〔l'Institut〕",
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["accent.diacritic"];

        for partition in [
            "substitutions:multiple",
            "substitutions:none",
            "resolution:partial",
            "resolution:none",
            "spelling-width:mixed",
            "letter-case:mixed",
            "byte-effect:mixed",
            "marker:acute",
            "marker:ring-or-ligature",
            "marker:tilde",
        ] {
            assert!(
                partitions.contains_key(partition),
                "{partition}: {:?}",
                partitions.keys().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn selected_iteration_rows_partition_spelling_and_source_role() {
        let patterns = vec![pattern("iteration.kunoji", vec![r"／＼", r"／″＼"])];
        let selected = vec!["iteration.kunoji".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            concat!(
                "フゴ／＼。",
                "｜とき／″＼《時々》。",
                "時々《とき／＼》。",
                "語［＃「語／″＼」は底本では「語／＼」］"
            ),
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["iteration.kunoji"];
        let variants = &summary.row_variants["iteration.kunoji"];

        assert_eq!(summary.row_counts["iteration.kunoji"].occurrences, 5);
        assert_eq!(variants["／＼"].occurrences, 3);
        assert_eq!(variants["／″＼"].occurrences, 2);

        for partition in [
            "form:unvoiced",
            "form:voiced",
            "role:body",
            "role:ruby-base",
            "role:ruby-reading",
            "role:directive-reference",
        ] {
            assert!(
                partitions.contains_key(partition),
                "{partition}: {:?}",
                partitions.keys().collect::<Vec<_>>()
            );
        }
        assert!(!partitions.contains_key("boundary:split-by-note-target"));
    }

    #[test]
    fn a_note_target_that_begins_inside_a_kunoji_splits_it() {
        let patterns = vec![pattern("iteration.kunoji", vec![r"／＼", r"／″＼"])];
        let selected = vec!["iteration.kunoji".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            concat!(
                "ごた／＼ゐ［＃「＼ゐ」に「マヽ」の注記］る。",
                "合間／＼［＃「／＼」に「ママ」の注記］に。",
                "おい／＼［＃「おい／＼」に「（追々）」の注記］なり。"
            ),
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["iteration.kunoji"];

        assert_eq!(partitions["boundary:split-by-note-target"].occurrences, 1);
        assert_eq!(
            partitions["boundary:split-by-note-target"].samples[0].raw,
            "／＼"
        );
    }

    #[test]
    fn selected_gaiji_rows_count_tokens_and_partition_source_roles() {
        let patterns = vec![pattern("gaiji.marker", vec![r"※［＃[^］]+］"])];
        let selected = vec!["gaiji.marker".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            concat!(
                "※［＃字、第3水準1-15-23］、",
                "※[#字,U+5B57]、",
                "｜※［＃字］《※[#音]》、",
                "※［＃黙］《もく》、",
                "［＃「※［＃字、12-3］」は底本では「字」］",
            ),
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["gaiji.marker"];
        let variants = &summary.row_variants["gaiji.marker"];

        assert_eq!(summary.row_counts["gaiji.marker"].occurrences, 6);
        assert_eq!(variants["※［＃…］"].occurrences, 4);
        assert_eq!(variants["※[#…]"].occurrences, 2);
        assert_eq!(
            variants
                .values()
                .map(|variant| variant.occurrences)
                .sum::<u64>(),
            6
        );
        assert_eq!(partitions["role:body"].occurrences, 2);
        assert_eq!(partitions["role:ruby-base"].occurrences, 2);
        assert_eq!(partitions["role:ruby-reading"].occurrences, 1);
        assert_eq!(partitions["role:directive-reference"].occurrences, 1);
        assert!(partitions.contains_key("identifier:jis"));
        assert!(partitions.contains_key("identifier:unicode"));
        assert!(partitions.contains_key("identifier:source-location"));
        assert!(partitions.contains_key("identifier:description-only"));
        assert!(
            variants
                .values()
                .flat_map(|variant| &variant.samples)
                .all(|sample| sample.kind.starts_with("Gaiji"))
        );
    }

    #[test]
    fn selected_jis_gaiji_row_counts_gaiji_tokens_instead_of_outer_commands() {
        let patterns = vec![pattern(
            "gaiji.jis_code",
            vec![
                r"第[1-4]水準[12]-[0-9]+-[0-9]+",
                r"[12]-[0-9]{1,2}-[0-9]{1,2}",
            ],
        )];
        let selected = vec!["gaiji.jis_code".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            concat!(
                "※［＃字、第4水準2-13-47］、",
                "※[#音,1-7-84]、",
                "｜※［＃基、第3水準1-15-23］《※［＃読、1-13-22］》、",
                "［＃「※［＃注、第3水準1-2-54］」はママ］、",
                "※［＃位置、12-3］",
            ),
            &patterns,
            &selected,
        );
        let variants = &summary.row_variants["gaiji.jis_code"];
        let partitions = &summary.row_semantic_partitions["gaiji.jis_code"];

        assert_eq!(summary.row_counts["gaiji.jis_code"].occurrences, 5);
        assert_eq!(
            variants
                .values()
                .map(|variant| variant.occurrences)
                .sum::<u64>(),
            5
        );
        assert_eq!(variants["※［＃○○、第N水準N-N-N］"].occurrences, 3);
        assert_eq!(variants["※［＃○○、N-N-N］"].occurrences, 1);
        assert_eq!(variants["※[#○○、N-N-N]"].occurrences, 1);
        assert_eq!(partitions["role:body"].occurrences, 2);
        assert_eq!(partitions["role:ruby-base"].occurrences, 1);
        assert_eq!(partitions["role:ruby-reading"].occurrences, 1);
        assert_eq!(partitions["role:directive-reference"].occurrences, 1);
        assert_eq!(partitions["plane:1"].occurrences, 4);
        assert_eq!(partitions["plane:2"].occurrences, 1);
    }

    #[test]
    fn structured_selectors_use_their_semantic_anchor() {
        let patterns = vec![pattern("emphasis.basic", vec![r"傍点］"])];
        let selected = vec!["emphasis.basic".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            concat!(
                "自然［＃「自」と「然」の間に白三角傍点］、",
                "行・実践［＃「行・実践」の「・」を除く部分に傍点］、",
                "法律［＃「法律」の「法」に傍点］",
            ),
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["emphasis.basic"];

        assert_eq!(
            partitions["selector:between;referent:adjacent-pair"].occurrences,
            1
        );
        assert_eq!(
            partitions["selector:exclusion;referent:adjacent-enclosing"].occurrences,
            1
        );
        assert_eq!(
            partitions["selector:subtarget;referent:adjacent-enclosing"].occurrences,
            1
        );
    }

    #[test]
    fn semantic_partitions_project_markup_from_visible_targets() {
        let patterns = vec![pattern("emphasis.basic", vec![r"(?:太字|傍点)］"])];
        let selected = vec!["emphasis.basic".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            concat!(
                "扨、私事《わたくしこと》、［＃「扨、私事、」は太字］、",
                "牛《ベゴ》の舌［＃「牛の舌」に傍点］、",
                "せれ［＃注記］んとする乎［＃「せれんとする乎」に傍点］",
            ),
            &patterns,
            &selected,
        );
        let partitions = &summary.row_semantic_partitions["emphasis.basic"];

        assert_eq!(
            partitions["selector:quoted;referent:adjacent-rich-unique"].occurrences,
            3
        );
        assert!(!partitions.contains_key("selector:quoted;referent:missing"));
    }

    #[test]
    fn semantic_partitions_select_an_adjacent_rich_repeat_over_an_earlier_plain_match() {
        let patterns = vec![pattern("emphasis.basic", vec![r"太字］"])];
        let selected = vec!["emphasis.basic".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            "に錯けば。に錯《お》けば［＃「に錯けば」は太字］",
            &patterns,
            &selected,
        );

        assert_eq!(
            summary.row_semantic_partitions["emphasis.basic"]
                ["selector:quoted;referent:adjacent-rich-repeated"]
                .occurrences,
            1
        );
    }

    #[test]
    fn semantic_partitions_keep_punctuation_separated_plain_references_detached() {
        let patterns = vec![pattern("emphasis.basic", vec![r"傍点］"])];
        let selected = vec!["emphasis.basic".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            "ひと\nひと！［＃「ひと」に傍点］",
            &patterns,
            &selected,
        );

        assert_eq!(
            summary.row_semantic_partitions["emphasis.basic"]
                ["selector:quoted;referent:detached-repeated"]
                .occurrences,
            1
        );
    }

    #[test]
    fn semantic_partition_uses_nested_marker_global_offset() {
        let patterns = vec![pattern("emphasis.basic", vec![r"［＃[^］]+傍点］"])];
        let selected = vec!["emphasis.basic".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            "｜音《おと［＃「と」に傍点］》",
            &patterns,
            &selected,
        );

        assert_eq!(
            summary.row_semantic_partitions["emphasis.basic"]
                ["selector:quoted;referent:adjacent-unique"]
                .occurrences,
            1
        );
    }

    #[test]
    fn selected_emphasis_rows_partition_scope_boundaries() {
        let patterns = vec![pattern(
            "emphasis.basic",
            vec![r"［＃(?:傍点|傍線|太字|斜体)(?:終わり)?］"],
        )];
        let selected = vec!["emphasis.basic".to_owned()];
        let matched = inventory_document_with_variants(
            "matched",
            "［＃傍点］語［＃傍点終わり］",
            &patterns,
            &selected,
        );
        assert_eq!(
            matched.row_semantic_partitions["emphasis.basic"]["boundary:matched;family:boten"]
                .occurrences,
            1
        );

        let malformed = inventory_document_with_variants(
            "malformed",
            "［＃傍点］語［＃傍線終わり］［＃斜体終わり］",
            &patterns,
            &selected,
        );
        let partitions = &malformed.row_semantic_partitions["emphasis.basic"];
        assert_eq!(
            partitions["boundary:mismatched;opened:boten;closed:bousen"].occurrences,
            1
        );
        assert_eq!(
            partitions["boundary:mismatched;opened:boten;closed:italic"].occurrences,
            1
        );
        assert_eq!(
            partitions["boundary:orphan-open;family:boten"].occurrences,
            1
        );
    }

    #[test]
    fn semantic_partitions_exclude_front_matter_examples() {
        let patterns = vec![pattern(
            "emphasis.basic",
            vec![r"［＃(?:傍点)(?:終わり)?］"],
        )];
        let selected = vec!["emphasis.basic".to_owned()];
        let separator = "-------------------------------------------------------";
        let source = format!(
            "題名\n{separator}\n［＃］：入力者注\n（例）［＃傍点］\n{separator}\n［＃傍点］本文［＃傍点終わり］"
        );
        let summary = inventory_document_with_variants("w1", &source, &patterns, &selected);
        let partitions = &summary.row_semantic_partitions["emphasis.basic"];

        assert_eq!(partitions.len(), 1);
        assert_eq!(partitions["boundary:matched;family:boten"].occurrences, 1);
    }

    #[test]
    fn variant_samples_descend_to_commands_without_promoting_wrappers() {
        let patterns = vec![pattern("emphasis.basic", vec![r"［＃[^］]+傍点］"])];
        let selected = vec!["emphasis.basic".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            "｜音《おと［＃「と」に傍点］》 ※［＃「大※」に傍点］",
            &patterns,
            &selected,
        );

        let variants = &summary.row_variants["emphasis.basic"];
        assert_eq!(variants.len(), 1);
        assert_eq!(
            variants["［＃「○○」に傍点］"].samples[0].kind,
            "CommandFullwidth"
        );
    }

    #[test]
    fn variant_descent_terminates_inside_accent_notation() {
        let patterns = vec![pattern("emphasis.basic", vec![r"［＃[^］]+は斜体］"])];
        let selected = vec!["emphasis.basic".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            "〔Je me suis arme' （sic［＃「sic」は斜体］） justice.〕",
            &patterns,
            &selected,
        );

        assert_eq!(
            summary.row_variants["emphasis.basic"]["［＃「○○」は斜体］"].occurrences,
            1
        );
    }

    #[test]
    fn unknown_command_is_reported_with_raw_body_and_span() {
        let patterns = vec![pattern("layout.yokogumi", vec![r"［＃ここから横組み］"])];
        let summary = inventory_document("w1", "［＃謎の注記］", &patterns);

        assert_eq!(summary.markers_total, 1);
        assert_eq!(summary.unknown_examples.len(), 1);
        assert_eq!(summary.unknown_examples[0].work_id, "w1");
        assert_eq!(summary.unknown_examples[0].raw, "［＃謎の注記］");
        assert_eq!(summary.unknown_examples[0].body, "謎の注記");
    }

    #[test]
    fn tortoise_brackets_follow_the_lexical_accent_or_literal_class() {
        let patterns = vec![
            pattern("accent.diacritic", vec![r"〔[A-Za-z][^〕]*〕"]),
            pattern("source.bracket_note", vec![]),
        ];
        let summary = inventory_document("w1", "〔cafe'〕と〔中略〕", &patterns);

        assert_eq!(summary.unknown_examples, []);
        assert_eq!(summary.row_counts["accent.diacritic"].occurrences, 1);
        assert_eq!(summary.row_counts["source.bracket_note"].occurrences, 1);
    }

    #[test]
    fn selected_bracket_notes_retain_their_located_occurrences() {
        let patterns = vec![
            pattern("accent.diacritic", vec![r"〔[A-Za-z][^〕]*〕"]),
            pattern("source.bracket_note", vec![]),
        ];
        let selected = vec!["source.bracket_note".to_owned()];
        let summary = inventory_document_with_all_variant_occurrences(
            "w1",
            "〔cafe'〕と〔中略〕",
            &patterns,
            &selected,
        );
        let variant = &summary.row_variants["source.bracket_note"]["〔中略〕"];

        assert_eq!(summary.row_counts["source.bracket_note"].occurrences, 1);
        assert_eq!(variant.samples[0].byte_start, Some(14));
    }

    #[test]
    fn catch_all_editor_note_row_does_not_exclude_unmapped_commands() {
        let patterns = vec![
            pattern("editor_note.unmapped", vec![r"［＃[^］]+］"]),
            pattern("layout.yokogumi", vec![r"［＃ここから横組み］"]),
        ];
        let summary = inventory_document("w1", "［＃謎の注記］", &patterns);

        assert_eq!(summary.markers_total, 1);
        assert_eq!(summary.unknown_examples.len(), 1);
        assert_eq!(summary.unknown_examples[0].raw, "［＃謎の注記］");
        assert!(!summary.row_counts.contains_key("editor_note.unmapped"));
    }

    #[test]
    fn adjacent_markers_can_classify_composite_source_patterns() {
        let patterns = vec![
            pattern("gaiji.marker", vec![r"※［＃[^］]+］"]),
            pattern("ruby.basic", vec![r"《[^》]+》"]),
            pattern("gaiji_ruby.inline_base", vec![r"※［＃[^］]+］《[^》]+》"]),
        ];
        let summary = inventory_document("w1", "※［＃「口＋世」、U+546D］《おくび》", &patterns);

        assert_eq!(summary.markers_total, 2);
        assert_eq!(summary.row_counts["gaiji.marker"].occurrences, 1);
        assert_eq!(summary.row_counts["ruby.basic"].occurrences, 1);
        assert_eq!(summary.row_counts["gaiji_ruby.inline_base"].occurrences, 1);
        assert!(summary.unknown_examples.is_empty());
    }

    #[test]
    fn editorial_and_segment_markers_are_inventory_visible() {
        let summary = inventory_document(
            "w1",
            "［ルビの「おもて」は底本では「うら」］\n［＃地付き］（fixture）\n底本：fixture",
            &[],
        );

        assert_eq!(summary.markers_total, 2);
        assert_eq!(summary.unknown_examples.len(), 2);
        assert!(summary.unknown_examples.iter().any(|example| {
            example.kind == "EditorialNoteRubyCorrection"
                && example.raw == "［ルビの「おもて」は底本では「うら」］"
        }));
        assert!(summary.unknown_examples.iter().any(|example| {
            example.kind == "SegmentBoundaryTerminalProvenance"
                && example.raw == "［＃地付き］（fixture）"
        }));
    }

    #[test]
    fn bottom_text_correction_notes_default_to_annotation_row() {
        let patterns = vec![
            pattern("annotation.chuuki", vec![]),
            pattern("ruby.basic", vec![]),
        ];
        let summary = inventory_document(
            "w1",
            "豌豆《ゑんどう》「豌豆」は底本では「跣豆」］の大さ",
            &patterns,
        );

        assert_eq!(summary.markers_total, 2);
        assert_eq!(summary.unknown_examples, []);
        assert_eq!(
            summary
                .row_counts
                .get("annotation.chuuki")
                .map(|count| count.occurrences),
            Some(1)
        );
    }

    #[test]
    fn selected_annotation_count_equals_its_retained_source_forms() {
        let patterns = vec![
            pattern(
                "annotation.chuuki",
                vec![r"［＃[^］]*(底本|ママ|注記)[^］]*］"],
            ),
            pattern("source.bracket_note", vec![]),
            pattern("gaiji.marker", vec![r"※［＃[^］]+］"]),
        ];
        let selected = vec!["annotation.chuuki".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            concat!(
                "喋［＃「喋」に「ママ」の注記］\n",
                "広場へに［＃「広場へに」はママ］\n",
                "〔欄外に〕\n",
                "※［＃「土へん＋占」、U+576B、13-註【三三】］\n",
            ),
            &patterns,
            &selected,
        );

        let variants = &summary.row_variants["annotation.chuuki"];
        let retained: u64 = variants.values().map(|variant| variant.occurrences).sum();
        assert_eq!(retained, 2);
        assert_eq!(
            summary.row_counts["annotation.chuuki"].occurrences,
            retained
        );
        assert_eq!(summary.row_counts["source.bracket_note"].occurrences, 1);
        assert_eq!(summary.row_counts["gaiji.marker"].occurrences, 1);
    }

    #[test]
    fn selected_annotation_row_partitions_semantic_families_and_boundaries() {
        let patterns = vec![pattern(
            "annotation.chuuki",
            vec![r"［＃[^］]*(底本|ママ|注記)[^］]*］"],
        )];
        let selected = vec!["annotation.chuuki".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            concat!(
                "喋［＃「喋」に「ママ」の注記］\n",
                "［＃左に注記付き］勝安房守［＃左に「本ト麟太郎」の注記付き終わり］\n",
                "広場へに［＃「広場へに」はママ］\n",
                "語《ご》［＃ルビの「ご」は底本では「こ」］\n",
                "［＃底本では「欠字」が脱落］\n",
            ),
            &patterns,
            &selected,
        );

        let partitions = &summary.row_semantic_partitions["annotation.chuuki"];
        for partition in [
            "form:attached-note",
            "form:paired-open",
            "form:paired-close",
            "form:ruby-apparatus",
            "form:source-apparatus",
            "relation:annotation",
            "relation:sic",
            "relation:omission",
            "position:left",
            "boundary:matched",
        ] {
            assert!(partitions.contains_key(partition), "missing {partition}");
        }
        assert_eq!(partitions["boundary:matched"].occurrences, 2);
        assert!(
            summary.row_variants["annotation.chuuki"]
                .values()
                .all(|variant| !variant.semantic_partitions.is_empty())
        );
    }

    #[test]
    fn annotation_uncertainty_ignores_punctuation_inside_quoted_source_text() {
        assert!(
            annotation_facts("「ご存じなんです？」は底本では「ご存じないんです？」")
                .contains(&"certainty:asserted".to_owned())
        );
        assert!(
            annotation_facts("「安の定」は「案の定」の誤記か")
                .contains(&"certainty:uncertain".to_owned())
        );
        assert!(annotation_facts("「涕」はママ").contains(&"form:sic-note".to_owned()));
    }

    #[test]
    fn annotation_semantics_ignore_vocabulary_inside_quoted_targets() {
        let formatting = annotation_facts("「[以下省略]」は太字");
        assert!(formatting.contains(&"form:editorial-statement".to_owned()));
        assert!(formatting.contains(&"relation:formatting".to_owned()));
        assert!(!formatting.contains(&"form:omission-note".to_owned()));
        assert!(!formatting.contains(&"relation:omission".to_owned()));

        let omission = annotation_facts("「年表」省略");
        assert!(omission.contains(&"form:omission-note".to_owned()));
        assert!(omission.contains(&"relation:omission".to_owned()));

        let uncertain_omission = annotation_facts("「や」が欠如か");
        assert!(uncertain_omission.contains(&"form:omission-note".to_owned()));
        assert!(uncertain_omission.contains(&"certainty:uncertain".to_owned()));
    }

    #[test]
    fn annotation_semantics_distinguish_addition_relations() {
        let duplication = annotation_facts("底本では「に」が重複");
        assert!(duplication.contains(&"relation:duplication".to_owned()));
        assert!(!duplication.contains(&"relation:addition".to_owned()));

        let excess = annotation_facts("「み」余分か、それとも「見」か？");
        assert!(excess.contains(&"relation:possible-excess".to_owned()));
        assert!(excess.contains(&"certainty:uncertain".to_owned()));

        for body in ["改行を挿入", "底本にはない「。」を補った"] {
            assert!(annotation_facts(body).contains(&"relation:addition".to_owned()));
        }

        assert!(
            annotation_facts("現代語訳「月は明るい。」")
                .contains(&"relation:supplied-translation".to_owned())
        );
        assert!(
            annotation_facts("「弟」は誤訳で本当は「兄」")
                .contains(&"relation:translation-error".to_owned())
        );
    }

    #[test]
    fn annotation_variant_partitions_do_not_inherit_quoted_target_vocabulary() {
        let patterns = vec![pattern(
            "annotation.chuuki",
            vec![r"［＃[^］]*太字[^］]*］"],
        )];
        let selected = vec!["annotation.chuuki".to_owned()];
        let summary = inventory_document_with_variants(
            "w1",
            "「[以下省略]」［＃「[以下省略]」は太字］\n本文［＃「本文」は太字］\n",
            &patterns,
            &selected,
        );

        let partitions =
            &summary.row_variants["annotation.chuuki"]["［＃「○○」は太字］"].semantic_partitions;
        assert_eq!(partitions["form:editorial-statement"], 2);
        assert_eq!(partitions["relation:formatting"], 2);
        assert!(!partitions.contains_key("form:omission-note"));
        assert!(!partitions.contains_key("relation:omission"));
    }

    #[test]
    fn region_classification_preserves_examples_without_body_coverage() {
        let text = "題名\n著者\n----------\n【テキスト中に現れる記号について】\n［＃］：入力者注\n（例）［＃本文終わり］\n（例）漢字《かんじ》\n----------\n［＃］\n漢字《かんじ》\n［＃本文終わり］\n翻訳の底本：例\n";
        let summary = inventory_document("w", text, &[pattern("ruby.basic", vec![])]);
        assert_eq!(summary.markers_total, 6);
        assert_eq!(summary.source_region_events.front_matter_occurrences, 3);
        assert_eq!(
            summary.source_region_events.body_end_boundary_occurrences,
            1
        );
        assert_eq!(summary.classified_region_markers.len(), 4);
        assert_eq!(summary.row_counts["ruby.basic"].occurrences, 1);
        assert_eq!(summary.unknown_examples.len(), 1);
        assert_eq!(summary.unknown_examples[0].line, 9);
    }

    #[test]
    fn bare_cr_line_endings_keep_legend_examples_out_of_the_body() {
        let text = "題名\r----------\r【テキスト中に現れる記号について】\r《》：ルビ\r（例）白い泡《あわ》\r----------\r\r本文《ほんぶん》。\r";
        let selected = vec!["ruby.basic".to_owned()];
        let summary = inventory_document_with_variants(
            "w",
            text,
            &[pattern("ruby.basic", vec![])],
            &selected,
        );
        assert_eq!(summary.source_region_events.front_matter_occurrences, 2);
        for partition in summary.row_semantic_partitions["ruby.basic"].values() {
            assert!(
                partition
                    .samples
                    .iter()
                    .all(|sample| sample.raw == "《ほんぶん》"),
                "{:?}",
                partition.samples
            );
        }
    }

    #[test]
    fn back_matter_examples_do_not_measure_body_syntax() {
        let text = concat!(
            "題名\n著者\n----------\n本文\n\n",
            "［＃本文終わり］\n\n",
            "底本：語［＃「語」に傍線］\n",
        );
        let patterns = [pattern(
            "decoration.bousen",
            vec![r"［＃「[^」]+」に傍線］"],
        )];
        let summary = inventory_document("w", text, &patterns);

        assert!(!summary.row_counts.contains_key("decoration.bousen"));
        assert_eq!(summary.source_region_events.back_matter_occurrences, 1);
        assert!(summary.classified_region_markers.iter().any(|marker| {
            marker.region == SourceMarkerRegion::BackMatter
                && marker.occurrence.raw == "［＃「語」に傍線］"
        }));
    }

    #[test]
    fn unfenced_legend_and_inline_boundary_remain_unreviewed() {
        let summary = inventory_document(
            "w",
            "【テキスト中に現れる記号について】\n［＃］\n引用「［＃本文終わり］」\n",
            &[],
        );
        assert_eq!(summary.unknown_examples.len(), 2);
        assert!(summary.classified_region_markers.is_empty());
    }

    #[test]
    fn variant_editorial_header_is_source_apparatus() {
        let summary = inventory_document(
            "w",
            "題名\n［表記について］\n●［＃］は、入力者注を示す。\n----------\n本文\n",
            &[],
        );
        assert_eq!(summary.source_region_events.front_matter_occurrences, 1);
        assert!(summary.unknown_examples.is_empty());
    }

    #[test]
    fn kaeriten_variants_carry_source_owned_semantic_partitions() {
        let patterns = vec![pattern(
            "kunten.kaeriten",
            vec![
                r"［＃[一二三四五六七八九十レ上中下甲乙丙丁天地人]+］",
                r"［＃[^］]*返り点[^］]*］",
            ],
        )];
        let selected = vec!["kunten.kaeriten".to_owned()];
        let summary = inventory_document_with_variants(
            "w",
            "漢［＃レ］［＃ここから２字下げ、「一銭」の「一」をのぞいて「レ一二」は返り点］\n文［＃返り点の「二」の右横に縦棒あり］",
            &patterns,
            &selected,
        );

        let partitions = &summary.row_semantic_partitions["kunten.kaeriten"];
        for expected in [
            "form:bare",
            "target:preceding-character",
            "form:declaration",
            "target:following-source-range",
            "scope:following-block",
            "marker:compound",
            "exclusion:present",
            "layout:indent-open",
            "form:insertion",
            "target:insertion-site",
            "adjunct:vertical-rule-right",
        ] {
            assert!(partitions.contains_key(expected), "missing {expected}");
        }
        assert_eq!(
            summary.row_variants["kunten.kaeriten"]["［＃レ］"].semantic_partitions["form:bare"],
            1
        );
    }

    fn pattern(row_id: &str, source_patterns: Vec<&str>) -> SourceInventoryPattern {
        SourceInventoryPattern {
            row_id: row_id.to_owned(),
            source_patterns: source_patterns.into_iter().map(str::to_owned).collect(),
        }
    }
}
