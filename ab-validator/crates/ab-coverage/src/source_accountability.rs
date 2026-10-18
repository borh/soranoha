//! Source-authority occurrences independent of any parser interpretation.

use std::collections::BTreeMap;

use ab_encoding::{decode_source_bytes, hex_sha256};
use ab_source_syntax::{SourceMarker, SourceMarkerKind, aozora_body_range, source_markers};
use serde_json::{Value, json};

use crate::source_inventory::{
    CompiledSourceInventoryPattern, SourceInventoryPattern, SourceMarkerRegion,
    append_composite_matching_rows, compile_patterns, inventory_document_observed,
    jis_gaiji_semantic_occurrences, kaeriten_semantic_partitions, matching_rows,
    nested_reading_ruby_occurrences, normalize_marker_form, semantic_partition_occurrences,
    semantic_partition_rows,
};

/// Record lexical evidence without treating recognition as semantic support.
/// Spans address the decoded UTF-8 text, whose hash is distinct from source bytes.
#[must_use]
pub fn source_accountability(
    bytes: &[u8],
    matrix_bytes: &[u8],
    patterns: &[SourceInventoryPattern],
) -> Value {
    let mut report = json!({
        "schema": "aozora-source-accountability/2",
        "source_sha256": format!("sha256:{}", hex_sha256(bytes)),
        "matrix_sha256": format!("sha256:{}", hex_sha256(matrix_bytes)),
        "coordinate_system": "decoded_utf8",
        "scope": "lexical-source-markers",
        "semantic_coverage": "not-assessed",
        "occurrences": [],
    });
    let decoded = match decode_source_bytes(bytes) {
        Ok(decoded) => decoded,
        Err(error) => {
            report["decode_outcome"] = json!("failed");
            report["decode_error"] = json!(error.to_string());
            return report;
        }
    };
    report["encoding"] = json!(decoded.encoding);
    report["decode_outcome"] = json!(if decoded.encoding == "windows-31j-lossy" {
        "lossy"
    } else {
        "lossless"
    });
    report["decoded_sha256"] = json!(format!("sha256:{}", hex_sha256(decoded.text.as_bytes())));
    let (body, _) = aozora_body_range(&decoded.text);
    report["lexical_body_range"] = json!({"start": body.start, "end": body.end});
    let mut occurrences = Vec::new();
    let component_patterns: Vec<_> = patterns
        .iter()
        .filter(|pattern| {
            matches!(
                pattern.row_id.as_str(),
                "kunten.kaeriten"
                    | "kunten.okurigana"
                    | "ruby.basic"
                    | "accent.diacritic"
                    | "accent.dotted_letter"
                    | "glyph.variant_note"
                    | "annotation.chuuki"
                    | "iteration.kunoji"
                    | "gaiji_ruby.inline_base"
                    | "gaiji.marker"
                    | "gaiji.jis_code"
                    | "gaiji.unicode_codepoint"
            )
        })
        .cloned()
        .collect();
    let compiled_patterns = compile_patterns(&component_patterns);
    let all_patterns = compile_patterns(patterns);
    let kaeriten_partitions = kaeriten_semantic_partitions(&decoded.text, "", &all_patterns);
    let mut semantic_partitions_by_start = BTreeMap::<usize, Vec<String>>::new();
    for row_id in semantic_partition_rows() {
        if let Some(occurrences) =
            semantic_partition_occurrences(row_id, &decoded.text, "", &all_patterns)
        {
            for (start, partition, _) in occurrences {
                let partitions = semantic_partitions_by_start.entry(start).or_default();
                if !partitions.contains(&partition) {
                    partitions.push(partition);
                }
            }
        }
    }
    let jis_evidence_by_start = jis_gaiji_semantic_occurrences(&decoded.text, "")
        .into_iter()
        .map(|(start, form, partitions, _)| (start, (form, partitions)))
        .collect::<BTreeMap<_, _>>();
    let mut kaeriten_evidence_by_start = BTreeMap::<usize, (String, Vec<String>)>::new();
    for (start, partition, occurrence) in kaeriten_partitions {
        kaeriten_evidence_by_start
            .entry(start)
            .or_insert_with(|| (normalize_marker_form(&occurrence.raw), Vec::new()))
            .1
            .push(partition);
    }
    inventory_document_observed(
        "",
        &decoded.text,
        patterns,
        |marker, classified, families| {
            let region = match classified {
                Some(SourceMarkerRegion::FrontMatter) => "front-matter",
                Some(SourceMarkerRegion::BodyEndBoundary) => "body-end-boundary",
                Some(SourceMarkerRegion::BackMatter) => "back-matter",
                None if marker.span.start >= body.end => "back-matter",
                None if marker.span.start >= body.start && marker.span.end <= body.end => "body",
                None => "unknown-region",
            };
            let mut families = families.to_vec();
            families.sort();
            occurrences.push(json!({
                "source_span": {"start": marker.span.start, "end": marker.span.end,
                                "line": marker.span.line, "coordinate_system": "decoded_utf8"},
                "kind": format!("{:?}", marker.kind),
                "raw": marker.raw,
                "normalized_form": accountability_marker_form(marker.kind, marker.raw),
                "region": region,
                "families": families,
                "semantic_partitions": semantic_partitions_by_start
                    .get(&marker.span.start).cloned().unwrap_or_default(),
                "family_evidence": family_evidence(
                    &families,
                    marker.span.start,
                    &jis_evidence_by_start,
                    &kaeriten_evidence_by_start,
                ),
                "components": nested_components(
                    marker,
                    &compiled_patterns,
                    &semantic_partitions_by_start,
                    &jis_evidence_by_start,
                    &kaeriten_evidence_by_start,
                ),
            }));
        },
    );
    for (start, end, occurrence) in
        nested_reading_ruby_occurrences(&decoded.text, "", &all_patterns)
    {
        let normalized_form = normalize_marker_form(&occurrence.raw);
        occurrences.push(json!({
            "source_span": {"start": start, "end": end,
                            "line": occurrence.line, "coordinate_system": "decoded_utf8"},
            "kind": occurrence.kind,
            "raw": occurrence.raw,
            "normalized_form": normalized_form,
            "region": "body",
            "families": ["ruby.nested_forbidden"],
            "semantic_partitions": ["form:nested-reading", "boundary:matched",
                                    "reading-content:ruby-delimiters"],
            "components": [],
        }));
    }
    occurrences.sort_by_key(|occurrence| occurrence["source_span"]["start"].as_u64());
    report["occurrences"] = json!(occurrences);
    report
}

