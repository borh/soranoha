//! Native classification retains source selectors independently of target resolution.

use std::num::NonZeroU32;

use ab_aozora_pipeline::{
    ClassifiedSourceDisposition, ClassifiedSourceEvidenceClass, ClassifiedSourceRole, ConstructId,
    lex,
};
use ab_aozora_render::serialize;
use ab_aozora_syntax::{
    AbsoluteSize, BoutenKind, BoutenPosition, DecorationPosition, DirectiveKind, ForwardAttr,
    LineFormat, QualitativeFontSize, RegionClose, RegionFormat,
    ast::{
        FormattingEffect, FormattingInstruction, FormattingPlacement, FormattingPredecessor,
        FormattingRuleStyle, FormattingSelector, HeadingTypography, KuntenScope, LexOutput, Node,
        NodeRef,
    },
};

#[test]
fn corpus_end_alignment_aliases_keep_their_line_geometry() {
    for (source, offset) in [
        ("［＃下げて、地より２字あきで］署名", 2),
        ("［＃下げて地より２字あきで］署名", 2),
        ("［＃下げて地付きで］署名", 0),
        ("［＃下げて、地付きで］署名", 0),
        ("日付［＃この日付は行末に記す］", 0),
    ] {
        let parsed = lex(source);
        assert!(
            parsed.source_nodes.iter().any(|node| matches!(
                node.node,
                NodeRef::Inline(Node::Line(LineFormat::AlignEnd { offset: actual, .. }))
                    | NodeRef::BlockLeaf(Node::Line(LineFormat::AlignEnd { offset: actual, .. }))
                    if actual == offset
            )),
            "{source}: {:?}",
            parsed.source_nodes
        );
    }
}

#[test]
fn corpus_compound_end_alignment_forms_keep_independent_meanings() {
    let (parsed, split) =
        instruction("日付［＃この行ここまで相対的に字が小さい、ここからは下揃え、下から３字上げ］");
    assert_eq!(split.operations.len(), 2);
    assert_eq!(
        split.operations[0].selector,
        FormattingSelector::PrecedingText
    );
    assert_eq!(
        split.operations[1].selector,
        FormattingSelector::FollowingText
    );
    assert_eq!(
        split.operations[1].effect,
        FormattingEffect::Attribute(ForwardAttr::AlignEnd { offset: 3 })
    );
    assert_eq!(
        text(
            &parsed,
            split.operations[0].selection.as_ref().unwrap().scope
        ),
        "日付"
    );

    let (parsed, geometry) =
        instruction("著　者［＃「著　者」は天より４０字下げ、地より２字上げ］");
    assert_eq!(geometry.operations.len(), 2);
    assert_eq!(
        geometry.operations[0].effect,
        FormattingEffect::Layout {
            indent: 40,
            horizontal: false,
            align_end: false,
        }
    );
    assert_eq!(
        geometry.operations[1].effect,
        FormattingEffect::Attribute(ForwardAttr::AlignEnd { offset: 2 })
    );
    assert!(
        geometry
            .operations
            .iter()
            .all(|operation| operation.selection.is_some())
    );
    assert_eq!(
        text(
            &parsed,
            geometry.operations[0].selection.as_ref().unwrap().scope
        ),
        "著　者"
    );

    let (parsed, mismatched) = instruction("九月　日　江馬兆策識」［＃『江馬兆策識」』は地付き］");
    let FormattingSelector::Quoted(operand) = mismatched.operations[0].selector else {
        panic!("quoted operand")
    };
    assert_eq!(text(&parsed, operand), "江馬兆策識」");
    assert!(mismatched.operations[0].selection.is_some());
}

#[test]
fn unqualified_larger_type_remains_distinct_from_a_supplied_degree() {
    let parsed = lex("署名［＃この行はポイントを上げ、は地より３字上げ］");
    assert!(
        parsed.source_nodes.iter().any(|node| matches!(
            node.node,
            NodeRef::Inline(Node::Line(LineFormat::AlignEnd {
                offset: 3,
                font: Some(QualitativeFontSize::Larger),
                ..
            })) | NodeRef::BlockLeaf(Node::Line(LineFormat::AlignEnd {
                offset: 3,
                font: Some(QualitativeFontSize::Larger),
                ..
            }))
        )),
        "{:?}",
        parsed.source_nodes
    );
}

#[test]
fn kunten_policies_preserve_scope_exclusions_and_adjuncts() {
    for (source, scope, marks, exclusions, repeated, ruled) in [
        (
            "「撃レ非如レ鷹」［＃「」内の「レ」は返り点］",
            KuntenScope::QuotedRegion,
            "レ",
            0,
            false,
            false,
        ),
        (
            "［＃以下の「」内の、「レ一二」は返り点］",
            KuntenScope::FollowingText,
            "レ一二",
            0,
            false,
            false,
        ),
        (
            "令［＃返り点の「二」の右横に縦棒あり］",
            KuntenScope::InsertionSite,
            "二",
            0,
            false,
            true,
        ),
        (
            "「一言」の一レ［＃「一言」の「一」をのぞいて「レ一二三」は返り点］",
            KuntenScope::QuotedRegion,
            "レ一二三",
            1,
            false,
            false,
        ),
        (
            "「欲レ食二其肉一」［＃「」内の「レ一二」は返り点、以下同じ］",
            KuntenScope::QuotedRegion,
            "レ一二",
            0,
            true,
            false,
        ),
    ] {
        let (parsed, instruction) = instruction(source);
        let FormattingEffect::KuntenPolicy(policy) = &instruction.operations[0].effect else {
            panic!("kunten policy: {source}");
        };
        assert_eq!(policy.scope, scope);
        assert_eq!(text(&parsed, policy.marks), marks);
        assert_eq!(policy.exclusions.len(), exclusions);
        assert_eq!(policy.repeat_following, repeated);
        assert_eq!(policy.vertical_rule_right, ruled);
    }
}

#[test]
fn compound_indent_keeps_return_mark_policy_separate_from_layout_geometry() {
    let source = "［＃ここから２字下げ、「レ」は返り点］\n漢レ文\n［＃ここで字下げ終わり］";
    let parsed = lex(source);
    let block = parsed
        .source_nodes
        .iter()
        .find_map(|node| match node.node {
            NodeRef::BlockOpen(RegionFormat::Indent(block)) => Some(block),
            _ => None,
        })
        .expect("indent block opener");
    assert_eq!(block.amount, 2);
    let partial = parsed
        .store
        .resolve_partial_layout(block.partial.expect("compound layout metadata"));
    assert!(partial.clauses.is_empty());
    let [instruction] = partial.instructions.as_slice() else {
        panic!("one typed source instruction")
    };
    let FormattingEffect::KuntenPolicy(policy) = &instruction.operations[0].effect else {
        panic!("kunten policy")
    };
    assert_eq!(policy.scope, KuntenScope::FollowingBlock);
    assert_eq!(text(&parsed, policy.marks), "レ");
    assert_eq!(serialize(&parsed), source);
}

