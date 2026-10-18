//! End-to-end tests that the fused lex pipeline emits its authoring-error
//! diagnostics.
//!
//! Each test parses a hand-written trigger string through [`lex`] and
//! asserts that the expected [`ab_aozora_spec::Diagnostic`] code is
//! emitted with the expected severity. The conformance render fixtures
//! (`crates/aozora-conformance/fixtures/render/<case>/`) pin the exact
//! span and JSON shape; these tests pin only whether the diagnostic is
//! emitted.

use ab_aozora_pipeline::lex;
use ab_aozora_spec::{Severity, codes};
/// Count diagnostics with the given stable code in a fresh parse of `src`.
fn count_code(src: &str, code: &str) -> usize {
    let out = lex(src);
    out.diagnostics.iter().filter(|d| d.code() == code).count()
}

/// Assert that exactly one diagnostic of `code` is emitted, and return its severity.
fn one_diag_severity(src: &str, code: &str) -> Severity {
    let out = lex(src);
    let hits: Vec<_> = out
        .diagnostics
        .iter()
        .filter(|d| d.code() == code)
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "expected exactly one `{code}` in {:?}",
        out.diagnostics
    );
    hits[0].severity()
}

// #8 accent_decomposition_applied (Note, sanitize stage)

#[test]
fn accent_decomposition_is_reported_as_note() {
    // `〔cafe'〕` decomposes to `〔café〕`; the span is not at byte 0, so a
    // coordinate error changes the conformance golden output.
    let sev = one_diag_severity("前〔cafe'〕後", codes::ACCENT_DECOMPOSITION_APPLIED);
    assert_eq!(sev, Severity::Note);
}

#[test]
fn accent_decomposition_one_note_per_substitution_site() {
    // Two spans with one digraph each → two notes.
    assert_eq!(
        count_code("前〔a`〕中〔e'〕後", codes::ACCENT_DECOMPOSITION_APPLIED),
        2
    );
    // Two digraphs inside one span also → two notes: the note is per
    // substitution site, bracketing the replacement character, not per span.
    assert_eq!(
        count_code("前〔ve'rite'〕後", codes::ACCENT_DECOMPOSITION_APPLIED),
        2
    );
}

#[test]
fn accent_span_without_digraph_reports_no_diagnostic() {
    // A `〔…〕` whose body holds no accent digraph must not produce the note.
    assert_eq!(
        count_code("〔plain〕text", codes::ACCENT_DECOMPOSITION_APPLIED),
        0
    );
    // Ordinary prose with no tortoiseshell brackets produces no note.
    assert_eq!(
        count_code("ふつうの文章です", codes::ACCENT_DECOMPOSITION_APPLIED),
        0
    );
}

// #6 unresolved_gaiji (Warning, classify stage)

#[test]
fn unresolved_gaiji_is_reported_as_warning() {
    // Same `※［＃「…」、<mencode>］` shape as a real gaiji, but the men-ku-ten
    // is out of range so it resolves to nothing and the multi-char
    // description is not a single fallback character → `ucs == None`.
    let sev = one_diag_severity(
        "未知の字※［＃「架空の外字」、第3水準99-99-99］です",
        codes::UNRESOLVED_GAIJI,
    );
    assert_eq!(sev, Severity::Warning);
}

#[test]
fn resolvable_gaiji_reports_no_diagnostic() {
    // A resolvable gaiji (JIS X 0213 第3水準1-85-54 → 𠀋) produces no diagnostic.
    assert_eq!(
        count_code(
            "珍しき木※［＃「木＋吶のつくり」、第3水準1-85-54］が立つ。",
            codes::UNRESOLVED_GAIJI
        ),
        0
    );
}

// #3 mismatched_container_close (Error, normalizer fold)

#[test]
fn mismatched_container_close_is_reported_as_error() {
    // Indent opener (`ここから2字下げ`) closed by an align-end closer
    // (`ここで地付き終わり`) → families differ → Error.
    let sev = one_diag_severity(
        "［＃ここから2字下げ］本文［＃ここで地付き終わり］",
        codes::MISMATCHED_CONTAINER_CLOSE,
    );
    assert_eq!(sev, Severity::Error);
}

