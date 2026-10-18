//! Non-canonical directive catalogue for the notation-hygiene lint.
//!
//! [`canonical_directive`] maps a `［＃…］` directive body that the parser
//! keeps as `DirectiveKind::Unknown` (because it is spelled as a verified
//! near-miss of a recognized construct: a 送り仮名 variant, a synonym, or a
//! malformed prefix / close) to its canonical spelling. It returns `None`
//! for a genuine editorial Unknown, so the lint that consumes it reports only
//! bodies in the closed, parser-verified catalogue below.
//!
//! The catalogue is a fixed map, not a fuzzy matcher, so it has no false
//! positives outside its keys. The `lint_catalogue` test in `ab-aozora-facade`
//! asserts that every suggested canonical parses to a non-Unknown node
//! and every catalogue key still parses to Unknown, so the map cannot suggest
//! a form the parser rejects or match a body the parser already recognizes.
//!
//! As [`crate::accent`] is for accents, this module is the single source of
//! canonical directive forms: the pipeline lint in `ab-aozora-pipeline` and
//! the `Canonical` serializer rewrite in `ab-aozora-render` both resolve the
//! canonical form here, on the trimmed body string, with no
//! `DirectiveKind` dependency.

use std::borrow::Cow;

/// Literal whole-body variant → canonical maps.
const EXACT: &[(&str, &str)] = &[
    // 字下げ close: okurigana, 文字下げ, and bare-with-N variants of ここで字下げ終わり.
    ("字下げ終り", "ここで字下げ終わり"),
    ("字下げおわり", "ここで字下げ終わり"),
    ("文字下げ終わり", "ここで字下げ終わり"),
    ("二字下げ終わり", "ここで字下げ終わり"),
    // Marker and synonym variants.
    ("斜体字", "斜体"),
    ("中中見出し", "中見出し"),
    // ゴチック → ゴシック体 (the parser recognises ゴシック体 as a first-class
    // gothic construct; ゴチック is a variant spelling rare in the corpus).
    ("ゴチック", "ゴシック体"),
    ("ここからゴチック", "ここからゴシック体"),
    ("ここでゴチック終わり", "ここでゴシック体終わり"),
];

/// Map a non-canonical `［＃…］` directive body to its canonical spelling, or
/// `None` for a genuine editorial Unknown. `body` is the trimmed inner text,
/// without the `［＃` / `］` delimiters.
#[must_use]
pub fn canonical_directive(body: &str) -> Option<Cow<'static, str>> {
    if let Some((_, canonical)) = EXACT.iter().find(|(variant, _)| *variant == body) {
        return Some(Cow::Borrowed(canonical));
    }
    parameterized(body)
        .or_else(|| forward_form(body))
        .map(Cow::Owned)
}

/// A digit run: one or more ASCII `0-9` or full-width `０-９` characters.
pub(crate) fn is_digit_run(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_digit() || ('０'..='９').contains(&c))
}

/// Digit-preserving rules: the `{N}` run is copied verbatim. Grouped by the
/// scope each family normalises (region-open / region-close / alignment); the
/// result of the first group that matches the body is returned.
fn parameterized(body: &str) -> Option<String> {
    parameterized_indent_open(body)
        .or_else(|| parameterized_region_close(body))
        .or_else(|| parameterized_align(body))
}

/// A truncated region-open prefix → canonical `ここから{N}字下げ`.
fn parameterized_indent_open(body: &str) -> Option<String> {
    let n = body
        .strip_prefix("ここか")
        .and_then(|rest| rest.strip_suffix("字下げ"))?;
    if is_digit_run(n) {
        return Some(format!("ここから{n}字下げ"));
    }
    None
}

/// Remove a trailing bracket from an otherwise complete region closer.
/// Tail-anchored and digit-run guarded, so `、`-bearing compound closes (which
/// carry a second axis) never match.
fn parameterized_region_close(body: &str) -> Option<String> {
    body.strip_prefix("ここで")?;
    // ここで…終わり」 → drop a stray trailing `」`.
    if let Some(head) = body.strip_suffix('」')
        && head.ends_with("終わり")
    {
        return Some(head.to_owned());
    }
    None
}

