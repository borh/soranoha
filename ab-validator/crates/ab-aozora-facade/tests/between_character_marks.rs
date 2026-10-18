//! A mark placed at the junction of two runs attaches to the junction, not to
//! either run.
use ab_aozora_facade::syntax::{MarginNoteKind, MarkAnchor};
use ab_aozora_facade::{Content, Document, Node, NodeRef};

/// Returns each junction mark as `(left run, right run, mark)`.
///
/// The two runs are returned separately because a single joined string would
/// also match when the parser attaches the mark to the joined pair as one
/// target.
fn between_marks(source: &str) -> Vec<(String, String, String)> {
    let document = Document::new(source);
    let tree = document.parse();
    let store = &tree.lex_output().store;
    let found = tree
        .source_nodes()
        .iter()
        .filter_map(|node| {
            let NodeRef::Inline(Node::MarginNote(note)) = node.node else {
                return None;
            };
            let MarkAnchor::Between = note.anchor? else {
                return None;
            };
            assert_eq!(note.kind, MarginNoteKind::SuppliedMark, "{source}");
            let [Content::Plain(left), Content::Plain(right)] =
                store.resolve_content_range(note.base)
            else {
                return None;
            };
            Some((
                store.resolve_str(*left).to_owned(),
                store.resolve_str(*right).to_owned(),
                store.content_range_as_plain(note.note)?.to_owned(),
            ))
        })
        .collect();
    // Serialization reproduces the pair form of the marker, not a note on the
    // joined run.
    assert_eq!(tree.to_source(), source, "{source}");
    found
}

#[test]
fn a_junction_mark_keeps_both_runs_it_sits_between() {
    assert_eq!(
        between_marks("自然［＃「自」と「然」の間に白三角傍点］"),
        [("自".to_owned(), "然".to_owned(), "白三角傍点".to_owned())]
    );
}

#[test]
fn consecutive_junction_marks_each_take_the_run_before_them() {
    // The corpus line this case reproduces has three junction marks in a row;
    // each mark applies only to the pair it quotes.
    assert_eq!(
        between_marks(
            "自然［＃「自」と「然」の間に白三角傍点］はう［＃「は」と「う」の間に白三角傍点］たふ［＃「た」と「ふ」の間に白三角傍点］"
        ),
        [
            ("自".to_owned(), "然".to_owned(), "白三角傍点".to_owned()),
            ("は".to_owned(), "う".to_owned(), "白三角傍点".to_owned()),
            ("た".to_owned(), "ふ".to_owned(), "白三角傍点".to_owned()),
        ]
    );
}

#[test]
fn runs_that_do_not_meet_locate_no_junction() {
    for source in [
        // Another character separates the quoted runs, so the source has two
        // gaps and no single junction.
        "自X然［＃「自」と「然」の間に白三角傍点］",
        // The quoted pair is not the text immediately before the marker.
        "自然他［＃「自」と「然」の間に白三角傍点］",
        // The runs are quoted in the reverse of their text order.
        "然自［＃「自」と「然」の間に白三角傍点］",
        // Three runs have two junctions, and the marker identifies neither.
        "自然は［＃「自」と「然」と「は」の間に白三角傍点］",
        // The mark is outside the supplied-mark vocabulary.
        "自然［＃「自」と「然」の間に何か］",
    ] {
        assert!(between_marks(source).is_empty(), "{source}");
    }
}