fn nested_components(
    marker: &SourceMarker<'_>,
    patterns: &[CompiledSourceInventoryPattern],
    semantic_partitions_by_start: &BTreeMap<usize, Vec<String>>,
    jis_evidence_by_start: &BTreeMap<usize, (String, Vec<String>)>,
    kaeriten_evidence_by_start: &BTreeMap<usize, (String, Vec<String>)>,
) -> Vec<Value> {
    let mut components = Vec::new();
    let mut pending = vec![marker.clone()];
    while let Some(parent) = pending.pop() {
        if !matches!(
            parent.kind,
            SourceMarkerKind::RubyExplicit
                | SourceMarkerKind::GaijiFullwidth
                | SourceMarkerKind::GaijiAscii
                | SourceMarkerKind::RubyImplicit
                | SourceMarkerKind::AccentNotation
                | SourceMarkerKind::BracketNote
                | SourceMarkerKind::CommandFullwidth
                | SourceMarkerKind::CommandAscii
                | SourceMarkerKind::EditorialNoteBottomTextCorrection
        ) {
            continue;
        }
        let offset = parent.span.start + parent.body.as_ptr().addr() - parent.raw.as_ptr().addr();
        for mut child in source_markers(parent.body) {
            child.span.start += offset;
            child.span.end += offset;
            child.span.line += parent.span.line - 1;
            if child.span.start <= parent.span.start && child.span.end >= parent.span.end {
                continue;
            }
            // The terminal reading delimiters belong to the enclosing explicit ruby.
            let same_ruby = parent.kind == SourceMarkerKind::RubyExplicit
                && child.kind == SourceMarkerKind::RubyImplicit
                && child.span.end == parent.span.end;
            let eligible_families: &[&str] = match child.kind {
                SourceMarkerKind::RubyExplicit => &["ruby.basic", "gaiji_ruby.inline_base"],
                SourceMarkerKind::RubyImplicit => &["ruby.basic"],
                SourceMarkerKind::IterationNotation => &["iteration.kunoji"],
                SourceMarkerKind::AccentNotation => &["accent.diacritic"],
                SourceMarkerKind::CommandFullwidth | SourceMarkerKind::CommandAscii => &[
                    "kunten.kaeriten",
                    "kunten.okurigana",
                    "accent.dotted_letter",
                    "glyph.variant_note",
                    "annotation.chuuki",
                ],
                SourceMarkerKind::GaijiFullwidth | SourceMarkerKind::GaijiAscii => {
                    &["gaiji.marker", "gaiji.jis_code", "gaiji.unicode_codepoint"]
                }
                _ => &[],
            };
            let malformed = matches!(
                child.kind,
                SourceMarkerKind::MalformedGaiji
                    | SourceMarkerKind::MalformedGaijiAscii
                    | SourceMarkerKind::MalformedCommand
                    | SourceMarkerKind::MalformedCommandAscii
                    | SourceMarkerKind::MalformedRuby
                    | SourceMarkerKind::MalformedImplicitRuby
                    | SourceMarkerKind::MalformedAccentNotation
            ) && source_markers(
                &marker.raw[child.span.start - marker.span.start..],
            )
            .first()
            .is_some_and(|complete| complete.kind == child.kind && complete.raw == child.raw);
            if !same_ruby && (!eligible_families.is_empty() || malformed) {
                let mut families = matching_rows(child.raw, patterns);
                families.retain(|family| eligible_families.contains(&family.as_str()));
                families.sort();
                if !families.is_empty()
                    || malformed
                    || child.kind == SourceMarkerKind::AccentNotation
                {
                    components.push(json!({
                        "source_span": {"start": child.span.start, "end": child.span.end,
                                        "line": child.span.line, "coordinate_system": "decoded_utf8"},
                        "kind": format!("{:?}", child.kind),
                        "raw": child.raw,
                        "normalized_form": accountability_marker_form(child.kind, child.raw),
                        "families": families,
                        "semantic_partitions": semantic_partitions_by_start
                            .get(&child.span.start).cloned().unwrap_or_default(),
                        "family_evidence": family_evidence(
                            &families,
                            child.span.start,
                            jis_evidence_by_start,
                            kaeriten_evidence_by_start,
                        ),
                    }));
                }
            }
            pending.push(child);
        }
    }
    components.sort_by_key(|component| component["source_span"]["start"].as_u64());
    // Composite identity follows source adjacency, independently of quotation nesting.
    for index in 0..components.len().saturating_sub(1) {
        let left = &components[index];
        let right = &components[index + 1];
        if left["kind"] != "GaijiFullwidth" && left["kind"] != "GaijiAscii" {
            continue;
        }
        if right["kind"] != "RubyImplicit"
            || left["source_span"]["end"] != right["source_span"]["start"]
        {
            continue;
        }
        let start = usize::try_from(
            left["source_span"]["start"]
                .as_u64()
                .expect("component start"),
        )
        .expect("source index");
        let end = usize::try_from(right["source_span"]["end"].as_u64().expect("component end"))
            .expect("source index");
        let mut families = Vec::new();
        append_composite_matching_rows(
            &mut families,
            &marker.raw[start - marker.span.start..end - marker.span.start],
            left["raw"].as_str().expect("component spelling"),
            right["raw"].as_str().expect("component spelling"),
            patterns,
        );
        if families
            .iter()
            .any(|family| family == "gaiji_ruby.inline_base")
        {
            let families = components[index]["families"]
                .as_array_mut()
                .expect("component families");
            families.push(json!("gaiji_ruby.inline_base"));
            families.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
        }
    }
    components
}

fn family_evidence(
    families: &[String],
    start: usize,
    jis_evidence_by_start: &BTreeMap<usize, (String, Vec<String>)>,
    kaeriten_evidence_by_start: &BTreeMap<usize, (String, Vec<String>)>,
) -> Value {
    let mut evidence = serde_json::Map::new();
    if families.iter().any(|family| family == "gaiji.jis_code")
        && let Some((form, partitions)) = jis_evidence_by_start.get(&start)
    {
        evidence.insert(
            "gaiji.jis_code".to_owned(),
            json!({
                "normalized_form": form,
                "semantic_partitions": partitions,
            }),
        );
    }
    if families.iter().any(|family| family == "kunten.kaeriten")
        && let Some((form, partitions)) = kaeriten_evidence_by_start.get(&start)
    {
        evidence.insert(
            "kunten.kaeriten".to_owned(),
            json!({
                "normalized_form": form,
                "semantic_partitions": partitions,
            }),
        );
    }
    Value::Object(evidence)
}

