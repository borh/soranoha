//! Aozora Bunko accent decomposition: ASCII digraph → Unicode letter.
//!
//! Spec: <https://www.aozora.gr.jp/accent_separation.html>
//!
//! The scheme encodes an accented Latin letter as a base ASCII letter followed
//! by a one-character marker. [`ACCENT_TABLE`] holds the spec's 114 entries as
//! a compile-time slice, and the lexer (for pre-parse rewriting) and downstream
//! tools read that one table.
//!
//! ```
//! use ab_aozora_syntax::accent::decompose_fragment;
//! assert_eq!(decompose_fragment("fune`bre"), "funèbre");
//! assert_eq!(decompose_fragment("ae&on"), "æon");
//! assert_eq!(decompose_fragment("plain"), "plain");
//! ```
//!
//! # Invariants
//!
//! - Each ASCII digraph maps to exactly one Unicode codepoint. Matching tries
//!   the ligatures (`ae&`, `AE&`, `oe&`, `OE&`) before the single-letter
//!   digraphs.
//! - `decompose_fragment` can increase the byte length of a substring
//!   (`m'` = ḿ, `e~` = ẽ are BMP codepoints ≥ U+1E00 whose UTF-8 forms are
//!   3 bytes, larger than their 2-byte ASCII digraphs). Callers that back-map
//!   diagnostic spans across the rewrite must record a per-position delta.
//!
//! # Scope of use
//!
//! Call the function only on the body of a `〔...〕` span; Aozora restricts
//! accent decomposition to that convention. Transcribers wrap whole foreign
//! passages in `〔…〕`, so prose punctuation also occurs inside the span, and
//! `digraph_applies` restricts two marker bytes per occurrence (cedilla only on
//! `c`/`C` before a letter, acute only on a vowel base), a rule validated on
//! the corpus against the archive's XHTML rendering. `text,` and `nicht,ihr`
//! keep their commas inside a span.

use std::borrow::Cow;

use crate::format::AccentMark;

mod supplied_marks;

pub use supplied_marks::supplied_mark_target_start;

/// The full accent decomposition table in spec-page order.
pub const ACCENT_TABLE: &[(&str, char)] = &[
    // --- Ligatures (3-char patterns take precedence over the 2-char group) ---
    ("ae&", 'æ'),
    ("AE&", 'Æ'),
    ("oe&", 'œ'),
    ("OE&", 'Œ'),
    ("s&", 'ß'), // eszett: `&` on `s` is a ligature, not ring-above
    // --- 【a】 ---
    ("a`", 'à'),
    ("a'", 'á'),
    ("a^", 'â'),
    ("a~", 'ã'),
    ("a:", 'ä'),
    ("a&", 'å'),
    ("a_", 'ā'),
    // --- 【c】 ---
    ("c,", 'ç'),
    ("c'", 'ć'),
    ("c^", 'ĉ'),
    // --- 【d】 ---
    ("d/", 'đ'),
    // --- 【e】 ---
    ("e`", 'è'),
    ("e'", 'é'),
    ("e^", 'ê'),
    ("e:", 'ë'),
    ("e_", 'ē'),
    ("e~", 'ẽ'),
    // --- 【g】 ---
    ("g^", 'ĝ'),
    // --- 【h】 ---
    ("h^", 'ĥ'),
    ("h/", 'ħ'),
    // --- 【i】 ---
    ("i`", 'ì'),
    ("i'", 'í'),
    ("i^", 'î'),
    ("i:", 'ï'),
    ("i_", 'ī'),
    ("i/", 'ɨ'),
    ("i~", 'ĩ'),
    // --- 【j】 ---
    ("j^", 'ĵ'),
    // --- 【l】 ---
    ("l/", 'ł'),
    ("l'", 'ĺ'),
    // --- 【m】 ---
    ("m'", 'ḿ'),
    // --- 【n】 ---
    ("n`", 'ǹ'),
    ("n~", 'ñ'),
    ("n'", 'ń'),
    // --- 【o】 ---
    ("o`", 'ò'),
    ("o'", 'ó'),
    ("o^", 'ô'),
    ("o~", 'õ'),
    ("o:", 'ö'),
    ("o/", 'ø'),
    ("o_", 'ō'),
    // --- 【r】 ---
    ("r'", 'ŕ'),
    // --- 【s】 ---
    ("s'", 'ś'),
    ("s,", 'ş'),
    ("s^", 'ŝ'),
    // --- 【t】 ---
    ("t,", 'ţ'),
    // --- 【u】 ---
    ("u`", 'ù'),
    ("u'", 'ú'),
    ("u^", 'û'),
    ("u:", 'ü'),
    ("u_", 'ū'),
    ("u&", 'ů'),
    ("u~", 'ũ'),
    // --- 【y】 ---
    ("y'", 'ý'),
    ("y:", 'ÿ'),
    // --- 【z】 ---
    ("z'", 'ź'),
    // --- 【A】 ---
    ("A`", 'À'),
    ("A'", 'Á'),
    ("A^", 'Â'),
    ("A~", 'Ã'),
    ("A:", 'Ä'),
    ("A&", 'Å'),
    ("A_", 'Ā'),
    // --- 【C】 ---
    ("C,", 'Ç'),
    ("C'", 'Ć'),
    ("C^", 'Ĉ'),
    // --- 【D】 ---
    ("D/", 'Đ'),
    // --- 【E】 ---
    ("E`", 'È'),
    ("E'", 'É'),
    ("E^", 'Ê'),
    ("E:", 'Ë'),
    ("E_", 'Ē'),
    ("E~", 'Ẽ'),
    // --- 【G】 ---
    ("G^", 'Ĝ'),
    // --- 【H】 ---
    ("H^", 'Ĥ'),
    // --- 【I】 ---
    ("I`", 'Ì'),
    ("I'", 'Í'),
    ("I^", 'Î'),
    ("I:", 'Ï'),
    ("I_", 'Ī'),
    ("I~", 'Ĩ'),
    // --- 【J】 ---
    ("J^", 'Ĵ'),
    // --- 【L】 ---
    ("L/", 'Ł'),
    ("L'", 'Ĺ'),
    // --- 【M】 ---
    ("M'", 'Ḿ'),
    // --- 【N】 ---
    ("N`", 'Ǹ'),
    ("N~", 'Ñ'),
    ("N'", 'Ń'),
    // --- 【O】 ---
    ("O`", 'Ò'),
    ("O'", 'Ó'),
    ("O^", 'Ô'),
    ("O~", 'Õ'),
    ("O:", 'Ö'),
    ("O/", 'Ø'),
    ("O_", 'Ō'),
    // --- 【R】 ---
    ("R'", 'Ŕ'),
    // --- 【S】 ---
    ("S'", 'Ś'),
    ("S,", 'Ş'),
    ("S^", 'Ŝ'),
    // --- 【T】 ---
    ("T,", 'Ţ'),
    // --- 【U】 ---
    ("U`", 'Ù'),
    ("U'", 'Ú'),
    ("U^", 'Û'),
    ("U:", 'Ü'),
    ("U_", 'Ū'),
    ("U&", 'Ů'),
    ("U~", 'Ũ'),
    // --- 【Y】 ---
    ("Y'", 'Ý'),
    // --- 【Z】 ---
    ("Z'", 'Ź'),
];

