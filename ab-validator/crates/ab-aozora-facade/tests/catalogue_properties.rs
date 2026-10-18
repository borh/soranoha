//! Property tests for the notation-hygiene catalogues (Tier1
//! [`canonical_directive`] and Tier2 [`degraded_directive`]).
//!
//! The `lint_catalogue` suite checks these invariants on the hand-picked
//! `CATALOGUE_SAMPLES` / `DEGRADED_SAMPLES`; these tests check them on generated
//! input, which detects a rule that matches an unlisted body it should not:
//!
//! - **no panic** on arbitrary UTF-8 in the `strip_suffix` and char-boundary
//!   logic;
//! - **disjoint**: no body resolves in both catalogues;
//! - **idempotent**: every output is a fixed point (not itself a key);
//! - **recognised output**: every Tier1/Tier2 output parses to a non-Unknown
//!   construct, and no Tier2 output is a Tier1 key, because the single
//!   serialize→lex pass would re-lex such an output to Unknown.

use ab_aozora_facade::Document;
use ab_aozora_syntax::degraded::degraded_directive;
use ab_aozora_syntax::lint::canonical_directive;
use proptest::prelude::*;

/// Returns true when `body` parses to a recognised (non-Unknown) directive, that
/// is, when the rendered HTML contains no `aozora-directive` span.
///
/// A forward-reference form (`「X」…` / bare `（…）は縦中横`) is recognised only
/// when its referent precedes it, so the referent is prepended, as in the
/// `lint_catalogue` sample loop.
fn is_recognised(body: &str) -> bool {
    let source = referent_context(body);
    !Document::new(source)
        .parse()
        .to_html()
        .contains("aozora-directive")
}

/// Prepend `body`'s forward referent (the first `「X」` inner text, or the whole
/// `（…）` operand for the bare-paren 縦中横 target), else a neutral `あ` lead.
fn referent_context(body: &str) -> String {
    if let Some(open) = body.find('「') {
        let rest = &body[open + '「'.len_utf8()..];
        if let Some(close) = rest.find('」') {
            return format!("{}\n［＃{body}］\n本文\n", &rest[..close]);
        }
    }
    if body.starts_with('（')
        && let Some(close) = body.find('）')
    {
        return format!("{}\n［＃{body}］\n本文\n", &body[..close + '）'.len_utf8()]);
    }
    format!("あ\n［＃{body}］\n本文\n")
}

/// A strategy for `［＃…］` bodies assembled from the fragments the catalogues
/// match (quoted targets, digit runs, region/marker keywords, editorial prose),
/// so that generated bodies match catalogue rules; uniformly random strings
/// almost always resolve to `None`.
fn directive_body() -> impl Strategy<Value = String> {
    let target = prop_oneof![
        Just("「梅」".to_owned()),
        Just("「AB」".to_owned()),
        Just("「甲」".to_owned()),
        Just("（一）".to_owned()),
    ];
    let digits = prop_oneof![
        Just("2".to_owned()),
        Just("10".to_owned()),
        Just("３".to_owned()),
    ];
    let keyword = prop_oneof![
        Just("は斜体字".to_owned()),
        Just("に黒丸傍点".to_owned()),
        Just("は横書き".to_owned()),
        Just("は小書き".to_owned()),
        Just("はゴチック".to_owned()),
        Just("は枠囲み".to_owned()),
        Just("に傍点（白丸）".to_owned()),
        Just("は縦中横".to_owned()),
        Just("は「乙」の誤記か".to_owned()), // editorial; must resolve to None
    ];
    let region = prop_oneof![
        Just("字下げ終わり".to_owned()),
        Just("ここで字下げおわり".to_owned()),
        Just("ここから表組".to_owned()),
        Just("中央寄せ".to_owned()),
        Just("地付きで".to_owned()),
        Just("ここから最後まで".to_owned()), // Tier2 prefix
        Just("地付き、地より".to_owned()),   // incomplete end-spacing prefix
    ];
    prop_oneof![
        // Forward form: 「X」 + keyword.
        (target, keyword.clone()).prop_map(|(t, k)| format!("{t}{k}")),
        // Digit-parameterised region open/close.
        (
            prop_oneof![
                Just("ここか".to_owned()),
                Just("以下".to_owned()),
                Just("この行".to_owned())
            ],
            digits.clone(),
            Just("字下げ".to_owned()),
        )
            .prop_map(|(p, n, s)| format!("{p}{n}{s}")),
        // Tier2 digit families.
        (region.clone(), digits.clone(), Just("字下げ".to_owned()))
            .prop_map(|(r, n, s)| format!("{r}{n}{s}")),
        (
            Just("地付き、地より".to_owned()),
            digits,
            Just("字アキ".to_owned())
        )
            .prop_map(|(p, n, s)| format!("{p}{n}{s}")),
        // Bare region / marker forms.
        region,
        keyword,
    ]
}

proptest! {
    /// Covers the `strip_suffix` and char-boundary arithmetic in `forward_form`
    /// and `parameterized`.
    #[test]
    fn matchers_never_panic_on_arbitrary_utf8(s in ".*") {
        let _c = canonical_directive(&s);
        let _d = degraded_directive(&s);
    }

    #[test]
    fn tier1_and_tier2_are_disjoint(body in directive_body()) {
        prop_assert!(
            !(canonical_directive(&body).is_some() && degraded_directive(&body).is_some()),
            "body {body:?} resolved in both catalogues",
        );
    }

    /// No catalogue output is a key of the same catalogue, so a second pass
    /// changes nothing; the fmt idempotency check depends on this property.
    #[test]
    fn outputs_are_fixed_points(body in directive_body()) {
        if let Some(c) = canonical_directive(&body) {
            prop_assert_eq!(canonical_directive(&c), None, "Tier1 output {} re-resolved", c);
        }
        if let Some(d) = degraded_directive(&body) {
            prop_assert_eq!(degraded_directive(&d), None, "Tier2 output {} re-reduced", d);
        }
    }

    /// Generated-input form of the `every_variant` sample loop: no rule emits a
    /// canonical form the parser does not recognise.
    #[test]
    fn tier1_output_is_parser_recognised(body in directive_body()) {
        if let Some(c) = canonical_directive(&body) {
            prop_assert!(is_recognised(&c), "Tier1 output {} is not recognised", c);
        }
    }

    /// The opt-in renderer does one serialize→lex pass, so a Tier2 output that is
    /// a Tier1 key would re-lex to Unknown and render as an unrecognised
    /// directive.
    #[test]
    fn tier2_output_is_recognised_and_not_a_tier1_key(body in directive_body()) {
        if let Some(d) = degraded_directive(&body) {
            prop_assert!(is_recognised(&d), "Tier2 output {} is not recognised", d);
            prop_assert_eq!(canonical_directive(&d), None, "Tier2 output {} is a Tier1 key", d);
        }
    }
}