#[test]
fn matched_container_close_reports_no_diagnostic() {
    // Same family open/close: the amount payload differs internally
    // (`Indent{2}` open vs `Indent{0}` close) but the discriminant is
    // equal, so no diagnostic is emitted.
    assert_eq!(
        count_code(
            "［＃ここから2字下げ］本文［＃ここで字下げ終わり］",
            codes::MISMATCHED_CONTAINER_CLOSE
        ),
        0
    );
    // Align-end pair: likewise no diagnostic.
    assert_eq!(
        count_code(
            "［＃ここから地から2字上げ］本文［＃ここで地付き終わり］",
            codes::MISMATCHED_CONTAINER_CLOSE
        ),
        0
    );
}

// #1 empty_ruby_reading (Error, classify stage)

#[test]
fn empty_ruby_reading_is_reported_as_error() {
    // Explicit `｜` base with an empty `《》` reading.
    let sev = one_diag_severity("｜青梅《》", codes::EMPTY_RUBY_READING);
    assert_eq!(sev, Severity::Error);
}

#[test]
fn valid_and_baseless_empty_ruby_report_no_diagnostic() {
    // A complete explicit ruby produces no diagnostic.
    assert_eq!(count_code("｜青梅《おうめ》", codes::EMPTY_RUBY_READING), 0);
    // An empty `《》` without an explicit base is literal text: without a
    // base, the source does not establish that ruby was intended, so no
    // diagnostic is emitted.
    assert_eq!(count_code("あ《》", codes::EMPTY_RUBY_READING), 0);
}

// #2 nested_ruby (Error, classify stage)

#[test]
fn nested_ruby_is_reported_as_error() {
    // The reading body of the outer `《…》` itself opens an inner `《ん》`.
    // `《` / `》` are single-character ruby triggers (the angle-quote
    // notation is the distinct `≪…≫`), so this is the genuine nested-ruby
    // shape the catalogue describes (`｜…《…《…》…》`).
    let sev = one_diag_severity("｜漢《か《ん》じ》", codes::NESTED_RUBY);
    assert_eq!(sev, Severity::Error);
}

#[test]
fn flat_ruby_is_not_nested() {
    // A normal ruby and two adjacent rubies produce no nested-ruby diagnostic.
    assert_eq!(count_code("｜青梅《おうめ》", codes::NESTED_RUBY), 0);
    assert_eq!(
        count_code("｜青《あお》｜梅《うめ》", codes::NESTED_RUBY),
        0
    );
}

// #7 unrecognised_container_directive (Warning, classify stage)

#[test]
fn unrecognised_container_directive_is_reported_as_warning() {
    // `ここから…` opener whose remainder is not a known container kind.
    let sev = one_diag_severity(
        "［＃ここからナントカ］本文",
        codes::UNRECOGNISED_CONTAINER_DIRECTIVE,
    );
    assert_eq!(sev, Severity::Warning);
}

#[test]
fn known_and_non_container_directives_report_no_diagnostic() {
    // A valid container opener resolves → no warning.
    assert_eq!(
        count_code(
            "［＃ここから2字下げ］本文［＃ここで字下げ終わり］",
            codes::UNRECOGNISED_CONTAINER_DIRECTIVE
        ),
        0
    );
    // An unknown annotation that is not a `ここから` directive is a plain
    // `Directive{Unknown}` and must not receive the container diagnostic.
    assert_eq!(
        count_code(
            "［＃ふつうの注記］",
            codes::UNRECOGNISED_CONTAINER_DIRECTIVE
        ),
        0
    );
}

// #4 tcy_target_not_found (Warning, classify stage)

#[test]
fn tcy_target_not_found_fires_as_warning() {
    // `は縦中横` shape, but the target `い` does not appear before the bracket.
    let sev = one_diag_severity("あ［＃「い」は縦中横］", codes::TCY_TARGET_NOT_FOUND);
    assert_eq!(sev, Severity::Warning);
}