const COMPOSED_LETTER_BOUNDS: (char, char) = {
    let mut bounds = (char::MAX, '\0');
    let mut i = 0;
    while i < ACCENT_TABLE.len() {
        let ch = ACCENT_TABLE[i].1;
        if ch < bounds.0 {
            bounds.0 = ch;
        }
        if ch > bounds.1 {
            bounds.1 = ch;
        }
        i += 1;
    }
    bounds
};

pub(crate) const fn is_composed_letter(ch: char) -> bool {
    if ch < COMPOSED_LETTER_BOUNDS.0 || ch > COMPOSED_LETTER_BOUNDS.1 {
        return false;
    }
    let mut i = 0;
    while i < ACCENT_TABLE.len() {
        if ACCENT_TABLE[i].1 == ch {
            return true;
        }
        i += 1;
    }
    false
}

/// ASCII characters used as accent markers in the spec.
///
/// Downstream consumers enumerate the marker bytes from this slice; runtime
/// membership checks use the `u128` bitmap `ACCENT_MARKER_MASK`, which
/// compiles to one shift and one AND.
pub const ACCENT_MARKERS: &[u8] = b"'`^:~&,/_";

/// 128-bit bitmap of [`ACCENT_MARKERS`] for branchless ASCII membership
/// testing. Bit `n` is 1 iff byte `n` is an accent marker. Computed at
/// compile time from [`ACCENT_MARKERS`].
const ACCENT_MARKER_MASK: u128 = {
    let mut m: u128 = 0;
    let bs = ACCENT_MARKERS;
    let mut i = 0;
    while i < bs.len() {
        // All marker bytes are < 128 (ASCII). Compile-time-asserted by
        // the const block below.
        m |= 1u128 << bs[i];
        i += 1;
    }
    m
};

const _: () = {
    // A non-ASCII marker does not fit the u128 bitmap; this assertion fails
    // the build if `ACCENT_MARKERS` contains one.
    let bs = ACCENT_MARKERS;
    let mut i = 0;
    while i < bs.len() {
        assert!(bs[i] < 128, "ACCENT_MARKERS must stay ASCII-only");
        i += 1;
    }
};

/// Branchless membership test against [`ACCENT_MARKERS`].
///
/// Compiles to `(b < 128) & ((MASK >> b) & 1)`: one cmp, one shift,
/// one AND, with no memory load, no loop, and no branch.
#[inline]
#[must_use]
pub const fn is_accent_marker(b: u8) -> bool {
    // Shifting a u128 by 128 or more overflows; the `b < 128` guard
    // short-circuits before the shift for a non-ASCII byte.
    (b < 128) && ((ACCENT_MARKER_MASK >> b) & 1) != 0
}

/// 3-byte ligatures (ASCII keys → Latin char). For four entries a `match`
/// costs less than `phf::Map`: the compiler lowers it to a small jump table
/// with the keys as immediates, the common ASCII miss path is predictable,
/// and no static array is loaded.
#[inline]
fn match_ligature(head: &[u8]) -> Option<char> {
    debug_assert_eq!(head.len(), 3, "match_ligature requires exactly 3 bytes");
    match head {
        b"ae&" => Some('æ'),
        b"AE&" => Some('Æ'),
        b"oe&" => Some('œ'),
        b"OE&" => Some('Œ'),
        _ => None,
    }
}

/// 2-byte digraphs as a compile-time perfect hash table. 110 entries,
/// `&[u8]` keys (the 2 ASCII bytes), `char` values. `phf::Map::get` is
/// O(1) with a constant bound on key comparisons.
static ACCENT_DIGRAPHS: phf::Map<&'static [u8], char> = phf::phf_map! {
    // The spec page groups s& with the ligatures, but its key is 2 bytes,
    // so the digraph map holds it.
    b"s&" => 'ß',
    // --- 【a】 ---
    b"a`" => 'à', b"a'" => 'á', b"a^" => 'â', b"a~" => 'ã',
    b"a:" => 'ä', b"a&" => 'å', b"a_" => 'ā',
    // --- 【c】 ---
    b"c," => 'ç', b"c'" => 'ć', b"c^" => 'ĉ',
    // --- 【d】 ---
    b"d/" => 'đ',
    // --- 【e】 ---
    b"e`" => 'è', b"e'" => 'é', b"e^" => 'ê', b"e:" => 'ë',
    b"e_" => 'ē', b"e~" => 'ẽ',
    // --- 【g】 ---
    b"g^" => 'ĝ',
    // --- 【h】 ---
    b"h^" => 'ĥ', b"h/" => 'ħ',
    // --- 【i】 ---
    b"i`" => 'ì', b"i'" => 'í', b"i^" => 'î', b"i:" => 'ï',
    b"i_" => 'ī', b"i/" => 'ɨ', b"i~" => 'ĩ',
    // --- 【j】 ---
    b"j^" => 'ĵ',
    // --- 【l】 ---
    b"l/" => 'ł', b"l'" => 'ĺ',
    // --- 【m】 ---
    b"m'" => 'ḿ',
    // --- 【n】 ---
    b"n`" => 'ǹ', b"n~" => 'ñ', b"n'" => 'ń',
    // --- 【o】 ---
    b"o`" => 'ò', b"o'" => 'ó', b"o^" => 'ô', b"o~" => 'õ',
    b"o:" => 'ö', b"o/" => 'ø', b"o_" => 'ō',
    // --- 【r】 ---
    b"r'" => 'ŕ',
    // --- 【s】 ---
    b"s'" => 'ś', b"s," => 'ş', b"s^" => 'ŝ',
    // --- 【t】 ---
    b"t," => 'ţ',
    // --- 【u】 ---
    b"u`" => 'ù', b"u'" => 'ú', b"u^" => 'û', b"u:" => 'ü',
    b"u_" => 'ū', b"u&" => 'ů', b"u~" => 'ũ',
    // --- 【y】 ---
    b"y'" => 'ý', b"y:" => 'ÿ',
    // --- 【z】 ---
    b"z'" => 'ź',
    // --- 【A】 ---
    b"A`" => 'À', b"A'" => 'Á', b"A^" => 'Â', b"A~" => 'Ã',
    b"A:" => 'Ä', b"A&" => 'Å', b"A_" => 'Ā',
    // --- 【C】 ---
    b"C," => 'Ç', b"C'" => 'Ć', b"C^" => 'Ĉ',
    // --- 【D】 ---
    b"D/" => 'Đ',
    // --- 【E】 ---
    b"E`" => 'È', b"E'" => 'É', b"E^" => 'Ê', b"E:" => 'Ë',
    b"E_" => 'Ē', b"E~" => 'Ẽ',
    // --- 【G】 ---
    b"G^" => 'Ĝ',
    // --- 【H】 ---
    b"H^" => 'Ĥ',
    // --- 【I】 ---
    b"I`" => 'Ì', b"I'" => 'Í', b"I^" => 'Î', b"I:" => 'Ï',
    b"I_" => 'Ī', b"I~" => 'Ĩ',
    // --- 【J】 ---
    b"J^" => 'Ĵ',
    // --- 【L】 ---
    b"L/" => 'Ł', b"L'" => 'Ĺ',
    // --- 【M】 ---
    b"M'" => 'Ḿ',
    // --- 【N】 ---
    b"N`" => 'Ǹ', b"N~" => 'Ñ', b"N'" => 'Ń',
    // --- 【O】 ---
    b"O`" => 'Ò', b"O'" => 'Ó', b"O^" => 'Ô', b"O~" => 'Õ',
    b"O:" => 'Ö', b"O/" => 'Ø', b"O_" => 'Ō',
    // --- 【R】 ---
    b"R'" => 'Ŕ',
    // --- 【S】 ---
    b"S'" => 'Ś', b"S," => 'Ş', b"S^" => 'Ŝ',
    // --- 【T】 ---
    b"T," => 'Ţ',
    // --- 【U】 ---
    b"U`" => 'Ù', b"U'" => 'Ú', b"U^" => 'Û', b"U:" => 'Ü',
    b"U_" => 'Ū', b"U&" => 'Ů', b"U~" => 'Ũ',
    // --- 【Y】 ---
    b"Y'" => 'Ý',
    // --- 【Z】 ---
    b"Z'" => 'Ź',
};