/// Normalize equivalent wording of quantified indentation.
fn parameterized_align(body: &str) -> Option<String> {
    // {N}字下げて → {N}字下げ.
    if let Some(n) = body.strip_suffix("字下げて")
        && is_digit_run(n)
    {
        return Some(format!("{n}字下げ"));
    }
    None
}

/// Substitute unrecognized forward-reference keywords while retaining the target.
fn forward_form(body: &str) -> Option<String> {
    if !body.starts_with('「') {
        return None;
    }

    // Variant keyword after the target: replace it with the canonical keyword. Each
    // suffix is distinct and anchored at the body end, so the already-canonical
    // forms (e.g. `は下付き小文字`) cannot re-match (idempotent).
    for (variant_kw, canonical_kw) in [
        ("は斜体字", "は斜体"),
        ("は中中見出し", "は中見出し"),
        ("は上付き", "は上付き小文字"),
    ] {
        if let Some(head) = body.strip_suffix(variant_kw) {
            return Some(format!("{head}{canonical_kw}"));
        }
    }
    None
}

/// Concrete sample bodies covering every catalogue tier.
///
/// For the parse-round-trip self-test in the `aozora` crate (which alone can
/// call the parser). Each must satisfy: `canonical_directive(sample)` is
/// `Some`, `［＃sample］` parses to Unknown, and `［＃<canonical>］` parses to a
/// non-Unknown node.
pub const CATALOGUE_SAMPLES: &[&str] = &[
    "字下げ終り",
    "字下げおわり",
    "文字下げ終わり",
    "二字下げ終わり",
    "斜体字",
    "中中見出し",
    "２字下げて",
    "「梅」は斜体字",
    // Forward keyword / particle variants.
    "「2」は上付き",
    // The parser keeps these as Unknown; the lint suggests the canonical.
    "ゴチック",
    "ここでゴチック終わり",
    // Region-close synonym (parameterized).
    "ここで字下げ終わり」",
];