#[test]
fn compound_indent_close_keeps_policy_on_the_preceding_block() {
    let source = "［＃ここから2字下げ］\n漢レ文\n［＃ここで字下げ終わり、「レ」は返り点］";
    let parsed = lex(source);
    let close = parsed
        .source_nodes
        .iter()
        .find_map(|node| match node.node {
            NodeRef::BlockClose(RegionClose::Indent { partial, .. }) => partial,
            _ => None,
        })
        .expect("compound indent closer metadata");
    let partial = parsed.store.resolve_partial_layout(close);
    assert!(partial.clauses.is_empty());
    let [instruction] = partial.instructions.as_slice() else {
        panic!("one typed source instruction")
    };
    let FormattingEffect::KuntenPolicy(policy) = &instruction.operations[0].effect else {
        panic!("kunten policy")
    };
    assert_eq!(policy.scope, KuntenScope::PrecedingBlock);
    assert_eq!(text(&parsed, policy.marks), "レ");
    assert_eq!(serialize(&parsed), source);
}

#[test]
fn compound_indent_keeps_comma_separated_return_mark_exclusion_together() {
    let source = "［＃ここから２字下げ、「此一部」の「一」をのぞき、「レ一二」は返り点］\n漢レ文\n［＃ここで字下げ終わり］";
    let parsed = lex(source);
    let block = parsed
        .source_nodes
        .iter()
        .find_map(|node| match node.node {
            NodeRef::BlockOpen(RegionFormat::Indent(block)) => Some(block),
            _ => None,
        })
        .expect("indent block opener");
    let partial = parsed
        .store
        .resolve_partial_layout(block.partial.expect("compound layout metadata"));
    assert!(partial.clauses.is_empty());
    let [instruction] = partial.instructions.as_slice() else {
        panic!("one typed source instruction")
    };
    let FormattingEffect::KuntenPolicy(policy) = &instruction.operations[0].effect else {
        panic!("kunten policy")
    };
    assert_eq!(policy.scope, KuntenScope::FollowingBlock);
    assert_eq!(text(&parsed, policy.marks), "レ一二");
    assert_eq!(policy.exclusions.len(), 1);
    assert_eq!(text(&parsed, policy.exclusions[0].enclosing), "此一部");
    assert_eq!(text(&parsed, policy.exclusions[0].mark), "一");
    assert_eq!(serialize(&parsed), source);
}

#[test]
fn compact_small_and_right_aligned_instruction_keeps_both_effects() {
    let (parsed, instruction) = instruction("江［＃「江」はポイント小さく右寄せ］");
    assert_eq!(instruction.operations.len(), 2);
    for operation in &instruction.operations {
        let FormattingSelector::Quoted(operand) = operation.selector else {
            panic!("quoted selector");
        };
        assert_eq!(text(&parsed, operand), "江");
        let selection = operation.selection.as_ref().expect("resolved target");
        assert_eq!(text(&parsed, selection.scope), "江");
        assert_eq!(selection.targets, [selection.scope]);
    }
    assert_eq!(
        instruction.operations[0].effect,
        FormattingEffect::Attribute(ForwardAttr::QualitativeFontSize(
            QualitativeFontSize::Smaller
        ))
    );
    assert_eq!(
        instruction.operations[1].effect,
        FormattingEffect::Attribute(ForwardAttr::AlignEnd { offset: 0 })
    );
}

#[test]
fn line_small_with_foot_offset_keeps_line_and_quoted_targets() {
    let source = "昔の武蔵野今は東京府下［＃この行はポイントを下げ、「昔の武蔵野今は東京府下」は地より１１字上げ］";
    let (parsed, instruction) = instruction(source);
    let [line, foot] = instruction.operations.as_slice() else {
        panic!("line size and foot offset operations");
    };
    assert_eq!(line.selector, FormattingSelector::PrecedingText);
    assert_eq!(
        line.effect,
        FormattingEffect::Attribute(ForwardAttr::QualitativeFontSize(
            QualitativeFontSize::Smaller
        ))
    );
    assert_eq!(
        text(&parsed, line.selection.as_ref().expect("line target").scope),
        "昔の武蔵野今は東京府下"
    );
    let FormattingSelector::Quoted(operand) = foot.selector else {
        panic!("quoted foot target");
    };
    assert_eq!(text(&parsed, operand), "昔の武蔵野今は東京府下");
    assert_eq!(
        foot.effect,
        FormattingEffect::Attribute(ForwardAttr::AlignEnd { offset: 11 })
    );
    assert!(foot.selection.is_some());
}

#[test]
fn missing_line_foot_operand_is_explicitly_unresolved() {
    let (parsed, instruction) = instruction("本文［＃この行はポイントを下げ、は地より７字上げ］");
    let [line, foot] = instruction.operations.as_slice() else {
        panic!("line size and unresolved foot offset operations");
    };
    assert!(line.selection.is_some());
    assert_eq!(foot.selector, FormattingSelector::Unresolved);
    assert_eq!(
        foot.effect,
        FormattingEffect::Attribute(ForwardAttr::AlignEnd { offset: 7 })
    );
    assert!(foot.selection.is_none());
    assert_eq!(
        serialize(&parsed),
        "本文［＃この行はポイントを下げ、は地より７字上げ］"
    );
}

fn instruction(source: &str) -> (LexOutput, FormattingInstruction) {
    let parsed = lex(source);
    let mut instructions = parsed
        .source_nodes
        .iter()
        .filter_map(|node| match node.node {
            NodeRef::Inline(Node::Directive(directive))
            | NodeRef::BlockLeaf(Node::Directive(directive)) => match directive.kind {
                DirectiveKind::FormattingInstruction(id) => {
                    assert_eq!(
                        parsed.store.resolve_str(directive.raw),
                        &source[source.rfind("［＃").unwrap()..],
                        "instruction retains the complete original marker"
                    );
                    Some(parsed.store.resolve_formatting_instruction(id).clone())
                }
                _ => None,
            },
            _ => None,
        });
    let instruction = instructions.next().unwrap_or_else(|| {
        panic!(
            "typed formatting instruction: {source}; nodes: {:?}",
            parsed.source_nodes
        )
    });
    assert!(
        instructions.next().is_none(),
        "one marker owns one instruction"
    );
    assert_eq!(serialize(&parsed), source, "exact instruction round trip");
    let marker_start = u32::try_from(source.rfind("［＃").unwrap()).unwrap();
    let facts: Vec<_> = parsed
        .classified_source_facts
        .iter()
        .filter(|fact| fact.source_span.start == marker_start)
        .collect();
    assert_eq!(facts.len(), 1, "{source}");
    let fact = facts[0];
    assert_eq!(fact.source_span.end as usize, source.len(), "{source}");
    assert_eq!(
        (
            fact.construct_id,
            fact.source_role,
            fact.disposition,
            fact.evidence_class
        ),
        (
            ConstructId::Emphasis,
            ClassifiedSourceRole::Typography,
            ClassifiedSourceDisposition::EmittedSemanticValue,
            ClassifiedSourceEvidenceClass::TypedNode
        ),
        "{source}"
    );
    for operation in &instruction.operations {
        assert!(
            operation.source_span.start >= marker_start,
            "operation lies in the marker"
        );
        assert!(
            operation.source_span.start < operation.source_span.end,
            "operation has source bytes"
        );
    }
    (parsed, instruction)
}