const _: () = {
    // The runtime tables hold the 114 spec entries: 4 ligatures in
    // `match_ligature` and 110 digraphs. The assertion fails the build when
    // a digraph entry is missing or added.
    assert!(
        ACCENT_DIGRAPHS.len() == 110,
        "ACCENT_DIGRAPHS must contain exactly 110 entries (114 spec − 4 ligatures)"
    );
};

/// Decompose Aozora accent digraphs anywhere inside `fragment`.
///
/// Call this only on the body of a `〔...〕` span; outside that convention,
/// English text (`isn't`, `text,`, `word's`) would match spec entries
/// (`n'`=ń, `t,`=ţ, and others).
///
/// Guarantees:
/// - Returns `Cow::Borrowed(fragment)`, without allocating, when no accent
///   marker byte appears, as in Japanese-only text.
/// - Greedy longest match: a ligature (3-byte, e.g. `ae&` = æ) takes
///   precedence over a 2-byte digraph that shares its prefix (`a&` = å).
/// - A 2-byte digraph whose target lies in U+1Exx (`m'` = ḿ, `e~` = ẽ)
///   produces 3 output bytes; a 3-byte ligature produces 2. The result is
///   always a valid UTF-8 string.
///
/// Running time is linear in `fragment.len()`: the scan reads the bytes left
/// to right, inspects at most 3 bytes at each position, and emits the
/// longest table match.
#[must_use]
pub fn decompose_fragment(fragment: &str) -> Cow<'_, str> {
    let bytes = fragment.as_bytes();
    // If no accent marker byte appears, the output equals the input byte for
    // byte and is returned borrowed.
    //
    // The membership test uses the [`ACCENT_MARKER_MASK`] u128 bitmap, one
    // compare, shift and AND per byte. A SIMD or memchr-style prefilter does
    // not reduce the candidate set, because Aozora text is predominantly
    // Japanese (3-byte UTF-8 with an 0xE3 lead byte).
    if !bytes.iter().any(|b| is_accent_marker(*b)) {
        return Cow::Borrowed(fragment);
    }

    let mut out = String::with_capacity(fragment.len());
    let mut i = 0;
    while i < bytes.len() {
        if let Some((pat_len, ch)) = try_match(bytes, i) {
            out.push(ch);
            i += pat_len;
        } else {
            // Advance one UTF-8 scalar value. Every index is a char boundary
            // because the loop advances only by `pat_len` (2 or 3 ASCII
            // bytes) or by `ch.len_utf8()`. `.get(i..)` satisfies
            // `clippy::string_slice` and returns `None` on a misaligned
            // index, which ends the loop.
            let Some(ch) = fragment.get(i..).and_then(|s| s.chars().next()) else {
                break;
            };
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    Cow::Owned(out)
}

/// Per-digraph length-changing edits made by [`decompose_fragment`],
/// for callers that back-map offsets across the rewrite (see the module
/// note on the per-position delta).
///
/// Each entry is `(in_off, in_len, out_len)`: the `in_len` input bytes at
/// byte offset `in_off` (relative to `fragment`) become `out_len` output
/// bytes. Only digraphs whose UTF-8 output length differs from the ASCII
/// source are reported; length-preserving substitutions (`s&` = ß, 2→2)
/// shift no later offset and are omitted, so the common case returns an
/// empty `Vec`.
///
/// ```
/// use ab_aozora_syntax::accent::decompose_fragment_edits;
/// // `ae&` (3 bytes) → æ (2 bytes): a −1 shift at offset 0.
/// assert_eq!(decompose_fragment_edits("ae&on"), vec![(0, 3, 2)]);
/// // `e~` (2 bytes) → ẽ (3 bytes): a +1 shift.
/// assert_eq!(decompose_fragment_edits("e~a"), vec![(0, 2, 3)]);
/// // `s&` (2 bytes) → ß (2 bytes): length-preserving, omitted.
/// assert!(decompose_fragment_edits("stras&e").is_empty());
/// ```
#[must_use]
pub fn decompose_fragment_edits(fragment: &str) -> Vec<(usize, usize, usize)> {
    decompose_fragment_sites(fragment)
        .into_iter()
        .filter(|&(_, in_len, ch)| in_len != ch.len_utf8())
        .map(|(off, in_len, ch)| (off, in_len, ch.len_utf8()))
        .collect()
}

/// Every substitution [`decompose_fragment`] makes, length-preserving ones
/// included.
///
/// Each entry is `(in_off, in_len, replacement)`: the `in_len` input bytes at
/// `in_off` become `replacement`. [`decompose_fragment_edits`] is this list
/// filtered to the length-changing entries. Callers that need the
/// substitution sites rather than the offset deltas use the unfiltered list:
/// the sanitize stage records one diagnostic and one offset-map edit per site,
/// so bytes between sites keep their source positions. A single edit for the
/// whole span would map every interior position to the span start.
///
/// ```
/// use ab_aozora_syntax::accent::decompose_fragment_sites;
/// // `s&` = ß is length-preserving (2→2) and still a site.
/// assert_eq!(decompose_fragment_sites("stras&e"), vec![(4, 2, 'ß')]);
/// // A ligature site and a growing site.
/// assert_eq!(decompose_fragment_sites("ae&e~"), vec![(0, 3, 'æ'), (3, 2, 'ẽ')]);
/// ```
#[must_use]
pub fn decompose_fragment_sites(fragment: &str) -> Vec<(usize, usize, char)> {
    let bytes = fragment.as_bytes();
    let mut sites = Vec::new();
    if !bytes.iter().any(|b| is_accent_marker(*b)) {
        return sites;
    }
    let mut i = 0;
    while i < bytes.len() {
        if let Some((pat_len, ch)) = try_match(bytes, i) {
            sites.push((i, pat_len, ch));
            i += pat_len;
        } else {
            let Some(ch) = fragment.get(i..).and_then(|s| s.chars().next()) else {
                break;
            };
            i += ch.len_utf8();
        }
    }
    sites
}

/// Byte offset and length (always 2) of each digraph key that is not composed.
///
/// A key is an `ACCENT_DIGRAPHS` entry in `fragment` whose context rule
/// rejects it: a `,` digraph unless its base is `c`/`C` and an ASCII letter
/// follows, or a `'` digraph whose base is not a vowel or `y`.
/// [`decompose_fragment_sites`] does not compose these keys.
#[must_use]
pub fn unresolved_accent_sites(fragment: &str) -> Vec<(usize, usize)> {
    let bytes = fragment.as_bytes();
    bytes
        .windows(2)
        .enumerate()
        .filter(|&(i, spelling)| {
            ACCENT_DIGRAPHS.contains_key(spelling) && digraph_is_unresolved(bytes, i)
        })
        .map(|(i, _)| (i, 2))
        .collect()
}

fn digraph_is_unresolved(bytes: &[u8], i: usize) -> bool {
    !digraph_applies(bytes[i], bytes[i + 1], bytes.get(i + 2).copied())
}

#[inline]
fn try_match(bytes: &[u8], i: usize) -> Option<(usize, char)> {
    if i + 3 <= bytes.len()
        && let Some(ch) = match_ligature(&bytes[i..i + 3])
    {
        return Some((3, ch));
    }
    if i + 2 <= bytes.len()
        && let Some(&ch) = ACCENT_DIGRAPHS.get(&bytes[i..i + 2])
        && !digraph_is_unresolved(bytes, i)
    {
        return Some((2, ch));
    }
    None
}

#[inline]
const fn digraph_applies(base: u8, marker: u8, next: Option<u8>) -> bool {
    match marker {
        b',' => matches!(base, b'c' | b'C') && matches!(next, Some(b) if b.is_ascii_alphabetic()),
        b'\'' => matches!(
            base.to_ascii_lowercase(),
            b'a' | b'e' | b'i' | b'o' | b'u' | b'y'
        ),
        _ => true,
    }
}

/// Compose a single Latin `letter` with an accent `mark` into its precomposed
/// glyph, reusing the `〔…〕` accent digraph table ([`ACCENT_TABLE`], via its
/// `ACCENT_DIGRAPHS` perfect-hash mirror).
///
/// The forward accent directive (`「e」はアクサン（´）付き` → é) gives its mark
/// as a word, so this function maps [`AccentMark`] to the table's ASCII marker
/// byte (Acute → `'`, Umlaut → `:`, Grave → `` ` ``), builds the 2-byte key,
/// and looks it up. Returns `None` when `letter` is not ASCII-alphabetic or the
/// `(letter, mark)` pair has no precomposed form (e.g. `q` + acute); the
/// classifier then produces `Directive{Unknown}` and the renderer emits the
/// letter unstyled. The forward-accent classifier and renderer both call this
/// function; [`compose_dotted`] has the same role for `AccentDot`.
#[must_use]
pub fn compose_accent(letter: char, mark: AccentMark) -> Option<char> {
    let base = u8::try_from(letter).ok().filter(u8::is_ascii_alphabetic)?;
    let marker = match mark {
        AccentMark::Acute => b'\'',
        AccentMark::Umlaut => b':',
        AccentMark::Grave => b'`',
    };
    ACCENT_DIGRAPHS.get(&[base, marker][..]).copied()
}

// Dotted-letter composition (ドット付き) is separate from the `〔…〕` digraph
// decomposition above.
//
// The `［＃mは上ドット付き］` directive family addresses a base Latin letter in
// the immediately preceding run and specifies a combining dot above or below
// it (`m` → ṁ, `s` → ṣ). The input is the directive body's selector grammar,
// not an inline marker as in the `〔…〕` ASCII-digraph scheme, so the
// forward-reference classifier and renderer call this code and
// `decompose_fragment` does not. Every attested `(letter, dot)` pair has a
// single NFC precomposed scalar, so no combining-mark (U+0307 / U+0323)
// fallback is needed. `accent.rs` is the one module that maps a Latin letter
// and diacritic to a precomposed glyph.

/// Position of the combining dot in a dotted-letter directive:
/// `上ドット付き` (above) or `下ドット付き` (below).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DotPosition {
    /// 上ドット: combining dot above (NFC-composed, e.g. `m` → ṁ U+1E41).
    Above,
    /// 下ドット: combining dot below (NFC-composed, e.g. `s` → ṣ U+1E63).
    Below,
}

