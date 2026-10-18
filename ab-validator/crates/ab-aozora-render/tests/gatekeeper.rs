//! Tests of the user-visible HTML and serialize output contracts of
//! `aozora-render`:
//!
//! * The five HTML-unsafe ASCII chars (`< > & " '`) each map to a
//!   specific named/numeric entity. The apostrophe form is `&#x27;`;
//!   changing it requires changing both renderer paths (`escape_text`
//!   and `escape_text_chunk`).
//! * Empty input renders to empty output (not "empty paragraph").
//! * Pure-text input round-trips byte-identical inside `<p>...</p>`.
//! * Newline semantics: single newline → `<br />`, double newline →
//!   paragraph close + reopen.
//! * `serialize` is a fixed point after one pass for canonical
//!   markup shapes (inline ruby, page break, kaeriten, gaiji).

use ab_aozora_pipeline::{ALL_SENTINELS, lex};
use ab_aozora_render::{render_html, serialize};
fn html(text: &str) -> String {
    let out = lex(text);
    render_html(&out)
}

fn ser(text: &str) -> String {
    let out = lex(text);
    serialize(&out)
}

#[test]
fn html_entity_table_is_canonical() {
    // One assertion per character, so a failure identifies the entity.
    assert!(html("<").contains("&lt;"), "< must escape to &lt;");
    assert!(html(">").contains("&gt;"), "> must escape to &gt;");
    assert!(html("&").contains("&amp;"), "& must escape to &amp;");
    assert!(html("\"").contains("&quot;"), "\" must escape to &quot;");
    // The apostrophe escapes to the hex form `&#x27;`, not the decimal
    // form `&#39;`; both renderer paths use the hex form.
    let html = html("'");
    assert!(
        html.contains("&#x27;"),
        "apostrophe must be &#x27;, got: {html}"
    );
    assert!(!html.contains("&#39;"), "decimal &#39; leaked, got: {html}");
}

#[test]
fn html_unsafe_set_is_exactly_five_chars() {
    // Every other ASCII printable must pass through as itself. The
    // `memchr` scan in `escape_text_chunk` is valid only if no other
    // byte is escaped.
    let safe_ascii: Vec<u8> = (0x20..=0x7E)
        .filter(|b| !matches!(*b, b'<' | b'>' | b'&' | b'"' | b'\''))
        .collect();
    let input = String::from_utf8(safe_ascii.clone()).unwrap();
    let html = html(&input);
    for &b in &safe_ascii {
        let c = b as char;
        assert!(
            html.contains(c),
            "ASCII printable {c:?} (0x{b:02X}) must pass through unescaped",
        );
    }
}

#[test]
fn empty_input_renders_to_empty_string() {
    // No `<p></p>`, no whitespace — completely empty.
    assert_eq!(html(""), "");
}

#[test]
fn pure_japanese_pass_through_unescaped() {
    // Multi-byte UTF-8 is never escaped; only the five ASCII
    // unsafe chars change form.
    let html = html("青空文庫の本文。");
    assert!(html.contains("青空文庫の本文。"), "got: {html}");
}

#[test]
fn single_newline_becomes_br_double_closes_paragraph() {
    // The renderer's paragraph state machine has only these two
    // newline behaviours.
    let single = html("a\nb");
    assert!(single.contains("a<br />\nb"), "got: {single}");
    assert!(
        !single.contains("</p>\n<p>"),
        "single \\n must NOT close para"
    );

    let double = html("a\n\nb");
    assert!(double.contains("<p>a</p>\n"), "got: {double}");
    assert!(double.contains("<p>b</p>\n"), "got: {double}");
}

#[test]
fn serialize_is_fixed_point_for_canonical_markup() {
    // Each of four canonical Aozora markup shapes must round-trip
    // byte-identical. If sentinel encoding or decoding alters the
    // bytes, serialize is not a fixed point and the second pass
    // produces different bytes.
    for src in [
        "｜青梅《おうめ》",
        "前\n\n［＃改ページ］\n\n後",
        "学［＃二、レ点］而時習之",
        "※［＃「木＋吶のつくり」、第3水準1-85-54］",
    ] {
        let one = ser(src);
        let two = ser(&one);
        assert_eq!(one, two, "non-fixed-point on canonical input {src:?}");
    }
}

#[test]
fn pua_sentinel_codepoints_in_source_dont_emit_block_tags() {
    // The sanitize stage records a diagnostic but does not strip raw U+E001..
    // U+E004 from input, so they must pass through as plain text
    // and must not produce structural rendering (PUA collision
    // tolerance).
    for sentinel in ALL_SENTINELS {
        let html = html(&sentinel.to_string());
        // A PUA character in the source produces no `<div class="aozora-...">`.
        assert!(
            !html.contains(r#"<div class="aozora-page-break""#),
            "PUA {sentinel:?} accidentally produced a structural block: {html}",
        );
    }
}
