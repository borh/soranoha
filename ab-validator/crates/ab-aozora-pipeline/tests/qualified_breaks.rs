//! Compound break directives retain their destination and following-layout semantics.

use core::num::NonZeroU8;

use ab_aozora_pipeline::lex;
use ab_aozora_render::serialize;
use ab_aozora_syntax::{
    DirectiveKind, FollowingPlacement, QualifiedBreak, QualifiedBreakKind,
    ast::{Node, NodeRef},
};

fn parsed_node(source: &str) -> Node {
    lex(source)
        .source_nodes
        .into_iter()
        .find_map(|entry| match entry.node {
            NodeRef::Inline(node) | NodeRef::BlockLeaf(node) => Some(node),
            _ => None,
        })
        .expect("source must produce a node")
}

#[test]
fn corpus_qualified_breaks_are_typed_and_serialize_canonically() {
    for (source, expected, canonical) in [
        (
            "［＃改見開き右］",
            QualifiedBreak {
                kind: QualifiedBreakKind::Kaimihiraki,
                destination_right: true,
                following_placement: None,
                following_indent: None,
                column_number: None,
            },
            "［＃改見開き右］",
        ),
        (
            "［＃改ページ、ページの左右中央］",
            QualifiedBreak {
                kind: QualifiedBreakKind::Page,
                destination_right: false,
                following_placement: Some(FollowingPlacement::HorizontalCenter),
                following_indent: None,
                column_number: None,
            },
            "［＃改ページ、ページの左右中央に］",
        ),
        (
            "［＃改丁、左寄せで］",
            QualifiedBreak {
                kind: QualifiedBreakKind::Kaicho,
                destination_right: false,
                following_placement: Some(FollowingPlacement::Left),
                following_indent: None,
                column_number: None,
            },
            "［＃改丁、左寄せで］",
        ),
        (
            "［＃改ページ、２字下げで］",
            QualifiedBreak {
                kind: QualifiedBreakKind::Page,
                destination_right: false,
                following_placement: None,
                following_indent: Some(2),
                column_number: None,
            },
            "［＃改ページ、2字下げで］",
        ),
        (
            "［＃２段目］",
            QualifiedBreak {
                kind: QualifiedBreakKind::Column,
                destination_right: false,
                following_placement: None,
                following_indent: None,
                column_number: NonZeroU8::new(2),
            },
            "［＃2段目］",
        ),
    ] {
        assert_eq!(
            parsed_node(source),
            Node::QualifiedBreak(expected),
            "{source}"
        );
        assert_eq!(serialize(&lex(source)), canonical, "{source}");
    }
}

#[test]
fn unknown_break_qualifier_remains_an_unknown_directive() {
    let node = parsed_node("［＃改ページ、右寄せで］");
    assert!(matches!(
        node,
        Node::Directive(directive) if directive.kind == DirectiveKind::Unknown
    ));
}