/// `(base ASCII letter, DotPosition)` → precomposed Unicode glyph.
///
/// The table holds only the pairs attested in the 17,889-work
/// `aozorabunko_text` mirror (above: `m`, `n`; below: `s t h r n d m` +
/// capitals `T`, `R`). Each pair has a single precomposed scalar whose NFD
/// decomposition was verified as base + U+0307/U+0323, so composition needs
/// no combining-mark fallback.
/// Case-preserving: `t` → ṭ, `T` → Ṭ.
pub const ACCENT_DOT_TABLE: &[(char, DotPosition, char)] = &[
    ('m', DotPosition::Above, 'ṁ'),
    ('n', DotPosition::Above, 'ṅ'),
    ('m', DotPosition::Below, 'ṃ'),
    ('n', DotPosition::Below, 'ṇ'),
    ('s', DotPosition::Below, 'ṣ'),
    ('t', DotPosition::Below, 'ṭ'),
    ('h', DotPosition::Below, 'ḥ'),
    ('r', DotPosition::Below, 'ṛ'),
    ('d', DotPosition::Below, 'ḍ'),
    ('T', DotPosition::Below, 'Ṭ'),
    ('R', DotPosition::Below, 'Ṛ'),
];

const _: () = {
    // The assertion fails the build when an entry is missing or duplicated,
    // as the `ACCENT_DIGRAPHS` size assertion does.
    assert!(
        ACCENT_DOT_TABLE.len() == 11,
        "ACCENT_DOT_TABLE must contain exactly 11 corpus-attested entries"
    );
};

