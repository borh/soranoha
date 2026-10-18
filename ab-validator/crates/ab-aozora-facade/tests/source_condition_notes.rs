//! Source-condition annotations preserve exact source while carrying typed nodes.

use ab_aozora_facade::{DirectiveKind, Document, ForwardAttr, Node, NodeRef};

#[test]
fn final_standalone_forms_round_trip_with_semantic_kinds() {
    for (source, serialized, expected) in [
        (
            "霰《みぞれ》［＃「霙」の誤り？、400-7］の音",
            "｜霰《みぞれ》［＃「霙」の誤り？、400-7］の音",
            DirectiveKind::UncertainCorrection,
        ),
        (
            "あたしまつぴらだわ［＃印刷不鮮明、87-14］。",
            "あたしまつぴらだわ［＃印刷不鮮明、87-14］。",
            DirectiveKind::SourceLegibilityNote,
        ),
    ] {
        let document = Document::new(source);
        let tree = document.parse();
        assert_eq!(tree.to_source(), serialized);
        assert!(tree.source_nodes().iter().any(|node| {
            matches!(node.node, NodeRef::Inline(Node::Directive(directive)) if directive.kind == expected)
        }));
    }
}

#[test]
fn detached_dot_note_round_trips_and_composes_the_target() {
    let source = "鳥のアラビア語 tair. ［＃「t」は下点付き、182-6］咽喉";
    let document = Document::new(source);
    let tree = document.parse();
    assert_eq!(tree.to_source(), source);
    assert!(tree.source_nodes().iter().any(|node| {
        matches!(
            node.node,
            NodeRef::Inline(Node::Format(format))
                if format.attrs.single() == Some(ForwardAttr::AccentDot)
        )
    }));
    assert!(tree.to_html().contains("ṭair"));
}