fn text(parsed: &LexOutput, span: ab_aozora_syntax::Span) -> &str {
    &parsed.sanitized[span.start as usize..span.end as usize]
}

#[test]
fn following_line_compound_instruction_resolves_each_physical_line() {
    let source = concat!(
        "［＃次３行は、文字はゴシック体、罫線は全て波線］\n",
        "　　　　　　　┌〜〜〜〜〜〜〜〜〜〜┐\n",
        "　　　　　　　│　秘密を孕む怪悲劇　│\n",
        "　　　　　　　└〜〜〜〜〜〜〜〜〜〜┘\n",
        "市内大森区"
    );
    let parsed = lex(source);
    assert_eq!(serialize(&parsed), source);
    let instruction = parsed
        .source_nodes
        .iter()
        .find_map(|node| match node.node {
            NodeRef::Inline(Node::Directive(directive))
            | NodeRef::BlockLeaf(Node::Directive(directive)) => match directive.kind {
                DirectiveKind::FormattingInstruction(id) => {
                    Some(parsed.store.resolve_formatting_instruction(id))
                }
                _ => None,
            },
            _ => None,
        })
        .expect("typed following-line instruction");
    let [typeface, rule] = instruction.operations.as_slice() else {
        panic!("typeface and rule operations")
    };
    let lines = NonZeroU32::new(3).unwrap();
    assert_eq!(
        typeface.selector,
        FormattingSelector::FollowingLines { lines }
    );
    assert_eq!(rule.selector, FormattingSelector::FollowingLines { lines });
    assert_eq!(
        typeface.effect,
        FormattingEffect::Attribute(ForwardAttr::Gothic)
    );
    assert_eq!(
        rule.effect,
        FormattingEffect::RuleStyle(FormattingRuleStyle::Wavy)
    );
    for operation in [typeface, rule] {
        let selection = operation.selection.as_ref().expect("three-line selection");
        assert_eq!(selection.targets.len(), 3);
        assert_eq!(
            selection
                .targets
                .iter()
                .map(|span| text(&parsed, *span))
                .collect::<Vec<_>>(),
            [
                "　　　　　　　┌〜〜〜〜〜〜〜〜〜〜┐",
                "　　　　　　　│　秘密を孕む怪悲劇　│",
                "　　　　　　　└〜〜〜〜〜〜〜〜〜〜┘",
            ]
        );
        assert_eq!(
            text(&parsed, selection.scope),
            concat!(
                "　　　　　　　┌〜〜〜〜〜〜〜〜〜〜┐\n",
                "　　　　　　　│　秘密を孕む怪悲劇　│\n",
                "　　　　　　　└〜〜〜〜〜〜〜〜〜〜┘"
            )
        );
    }
}

#[test]
fn separate_operations_keep_source_selectors_without_requiring_resolved_targets() {
    for preceding in ["B1", "missing"] {
        let source = format!("{preceding}［＃「B1」は縦中横、「1」は下付き小文字］");
        let (parsed, instruction) = instruction(&source);
        assert_eq!(instruction.operations.len(), 2);
        for (operation, expected, attribute) in [
            (
                &instruction.operations[0],
                "B1",
                ForwardAttr::CombineUpright,
            ),
            (&instruction.operations[1], "1", ForwardAttr::SubScript),
        ] {
            let FormattingSelector::Quoted(operand) = operation.selector else {
                panic!("quoted selector");
            };
            assert_eq!(text(&parsed, operand), expected);
            assert!(operand.start >= operation.source_span.start);
            assert!(operand.end <= operation.source_span.end);
            assert_eq!(operation.effect, FormattingEffect::Attribute(attribute));
        }
    }
}

#[test]
fn parenthesized_tcy_can_restore_vertical_writing_to_its_inner_text() {
    let source = "（十一）［＃（十一）は縦中横、「十一」は縦組み］";
    let (parsed, instruction) = instruction(source);
    let [outer, inner] = instruction.operations.as_slice() else {
        panic!("two composition operations");
    };

    let FormattingSelector::Parenthesized(operand) = outer.selector else {
        panic!("parenthesized selector");
    };
    assert_eq!(text(&parsed, operand), "（十一）");
    assert_eq!(
        outer.effect,
        FormattingEffect::Attribute(ForwardAttr::CombineUpright)
    );
    assert_eq!(
        outer.selection.as_ref().unwrap().targets,
        [ab_aozora_syntax::Span::new(0, 12)]
    );

    let FormattingSelector::Quoted(operand) = inner.selector else {
        panic!("quoted inner selector");
    };
    assert_eq!(text(&parsed, operand), "十一");
    assert_eq!(
        inner.effect,
        FormattingEffect::Attribute(ForwardAttr::Vertical)
    );
    assert_eq!(
        inner.selection.as_ref().unwrap().targets,
        [ab_aozora_syntax::Span::new(3, 9)]
    );
    assert_eq!(serialize(&parsed), source);
}

#[test]
fn tcy_can_center_its_target_across_a_supplied_line_count() {
    let source = "20［＃「20」は縦中横、２行の左右中央］";
    let (parsed, instruction) = instruction(source);
    let [tcy, centered] = instruction.operations.as_slice() else {
        panic!("two layout operations");
    };

    assert_eq!(
        tcy.effect,
        FormattingEffect::Attribute(ForwardAttr::CombineUpright)
    );
    assert_eq!(
        centered.effect,
        FormattingEffect::HorizontalCenter {
            lines: NonZeroU32::new(2).unwrap()
        }
    );
    for operation in [tcy, centered] {
        let FormattingSelector::Quoted(operand) = operation.selector else {
            panic!("quoted selector");
        };
        assert_eq!(text(&parsed, operand), "20");
        assert_eq!(
            operation.selection.as_ref().unwrap().targets,
            [ab_aozora_syntax::Span::new(0, 2)]
        );
    }
    assert_eq!(serialize(&parsed), source);
}