/// Compose a base letter with a dot at `pos` into its precomposed glyph.
///
/// Case-preserving (`t` → ṭ, `T` → Ṭ); returns `None` for any `(letter, pos)`
/// pair not in [`ACCENT_DOT_TABLE`] (e.g. an uppercase `S`-below, which the
/// corpus does not contain). For 11 entries a linear scan costs less than a
/// map lookup.
#[must_use]
pub fn compose_dotted(base: char, pos: DotPosition) -> Option<char> {
    ACCENT_DOT_TABLE
        .iter()
        .find(|&&(b, p, _)| b == base && p == pos)
        .map(|&(_, _, glyph)| glyph)
}

/// Which occurrence of the addressed letter (within the preceding run) a
/// clause selects. Explicit ordinal counting is case-insensitive (an uppercase
/// `S` counts toward a lowercase `s` ordinal); literal selectors retain case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Occ {
    /// A bare selector identifies one matching letter without an occurrence index.
    Unique,
    /// `ともに` / `それぞれ` explicitly selects all matching letters.
    All,
    /// `前の`: the first occurrence.
    First,
    /// `Nつめの`: the N-th occurrence (1-indexed).
    Nth(usize),
    /// `最後の`: the last occurrence.
    Last,
}

/// One resolved substitution instruction: dot `letter`'s `occ`-th occurrence
/// at `pos`. Parsed from the directive body, applied against the run.
#[derive(Debug, Clone, Copy)]
struct DotOp {
    letter: char,
    pos: DotPosition,
    occ: Occ,
}

/// Parse a leading decimal (ASCII `0-9` or fullwidth `０-９`) off `s`, returning
/// the value and the remainder. `None` when `s` has no leading digit.
fn parse_leading_number(s: &str) -> Option<(usize, &str)> {
    let mut n: usize = 0;
    let mut end = 0;
    for (i, ch) in s.char_indices() {
        let digit = match ch {
            '0'..='9' => ch as usize - '0' as usize,
            '０'..='９' => ch as usize - '０' as usize,
            _ => break,
        };
        n = n.checked_mul(10)?.checked_add(digit)?;
        end = i + ch.len_utf8();
    }
    (end != 0).then(|| (n, &s[end..]))
}

/// Parse a selector `[ordinal] letters` into an occurrence rule + the trailing
/// ASCII letter run. `None` if no letters follow the ordinal.
///
/// `前の` (former) / `後の` (latter) refer to the earlier / later occurrence of a
/// letter that appears twice across a `、`-joined clause pair
/// (`前のn…、後のn…`), so they map to the first / last occurrence. A bare
/// selector must identify one literal matching letter; repeated matches do not
/// imply an occurrence order.
fn parse_selector(sel: &str) -> Option<(Occ, &str)> {
    if let Some(rest) = sel.strip_prefix("最後の") {
        return Some((Occ::Last, rest));
    }
    if let Some(rest) = sel.strip_prefix("前の") {
        return Some((Occ::First, rest));
    }
    if let Some(rest) = sel.strip_prefix("後の") {
        return Some((Occ::Last, rest));
    }
    if let Some((n, rest)) = parse_leading_number(sel) {
        // A bare number with no `つめの` (or `つめの` with no letters) is not a
        // selector; return `None`.
        let letters = rest.strip_prefix("つめの")?;
        return Some((Occ::Nth(n), letters));
    }
    Some((Occ::Unique, sel))
}

/// Parse one clause `<selector>は[ともに|それぞれ]<上|下>ドット付き` into ops.
///
/// A cluster selector (`stはともに…`) yields one op per selected letter.
/// `ともに` / `それぞれ` selects all matching occurrences, including repeated
/// letters under a single-letter selector. Returns `None` for a letter/pos pair
/// absent from [`ACCENT_DOT_TABLE`] and for an ordinal applied to a
/// multi-letter cluster (not attested).
fn parse_accent_clause(clause: &str) -> Option<Vec<DotOp>> {
    let (after_letters, quoted_point_spelling) = clause
        .strip_suffix("ドット付き")
        .map(|body| (body, false))
        .or_else(|| clause.strip_suffix("点付き").map(|body| (body, true)))?;
    let (selector, tail) = after_letters.split_once('は')?;
    let selector = if quoted_point_spelling {
        selector.strip_prefix('「')?.strip_suffix('」')?
    } else {
        selector
    };
    let all = tail.starts_with("ともに") || tail.starts_with("それぞれ");
    let posword = tail
        .strip_prefix("ともに")
        .or_else(|| tail.strip_prefix("それぞれ"))
        .unwrap_or(tail);
    let pos = match posword {
        "上" => DotPosition::Above,
        "下" => DotPosition::Below,
        _ => return None,
    };
    let (mut occ, letters) = parse_selector(selector)?;
    if all {
        if occ != Occ::Unique {
            return None;
        }
        occ = Occ::All;
    }
    if letters.is_empty() || !letters.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let chars: Vec<char> = letters.chars().collect();
    // An ordinal selects a single occurrence, so it cannot apply to a cluster.
    if chars.len() > 1 && !matches!(occ, Occ::Unique | Occ::All) {
        return None;
    }
    let mut ops = Vec::with_capacity(chars.len());
    for &letter in &chars {
        // If any addressed letter has no dotted form at this position, the
        // whole clause returns `None` and the directive becomes `Unknown` with
        // its bytes unchanged.
        compose_dotted(letter, pos)?;
        ops.push(DotOp { letter, pos, occ });
    }
    Some(ops)
}

/// Parse a dotted-letter directive body into substitution ops.
///
/// A body may be one clause or several `。` / `、`-joined clauses
/// (`mは上ドット付き。２つめのsは下ドット付き`); every clause addresses the same
/// reclaimed run, so their ops are concatenated and applied together. A
/// malformed clause makes the whole body return `None`; word-qualified
/// (`simhaのm…`) and `段目` table-row forms return `None` this way, because
/// their `、`-split pieces are not pure ASCII-letter selectors.
fn parse_accent_dot_body(body: &str) -> Option<Vec<DotOp>> {
    let body = body.rsplit_once('、').map_or(body, |(head, tail)| {
        let mut parts = tail.split('-');
        if head.starts_with('「')
            && parts.next().is_some_and(|part| {
                !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())
            })
            && parts.next().is_some_and(|part| {
                !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())
            })
            && parts.next().is_none()
        {
            head
        } else {
            body
        }
    });
    let mut ops = Vec::new();
    for clause in body.split(['。', '、']) {
        ops.extend(parse_accent_clause(clause)?);
    }
    (!ops.is_empty()).then_some(ops)
}

