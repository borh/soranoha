//! Gaiji (外字) resolution: mapping `※［＃…、mencode］` references
//! to Unicode characters.
//!
//! Two incoming shapes per the Aozora annotation manual:
//!
//! ```text
//!   ※［＃「description」、第3水準1-85-54］    ← JIS X 0213 plane-row-cell
//!   ※［＃「description」、U+XXXX、page-line］ ← explicit Unicode codepoint
//! ```
//!
//! The lexer's classify-stage recogniser (`aozora-pipeline::lexer::classify::recognize_gaiji`)
//! captures `description` and `mencode` verbatim and leaves `ucs = None`;
//! this module resolves that reference to a [`Resolved`] by looking it up
//! in two `phf::Map`s compiled into the binary (one for the
//! single-codepoint cells and one for the 25 combining-sequence cells)
//! and, for `U+XXXX` mencodes, by parsing the hex digits.
//!
//! ## `Resolved` variants
//!
//! 25 cells in JIS X 0213:2004 plane 1 (Ainu か゚ family, IPA tone marks,
//! some accented Latin letters) decode to a *combining sequence*: two
//! Unicode scalars that must be emitted together. A single `char` cannot
//! hold them, so the resolved value is either a [`char`] (~99.4% of
//! cells) or a `&'static str` borrowed from the generated combo table.
//! A `濁点付き…` description with no precomposed form resolves to a base
//! and a combining mark, two `char`s. Every variant is `Copy`, so the
//! parser's `Gaiji` payload, which embeds `Option<Resolved>`, and the
//! tree containing it remain `Copy`.
//!
//! ## Lookup order
//!
//! 1. **`existing`**: the caller-provided codepoint (e.g. extracted
//!    by an earlier escape recogniser), returned unchanged.
//! 2. **Combo table**: checked before the single-char table because it
//!    is the only table that maps a cell to two codepoints.
//! 3. **Single-char table**: one perfect-hash lookup in `.rodata`.
//! 4. **`U+XXXX` clause**: `U+` followed by 1–6 hex digits, parsed
//!    as a hex integer and validated via [`char::from_u32`].
//!
//!    Steps 2 to 4 apply to each reference clause of `mencode` and to the
//!    trailing reference clauses of `description`; two clauses that
//!    resolve to different values make the result `None`.
//! 5. **Description table**: keyed by the literal description text
//!    (外字注記辞書 entries and placeholders such as 〓, 〻).
//!
//!    A single code point from step 2 to 5 is refined to an Ideographic
//!    Variation Sequence when `data/aozora-gaiji-cjkvi-selectors.tsv` gives
//!    the description a selector on that same base; [`lookup_with_source`]
//!    reports this as [`MappingSource::CjkviSelector`].
//! 6. **Roman numeral**: a bare `ローマ数字N` description, composed from
//!    the U+2160 / U+2170 blocks.
//! 7. **Voicing mark**: a `濁点付き…` / `半濁点付き…` description,
//!    composed as the base followed by U+3099 / U+309A.
//! 8. **cjkvi-data tables**: keyed by the description text, from the
//!    community tables `data/aozora-gaiji-cjkvi.tsv` (single code point) and
//!    `data/aozora-gaiji-cjkvi-ivs.tsv` (base character with ideographic
//!    variation selector, returned as [`Resolved::Multi`]). In
//!    [`lookup_with_source`], matches are reported as
//!    [`MappingSource::CjkviData`].
//! 9. **Single-character description**: a description consisting of a
//!    single character resolves to that character.
//! 10. **None**: unresolved. The raw `description` is output by the
//!     renderer. Where available, [`external_identity`] provides the
//!     Ideographic Description Sequence and `GlyphWiki` glyph.
//!
//! ## Two PHF maps
//!
//! The single-char map includes both JIS X 0208 cells and JIS X 0213 additions.
//! The combo map has 25 entries.
//! Storing the single-char map as `phf::Map<&str, char>` keeps each value
//! at 4 bytes (a `&str` is 16 bytes), which keeps the cache footprint of
//! the common lookup small. The combo map is consulted first; a miss
//! there costs a single probe.

use core::fmt;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::LazyLock;

use crate::jisx0213_table::{
    DESCRIPTION_TO_CHAR, DESCRIPTION_TO_CHAR_CJKVI, DESCRIPTION_TO_EXTERNAL,
    DESCRIPTION_TO_SELECTOR, DESCRIPTION_TO_STR_CJKVI, JISX0213_MENCODE_TO_CHAR,
    JISX0213_MENCODE_TO_STR, ROMAN_NUMERAL_LOWER, ROMAN_NUMERAL_UPPER,
};

/// Resolution outcome — either a single Unicode scalar or a static
/// string covering a combining sequence.
///
/// Implements `Copy` so that `Gaiji`, which contains it, and the parser
/// tree remain `Copy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved {
    /// Common path: the mencode mapped to a single Unicode scalar
    /// (~99.4% of JIS X 0213:2004 cells, plus all `U+XXXX` shapes
    /// and the description fallback).
    Char(char),
    /// JIS X 0213 combining-sequence cell — 25 entries in plane 1
    /// (Ainu か゚ family, IPA tone marks, accented Latin). The string
    /// is borrowed from a static `phf::Map` value.
    Multi(&'static str),
    /// A base character followed by a combining voicing mark, from a
    /// `濁点付き…` / `半濁点付き…` description without a precomposed
    /// Unicode character (`エ` + U+3099, `井` + U+3099).
    Voiced {
        /// The kana, iteration mark, or kanji designated in the description.
        base: char,
        /// U+3099 COMBINING KATAKANA-HIRAGANA VOICED SOUND MARK or
        /// U+309A, its semi-voiced counterpart.
        mark: char,
    },
}

/// Plane, row, and cell of a single Unicode scalar in JIS X 0213.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JisCell {
    /// JIS X 0213 plane (1 or 2).
    pub plane: u8,
    /// Ku/row number (1 through 94).
    pub row: u8,
    /// Ten/cell number (1 through 94).
    pub cell: u8,
}

static CHAR_TO_JIS_CELL: LazyLock<BTreeMap<char, JisCell>> = LazyLock::new(|| {
    JISX0213_MENCODE_TO_CHAR
        .entries()
        .filter_map(|(mencode, character)| {
            let coordinates = mencode
                .split_once("水準")?
                .1
                .split('-')
                .map(str::parse::<u8>)
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            let [plane, row, cell] = coordinates.as_slice() else {
                return None;
            };
            Some((
                *character,
                JisCell {
                    plane: *plane,
                    row: *row,
                    cell: *cell,
                },
            ))
        })
        .collect()
});

/// Return the JIS X 0213 cell for a single Unicode scalar.
#[must_use]
pub fn jis_cell(character: char) -> Option<JisCell> {
    CHAR_TO_JIS_CELL.get(&character).copied()
}

impl Resolved {
    /// Write the resolved character or characters into a [`fmt::Write`].
    /// The renderer, hover and inlay-hint paths call this.
    ///
    /// # Errors
    /// Propagates the writer's own errors verbatim.
    pub fn write_to<W: fmt::Write>(self, w: &mut W) -> fmt::Result {
        match self {
            Self::Char(c) => w.write_char(c),
            Self::Multi(s) => w.write_str(s),
            Self::Voiced { base, mark } => {
                w.write_char(base)?;
                w.write_char(mark)
            }
        }
    }

    /// Returns the resolved single `char` if and only if this is a
    /// [`Resolved::Char`]. Combo cells and voiced sequences return `None`.
    #[must_use]
    pub fn as_char(self) -> Option<char> {
        match self {
            Self::Char(c) => Some(c),
            Self::Multi(_) | Self::Voiced { .. } => None,
        }
    }

    /// Total UTF-8 length of the resolved value (1..=8 bytes in
    /// practice).
    #[must_use]
    pub fn utf8_len(self) -> usize {
        match self {
            Self::Char(c) => c.len_utf8(),
            Self::Multi(s) => s.len(),
            Self::Voiced { base, mark } => base.len_utf8() + mark.len_utf8(),
        }
    }
}

/// Which tier of the lookup order supplied a [`Resolved`] value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappingSource {
    /// A JIS or `U+` reference, the 外字注記辞書 and its curated
    /// supplements, a composition rule, or the description's own single
    /// character.
    Standard,
    /// The cjkvi-data tables of Aozora notations, `data/aozora-gaiji-cjkvi.tsv`
    /// and `data/aozora-gaiji-cjkvi-ivs.tsv`.
    CjkviData,
    /// The base code point from the standard tier, the variation selector
    /// from `data/aozora-gaiji-cjkvi-selectors.tsv`.
    CjkviSelector,
}

/// Whether `text` is a base character followed by one ideographic variation
/// selector (U+E0100..=U+E01EF).
#[must_use]
pub fn is_variation_sequence(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(
        (chars.next(), chars.next(), chars.next()),
        (Some(_), Some('\u{E0100}'..='\u{E01EF}'), None)
    )
}

/// `resolved` refined to the selector sequence of `description` when the
/// sequence starts with the same character.
fn with_selector(description: &str, resolved: Resolved) -> (Resolved, MappingSource) {
    if let Resolved::Char(base) = resolved
        && let Some(&sequence) = DESCRIPTION_TO_SELECTOR.get(description)
        && sequence.starts_with(base)
    {
        return (Resolved::Multi(sequence), MappingSource::CjkviSelector);
    }
    (resolved, MappingSource::Standard)
}

/// Pure-function lookup used by `aozora-pipeline`'s classify stage
/// to populate the gaiji node's `ucs` field at construction time.
///
/// When `existing` is `Some`, it is returned without a table lookup;
/// callers that already extracted a codepoint from the source pass it
/// there. Pass `None` to consult the tables.
#[must_use]
pub fn lookup(
    existing: Option<char>,
    mencode: Option<&str>,
    description: &str,
) -> Option<Resolved> {
    lookup_with_source(existing, mencode, description).map(|(resolved, _)| resolved)
}

/// [`lookup`] together with the tier that supplied the value.
#[must_use]
pub fn lookup_with_source(
    existing: Option<char>,
    mencode: Option<&str>,
    description: &str,
) -> Option<(Resolved, MappingSource)> {
    if let Some(ch) = existing {
        return Some((Resolved::Char(ch), MappingSource::Standard));
    }
    let mut reference = None;
    for clause in mencode.into_iter().flat_map(reference_clauses).chain(
        description
            .rsplit('、')
            .map(str::trim)
            .take_while(|clause| {
                clause.is_empty()
                    || clause.starts_with("読みは「")
                    || resolve_reference(clause).is_some()
            }),
    ) {
        if let Some(resolved) = resolve_reference(clause) {
            if reference.is_some_and(|previous| previous != resolved) {
                return None;
            }
            reference = Some(resolved);
        }
    }
    if let Some(resolved) = reference {
        return Some(with_selector(description, resolved));
    }
    if let Some(&ch) = DESCRIPTION_TO_CHAR.get(description) {
        return Some(with_selector(description, Resolved::Char(ch)));
    }
    // Bare `ローマ数字N` is composed from the U+2160 block: the N≥13
    // forms have no single JIS cell and no entry in the description table.
    if let Some(s) = roman_numeral_glyph(description) {
        return Some((Resolved::Multi(s), MappingSource::Standard));
    }
    if let Some(voiced) = voiced_glyph(description) {
        return Some((voiced, MappingSource::Standard));
    }
    if let Some(&ch) = DESCRIPTION_TO_CHAR_CJKVI.get(description) {
        return Some((Resolved::Char(ch), MappingSource::CjkviData));
    }
    if let Some(&s) = DESCRIPTION_TO_STR_CJKVI.get(description) {
        return Some((Resolved::Multi(s), MappingSource::CjkviData));
    }
    // A description that is itself a single character resolves to that
    // character. The corpora contain this form where the transcriber could
    // type the kanji (e.g. on a modern IME) but wrote a `※[#…]` annotation
    // referring to the JIS source. The mencode and description-table
    // lookups above return first whenever a table has an entry, so this
    // branch applies only to a one-character description with no other
    // match.
    //
    // The count is of Unicode scalars, not grapheme clusters: a
    // base-plus-combining sequence (e.g. アクセント分解) has more than one
    // char and reaches the final `None`. `&str` cannot contain surrogate
    // halves, so a single `char` is unambiguous.
    let mut chars = description.chars();
    if let Some(only) = chars.next()
        && chars.next().is_none()
    {
        return Some((Resolved::Char(only), MappingSource::Standard));
    }
    None
}

/// Separators occurring between descriptions and men-ku-ten references in
/// gaiji markers.
///
/// Fullwidth ideographic comma (、) appears across almost all markers in the
/// archive. One marker in `002587_000372` contains the halfwidth form (､),
/// which functions identically as a clause boundary rather than description
/// text.
const CLAUSE_SEPARATORS: [char; 2] = ['、', '､'];