#[test]
fn single_explicit_operations_remain_typed_without_inventing_a_scope() {
    let source = "s1 a2［＃アラビア数字はすべて下付き小文字］";
    let (_, instruction) = instruction(source);
    let [operation] = instruction.operations.as_slice() else {
        panic!("one global operation");
    };
    assert_eq!(operation.selector, FormattingSelector::Digits);
    assert_eq!(
        operation.effect,
        FormattingEffect::Attribute(ForwardAttr::SubScript)
    );
    assert!(operation.selection.is_none());

    let source = "e-λt［＃「-λt」は「e」の上付き］";
    let (parsed, instruction) = self::instruction(source);
    let [operation] = instruction.operations.as_slice() else {
        panic!("one relative operation");
    };
    let FormattingSelector::Quoted(operand) = operation.selector else {
        panic!("quoted selector");
    };
    assert_eq!(text(&parsed, operand), "-λt");
    let FormattingEffect::UpperRelativeTo { operand, target } = operation.effect else {
        panic!("relative effect");
    };
    assert_eq!(text(&parsed, operand), "e");
    assert_eq!(target.map(|span| text(&parsed, span)), Some("e"));
}

#[test]
fn locally_small_kana_keeps_its_quoted_selector_and_size() {
    let source = "っ［＃ここのみ拗音が小さい字「っ」になっている］";
    let (parsed, instruction) = instruction(source);
    let [operation] = instruction.operations.as_slice() else {
        panic!("one local-size operation");
    };
    let FormattingSelector::Quoted(operand) = operation.selector else {
        panic!("quoted selector");
    };
    assert_eq!(text(&parsed, operand), "っ");
    assert_eq!(
        operation.effect,
        FormattingEffect::Attribute(ForwardAttr::FontSizeAbsolute(AbsoluteSize::Small))
    );
}

#[test]
fn supplied_all_quantifier_selects_every_quoted_occurrence() {
    let (parsed, instruction) = instruction("5と5［＃「5」はすべて下付き小文字］");
    let [operation] = instruction.operations.as_slice() else {
        panic!("one subscript operation");
    };
    let FormattingSelector::AllQuoted(operand) = operation.selector else {
        panic!("all-quoted selector");
    };
    assert_eq!(text(&parsed, operand), "5");
    assert_eq!(
        operation.effect,
        FormattingEffect::Attribute(ForwardAttr::SubScript)
    );
    assert_eq!(
        operation
            .selection
            .as_ref()
            .expect("resolved repeated targets")
            .targets
            .len(),
        2
    );
}

#[test]
fn heading_typography_selects_preceding_text_without_inventing_outline_structure() {
    for (marker, expected) in [
        ("見出し文字", HeadingTypography::Heading),
        ("小見出し文字", HeadingTypography::SmallHeading),
    ] {
        let source = format!("新聞標題［＃{marker}］");
        let (parsed, instruction) = instruction(&source);
        let [operation] = instruction.operations.as_slice() else {
            panic!("one heading typography operation");
        };
        assert_eq!(operation.selector, FormattingSelector::PrecedingText);
        assert_eq!(
            operation.effect,
            FormattingEffect::HeadingTypography(expected)
        );
        let selection = operation.selection.as_ref().expect("resolved line target");
        assert_eq!(selection.targets, vec![selection.scope]);
        assert_eq!(text(&parsed, selection.scope), "新聞標題");
    }
}

#[test]
fn suffix_compound_effects_select_the_preceding_text() {
    let source = "三万円の懸賞［＃特大文字、二重丸傍点］";
    let (parsed, instruction) = instruction(source);
    assert_eq!(instruction.operations.len(), 2);
    for operation in &instruction.operations {
        assert_eq!(operation.selector, FormattingSelector::PrecedingText);
        let selection = operation.selection.as_ref().expect("resolved selection");
        assert_eq!(text(&parsed, selection.scope), "三万円の懸賞");
        assert_eq!(selection.targets, [selection.scope]);
    }
    assert_eq!(
        instruction.operations[0].effect,
        FormattingEffect::Attribute(ForwardAttr::FontSizeAbsolute(AbsoluteSize::ExtraLarge))
    );
    assert_eq!(
        instruction.operations[1].effect,
        FormattingEffect::Attribute(ForwardAttr::Bouten {
            kind: BoutenKind::DoubleCircle,
            position: DecorationPosition::Right,
        })
    );
}

#[test]
fn repeated_operands_and_all_quantifier_are_retained_without_correcting_the_source() {
    let source =
        "x2＋y2＝r2［＃「x2」、「y2」、「x2」はそれぞれ縦中横、すべての「2」は上付き小書き］";
    let (parsed, instruction) = instruction(source);
    assert_eq!(instruction.operations.len(), 2);
    let FormattingSelector::EachQuoted(operands) = &instruction.operations[0].selector else {
        panic!("individual quoted operands");
    };
    assert_eq!(
        operands
            .iter()
            .map(|span| text(&parsed, *span))
            .collect::<Vec<_>>(),
        ["x2", "y2", "x2"]
    );
    assert!(operands.windows(2).all(|pair| pair[0].end < pair[1].start));
    let FormattingSelector::AllQuoted(operand) = instruction.operations[1].selector else {
        panic!("all quoted occurrences");
    };
    assert_eq!(text(&parsed, operand), "2");
    assert_eq!(
        instruction.operations[0].effect,
        FormattingEffect::Attribute(ForwardAttr::CombineUpright)
    );
    assert_eq!(
        instruction.operations[1].effect,
        FormattingEffect::Attribute(ForwardAttr::SuperScript)
    );
    assert!(instruction.operations[0].selection.is_none());
    assert!(
        !parsed
            .source_nodes
            .iter()
            .any(|node| matches!(node.node, NodeRef::Inline(Node::Gaiji(_))))
    );
}

#[test]
fn adjacent_plural_tcy_binds_supplied_targets_in_source_order() {
    let (parsed, instruction) = instruction("表　３回　宮の下［＃「３回」「宮の下」は縦中横］");
    let operation = &instruction.operations[0];
    let selection = operation.selection.as_ref().unwrap();

    assert_eq!(
        operation.effect,
        FormattingEffect::Attribute(ForwardAttr::CombineUpright)
    );
    assert_eq!(
        selection
            .targets
            .iter()
            .map(|span| text(&parsed, *span))
            .collect::<Vec<_>>(),
        ["３回", "宮の下"]
    );
}

#[test]
fn occurrence_numbers_are_distinct_from_character_positions() {
    let (parsed, instruction) =
        instruction("biνi［＃２文字目の「i」は下付き小文字、４文字目の「i」は上付き小文字］");
    for (operation, ordinal) in instruction.operations.iter().zip([2, 4]) {
        let FormattingSelector::CharacterPosition {
            ordinal: actual,
            operand,
        } = operation.selector
        else {
            panic!("character position");
        };
        assert_eq!(actual, NonZeroU32::new(ordinal).unwrap());
        assert_eq!(text(&parsed, operand), "i");
    }
    assert_eq!(instruction.operations.len(), 2);
    let (parsed, instruction) =
        self::instruction("1/144［＃２つめの「4」は上付き小文字、「1/144」は分数］");
    let FormattingSelector::Occurrence { ordinal, operand } = instruction.operations[0].selector
    else {
        panic!("occurrence number");
    };
    assert_eq!(ordinal.get(), 2);
    assert_eq!(text(&parsed, operand), "4");
}