#[test]
fn tcy_with_present_target_reports_no_diagnostic() {
    // Target `64` precedes the directive → recognised → no warning.
    assert_eq!(
        count_code("昭和64［＃「64」は縦中横］年", codes::TCY_TARGET_NOT_FOUND),
        0
    );
}

// #5 bouten_target_ambiguous (Warning, classify stage)

#[test]
fn detached_bouten_target_ambiguous_is_reported_as_warning() {
    let sev = one_diag_severity(
        "青空と青空。［＃「青空」に傍点］",
        codes::BOUTEN_TARGET_AMBIGUOUS,
    );
    assert_eq!(sev, Severity::Warning);
}

#[test]
fn bouten_unique_or_adjacent_target_reports_no_diagnostic() {
    for source in ["青空［＃「青空」に傍点］", "青空青空［＃「青空」に傍点］"]
    {
        assert_eq!(count_code(source, codes::BOUTEN_TARGET_AMBIGUOUS), 0);
    }
}

// break_in_single_line_container (Warning, normalizer fold)

#[test]
fn break_in_single_line_align_end_is_reported_as_warning() {
    // `［＃地付き］` is a single-line align-end marker; the `［＃改ページ］` on
    // the same source line drops it.
    let sev = one_diag_severity(
        "［＃地付き］本文［＃改ページ］",
        codes::BREAK_IN_SINGLE_LINE_CONTAINER,
    );
    assert_eq!(sev, Severity::Warning);
}

#[test]
fn break_in_single_line_indent_is_reported() {
    // Single-line `［＃2字下げ］` directive sharing a line with a section break.
    assert_eq!(
        count_code(
            "［＃2字下げ］本文［＃改段］",
            codes::BREAK_IN_SINGLE_LINE_CONTAINER
        ),
        1
    );
}

#[test]
fn break_inside_warichu_is_reported() {
    // A break between `［＃割り注］` and `［＃割り注終わり］` falls inside an
    // inline warichu range.
    assert_eq!(
        count_code(
            "本文［＃割り注］注記［＃改ページ］続き［＃割り注終わり］",
            codes::BREAK_IN_SINGLE_LINE_CONTAINER
        ),
        1
    );
}

#[test]
fn break_on_next_line_reports_no_diagnostic() {
    // The single-line directive ends with its line; a break on the following
    // line does not drop it.
    assert_eq!(
        count_code(
            "［＃地付き］本文\n［＃改ページ］",
            codes::BREAK_IN_SINGLE_LINE_CONTAINER
        ),
        0
    );
}

#[test]
fn break_in_block_container_reports_no_diagnostic() {
    // Paired `［＃ここから…］` block containers persist across breaks (print
    // typography): a break inside one produces no diagnostic.
    assert_eq!(
        count_code(
            "［＃ここから2字下げ］本文［＃改ページ］残り［＃ここで字下げ終わり］",
            codes::BREAK_IN_SINGLE_LINE_CONTAINER
        ),
        0
    );
    // A standalone break with no container in scope likewise produces none.
    assert_eq!(
        count_code(
            "本文［＃改ページ］次",
            codes::BREAK_IN_SINGLE_LINE_CONTAINER
        ),
        0
    );
}

// bracketed_kaeriten_no_pair (Error, classify-stage finalize)

#[test]
fn bracketed_kaeriten_no_pair_is_reported_as_error() {
    // A `［＃二］` whose clause has no `［＃一］` partner.
    let sev = one_diag_severity("怪物［＃二］", codes::BRACKETED_KAERITEN_NO_PAIR);
    assert_eq!(sev, Severity::Error);
}

#[test]
fn kaeriten_base_present_reports_no_diagnostic() {
    // `二` and `一` both present: order does not matter (real kanbun writes
    // `二` before `一`).
    assert_eq!(
        count_code("非［＃二］怪物［＃一］", codes::BRACKETED_KAERITEN_NO_PAIR),
        0
    );
    // A bare `レ` (re-ten) is standalone and never ladders.
    assert_eq!(
        count_code("有［＃レ］朋", codes::BRACKETED_KAERITEN_NO_PAIR),
        0
    );
    // 上下点 may skip `中`: `上` + `下` with no `中` is a valid two-level
    // pair (base `上` is present).
    assert_eq!(
        count_code("有［＃下］其人［＃上］", codes::BRACKETED_KAERITEN_NO_PAIR),
        0
    );
}