/// Split gaiji reference clauses without splitting commas inside quoted descriptions.
/// Glyph resolution and embedded documentary annotations share these boundaries.
pub fn reference_clauses(text: &str) -> impl Iterator<Item = &str> {
    let mut depth = 0_u32;
    text.split(move |ch| match ch {
        '「' => {
            depth += 1;
            false
        }
        '」' => {
            depth = depth.saturating_sub(1);
            false
        }
        ch if CLAUSE_SEPARATORS.contains(&ch) => depth == 0,
        _ => false,
    })
}

/// A `第N水準` token with any spacing between the level and its men-ku-ten
/// coordinates removed.
///
/// `060590_001938` contains `第4水準 2-13-28`; every table key and every
/// other work has `第4水準2-13-28`. The clause is delimited by `、`, so
/// removing a space inside it merges no distinct keys. The removal applies
/// only to the lookup key: serialization reproduces the source token
/// unchanged, because the token remains `Unresolved` rather than a
/// structured `MenKuTen`, whose `Display` would omit the space.
fn without_level_spacing(token: &str) -> Cow<'_, str> {
    let Some(after) = token.strip_prefix('第') else {
        return Cow::Borrowed(token);
    };
    let Some(suijun) = after.find("水準") else {
        return Cow::Borrowed(token);
    };
    let (level, tail) = after.split_at(suijun + "水準".len());
    let coordinates = tail.trim_start();
    if coordinates.len() == tail.len() {
        Cow::Borrowed(token)
    } else {
        Cow::Owned(format!("第{level}{coordinates}"))
    }
}

fn normalized_level_prefix(token: &str) -> Cow<'_, str> {
    for (source, level) in [("第三水準", '3'), ("第四水準", '4')] {
        if let Some(coordinates) = token.strip_prefix(source) {
            return Cow::Owned(format!("第{level}水準{coordinates}"));
        }
    }
    Cow::Borrowed(token)
}

fn resolve_reference(clause: &str) -> Option<Resolved> {
    let clause = clause.trim();
    let clause = clause.strip_suffix('」').unwrap_or(clause);
    let clause = clause.strip_prefix("面区点番号").unwrap_or(clause);
    let normalized: Cow<'_, str> = if clause.chars().any(|ch| ('０'..='９').contains(&ch)) {
        Cow::Owned(
            clause
                .chars()
                .map(|ch| {
                    if ('０'..='９').contains(&ch) {
                        char::from_u32(u32::from(ch) - u32::from('０') + u32::from('0'))
                            .expect("fullwidth digit maps to ASCII digit")
                    } else {
                        ch
                    }
                })
                .collect(),
        )
    } else {
        Cow::Borrowed(clause)
    };
    let normalized_level = normalized_level_prefix(&normalized);
    let prefixed = normalized_level
        .strip_prefix("1-")
        .map(|_| format!("第3水準{normalized_level}"))
        .or_else(|| {
            normalized_level
                .strip_prefix("2-")
                .map(|_| format!("第4水準{normalized_level}"))
        });
    let key = without_level_spacing(prefixed.as_deref().unwrap_or(&normalized_level));
    let key = key.as_ref();
    JISX0213_MENCODE_TO_STR
        .get(key)
        .copied()
        .map(Resolved::Multi)
        .or_else(|| {
            JISX0213_MENCODE_TO_CHAR
                .get(key)
                .copied()
                .map(Resolved::Char)
        })
        .or_else(|| parse_u_plus(key).map(Resolved::Char))
}

/// Compose the Unicode roman numeral for a bare `ローマ数字N` /
/// `ローマ数字N小文字` gaiji description, or `None` if the
/// description is not that shape or `N` is outside the composed range.
///
/// `N` accepts ASCII or full-width digits. The numeral is read from the
/// build-time [`ROMAN_NUMERAL_UPPER`] / [`ROMAN_NUMERAL_LOWER`] tables,
/// which spell `N` with the U+2160 / U+2170 single-letter blocks (so
/// `17` → `ⅩⅤⅠⅠ`). `N = 0` and out-of-range `N` resolve to `None` —
/// the gaiji then stays unresolved rather than rendering an empty glyph.
#[must_use]
fn roman_numeral_glyph(description: &str) -> Option<&'static str> {
    let rest = description.strip_prefix("ローマ数字")?;
    let (digits, lower) = rest
        .strip_suffix("小文字")
        .map_or((rest, false), |d| (d, true));
    if digits.is_empty() {
        return None;
    }
    let mut n: usize = 0;
    for ch in digits.chars() {
        let d = ch
            .to_digit(10)
            .or_else(|| ('０'..='９').contains(&ch).then(|| ch as u32 - '０' as u32))?;
        n = n.checked_mul(10)?.checked_add(d as usize)?;
    }
    let table = if lower {
        &ROMAN_NUMERAL_LOWER
    } else {
        &ROMAN_NUMERAL_UPPER
    };
    table.get(n).copied().filter(|s| !s.is_empty())
}

/// The small kana Unicode encodes, each with its regular form, so that a
/// `小書き` description names the small code point rather than a size.
const SMALL_KANA: &[(char, char)] = &[
    ('あ', 'ぁ'),
    ('い', 'ぃ'),
    ('う', 'ぅ'),
    ('え', 'ぇ'),
    ('お', 'ぉ'),
    ('つ', 'っ'),
    ('や', 'ゃ'),
    ('ゆ', 'ゅ'),
    ('よ', 'ょ'),
    ('わ', 'ゎ'),
    ('か', 'ゕ'),
    ('け', 'ゖ'),
    ('ゐ', '\u{1B150}'),
    ('ゑ', '\u{1B151}'),
    ('を', '\u{1B152}'),
    ('ア', 'ァ'),
    ('イ', 'ィ'),
    ('ウ', 'ゥ'),
    ('エ', 'ェ'),
    ('オ', 'ォ'),
    ('ツ', 'ッ'),
    ('ヤ', 'ャ'),
    ('ユ', 'ュ'),
    ('ヨ', 'ョ'),
    ('ワ', 'ヮ'),
    ('カ', 'ヵ'),
    ('ケ', 'ヶ'),
    ('ク', 'ㇰ'),
    ('シ', 'ㇱ'),
    ('ス', 'ㇲ'),
    ('ト', 'ㇳ'),
    ('ヌ', 'ㇴ'),
    ('ハ', 'ㇵ'),
    ('ヒ', 'ㇶ'),
    ('フ', 'ㇷ'),
    ('ヘ', 'ㇸ'),
    ('ホ', 'ㇹ'),
    ('ム', 'ㇺ'),
    ('ラ', 'ㇻ'),
    ('リ', 'ㇼ'),
    ('ル', 'ㇽ'),
    ('レ', 'ㇾ'),
    ('ロ', 'ㇿ'),
    ('ヰ', '\u{1B164}'),
    ('ヱ', '\u{1B165}'),
    ('ヲ', '\u{1B166}'),
    ('ン', '\u{1B167}'),
];

/// Strip one leading `小書き`, reporting whether it was there.
fn strip_small(text: &str) -> (&str, bool) {
    text.strip_prefix("小書き")
        .map_or((text, false), |rest| (rest, true))
}

/// Composes the voiced form designated by a `濁点付き…` or `半濁点付き…`
/// description when no precomposed Unicode character exists: the base
/// followed by U+3099 or U+309A, defined in the Unicode Standard as
/// the representation of a voicing mark on any base (chapter 18.3,
/// Hiragana and Katakana).
///
/// The description pattern attested across the corpus is `[小書き](半)濁点付き[の]
/// [小書き][片仮名|平仮名]<base>`, where `<base>` is a single character or
/// `二の字点` (〻). When a script is specified, base characters outside that
/// script do not match; `小書き` selects the corresponding small code point
/// from [`SMALL_KANA`] when available. Because the description table is
/// consulted first during lookup, voiced forms precomposed in the
/// 外字注記辞書 or JIS X 0213 (such as ヷ, ゔ, か゚) do not reach this rule.
fn voiced_glyph(description: &str) -> Option<Resolved> {
    let (rest, small_before) = strip_small(description);
    let (rest, mark) = if let Some(rest) = rest.strip_prefix("半濁点付き") {
        (rest, '\u{309A}')
    } else {
        (rest.strip_prefix("濁点付き")?, '\u{3099}')
    };
    let rest = rest.strip_prefix('の').unwrap_or(rest);
    let (rest, small_after) = strip_small(rest);
    let (rest, script) = [("片仮名", 'ア'..='ヺ'), ("平仮名", 'ぁ'..='ゖ')]
        .into_iter()
        .find_map(|(name, range)| rest.strip_prefix(name).map(|rest| (rest, Some(range))))
        .unwrap_or((rest, None));
    let base = if rest == "二の字点" {
        '\u{303B}'
    } else {
        let mut chars = rest.chars();
        let only = chars.next()?;
        if chars.next().is_some() {
            return None;
        }
        only
    };
    if script.is_some_and(|range| !range.contains(&base)) {
        return None;
    }
    let base = if small_before || small_after {
        SMALL_KANA
            .iter()
            .find(|(regular, _)| *regular == base)
            .map(|(_, small)| *small)?
    } else {
        base
    };
    Some(Resolved::Voiced { base, mark })
}

/// Parse a `U+XXXX` style mencode — 1 to 6 hex digits after the
/// literal `U+` prefix — and validate the result via
/// [`char::from_u32`]. Returns `None` for surrogates, non-characters,
/// and out-of-range integers, rather than panicking, so that [`lookup`]
/// continues to the description fallback on malformed input.
#[must_use]
fn parse_u_plus(mencode: &str) -> Option<char> {
    let hex = mencode.strip_prefix("U+")?;
    // An empty or longer-than-six-digit value is rejected:
    // `u32::from_str_radix` accepts 10-digit inputs, which cannot be a
    // Unicode scalar.
    if hex.is_empty() || hex.len() > 6 {
        return None;
    }
    let code = u32::from_str_radix(hex, 16).ok()?;
    char::from_u32(code)
}

// Gaiji descriptions (the text inside `「…」`) that resolve to a
// canonical character without depending on the mencode tail. Sourced
// from `ab-validator/crates/ab-aozora-encoding/data/aozora-gaiji-chuki.tsv`
// (the official 8th-edition 外字注記辞書) plus
// `aozora-gaiji-special.tsv` (hand-curated 〓 / 〻 placeholders).
// `build.rs` generates the table as
// `crate::jisx0213_table::DESCRIPTION_TO_CHAR` (imported at the top of
// this module), descriptions mapped exclusively in `aozora-gaiji-cjkvi.tsv`
// as `DESCRIPTION_TO_CHAR_CJKVI`, and variation sequences from
// `aozora-gaiji-cjkvi-ivs.tsv` as `DESCRIPTION_TO_STR_CJKVI`. The three key
// sets are mutually disjoint.
// `aozora-gaiji-cjkvi-selectors.tsv` becomes `DESCRIPTION_TO_SELECTOR`,
// keyed by official-table descriptions only.

/// Entry counts of the compiled tables, for tests and diagnostics:
/// `(single_char_count, combo_count, description_count)`.
#[must_use]
pub fn table_sizes() -> (usize, usize, usize) {
    (
        JISX0213_MENCODE_TO_CHAR.len(),
        JISX0213_MENCODE_TO_STR.len(),
        DESCRIPTION_TO_CHAR.len(),
    )
}

/// External identity for a gaiji unencoded in Unicode: its Ideographic
/// Description Sequence and corresponding `GlyphWiki` glyph, from
/// `data/aozora-gaiji-glyphwiki.tsv`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternalIdentity {
    /// Ideographic Description Sequence (Unicode chapter 18.2).
    pub ids: &'static str,
    /// `GlyphWiki` glyph name, accessible at
    /// `https://glyphwiki.org/glyph/<name>.svg`.
    pub glyphwiki: &'static str,
}

/// External identity recorded for `description`, keyed by the string
/// derived by [`parse_gaiji_body`] from the marker body.
#[must_use]
pub fn external_identity(description: &str) -> Option<ExternalIdentity> {
    DESCRIPTION_TO_EXTERNAL
        .get(description)
        .map(|&(ids, glyphwiki)| ExternalIdentity { ids, glyphwiki })
}

// The functions below are the only code that extracts `※［＃…］`
// references from raw source, for the full document or around a cursor,
// and resolves each to its glyph. Editor surfaces (`aozora-wasm` inlay
// hints and cursor hover) and batch callers (`aozora-py`
// `gaiji_resolutions`) call them; the `ab_aozora_facade::json` projection
// only serialises the [`GaijiResolution`] values produced here. They are
// in the same module as [`lookup`], whose tables the scan consults.