#[test]
fn inherited_selector_and_relative_base_retain_operation_order() {
    let (parsed, instruction) =
        instruction("現金三万円［＃「現金三万円」は大文字、太字、「三万円」に二重丸傍点］");
    assert_eq!(instruction.operations.len(), 3);
    assert_eq!(
        instruction.operations[0].selector,
        instruction.operations[1].selector
    );
    assert_eq!(
        instruction.operations[0].effect,
        FormattingEffect::Attribute(ForwardAttr::FontSizeAbsolute(AbsoluteSize::Large))
    );
    assert_eq!(
        instruction.operations[1].effect,
        FormattingEffect::Attribute(ForwardAttr::Bold)
    );
    assert_eq!(text(&parsed, instruction.operations[1].source_span), "太字");
    let (parsed, instruction) = self::instruction(
        "e-r2/4Dt［＃「2」は上付き小文字、「r2/4Dt」は分数、「-r2/4Dt」は「e」の上付き］",
    );
    assert_eq!(instruction.operations.len(), 3);
    let FormattingEffect::UpperRelativeTo {
        operand: base,
        target,
    } = instruction.operations[2].effect
    else {
        panic!("explicit upper-position base");
    };
    assert_eq!(text(&parsed, base), "e");
    assert_eq!(text(&parsed, target.unwrap()), "e");
    assert!(target.unwrap().end < base.start);
    assert_eq!(
        instruction.operations[1].effect,
        FormattingEffect::Attribute(ForwardAttr::Fraction)
    );
}

#[test]
fn tcy_below_day_range_resolves_only_a_consecutive_preceding_schedule() {
    let source = concat!(
        "１日　六・四五\n",
        "２日　六・二〇\n",
        "３日　五・四〇\n",
        "４日　六・四〇\n",
        "５日　七・〇〇\n",
        "６日　七・〇〇\n",
        "７日　六・三〇\n",
        "８日　七・〇〇\n",
        "９日　六・〇五\n",
        "10［＃「10」は縦中横］日　七・〇〇\n",
        "注記［＃「注記」は２日〜10日の下に縦中横］",
    );
    let (parsed, instruction) = instruction(source);
    assert_eq!(instruction.operations.len(), 2);
    assert_eq!(
        instruction.operations[0].effect,
        FormattingEffect::Attribute(ForwardAttr::CombineUpright)
    );
    let FormattingEffect::RelativePlacement(FormattingPlacement::BelowDayRange {
        start_operand,
        end_operand,
        start_day,
        end_day,
        start_target,
        end_target,
    }) = instruction.operations[1].effect
    else {
        panic!("day-range placement");
    };
    assert_eq!((start_day.get(), end_day.get()), (2, 10));
    assert_eq!(text(&parsed, start_operand), "２");
    assert_eq!(text(&parsed, end_operand), "10");
    assert_eq!(text(&parsed, start_target.unwrap()), "２");
    assert_eq!(text(&parsed, end_target.unwrap()), "10");
    for operation in &instruction.operations {
        let selection = operation.selection.as_ref().unwrap();
        assert_eq!(text(&parsed, selection.scope), "注記");
        assert_eq!(text(&parsed, selection.targets[0]), "注記");
    }

    let (_, unresolved) =
        self::instruction("２日\n４日\n注記［＃「注記」は２日〜４日の下に縦中横］");
    let FormattingEffect::RelativePlacement(FormattingPlacement::BelowDayRange {
        start_target,
        end_target,
        ..
    }) = unresolved.operations[1].effect
    else {
        panic!("day-range placement");
    };
    assert!(start_target.is_none() && end_target.is_none());
}

#[test]
fn conditional_digits_and_quoted_templates_keep_the_supplied_selection_rule() {
    let (parsed, instruction) = instruction(
        "P = s1a1b1ν + s2a2b2ν2［＃ s、a、b に続くアラビア数字はすべて下付き小文字、νに続くアラビア数字はすべて上付き小文字］",
    );
    assert_eq!(instruction.operations.len(), 2);
    for (operation, expected) in instruction
        .operations
        .iter()
        .zip([vec!["s", "a", "b"], vec!["ν"]])
    {
        let FormattingSelector::ArabicDigitsAfter(FormattingPredecessor::NamedCharacters(operands)) =
            &operation.selector
        else {
            panic!("named predecessor condition");
        };
        assert_eq!(
            operands
                .iter()
                .map(|span| text(&parsed, *span))
                .collect::<Vec<_>>(),
            expected
        );
    }
    let (_, instruction) = self::instruction(
        "a2 = 4/10; a4 = 2/10［＃アルファベットに続くアラビア数字はすべて下付き小文字、「4/10」「2/10」は分数］",
    );
    assert_eq!(
        instruction.operations[0].selector,
        FormattingSelector::ArabicDigitsAfter(FormattingPredecessor::Alphabetic)
    );
    let (parsed, instruction) =
        self::instruction("Ａ２Ａ３Ａ４［＃「Ａｎ」はそれぞれ縦中横、数字は上付き小書き］");
    let FormattingSelector::EachQuoted(operands) = &instruction.operations[0].selector else {
        panic!("supplied template spelling");
    };
    assert_eq!(operands.len(), 1);
    assert_eq!(text(&parsed, operands[0]), "Ａｎ");
    assert_eq!(
        instruction.operations[1].selector,
        FormattingSelector::Digits
    );
}

#[test]
fn existing_formatting_and_genuine_gaiji_keep_their_native_roles() {
    let source = "１）［＃「１）」は縦中横、行右小書き］";
    assert!(
        lex(source)
            .source_nodes
            .iter()
            .any(|node| matches!(node.node, NodeRef::Inline(Node::Format(_))))
    );
    let source = "［＃「土へん＋劣」、第3水準1-15-22］";
    assert!(
        lex(source)
            .source_nodes
            .iter()
            .any(|node| matches!(node.node, NodeRef::Inline(Node::Gaiji(_))))
    );
    for source in [
        "B1［＃「B1」は縦中横、「1」は未知の効果］",
        "B1［＃０文字目の「1」は下付き小文字、「B1」は縦中横］",
        "B1［＃「B1」は縦中横、］",
    ] {
        assert!(!lex(source).source_nodes.iter().any(|node| matches!(node.node, NodeRef::Inline(Node::Directive(directive)) if matches!(directive.kind, DirectiveKind::FormattingInstruction(_)))));
    }
}

