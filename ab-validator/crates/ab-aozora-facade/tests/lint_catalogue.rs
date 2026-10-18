//! Parse round-trip tests of the notation-hygiene lint
//! (`aozora::lint::non_canonical_directive`), which check the
//! zero-false-positive invariant against changes in the parser's recognisers.
//!
//! For every catalogue sample, the variant parsed as a directive must (a) be an
//! Unknown directive, the condition for linting it, and (b) trigger the lint;
//! its suggested canonical form must parse to a recognised construct (no lint,
//! no bare directive). If a parser change starts recognising a variant or stops
//! recognising a canonical form, this test fails.

use ab_aozora_facade::Document;
use ab_aozora_facade::render::{DirectiveNormalization, RenderOptions, SerializeOptions};
use ab_aozora_syntax::degraded::{DEGRADED_SAMPLES, degraded_directive};
use ab_aozora_syntax::lint::{CATALOGUE_SAMPLES, EDITORIAL_MUST_STAY_UNKNOWN, canonical_directive};

const LINT_CODE: &str = "aozora::lint::non_canonical_directive";

fn fires_lint(body: &str) -> bool {
    source_fires_lint(&format!("あ［＃{body}］"))
}

fn source_fires_lint(source: &str) -> bool {
    Document::new(source.to_owned())
        .parse()
        .diagnostics()
        .iter()
        .any(|d| d.code() == LINT_CODE)
}

#[test]
fn every_variant_is_unknown_and_triggers_the_lint() {
    for &variant in CATALOGUE_SAMPLES {
        // The variant resolves to a canonical form,
        let canonical = canonical_directive(variant)
            .unwrap_or_else(|| panic!("catalogue sample {variant:?} did not resolve"));
        // is Unknown (renders as the hidden directive span),
        let vhtml = Document::new(format!("あ［＃{variant}］"))
            .parse()
            .to_html();
        assert!(
            vhtml.contains("aozora-directive"),
            "variant {variant:?} is unexpectedly recognised; drop it from the catalogue"
        );
        // and triggers the notation-hygiene lint.
        assert!(
            fires_lint(variant),
            "catalogue sample {variant:?} did not fire the lint"
        );
        // The suggested canonical form is not itself a lint variant; otherwise
        // the suggestion would trigger the lint again.
        assert!(
            !fires_lint(&canonical),
            "canonical {canonical:?} for {variant:?} is itself a lint variant"
        );
    }
}

/// The `aozora fmt --fix` autofix uses the canonical catalogue, as do the
/// pipeline lint and the LSP quick-fix. For every catalogue sample, the default
/// serialization keeps the variant verbatim and lint-flagged; the opt-in
/// serialization replaces the near-miss so that the lint does not trigger, and
/// a second opt-in pass changes nothing, a property on which the `write_back`
/// idempotency check depends.
#[test]
fn fix_resolves_every_variant_and_is_idempotent() {
    let fix = SerializeOptions {
        directives: DirectiveNormalization::Canonical,
    };
    for &variant in CATALOGUE_SAMPLES {
        let input = format!("あ［＃{variant}］");

        // Default fmt output contains the flagged near-miss verbatim, because
        // the parser is lossless and only opt-in fmt rewrites directives.
        let plain = Document::new(input.clone()).parse().to_source();
        assert!(
            plain.contains(&format!("［＃{variant}］")),
            "default fmt must preserve variant {variant:?} verbatim; got {plain:?}"
        );
        assert!(
            source_fires_lint(&plain),
            "default fmt output should still fire the lint for {variant:?}"
        );

        // --fix rewrites the near-miss to canonical form, on which the
        // notation-hygiene lint does not trigger.
        let fixed = Document::new(input.clone()).parse().to_source_with(fix);
        assert!(
            !source_fires_lint(&fixed),
            "fix should resolve the near-miss {variant:?}; got {fixed:?}"
        );

        // After the rewrite every directive is canonical (non-Unknown), so a
        // second fix pass changes nothing.
        let again = Document::new(fixed.clone()).parse().to_source_with(fix);
        assert_eq!(fixed, again, "fix must be idempotent for {variant:?}");
    }
}

/// Returns a source in which `variant`'s canonical spelling can render. A
/// forward-reference form renders only when its referent is in the preceding
/// text, so the forward target literal is prepended: the inner text of the
/// first `「X」` for a quoted forward, or the whole `（…）` operand for the
/// bare-parenthesised 縦中横 target `（…）`, whose canonical `「（…）」は縦中横`
/// quotes the parenthesised run. Every non-forward form (region open/close,
/// alignment, marker synonyms) receives a neutral `あ` lead.
fn render_context_source(variant: &str) -> String {
    if let Some(open) = variant.find('「') {
        let rest = &variant[open + '「'.len_utf8()..];
        if let Some(close) = rest.find('」') {
            return format!("{referent}［＃{variant}］", referent = &rest[..close]);
        }
    }
    if variant.starts_with('（')
        && let Some(close) = variant.find('）')
    {
        return format!(
            "{referent}［＃{variant}］",
            referent = &variant[..close + '）'.len_utf8()]
        );
    }
    format!("あ［＃{variant}］")
}