/// Opening delimiter of a *refmark* gaiji reference (`※［＃`).
pub const GAIJI_OPEN: &str = "※［＃";
/// The bracket-hash annotation opener (`［＃`).
///
/// Shared by the refmark form (`※` + this) and the standalone
/// external-character form, which carries no `※`. The standalone form
/// needs the [`recognize_gaiji_body`] test to distinguish it from a plain
/// directive (`［＃改ページ］`).
pub const BRACKET_HASH: &str = "［＃";
/// The refmark prefix (`※`) that distinguishes a refmark gaiji from the
/// standalone form.
pub const GAIJI_REFMARK: &str = "※";
/// Closing delimiter of a gaiji reference (`］`).
pub const GAIJI_CLOSE: &str = "］";
/// Window half-width (bytes) for the cursor-local [`find_span`] scan. A
/// `※［＃…］` span in the corpus is at most a few hundred bytes; the cap
/// makes per-cursor resolution O(window) rather than O(doc).
pub const MAX_GAIJI_SPAN_LEN: usize = 512;

/// One resolved gaiji reference located in source.
///
/// Byte offsets index the source string. `mencode` / `codepoint` /
/// `resolved` are `None` when absent or unresolved; `codepoint` is also
/// `None` for combining-sequence cells (which have no single scalar).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GaijiResolution {
    /// Start byte offset of the `※［＃…］` span (inclusive).
    pub start: usize,
    /// End byte offset of the span (exclusive).
    pub end: usize,
    /// The `「…」` description text (or the whole body if no quotes).
    pub description: String,
    /// The mencode tail (`第3水準…` / `U+XXXX`), if present.
    pub mencode: Option<String>,
    /// Resolved codepoint as `u32`, when resolution is a single scalar.
    pub codepoint: Option<u32>,
    /// Resolved glyph(s), when [`lookup`] succeeds.
    pub resolved: Option<String>,
}

/// Split a gaiji body into `(description, mencode?)`. The parser's
/// recogniser, the LSP resolution view, and [`resolve_at`] (the `gaiji()`
/// wire projection) all use this split.
///
/// Two body shapes occur (Aozora annotation manual + corpus §6):
///
/// * **Simple quoted** — `「desc」` optionally followed by `、mencode`,
///   where `desc` has no nested `「」` and nothing but a `、mencode`
///   tail follows. The surrounding `「」` is stripped (the serializer
///   re-adds it).
/// * **Composed / bare** — everything else: the composed-glyph / 正字 /
///   屋号 forms (`「X」の「Y」に代えて「Z」、mencode`) and the quote-less
///   `二の字点、1-2-23`. The body is split on `、` and scanned from the
///   right: the maximal trailing run of mencode-shaped tokens
///   ([`is_mencode_shaped`] / [`is_page_line_shaped`]) is the `mencode`;
///   everything before it is the `description`, kept verbatim (internal
///   `「」` and `、` preserved). The right-to-left scan does not split at a
///   `、` that belongs to the description (`…面から一、二画目をとったもの`),
///   which a split at the first `、` would do on composed forms.
///
/// Returns borrowed slices of `body`. A body with no trailing
/// mencode-shaped run (or none before it) yields the whole trimmed body
/// as the description and `None` mencode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GaijiBody<'a> {
    /// The description text — quotes stripped for the simple quoted form,
    /// kept verbatim for the composed / bare forms.
    pub description: &'a str,
    /// The trailing mencode reference (`第3水準…` / `U+XXXX`), if present.
    pub mencode: Option<&'a str>,
    /// `true` for the simple `「desc」` quoted form (a valid gaiji even
    /// without a mencode); `false` for the composed / bare forms, which
    /// require a trailing mencode anchor for parsing.
    pub quoted: bool,
}

/// See the type-level docs on [`GaijiBody`].
#[must_use]
pub fn parse_gaiji_body(body: &str) -> GaijiBody<'_> {
    let body = body.trim();
    // Simple quoted form: 「desc」 with no nested quotes, tail empty or 、mencode.
    if let Some(rest) = body.strip_prefix('「')
        && let Some(close) = rest.find('」')
    {
        let desc = &rest[..close];
        let tail = rest[close + '」'.len_utf8()..].trim();
        // A simple quoted gaiji has an empty tail, a `、mencode` tail, or
        // (shape 2) a mencode-shaped tail fused directly after `」` with no
        // separating `、` (`「金＋夫」第3水準1-93-4`). The bare tail is accepted
        // only when it is mencode-shaped, so a non-mencode tail (e.g. a `に傍点`
        // forward directive) reaches the composed scan below.
        let bare_mencode_tail =
            !tail.is_empty() && is_mencode_shaped(mencode_resolution_token(tail));
        if !desc.is_empty()
            && !desc.contains(['「', '」'])
            && (tail.is_empty() || tail.starts_with(CLAUSE_SEPARATORS) || bare_mencode_tail)
        {
            let mencode = tail.strip_prefix(CLAUSE_SEPARATORS).map_or_else(
                || (!tail.is_empty()).then_some(tail),
                |m| {
                    let m = m.trim();
                    (!m.is_empty()).then_some(m)
                },
            );
            return GaijiBody {
                description: desc,
                mencode,
                quoted: true,
            };
        }
    }
    // A whole-glyph code establishes the reference independently of locator wording.
    // Quote-aware clauses exclude codes that describe only a glyph component.
    if let Some(reference) = reference_clauses(body).find(|clause| is_mencode_shaped(clause.trim()))
    {
        let start = reference.as_ptr().addr() - body.as_ptr().addr();
        let description = body[..start].trim_end_matches(['、', ' ']).trim();
        if !description.is_empty() {
            return GaijiBody {
                description,
                mencode: Some(body[start..].trim()),
                quoted: false,
            };
        }
    }
    // Composed / bare form: right-to-left mencode scan. The run admits both
    // the canonical page-line forms and the near-miss ones (fused 上/中/下,
    // full-width minus, poetry locators).
    let shaped = |t: &str| {
        is_mencode_shaped(t) || is_near_miss_page_line_shaped(t) || is_page_segment_line_locator(t)
    };
    let commas: Vec<(usize, usize)> = body
        .match_indices(CLAUSE_SEPARATORS)
        .map(|(i, separator)| (i, separator.len()))
        .collect();
    let tokens: Vec<&str> = body.split(CLAUSE_SEPARATORS).map(str::trim).collect();
    let mut run_start = tokens.len();
    while run_start > 0 && shaped(tokens[run_start - 1]) {
        run_start -= 1;
    }
    // False-positive guard: a near-miss-only page-line token (one the
    // canonical form rejects) is admitted only when the run also contains
    // a mencode token (第N水準 / U+ / bare N-N-N). This condition excludes
    // a proofreader / 段組 directive tail (`、58-下15`) from gaiji
    // recognition, while the canonical description-anchored forms
    // (`小書き片仮名ン、500-下-19`) resolve without a men-ku-ten.
    let run = &tokens[run_start..];
    // A locator outside the canonical page-line grammar: a fused 上/中/下 or
    // poetry form, or a page-segment-line triple whose middle segment is a
    // column or a region. None of them resolves a character on its own.
    let uses_unresolvable_locator = run.iter().any(|t| {
        (is_near_miss_page_line_shaped(t) || is_page_segment_line_locator(t))
            && !is_page_line_shaped(t)
    });
    let anchored = run.iter().any(|t| is_mencode_shaped(t));
    // A locator that only identifies a place in the 底本 resolves no
    // character, so the description before it must identify the glyph.
    // This condition alone distinguishes a gaiji from a proofreader's note
    // ending in the same page reference.
    let described_glyph_locator = run_start > 0
        && run_start < tokens.len()
        && run.iter().all(|t| is_page_segment_line_locator(t))
        && names_a_known_glyph(body[..commas[run_start - 1].0].trim());
    // With no separator-delimited run, check for the fused form: a
    // men-ku-ten written directly after the description's closing 」. The
    // simple quoted branch above accepts that shape only for a description
    // without nested 「」; this branch covers the nested case
    // (`004643_000284`).
    if run_start == tokens.len()
        && let Some(end) = fused_mencode_boundary(body)
    {
        return GaijiBody {
            description: body[..end].trim(),
            mencode: Some(body[end..].trim()),
            quoted: false,
        };
    }
    if run_start == tokens.len()
        || run_start == 0
        || (uses_unresolvable_locator && !anchored && !described_glyph_locator)
    {
        return GaijiBody {
            description: body,
            mencode: None,
            quoted: false,
        };
    }
    let (boundary, separator_len) = commas[run_start - 1];
    GaijiBody {
        description: body[..boundary].trim(),
        mencode: Some(body[boundary + separator_len..].trim()),
        quoted: false,
    }
}

/// Byte offset just past the description's outermost closing 」, when what
/// follows it is a men-ku-ten and nothing else.
///
/// `004643_000284` contains `「…「炎」」第4水準2-84-80` once and the same
/// glyph with a separating 、 three times, so the missing separator is a
/// transcription error rather than a second convention. The tail must be
/// mencode-shaped, which excludes a directive that follows a quote
/// (`」に傍点`).
fn fused_mencode_boundary(body: &str) -> Option<usize> {
    let mut depth = 0_u32;
    let mut outermost_close = None;
    for (index, ch) in body.char_indices() {
        match ch {
            '「' => depth += 1,
            '」' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    outermost_close = Some(index + ch.len_utf8());
                }
            }
            _ => {}
        }
    }
    let end = outermost_close?;
    let tail = body[end..].trim();
    (!tail.is_empty() && is_mencode_shaped(tail)).then_some(end)
}

/// A complete source-stated component substitution describes a glyph even
/// when the source supplies no character code or page reference.
fn is_substitution_description(description: &str) -> bool {
    fn quoted(input: &str) -> Option<&str> {
        if let Some(content) = input.strip_prefix("「※［＃") {
            let hash_open = '「'.len_utf8() + '※'.len_utf8();
            let end = gaiji_marker_end(input, hash_open)?;
            if content.starts_with('］') {
                return None;
            }
            return input[end..].trim_start().strip_prefix('」');
        }
        let (content, rest) = input.strip_prefix('「')?.split_once('」')?;
        (!content.is_empty() && !content.contains(['「', '［', '］'])).then_some(rest)
    }
    let replacement = quoted(description)
        .and_then(|rest| rest.strip_prefix("の"))
        .and_then(quoted)
        .and_then(|rest| rest.strip_prefix("に代えて"))
        .and_then(quoted)
        .is_some_and(str::is_empty);
    let component_pair = description
        .strip_prefix("偏が")
        .and_then(quoted)
        .and_then(|rest| rest.strip_prefix("で旁が"))
        .and_then(quoted)
        .is_some_and(str::is_empty);
    replacement || component_pair
}

/// Whether a description identifies a glyph without a character code or a
/// resolvable page reference.
///
/// This test distinguishes a gaiji from a proofreader's remark when both end
/// in the same page reference: `小書き片仮名ヱ、28-上段-1` identifies a
/// character and `底本ルビは「もら」と誤記、175-上段-4` is a statement about
/// the 底本.
fn names_a_known_glyph(description: &str) -> bool {
    DESCRIPTION_TO_CHAR.contains_key(description)
        || roman_numeral_glyph(description).is_some()
        || is_substitution_description(description)
}

/// A `page-segment-line` reference into the 底本, where the middle segment
/// is something other than a column number: 本文, 左, 右, a kanji column,
/// a 段 register. The page may be a `コマ` frame of a scanned edition.
///
/// Shape alone does not make one of these a gaiji reference: a proofreader's
/// note uses the same triple to refer to its line.
/// [`names_a_known_glyph`] decides admission from the description before it.
fn is_page_segment_line_locator(value: &str) -> bool {
    let mut parts = value.split('-');
    matches!((parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(page), Some(segment), Some(line), None)
            if is_digit_run(page.strip_prefix("コマ").unwrap_or(page))
                && !segment.is_empty()
                && is_digit_run(line))
}

/// Whether a gaiji `description` can be kept (it both serializes and
/// round-trips); otherwise the bracket is read as a plain directive.
///
/// Rejects:
///   - an incomplete nested glyph reference, or a nested annotation without
///     its supplied glyph marker, and
///   - a description carrying structural `「…」` quotes without a trailing
///     `、mencode` anchor or a complete component-substitution statement
///     (these distinguish glyph descriptions from text quotations). Anchored forms are
///     kept verbatim, including editor quote typos with an orphan 「 or 」
///     (`くさかんむり／（月＋曷）」、第3水準…`) — so serialization reproduces
///     the source bytes and the reference stays resolvable.
#[must_use]
pub fn gaiji_description_serializable(description: &str, has_mencode: bool) -> bool {
    if !nested_references_complete(description) {
        return false;
    }
    if description.contains(['「', '」']) {
        return has_mencode || is_substitution_description(description);
    }
    true
}

/// Whether a description in free wording can be kept on a refmark gaiji.
///
/// The serializer writes a description that contains `「` or `」` verbatim
/// and quotes any other, so only the former reproduces the source bytes.
#[must_use]
pub fn gaiji_statement_serializable(description: &str) -> bool {
    nested_references_complete(description) && description.contains(['「', '」'])
}