#[test]
fn enclosing_quoted_expression_binds_nested_and_ordinal_targets() {
    for (source, scope, targets) in [
        (
            "B1［＃「B1」は縦中横、「1」は下付き小文字］",
            "B1",
            vec!["B1", "1"],
        ),
        (
            "1/144［＃２つめの「4」は上付き小文字、「1/144」は分数］",
            "1/144",
            vec!["4", "1/144"],
        ),
        (
            "現金三万円［＃「現金三万円」は大文字、太字、「三万円」に二重丸傍点］",
            "現金三万円",
            vec!["現金三万円", "現金三万円", "三万円"],
        ),
    ] {
        let (parsed, instruction) = instruction(source);
        for (operation, target) in instruction.operations.iter().zip(targets) {
            let selection = operation.selection.as_ref().expect("established selection");
            assert_eq!(text(&parsed, selection.scope), scope);
            assert_eq!(selection.targets.len(), 1);
            assert_eq!(text(&parsed, selection.targets[0]), target);
            assert!(selection.scope.start <= selection.targets[0].start);
            assert!(selection.targets[0].end <= selection.scope.end);
        }
        if scope == "1/144" {
            assert_eq!(
                instruction.operations[0]
                    .selection
                    .as_ref()
                    .unwrap()
                    .targets[0]
                    .start,
                4
            );
        }
    }
}

#[test]
fn missing_ambiguous_or_unscoped_selectors_do_not_invent_targets() {
    for source in [
        "B2［＃「B1」は縦中横、「1」は下付き小文字］",
        "Ｂ１［＃「B1」は縦中横、「1」は下付き小文字］",
        "B1［＃改行］［＃「B1」は縦中横、「1」は下付き小文字］",
        "x2＋y2［＃「x2」、「y2」はそれぞれ縦中横、すべての「2」は上付き小書き］",
    ] {
        let (_, instruction) = instruction(source);
        assert!(
            instruction
                .operations
                .iter()
                .all(|operation| operation.selection.is_none()),
            "{source}"
        );
    }
    let (_, ambiguous) = instruction("B11［＃「B11」は縦中横、「1」は下付き小文字］");
    assert!(ambiguous.operations[0].selection.is_some());
    assert!(ambiguous.operations[1].selection.is_none());
    let (_, instruction) = instruction(
        "x2＋y2＝r2［＃「x2＋y2＝r2」は横組み、「x2」、「y2」、「x2」はそれぞれ縦中横］",
    );
    assert!(instruction.operations[0].selection.is_some());
    let targets = &instruction.operations[1]
        .selection
        .as_ref()
        .unwrap()
        .targets;
    assert_eq!(targets.len(), 3);
    assert_eq!(targets[0], targets[2]);
    assert_ne!(targets[0], targets[1]);
    let (_, ambiguous) =
        self::instruction("x2＋x2＝r2［＃「x2＋x2＝r2」は横組み、「x2」、「x2」はそれぞれ縦中横］");
    assert!(ambiguous.operations[1].selection.is_none());
}

#[test]
fn ruby_content_owns_its_instruction_target_window() {
    use ab_aozora_syntax::ast::{Content, Segment};
    for source in [
        "｜B1［＃「B1」は縦中横、「1」は下付き小文字］《びー》",
        "｜漢《B1［＃「B1」は縦中横、「1」は下付き小文字］》",
    ] {
        let parsed = lex(source);
        let mut found = 0;
        for node in &parsed.source_nodes {
            let NodeRef::Inline(Node::Ruby(ruby)) = node.node else {
                continue;
            };
            for range in [ruby.base, ruby.reading] {
                for content in parsed.store.resolve_content_range(range) {
                    let Content::Segments(segments) = content else {
                        continue;
                    };
                    for segment in parsed.store.resolve_seg_range(*segments) {
                        let Segment::Directive { value, .. } = segment else {
                            continue;
                        };
                        let DirectiveKind::FormattingInstruction(id) = value.kind else {
                            continue;
                        };
                        let instruction = parsed.store.resolve_formatting_instruction(id);
                        for (operation, expected) in instruction.operations.iter().zip(["B1", "1"])
                        {
                            let selection =
                                operation.selection.as_ref().expect("ruby-local selection");
                            assert_eq!(text(&parsed, selection.scope), "B1");
                            assert_eq!(text(&parsed, selection.targets[0]), expected);
                        }
                        found += 1;
                    }
                }
            }
        }
        assert_eq!(found, 1, "{source}");
        assert_eq!(serialize(&parsed), source);
    }
}

#[test]
fn relative_base_binding_requires_adjacent_owned_body_text() {
    let marker = "［＃「x」は太字、「-x」は「e」の上付き］";
    for (body, resolved) in [
        ("e-x", true),
        ("x e-x", true),
        ("a-x", false),
        ("é-x", false),
        ("e -x", false),
        ("e［＃改行］-x", false),
        ("e-xx", false),
    ] {
        let source = format!("{body}{marker}");
        let (parsed, instruction) = instruction(&source);
        let FormattingEffect::UpperRelativeTo { operand, target } =
            instruction.operations[1].effect
        else {
            panic!("relative placement effect");
        };
        assert_eq!(text(&parsed, operand), "e");
        assert_eq!(target.is_some(), resolved, "{source}");
        if let Some(target) = target {
            assert_eq!(text(&parsed, target), "e");
            assert_eq!(
                target.end,
                instruction.operations[1]
                    .selection
                    .as_ref()
                    .unwrap()
                    .targets[0]
                    .start
            );
        }
    }
}

#[test]
fn character_positions_use_the_adjacent_owned_word_and_unicode_characters() {
    let marker = "［＃２文字目の「i」は下付き小文字、４文字目の「i」は上付き小文字］";
    for prefix in ["", "確率は ", "前\n", "前\t", "前　"] {
        let source = format!("{prefix}biνi{marker}");
        let (parsed, instruction) = instruction(&source);
        for (operation, byte_offset) in instruction.operations.iter().zip([1, 4]) {
            let selection = operation.selection.as_ref().unwrap();
            assert_eq!(text(&parsed, selection.scope), "biνi");
            assert_eq!(text(&parsed, selection.targets[0]), "i");
            assert_eq!(
                selection.targets[0].start - selection.scope.start,
                byte_offset
            );
        }
    }
    for body in [
        "biνi ",
        "biνi\n",
        "(biνi)",
        "bi\u{301}νi",
        "bi［＃改行］νi",
        "前［＃注記］biνi",
        "｜biνi",
    ] {
        let (_, instruction) = instruction(&format!("{body}{marker}"));
        assert!(
            instruction
                .operations
                .iter()
                .all(|operation| operation.selection.is_none()),
            "{body}"
        );
    }
    let (_, instruction) = instruction(&format!("bibj{marker}"));
    assert!(instruction.operations[0].selection.is_some());
    assert!(instruction.operations[1].selection.is_none());
}

