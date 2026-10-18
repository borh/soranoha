//! Private-use characters in the source parse as literal text, not as the
//! parser's internal markers.

use ab_aozora_facade::Document;

#[test]
fn literal_private_use_characters_round_trip_beside_markup() {
    let literal = "前\u{e001}\u{e002}\u{e003}\u{e004}後";
    let document = Document::new(literal);
    assert_eq!(document.parse().to_source(), literal);
    for codepoint in ['\u{e001}', '\u{e002}', '\u{e003}', '\u{e004}'] {
        let source = format!("{codepoint}漢《かん》{codepoint}");
        let document = Document::new(source.clone());
        let tree = document.parse();
        assert_eq!(tree.source(), source);
        assert_eq!(
            tree.to_source(),
            format!("{codepoint}｜漢《かん》{codepoint}")
        );
        assert_eq!(tree.source_nodes().len(), 1);
    }
}
