//! Corpus font-size aliases retain the same semantic values as canonical forms.

use core::num::NonZeroU8;

use ab_aozora_pipeline::lex;
use ab_aozora_syntax::{
    AbsoluteSize, FontShift, ForwardAttr, QualitativeFontSize, RegionClose, RegionFormat,
    ast::{Node, NodeRef},
};

#[test]
fn compound_small_indent_range_keeps_both_indents_and_typography() {
    let source =
        "［＃ここから小文字、２字下げ。冒頭のみ１字下げ］\n本文\n［＃ここで小文字、字下げ終わり］";
    let tree = lex(source);
    assert!(tree.source_nodes.iter().any(|node| {
        matches!(
            node.node,
            NodeRef::BlockOpen(RegionFormat::Indent(block))
                if block.amount == 1
                    && block.wrap == Some(2)
                    && block.styles.font == Some(QualitativeFontSize::Smaller)
        )
    }));
    assert!(tree.source_nodes.iter().any(|node| {
        matches!(
            node.node,
            NodeRef::BlockClose(RegionClose::Indent { styles, .. })
                if styles.font == Some(QualitativeFontSize::Smaller)
        )
    }));
}

#[test]
fn mawari_relative_size_opens_a_typed_range() {
    let source = "［＃１回り大きな文字］本文［＃大きな文字終わり］";
    let tree = lex(source);
    assert!(tree.source_nodes.iter().any(|node| {
        matches!(
            node.node,
            NodeRef::BlockOpen(RegionFormat::FontSize(FontShift(shift))) if shift.get() == 1
        )
    }));
}

#[test]
fn no_particle_small_text_selects_absolute_small_size() {
    let source = "ヰ［＃「ヰ」の小文字］";
    let tree = lex(source);
    assert!(tree.source_nodes.iter().any(|node| {
        matches!(
            node.node,
            NodeRef::Inline(Node::Format(format)) if format.attrs.single()
                == Some(ForwardAttr::FontSizeAbsolute(AbsoluteSize::Small))
        )
    }));
}

#[test]
fn qualitative_small_size_spellings_keep_the_supplied_degree() {
    for (effect, expected) in [
        ("小さい文字", QualitativeFontSize::Smaller),
        ("本文より小さいサイズの文字", QualitativeFontSize::Smaller),
        ("ポイントを下げる", QualitativeFontSize::Smaller),
        ("小さめの文字", QualitativeFontSize::SlightlySmaller),
    ] {
        let source = format!("語［＃「語」は{effect}］");
        let tree = lex(&source);
        assert!(
            tree.source_nodes.iter().any(|node| matches!(
                node.node,
                NodeRef::Inline(Node::Format(format))
                    if format.attrs.single()
                        == Some(ForwardAttr::QualitativeFontSize(expected))
            )),
            "{effect}: {:?}",
            tree.source_nodes
        );
    }
}

#[test]
fn raised_baseline_spellings_do_not_invent_small_type() {
    for (effect, degrees) in [
        ("上部に出ている", None),
        ("３０度位右上がり", NonZeroU8::new(30)),
    ] {
        let source = format!("語［＃「語」は{effect}］");
        let tree = lex(&source);
        assert!(
            tree.source_nodes.iter().any(|node| matches!(
                node.node,
                NodeRef::Inline(Node::Format(format))
                    if format.attrs.single() == Some(ForwardAttr::Raised { degrees })
            )),
            "{effect}: {:?}",
            tree.source_nodes
        );
    }
}
