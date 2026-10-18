use ab_aozora_facade::syntax::ast::{
    FormattingEffect, FormattingInstruction, FormattingPlacement, FormattingPredecessor,
    FormattingRuleStyle, FormattingSelector, HeadingTypography, KuntenPolicy, KuntenScope,
    LineStartAlignment,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, iter, mem, ops::Range, slice};

use super::{DecodedSource, EstablishedInterpretation, Span, formatting_fields, span_json};

fn operand(decoded: &DecodedSource, span: ab_aozora_facade::Span) -> Value {
    let span = Span::from(span);
    let start = decoded.span_ctx.to_decoded(span.start);
    let end = decoded.span_ctx.to_decoded_end(span.end);
    json!({"text":&decoded.text[start..end], "span":span_json(&span, &decoded.span_ctx)})
}

fn selector(decoded: &DecodedSource, selector: &FormattingSelector) -> Value {
    match selector {
        FormattingSelector::Unresolved => json!({"kind":"unresolved"}),
        FormattingSelector::PrecedingText => json!({"kind":"preceding-text"}),
        FormattingSelector::Quoted(span) => {
            json!({"kind":"quoted", "operands":[operand(decoded, *span)]})
        }
        FormattingSelector::Parenthesized(span) => {
            json!({"kind":"parenthesized", "operands":[operand(decoded, *span)]})
        }
        FormattingSelector::Named(span) => {
            json!({"kind":"named", "operands":[operand(decoded, *span)]})
        }
        FormattingSelector::Subtarget {
            enclosing,
            operand: selected,
        } => {
            json!({"kind":"subtarget", "enclosing":operand(decoded, *enclosing), "operands":[operand(decoded, *selected)]})
        }
        FormattingSelector::EachQuoted(spans) => {
            json!({"kind":"each-quoted", "operands":spans.iter().map(|span| operand(decoded, *span)).collect::<Vec<_>>()})
        }
        FormattingSelector::AllQuoted(span) => {
            json!({"kind":"all-quoted", "operands":[operand(decoded, *span)]})
        }
        FormattingSelector::Digits => json!({"kind":"digits"}),
        FormattingSelector::ArabicDigitsAfter(predecessor) => {
            json!({"kind":"arabic-digits-after", "predecessor":match predecessor {
                FormattingPredecessor::Alphabetic => json!({"kind":"alphabetic"}),
                FormattingPredecessor::NamedCharacters(spans) => json!({"kind":"named-characters", "operands":spans.iter().map(|span| operand(decoded, *span)).collect::<Vec<_>>()}),
            }})
        }
        FormattingSelector::Occurrence {
            ordinal,
            operand: span,
        } => json!({"kind":"occurrence", "ordinal":ordinal, "operands":[operand(decoded, *span)]}),
        FormattingSelector::CharacterPosition {
            ordinal,
            operand: span,
        } => {
            json!({"kind":"character-position", "ordinal":ordinal, "operands":[operand(decoded, *span)]})
        }
        FormattingSelector::EachLine => json!({"kind":"each-line"}),
        FormattingSelector::FollowingText => json!({"kind":"following-text"}),
        FormattingSelector::FollowingLines { lines } => {
            json!({"kind":"following-lines", "lines":lines})
        }
        FormattingSelector::QuotedWhole(span) => {
            json!({"kind":"quoted-whole", "operands":[operand(decoded, *span)]})
        }
    }
}

fn relative_placement(decoded: &DecodedSource, placement: FormattingPlacement) -> Value {
    let FormattingPlacement::BelowDayRange {
        start_operand,
        end_operand,
        start_day,
        end_day,
        start_target,
        end_target,
    } = placement
    else {
        unreachable!("relative placement requires a day-range anchor");
    };
    let endpoint = |day, operand_span, target: Option<ab_aozora_facade::Span>| {
        let mut endpoint = json!({"day":day, "operand":operand(decoded, operand_span)});
        if let Some(target) = target {
            endpoint["target"] = span_json(&target.into(), &decoded.span_ctx);
        }
        endpoint
    };
    json!({
        "kind":"relative-placement",
        "relation":"below",
        "anchor":{
            "kind":"day-range",
            "start":endpoint(start_day, start_operand, start_target),
            "end":endpoint(end_day, end_operand, end_target),
        },
    })
}