/// Bodies that stay a lossless `Unknown`: the negative catalogue for the
/// zero-false-positive invariant, listing bodies that no rule may match.
///
/// Drawn from the occurrence-ranked corpus residue (`corpus/render-digest.json`
/// `unknown_shapes_top`), these are the three families that account for most
/// of the tail and that neither Tier1 ([`canonical_directive`]) nor Tier2
/// ([`crate::degraded::degraded_directive`]) may match:
///
/// - **Editorial prose**: bibliographic / collation / conjecture / semantic
///   notes (edition names, `では`, `誤記か`, `伏字`, `注釈番号`, `正字`) and
///   free-form spatial-layout descriptions (`上に…付き`, `右側に…形で`) for which
///   the core models no construct. Matching one would misclassify an editor's note
///   as a directive.
/// - **Multi-axis compounds**: `、`-joined directives whose reduction
///   would drop an axis.
/// - **Gaiji-composition descriptions**: `「X」の下に「Y」` glyph builds, owned
///   by the 外字 layer, not the directive catalogues.
///
/// The `catalogue_refuses_every_editorial_body` self-test in the `aozora` crate
/// asserts both catalogues return `None` for every entry. Each catalogue
/// addition adds its adjacent editorial bodies here, so a rule that
/// over-generalizes fails this check before it misclassifies prose.
pub const EDITORIAL_MUST_STAY_UNKNOWN: &[&str] = &[
    // Editorial prose: bibliographic / collation / conjecture / semantic.
    "底本では「蒼空」",
    "入力者注",
    "未完",
    "「甲」は「乙」の誤記か",
    "初出時「甲」",
    "「甲」は筑摩版では「乙」",
    "底本3字伏字",
    "「甲」は注釈番号",
    "「甲」の「乙」に代えて「丙」",
    "一つ目の「甲」は「乙」付き",
    "「甲」は「乙」の右側に注記するような形で",
    // Unsupported spatial description: ruby/annotation attaches only right
    // (default) or 左に (left), never 上に (above).
    "「甲」は上に「乙」付き",
    // Gaiji-composition descriptions: owned by the 外字 layer.
    "「窗」の下に「心」",
    "「甲」の中に「乙」",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcription_statement_is_not_rewritten_as_an_extra_break() {
        assert_eq!(canonical_directive("改行を挿入"), None);
    }

    #[test]
    fn exact_maps_resolve() {
        assert_eq!(canonical_directive("斜体字").as_deref(), Some("斜体"));
        assert_eq!(
            canonical_directive("字下げおわり").as_deref(),
            Some("ここで字下げ終わり")
        );
        // 中文字、ゴシック体 is lossy (gothic→bold erases the spelling the parser
        // keeps Unknown to preserve), so Tier1 excludes it.
        assert_eq!(canonical_directive("中文字、ゴシック体"), None);
    }

    #[test]
    fn lowered_position_does_not_add_a_small_type_assertion() {
        assert_eq!(canonical_directive("「2」は下付き"), None);
    }

    #[test]
    fn parameterized_preserves_n() {
        assert_eq!(
            canonical_directive("ここか２字下げ").as_deref(),
            Some("ここから２字下げ")
        );
        assert_eq!(canonical_directive("ここより10字下げ"), None);
        assert_eq!(canonical_directive("2字下げて").as_deref(), Some("2字下げ"));
    }

    #[test]
    fn forward_form_preserves_target() {
        assert_eq!(canonical_directive("「梅」に黒丸傍点"), None);
        assert_eq!(
            canonical_directive("「梅」は斜体字").as_deref(),
            Some("「梅」は斜体")
        );
        assert_eq!(canonical_directive("「梅」傍点"), None);
        assert_eq!(canonical_directive("「語」は傍点"), None);
        assert_eq!(canonical_directive("「語」の傍点"), None);
    }

    #[test]
    fn forward_keyword_particle_variants_resolve() {
        assert_eq!(
            canonical_directive("「2」は上付き").as_deref(),
            Some("「2」は上付き小文字")
        );
        // Idempotent: the emitted canonical is not itself a catalogue key.
        assert_eq!(canonical_directive("「2」は上付き小文字"), None);
        assert_eq!(canonical_directive("「abc」はすべて下付き小文字"), None);
    }

    #[test]
    fn parenthesized_tcy_needs_no_spelling_repair() {
        assert_eq!(canonical_directive("（一）は縦中横"), None);
        // Already-quoted and compound-tail forms are not mapped.
        assert_eq!(canonical_directive("「（一）」は縦中横"), None);
        assert_eq!(
            canonical_directive("（十一）は縦中横、「十一」は縦組み"),
            None
        );
    }

    #[test]
    fn recognized_historical_alignment_is_not_a_lint_repair() {
        assert_eq!(canonical_directive("こ地付き"), None);
        assert_eq!(canonical_directive("地付き"), None, "canonical re-matched");
    }

    #[test]
    fn region_numeric_parameterized() {
        let variant = "ここで字下げ終わり」";
        let canonical = "ここで字下げ終わり";
        assert_eq!(canonical_directive(variant).as_deref(), Some(canonical));
        assert_eq!(canonical_directive(canonical), None);
    }

    #[test]
    fn region_numeric_excludes_editorial_and_compound() {
        assert_eq!(canonical_directive("この行は底本では３字下げ"), None);
        // A `、`-bearing compound close is not mapped, so its second axis is kept.
        assert_eq!(canonical_directive("ここで小文字、字下げ終わり"), None);
    }

    #[test]
    fn genuine_editorial_unknown_returns_none() {
        // Editorial notes do not match (the zero-false-positive invariant).
        assert!(canonical_directive("底本では「青空」").is_none());
        assert!(canonical_directive("入力者注").is_none());
        assert!(canonical_directive("「」は「」の「」").is_none());
        assert!(canonical_directive("未完").is_none());
        // Already-canonical forms parse to recognized nodes and are not keys.
        assert!(canonical_directive("ここで字下げ終わり").is_none());
        assert!(canonical_directive("地より2字上げ").is_none());
    }

    #[test]
    fn every_sample_resolves() {
        for &sample in CATALOGUE_SAMPLES {
            assert!(
                canonical_directive(sample).is_some(),
                "catalogue sample {sample:?} did not resolve"
            );
        }
    }
}
