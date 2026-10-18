//! Physical line declarations remain distinct from writing-mode-relative sides.

use ab_aozora_facade::{DecorationPosition, Document, ForwardAttr, Node, NodeRef};

#[test]
fn physical_line_forms_round_trip_with_distinct_placement() {
    for (source, position, class) in [
        (
            "Khayyam（Umar Khaiyam［＃「Kh」に下線］）",
            DecorationPosition::Below,
            "aozora-bouten-below",
        ),
        (
            "hove［＃「v」は下線（_）付き、181-表組2行目］",
            DecorationPosition::Below,
            "aozora-bouten-below",
        ),
        (
            "P = 0.07693694［＃「P = 0.07693694」は上線（￣）付き］",
            DecorationPosition::Above,
            "aozora-bouten-above",
        ),
    ] {
        let document = Document::new(source);
        let tree = document.parse();
        assert_eq!(tree.to_source(), source);
        assert!(tree.source_nodes().iter().any(|node| match node.node {
            NodeRef::Inline(Node::Format(format)) => matches!(
                format.attrs.single(),
                Some(ForwardAttr::Bouten {
                    position: actual_position,
                    ..
                }) if actual_position == position
            ),
            _ => false,
        }));
        assert!(tree.to_html().contains(class));
    }
}