fn kunten_policy(decoded: &DecodedSource, policy: &KuntenPolicy) -> Value {
    json!({
        "kind":"kunten-policy",
        "marks":operand(decoded, policy.marks),
        "scope":match policy.scope {
            KuntenScope::QuotedRegion => "quoted-region",
            KuntenScope::FollowingText => "following-text",
            KuntenScope::InsertionSite => "insertion-site",
            KuntenScope::FollowingBlock => "following-block",
            KuntenScope::PrecedingBlock => "preceding-block",
        },
        "exclusions":policy.exclusions.iter().map(|exclusion| json!({
            "enclosing":operand(decoded, exclusion.enclosing),
            "mark":operand(decoded, exclusion.mark),
        })).collect::<Vec<_>>(),
        "repeat_following":policy.repeat_following,
        "vertical_rule_right":policy.vertical_rule_right,
    })
}

pub(super) fn project(
    decoded: &DecodedSource,
    marker: Span,
    instruction: &FormattingInstruction,
) -> Value {
    let start = decoded.span_ctx.to_decoded(marker.start);
    let end = decoded.span_ctx.to_decoded_end(marker.end);
    let operations = instruction.operations.iter().map(|operation| {
        let effect = match operation.effect.clone() {
            FormattingEffect::Attribute(attribute) => json!({"kind":"attribute", "attribute":formatting_fields(attribute).expect("native instruction attribute has AAT fields")}),
            FormattingEffect::UpperRelativeTo { operand: base, target } => {
                let mut effect = json!({"kind":"upper-relative", "base":operand(decoded, base)});
                if let Some(target) = target { effect["base_target"] = span_json(&target.into(), &decoded.span_ctx); }
                effect
            },
            FormattingEffect::ReducedPointLines { placement, lines, parentheses_lines } => {
                let placement = match placement {
                    FormattingPlacement::Below(anchor) => json!({"kind":"below", "anchor":operand(decoded, anchor)}),
                    FormattingPlacement::Between { preceding, following } => json!({"kind":"between", "preceding":operand(decoded, preceding), "following":operand(decoded, following)}),
                    FormattingPlacement::BelowDayRange { .. } => unreachable!("day-range placement is a standalone formatting effect"),
                };
                let mut effect = json!({"kind":"reduced-point-lines", "placement":placement, "lines":lines});
                if let Some(lines) = parentheses_lines { effect["parentheses_lines"] = json!(lines); }
                effect
            }
            FormattingEffect::HorizontalCenter { lines } => {
                json!({"kind":"horizontal-center", "lines":lines})
            }
            FormattingEffect::LineSetting { lines, parentheses_lines } => {
                let mut effect = json!({"kind":"line-setting", "lines":lines});
                if let Some(lines) = parentheses_lines { effect["parentheses_lines"] = json!(lines); }
                effect
            }
            FormattingEffect::RelativePlacement(placement) => relative_placement(decoded, placement),
            FormattingEffect::Rotation { degrees } => {
                json!({"kind":"rotation", "degrees":degrees})
            }
            FormattingEffect::Layout { indent, horizontal, align_end } => {
                json!({"kind":"layout-policy", "indent":indent, "horizontal":horizontal, "align_end":align_end})
            }
            FormattingEffect::LetterSpacing { amount } => {
                json!({"kind":"letter-spacing", "amount":amount})
            }
            FormattingEffect::LineStartAlignment(relation) => match relation {
                LineStartAlignment::RelativeLine { lines_before, reference_target } => {
                    let mut relation = json!({"kind":"relative-line", "lines_before":lines_before});
                    if let Some(target) = reference_target {
                        relation["target"] = span_json(&target.into(), &decoded.span_ctx);
                    }
                    json!({"kind":"line-start-alignment", "relation":relation})
                }
                LineStartAlignment::Collective { members } => json!({
                    "kind":"line-start-alignment",
                    "relation":{"kind":"collective", "members":operand(decoded, members)},
                }),
            },
            FormattingEffect::HeadingTypography(typography) => {
                json!({"kind":"heading-typography", "variant":match typography {
                    HeadingTypography::Heading => "heading",
                    HeadingTypography::SmallHeading => "small-heading",
                }})
            }
            FormattingEffect::KuntenPolicy(policy) => kunten_policy(decoded, &policy),
            FormattingEffect::RuleStyle(style) => {
                json!({"kind":"rule-style", "style":match style {
                    FormattingRuleStyle::Wavy => "wavy",
                }})
            }
            FormattingEffect::Ligature { components } => {
                json!({"kind":"ligature", "components":components.iter().map(|span| operand(decoded, *span)).collect::<Vec<_>>()})
            }
        };
        let mut projected = json!({"span":span_json(&operation.source_span.into(), &decoded.span_ctx),
            "selector":selector(decoded, &operation.selector), "effect":effect});
        if let Some(selection) = &operation.selection {
            projected["selection"] = json!({"scope":span_json(&selection.scope.into(), &decoded.span_ctx),
                "targets":selection.targets.iter().map(|target| span_json(&(*target).into(), &decoded.span_ctx)).collect::<Vec<_>>()});
        } else if projected["effect"]["kind"] == "kunten-policy"
            && projected["effect"]["scope"] == "insertion-site"
        {
            projected["selection"] = json!({
                "scope":span_json(&marker, &decoded.span_ctx),
                "targets":[projected["effect"]["marks"]["span"].clone()],
            });
        }
        projected
    }).collect::<Vec<_>>();
    let mut node = json!({"kind":"formatting_instruction", "source":&decoded.text[start..end],
        "span":span_json(&marker, &decoded.span_ctx), "operations":operations});
    let interpreted_operations = interpretations(&node).map_or(0, |facts| facts.len());
    if interpreted_operations != instruction.operations.len() {
        node["interpretation_problem"] = json!({"kind":"uninterpreted-notation", "code":"uninterpreted-notation",
            "aspects":["structure","layout"], "influence":{"kind":"document"}});
    }
    node
}