/// Compose the dotted-letter substitutions described by directive `body` onto
/// the reclaimed preceding `run`.
///
/// Returns the run with each addressed letter replaced by its precomposed
/// dotted glyph (`Sam` + `mは上ドット付き` → `Saṁ`), or `None` when `body` is
/// not a recognized dotted directive or an addressed occurrence
/// is absent / not composable in `run`. The classifier and the renderer both
/// call this function: the classifier accepts the directive when it returns
/// `Some`, and the renderer emits the returned string as the visible glyphs.
///
/// A body outside the dotted grammar is read as a supplied-mark note, whose
/// `run` is the text from [`supplied_mark_target_start`] to the note.
///
/// Every op is resolved to an absolute byte index in `run` before any
/// substitution, so an earlier substitution does not shift a later op's index.
#[must_use]
pub fn compose_accent_dots(run: &str, body: &str) -> Option<String> {
    let Some(ops) = parse_accent_dot_body(body) else {
        return supplied_marks::compose(run, body);
    };
    let mut subs: Vec<(usize, char, usize)> = Vec::with_capacity(ops.len());
    for op in &ops {
        if op.occ == Occ::All {
            let before = subs.len();
            for (index, base) in run
                .char_indices()
                .filter(|(_, letter)| *letter == op.letter)
            {
                subs.push((index, compose_dotted(base, op.pos)?, base.len_utf8()));
            }
            if subs.len() == before {
                return None;
            }
            continue;
        }
        let idx = resolve_occurrence(run, op.letter, op.pos, op.occ)?;
        let base = run[idx..].chars().next()?;
        let glyph = compose_dotted(base, op.pos)?;
        subs.push((idx, glyph, base.len_utf8()));
    }
    subs.sort_by_key(|&(idx, _, _)| idx);
    let mut out = String::with_capacity(run.len());
    let mut last = 0;
    for (idx, glyph, base_len) in subs {
        // Two ops that resolve to the same char would overlap; return `None`.
        if idx < last {
            return None;
        }
        out.push_str(&run[last..idx]);
        out.push(glyph);
        last = idx + base_len;
    }
    out.push_str(&run[last..]);
    Some(out)
}