#[test]
fn kaeriten_base_presence_is_document_wide() {
    // The base `一` occurs in a different clause (after the `。`) than the
    // `二`: kanbun return groups span clause boundaries, so this is not
    // flagged (document-wide base presence).
    assert_eq!(
        count_code(
            "胡［＃二］。自國［＃一］",
            codes::BRACKETED_KAERITEN_NO_PAIR
        ),
        0
    );
    // `三` with `一` present but no `二` is silent: base-only, not a strict
    // ladder.
    assert_eq!(
        count_code("見［＃三］而知［＃一］", codes::BRACKETED_KAERITEN_NO_PAIR),
        0
    );
    // A document using `下` (rank 2/3) with no `上` anywhere → missing
    // base → reported.
    assert_eq!(
        count_code("有［＃下］耳", codes::BRACKETED_KAERITEN_NO_PAIR),
        1
    );
}

// kaeriten_outside_kanbun (Warning, classify-stage finalize, conservative)

#[test]
fn kaeriten_outside_kanbun_is_reported_as_warning() {
    // A single `レ` kaeriten (non-laddering, so no pair diagnostic) in
    // kana-dominant prose: the only kaeriten in the document.
    let sev = one_diag_severity("これは［＃レ］と書いた。", codes::KAERITEN_OUTSIDE_KANBUN);
    assert_eq!(sev, Severity::Warning);
}

#[test]
fn kaeriten_in_kanbun_context_reports_no_diagnostic() {
    // Multiple kaeriten (a real kanbun cluster) → never flagged.
    assert_eq!(
        count_code("自［＃二］女王國［＃一］", codes::KAERITEN_OUTSIDE_KANBUN),
        0
    );
    // A lone kaeriten in a kanji-dense (漢文-like) run → not prose → silent.
    assert_eq!(
        count_code("非［＃レ］怪", codes::KAERITEN_OUTSIDE_KANBUN),
        0
    );
}

// mismatched_bouten_container (Error, normalizer fold)

#[test]
fn mismatched_bouten_container_is_reported_as_error() {
    // 傍点 (点) range opened, closed by a 傍線 (線) closer → family mismatch.
    let sev = one_diag_severity(
        "彼は［＃傍点］必ず［＃傍線終わり］来る",
        codes::MISMATCHED_BOUTEN_CONTAINER,
    );
    assert_eq!(sev, Severity::Error);
    // The discriminant-level `mismatched_container_close` must not also be emitted
    // (both ends are `BoutenRange`).
    assert_eq!(
        count_code(
            "彼は［＃傍点］必ず［＃傍線終わり］来る",
            codes::MISMATCHED_CONTAINER_CLOSE
        ),
        0
    );
}

#[test]
fn matched_bouten_range_reports_no_diagnostic() {
    // Same family, exact variant → no diagnostic.
    assert_eq!(
        count_code(
            "彼は［＃傍点］必ず［＃傍点終わり］来る",
            codes::MISMATCHED_BOUTEN_CONTAINER
        ),
        0
    );
    // Same 点 family, different variant (丸傍点 vs 傍点) → recovers on the
    // opener's variant, not flagged (the catalogue scopes the diagnostic to
    // the 点/線 family boundary).
    assert_eq!(
        count_code(
            "彼は［＃丸傍点］必ず［＃傍点終わり］来る",
            codes::MISMATCHED_BOUTEN_CONTAINER
        ),
        0
    );
    // Line family matched.
    assert_eq!(
        count_code(
            "彼は［＃二重傍線］必ず［＃傍線終わり］来る",
            codes::MISMATCHED_BOUTEN_CONTAINER
        ),
        0
    );
}