#[test]
fn layout_policies_retain_semantic_selectors_without_inventing_body_ranges() {
    for (source, expected_selector, indent, horizontal, align_end) in [
        ("［＃改行ごとに二字下げ］", "each-line", 2, false, false),
        ("［＃以下地付き］", "following-text", 0, false, true),
        ("［＃以下、地付き］", "following-text", 0, false, true),
        ("［＃以下の文章は地付き］", "following-text", 0, false, true),
        (
            "［＃「序にかえて」全体、天より２字下げ］",
            "quoted-whole",
            2,
            false,
            false,
        ),
    ] {
        let (parsed, instruction) = instruction(source);
        assert_eq!(instruction.operations.len(), 1);
        let operation = &instruction.operations[0];
        assert!(operation.selection.is_none());
        let selector = match operation.selector {
            FormattingSelector::EachLine => "each-line",
            FormattingSelector::FollowingText => "following-text",
            FormattingSelector::QuotedWhole(operand) => {
                assert_eq!(text(&parsed, operand), "序にかえて");
                "quoted-whole"
            }
            _ => panic!("layout policy selector"),
        };
        assert_eq!(selector, expected_selector);
        assert_eq!(
            operation.effect,
            FormattingEffect::Layout {
                indent,
                horizontal,
                align_end,
            }
        );
    }
}

#[test]
fn parenthesized_named_token_can_declare_flush_end_alignment() {
    let source = "［＃（Ｆ・Ｏ）は下揃え］";
    let (parsed, instruction) = instruction(source);
    assert_eq!(instruction.operations.len(), 1);
    let operation = &instruction.operations[0];
    let FormattingSelector::Parenthesized(operand) = operation.selector else {
        panic!("parenthesized selector")
    };
    assert_eq!(text(&parsed, operand), "（Ｆ・Ｏ）");
    assert_eq!(
        operation.effect,
        FormattingEffect::Layout {
            indent: 0,
            horizontal: false,
            align_end: true,
        }
    );
    assert!(operation.selection.is_none());
}

#[test]
fn named_line_part_keeps_spacing_beside_end_alignment() {
    let source = "松永哲夫［＃この行は下揃え一字上げ、名前の部分は一字毎に空きあり］";
    let (parsed, instruction) = instruction(source);
    assert_eq!(instruction.operations.len(), 2);
    let alignment = &instruction.operations[0];
    assert_eq!(alignment.selector, FormattingSelector::PrecedingText);
    assert_eq!(
        alignment.effect,
        FormattingEffect::Attribute(ForwardAttr::AlignEnd { offset: 1 })
    );
    assert!(alignment.selection.is_some());

    let spacing = &instruction.operations[1];
    let FormattingSelector::Named(operand) = spacing.selector else {
        panic!("named selector")
    };
    assert_eq!(text(&parsed, operand), "名前の部分");
    assert_eq!(
        spacing.effect,
        FormattingEffect::LetterSpacing {
            amount: NonZeroU32::new(1).unwrap(),
        }
    );
    assert!(spacing.selection.is_none());
}

#[test]
fn indexed_templates_select_instances_and_whole_decimal_indices() {
    for (prefix, template, series, instances, indices) in [
        (
            "類推のできる",
            "Ａｎ",
            "Ａ２Ａ３Ａ４",
            vec!["Ａ２", "Ａ３", "Ａ４"],
            vec!["２", "３", "４"],
        ),
        (
            "もし",
            "Ａｎ",
            "Ａ１Ａ２Ａ３Ａ４",
            vec!["Ａ１", "Ａ２", "Ａ３", "Ａ４"],
            vec!["１", "２", "３", "４"],
        ),
        (
            "",
            "Bn",
            "B1B10B1",
            vec!["B1", "B10", "B1"],
            vec!["1", "10", "1"],
        ),
        (
            "前　",
            "Ａｎ",
            "Ａ９Ａ１０",
            vec!["Ａ９", "Ａ１０"],
            vec!["９", "１０"],
        ),
    ] {
        let source =
            format!("{prefix}{series}［＃「{template}」はそれぞれ縦中横、数字は上付き小書き］");
        let (parsed, instruction) = instruction(&source);
        let FormattingSelector::EachQuoted(operands) = &instruction.operations[0].selector else {
            panic!("supplied template")
        };
        assert_eq!(text(&parsed, operands[0]), template);
        for (operation, expected) in instruction.operations.iter().zip([instances, indices]) {
            let selection = operation.selection.as_ref().unwrap();
            assert_eq!(text(&parsed, selection.scope), series);
            assert_eq!(
                selection
                    .targets
                    .iter()
                    .map(|span| text(&parsed, *span))
                    .collect::<Vec<_>>(),
                expected
            );
            assert!(
                selection
                    .targets
                    .windows(2)
                    .all(|pair| pair[0].end <= pair[1].start)
            );
        }
    }
    let marker = "［＃「Ａｎ」はそれぞれ縦中横、数字は上付き小書き］";
    for body in [
        "Ａ２",
        "ＡｎＡｎ",
        "Ａ２Ｂ３Ａ４",
        "Ｂ１Ａ２Ａ３",
        "Ａ２Ａ３ ",
        "Ａ２Ａ３\n",
        "Ａ１、Ａ２Ａ３",
        "Ａ１ Ａ２Ａ３",
        "Ａ１　Ａ２Ａ３",
        "Ａ１［＃改行］Ａ２Ａ３",
        "Ａ１\u{301}Ａ２Ａ３",
        "｜Ａ２Ａ３",
        "A2A3",
        "Ａ二Ａ三",
    ] {
        let (_, instruction) = instruction(&format!("{body}{marker}"));
        assert!(
            instruction
                .operations
                .iter()
                .all(|operation| operation.selection.is_none()),
            "{body}"
        );
    }
}

#[test]
fn character_rotation_retains_angle_and_exact_target() {
    for (source, degrees) in [
        ("月二［＃「月二」は１８０度回転］", 180),
        ("AB［＃「AB」は90度回転］", 90),
    ] {
        let (parsed, instruction) = instruction(source);
        let [operation] = instruction.operations.as_slice() else {
            panic!("one rotation operation")
        };
        assert_eq!(
            operation.effect,
            FormattingEffect::Rotation {
                degrees: NonZeroU32::new(degrees).unwrap(),
            }
        );
        let FormattingSelector::Quoted(operand) = operation.selector else {
            panic!("quoted rotation selector")
        };
        assert_eq!(
            text(&parsed, operand),
            text(&parsed, operation.selection.as_ref().unwrap().scope)
        );
        let selection = operation.selection.as_ref().unwrap();
        assert_eq!(selection.targets, vec![selection.scope]);
    }
}

