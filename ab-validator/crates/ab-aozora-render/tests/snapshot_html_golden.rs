//! Snapshot HTML output for a curated sample of inputs.
//!
//! Complementary to `aozora-conformance::render_gate.rs` — that test
//! does byte-identical golden comparison against `expected.html`
//! files committed alongside fixtures. This one snapshots the
//! rendered HTML for a smaller hand-curated set of inputs that are
//! easier to read in `cargo insta review`, with `insta` filters that
//! mask incidental whitespace runs so reviewers can focus on the
//! structural part of the diff.
//!
//! Each test covers one kind of construct in isolation, so a renderer
//! change that alters the wrapper tag for one variant appears as a
//! one-test diff instead of a mass conformance failure.

use ab_aozora_pipeline::lex;
use ab_aozora_render::render_html;
fn render(source: &str) -> String {
    let out = lex(source);
    render_html(&out)
}

#[test]
fn snapshot_plain_text() {
    insta::assert_snapshot!(render("Hello, world."));
}

#[test]
fn snapshot_explicit_ruby() {
    insta::assert_snapshot!(render("｜青梅《おうめ》"));
}

#[test]
fn snapshot_implicit_ruby() {
    insta::assert_snapshot!(render("青梅《おうめ》"));
}

#[test]
fn snapshot_angle_quote() {
    insta::assert_snapshot!(render("≪重要≫"));
}

#[test]
fn snapshot_bracket_annotation() {
    insta::assert_snapshot!(render("text［＃改ページ］more text"));
}

#[test]
fn snapshot_gaiji_marker() {
    insta::assert_snapshot!(render("※［＃「木＋吶のつくり」、第3水準1-85-54］"));
}

#[test]
fn snapshot_paired_indent_container() {
    insta::assert_snapshot!(render(
        "［＃ここから2字下げ］\nbody\n［＃ここで字下げ終わり］"
    ));
}

#[test]
fn snapshot_xss_payload_is_escaped() {
    // A rendered `<script>` substring would be a script-injection
    // defect. The snapshot records the escaped form the renderer emits
    // for this input.
    insta::assert_snapshot!(render("<script>alert(1)</script>"));
}

#[test]
fn snapshot_contiguous_forward_styles_referent() {
    // A non-adjacent referent in the same plain run is styled in place
    // (a `Detached` decoration spliced at 青空) while the bracket stays
    // `Referenced` and renders nothing — 青空 appears once, bouten-styled,
    // preserving the no-double-render invariant.
    insta::assert_snapshot!(render("青空の下を歩く［＃「青空」に傍点］"));
}

#[test]
fn snapshot_referenced_ruby_base_forward_no_double_render() {
    // The bouten target resolves to a ruby base (not representable as a
    // text-only leaf), so the bracket stays `Referenced` (renders nothing) and
    // the lowering pass decorates the ruby's base instead — 我 renders once,
    // wrapped in the emphasis element inside the `<ruby>`, preserving the
    // no-double-render invariant.
    insta::assert_snapshot!(render("我《われ》の名は［＃「我」に傍点］"));
}