fn accountability_marker_form(kind: SourceMarkerKind, raw: &str) -> String {
    match kind {
        SourceMarkerKind::GaijiFullwidth => "※［＃…］".to_owned(),
        SourceMarkerKind::GaijiAscii => "※[#…]".to_owned(),
        SourceMarkerKind::RubyExplicit => "｜○○《○○》".to_owned(),
        SourceMarkerKind::RubyImplicit => "《○○》".to_owned(),
        _ => normalize_marker_form(raw),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kaeriten_accountability_keeps_family_owned_form_and_partitions() {
        let patterns = vec![SourceInventoryPattern {
            row_id: "kunten.kaeriten".into(),
            source_patterns: vec![r"［＃[^］]*返り点[^］]*］".into()],
        }];
        let report = source_accountability(
            "［＃ここから２字下げ、「レ一二」は返り点］\n漢文".as_bytes(),
            b"matrix",
            &patterns,
        );
        let occurrence = &report["occurrences"][0];
        let evidence = &occurrence["family_evidence"]["kunten.kaeriten"];

        assert_eq!(
            evidence["normalized_form"],
            "［＃ここからN字下げ、「○○」は返り点］"
        );
        assert!(
            evidence["semantic_partitions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|partition| partition == "target:following-source-range")
        );
    }

    #[test]
    fn emphasis_occurrences_expose_source_semantic_partitions() {
        let patterns = vec![SourceInventoryPattern {
            row_id: "emphasis.basic".into(),
            source_patterns: vec![r"［＃(?:「[^」]+」に)?傍点(?:終わり)?］".into()],
        }];
        let report = source_accountability(
            "語［＃「語」に傍点］［＃傍点］字［＃傍点終わり］".as_bytes(),
            b"matrix",
            &patterns,
        );
        let occurrences = report["occurrences"].as_array().unwrap();

        assert_eq!(
            occurrences[0]["semantic_partitions"],
            json!(["selector:quoted;referent:adjacent-unique"])
        );
        assert_eq!(
            occurrences[2]["semantic_partitions"],
            json!(["boundary:matched;family:boten"])
        );
    }

    #[test]
    fn tcy_occurrences_expose_source_semantic_partitions() {
        let patterns = vec![SourceInventoryPattern {
            row_id: "layout.tcy".into(),
            source_patterns: vec![r"［＃[^］]*縦中横[^］]*］".into()],
        }];
        let report = source_accountability(
            "12［＃「12」は縦中横］［＃縦中横］10［＃縦中横終わり］".as_bytes(),
            b"matrix",
            &patterns,
        );
        let occurrences = report["occurrences"].as_array().unwrap();

        assert!(
            occurrences[0]["semantic_partitions"]
                .as_array()
                .unwrap()
                .contains(&json!("selector:quoted;referent:adjacent-unique"))
        );
        assert!(
            occurrences[1]["semantic_partitions"]
                .as_array()
                .unwrap()
                .contains(&json!("boundary:matched-open"))
        );
        assert!(
            occurrences[2]["semantic_partitions"]
                .as_array()
                .unwrap()
                .contains(&json!("boundary:matched-close"))
        );
    }

    #[test]
    fn rotation_occurrences_expose_source_semantic_partitions() {
        let patterns = vec![SourceInventoryPattern {
            row_id: "layout.rotation".into(),
            source_patterns: vec![r"［＃「[^」]+」は[０-９0-9]+度回転］".into()],
        }];
        let report = source_accountability(
            "月二［＃「月二」は１８０度回転］".as_bytes(),
            b"matrix",
            &patterns,
        );
        let partitions = report["occurrences"][0]["semantic_partitions"]
            .as_array()
            .unwrap();

        for expected in [
            "effect:rotation",
            "angle-degrees:180",
            "direction:unspecified",
            "selector:quoted;referent:adjacent-unique",
        ] {
            assert!(partitions.contains(&json!(expected)), "missing {expected}");
        }
    }

    #[test]
    fn front_reference_occurrences_expose_their_own_semantic_partitions() {
        let patterns = vec![SourceInventoryPattern {
            row_id: "reference.frontref".into(),
            source_patterns: vec![r"［＃「[^」]+」に傍点、底本では「[^」]+」に傍点］".into()],
        }];
        let report = source_accountability(
            "あとさし［＃「あとさし」に傍点、底本では「あとさ」に傍点］".as_bytes(),
            b"matrix",
            &patterns,
        );
        let occurrences = report["occurrences"].as_array().unwrap();

        assert_eq!(
            occurrences[0]["semantic_partitions"],
            json!(["selector:plural;referent:detached-unique"])
        );
    }

    #[test]
    fn bousen_occurrences_share_the_corpus_partition_registry() {
        let patterns = vec![SourceInventoryPattern {
            row_id: "decoration.bousen".into(),
            source_patterns: vec![r"［＃[^］]*(?:傍線|鎖線|破線|波線|上線|下線)[^］]*］".into()],
        }];
        let source = "線［＃「線」に二重傍線］［＃左に波線］波［＃左に波線終わり］";
        let report = source_accountability(source.as_bytes(), b"matrix", &patterns);
        let fixture_partitions = report["occurrences"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|occurrence| {
                occurrence["semantic_partitions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|partition| partition.as_str().unwrap().to_owned())
            })
            .collect::<std::collections::BTreeSet<_>>();
        let selected = vec!["decoration.bousen".to_owned()];
        let corpus = crate::source_inventory::inventory_document_with_variants(
            "work", source, &patterns, &selected,
        );
        let corpus_partitions = corpus.row_semantic_partitions["decoration.bousen"]
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(fixture_partitions, corpus_partitions);
        for expected in [
            "effect:bousen",
            "form:retrospective",
            "line-kind:double",
            "position:left",
            "boundary:matched;family:bousen",
        ] {
            assert!(fixture_partitions.contains(expected), "missing {expected}");
        }
    }

    #[test]
    fn indentation_occurrences_expose_layout_semantic_partitions() {
        let patterns = vec![SourceInventoryPattern {
            row_id: "indentation.basic".into(),
            source_patterns: vec![
                r"［＃[０-９0-9]+字下げ］".into(),
                r"［＃ここから[０-９0-9]+字下げ］".into(),
                r"［＃地付き］".into(),
            ],
        }];
        let report = source_accountability(
            "［＃２字下げ］行\n［＃ここから３字下げ］\n本文\n［＃ここで字下げ終わり］\n［＃地付き］署名"
                .as_bytes(),
            b"matrix",
            &patterns,
        );
        let occurrences = report["occurrences"].as_array().unwrap();

        assert_eq!(
            occurrences[0]["semantic_partitions"],
            json!([
                "form:line-indent",
                "scope:line",
                "indent:2",
                "placement:line-prefix"
            ])
        );
        assert_eq!(
            occurrences[1]["semantic_partitions"],
            json!([
                "form:block-indent",
                "scope:block",
                "indent:3",
                "placement:standalone-line",
                "boundary:matched-close"
            ])
        );
        assert_eq!(
            occurrences[3]["semantic_partitions"],
            json!([
                "form:line-align-end",
                "scope:line",
                "alignment:end",
                "placement:line-prefix"
            ])
        );
    }

    #[test]
    fn chitsuki_occurrences_expose_offset_and_target_partitions() {
        let patterns = vec![SourceInventoryPattern {
            row_id: "indentation.chitsuki".into(),
            source_patterns: vec![r"［＃[^］]*(?:地付き|字上げ)[^］]*］".into()],
        }];
        let report = source_accountability(
            "［＃地から２字上げ］署名\n幕。［＃「幕。」は地付き］".as_bytes(),
            b"matrix",
            &patterns,
        );
        let occurrences = report["occurrences"].as_array().unwrap();
        let offset = occurrences[0]["semantic_partitions"].as_array().unwrap();
        let quoted = occurrences[1]["semantic_partitions"].as_array().unwrap();

        for expected in [
            json!("form:line-offset"),
            json!("target:same-line-remainder"),
            json!("offset-from-end:2"),
        ] {
            assert!(offset.contains(&expected));
        }
        for expected in [
            json!("form:retrospective-align-end"),
            json!("target:quoted-previous-occurrence"),
            json!("selector:quoted;referent:adjacent-unique"),
        ] {
            assert!(quoted.contains(&expected));
        }
    }

    #[test]
    fn cross_line_head_alignment_exposes_relative_and_collective_partitions() {
        let source = concat!(
            "一首目\n二首目\n三首目［＃この歌、二行前の歌に頭揃え。］\n",
            "［＃で各歌の頭は全て揃っている。］",
        );
        let patterns = vec![SourceInventoryPattern {
            row_id: "layout.cross_line_head_alignment".into(),
            source_patterns: vec![
                r"［＃この歌、二行前の歌に頭揃え。］".into(),
                r"［＃で各歌の頭は全て揃っている。］".into(),
            ],
        }];
        let report = source_accountability(source.as_bytes(), b"matrix", &patterns);
        let occurrences = report["occurrences"].as_array().unwrap();

        for expected in [
            "effect:line-start-alignment",
            "relation:relative-line",
            "lines-before:2",
            "target:two-lines-before",
            "scope:current-line",
        ] {
            assert!(
                occurrences[0]["semantic_partitions"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(expected)),
                "{expected}: {}",
                occurrences[0]
            );
        }
        for expected in [
            "effect:line-start-alignment",
            "relation:collective",
            "operand:各歌",
            "target:unresolved-members",
            "scope:named-group",
        ] {
            assert!(
                occurrences[1]["semantic_partitions"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(expected)),
                "{expected}: {}",
                occurrences[1]
            );
        }

        let selected = vec!["layout.cross_line_head_alignment".to_owned()];
        let corpus = crate::source_inventory::inventory_document_with_variants(
            "work", source, &patterns, &selected,
        );
        let corpus_partitions = corpus.row_semantic_partitions["layout.cross_line_head_alignment"]
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let fixture_partitions = occurrences
            .iter()
            .flat_map(|occurrence| {
                occurrence["semantic_partitions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|partition| partition.as_str().unwrap().to_owned())
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(fixture_partitions, corpus_partitions);
    }

    #[test]
    fn oneline_jisage_occurrences_expose_spelling_and_target_partitions() {
        let patterns = vec![SourceInventoryPattern {
            row_id: "indentation.jisage_oneline".into(),
            source_patterns: vec![
                r"［＃この行[０-９0-9]+字下げ］".into(),
                r"［＃天から[０-９0-9]+字下げ］".into(),
                r"［＃[０-９0-9]+字下がる］".into(),
                r"［＃[０-９0-9]+字下げ］".into(),
            ],
        }];
        let report =
            source_accountability("［＃この行２字下げ］本文".as_bytes(), b"matrix", &patterns);
        let occurrence = &report["occurrences"][0];

        assert_eq!(
            occurrence["semantic_partitions"],
            json!([
                "form:explicit-this-line",
                "scope:line",
                "target:same-line-remainder",
                "indent:2",
                "digits:fullwidth",
                "placement:line-prefix",
                "content:present"
            ])
        );
    }

    #[test]
    fn block_jisage_occurrences_expose_compound_geometry_and_boundaries() {
        let patterns = vec![SourceInventoryPattern {
            row_id: "indentation.jisage_block".into(),
            source_patterns: vec![r"［＃[^］]*字下げ[^］]*］".into()],
        }];
        let report = source_accountability(
            concat!(
                "［＃ここから一字下げ、折り返して二字下げ、２１字詰め］\n",
                "本文\n［＃ここで字下げ終わり］",
            )
            .as_bytes(),
            b"matrix",
            &patterns,
        );
        let occurrences = report["occurrences"].as_array().unwrap();
        let opener = occurrences
            .iter()
            .find(|occurrence| {
                occurrence["raw"]
                    .as_str()
                    .unwrap()
                    .starts_with("［＃ここから")
            })
            .unwrap();
        let partitions = opener["semantic_partitions"].as_array().unwrap();

        for partition in [
            "form:block-open",
            "geometry:hanging-indent",
            "geometry:line-width",
            "indent:1",
            "continuation-indent:2",
            "width:21",
            "boundary:matched-close",
        ] {
            assert!(
                partitions.contains(&json!(partition)),
                "{partition}: {opener}"
            );
        }
    }

    #[test]
    fn heading_occurrences_expose_form_level_target_and_boundary_partitions() {
        let patterns = vec![SourceInventoryPattern {
            row_id: "heading.basic".into(),
            source_patterns: vec![
                r#"［＃「[^\n]*」は[大中小]見出し］"#.into(),
                r"［＃(ここから|ここで)?[大中小]見出し?(終わり)?］".into(),
                r"［＃ここで見出し終わり］".into(),
            ],
        }];
        let report = source_accountability(
            concat!(
                "章《しょう》［＃「章」は中見出し］\n",
                "［＃ここから大見出し］\n本文\n［＃ここで見出し終わり］",
            )
            .as_bytes(),
            b"matrix",
            &patterns,
        );
        let occurrences = report["occurrences"].as_array().unwrap();

        let forward = occurrences
            .iter()
            .find(|occurrence| occurrence["raw"] == "［＃「章」は中見出し］")
            .unwrap();
        for partition in [
            "form:forward",
            "level:2",
            "referent:adjacent-rich",
            "target-structure:ruby",
        ] {
            assert!(
                forward["semantic_partitions"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(partition))
            );
        }

        let close = occurrences
            .iter()
            .find(|occurrence| occurrence["raw"] == "［＃ここで見出し終わり］")
            .unwrap();
        for partition in [
            "form:block-close",
            "level:inherited",
            "boundary:matched-open",
        ] {
            assert!(
                close["semantic_partitions"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(partition))
            );
        }
    }

    #[test]
    fn ruby_occurrences_expose_source_semantic_partitions() {
        let patterns = vec![SourceInventoryPattern {
            row_id: "ruby.basic".into(),
            source_patterns: vec![],
        }];
        let report = source_accountability(
            "稍々《やや》、｜お伽話《フェヤリー・ストーリース》".as_bytes(),
            b"matrix",
            &patterns,
        );
        let occurrences = report["occurrences"].as_array().unwrap();

        assert_eq!(
            occurrences[0]["semantic_partitions"],
            json!([
                "form:implicit",
                "preceding-class:special-kanji",
                "reading-content:plain"
            ])
        );
        assert_eq!(
            occurrences[1]["semantic_partitions"],
            json!([
                "form:explicit",
                "base-content:mixed",
                "reading-content:plain"
            ])
        );
    }

    #[test]
    fn gaiji_ruby_occurrences_expose_source_semantic_partitions() {
        let patterns = vec![SourceInventoryPattern {
            row_id: "gaiji_ruby.inline_base".into(),
            source_patterns: vec![r"※［＃[^］]+］《[^》]+》".into()],
        }];
        let report = source_accountability(
            "※［＃「口＋愛」、第3水準1-15-23］《おくび》".as_bytes(),
            b"matrix",
            &patterns,
        );
        let occurrences = report["occurrences"].as_array().unwrap();
        let gaiji = occurrences
            .iter()
            .find(|occurrence| occurrence["kind"] == "GaijiFullwidth")
            .unwrap();

        assert_eq!(
            gaiji["semantic_partitions"],
            json!([
                "marker-syntax:fullwidth",
                "role:ruby-base",
                "identifier:jis",
                "form:implicit",
                "gaiji-count:single",
                "base-shape:gaiji-only"
            ])
        );
    }

    #[test]
    fn accent_occurrences_expose_source_semantic_partitions() {
        let patterns = vec![SourceInventoryPattern {
            row_id: "accent.diacritic".into(),
            source_patterns: vec![r"〔[A-Za-z][^〕]*〕".into()],
        }];
        let report = source_accountability("〔AE& E' e~ L'art〕".as_bytes(), b"matrix", &patterns);
        let occurrence = &report["occurrences"].as_array().unwrap()[0];

        assert_eq!(
            occurrence["semantic_partitions"],
            json!([
                "substitutions:multiple",
                "resolution:partial",
                "spelling-width:mixed",
                "letter-case:mixed",
                "byte-effect:mixed",
                "marker:acute",
                "marker:ring-or-ligature",
                "marker:tilde"
            ])
        );
    }

    #[test]
    fn directional_ruby_occurrences_expose_source_semantic_partitions() {
        let matrix = r#"
[[syntax]]
id = "ruby.placement_directional"
source_patterns = ['［＃「[^」]+」の左に「[^」]+」のルビ］']
"#;
        let patterns = vec![SourceInventoryPattern {
            row_id: "ruby.placement_directional".to_owned(),
            source_patterns: vec!["［＃「[^」]+」の左に「[^」]+」のルビ］".to_owned()],
        }];
        let source = "青空《あおぞら》［＃「青空」の左に「blue」のルビ］";
        let report = source_accountability(source.as_bytes(), matrix.as_bytes(), &patterns);
        let occurrence = report["occurrences"]
            .as_array()
            .expect("occurrences")
            .iter()
            .find(|occurrence| {
                occurrence["families"].as_array().is_some_and(|families| {
                    families
                        .iter()
                        .any(|family| family == "ruby.placement_directional")
                })
            })
            .expect("directional ruby occurrence");
        let partitions = occurrence["semantic_partitions"]
            .as_array()
            .expect("semantic partitions");
        for expected in [
            "form:forward",
            "referent:adjacent-rich",
            "target-structure:ruby",
        ] {
            assert!(partitions.iter().any(|partition| partition == expected));
        }
    }

    #[test]
    fn double_ruby_occurrences_expose_both_reading_partitions() {
        let matrix = r#"
[[syntax]]
id = "ruby.double"
source_patterns = ['《[^》]+》［＃「[^」]+」の(左|右)に「[^」]+」のルビ］']
"#;
        let patterns = vec![SourceInventoryPattern {
            row_id: "ruby.double".to_owned(),
            source_patterns: vec![
                "《[^》]+》［＃「[^」]+」の(左|右)に「[^」]+」のルビ］".to_owned(),
            ],
        }];
        let source = "食《ヲセ（？）》［＃「食」の左に「クヘ（？）」のルビ］";
        let report = source_accountability(source.as_bytes(), matrix.as_bytes(), &patterns);
        let occurrence = report["occurrences"]
            .as_array()
            .expect("occurrences")
            .iter()
            .find(|occurrence| {
                occurrence["families"]
                    .as_array()
                    .is_some_and(|families| families.iter().any(|family| family == "ruby.double"))
            })
            .expect("double ruby occurrence");
        let partitions = occurrence["semantic_partitions"]
            .as_array()
            .expect("semantic partitions");
        for expected in [
            "form:directional",
            "placement:right-left",
            "right-reading-notation:question-mark",
            "left-reading-notation:question-mark",
        ] {
            assert!(partitions.iter().any(|partition| partition == expected));
        }
    }

    #[test]
    fn nested_reading_ruby_keeps_complete_synthetic_accountability() {
        let matrix = r#"
[[syntax]]
id = "ruby.nested_forbidden"
source_patterns = ['｜[^《\n]+《[^》\n]*《[^》\n]+》[^》\n]*》']
"#;
        let patterns = vec![SourceInventoryPattern {
            row_id: "ruby.nested_forbidden".to_owned(),
            source_patterns: vec!["｜[^《\\n]+《[^》\\n]*《[^》\\n]+》[^》\\n]*》".to_owned()],
        }];
        let report = source_accountability(
            "｜漢《か《ん》じ》".as_bytes(),
            matrix.as_bytes(),
            &patterns,
        );
        let occurrence = report["occurrences"]
            .as_array()
            .expect("occurrences")
            .iter()
            .find(|occurrence| occurrence["kind"] == "NestedRuby")
            .expect("synthetic nested ruby occurrence");

        assert_eq!(occurrence["raw"], "｜漢《か《ん》じ》");
        assert_eq!(occurrence["families"], json!(["ruby.nested_forbidden"]));
        assert_eq!(occurrence["source_span"]["start"], 0);
        assert_eq!(occurrence["source_span"]["end"], 27);
    }

    #[test]
    fn nested_malformed_prefixes_remain_lexical_evidence() {
        for (source, kind, raw) in [
            ("｜※［＃字」然《ぼうぜん》", "MalformedGaiji", "※［＃"),
            ("｜※[#字」然《ぼうぜん》", "MalformedGaijiAscii", "※[#"),
            ("｜前［＃謎」後《よみ》", "MalformedCommand", "［＃"),
            ("｜前[#謎」後《よみ》", "MalformedCommandAscii", "[#"),
            ("［＃「《」に傍点］", "MalformedImplicitRuby", "《"),
            ("｜前〔e'後《よみ》", "MalformedAccentNotation", "〔"),
        ] {
            let source = format!("前\n{source}");
            let marker = source_markers(&source).remove(0);
            let components = nested_components(
                &marker,
                &[],
                &BTreeMap::new(),
                &BTreeMap::new(),
                &BTreeMap::new(),
            );
            assert_eq!(components.len(), 1, "{source}: {components:?}");
            let component = &components[0];
            assert_eq!(component["kind"], kind);
            assert_eq!(component["raw"], raw);
            assert_eq!(component["families"], json!([]));
            assert_eq!(component["source_span"]["line"], 2);
            let start =
                usize::try_from(component["source_span"]["start"].as_u64().unwrap()).unwrap();
            let end = usize::try_from(component["source_span"]["end"].as_u64().unwrap()).unwrap();
            assert_eq!(&source[start..end], raw);
            assert!(marker.span.start < start && end < marker.span.end);
        }
        let marker = source_markers("［＃「｜」に傍点］").remove(0);
        assert!(
            nested_components(
                &marker,
                &[],
                &BTreeMap::new(),
                &BTreeMap::new(),
                &BTreeMap::new(),
            )
            .is_empty()
        );
    }

    #[test]
    fn truncated_nested_fragments_do_not_establish_malformed_source() {
        for source in [
            "〔訳注〔cafe'〕続き〕",
            "［＃「〔e'〕」は底本では「〔a'※［＃字］〕」］",
        ] {
            let marker = source_markers(source).remove(0);
            assert!(
                nested_components(
                    &marker,
                    &[],
                    &BTreeMap::new(),
                    &BTreeMap::new(),
                    &BTreeMap::new(),
                )
                .iter()
                .all(|child| child["kind"] != "MalformedAccentNotation"),
                "{source}"
            );
        }
    }

    #[test]
    fn nested_accents_are_kept_without_matrix_credit_and_retain_gaiji() {
        let source = "前\n［＃「〔e'〕」は底本では「〔a'※［＃字］〕」］";
        let marker = source_markers(source).remove(0);
        let patterns = vec![SourceInventoryPattern {
            row_id: "gaiji.marker".into(),
            source_patterns: vec!["※［＃".into()],
        }];
        let components = nested_components(
            &marker,
            &compile_patterns(&patterns),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        );
        assert_eq!(components.len(), 3);
        for (component, (kind, raw, families)) in components.iter().zip([
            ("AccentNotation", "〔e'〕", json!([])),
            ("AccentNotation", "〔a'※［＃字］〕", json!([])),
            ("GaijiFullwidth", "※［＃字］", json!(["gaiji.marker"])),
        ]) {
            assert_eq!(component["kind"], kind);
            assert_eq!(component["raw"], raw);
            assert_eq!(component["families"], families);
            let start =
                usize::try_from(component["source_span"]["start"].as_u64().unwrap()).unwrap();
            let end = usize::try_from(component["source_span"]["end"].as_u64().unwrap()).unwrap();
            assert_eq!(&source[start..end], raw);
            assert_eq!(component["source_span"]["line"], 2);
        }
    }

    #[test]
    fn gaiji_accountability_uses_the_source_owned_delimiter_form() {
        let patterns = vec![SourceInventoryPattern {
            row_id: "gaiji.marker".into(),
            source_patterns: vec!["※［＃".into(), r"※\[#".into()],
        }];
        let report = source_accountability("※［＃字］ ※[#字]".as_bytes(), b"matrix", &patterns);
        let forms = report["occurrences"]
            .as_array()
            .unwrap()
            .iter()
            .map(|occurrence| occurrence["normalized_form"].as_str().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(forms, ["※［＃…］", "※[#…]"]);
    }

    #[test]
    fn shared_gaiji_occurrence_keeps_family_specific_jis_evidence() {
        let patterns = vec![
            SourceInventoryPattern {
                row_id: "gaiji.marker".into(),
                source_patterns: vec!["※［＃".into()],
            },
            SourceInventoryPattern {
                row_id: "gaiji.jis_code".into(),
                source_patterns: vec![r"第[1-4]水準[12]-[0-9]+-[0-9]+".into()],
            },
        ];
        let report = source_accountability(
            "※［＃「てへん＋掌」、第4水準2-13-47］".as_bytes(),
            b"matrix",
            &patterns,
        );
        let occurrence = &report["occurrences"][0];

        assert_eq!(occurrence["normalized_form"], "※［＃…］");
        assert_eq!(
            occurrence["family_evidence"]["gaiji.jis_code"]["normalized_form"],
            "※［＃○○、第N水準N-N-N］"
        );
        assert!(
            occurrence["family_evidence"]["gaiji.jis_code"]["semantic_partitions"]
                .as_array()
                .unwrap()
                .contains(&json!("plane:2"))
        );
        assert!(
            occurrence["semantic_partitions"]
                .as_array()
                .unwrap()
                .contains(&json!("identifier:jis"))
        );
    }

    #[test]
    fn nested_glyph_components_keep_independent_source_spans() {
        let source = "前※［＃「姉」の正字、「女＋※［＃第3水準1-85-57］のつくり」、252-下-27］後";
        let patterns = vec![SourceInventoryPattern {
            row_id: "gaiji.marker".into(),
            source_patterns: vec!["※［＃".into()],
        }];
        let marker = source_markers(source).remove(0);
        let components = nested_components(
            &marker,
            &compile_patterns(&patterns),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        );
        assert_eq!(components.len(), 1);
        assert_eq!(components[0]["raw"], "※［＃第3水準1-85-57］");
        let start =
            usize::try_from(components[0]["source_span"]["start"].as_u64().unwrap()).unwrap();
        let end = usize::try_from(components[0]["source_span"]["end"].as_u64().unwrap()).unwrap();
        assert_eq!(&source[start..end], "※［＃第3水準1-85-57］");
        assert!(marker.span.start < start && end < marker.span.end);
    }

    #[test]
    fn nested_iteration_components_retain_source_owned_partitions() {
        let source = "［＃「とき／″＼」は太字］";
        let marker = source_markers(source).remove(0);
        let patterns = vec![SourceInventoryPattern {
            row_id: "iteration.kunoji".into(),
            source_patterns: vec!["／″＼".into()],
        }];
        let partitions = BTreeMap::from([(
            15,
            vec!["form:voiced".into(), "role:directive-reference".into()],
        )]);
        let components = nested_components(
            &marker,
            &compile_patterns(&patterns),
            &partitions,
            &BTreeMap::new(),
            &BTreeMap::new(),
        );

        assert_eq!(components.len(), 1);
        assert_eq!(components[0]["raw"], "／″＼");
        assert_eq!(
            components[0]["semantic_partitions"],
            json!(["form:voiced", "role:directive-reference"])
        );
    }

    #[test]
    fn nested_gaiji_ruby_uses_the_same_adjacent_marker_identity_as_top_level() {
        let patterns = vec![
            SourceInventoryPattern {
                row_id: "gaiji.marker".into(),
                source_patterns: vec![r"※［＃[^］]+］".into()],
            },
            SourceInventoryPattern {
                row_id: "ruby.basic".into(),
                source_patterns: vec![r"《[^》]+》".into()],
            },
            SourceInventoryPattern {
                row_id: "gaiji_ruby.inline_base".into(),
                source_patterns: vec![r"※［＃[^］]+］《[^》]+》".into()],
            },
        ];
        for (fragment, associated) in [
            ("※［＃字］《じ》", true),
            ("※［＃字］別《べつ》", false),
            ("※［＃字］ 《じ》", false),
            ("※［＃字］《じ", false),
        ] {
            let source = format!("［＃「{fragment}」は底本では「別」］");
            let report = source_accountability(source.as_bytes(), b"matrix", &patterns);
            let components = report["occurrences"][0]["components"].as_array().unwrap();
            let gaiji = components
                .iter()
                .find(|item| item["kind"] == "GaijiFullwidth")
                .unwrap();
            assert_eq!(
                gaiji["families"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|family| family == "gaiji_ruby.inline_base"),
                associated
            );
            assert_eq!(gaiji["raw"], "※［＃字］");
            let start = usize::try_from(gaiji["source_span"]["start"].as_u64().unwrap()).unwrap();
            let end = usize::try_from(gaiji["source_span"]["end"].as_u64().unwrap()).unwrap();
            assert_eq!(&source[start..end], "※［＃字］");
            assert!(
                components
                    .iter()
                    .filter(|item| item["kind"] == "RubyImplicit")
                    .all(|item| item["families"] == json!(["ruby.basic"]))
            );
        }
        let source = "［＃「※［＃字］《じ》」は底本では「※［＃字］《じ》」］";
        let report = source_accountability(source.as_bytes(), b"matrix", &patterns);
        let gaiji = report["occurrences"][0]["components"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|component| component["kind"] == "GaijiFullwidth")
            .collect::<Vec<_>>();
        assert_eq!(gaiji.len(), 2);
        assert!(gaiji.iter().all(|component| {
            component["families"]
                .as_array()
                .unwrap()
                .contains(&json!("gaiji_ruby.inline_base"))
        }));
    }

    #[test]
    fn nested_explicit_ruby_keeps_its_whole_association_extent() {
        let patterns = vec![
            SourceInventoryPattern {
                row_id: "gaiji.marker".into(),
                source_patterns: vec![r"※［＃[^］]+］".into()],
            },
            SourceInventoryPattern {
                row_id: "ruby.basic".into(),
                source_patterns: vec![r"《[^》]+》".into()],
            },
            SourceInventoryPattern {
                row_id: "gaiji_ruby.inline_base".into(),
                source_patterns: vec![r"※［＃[^］]+］《[^》]+》".into()],
            },
        ];
        let source = "［＃「｜前※［＃字］《ぜんじ》」は底本では「別」］";
        let report = source_accountability(source.as_bytes(), b"matrix", &patterns);
        let components = report["occurrences"][0]["components"].as_array().unwrap();
        assert_eq!(components.len(), 2);
        assert_eq!(components[0]["kind"], "RubyExplicit");
        assert_eq!(components[0]["raw"], "｜前※［＃字］《ぜんじ》");
        assert_eq!(
            components[0]["families"],
            json!(["gaiji_ruby.inline_base", "ruby.basic"])
        );
        assert_eq!(components[1]["kind"], "GaijiFullwidth");
        assert_eq!(components[1]["families"], json!(["gaiji.marker"]));
    }

    #[test]
    fn edition_notes_inside_brackets_keep_their_own_source_extent() {
        let patterns = vec![SourceInventoryPattern {
            row_id: "annotation.chuuki".into(),
            source_patterns: vec!["」は底本では「".into()],
        }];
        for (base, command) in [
            ("Hu:lshoff", "［＃「Hu:lshoff」は底本では「Hu:lshoffs」］"),
            (
                "schla:gt",
                "［＃「〔schla:gt〕」は底本では「〔scha:gt〕」］",
            ),
        ] {
            let source = format!("題\n作者\n\n〔前{base}{command}後〕\n");
            let report = source_accountability(source.as_bytes(), b"matrix", &patterns);
            let components = report["occurrences"][0]["components"].as_array().unwrap();
            let component = components
                .iter()
                .find(|node| node["raw"] == command)
                .unwrap();
            assert_eq!(component["kind"], "CommandFullwidth");
            assert_eq!(component["families"], json!(["annotation.chuuki"]));
            let start = component["source_span"]["start"].as_u64().unwrap() as usize;
            let end = component["source_span"]["end"].as_u64().unwrap() as usize;
            assert_eq!(&source[start..end], command);
            assert_eq!(component["source_span"]["line"], 4);
        }
    }

    #[test]
    fn iteration_components_keep_original_coordinates_without_expansion() {
        let source = "題\n作者\n\nフゴ／＼。｜時／″＼《とき／＼》。／゛＼〳〵\n";
        let patterns = vec![
            SourceInventoryPattern {
                row_id: "iteration.kunoji".into(),
                source_patterns: vec!["／＼".into(), "／″＼".into()],
            },
            SourceInventoryPattern {
                row_id: "ruby.basic".into(),
                source_patterns: vec!["《[^》]+》".into()],
            },
        ];
        let report = source_accountability(source.as_bytes(), b"matrix", &patterns);
        let occurrences = report["occurrences"].as_array().unwrap();
        assert_eq!(occurrences.len(), 2);
        assert_eq!(occurrences[0]["kind"], "IterationNotation");
        let components = occurrences[1]["components"].as_array().unwrap();
        assert_eq!(components.len(), 2);
        for event in std::iter::once(&occurrences[0]).chain(components) {
            let span = &event["source_span"];
            let start = span["start"].as_u64().unwrap() as usize;
            let end = span["end"].as_u64().unwrap() as usize;
            assert_eq!(&source[start..end], event["raw"].as_str().unwrap());
            assert_eq!(event["families"], json!(["iteration.kunoji"]));
        }
    }

    #[test]
    fn gaiji_occurrences_keep_token_owned_semantic_partitions() {
        let source = "題\n作者\n\n※［＃字、第3水準1-15-23］、｜※［＃音］《※［＃聲、12-3］》\n";
        let patterns = vec![SourceInventoryPattern {
            row_id: "gaiji.marker".into(),
            source_patterns: vec![r"※［＃[^］]+］".into()],
        }];
        let report = source_accountability(source.as_bytes(), b"matrix", &patterns);
        let occurrences = report["occurrences"].as_array().unwrap();

        assert_eq!(
            occurrences[0]["semantic_partitions"],
            json!(["marker-syntax:fullwidth", "role:body", "identifier:jis"])
        );
        let components = occurrences[1]["components"].as_array().unwrap();
        assert_eq!(components[0]["kind"], "GaijiFullwidth");
        assert!(
            components[0]["semantic_partitions"]
                .as_array()
                .unwrap()
                .contains(&json!("role:ruby-base"))
        );
        assert_eq!(components[1]["kind"], "GaijiFullwidth");
        assert!(
            components[1]["semantic_partitions"]
                .as_array()
                .unwrap()
                .contains(&json!("role:ruby-reading"))
        );
    }

    #[test]
    fn quoted_command_components_are_lexical_evidence_with_exact_spelling() {
        for (open, close) in [("［＃", "］"), ("[#", "]")] {
            let source =
                format!("題\n作者\n\n{open}ルビの「わざ／＼」は底本では「わさ／″＼」{close}\n");
            let patterns = vec![SourceInventoryPattern {
                row_id: "iteration.kunoji".into(),
                source_patterns: vec!["／＼".into(), "／″＼".into()],
            }];
            let report = source_accountability(source.as_bytes(), b"matrix", &patterns);
            let occurrences = report["occurrences"].as_array().unwrap();
            assert_eq!(occurrences.len(), 1);
            let components = occurrences[0]["components"].as_array().unwrap();
            assert_eq!(components.len(), 2, "{report}");
            for (component, expected) in components.iter().zip(["／＼", "／″＼"]) {
                let span = &component["source_span"];
                let start = span["start"].as_u64().unwrap() as usize;
                let end = span["end"].as_u64().unwrap() as usize;
                assert_eq!(&source[start..end], expected);
                assert_eq!(component["raw"], expected);
                assert_eq!(component["families"], json!(["iteration.kunoji"]));
                assert!(component.get("claims").is_none());
            }
        }
    }

    #[test]
    fn quoted_ruby_and_its_nested_marks_remain_distinct_components() {
        let source = "題\n作者\n\n［＃「漢《かん／＼》」は底本では「字《じ》」］\n";
        let patterns = vec![
            SourceInventoryPattern {
                row_id: "ruby.basic".into(),
                source_patterns: vec!["《[^》]+》".into()],
            },
            SourceInventoryPattern {
                row_id: "iteration.kunoji".into(),
                source_patterns: vec!["／＼".into()],
            },
        ];
        let report = source_accountability(source.as_bytes(), b"matrix", &patterns);
        let occurrences = report["occurrences"].as_array().unwrap();
        assert_eq!(occurrences.len(), 1);
        let components = occurrences[0]["components"].as_array().unwrap();
        assert_eq!(components.len(), 3);
        for (component, (raw, family)) in components.iter().zip([
            ("《かん／＼》", "ruby.basic"),
            ("／＼", "iteration.kunoji"),
            ("《じ》", "ruby.basic"),
        ]) {
            let span = &component["source_span"];
            assert_eq!(
                &source[span["start"].as_u64().unwrap() as usize
                    ..span["end"].as_u64().unwrap() as usize],
                raw
            );
            assert_eq!(component["raw"], raw);
            assert_eq!(component["families"], json!([family]));
        }

        let image = "［＃「漢《かん》」のキャプション付きの図（fig1.png、横20×縦30）入る］";
        let report = source_accountability(image.as_bytes(), b"matrix", &patterns);
        let components = report["occurrences"][0]["components"].as_array().unwrap();
        assert_eq!(components.len(), 1);
        assert_eq!(components[0]["raw"], "《かん》");
        assert_eq!(components[0]["kind"], "RubyImplicit");
        assert_eq!(components[0]["families"], json!(["ruby.basic"]));
        let span = &components[0]["source_span"];
        assert_eq!(
            &image
                [span["start"].as_u64().unwrap() as usize..span["end"].as_u64().unwrap() as usize],
            "《かん》"
        );
    }

    #[test]
    fn evidence_has_exact_decoded_spans_and_no_semantic_claim() {
        let source = "題名\n著者\n----------\n【テキスト中に現れる記号について】\n［＃］：入力者注\n----------\n漢字《かんじ》［＃］\n［＃本文終わり］\n底本：例\n";
        let patterns = vec![SourceInventoryPattern {
            row_id: "ruby.basic".into(),
            source_patterns: vec![],
        }];
        let report = source_accountability(source.as_bytes(), b"matrix", &patterns);
        let events = report["occurrences"].as_array().unwrap();
        assert_eq!(events.len(), 4);
        assert_eq!(events[0]["region"], "front-matter");
        assert_eq!(events[1]["families"], json!(["ruby.basic"]));
        assert_eq!(events[2]["region"], "body");
        assert_eq!(events[2]["families"], json!([]));
        assert_eq!(events[3]["region"], "body-end-boundary");
        for event in events {
            let start = event["source_span"]["start"].as_u64().unwrap() as usize;
            let end = event["source_span"]["end"].as_u64().unwrap() as usize;
            assert_eq!(&source[start..end], event["raw"].as_str().unwrap());
        }
        assert_eq!(report["semantic_coverage"], "not-assessed");
    }

    #[test]
    fn composite_kunten_keep_one_occurrence_and_exact_component_extents() {
        let source = "題\n作者\n\n｜遊［＃二］松島［＃一］記《まつしまにあそぶき》\n";
        let patterns = vec![SourceInventoryPattern {
            row_id: "kunten.kaeriten".into(),
            source_patterns: vec!["［＃[一二]］".into()],
        }];
        let report = source_accountability(source.as_bytes(), b"matrix", &patterns);
        let occurrences = report["occurrences"].as_array().unwrap();
        assert_eq!(occurrences.len(), 1);
        let components = occurrences[0]["components"].as_array().unwrap();
        assert_eq!(components.len(), 2);
        for (component, raw) in components.iter().zip(["［＃二］", "［＃一］"]) {
            assert_eq!(component["raw"], raw);
            let span = &component["source_span"];
            assert_eq!(
                &source[span["start"].as_u64().unwrap() as usize
                    ..span["end"].as_u64().unwrap() as usize],
                raw
            );
            assert_eq!(component["families"], json!(["kunten.kaeriten"]));
        }
    }

    #[test]
    fn decoding_failures_and_loss_are_explicit() {
        assert_eq!(
            source_accountability(&[0xef, 0xbb, 0xbf, 0xff], b"", &[])["decode_outcome"],
            "failed"
        );
        assert_eq!(
            source_accountability(&[0x81], b"", &[])["decode_outcome"],
            "lossy"
        );
        let plain = source_accountability("本文".as_bytes(), b"", &[]);
        let bom = source_accountability("\u{feff}本文".as_bytes(), b"", &[]);
        assert_eq!(plain["decoded_sha256"], bom["decoded_sha256"]);
        assert_ne!(plain["source_sha256"], bom["source_sha256"]);
    }
}