#[test]
fn equation_scopes_select_conditional_indices_and_fractions() {
    let marker =
        "［＃アルファベットに続くアラビア数字はすべて下付き小文字、「4/10」「2/10」は分数］";
    for prefix in ["", "であるから ", "前\n", "前　"] {
        let equation = "a10 = b2 = 4/10; c3 = 2/10";
        let (parsed, instruction) = instruction(&format!("{prefix}{equation}{marker}"));
        for (operation, expected) in instruction
            .operations
            .iter()
            .zip([vec!["10", "2", "3"], vec!["4/10", "2/10"]])
        {
            let selection = operation.selection.as_ref().unwrap();
            assert_eq!(text(&parsed, selection.scope), equation);
            assert_eq!(
                selection
                    .targets
                    .iter()
                    .map(|target| text(&parsed, *target))
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }
    let equation = "P = s1a1b1ν + s2a2b2ν2 + s3a3b3ν3 + ……";
    let (parsed, instruction) = instruction(&format!(
        "{equation}［＃ s、a、b に続くアラビア数字はすべて下付き小文字、νに続くアラビア数字はすべて上付き小文字］"
    ));
    for (operation, expected) in instruction.operations.iter().zip([
        vec!["1", "1", "1", "2", "2", "2", "3", "3", "3"],
        vec!["2", "3"],
    ]) {
        let selection = operation.selection.as_ref().unwrap();
        assert_eq!(text(&parsed, selection.scope), equation);
        assert_eq!(
            selection
                .targets
                .iter()
                .map(|target| text(&parsed, *target))
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn equation_scope_requires_complete_owned_syntax() {
    let marker =
        "［＃ s に続くアラビア数字はすべて下付き小文字、νに続くアラビア数字はすべて上付き小文字］";
    for body in [
        "s1 + ν2",
        "therefore s1 = ν2",
        "s1 = ν2 ",
        "s1 = ν2\n",
        "s1 =",
        "s1 = ν2;",
        "s1 == ν2",
        "(s1 = ν2",
        "s1 = ν2)",
        "s1 = () + ν2",
        "s1 = ν2; s3",
        "s1 @ s2 = ν2",
        "s1 ^ s2 = ν2",
        "s1 + \u{03a2} = ν2",
        "s1=［＃改行］s2=ν2",
        "｜s1 = ν2",
    ] {
        let (_, instruction) = instruction(&format!("{body}{marker}"));
        assert!(
            instruction
                .operations
                .iter()
                .all(|op| op.selection.is_none()),
            "{body}"
        );
    }
    for body in [
        "s10 = -ν20",
        "（s１＋s２）＝ν３",
        "s1 = 0.2; s2 = ν3",
        "(s1 = ν2)",
    ] {
        let (_, instruction) = instruction(&format!("{body}{marker}"));
        assert!(
            instruction
                .operations
                .iter()
                .all(|op| op.selection.is_some()),
            "{body}"
        );
    }
}

#[test]
fn equation_universal_selector_does_not_repair_repeated_operands() {
    let (parsed, instruction) = instruction(
        "x2＋y2＝r2［＃「x2」、「y2」、「x2」はそれぞれ縦中横、すべての「2」は上付き小書き］",
    );
    assert!(instruction.operations[0].selection.is_none());
    let selection = instruction.operations[1].selection.as_ref().unwrap();
    assert_eq!(text(&parsed, selection.scope), "x2＋y2＝r2");
    assert_eq!(
        selection
            .targets
            .iter()
            .map(|target| target.start - selection.scope.start)
            .collect::<Vec<_>>(),
        [1, 6, 11]
    );
    assert!(
        selection
            .targets
            .iter()
            .all(|target| text(&parsed, *target) == "2")
    );
}

#[test]
fn reduced_point_lines_retain_anchors_counts_and_enclosure() {
    let cases = [
        (
            "［＃「注記」は「本文」の下にポイントを下げて２行で］",
            false,
            None,
        ),
        (
            "［＃「注記」は、「前」と「後」の間に挟まれるような形でポイントを下げて３行で］",
            true,
            None,
        ),
        (
            "［＃「（注記）」は「本文」の下にポイントを下げて２行で、カッコはその２行を括る形で］",
            false,
            Some(2),
        ),
    ];
    for (source, between, parentheses) in cases {
        let (parsed, instruction) = instruction(source);
        let operation = &instruction.operations[0];
        assert!(operation.selection.is_none());
        let FormattingSelector::Quoted(target) = operation.selector else {
            panic!("quoted arranged text");
        };
        assert!(text(&parsed, target).contains("注記"));
        let FormattingEffect::ReducedPointLines {
            placement,
            lines,
            parentheses_lines,
        } = operation.effect
        else {
            panic!("reduced point line arrangement");
        };
        assert_eq!(lines.get(), if between { 3 } else { 2 });
        assert_eq!(parentheses_lines.map(NonZeroU32::get), parentheses);
        match placement {
            FormattingPlacement::Below(anchor) => {
                assert!(!between);
                assert_eq!(text(&parsed, anchor), "本文");
            }
            FormattingPlacement::Between {
                preceding,
                following,
            } => {
                assert!(between);
                assert_eq!(
                    (text(&parsed, preceding), text(&parsed, following)),
                    ("前", "後")
                );
            }
            FormattingPlacement::BelowDayRange { .. } => {
                panic!("reduced-point placement cannot be a day range");
            }
        }
    }
}

#[test]
fn nested_subtarget_resolves_within_its_adjacent_enclosing_phrase() {
    let (parsed, instruction) = instruction("一ノ戸［＃「一ノ戸」の「ノ」は小書き］");
    let operation = &instruction.operations[0];
    let FormattingSelector::Subtarget { enclosing, operand } = operation.selector else {
        panic!("subtarget selector");
    };
    assert_eq!(
        (text(&parsed, enclosing), text(&parsed, operand)),
        ("一ノ戸", "ノ")
    );
    let selection = operation
        .selection
        .as_ref()
        .expect("adjacent enclosing phrase");
    assert_eq!(text(&parsed, selection.scope), "一ノ戸");
    assert_eq!(text(&parsed, selection.targets[0]), "ノ");
}

#[test]
fn unmatched_outer_quote_retains_the_supplied_operand_without_a_target() {
    let (parsed, instruction) = instruction("［＃「「注―巻五十」は行右小書き］");
    let operation = &instruction.operations[0];
    let FormattingSelector::Quoted(operand) = operation.selector else {
        panic!("quoted selector");
    };
    assert_eq!(text(&parsed, operand), "「注―巻五十");
    assert!(operation.selection.is_none());
    assert_eq!(
        operation.effect,
        FormattingEffect::Attribute(ForwardAttr::SmallScript(BoutenPosition::Right))
    );
}