fn interpretations(node: &Value) -> Option<Vec<EstablishedInterpretation>> {
    let operations = node["operations"].as_array()?;
    if operations.is_empty() {
        return None;
    }
    let mut interpretations = Vec::new();
    for operation in operations {
        let effect = &operation["effect"];
        let interpretation = match effect["kind"].as_str()? {
            "layout-policy" | "letter-spacing" | "reduced-point-lines" => {
                Some(EstablishedInterpretation::Layout)
            }
            "rule-style"
                if operation["selection"]["targets"]
                    .as_array()
                    .is_some_and(|targets| !targets.is_empty()) =>
            {
                Some(EstablishedInterpretation::Layout)
            }
            "line-start-alignment"
                if effect["relation"]["kind"] == "relative-line"
                    && effect["relation"]["target"].is_object()
                    && operation["selection"]["targets"]
                        .as_array()
                        .is_some_and(|targets| !targets.is_empty()) =>
            {
                Some(EstablishedInterpretation::Layout)
            }
            "horizontal-center" | "line-setting"
                if operation["selection"]["targets"]
                    .as_array()
                    .is_some_and(|targets| !targets.is_empty()) =>
            {
                Some(EstablishedInterpretation::Layout)
            }
            "relative-placement"
                if operation["selection"]["targets"]
                    .as_array()
                    .is_some_and(|targets| !targets.is_empty())
                    && effect["anchor"]["start"]["target"].is_object()
                    && effect["anchor"]["end"]["target"].is_object() =>
            {
                Some(EstablishedInterpretation::Layout)
            }
            "rotation"
                if operation["selection"]["targets"]
                    .as_array()
                    .is_some_and(|targets| !targets.is_empty()) =>
            {
                Some(EstablishedInterpretation::Layout)
            }
            "kunten-policy" => Some(EstablishedInterpretation::Kunten),
            "ligature"
                if operation["selection"]["targets"]
                    .as_array()
                    .is_some_and(|targets| !targets.is_empty()) =>
            {
                Some(EstablishedInterpretation::GlyphShapeAssertion)
            }
            "heading-typography"
                if operation["selection"]["targets"]
                    .as_array()
                    .is_some_and(|targets| !targets.is_empty()) =>
            {
                Some(EstablishedInterpretation::Emphasis)
            }
            "upper-relative" if effect["base_target"].is_object() => {
                Some(EstablishedInterpretation::Exponent)
            }
            "attribute"
                if operation["selection"]["targets"]
                    .as_array()
                    .is_some_and(|targets| !targets.is_empty())
                    || (operation["selector"]["kind"] == "following-text"
                        && effect["attribute"]["kind"] == "chitsuki") =>
            {
                match effect["attribute"]["kind"].as_str()? {
                    "fraction" | "keigakomi" | "yokogumi" | "tategumi" | "chitsuki" => {
                        Some(EstablishedInterpretation::Layout)
                    }
                    "tcy" | "font_size" => Some(EstablishedInterpretation::Emphasis),
                    "style"
                        if EstablishedInterpretation::is_emphasis_style(&effect["attribute"]) =>
                    {
                        Some(EstablishedInterpretation::Emphasis)
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        match interpretation {
            Some(interpretation) => interpretations.push(interpretation),
            None if operation["selector"]["kind"] == "unresolved" => {}
            None => return None,
        }
    }
    (!interpretations.is_empty()).then_some(interpretations)
}

pub(super) fn interpretation_facts(node: &Value) -> Vec<Value> {
    if node["kind"] != "formatting_instruction" {
        return Vec::new();
    }
    let span = &node["span"];
    interpretations(node)
        .into_iter()
        .flatten()
        .map(|interpretation| {
            json!({"kind":interpretation.kind(), "outcome":"established", "aspects":interpretation.aspects(node),
                "source_span":{"start":span["byte_start"], "end":span["byte_end"], "line":span["line_start"], "coordinate_system":"decoded_utf8"}})
        })
        .collect()
}

pub(super) fn rebase(instruction: &mut FormattingInstruction, offset: usize) {
    let offset = u32::try_from(offset).expect("native source offset fits u32");
    let shift = |span: &mut ab_aozora_facade::Span| {
        span.start += offset;
        span.end += offset;
    };
    for operation in &mut instruction.operations {
        shift(&mut operation.source_span);
        if let Some(selection) = &mut operation.selection {
            shift(&mut selection.scope);
            selection.targets.iter_mut().for_each(shift);
        }
        match &mut operation.selector {
            FormattingSelector::Quoted(span)
            | FormattingSelector::Parenthesized(span)
            | FormattingSelector::Named(span)
            | FormattingSelector::AllQuoted(span)
            | FormattingSelector::QuotedWhole(span)
            | FormattingSelector::Occurrence { operand: span, .. }
            | FormattingSelector::CharacterPosition { operand: span, .. } => shift(span),
            FormattingSelector::Subtarget { enclosing, operand } => {
                shift(enclosing);
                shift(operand);
            }
            FormattingSelector::EachQuoted(spans)
            | FormattingSelector::ArabicDigitsAfter(FormattingPredecessor::NamedCharacters(
                spans,
            )) => spans.iter_mut().for_each(shift),
            FormattingSelector::Unresolved
            | FormattingSelector::PrecedingText
            | FormattingSelector::Digits
            | FormattingSelector::EachLine
            | FormattingSelector::FollowingText
            | FormattingSelector::FollowingLines { .. }
            | FormattingSelector::ArabicDigitsAfter(FormattingPredecessor::Alphabetic) => {}
        }
        if let FormattingEffect::UpperRelativeTo { operand, target } = &mut operation.effect {
            shift(operand);
            if let Some(target) = target {
                shift(target);
            }
        }
        if let FormattingEffect::ReducedPointLines { placement, .. } = &mut operation.effect {
            match placement {
                FormattingPlacement::Below(anchor) => shift(anchor),
                FormattingPlacement::Between {
                    preceding,
                    following,
                } => {
                    shift(preceding);
                    shift(following);
                }
                FormattingPlacement::BelowDayRange { .. } => {}
            }
        }
        if let FormattingEffect::RelativePlacement(FormattingPlacement::BelowDayRange {
            start_operand,
            end_operand,
            start_target,
            end_target,
            ..
        }) = &mut operation.effect
        {
            shift(start_operand);
            shift(end_operand);
            if let Some(target) = start_target {
                shift(target);
            }
            if let Some(target) = end_target {
                shift(target);
            }
        }
        if let FormattingEffect::Ligature { components } = &mut operation.effect {
            components.iter_mut().for_each(shift);
        }
        if let FormattingEffect::KuntenPolicy(policy) = &mut operation.effect {
            shift(&mut policy.marks);
            for exclusion in &mut policy.exclusions {
                shift(&mut exclusion.enclosing);
                shift(&mut exclusion.mark);
            }
        }
        if let FormattingEffect::LineStartAlignment(relation) = &mut operation.effect {
            match relation {
                LineStartAlignment::RelativeLine {
                    reference_target, ..
                } => {
                    if let Some(target) = reference_target {
                        shift(target);
                    }
                }
                LineStartAlignment::Collective { members } => shift(members),
            }
        }
    }
}

#[derive(Debug)]
struct BlockKuntenPolicy {
    instruction_index: usize,
    operation_index: usize,
    marks: BTreeSet<char>,
    exclusions: Vec<(String, String)>,
    marker_span: Value,
}

pub(super) fn realize_block_kunten(block: &mut Value, source: &str) {
    let Some(instructions) = block.get("source_instructions").and_then(Value::as_array) else {
        return;
    };
    let policies = instructions
        .iter()
        .enumerate()
        .flat_map(|(instruction_index, instruction)| {
            instruction["operations"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
                .filter_map(move |(operation_index, operation)| {
                    let effect = &operation["effect"];
                    matches!(
                        effect["scope"].as_str(),
                        Some("following-block" | "preceding-block")
                    )
                    .then(|| BlockKuntenPolicy {
                        instruction_index,
                        operation_index,
                        marks: effect["marks"]["text"]
                            .as_str()
                            .unwrap_or("")
                            .chars()
                            .collect(),
                        exclusions: effect["exclusions"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|exclusion| {
                                Some((
                                    exclusion["enclosing"]["text"].as_str()?.to_owned(),
                                    exclusion["mark"]["text"].as_str()?.to_owned(),
                                ))
                            })
                            .collect(),
                        marker_span: instruction["span"].clone(),
                    })
                })
        })
        .filter(|policy| !policy.marks.is_empty())
        .collect::<Vec<_>>();

    for policy in policies {
        let Some(children) = block.get_mut("children") else {
            continue;
        };
        let Some(scope) = block_child_scope(children) else {
            continue;
        };
        let mut targets = Vec::new();
        realize_kunten_in_blocks(children, source, &policy, &[], &mut targets);
        if targets.is_empty() {
            continue;
        }
        block["source_instructions"][policy.instruction_index]["operations"]
            [policy.operation_index]["selection"] = json!({"scope":scope, "targets":targets});
    }
}

fn realize_kunten_in_blocks(
    value: &mut Value,
    source: &str,
    policy: &BlockKuntenPolicy,
    ranges: &[Range<usize>],
    targets: &mut Vec<Value>,
) {
    match value {
        Value::Array(values) => {
            for value in values {
                realize_kunten_in_blocks(value, source, policy, ranges, targets);
            }
        }
        Value::Object(object) => {
            if object.get("kind").and_then(Value::as_str) == Some("paragraph")
                && let Some(content) = object.get_mut("content").and_then(Value::as_array_mut)
            {
                realize_kunten_in_content(content, source, policy, ranges, targets);
            }
            if let Some(children) = object.get_mut("children") {
                realize_kunten_in_blocks(children, source, policy, ranges, targets);
            }
        }
        _ => {}
    }
}

fn realize_kunten_in_content(
    content: &mut Vec<Value>,
    source: &str,
    policy: &BlockKuntenPolicy,
    ranges: &[Range<usize>],
    targets: &mut Vec<Value>,
) {
    let mut realized = Vec::with_capacity(content.len());
    for mut node in mem::take(content) {
        if node["kind"] == "text"
            && let Some(parts) = split_kunten_text(&node, source, policy, ranges, targets)
        {
            realized.extend(parts);
            continue;
        }
        if node["kind"] == "kunten"
            && node["kunten_kind"] == "return-mark"
            && node["interpretation_marker_spans"].is_array()
            && node["text"]
                .as_str()
                .is_some_and(|text| text.chars().all(|mark| policy.marks.contains(&mark)))
            && let Some(span) = node.get("span").cloned()
            && let Some(start) = span["byte_start"].as_u64()
            && (ranges.is_empty()
                || ranges
                    .iter()
                    .any(|range| range.contains(&usize::try_from(start).unwrap_or(usize::MAX))))
        {
            targets.push(span);
            let marker_spans = node["interpretation_marker_spans"]
                .as_array_mut()
                .expect("checked above");
            if !marker_spans.contains(&policy.marker_span) {
                marker_spans.push(policy.marker_span.clone());
            }
        }
        if let Some(children) = node.get_mut("content").and_then(Value::as_array_mut) {
            realize_kunten_in_content(children, source, policy, ranges, targets);
        }
        realized.push(node);
    }
    *content = realized;
}

fn split_kunten_text(
    node: &Value,
    source: &str,
    policy: &BlockKuntenPolicy,
    ranges: &[Range<usize>],
    targets: &mut Vec<Value>,
) -> Option<Vec<Value>> {
    let text = node["value"].as_str()?;
    let source_start = usize::try_from(node["span"]["byte_start"].as_u64()?).ok()?;
    let source_end = usize::try_from(node["span"]["byte_end"].as_u64()?).ok()?;
    let source_boundaries = source_boundaries(text, source.get(source_start..source_end)?)?;
    let excluded = excluded_offsets(text, &policy.exclusions);
    let mut marks = text.char_indices().filter_map(|(offset, character)| {
        let absolute = source_start + source_boundaries[offset];
        (policy.marks.contains(&character)
            && !excluded.contains(&offset)
            && (ranges.is_empty() || ranges.iter().any(|range| range.contains(&absolute))))
        .then_some((offset, offset + character.len_utf8(), character))
    });
    let first = marks.next()?;
    let mut parts = Vec::new();
    let mut cursor = 0;
    for (start, end, character) in iter::once(first).chain(marks) {
        if cursor < start {
            parts.push(text_node_slice(
                node,
                text,
                cursor,
                start,
                source_start,
                &source_boundaries,
            ));
        }
        let line = node["span"]["line_start"].as_u64()?
            + u64::try_from(text[..start].matches('\n').count()).ok()?;
        let span = json!({
            "byte_start":source_start + source_boundaries[start],
            "byte_end":source_start + source_boundaries[end],
            "line_start":line,
            "line_end":line,
        });
        targets.push(span.clone());
        parts.push(json!({
            "kind":"kunten",
            "kunten_kind":"return-mark",
            "text":character.to_string(),
            "span":span,
            "interpretation_marker_spans":[policy.marker_span.clone()],
        }));
        cursor = end;
    }
    if cursor < text.len() {
        parts.push(text_node_slice(
            node,
            text,
            cursor,
            text.len(),
            source_start,
            &source_boundaries,
        ));
    }
    Some(parts)
}

pub(super) fn realize_adjacent_kunten(blocks: &mut [Value], source: &str) {
    for block in blocks {
        realize_adjacent_kunten_value(block, source);
    }
}

fn realize_adjacent_kunten_value(value: &mut Value, source: &str) {
    match value {
        Value::Array(values) => {
            for value in values {
                realize_adjacent_kunten_value(value, source);
            }
        }
        Value::Object(object) => {
            if object.get("kind").and_then(Value::as_str) == Some("paragraph")
                && let Some(content) = object.get_mut("content").and_then(Value::as_array_mut)
            {
                realize_adjacent_kunten_in_content(content, source);
            }
            if let Some(children) = object.get_mut("children") {
                realize_adjacent_kunten_value(children, source);
            }
        }
        _ => {}
    }
}

fn realize_adjacent_kunten_in_content(content: &mut Vec<Value>, source: &str) {
    let policies = content
        .iter()
        .filter(|node| node["kind"] == "formatting_instruction")
        .filter_map(|instruction| {
            let marker_start = usize::try_from(instruction["span"]["byte_start"].as_u64()?).ok()?;
            let marker_end = usize::try_from(instruction["span"]["byte_end"].as_u64()?).ok()?;
            Some(
                instruction["operations"]
                    .as_array()?
                    .iter()
                    .enumerate()
                    .filter_map(move |(operation_index, operation)| {
                        let effect = &operation["effect"];
                        let scope = effect["scope"].as_str()?;
                        let marks = effect["marks"]["text"]
                            .as_str()?
                            .chars()
                            .collect::<BTreeSet<_>>();
                        let range = match scope {
                            "quoted-region" => {
                                let previous = previous_quote(source, marker_start);
                                if previous
                                    .as_ref()
                                    .is_some_and(|range| range_contains_mark(source, range, &marks))
                                {
                                    previous?
                                } else {
                                    next_quote(source, marker_end)?
                                }
                            }
                            "following-text" if !effect["repeat_following"].as_bool()? => {
                                next_quote(source, marker_end)?
                            }
                            _ => return None,
                        };
                        Some((
                            instruction["span"].clone(),
                            operation_index,
                            range,
                            BlockKuntenPolicy {
                                instruction_index: 0,
                                operation_index,
                                marks,
                                exclusions: effect["exclusions"]
                                    .as_array()?
                                    .iter()
                                    .filter_map(|exclusion| {
                                        Some((
                                            exclusion["enclosing"]["text"].as_str()?.to_owned(),
                                            exclusion["mark"]["text"].as_str()?.to_owned(),
                                        ))
                                    })
                                    .collect(),
                                marker_span: instruction["span"].clone(),
                            },
                        ))
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .flatten()
        .collect::<Vec<_>>();

    for (marker_span, operation_index, range, policy) in policies {
        let mut targets = Vec::new();
        realize_kunten_in_content(
            content,
            source,
            &policy,
            slice::from_ref(&range),
            &mut targets,
        );
        if targets.is_empty() {
            continue;
        }
        let Some(instruction) = content
            .iter_mut()
            .find(|node| node["kind"] == "formatting_instruction" && node["span"] == marker_span)
        else {
            continue;
        };
        instruction["operations"][operation_index]["selection"] = json!({
            "scope":decoded_span(source, &range),
            "targets":targets,
        });
    }
}

fn range_contains_mark(source: &str, range: &Range<usize>, marks: &BTreeSet<char>) -> bool {
    source
        .get(range.clone())
        .is_some_and(|text| text.chars().any(|character| marks.contains(&character)))
}

fn previous_quote(source: &str, marker_start: usize) -> Option<Range<usize>> {
    let before = source.get(..marker_start)?;
    let close = before.rfind('」')?;
    if !before.get(close + '」'.len_utf8()..)?.trim().is_empty() {
        return None;
    }
    let open = before.get(..close)?.rfind('「')?;
    Some(open + '「'.len_utf8()..close)
}

fn next_quote(source: &str, marker_end: usize) -> Option<Range<usize>> {
    let after = source.get(marker_end..)?;
    let relative_open = after.find('「')?;
    if !after.get(..relative_open)?.trim().is_empty() {
        return None;
    }
    let open = marker_end + relative_open;
    let relative_close = source.get(open + '「'.len_utf8()..)?.find('」')?;
    let close = open + '「'.len_utf8() + relative_close;
    Some(open + '「'.len_utf8()..close)
}

fn decoded_span(source: &str, range: &Range<usize>) -> Value {
    json!({
        "byte_start":range.start,
        "byte_end":range.end,
        "line_start":source_line(source, range.start),
        "line_end":source_line(source, range.end.saturating_sub(1)),
    })
}

fn source_line(source: &str, end: usize) -> usize {
    let bytes = source.get(..end).unwrap_or(source).as_bytes();
    let mut lines = 1;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' => {
                lines += 1;
                index += 1 + usize::from(bytes.get(index + 1) == Some(&b'\n'));
            }
            b'\n' => {
                lines += 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    lines
}

fn excluded_offsets(text: &str, exclusions: &[(String, String)]) -> BTreeSet<usize> {
    let mut excluded = BTreeSet::new();
    for (enclosing, mark) in exclusions {
        for (enclosing_start, _) in text.match_indices(enclosing) {
            for (mark_start, _) in enclosing.match_indices(mark) {
                excluded.extend(
                    mark.char_indices()
                        .map(|(offset, _)| enclosing_start + mark_start + offset),
                );
            }
        }
    }
    excluded
}

fn text_node_slice(
    original: &Value,
    text: &str,
    start: usize,
    end: usize,
    source_start: usize,
    source_boundaries: &[usize],
) -> Value {
    let base_line = original["span"]["line_start"].as_u64().unwrap_or(1);
    let line_start = base_line + text[..start].matches('\n').count() as u64;
    let line_end = base_line + text[..end].matches('\n').count() as u64
        - u64::from(text[..end].ends_with('\n'));
    json!({
        "kind":"text",
        "value":&text[start..end],
        "span":{
            "byte_start":source_start + source_boundaries[start],
            "byte_end":source_start + source_boundaries[end],
            "line_start":line_start,
            "line_end":line_end,
        }
    })
}

fn source_boundaries(text: &str, source: &str) -> Option<Vec<usize>> {
    let mut boundaries = vec![usize::MAX; text.len() + 1];
    let mut source_offset = 0;
    boundaries[0] = 0;
    for (text_offset, character) in text.char_indices() {
        let source_tail = source.get(source_offset..)?;
        let consumed = if character == '\n' && source_tail.starts_with("\r\n") {
            2
        } else if character == '\n' && source_tail.starts_with('\r') {
            1
        } else if source_tail.starts_with(character) {
            character.len_utf8()
        } else {
            return None;
        };
        boundaries[text_offset] = source_offset;
        source_offset += consumed;
        boundaries[text_offset + character.len_utf8()] = source_offset;
    }
    matches!(source.get(source_offset..)?, "" | "\n" | "\r" | "\r\n").then_some(boundaries)
}

fn block_child_scope(children: &Value) -> Option<Value> {
    let spans = children.as_array()?.iter().filter_map(|child| {
        let span = child.get("span")?;
        Some((
            span["byte_start"].as_u64()?,
            span["byte_end"].as_u64()?,
            span["line_start"].as_u64()?,
            span["line_end"].as_u64()?,
        ))
    });
    let mut byte_start = u64::MAX;
    let mut byte_end = 0;
    let mut line_start = u64::MAX;
    let mut line_end = 0;
    for (start, end, first_line, last_line) in spans {
        byte_start = byte_start.min(start);
        byte_end = byte_end.max(end);
        line_start = line_start.min(first_line);
        line_end = line_end.max(last_line);
    }
    (byte_start != u64::MAX).then(|| {
        json!({
            "byte_start":byte_start,
            "byte_end":byte_end,
            "line_start":line_start,
            "line_end":line_end,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::{excluded_offsets, source_boundaries};

    #[test]
    fn exclusions_exempt_only_quoted_marks_inside_quoted_phrases() {
        let exclusions = vec![
            ("一日".to_owned(), "一".to_owned()),
            ("上下".to_owned(), "上下".to_owned()),
        ];

        assert_eq!(
            excluded_offsets("一日一レ上下", &exclusions),
            [0, 12, 15].into()
        );
    }

    #[test]
    fn source_boundaries_account_for_canonicalized_line_endings() {
        let boundaries = source_boundaries("甲\n乙", "甲\r\n乙").unwrap();

        assert_eq!(boundaries["甲".len()], "甲".len());
        assert_eq!(boundaries["甲\n".len()], "甲\r\n".len());
        assert_eq!(boundaries["甲\n乙".len()], "甲\r\n乙".len());
        assert!(source_boundaries("甲", "甲\r\n").is_some());
    }
}