/// For every catalogue variant, the normalising render path replaces the
/// `Unknown` directive span with a rendering of the canonical spelling.
/// `every_variant_is_unknown_and_triggers_the_lint` checks that a canonical form
/// is not a lint variant; this test checks that it renders. The default render
/// keeps the `Unknown` directive span because the parser does not reinterpret a
/// near-miss.
#[test]
fn normalize_render_replaces_every_inert_variant() {
    let norm = RenderOptions {
        directives: DirectiveNormalization::Canonical,
    };
    for &variant in CATALOGUE_SAMPLES {
        let source = render_context_source(variant);

        // (a) Without normalisation the parser keeps the near-miss Unknown and
        // renders the hidden `aozora-directive` span.
        let default_html = Document::new(source.clone()).parse().to_html();
        assert!(
            default_html.contains("aozora-directive"),
            "variant {variant:?} should render inert by default; got {default_html:?}"
        );

        // (b) With normalisation the canonical form replaces the near-miss, and
        // the rendered HTML contains neither `aozora-directive` nor a ` hidden>`
        // span: the output is a visible element, or nothing for a referent-less
        // or region form, and no directive or heading-hint span.
        let normalized_html = Document::new(source.clone()).parse().to_html_with(norm);
        assert!(
            !normalized_html.contains("aozora-directive") && !normalized_html.contains(" hidden>"),
            "variant {variant:?} still renders inert after normalisation; \
             source {source:?} gave {normalized_html:?}"
        );
    }
}

/// `to_html_with` with the default (opt-out) [`RenderOptions`] is byte-identical
/// to `to_html()` for plain text, a construct, a recognised directive, and a
/// catalogue near-miss.
#[test]
fn default_render_options_are_byte_identical_to_to_html() {
    for source in [
        "ただの平文です。",
        "｜青梅《おうめ》",
        "重要［＃「重要」は太字］",
        "あ［＃斜体字］",             // a Tier1 near-miss
        "あ［＃中文字、ゴシック体］", // a Tier2 / degraded form
    ] {
        let doc = Document::new(source.to_owned());
        let tree = doc.parse();
        assert_eq!(
            tree.to_html_with(RenderOptions::default()),
            tree.to_html(),
            "default RenderOptions must be byte-identical to to_html() for {source:?}"
        );
    }
}

#[test]
fn editorial_unknown_does_not_trigger_the_lint() {
    for body in ["底本では「蒼空」", "入力者注", "「」は「」の「」"] {
        assert!(
            !fires_lint(body),
            "editorial Unknown {body:?} wrongly fired the notation-hygiene lint"
        );
    }
}

/// Every body in the negative catalogue `EDITORIAL_MUST_STAY_UNKNOWN`, the
/// exclusion list drawn from corpus residue, is an editorial, compound or
/// gaiji-composition body that resolves in neither Tier1 nor Tier2, remains a
/// lossless `Unknown` directive, and does not trigger the lint. This test checks
/// the zero-false-positive invariant on negative cases: a rule that matches
/// editorial prose fails it.
#[test]
fn catalogue_refuses_every_editorial_body() {
    for &body in EDITORIAL_MUST_STAY_UNKNOWN {
        assert_eq!(
            canonical_directive(body),
            None,
            "Tier1 must refuse editorial body {body:?}"
        );
        assert_eq!(
            degraded_directive(body),
            None,
            "Tier2 must refuse editorial body {body:?}"
        );
        assert!(
            !fires_lint(body),
            "editorial body {body:?} wrongly fired the notation-hygiene lint"
        );
        // The body parses to the hidden directive span of a lossless Unknown.
        let html = Document::new(format!("あ\n［＃{body}］\n本文\n"))
            .parse()
            .to_html();
        assert!(
            html.contains("aozora-directive"),
            "editorial body {body:?} must stay an inert Unknown directive"
        );
    }
}

/// Every Tier2 (`ab_aozora_syntax::degraded`) sample is Unknown as written,
/// reduces to a spelling the parser recognises, and is idempotent; this is the
/// Tier2 counterpart of `every_variant_is_unknown_and_triggers_the_lint`.
#[test]
fn every_degraded_sample_reduces() {
    for &sample in DEGRADED_SAMPLES {
        let reduced = degraded_directive(sample)
            .unwrap_or_else(|| panic!("degraded sample {sample:?} did not reduce"));
        // The parser does not recognise the sample, which remains Unknown.
        let sample_html = Document::new(format!("あ\n［＃{sample}］\n本文\n"))
            .parse()
            .to_html();
        assert!(
            sample_html.contains("aozora-directive"),
            "degraded sample {sample:?} is unexpectedly recognised by the parser"
        );
        // Its reduction parses to a recognised (non-Unknown) construct.
        let reduced_html = Document::new(format!("あ\n［＃{reduced}］\n本文\n"))
            .parse()
            .to_html();
        assert!(
            !reduced_html.contains("aozora-directive"),
            "degraded reduction {reduced:?} for {sample:?} must be recognised; got {reduced_html:?}"
        );
        // Idempotent: the reduction is not itself a degraded key.
        assert_eq!(
            degraded_directive(&reduced),
            None,
            "degraded reduction {reduced:?} must not re-reduce"
        );
    }
}