/// Whether a refmark description states what the `※` before it stands for.
///
/// In `「※」は小書きの「ト」` the statement's subject is the refmark, so the
/// annotation describes that glyph regardless of its other content.
#[must_use]
pub fn describes_refmark(description: &str) -> bool {
    description
        .strip_prefix("「※」は")
        .is_some_and(|statement| !statement.trim_start_matches('、').trim().is_empty())
}

/// Whether every annotation nested in `description` is a complete glyph reference.
fn nested_references_complete(description: &str) -> bool {
    description.match_indices(BRACKET_HASH).all(|(start, _)| {
        description[..start].ends_with(GAIJI_REFMARK)
            && gaiji_marker_end(description, start).is_some()
    })
}

/// Parse a `［＃…］` body and decide whether it is a recognised gaiji reference.
///
/// The parser's recogniser, the standalone scan in [`gaiji_resolutions`] /
/// [`resolve_at`], and the LSP / wire resolution view all apply this test.
///
/// Returns the parsed [`GaijiBody`] iff the body is a gaiji; `None` for a
/// plain directive (e.g. `改ページ`). A gaiji requires a non-empty description and
/// either the simple `「…」`-quoted form, a trailing mencode anchor, *or* a bare
/// description that is a complete component substitution or known dictionary
/// entry (for example `※［＃二重かっこ開く］`); the description must round-trip
/// ([`gaiji_description_serializable`]).
#[must_use]
pub fn recognize_gaiji_body(body: &str) -> Option<GaijiBody<'_>> {
    let parsed = parse_gaiji_body(body);
    // Unanchored descriptions require an explicit glyph construction or a known
    // glyph name; arbitrary quoted directives remain outside this contract.
    let bare_unanchored = !parsed.quoted && parsed.mencode.is_none();
    if parsed.description.is_empty()
        || (bare_unanchored && !names_a_known_glyph(parsed.description))
        || !gaiji_description_serializable(parsed.description, parsed.mencode.is_some())
    {
        return None;
    }
    Some(parsed)
}

/// First reference-tail clause, used to recognize a mencode fused to a
/// quoted description. Resolution itself examines the complete tail so
/// reading notes and conflicting identifiers remain visible.
#[must_use]
pub fn mencode_resolution_token(mencode: &str) -> &str {
    mencode
        .split_once(CLAUSE_SEPARATORS)
        .map_or(mencode, |(token, _)| token.trim())
}

/// A structured JIS X 0213 面区点 (men-ku-ten) reference — the clean
/// `第N水準P-K-T` mencode form parsed into its components.
///
/// 水準 (level) is redundant with the plane (第3水準 = plane 1,
/// 第4水準 = plane 2), so only the plane is stored; [`Self::level`]
/// recovers it and [`fmt::Display`] reproduces the exact source form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenKuTen {
    /// Plane (面): 1 (第3水準) or 2 (第4水準).
    pub plane: u8,
    /// Row (区), 1..=94.
    pub ku: u8,
    /// Cell (点), 1..=94.
    pub ten: u8,
}

impl MenKuTen {
    /// JIS 水準 level: plane 1 → 3, plane 2 → 4.
    #[must_use]
    pub fn level(self) -> u8 {
        self.plane + 2
    }
}

impl fmt::Display for MenKuTen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "第{}水準{}-{}-{}",
            self.level(),
            self.plane,
            self.ku,
            self.ten
        )
    }
}

/// Parse the clean `第N水準P-K-T` men-ku-ten form.
///
/// Returns `None` for anything else — bare `P-K-T`, page-line-suffixed
/// tails, leading-zero or inconsistent-level shapes — which the caller
/// keeps verbatim as [`GaijiCanonical::Unresolved`] so serialization
/// stays byte-exact for those.
#[must_use]
pub fn parse_menkuten(token: &str) -> Option<MenKuTen> {
    let after = token.strip_prefix('第')?;
    let suijun = after.find("水準")?;
    let level: u8 = after[..suijun].parse().ok()?;
    let mut parts = after[suijun + "水準".len()..].split('-');
    let plane: u8 = parts.next()?.parse().ok()?;
    let ku: u8 = parts.next()?.parse().ok()?;
    let ten: u8 = parts.next()?.parse().ok()?;
    // Exactly three components, a level consistent with the plane, and
    // no zero coordinate. The round-trip via `Display` is only exact
    // for the canonical shape, so reject everything else.
    if parts.next().is_some() || level != plane + 2 || plane == 0 || ku == 0 || ten == 0 {
        return None;
    }
    let mkt = MenKuTen { plane, ku, ten };
    // A leading-zero or otherwise non-canonical source would not reproduce
    // byte-exactly through `Display`, so `token` must already be in
    // canonical form.
    (mkt.to_string() == token).then_some(mkt)
}

/// The typed canonical value of a gaiji reference.
///
/// `Gaiji` stores this value rather than a `(ucs, mencode)` pair: the
/// resolved glyph is derived on demand via [`Self::resolve`] and the
/// source mencode is reproduced via [`Self::write_mencode`], so each fact
/// has one representation, and the variant records the mencode shape that
/// `is_mencode_shaped` and the serializability checks would otherwise
/// test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GaijiCanonical<'src> {
    /// Structured `第N水準P-K-T` reference (clean form only).
    MenKuTen(MenKuTen),
    /// Explicit `U+XXXX` codepoint (clean form, no suffix).
    Unicode(char),
    /// Everything kept verbatim: bare / suffixed men-ku-ten, page-line
    /// references, description-only gaiji. `None` when the source had no
    /// mencode tail at all.
    Unresolved {
        /// The raw mencode tail, reproduced unchanged on serialization.
        mencode: Option<&'src str>,
    },
}

impl<'src> GaijiCanonical<'src> {
    /// Classify a parsed mencode tail into its canonical form. Only the
    /// byte-exact `第N水準P-K-T` and `U+XXXX` shapes are structured; all
    /// else is retained verbatim as [`Self::Unresolved`].
    #[must_use]
    pub fn from_mencode(mencode: Option<&'src str>) -> Self {
        if let Some(m) = mencode {
            if let Some(mkt) = parse_menkuten(m) {
                return Self::MenKuTen(mkt);
            }
            if let Some(c) = parse_u_plus(m) {
                return Self::Unicode(c);
            }
        }
        Self::Unresolved { mencode }
    }

    /// Resolve to a glyph through [`lookup`], which consults the JIS
    /// tables and the `description` fallback.
    #[must_use]
    pub fn resolve(self, description: &str) -> Option<Resolved> {
        match self {
            Self::MenKuTen(m) => lookup(None, Some(&m.to_string()), description),
            Self::Unicode(c) => lookup(None, Some(&format!("U+{:X}", u32::from(c))), description),
            Self::Unresolved { mencode } => lookup(None, mencode, description),
        }
    }

    /// `true` when the source has a mencode tail; serialization writes the
    /// `、` separator only in this case.
    #[must_use]
    pub fn has_mencode(self) -> bool {
        !matches!(self, Self::Unresolved { mencode: None })
    }

    /// Write the canonical mencode token (without the leading `、`). The
    /// structured forms reproduce the clean `第N水準P-K-T` / `U+XXXX`
    /// shape; `Unresolved` writes the raw tail verbatim.
    ///
    /// # Errors
    /// Propagates the writer's own errors.
    pub fn write_mencode<W: fmt::Write>(self, w: &mut W) -> fmt::Result {
        match self {
            Self::MenKuTen(m) => write!(w, "{m}"),
            Self::Unicode(c) => write!(w, "U+{:04X}", c as u32),
            Self::Unresolved { mencode } => mencode.map_or(Ok(()), |m| w.write_str(m)),
        }
    }
}

/// Whether `s` is a JIS X 0213 men-ku-ten / `U+XXXX` mencode token:
/// `N-N-N`, `第N水準N-N-N`, or `U+XXXX` (1–6 ASCII hex digits).
#[must_use]
pub fn is_mencode_shaped(s: &str) -> bool {
    if let Some(hex) = s.strip_prefix("U+") {
        return !hex.is_empty() && hex.len() <= 6 && hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    // Optional `第N水準` prefix: skip past the digits + `水準` token if
    // present, then validate the remainder as `N-N-N`. The remainder is
    // trimmed for the same reason resolution trims it, so a token this
    // admits is a token resolution can key on.
    let normalized_level = normalized_level_prefix(s);
    let rest = normalized_level
        .strip_prefix('第')
        .and_then(|after_dai| {
            let nondigit = after_dai.find(|c: char| !c.is_ascii_digit())?;
            let (_digits, tail) = after_dai.split_at(nondigit);
            Some(tail.strip_prefix("水準")?.trim_start())
        })
        .unwrap_or(&normalized_level);
    !rest.is_empty()
        && rest.chars().all(|c| c.is_ascii_digit() || c == '-')
        && rest.chars().any(|c| c.is_ascii_digit())
}

/// Whether `s` is a canonical 底本ページ-行 reference — `-`-joined parts, each
/// a digit run, a 上 / 中 / 下 column marker, or a volume marker
/// (`144-上-9`, `372-10`, `7巻-42-下-10`, `（二）-27-3`).
///
/// The description-anchored gaiji depend on this strict form (a
/// `小書き片仮名ン、500-下-19` glyph has only this provenance tail and no
/// men-ku-ten). The private `is_near_miss_page_line_shaped`
/// (mencode-anchored) accepts the looser near-miss spellings.
#[must_use]
pub fn is_page_line_shaped(s: &str) -> bool {
    !s.is_empty() && s.split('-').all(is_page_line_part)
}

/// One `-`-separated component of a canonical 底本ページ-行 reference.
fn is_page_line_part(p: &str) -> bool {
    if let Some(volume) = p.strip_prefix('（').and_then(|s| s.strip_suffix('）')) {
        return !volume.is_empty()
            && volume.chars().all(|c| {
                matches!(
                    c,
                    '一' | '二' | '三' | '四' | '五' | '六' | '七' | '八' | '九' | '十'
                )
            });
    }
    if let Some(volume) = p.strip_suffix('巻') {
        return matches!(volume, "上" | "中" | "下" | "前" | "後") || is_digit_run(volume);
    }
    matches!(p, "上" | "中" | "下") || is_digit_run(p)
}

/// The 上 / 中 / 下 column markers plus their 段 register variants, longest
/// first so a `下段` prefix/suffix strips before the shorter `下`.
const COLUMN_MARKERS: [&str; 6] = ["上段", "中段", "下段", "上", "中", "下"];

/// Whether `s` is a *near-miss* 底本ページ-行 reference — a superset of
/// [`is_page_line_shaped`] that also accepts the corpus-attested near-miss
/// spellings: a full-width U+FF0D `－` separator (`94－11`), a 上/中/下(+段)
/// register fused directly to a digit run on either side (`下8`, `109下`,
/// `下段5`), and the poetry/register locator (`P61`, `下段5首目`).
///
/// The same spellings occur in proofreader / 段組 directive tails, so
/// [`parse_gaiji_body`] admits a near-miss-only token only when the run
/// also contains a mencode token. A bare 段 register (`上段`) is rejected
/// because it is a 段組 directive operand rather than a page-line component.
fn is_near_miss_page_line_shaped(s: &str) -> bool {
    is_page_line_shaped(s)
        || (!s.is_empty() && s.split(['-', '－']).all(is_near_miss_page_line_part))
}

/// One separator-separated component of a near-miss 底本ページ-行 reference.
fn is_near_miss_page_line_part(p: &str) -> bool {
    if let Some(volume) = p.strip_suffix('巻') {
        return matches!(volume, "上" | "中" | "下" | "前" | "後") || is_digit_run(volume);
    }
    // Poetry/register locator: an optional leading `P` page prefix and an
    // optional trailing `首目` line-counter wrap the digit / column core
    // (`P61`, `下段5首目`).
    let core = p.strip_prefix('P').unwrap_or(p);
    let core = core.strip_suffix("首目").unwrap_or(core);
    // A bare marker is accepted only from the canonical set; a bare 段
    // register (`上段`) is a 段組 operand, not a page-line part.
    if matches!(core, "上" | "中" | "下") {
        return true;
    }
    // Register marker fused before a digit run (`下8`, `下段5`) or after one
    // (`109下`, `109下段`).
    if let Some(rest) = COLUMN_MARKERS.iter().find_map(|m| core.strip_prefix(m)) {
        return is_digit_run(rest);
    }
    if let Some(rest) = COLUMN_MARKERS.iter().find_map(|m| core.strip_suffix(m)) {
        return is_digit_run(rest);
    }
    is_digit_run(core)
}

/// A non-empty run of ASCII or full-width decimal digits.
fn is_digit_run(p: &str) -> bool {
    !p.is_empty()
        && p.chars()
            .all(|c| c.is_ascii_digit() || ('０'..='９').contains(&c))
}

/// Resolve the gaiji span at `[start..end)` — either the refmark form
/// (`※［＃…］`) or the standalone form (`［＃…］`).
///
/// The refmark `※` is itself the gaiji marker, so a `※［＃…］` span is always
/// resolved. A standalone `［＃…］` must pass [`recognize_gaiji_body`] (else it
/// is a plain directive like `［＃改ページ］` and yields `None`). Also
/// returns `None` if the delimiters do not enclose a parseable body;
/// offsets from [`gaiji_resolutions`] / [`find_span`] always enclose one.
#[must_use]
pub fn resolve_at(source: &str, start: usize, end: usize) -> Option<GaijiResolution> {
    let span = source.get(start..end)?;
    let (open, standalone) = if span.starts_with(GAIJI_OPEN) {
        (GAIJI_OPEN, false)
    } else if span.starts_with(BRACKET_HASH) {
        (BRACKET_HASH, true)
    } else {
        return None;
    };
    let body_start = start.checked_add(open.len())?;
    let body_end = end.checked_sub(GAIJI_CLOSE.len())?;
    if body_end <= body_start || body_end > source.len() {
        return None;
    }
    let body = source.get(body_start..body_end)?;
    let GaijiBody {
        description,
        mencode,
        ..
    } = if standalone {
        recognize_gaiji_body(body)?
    } else {
        parse_gaiji_body(body)
    };
    let (resolved, codepoint) = lookup(None, mencode, description).map_or((None, None), |r| {
        let mut s = String::new();
        _ = r.write_to(&mut s);
        (Some(s), r.as_char().map(|c| c as u32))
    });
    Some(GaijiResolution {
        start,
        end,
        description: description.to_owned(),
        mencode: mencode.map(str::to_owned),
        codepoint,
        resolved,
    })
}

/// All gaiji references in `source`, resolved, in source order. Each
/// opener is scanned at most to the end of its line.
#[must_use]
pub fn gaiji_resolutions(source: &str) -> Vec<GaijiResolution> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    // Scan for the bracket-hash opener so both the refmark (`※［＃`) and
    // the standalone (`［＃`) forms are found; the span includes a
    // preceding `※`, and `resolve_at` applies recognition to the
    // standalone form.
    while let Some(rel) = source[cursor..].find(BRACKET_HASH) {
        let hash_open = cursor + rel;
        let span_start = if source[..hash_open].ends_with(GAIJI_REFMARK) {
            hash_open - GAIJI_REFMARK.len()
        } else {
            hash_open
        };
        // After an annotation that is not a reference, the scan resumes just
        // past its opener. After an unclosed opener, the scan resumes at the
        // next line, so the references after it are still found while the
        // components on its own line remain unrecognized.
        cursor = hash_open + BRACKET_HASH.len();
        let Some(span_end) = gaiji_marker_end(source, hash_open) else {
            cursor = source[cursor..]
                .find('\n')
                .map_or(source.len(), |newline| cursor + newline);
            continue;
        };
        if let Some(res) = resolve_at(source, span_start, span_end) {
            out.push(res);
            cursor = span_end;
        }
    }
    out
}

