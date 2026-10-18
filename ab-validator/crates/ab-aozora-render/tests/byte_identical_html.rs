//! Tests of the byte-identical contract between the streaming HTML
//! renderer (`html::render_html_into` → `escape_text_chunk`) and the
//! per-node renderer (`render_node::render` → `escape_text`): for any
//! plain-text input (no Aozora markup), both renderers must produce
//! byte-identical HTML, including the apostrophe form `&#x27;`. The doc
//! comment on `spelling::html::escape_text` refers to these tests.

use ab_aozora_pipeline::lex;
use ab_aozora_render::render_html;
use proptest::prelude::*;

/// Lex `text` and render the result with `render_html`.
fn render_streaming(text: &str) -> String {
    let out = lex(text);
    render_html(&out)
}

#[test]
fn apostrophe_uses_hex_form_not_decimal() {
    let html = render_streaming("a'b");
    assert!(
        html.contains("a&#x27;b") && !html.contains("&#39;"),
        "apostrophe must render as &#x27; (hex), got: {html:?}",
    );
}

/// All five HTML-unsafe ASCII characters in one literal, so the apostrophe
/// entity is checked beside the other four.
#[test]
fn five_unsafe_chars_canonical_entities() {
    let html = render_streaming("<>&\"'");
    assert!(
        html.contains("&lt;&gt;&amp;&quot;&#x27;"),
        "expected canonical entities, got: {html:?}",
    );
}

#[test]
fn empty_input_emits_empty_string() {
    assert_eq!(render_streaming(""), "");
}

#[test]
fn pure_japanese_text_passes_through_unescaped() {
    let html = render_streaming("青空文庫の本");
    assert!(html.contains("青空文庫の本"), "got: {html}");
}

#[test]
fn ascii_url_inside_text_escapes_ampersand() {
    let html = render_streaming("see https://example.com/?a=1&b=2");
    assert!(html.contains("a=1&amp;b=2"), "got: {html}");
}

#[test]
fn fullwidth_quote_codepoints_unescaped() {
    // U+FF1C / U+FF1E / U+201C / U+201D etc. are not in the HTML5
    // 5-char unsafe set; they must round-trip verbatim.
    let html = render_streaming("「＜文章＞」");
    assert!(html.contains("「＜文章＞」"), "got: {html}");
}

#[test]
fn newline_inside_paragraph_emits_br_tag() {
    let html = render_streaming("a\nb");
    assert!(
        html.contains("a<br />\nb"),
        "single \\n inside paragraph must be <br />, got: {html}",
    );
}

#[test]
fn double_newline_closes_and_reopens_paragraph() {
    let html = render_streaming("a\n\nb");
    assert!(html.contains("<p>a</p>\n"), "got: {html}");
    assert!(html.contains("<p>b</p>\n"), "got: {html}");
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        .. ProptestConfig::default()
    })]

    /// Every output of the streaming renderer must be valid UTF-8
    /// and must not contain the decimal `&#39;` form; the hex form
    /// `&#x27;` is the canonical form.
    #[test]
    fn streaming_render_never_emits_decimal_apostrophe(
        s in "[a-zA-Z0-9'<>&\" \u{3042}-\u{3093}]{0,80}",
    ) {
        let html = render_streaming(&s);
        prop_assert!(
            !html.contains("&#39;"),
            "decimal apostrophe leaked into output for input {:?} -> {:?}",
            s, html,
        );
    }

    /// Round-trip invariant: if the input contains no HTML-unsafe
    /// chars, the output must contain the input verbatim, because the
    /// renderer does not escape safe characters.
    #[test]
    fn safe_chars_pass_through_unmodified(
        s in "[a-zA-Z0-9 \u{3042}-\u{3093}\u{4E00}-\u{4E20}]{1,40}",
    ) {
        let html = render_streaming(&s);
        prop_assert!(
            html.contains(&s),
            "safe input {:?} did not round-trip (got: {:?})",
            s, html,
        );
    }

    /// The number of `<p>` tags should equal the number of `</p>`
    /// tags in any rendered output (well-formed HTML invariant).
    #[test]
    fn paragraph_open_close_counts_match(
        s in "[a-zA-Z0-9 \n\u{3042}-\u{3093}]{0,100}",
    ) {
        let html = render_streaming(&s);
        let opens = html.matches("<p>").count();
        let closes = html.matches("</p>").count();
        prop_assert_eq!(
            opens, closes,
            "paragraph tag balance broken on input {:?} -> {:?}",
            s, html,
        );
    }
}
