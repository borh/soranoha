//! Documentary notation examples remain distinct from active commands.

use ab_aozora_pipeline::lex;
use ab_aozora_render::serialize;
use ab_aozora_syntax::{
    DirectiveKind,
    ast::{Node, NodeRef},
};

#[test]
fn quoted_indentation_word_is_documentary_not_an_active_layout_command() {
    let source = "［＃「３字下げ」］";
    let parsed = lex(source);
    assert!(parsed.source_nodes.iter().any(|node| matches!(
        node.node,
        NodeRef::Inline(Node::Directive(directive))
            if directive.kind == DirectiveKind::NotationExample
    )));
    assert_eq!(serialize(&parsed), source);
}