/// Byte index of the addressed occurrence of `letter` in `run`, subject to
/// composability. Counting is case-insensitive (`S` counts toward a lowercase
/// `s`).
///
/// A bare selector requires one case-sensitive occurrence. Explicit first/last
/// selectors count composable matches; an `Nつめの`
/// ordinal instead counts every case-insensitive occurrence (a capital `S`
/// is position 1 for `２つめのs` over `Sāraksā`); the counted position must
/// itself be composable, otherwise the function returns `None`.
fn resolve_occurrence(run: &str, letter: char, pos: DotPosition, occ: Occ) -> Option<usize> {
    let target = letter.to_ascii_lowercase();
    let composable = |idx: usize| {
        run[idx..]
            .chars()
            .next()
            .and_then(|c| compose_dotted(c, pos))
            .is_some()
    };
    let mut hits = run
        .char_indices()
        .filter(|(_, c)| c.to_ascii_lowercase() == target)
        .map(|(i, _)| i);
    match occ {
        Occ::All => None,
        Occ::Unique => {
            let mut exact = run
                .char_indices()
                .filter(|(_, character)| *character == letter);
            let (index, _) = exact.next()?;
            (exact.next().is_none() && composable(index)).then_some(index)
        }
        Occ::First => hits.find(|&i| composable(i)),
        Occ::Last => hits.rfind(|&i| composable(i)),
        Occ::Nth(n) => n
            .checked_sub(1)
            .and_then(|k| hits.nth(k))
            .filter(|&i| composable(i)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_size_is_pinned_to_spec_count() {
        // Counted 2026-04-23 against <https://www.aozora.gr.jp/accent_separation.html>
        // (archived at docs/specs/aozora/accent_separation.html) by enumerating
        // every ASCII digraph and ligature in the 【a..z】, 【A..Z】, and 【合字】
        // groups. A lower count means table entries are missing; a higher count
        // is valid only after the spec adds entries.
        const EXPECTED: usize = 114;
        assert_eq!(
            ACCENT_TABLE.len(),
            EXPECTED,
            "spec count drift: see docs/specs/aozora/accent_separation.html"
        );
    }

    #[test]
    fn every_table_entry_is_representable_ascii_source() {
        for (pat, _) in ACCENT_TABLE {
            assert!(
                pat.is_ascii(),
                "digraph {pat:?} must be pure ASCII per spec"
            );
            assert!(
                pat.len() == 2 || pat.len() == 3,
                "digraph {pat:?} must be 2 or 3 bytes"
            );
        }
    }

    #[test]
    fn every_table_entry_has_unique_pattern() {
        use std::collections::HashSet;
        let mut seen: HashSet<&str> = HashSet::new();
        for (pat, _) in ACCENT_TABLE {
            assert!(seen.insert(pat), "duplicate digraph {pat:?}");
        }
    }

    #[test]
    fn digraph_size_growth_stays_within_one_extra_byte() {
        // Some entries grow (`m'` = ḿ U+1E3F grows 2 → 3 bytes), but none grows
        // by more than one byte, so callers that back-map diagnostic spans
        // allocate at most `input_len + count_of_digraphs` output bytes.
        for (pat, ch) in ACCENT_TABLE {
            let out_len = ch.len_utf8();
            let in_len = pat.len();
            let growth = out_len.saturating_sub(in_len);
            assert!(
                growth <= 1,
                "digraph {pat:?} → {ch} grew by {growth} bytes (cap is 1)"
            );
        }
    }

    // --- Spec examples sampled across the table's groups ---

    #[test]
    fn spec_point_e_grave() {
        assert_eq!(decompose_fragment("fune`bre"), "funèbre");
    }

    #[test]
    fn spec_point_acute_accents() {
        assert_eq!(decompose_fragment("ve'rite'"), "vérité");
    }

    #[test]
    fn spec_point_circumflex_and_cedilla_together() {
        assert_eq!(decompose_fragment("C,a va^"), "Ça vâ");
    }

    #[test]
    fn spec_point_all_vowel_graves() {
        assert_eq!(decompose_fragment("a` e` i` o` u`"), "à è ì ò ù");
    }

    #[test]
    fn spec_point_uppercase_accents() {
        assert_eq!(decompose_fragment("A` E' N~"), "À É Ñ");
    }

    #[test]
    fn spec_point_ligatures_take_precedence_over_ring_above() {
        // `s&` = ß (eszett), not `s` + ring-above; longest-match ordering.
        assert_eq!(decompose_fragment("stras&e"), "straße");
        // A ligature takes precedence over a single-letter digraph: ae& = æ,
        // not a& + e.
        assert_eq!(decompose_fragment("ae&on"), "æon");
        assert_eq!(decompose_fragment("OE&uvre"), "Œuvre");
    }

    #[test]
    fn spec_point_stroke_and_macron() {
        assert_eq!(decompose_fragment("d/o_g"), "đōg");
    }

    #[test]
    fn input_without_any_marker_byte_is_borrowed() {
        // The input contains none of the ASCII markers: ' ` ^ : ~ & , / _
        let input = "plain Japanese prose ここはテストです 春夏秋冬";
        let out = decompose_fragment(input);
        assert!(
            matches!(out, Cow::Borrowed(_)),
            "expected zero-alloc path for {input:?}"
        );
        assert_eq!(out, input);
    }

    #[test]
    fn isolated_markers_not_preceded_by_table_base_are_preserved() {
        // A marker without a valid preceding base letter is preserved. Inside
        // a 〔〕 span, the call site, such a marker is an author typo or
        // punctuation.
        assert_eq!(decompose_fragment("'tis"), "'tis"); // leading apostrophe
        assert_eq!(decompose_fragment("5^2"), "5^2"); // digit base not in spec
        assert_eq!(decompose_fragment("q^"), "q^"); // q not in spec table
    }

    #[test]
    fn ungated_markers_are_greedy_for_any_valid_preceding_base() {
        // Markers that do not collide with prose punctuation follow the spec
        // rule: `<base-letter><marker>` decomposes, including where the
        // author may have meant punctuation (`` ` `` as a quote here).
        assert_eq!(decompose_fragment("`hello`"), "`hellò"); // o` → ò
    }

    #[test]
    fn cedilla_composes_only_on_c_before_a_letter() {
        // Word-internal c, is notation; a comma before space, end, or
        // punctuation is prose. Oracle-validated (52/3 vs 4/149).
        assert_eq!(decompose_fragment("garc,on"), "garçon");
        assert_eq!(decompose_fragment("Franc,ois"), "François");
        assert_eq!(decompose_fragment("LE MAC,ON"), "LE MAÇON");
        assert_eq!(decompose_fragment("text,"), "text,");
        assert_eq!(decompose_fragment("Films, 1930"), "Films, 1930");
        assert_eq!(decompose_fragment("hot,\ncold,"), "hot,\ncold,");
        assert_eq!(decompose_fragment("C,ri^bho^dja"), "Çrîbhôdja");
        // A comma after s or t is prose: the corpus spellings are clause
        // boundaries without a space, and the published XHTML keeps the comma.
        assert_eq!(decompose_fragment("nicht,ihr"), "nicht,ihr");
        assert_eq!(decompose_fragment("na:hert,belebt"), "nähert,belebt");
        assert_eq!(decompose_fragment("Re'musat,Nouveaux"), "Rémusat,Nouveaux");
        assert_eq!(unresolved_accent_sites("na:hert,belebt"), vec![(6, 2)]);
        assert_eq!(unresolved_accent_sites("Sciences,etc"), vec![(7, 2)]);
        assert!(unresolved_accent_sites("garc,on").is_empty());
    }

    #[test]
    fn acute_composes_only_on_a_vowel_base() {
        // French élision (and the English apostrophe) collide with the
        // consonant acute rows; vowel acutes are accents in every context,
        // word-final é included. Oracle-validated (765/3 vs 0/114).
        assert_eq!(decompose_fragment("ve'rite'"), "vérité");
        assert_eq!(decompose_fragment("L'art"), "L'art");
        assert_eq!(decompose_fragment("c'est"), "c'est");
        assert_eq!(decompose_fragment("s'e'prennent"), "s'éprennent");
        assert_eq!(decompose_fragment("isn't"), "isn't");
        // Accent notation is lexical: the official table maps u' even in a
        // French contraction, as the published corpus HTML does.
        assert_eq!(decompose_fragment("qu'on"), "qúon");
        // ý keeps its vowel-base entry.
        assert_eq!(decompose_fragment("Nagy'"), "Nagý");
    }

    #[test]
    fn unknown_base_letters_stay_unchanged() {
        // f doesn't have entries in the spec; f' must stay.
        assert_eq!(decompose_fragment("f'x"), "f'x");
        // q also absent.
        assert_eq!(decompose_fragment("q^"), "q^");
    }

    #[test]
    fn mixed_japanese_and_accents_round_trip_on_japanese() {
        assert_eq!(
            decompose_fragment("ここは fune`bre です"),
            "ここは funèbre です"
        );
    }

    #[test]
    fn empty_input_is_borrowed() {
        let out = decompose_fragment("");
        assert!(matches!(out, Cow::Borrowed("")));
    }

    #[test]
    fn three_byte_ligatures_shrink_output_byte_length() {
        // 3-byte ASCII ligature → 2-byte UTF-8: strictly shorter.
        // `s&` = ß is not a 3-byte ligature but a 2-byte digraph → 2 UTF-8
        // bytes, so length is preserved; see
        // `two_byte_eszett_preserves_output_byte_length`.
        for (input, expected) in [("ae&on", "æon"), ("OE&uvre", "Œuvre")] {
            let out = decompose_fragment(input);
            assert!(
                out.len() < input.len(),
                "3-byte ligature should shrink: {input:?} → {out:?}"
            );
            assert_eq!(out, expected);
        }
    }

    #[test]
    fn two_byte_eszett_preserves_output_byte_length() {
        // `s&` = ß is a 2-byte source → 2-byte UTF-8 output: neutral length.
        let out = decompose_fragment("stras&e");
        assert_eq!(out, "straße");
        assert_eq!(out.len(), "stras&e".len());
    }

    #[test]
    fn bmp_above_u1e00_digraphs_may_grow_output() {
        // `e~` → ẽ U+1EBD is 3 bytes. (`m'` → ḿ also grows, but it is a
        // consonant acute, which `digraph_applies` rejects in every context.)
        let out = decompose_fragment("e~a");
        assert_eq!(out, "ẽa");
        assert!(out.len() > "e~a".len());
    }

    #[test]
    fn sites_agree_with_decompose_and_edits_filter_them() {
        // Replaying the sites reproduces the output of decompose_fragment, and
        // the edits list equals the length-changing subset of the sites.
        for input in ["stras&e", "ae&on m'a", "ve'rite'", "text,", "plain", ""] {
            let sites = decompose_fragment_sites(input);
            let edits = decompose_fragment_edits(input);
            assert_eq!(
                edits,
                sites
                    .iter()
                    .copied()
                    .filter(|&(_, in_len, ch)| in_len != ch.len_utf8())
                    .map(|(off, in_len, ch)| (off, in_len, ch.len_utf8()))
                    .collect::<Vec<_>>(),
                "edits must be the length-changing sites for {input:?}"
            );
            let mut replayed = String::new();
            let mut cursor = 0;
            for &(off, in_len, ch) in &sites {
                replayed.push_str(&input[cursor..off]);
                replayed.push(ch);
                cursor = off + in_len;
            }
            replayed.push_str(&input[cursor..]);
            assert_eq!(replayed, decompose_fragment(input), "replay for {input:?}");
        }
    }

    #[test]
    fn property_all_table_entries_round_trip_in_admissible_context() {
        // Every table entry decomposes to its target char in a context that
        // satisfies its applicability gate, except the consonant acute rows
        // and the s/t cedilla rows, which no fragment context admits (every
        // corpus occurrence is élision, an apostrophe or a comma).
        for (pat, ch) in ACCENT_TABLE {
            let bytes = pat.as_bytes();
            let marker = bytes[bytes.len() - 1];
            let consonant_acute = marker == b'\''
                && !matches!(
                    bytes[0].to_ascii_lowercase(),
                    b'a' | b'e' | b'i' | b'o' | b'u' | b'y'
                );
            let non_c_cedilla = marker == b',' && !matches!(bytes[0], b'c' | b'C');
            if consonant_acute || non_c_cedilla {
                for context in [format!("_{pat}_"), format!("_{pat}a"), pat.to_string()] {
                    assert_eq!(
                        decompose_fragment(&context),
                        context,
                        "{pat:?} must never compose in a fragment"
                    );
                }
                continue;
            }
            // Cedilla requires a following letter; every other entry is
            // context-free. `a` satisfies both.
            let input = format!("_{pat}a");
            let out = decompose_fragment(&input);
            let expected: String = format!("_{ch}a");
            assert_eq!(*out, *expected, "pattern {pat:?} failed");
        }
    }

    // --- forward accent-mark composition ---

    #[test]
    fn compose_accent_maps_corpus_pairs() {
        assert_eq!(compose_accent('e', AccentMark::Acute), Some('é'));
        assert_eq!(compose_accent('o', AccentMark::Umlaut), Some('ö'));
        assert_eq!(compose_accent('a', AccentMark::Umlaut), Some('ä'));
        // Grave is corpus-absent but supported.
        assert_eq!(compose_accent('e', AccentMark::Grave), Some('è'));
        // Case-preserving via the shared table.
        assert_eq!(compose_accent('E', AccentMark::Acute), Some('É'));
    }

    #[test]
    fn compose_accent_declines_uncomposable() {
        // `q` has no accented form in the table.
        assert_eq!(compose_accent('q', AccentMark::Acute), None);
        // Non-ASCII / non-letter targets decline.
        assert_eq!(compose_accent('あ', AccentMark::Acute), None);
        assert_eq!(compose_accent('1', AccentMark::Umlaut), None);
        // `g` + umlaut is not a real pair (no `g:` entry).
        assert_eq!(compose_accent('g', AccentMark::Umlaut), None);
    }

    // --- dotted-letter composition ---

    #[test]
    fn dot_table_composes_case_preserving() {
        assert_eq!(compose_dotted('m', DotPosition::Above), Some('ṁ'));
        assert_eq!(compose_dotted('s', DotPosition::Below), Some('ṣ'));
        assert_eq!(compose_dotted('T', DotPosition::Below), Some('Ṭ'));
        // Pairs outside the table return `None` (the corpus has no uppercase
        // S-below).
        assert_eq!(compose_dotted('S', DotPosition::Below), None);
        assert_eq!(compose_dotted('m', DotPosition::Below), Some('ṃ'));
    }

    #[test]
    fn accent_dot_single_clause_bare() {
        assert_eq!(
            compose_accent_dots("Sam", "mは上ドット付き").as_deref(),
            Some("Saṁ")
        );
        assert_eq!(
            compose_accent_dots("Sas", "sは下ドット付き").as_deref(),
            Some("Saṣ")
        );
    }

    #[test]
    fn retrospective_quoted_dot_retains_source_locator() {
        assert_eq!(
            compose_accent_dots("tair", "「t」は下点付き、182-6").as_deref(),
            Some("ṭair")
        );
        assert_eq!(compose_accent_dots("tair", "「t」は下点付き、位置"), None);
    }

    #[test]
    fn accent_dot_reclaims_tortoise_span_verbatim_brackets() {
        // The `〔…〕` run keeps its brackets; only the addressed letter changes.
        assert_eq!(
            compose_accent_dots("〔Mīhr〕", "hは下ドット付き").as_deref(),
            Some("〔Mīḥr〕")
        );
    }

    #[test]
    fn accent_dot_ordinal_counts_case_insensitively() {
        // `２つめのs` over `Sisa`: S counts as 1, lowercase s as 2 → dot the s.
        assert_eq!(
            compose_accent_dots("Sisa", "２つめのsは下ドット付き").as_deref(),
            Some("Siṣa")
        );
        // `最後の` selects the last occurrence.
        assert_eq!(
            compose_accent_dots("mama", "最後のmは上ドット付き").as_deref(),
            Some("maṁa")
        );
    }

    #[test]
    fn accent_dot_cluster_with_set_adverb() {
        // `snはともに下ドット付き` dots the first s and the first n → Viṣṇu.
        assert_eq!(
            compose_accent_dots("Visnu", "snはともに下ドット付き").as_deref(),
            Some("Viṣṇu")
        );
    }

    #[test]
    fn accent_dot_multi_clause_composes() {
        // `。`-joined clauses all address the same reclaimed run.
        assert_eq!(
            compose_accent_dots("Samsa", "mは上ドット付き。２つめのsは下ドット付き").as_deref(),
            Some("Saṁṣa")
        );
    }

    #[test]
    fn accent_dot_former_latter_pair() {
        // `前の` / `後の` over the two n's of Konkana → first ṅ (above), last ṇ (below).
        assert_eq!(
            compose_accent_dots("Konkana", "前のnは上ドット付き、後のnは下ドット付き").as_deref(),
            Some("Koṅkaṇa")
        );
    }

    #[test]
    fn accent_dot_composes_word_qualified_and_declines_dangyou() {
        // `simhaのm` selects within the word through the supplied-mark grammar.
        assert_eq!(
            compose_accent_dots("Nara-simha-pota-varman", "simhaのmは上ドット付き").as_deref(),
            Some("Nara-siṁha-pota-varman")
        );
        // 段目 table-row form.
        assert_eq!(
            compose_accent_dots("Sinha", "７段目、Sinhaのnは上ドット付き"),
            None
        );
    }

    #[test]
    fn accent_dot_declines_absent_or_uncomposable_occurrence() {
        // Letter absent from the run.
        assert_eq!(compose_accent_dots("abc", "mは上ドット付き"), None);
        // Nth out of range.
        assert_eq!(compose_accent_dots("Sam", "２つめのmは上ドット付き"), None);
        // The run's only `s` is uppercase `S`, which has no dot-below form.
        assert_eq!(compose_accent_dots("Sax", "sは下ドット付き"), None);
    }
}
