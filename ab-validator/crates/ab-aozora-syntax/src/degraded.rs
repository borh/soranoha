//! Tier2 degraded-form matcher for the notation-hygiene layers.
//!
//! [`degraded_directive`] maps a `［＃…］` directive body that the parser keeps
//! as `DirectiveKind::Unknown` (and that Tier1, [`crate::lint::canonical_directive`],
//! excludes) to a directly parser-recognized spelling. Tier1 maps only verified
//! near-misses that lose no meaning, with zero false positives. Tier2 holds the
//! opt-in reductions that Tier1 excludes because they are lossy (a spelling or
//! scope the parser preserves is erased) or depend on judgment (two measurement
//! vocabularies folded via a typographic identity, not a spelling repair).
//!
//! Contract (looser input than Tier1, the same output requirements):
//! - Input: a degraded body for which `canonical_directive` returns `None`.
//! - Output: a directly parser-recognized spelling, never a Tier1 key,
//!   because the opt-in renderer performs a single serialize→lex pass
//!   ([`crate::lint`] doc); a Tier1-key output would re-lex to Unknown and
//!   render as an unrecognized directive.
//! - Disjoint from Tier1 and idempotent: `degraded_directive` is `None` for
//!   every Tier1 key and for its own outputs; `canonical_directive` is `None`
//!   for every Tier2 key.
//!
//! Only the opt-in renderer/interpreter
//! (`RenderOptions::directives == Degraded` / `aozora render --degraded`) calls
//! this module; the parser, the default lint, the default `fmt`, and `fmt --fix`
//! do not. Because `DirectiveNormalization::Degraded` is constructed at a single
//! ephemeral render site, an incorrect Tier2 reduction affects only `--degraded`
//! render output and does not rewrite source.

use std::borrow::Cow;

use crate::lint::is_digit_run;

/// Map a degraded `［＃…］` directive body to a recognized spelling, or `None`.
///
/// `body` is the trimmed inner text, without the `［＃` / `］` delimiters. See
/// the module docs for the Tier1/Tier2 contract. Each rule here loses or
/// re-derives meaning, so admitting it to Tier1 would let `fmt --fix` rewrite
/// source lossily while the purely syntactic Tier1 self-test still passes.
#[must_use]
pub fn degraded_directive(body: &str) -> Option<Cow<'static, str>> {
    // ゴシック体 is distinct from 太字; folding it to bold would lose meaning.
    // The size+gothic compound 中文字、ゴシック体 remains verbatim Unknown.

    // ここから最後まで{N}字下げ → ここから{N}字下げ. Lossy: 最後まで marks an
    // indent that auto-closes at document/section end; the parser has no
    // until-end concept. As a render-only reduction the output keeps that
    // meaning, because the EOF drain closes the open block at document end. As
    // a source rewrite it would leave an unclosed block, so fmt does not apply it.
    if let Some(n) = body
        .strip_prefix("ここから最後まで")
        .and_then(|r| r.strip_suffix("字下げ"))
        && is_digit_run(n)
    {
        return Some(Cow::Owned(format!("ここから{n}字下げ")));
    }

    None
}

/// Concrete degraded sample bodies, one per rule family.
///
/// For the parse-round-trip + meaning self-tests in the `aozora` crate (which
/// alone can call the parser). Each must satisfy: `degraded_directive(sample)`
/// is `Some`, `canonical_directive(sample)` is `None` (disjoint from Tier1),
/// `［＃sample］` parses to Unknown, `［＃<output>］` parses to a non-Unknown
/// node, and `degraded_directive(output)` is `None` (idempotent).
pub const DEGRADED_SAMPLES: &[&str] = &["ここから最後まで3字下げ"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_families_reduce() {
        assert_eq!(
            degraded_directive("ここから最後まで3字下げ").as_deref(),
            Some("ここから3字下げ")
        );
    }

    #[test]
    fn outputs_are_idempotent() {
        for sample in DEGRADED_SAMPLES {
            let out = degraded_directive(sample).expect("sample reduces");
            assert_eq!(
                degraded_directive(&out),
                None,
                "degraded output {out} must not re-reduce"
            );
        }
    }

    #[test]
    fn genuine_unknown_returns_none() {
        // A form the parser already recognizes, an editorial note, and a plain
        // typo, none of which the catalogue maps.
        assert_eq!(degraded_directive("中文字、太字"), None);
        assert_eq!(degraded_directive("「甲」は「乙」の誤記か"), None);
        assert_eq!(degraded_directive("ここから3字下げ"), None);
        assert_eq!(degraded_directive("字下げ終わり"), None);
    }
}