/// No sample resolves in both catalogues, which keeps the lossy and
/// judgment-dependent Tier2 reductions out of the zero-false-positive Tier1 map.
#[test]
fn tier1_and_tier2_are_disjoint() {
    for &t1 in CATALOGUE_SAMPLES {
        assert_eq!(
            degraded_directive(t1),
            None,
            "Tier1 sample {t1:?} must not also be a Tier2 (degraded) reduction"
        );
    }
    for &t2 in DEGRADED_SAMPLES {
        assert_eq!(
            canonical_directive(t2),
            None,
            "Tier2 sample {t2:?} must not also be a Tier1 (canonical) near-miss"
        );
    }
}

/// A Tier2 reduction applies only in the opt-in `Degraded` render. Under
/// `Degraded` each form renders as a recognised construct; under `Canonical`
/// (`render --normalize`) it remains an `Unknown` directive because it is not a
/// Tier1 form; and `fmt --fix` (`Canonical` serialization) leaves its bytes
/// verbatim, so a lossy Tier2 reduction does not rewrite source.
#[test]
fn degraded_reductions_are_render_only() {
    let degraded = RenderOptions {
        directives: DirectiveNormalization::Degraded,
    };
    let canonical = RenderOptions {
        directives: DirectiveNormalization::Canonical,
    };
    let fix = SerializeOptions {
        directives: DirectiveNormalization::Canonical,
    };
    for &sample in DEGRADED_SAMPLES {
        let source = format!("あ\n［＃{sample}］\n本文\n");
        // (a) --degraded renders it without a directive span.
        let deg_html = Document::new(source.clone()).parse().to_html_with(degraded);
        assert!(
            !deg_html.contains("aozora-directive") && !deg_html.contains(" hidden>"),
            "degraded sample {sample:?} still renders inert under --degraded; got {deg_html:?}"
        );
        // (b) --normalize (Tier1 only) keeps the directive span, because the
        // sample is not a Tier1 form.
        let canon_html = Document::new(source.clone())
            .parse()
            .to_html_with(canonical);
        assert!(
            canon_html.contains("aozora-directive"),
            "degraded sample {sample:?} must stay inert under --normalize (Tier1 only); \
             got {canon_html:?}"
        );
        // (c) fmt --fix leaves the exact bytes verbatim (no source rewrite).
        let fixed = Document::new(source.clone()).parse().to_source_with(fix);
        assert!(
            fixed.contains(&format!("［＃{sample}］")),
            "fmt --fix must keep degraded sample {sample:?} verbatim; got {fixed:?}"
        );
    }
}

/// The parser keeps `ここから最後まで3字下げ` as Unknown to preserve its
/// until-EOF scope, because the parser has no until-end indent construct. Tier1
/// does not resolve it; the lossy reduction, which drops 最後まで, is in Tier2
/// and applies only under `--degraded`.
#[test]
fn tier1_never_overrides_parser_spelling_preservation() {
    let preserved = "ここから最後まで3字下げ";
    let html = Document::new(format!("［＃{preserved}］\n本文\n"))
        .parse()
        .to_html();
    assert!(
        html.contains("aozora-directive"),
        "parser must keep {preserved:?} Unknown to preserve its scope"
    );
    // A Tier1 resolution would discard the 最後まで scope without the
    // degradation opt-in.
    assert_eq!(
        canonical_directive(preserved),
        None,
        "Tier1 must not override the parser's preservation of {preserved:?}"
    );
    // It is a Tier2 reduction instead (opt-in, render-only).
    assert_eq!(
        degraded_directive(preserved).as_deref(),
        Some("ここから3字下げ")
    );
}

/// Tier2 does not reduce editorial, compound, or composition-note bodies,
/// because a reduction would alter editorial prose or synthesize missing data.
#[test]
fn degraded_does_not_reduce_editorial_or_compound_bodies() {
    for body in [
        "「甲」は「乙」の誤記か",               // editorial conjecture
        "初出時「甲」",                         // bibliographic note
        "「甲」は筑摩版では「乙」",             // collation note
        "「窗」の下に「心」",                   // gaiji composition description
        "「甲」は縦中横、「乙」は上付き小書き", // multi-axis compound
        "図１入る",                             // truncated illustration (no file operand)
    ] {
        assert_eq!(
            degraded_directive(body),
            None,
            "degraded matcher must refuse {body:?}"
        );
    }
}
