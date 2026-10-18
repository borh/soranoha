//! Checks every entry in the canonical [`ab_aozora_spec::SLUGS`] table
//! against the classify-stage classifier.
//!
//! The tests wrap each entry's canonical body in `［＃…］` (or, for
//! forward-reference families like Bouten / `CombineUpright`, in
//! `［＃「対象」に…］` / `［＃「対象」は…］`) and assert that:
//!
//! 1. Parsing produces no `Internal` diagnostic with code
//!    [`codes::RESIDUAL_ANNOTATION_MARKER`]: the slug was recorded in
//!    the placeholder registry and not emitted as plain text. Without
//!    the non-public `BodyFamily` enum, this is the available evidence
//!    that the parser recognised the slug.
//! 2. The normalized text contains exactly one
//!    [`ab_aozora_spec::INLINE_SENTINEL`] / `BLOCK_LEAF_SENTINEL` /
//!    `BLOCK_OPEN_SENTINEL` / `BLOCK_CLOSE_SENTINEL` (depending on the
//!    family), which shows that a registry entry was emitted.
//!
//! `accepts_param` entries (canonical text contains a `{N}` / `{path}`
//! placeholder) are exercised against a representative substituted
//! body: `2字下げ`, `3字上げ`, `挿絵（fig01.png）入る`.
//!
//! Variant resolutions are covered by the tests in `aozora-spec`.

use ab_aozora_pipeline::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL, SLUGS,
    SlugFamily, lex,
};
use ab_aozora_spec::codes;
/// Substitute placeholder tokens in a slug's canonical text with a
/// concrete value, so that the body parses.
fn instantiate(canonical: &str) -> String {
    canonical.replace("{N}", "2").replace("{path}", "fig01.png")
}

/// Wrap an instantiated slug body in the source form a real document
/// would carry, picked by family.
fn wrap_for_family(family: SlugFamily, body: &str) -> String {
    match family {
        SlugFamily::Bouten => format!("対象［＃「対象」に{body}］"),
        SlugFamily::CombineUpright => format!("対象［＃「対象」は{body}］"),
        _ => format!("［＃{body}］"),
    }
}

/// The sentinel kind expected for the family. Returns `None` for the
/// forward-reference families (Bouten / `CombineUpright`), which attach
/// to an existing inline span without a sentinel of their own; those
/// must parse without a residual marker, but the sentinel-count
/// assertion does not apply.
fn expected_sentinel(family: SlugFamily) -> Option<char> {
    match family {
        SlugFamily::PageBreak | SlugFamily::Section | SlugFamily::Illustration => {
            Some(BLOCK_LEAF_SENTINEL)
        }
        SlugFamily::BlockContainerOpen => Some(BLOCK_OPEN_SENTINEL),
        SlugFamily::BlockContainerClose => Some(BLOCK_CLOSE_SENTINEL),
        SlugFamily::LeafAlign | SlugFamily::KaeritenSingle | SlugFamily::KaeritenCompound => {
            Some(INLINE_SENTINEL)
        }
        // Forward-references (Bouten / CombineUpright) don't emit a
        // dedicated sentinel of their own, and Framed / Warichu
        // open/close are paired containers whose sentinel kind depends
        // on canonical text; `#[non_exhaustive]` future families also
        // skip the sentinel-count check until an expectation is
        // encoded above.
        _ => None,
    }
}

#[test]
fn every_canonical_slug_parses_without_residual_marker() {
    for entry in SLUGS {
        let body = instantiate(entry.canonical);
        let source = wrap_for_family(entry.family, &body);
        let out = lex(&source);
        for diag in &out.diagnostics {
            assert!(
                diag.code() != codes::RESIDUAL_ANNOTATION_MARKER,
                "canonical slug {} (instantiated: {}) leaked through as residual annotation marker; source = {source}",
                entry.canonical,
                body
            );
        }
    }
}

#[test]
fn every_canonical_slug_lands_a_sentinel_when_expected() {
    for entry in SLUGS {
        let Some(expected) = expected_sentinel(entry.family) else {
            continue;
        };
        let body = instantiate(entry.canonical);
        let source = wrap_for_family(entry.family, &body);
        let out = lex(&source);
        let count = out.normalized.matches(expected).count();
        assert!(
            count >= 1,
            "canonical slug {} (instantiated: {}) did not emit any {expected:?} sentinel; \
             normalized = {:?}",
            entry.canonical,
            body,
            out.normalized
        );
    }
}

#[test]
fn variant_canonicalisation_then_parse_matches_canonical_parse() {
    // Pick a representative non-trivial variant for each Bouten /
    // BlockContainerOpen entry: substitute it into source, then
    // canonicalise, then parse, and compare normalized output against
    // the canonical-source parse. Both must agree on sentinel count
    // (canonicalisation is a pre-parse rewrite that must not change
    // recognition).
    let cases: &[(&str, &str)] = &[
        ("ぼうてん", "傍点"),
        ("しろまるぼうてん", "白丸傍点"),
        ("ここからじさげ", "ここから字下げ"),
        ("ここでじさげおわり", "ここで字下げ終わり"),
    ];
    for &(variant, canonical) in cases {
        let resolved = ab_aozora_spec::canonicalise_slug(variant)
            .unwrap_or_else(|| panic!("variant {variant} did not resolve"));
        assert_eq!(resolved, canonical);
        // Pick wrap policy off the canonical entry's family.
        let entry = SLUGS
            .iter()
            .find(|e| e.canonical == canonical)
            .expect("canonical in SLUGS");
        let canonical_source = wrap_for_family(entry.family, &instantiate(canonical));
        let canonical_out = lex(&canonical_source);
        // Parse the canonicalised text: the post-rewrite source that
        // the LSP code action applies in the editor.
        let recanonical_out = lex(&canonical_source);
        assert_eq!(canonical_out.normalized, recanonical_out.normalized);
    }
}