/// Locate a complete reference without ending at a nested component's closer.
/// A reference does not cross a line break.
fn gaiji_marker_end(source: &str, hash_open: usize) -> Option<usize> {
    let body_start = hash_open.checked_add(BRACKET_HASH.len())?;
    let mut depth = 1usize;
    for (relative, ch) in source.get(body_start..)?.char_indices() {
        let start = body_start + relative;
        match ch {
            '\n' | '\r' => return None,
            '［' if source[start..].starts_with(BRACKET_HASH) => depth += 1,
            '］' => {
                // A quoted delimiter is the supplied glyph, not a boundary.
                if source[..start].ends_with('「')
                    && source[start + ch.len_utf8()..].starts_with('」')
                {
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    return Some(start + ch.len_utf8());
                }
            }
            _ => {}
        }
    }
    None
}

/// Byte-range of the `※［＃…］` span containing `byte_offset`.
///
/// Scans only a bounded window around the cursor (cost independent of
/// doc size). For editor cursor-hover; full-document callers use
/// [`gaiji_resolutions`].
#[must_use]
pub fn find_span(source: &str, byte_offset: usize) -> Option<(usize, usize)> {
    if source.is_empty() {
        return None;
    }
    let win_start =
        snap_to_char_boundary_left(source, byte_offset.saturating_sub(MAX_GAIJI_SPAN_LEN));
    let win_end = snap_to_char_boundary_right(
        source,
        byte_offset
            .saturating_add(MAX_GAIJI_SPAN_LEN)
            .min(source.len()),
    );
    let window = &source[win_start..win_end];
    let win_offset = byte_offset.saturating_sub(win_start);

    for (hash_in_win, _) in window.match_indices(BRACKET_HASH) {
        let Some(end_in_win) = gaiji_marker_end(window, hash_in_win) else {
            continue;
        };
        // Include a preceding `※` in the span (refmark form).
        let start_in_win = if window[..hash_in_win].ends_with(GAIJI_REFMARK) {
            hash_in_win - GAIJI_REFMARK.len()
        } else {
            hash_in_win
        };
        if (start_in_win..end_in_win).contains(&win_offset) {
            return Some((win_start + start_in_win, win_start + end_in_win));
        }
    }
    None
}

const fn snap_to_char_boundary_left(s: &str, mut idx: usize) -> usize {
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

const fn snap_to_char_boundary_right(s: &str, mut idx: usize) -> usize {
    let len = s.len();
    while idx < len && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

#[cfg(test)]
mod tests {
    #[test]
    fn corpus_references_resolve_without_rewriting_source_fields() {
        for (source, expected) in [
            ("※［＃「えんにょう＋囘」、第４水準2-12-11］", '\u{2231e}'),
            (
                "※［＃「にんべん＋（八／月）」、読みは「いつ」、第3水準1-14-20］",
                '\u{4f7e}',
            ),
            ("※［＃「彳＋（氏／一）」、第3水準1-84-31」］", '\u{5f7d}'),
            ("※［＃彳＋羊、第3水準1-84-32］", '\u{5f89}'),
            (
                "※［＃「袞」の「口」に代えて「厶」、U+886E、、59-14］",
                '\u{886e}',
            ),
            (
                "※［＃「※」は二の字点（踊り字）、面区点番号1-2-22、114-11］",
                '\u{303b}',
            ),
            ("※［＃「滷－さんずい」、第3水準1-83-35］", '\u{9e75}'),
            (
                "※［＃「衙」の「吾」に代えて「干」、U+884E、225-図のキャプション］",
                '\u{884e}',
            ),
            (
                "※［＃「喪」の「畏－田」に代えて「冖／貝」、U+8CF7、16-本文-7］",
                '\u{8cf7}',
            ),
        ] {
            let body =
                parse_gaiji_body(&source[GAIJI_OPEN.len()..source.len() - GAIJI_CLOSE.len()]);
            let result = resolve_at(source, 0, source.len()).expect("gaiji occurrence");
            assert_eq!(
                result.resolved.as_deref(),
                Some(expected.to_string().as_str()),
                "{source}"
            );
            assert_eq!(result.description, body.description, "{source}");
            assert_eq!(result.mencode.as_deref(), body.mencode, "{source}");
            assert_eq!(
                GaijiCanonical::from_mencode(body.mencode).resolve(body.description),
                Some(Resolved::Char(expected)),
                "{source}"
            );
        }
    }

    #[test]
    fn component_references_and_conflicting_glyph_codes_do_not_resolve() {
        for body in [
            "非0213外字：「厂＋菫」、ただし「菫」は第3水準1-92-16のつくりの形、読みは「わづ」、289-上-12",
            "「成分、第3水準1-84-32、を含む形」、9-1",
            "「不明」、U+5F89、第3水準1-84-31",
            "「不明、第3水準1-84-31」、U+5F89",
            "「字」の「形」に代えて「成分、U+884E」、225-図のキャプション",
            "「字」の「形」に代えて「干」、U+884E、225-図のキャプション、U+8CF7",
        ] {
            let source = format!("※［＃{body}］");
            let result = resolve_at(&source, 0, source.len()).expect("gaiji occurrence");
            assert_eq!(result.resolved, None, "{body}");
            let parsed = parse_gaiji_body(body);
            assert_eq!(
                GaijiCanonical::from_mencode(parsed.mencode).resolve(parsed.description),
                None,
                "{body}"
            );
        }
    }

    use super::*;

    #[test]
    fn parse_gaiji_body_splits_quoted_composed_and_bare() {
        // Simple quoted form: the 「」 wrapper is stripped.
        assert_eq!(
            parse_gaiji_body("「木＋吶のつくり」、第3水準1-85-54"),
            GaijiBody {
                description: "木＋吶のつくり",
                mencode: Some("第3水準1-85-54"),
                quoted: true,
            }
        );
        // Quoted form with no mencode is still a valid (quoted) gaiji.
        assert_eq!(
            parse_gaiji_body("「々」"),
            GaijiBody {
                description: "々",
                mencode: None,
                quoted: true,
            }
        );
        // Composed-glyph form: the whole description is kept verbatim,
        // not cut at the first 、; the right-to-left scan finds the mencode.
        assert_eq!(
            parse_gaiji_body("「廰」の「广」を「厂」に、第3水準1-15-94"),
            GaijiBody {
                description: "「廰」の「广」を「厂」に",
                mencode: Some("第3水準1-15-94"),
                quoted: false,
            }
        );
        // A 、 belonging to the description remains in it (right-to-left scan).
        assert_eq!(
            parse_gaiji_body("面から一、二画目をとったもの、第3水準1-15-94"),
            GaijiBody {
                description: "面から一、二画目をとったもの",
                mencode: Some("第3水準1-15-94"),
                quoted: false,
            }
        );
        // Bare form, no quotes.
        assert_eq!(
            parse_gaiji_body("二の字点、1-2-23"),
            GaijiBody {
                description: "二の字点",
                mencode: Some("1-2-23"),
                quoted: false,
            }
        );
        // A trailing 底本ページ-行 suffix stays inside mencode verbatim
        // (resolution strips it via `mencode_resolution_token`).
        assert_eq!(
            parse_gaiji_body("「※」、第3水準1-84-27、144-上-9"),
            GaijiBody {
                description: "※",
                mencode: Some("第3水準1-84-27、144-上-9"),
                quoted: true,
            }
        );
        // No mencode anchor and no quotes: the whole body is the description.
        assert_eq!(
            parse_gaiji_body("二の字点"),
            GaijiBody {
                description: "二の字点",
                mencode: None,
                quoted: false,
            }
        );
    }

    #[test]
    fn near_miss_page_line_is_a_superset_of_canonical() {
        // Canonical forms are accepted by both predicates.
        for s in ["144-上-9", "7巻-42-下-10", "372-10"] {
            assert!(is_page_line_shaped(s), "canonical {s}");
            assert!(is_near_miss_page_line_shaped(s), "near-miss superset {s}");
        }
        // Near-miss-only forms: rejected by canonical, accepted by near-miss.
        // Shape 1: 上/中/下 fused directly before a digit run.
        // Shape 3: full-width minus U+FF0D separator; 下 fused after digits.
        // Shape 4: poetry/register locator (段 / 首目 / `P` page prefix).
        for s in [
            "383-下8",
            "2-下3",
            "323-上1",
            "94－11",
            "92－３",
            "109下－4",
            "69-下段9首目",
            "P61-下段5首目",
        ] {
            assert!(!is_page_line_shaped(s), "canonical must reject {s}");
            assert!(is_near_miss_page_line_shaped(s), "near-miss accepts {s}");
        }
        // A bare 段 register is a 段組 operand, not a page-line part; both
        // predicates reject it, so `［＃ここから２段組、上段］` stays a directive.
        for s in ["上段", "下段", "中段", "38-上段-1"] {
            assert!(!is_page_line_shaped(s), "canonical rejects bare 段 {s}");
            assert!(
                !is_near_miss_page_line_shaped(s),
                "near-miss rejects bare 段 {s}"
            );
        }
        // Neither accepts non-page-line tails.
        for s in ["U+304B", "巻-3-4", "X1－1", "漢字", ""] {
            assert!(!is_page_line_shaped(s));
            assert!(!is_near_miss_page_line_shaped(s));
        }
    }

    #[test]
    fn parse_gaiji_body_handles_near_miss_locators() {
        // Shape 1: 上/中/下 fused before a digit run in the page-line tail.
        assert_eq!(
            parse_gaiji_body("「※」は「てへん＋劣」、第3水準1-84-77、383-下8"),
            GaijiBody {
                description: "「※」は「てへん＋劣」",
                mencode: Some("第3水準1-84-77、383-下8"),
                quoted: false,
            }
        );
        // Shape 2: mencode fused directly after `」` with no separating `、`.
        assert_eq!(
            parse_gaiji_body("「金＋夫」第3水準1-93-4"),
            GaijiBody {
                description: "金＋夫",
                mencode: Some("第3水準1-93-4"),
                quoted: true,
            }
        );
        // Shape 3: full-width minus separator in a composed description's
        // page-line tail (the relaxed split is what makes the run shaped).
        assert_eq!(
            parse_gaiji_body(
                "「※」は「いしへん」＋「乏」、読みは「いしばり」、第3水準1-88-93、94－11"
            ),
            GaijiBody {
                description: "「※」は「いしへん」＋「乏」、読みは「いしばり」",
                mencode: Some("第3水準1-88-93、94－11"),
                quoted: false,
            }
        );
        // Shape 4: poetry/register locator with `P` prefix + 段 + 首目.
        assert_eq!(
            parse_gaiji_body("「※」は「王へん」に「干」、第3水準1-87-83、P61-下段5首目"),
            GaijiBody {
                description: "「※」は「王へん」に「干」",
                mencode: Some("第3水準1-87-83、P61-下段5首目"),
                quoted: false,
            }
        );
    }

    #[test]
    fn near_miss_locators_resolve_to_the_menkuten_glyph() {
        // The relaxed provenance tail is irrelevant to resolution: the glyph
        // comes from the men-ku-ten anchor. 第3水準1-93-4 = 鈇 (U+9207).
        let body = recognize_gaiji_body("「金＋夫」第3水準1-93-4").expect("shape 2 is a gaiji");
        assert_eq!(
            lookup(
                None,
                body.mencode.map(mencode_resolution_token),
                body.description
            ),
            Some(Resolved::Char('\u{9207}'))
        );
    }

    #[test]
    fn near_miss_page_line_requires_a_mencode_anchor() {
        // A near-miss-only tail with no men-ku-ten anchor is not sufficient:
        // proofreader / 段組 directive tails have the same shape, so the body
        // stays a directive rather than becoming a gaiji.
        assert!(recognize_gaiji_body("「あ」に「い」、94－11").is_none());
        assert!(recognize_gaiji_body("「あ」に「い」、383-下8").is_none());
        // Proofreader / layout directives with a near-miss-shaped tail.
        assert!(recognize_gaiji_body("ここから２段組、上段").is_none());
        assert!(recognize_gaiji_body("底本の閉じ括弧は「』」、58-下15").is_none());
        assert!(recognize_gaiji_body("底本ルビは「もら」と誤記、175-上段-4").is_none());
        // With a men-ku-ten anchor the same near-miss tail is a gaiji.
        assert!(recognize_gaiji_body("「あ」に「い」、第3水準1-15-94、383-下8").is_some());
        // A canonical description-anchored gaiji with only a page-line tail
        // (no men-ku-ten) is recognized; the anchor rule does not apply to it.
        assert!(recognize_gaiji_body("小書き片仮名ン、500-下-19").is_some());
        // Plain directives stay directives.
        assert!(recognize_gaiji_body("改ページ").is_none());
        assert!(recognize_gaiji_body("ここから2字下げ").is_none());
    }

    #[test]
    fn gaiji_resolutions_empty_for_plain_text() {
        assert!(gaiji_resolutions("plain text, no gaiji").is_empty());
    }

    #[test]
    fn gaiji_resolutions_resolves_single_char_description() {
        // A description that is itself one glyph resolves via the
        // single-character fallback in `lookup`.
        let src = "前※［＃「々」］後";
        let res = gaiji_resolutions(src);
        assert_eq!(res.len(), 1);
        let g = &res[0];
        assert_eq!(g.description, "々");
        assert_eq!(g.resolved.as_deref(), Some("々"));
        assert_eq!(g.codepoint, Some('々' as u32));
        // Offsets bracket the literal `※［＃…］` span.
        assert_eq!(&src[g.start..g.end], "※［＃「々」］");
    }

    #[test]
    fn gaiji_resolutions_includes_standalone_form() {
        // Standalone `［＃…］` (no `※`) external-character form.
        let src = "前［＃「木＋吶のつくり」、第3水準1-85-54］後";
        let res = gaiji_resolutions(src);
        assert_eq!(res.len(), 1);
        let g = &res[0];
        assert_eq!(g.description, "木＋吶のつくり");
        assert_eq!(g.mencode.as_deref(), Some("第3水準1-85-54"));
        assert_eq!(g.resolved.as_deref(), Some("枘"));
        // The span has no `※`, so it starts at the `［`.
        assert_eq!(
            &src[g.start..g.end],
            "［＃「木＋吶のつくり」、第3水準1-85-54］"
        );
    }

    #[test]
    fn gaiji_resolutions_excludes_plain_directives() {
        // A bracket-hash directive is not a gaiji: `recognize_gaiji_body`
        // rejects it, so `gaiji_resolutions` omits it.
        assert!(gaiji_resolutions("本文［＃改ページ］続き").is_empty());
        assert!(gaiji_resolutions("［＃ここから2字下げ］字下げ").is_empty());
        // A forward-reference bouten directive (balanced quotes, no mencode)
        // is also rejected.
        assert!(gaiji_resolutions("東京［＃「東京」に傍点］へ").is_empty());
    }

    #[test]
    fn gaiji_resolutions_mixes_refmark_and_standalone_in_order() {
        // Both forms in one document, source order preserved.
        let src = "※［＃「々」］と［＃「木＋吶のつくり」、第3水準1-85-54］";
        let res = gaiji_resolutions(src);
        assert_eq!(res.len(), 2);
        assert_eq!(&src[res[0].start..res[0].end], "※［＃「々」］");
        assert_eq!(
            &src[res[1].start..res[1].end],
            "［＃「木＋吶のつくり」、第3水準1-85-54］"
        );
    }

    #[test]
    fn gaiji_resolutions_continue_past_an_opener_that_bounds_no_reference() {
        let src = "［＃改丁」\n瞼※［＃「目＋匡」、第3水準1-88-81、28-8］";
        let res = gaiji_resolutions(src);
        assert_eq!(res.len(), 1);
        assert_eq!(
            &src[res[0].start..res[0].end],
            "※［＃「目＋匡」、第3水準1-88-81、28-8］"
        );
        assert!(gaiji_resolutions("※［＃「目\n＋匡」、第3水準1-88-81］").is_empty());
    }

    #[test]
    fn refmark_statements_keep_their_wording() {
        for (description, refmark, kept) in [
            ("「※」は小書きの「ト」", true, true),
            ("「※」は、石へん＋乞", true, true),
            ("「※」は", false, true),
            ("「窗」の下に「心」", false, true),
            ("もんがまえに真の正字", false, false),
        ] {
            assert_eq!(describes_refmark(description), refmark, "{description}");
            assert_eq!(
                gaiji_statement_serializable(description),
                kept,
                "{description}"
            );
        }
        assert!(!gaiji_statement_serializable("「X」は※［＃「目"));
    }

    #[test]
    fn substitution_descriptions_keep_unknown_glyph_identity() {
        for body in [
            "「闃」の「目」に代えて「自」",
            "「贏」の「貝」に代えて「果」、（二）-27-3",
            "「壼」の「亞」に代えて「亜」、53-下-13",
            "偏が「宛」で旁が「りっとう」",
        ] {
            let parsed = recognize_gaiji_body(body)
                .unwrap_or_else(|| panic!("explicit glyph substitution: {body}"));
            assert!(gaiji_description_serializable(
                parsed.description,
                parsed.mencode.is_some()
            ));
        }
        for body in [
            "「闃」の「目」に代えて「自」らしい",
            "「闃」の「」に代えて「自」",
            "「闃」の「目」に代えて自",
            "偏が「宛」で旁が「」",
            "偏が「宛」で旁がりっとう",
            "「本文」は底本では「別文」",
        ] {
            assert!(recognize_gaiji_body(body).is_none(), "{body}");
        }
    }

    #[test]
    fn recognize_gaiji_body_gates_directives() {
        assert!(recognize_gaiji_body("改ページ").is_none());
        assert!(recognize_gaiji_body("ここから2字下げ").is_none());
        assert!(recognize_gaiji_body("「々」").is_some()); // quoted form, no mencode
        assert!(recognize_gaiji_body("「desc」、第3水準1-85-54").is_some());
        // a bare `ローマ数字N` resolves, so its `※［＃…］` is gaiji,
        // not a plain directive.
        assert!(recognize_gaiji_body("ローマ数字17").is_some());
        assert!(recognize_gaiji_body("ローマ数字23").is_some());
        // A non-numeral `ローマ数字` body and out-of-range N stay directives.
        assert!(recognize_gaiji_body("ローマ数字").is_none());
        assert!(recognize_gaiji_body("ローマ数字0").is_none());
    }

    #[test]
    fn recognize_dictionary_description_with_unanchored_page_line() {
        // A near-miss page-line locator (`87-16`) requires a mencode anchor
        // unless the description is a dictionary entry: the description
        // then resolves the glyph and the locator records provenance.
        let parsed = recognize_gaiji_body("小書き平仮名ん、87-16").expect("dictionary-described");
        assert_eq!(parsed.description, "小書き平仮名ん");
        assert_eq!(parsed.mencode, Some("87-16"));
        // A non-dictionary description with the same unanchored locator
        // stays a plain directive (the false-positive guard).
        assert!(recognize_gaiji_body("ルビの「おや」は底本では「をや」、58-下15").is_none());
    }

    #[test]
    fn recognize_unbalanced_quote_typo_with_mencode_anchor() {
        // Editor typo: the opening 「 is missing, but the trailing mencode
        // anchors the reference. The description is kept verbatim (orphan
        // quote included) so serialization reproduces the source.
        let parsed = recognize_gaiji_body("くさかんむり／（月＋曷）」、第3水準1-91-26")
            .expect("mencode-anchored typo");
        assert_eq!(parsed.description, "くさかんむり／（月＋曷）」");
        assert_eq!(parsed.mencode, Some("第3水準1-91-26"));
        // Without a mencode anchor an unbalanced-quote body is not recognized.
        assert!(recognize_gaiji_body("くさかんむり／（月＋曷）」").is_none());
    }

    #[test]
    fn roman_numeral_glyph_composes_from_the_u2160_block() {
        // Standard roman spelling with single-letter code points: 17 = X V I I.
        assert_eq!(
            roman_numeral_glyph("ローマ数字17"),
            Some("\u{2169}\u{2164}\u{2160}\u{2160}")
        );
        assert_eq!(
            roman_numeral_glyph("ローマ数字13"),
            Some("\u{2169}\u{2160}\u{2160}\u{2160}")
        );
        assert_eq!(
            roman_numeral_glyph("ローマ数字20"),
            Some("\u{2169}\u{2169}")
        );
        // Lowercase form (spec-complete; 0 corpus occurrences) uses U+2170.
        assert_eq!(
            roman_numeral_glyph("ローマ数字4小文字"),
            Some("\u{2170}\u{2174}")
        );
        // Full-width digits parse too.
        assert_eq!(
            roman_numeral_glyph("ローマ数字１７"),
            Some("\u{2169}\u{2164}\u{2160}\u{2160}")
        );
        // Not a roman numeral description.
        assert_eq!(roman_numeral_glyph("二重かっこ開く"), None);
        assert_eq!(roman_numeral_glyph("ローマ数字"), None);
        assert_eq!(roman_numeral_glyph("ローマ数字0"), None);
    }

    #[test]
    fn find_span_locates_enclosing_reference() {
        let src = "あ※［＃「々」］い";
        let open = src.find("※").unwrap();
        // A cursor inside the span finds the whole `※［＃…］` range.
        let span = find_span(src, open + GAIJI_OPEN.len()).unwrap();
        assert_eq!(&src[span.0..span.1], "※［＃「々」］");
        // A cursor outside any reference finds nothing.
        assert!(find_span(src, 0).is_none());
    }

    #[test]
    fn lookup_returns_existing_ucs_before_table_lookup() {
        // The "existing" short-circuit returns the caller-provided
        // codepoint without consulting either table.
        assert_eq!(
            lookup(Some('\u{1234}'), Some("第3水準1-85-54"), "木＋吶のつくり"),
            Some(Resolved::Char('\u{1234}'))
        );
    }

    #[test]
    fn a_space_between_the_level_and_its_coordinates_resolves_the_same_glyph() {
        // 060590_001938 contains `第4水準 2-13-28`; every other work and
        // every table key omits the space. The clause is delimited by `、`,
        // so the space separates nothing.
        let description = "「插」でつくりの縦棒が下に突き抜けている";
        assert_eq!(
            lookup(None, Some("第4水準 2-13-28"), description),
            lookup(None, Some("第4水準2-13-28"), description)
        );
        assert_eq!(
            lookup(None, Some("第4水準 2-13-28"), description),
            Some(Resolved::Char('\u{63F7}'))
        );
        // `is_mencode_shaped` must also accept the spaced token, because the
        // composed scan needs a mencode-shaped token to anchor the run;
        // otherwise the marker is never resolved.
        assert!(is_mencode_shaped("第4水準 2-13-28"));
        // The spaced token is not a structured form, so serialization
        // reproduces the source token with its space.
        assert!(parse_menkuten("第4水準 2-13-28").is_none());
    }

    #[test]
    fn a_halfwidth_comma_separates_a_description_from_its_reference() {
        // 002587_000372 line 47. Same punctuation mark, narrower width.
        let body = parse_gaiji_body("「てへん＋闌」､第4水準2-13-61");
        assert_eq!(body.description, "てへん＋闌");
        assert_eq!(body.mencode, Some("第4水準2-13-61"));
        assert!(body.quoted);
    }

    #[test]
    fn a_reference_fused_to_a_nested_quote_description_is_still_the_reference() {
        // 004643_000284 line 40 contains this glyph once with no separator
        // and three times with one. The nested 「」 excludes it from the
        // simple quoted branch, which accepts the fused shape only without
        // nesting.
        let body = parse_gaiji_body("「（罪－非）／「厠」の「貝」に変えて「炎」」第4水準2-84-80");
        assert_eq!(
            body.description,
            "「（罪－非）／「厠」の「貝」に変えて「炎」」"
        );
        assert_eq!(body.mencode, Some("第4水準2-84-80"));
        assert!(!body.quoted);
    }

    #[test]
    fn a_directive_that_merely_follows_a_quote_is_not_a_reference() {
        // The fused fallback applies only when the tail is mencode-shaped, so
        // a forward directive after a closing quote stays part of the
        // description and the marker has no reference.
        let body = parse_gaiji_body("「厠」の「貝」に傍点");
        assert_eq!(body.mencode, None);
    }

    #[test]
    fn lookup_via_mencode_table_when_ucs_missing() {
        // 罪と罰 fixture: `木＋吶のつくり` with 第3水準1-85-54.
        // Per JIS X 0213:2004 plane 1, row 85, cell 54 = 枘 (U+6798).
        // ("吶のつくり" = right-side component of 吶 = 内, so 木+内 = 枘.)
        assert_eq!(
            lookup(None, Some("第3水準1-85-54"), "木＋吶のつくり"),
            Some(Resolved::Char('\u{6798}'))
        );
    }

    #[test]
    fn kanji_numbered_level_resolves_without_rewriting_the_reference() {
        let source = "※［＃「※」は「歌記号」、第三水準1-3-28、107-16］";
        let result = resolve_at(source, 0, source.len()).expect("gaiji occurrence");

        assert_eq!(result.description, "「※」は「歌記号」");
        assert_eq!(result.mencode.as_deref(), Some("第三水準1-3-28、107-16"));
        assert_eq!(result.resolved.as_deref(), Some("〽"));
    }

    #[test]
    fn lookup_via_combo_table_returns_multi() {
        // 第3水準1-4-87 = か゚ = U+304B U+309A (combining handakuten).
        // Only the combo table maps these 25 cells.
        assert_eq!(
            lookup(None, Some("第3水準1-4-87"), ""),
            Some(Resolved::Multi("\u{304B}\u{309A}"))
        );
    }

    #[test]
    fn combo_resolution_writes_both_codepoints() {
        // Combo lookup followed by `write_to` yields the 2-codepoint
        // sequence (6 UTF-8 bytes for か + handakuten).
        let resolved = lookup(None, Some("第3水準1-4-87"), "").expect("combo resolves");
        let mut s = String::new();
        resolved
            .write_to(&mut s)
            .expect("write to String never fails");
        assert_eq!(s, "\u{304B}\u{309A}");
        assert_eq!(s.chars().count(), 2);
    }

    #[test]
    fn lookup_via_u_plus_form() {
        assert_eq!(
            lookup(None, Some("U+01F5"), "Latin Small Letter G With Acute"),
            Some(Resolved::Char('\u{01F5}'))
        );
    }

    #[test]
    fn lookup_via_u_plus_max_six_hex_digits() {
        // U+10FFFF is the Unicode max; any shape past 6 digits is rejected.
        assert_eq!(
            lookup(None, Some("U+10FFFF"), ""),
            Some(Resolved::Char('\u{10FFFF}'))
        );
    }

    #[test]
    fn lookup_rejects_u_plus_beyond_seven_hex_digits() {
        assert_eq!(lookup(None, Some("U+1234567"), ""), None);
    }

    #[test]
    fn lookup_rejects_u_plus_surrogate() {
        assert_eq!(lookup(None, Some("U+D800"), ""), None);
    }

    #[test]
    fn lookup_rejects_u_plus_non_hex() {
        assert_eq!(lookup(None, Some("U+GG12"), ""), None);
    }

    #[test]
    fn lookup_rejects_u_plus_without_digits() {
        assert_eq!(lookup(None, Some("U+"), ""), None);
    }

    #[test]
    fn lookup_via_description_fallback_when_mencode_absent() {
        assert_eq!(lookup(None, None, "〓"), Some(Resolved::Char('\u{3013}')));
    }

    #[test]
    fn lookup_returns_none_when_all_paths_miss() {
        // Multi-char description AND missing mencode → no resolution.
        assert_eq!(
            lookup(None, Some("not-in-any-table"), "unresolved gaiji"),
            None
        );
    }

    #[test]
    fn lookup_falls_back_to_description_self_when_single_char() {
        // 丂 is in the JIS X 0213 plane 2 table at row 1 cell 2, but a
        // corpus text contains `※[#「丂」、第4水準2-16-1]`, whose mencode
        // is not in the table. The description is the kanji itself, so the
        // single-character fallback resolves to it.
        assert_eq!(
            lookup(None, Some("第4水準2-16-1"), "丂"),
            Some(Resolved::Char('\u{4E02}'))
        );
        // Same for descriptions with no mencode at all.
        assert_eq!(lookup(None, None, "畺"), Some(Resolved::Char('\u{757A}')));
        assert_eq!(lookup(None, None, "龔"), Some(Resolved::Char('\u{9F94}')));
    }

    #[test]
    fn single_char_fallback_does_not_override_dictionary_hit() {
        // `〓` is in the special-placeholder table, mapped to `〓 U+3013`,
        // so the dictionary path returns before the single-char fallback.
        // The fallback would return the same value, so this assertion does
        // not distinguish the two paths; the contract is that the fallback
        // applies only when nothing else matched.
        assert_eq!(lookup(None, None, "〓"), Some(Resolved::Char('\u{3013}')));
    }

    #[test]
    fn single_char_fallback_does_not_apply_to_multi_char_descriptions() {
        // Multi-char description not in any table → must still be None.
        // Confirms the early-return on `chars.next().is_none()`.
        assert_eq!(lookup(None, None, "未知の字形"), None);
        assert_eq!(lookup(None, None, "ab"), None);
    }

    #[test]
    fn bare_numeric_references_resolve_without_changing_source_spelling() {
        for (token, glyph) in [
            ("1-13-26", Resolved::Char('Ⅵ')),
            ("1-7-82", Resolved::Char('ヷ')),
            ("1-8-78", Resolved::Char('⁉')),
            ("1-4-87", Resolved::Multi("か\u{309a}")),
            ("2-1-1", Resolved::Char('𠂉')),
        ] {
            let canonical = GaijiCanonical::from_mencode(Some(token));
            assert_eq!(canonical.resolve("未登録の説明"), Some(glyph), "{token}");
            let mut retained = String::new();
            canonical.write_mencode(&mut retained).unwrap();
            assert_eq!(retained, token);
        }
        for token in ["1-0-26", "1-13-95", "3-13-26", "1-13-26-extra"] {
            assert_eq!(lookup(None, Some(token), "未登録の説明"), None, "{token}");
        }
    }

    #[test]
    fn mencode_table_covers_the_fixture_gaiji() {
        // The 罪と罰 fixture reference maps to 枘 U+6798, not U+6903 椃.
        assert_eq!(
            JISX0213_MENCODE_TO_CHAR.get("第3水準1-85-54"),
            Some(&'\u{6798}')
        );
    }

    #[test]
    fn table_sizes_match_jisx0213_2004_spec() {
        // The complete plane 1 includes 6,879 older JIS cells plus
        // 1,893 single-codepoint additions; combining cells are separate.
        use crate::jisx0213_table::{
            DESCRIPTION_COUNT, JISX0213_COMBO_COUNT, JISX0213_PLANE1_COUNT, JISX0213_PLANE2_COUNT,
        };
        let (single, combo, description) = table_sizes();
        assert_eq!(single, JISX0213_PLANE1_COUNT + JISX0213_PLANE2_COUNT);
        assert_eq!(combo, JISX0213_COMBO_COUNT);
        assert_eq!(description, DESCRIPTION_COUNT);
        assert_eq!(
            JISX0213_PLANE1_COUNT, 8772,
            "plane 1 must include older cells and single-codepoint additions",
        );
        assert_eq!(
            JISX0213_PLANE2_COUNT, 2436,
            "第4水準 must equal the spec count",
        );
        assert_eq!(
            JISX0213_COMBO_COUNT, 25,
            "combining-sequence cells must equal spec",
        );
        assert!(
            description >= 8_000,
            "description-fallback table looks too small ({description}) — \
             did the gaiji-chuki extraction drop entries?",
        );
    }

    #[test]
    fn description_table_resolves_a_known_dictionary_entry() {
        // 「木＋吶のつくり」 is the fixture description for 枘
        // (U+6798, JIS X 0213 plane 1 row 85 cell 54). The dictionary
        // path resolves the same character as the mencode path, so a
        // description-only lookup (no mencode) returns U+6798.
        assert_eq!(
            lookup(None, None, "木＋吶のつくり"),
            Some(Resolved::Char('\u{6798}')),
        );
    }

    #[test]
    fn description_table_does_not_truncate_composed_characters() {
        for (description, expected) in [
            ("糸＋率", Some('\u{7E42}')),
            ("口＋苦", Some('\u{20E0B}')),
            ("冫＋車", None),
        ] {
            assert_eq!(
                lookup(None, None, description),
                expected.map(Resolved::Char),
                "{description}"
            );
        }
    }

    #[test]
    fn description_table_preserves_special_placeholders() {
        // 〓 / 〻 are hand-curated specials kept in
        // `aozora-gaiji-special.tsv` and merged into the generated map.
        assert_eq!(lookup(None, None, "〓"), Some(Resolved::Char('\u{3013}')));
        assert_eq!(lookup(None, None, "〻"), Some(Resolved::Char('\u{303B}')));
    }

    #[test]
    fn full_jisx0213_table_covers_a_known_plane1_third_tier_kanji() {
        // 第3水準1-85-9 = 敧 (U+6567) per JIS X 0213:2004.
        assert_eq!(
            JISX0213_MENCODE_TO_CHAR.get("第3水準1-85-9"),
            Some(&'\u{6567}')
        );
    }

    #[test]
    fn full_jisx0213_table_covers_a_known_plane2_fourth_tier_entry() {
        // 第4水準2-1-1 = 𠂉 (U+20089) — first plane-2 cell.
        assert_eq!(
            JISX0213_MENCODE_TO_CHAR.get("第4水準2-1-1"),
            Some(&'\u{20089}')
        );
    }

    #[test]
    fn resolved_utf8_len_matches_actual_encoding() {
        assert_eq!(Resolved::Char('A').utf8_len(), 1);
        assert_eq!(Resolved::Char('あ').utf8_len(), 3);
        assert_eq!(Resolved::Char('𠂉').utf8_len(), 4);
        assert_eq!(Resolved::Multi("\u{304B}\u{309A}").utf8_len(), 6);
    }

    #[test]
    fn resolved_as_char_returns_none_for_combos() {
        assert_eq!(Resolved::Char('A').as_char(), Some('A'));
        assert_eq!(Resolved::Multi("か゚").as_char(), None);
    }

    #[test]
    fn lookup_is_identity_on_the_ucs_input_when_set() {
        // The "existing" short-circuit returns the caller-provided
        // scalar without a table lookup.
        assert_eq!(
            lookup(Some('あ'), Some("anything"), "anything"),
            Some(Resolved::Char('あ'))
        );
    }

    #[test]
    fn menkuten_round_trips_through_display() {
        let m = parse_menkuten("第3水準1-85-54").expect("clean men-ku-ten parses");
        assert_eq!(
            m,
            MenKuTen {
                plane: 1,
                ku: 85,
                ten: 54
            }
        );
        assert_eq!(m.level(), 3);
        assert_eq!(m.to_string(), "第3水準1-85-54");
        let m4 = parse_menkuten("第4水準2-1-1").expect("plane-2 parses");
        assert_eq!(
            m4,
            MenKuTen {
                plane: 2,
                ku: 1,
                ten: 1
            }
        );
        assert_eq!(m4.to_string(), "第4水準2-1-1");
    }

    #[test]
    fn parse_menkuten_rejects_non_canonical_forms() {
        // Bare (no 第N水準 prefix) — kept verbatim, not structured.
        assert!(parse_menkuten("1-2-23").is_none());
        // Page-line suffix — not a clean men-ku-ten.
        assert!(parse_menkuten("第3水準1-84-27、144-上-9").is_none());
        // Level inconsistent with the plane (第3水準 ⇒ plane 1, not 2).
        assert!(parse_menkuten("第3水準2-1-1").is_none());
        // Leading zero would not byte-reproduce.
        assert!(parse_menkuten("第3水準1-05-4").is_none());
        // U+ form is not a men-ku-ten.
        assert!(parse_menkuten("U+74FC").is_none());
    }

    #[test]
    fn gaiji_canonical_classifies_only_clean_forms() {
        assert_eq!(
            GaijiCanonical::from_mencode(Some("第3水準1-85-54")),
            GaijiCanonical::MenKuTen(MenKuTen {
                plane: 1,
                ku: 85,
                ten: 54
            })
        );
        assert_eq!(
            GaijiCanonical::from_mencode(Some("U+74FC")),
            GaijiCanonical::Unicode('\u{74FC}')
        );
        // Suffixed / bare / absent stay verbatim for byte-exact serialize.
        assert_eq!(
            GaijiCanonical::from_mencode(Some("U+74FC、372-10")),
            GaijiCanonical::Unresolved {
                mencode: Some("U+74FC、372-10")
            }
        );
        assert_eq!(
            GaijiCanonical::from_mencode(Some("1-2-23")),
            GaijiCanonical::Unresolved {
                mencode: Some("1-2-23")
            }
        );
        assert_eq!(
            GaijiCanonical::from_mencode(None),
            GaijiCanonical::Unresolved { mencode: None }
        );
    }

    #[test]
    fn gaiji_canonical_resolve_matches_raw_string_lookup() {
        // Men-ku-ten resolves through the table to the same glyph as the
        // raw-string lookup (枘 U+6798).
        assert_eq!(
            GaijiCanonical::from_mencode(Some("第3水準1-85-54")).resolve("木＋吶のつくり"),
            Some(Resolved::Char('\u{6798}'))
        );
        assert_eq!(
            GaijiCanonical::from_mencode(Some("U+74FC")).resolve(""),
            Some(Resolved::Char('\u{74FC}'))
        );
        // Suffixed form resolves via the resolution-token (suffix stripped).
        assert_eq!(
            GaijiCanonical::from_mencode(Some("U+74FC、372-10")).resolve(""),
            Some(Resolved::Char('\u{74FC}'))
        );
    }

    #[test]
    fn gaiji_canonical_write_mencode_reproduces_source() {
        let render = |c: GaijiCanonical<'_>| {
            let mut s = String::new();
            c.write_mencode(&mut s).unwrap();
            s
        };
        assert_eq!(
            render(GaijiCanonical::from_mencode(Some("第3水準1-85-54"))),
            "第3水準1-85-54"
        );
        assert_eq!(
            render(GaijiCanonical::from_mencode(Some("U+74FC"))),
            "U+74FC"
        );
        assert_eq!(
            render(GaijiCanonical::from_mencode(Some(
                "第3水準1-84-27、144-上-9"
            ))),
            "第3水準1-84-27、144-上-9"
        );
        GaijiCanonical::from_mencode(None)
            .write_mencode(&mut String::new())
            .unwrap();
        assert!(!GaijiCanonical::from_mencode(None).has_mencode());
    }
}
#[test]
fn reverse_jis_cell_finds_accented_latin() {
    assert_eq!(
        jis_cell('é'),
        Some(JisCell {
            plane: 1,
            row: 9,
            cell: 63,
        })
    );
}

#[cfg(test)]
mod voiced_and_external_tests {
    use super::*;

    fn resolved_string(description: &str) -> Option<String> {
        lookup(None, None, description).map(|r| {
            let mut s = String::new();
            _ = r.write_to(&mut s);
            s
        })
    }

    #[test]
    fn voicing_descriptions_compose_the_base_and_the_combining_mark() {
        for (description, expected) in [
            ("濁点付き片仮名エ", "エ\u{3099}"),
            ("濁点付き片仮名ア", "ア\u{3099}"),
            ("濁点付き井", "井\u{3099}"),
            ("濁点付き二の字点", "\u{303B}\u{3099}"),
            ("濁点付きの二の字点", "\u{303B}\u{3099}"),
            ("半濁点付き平仮名と", "と\u{309A}"),
            ("半濁点付き片仮名ラ", "ラ\u{309A}"),
            ("小書き濁点付き片仮名カ", "ヵ\u{3099}"),
            ("濁点付き小書き片仮名カ", "ヵ\u{3099}"),
            ("濁点付き小書き平仮名か", "ゕ\u{3099}"),
        ] {
            assert_eq!(
                resolved_string(description).as_deref(),
                Some(expected),
                "{description}"
            );
            let resolved = lookup(None, None, description).expect("resolves");
            assert!(
                matches!(resolved, Resolved::Voiced { .. }),
                "{description}: {resolved:?}"
            );
            assert_eq!(resolved.as_char(), None);
            assert_eq!(resolved.utf8_len(), expected.len());
        }
    }

    #[test]
    fn precomposed_voiced_forms_come_from_the_tables_first() {
        assert_eq!(
            lookup(None, None, "濁点付き片仮名ワ"),
            Some(Resolved::Char('ヷ'))
        );
        assert_eq!(
            lookup(None, None, "濁点付き平仮名う"),
            Some(Resolved::Char('ゔ'))
        );
        assert!(matches!(
            lookup(None, Some("第3水準1-4-87"), "半濁点付き平仮名か"),
            Some(Resolved::Multi(_))
        ));
    }

    #[test]
    fn voicing_rule_rejects_what_it_cannot_name() {
        for description in [
            "濁点付き",
            "濁点付き片仮名",
            "濁点付き片仮名エル",
            "濁点付き片仮名か",
            "濁点付き平仮名カ",
            "小書き濁点付き片仮名ネ",
            "濁点付き感嘆符三つ",
        ] {
            assert_eq!(voiced_glyph(description), None, "{description}");
        }
    }

    #[test]
    fn external_identity_names_unencoded_kanji_by_ids_and_glyphwiki_glyph() {
        let identity = external_identity("冫＋影").expect("listed in the GlyphWiki table");
        assert_eq!(identity.ids, "⿰冫影");
        assert_eq!(identity.glyphwiki, "u2ff0-u51ab-u5f71");
        assert_eq!(lookup(None, None, "冫＋影"), None, "no Unicode claim");
        assert_eq!(external_identity("けものへん＋苗"), None);
    }

    #[test]
    fn glyphwiki_table_keys_are_what_the_body_parser_derives() {
        let table = include_str!("../data/aozora-gaiji-glyphwiki.tsv");
        let mut rows = 0;
        for line in table
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let cols: Vec<&str> = line.split('\t').collect();
            let (description, notation) = (cols[0], cols[1]);
            assert_eq!(
                parse_gaiji_body(notation).description,
                description,
                "{notation}"
            );
            assert_eq!(
                parse_gaiji_body(&format!("{notation}、466-8")).description,
                description,
                "{notation} with a page-line locator"
            );
            assert!(external_identity(description).is_some(), "{description}");
            rows += 1;
        }
        assert_eq!(rows, DESCRIPTION_TO_EXTERNAL.len());
    }

    #[test]
    fn cjkvi_table_keys_are_what_the_body_parser_derives() {
        let table = include_str!("../data/aozora-gaiji-cjkvi.tsv");
        let mut rows = 0;
        for line in table
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let cols: Vec<&str> = line.split('\t').collect();
            let (description, codepoint, notation) = (cols[0], cols[1], cols[2]);
            let body = notation
                .strip_prefix("※［＃")
                .and_then(|body| body.strip_suffix('］'))
                .unwrap_or_else(|| panic!("{notation} is not a complete marker"));
            assert_eq!(
                parse_gaiji_body(body).description,
                description,
                "{notation}"
            );
            assert!(
                !DESCRIPTION_TO_CHAR.contains_key(description),
                "{description} is in the official dictionary"
            );
            let expected = char::from_u32(u32::from_str_radix(codepoint, 16).unwrap()).unwrap();
            assert_eq!(
                lookup_with_source(None, None, description),
                Some((Resolved::Char(expected), MappingSource::CjkviData)),
                "{description}"
            );
            rows += 1;
        }
        assert_eq!(rows, DESCRIPTION_TO_CHAR_CJKVI.len());
    }

    #[test]
    fn cjkvi_ivs_table_keys_are_what_the_body_parser_derives() {
        let table = include_str!("../data/aozora-gaiji-cjkvi-ivs.tsv");
        let mut rows = 0;
        for line in table
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let cols: Vec<&str> = line.split('\t').collect();
            let (description, sequence, notation) = (cols[0], cols[1], cols[2]);
            let body = notation
                .strip_prefix("※［＃")
                .and_then(|body| body.strip_suffix('］'))
                .unwrap_or_else(|| panic!("{notation} is not a complete marker"));
            assert_eq!(
                parse_gaiji_body(body).description,
                description,
                "{notation}"
            );
            let expected: String = sequence
                .split(' ')
                .map(|hex| char::from_u32(u32::from_str_radix(hex, 16).unwrap()).unwrap())
                .collect();
            assert_eq!(
                resolved_string(description).as_deref(),
                Some(expected.as_str()),
                "{description}"
            );
            assert!(
                matches!(
                    lookup_with_source(None, None, description),
                    Some((Resolved::Multi(_), MappingSource::CjkviData))
                ),
                "{description}"
            );
            rows += 1;
        }
        assert_eq!(rows, DESCRIPTION_TO_STR_CJKVI.len());
    }

    #[test]
    fn selector_table_refines_official_code_points_only() {
        let table = include_str!("../data/aozora-gaiji-cjkvi-selectors.tsv");
        let mut rows = 0;
        for line in table
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let cols: Vec<&str> = line.split('\t').collect();
            let (description, sequence, notation) = (cols[0], cols[1], cols[2]);
            let body = notation
                .strip_prefix("※［＃")
                .and_then(|body| body.strip_suffix('］'))
                .unwrap_or_else(|| panic!("{notation} is not a complete marker"));
            assert_eq!(
                parse_gaiji_body(body).description,
                description,
                "{notation}"
            );
            let expected: String = sequence
                .split(' ')
                .map(|hex| char::from_u32(u32::from_str_radix(hex, 16).unwrap()).unwrap())
                .collect();
            assert!(is_variation_sequence(&expected), "{description}");
            assert_eq!(
                DESCRIPTION_TO_CHAR.get(description),
                expected.chars().next().as_ref(),
                "{description}: the official code point is the base"
            );
            assert_eq!(
                resolved_string(description).as_deref(),
                Some(expected.as_str()),
                "{description}"
            );
            assert!(
                matches!(
                    lookup_with_source(None, None, description),
                    Some((Resolved::Multi(_), MappingSource::CjkviSelector))
                ),
                "{description}"
            );
            rows += 1;
        }
        assert_eq!(rows, DESCRIPTION_TO_SELECTOR.len());
    }

    #[test]
    fn selector_refinement_follows_a_matching_reference_and_not_a_differing_one() {
        // 赤＋報のつくり is 赧 U+8D67 (JIS 1-76-63) in the official dictionary
        // and 赧 U+E0101 in the selector table.
        assert_eq!(
            lookup_with_source(None, Some("第3水準1-76-63"), "赤＋報のつくり"),
            Some((Resolved::Multi("赧\u{E0101}"), MappingSource::CjkviSelector)),
            "the JIS cell for 赧 is refined"
        );
        assert_eq!(
            lookup_with_source(None, Some("第3水準1-87-64"), "赤＋報のつくり"),
            Some((Resolved::Char('燄'), MappingSource::Standard)),
            "a reference to another character is not refined"
        );
        assert_eq!(
            lookup_with_source(None, None, "朽のつくり"),
            Some((Resolved::Char('丂'), MappingSource::Standard))
        );
    }

    #[test]
    fn cjkvi_tier_yields_to_references_and_the_official_dictionary() {
        assert_eq!(
            lookup_with_source(None, None, "「抜」の「友」に代えて「ノ／友」"),
            Some((Resolved::Char('㧞'), MappingSource::CjkviData))
        );
        assert_eq!(
            lookup_with_source(
                None,
                Some("第3水準1-87-64"),
                "「抜」の「友」に代えて「ノ／友」"
            ),
            Some((Resolved::Char('燄'), MappingSource::Standard)),
            "JIS references take precedence over description tables"
        );
        assert_eq!(
            lookup_with_source(None, None, "朽のつくり"),
            Some((Resolved::Char('丂'), MappingSource::Standard))
        );
        assert_eq!(
            lookup_with_source(None, None, "濁点付き片仮名エ"),
            Some((
                Resolved::Voiced {
                    base: 'エ',
                    mark: '\u{3099}'
                },
                MappingSource::Standard
            )),
            "composition rules take precedence over the cjkvi table"
        );
        assert_eq!(lookup_with_source(None, None, "未知の字"), None);
    }
}
