//! Forked from <https://github.com/P4suta/aozora>
//! at rev 1a4f864603970983719655aa4af4525958ac2d38 (independent fork).
//! Upstream crate: aozora-syntax. License: MIT OR Apache-2.0 (see NOTICE).

//! AST type definitions for the aozora parser.
//!
//! # AST shape
//!
//! The only AST is the owned AST defined in [`mod@ast`]:
//! lifetime-free, `Copy`-able nodes whose string and run payloads are
//! `u32` handles into a flat `NodeStore` (a string interner plus
//! content / segment pools), deduplicated through the store's interner.
//! Public consumers (`ab_aozora_facade` meta crate, FFI / WASM / Python drivers,
//! CLI) parse via `ab_aozora_facade::Document::parse()` and walk the owned
//! `ast::Node` values.
//!
//! # Top-level surface
//!
//! Only the shared `Copy`-able payloads referenced by the owned
//! AST (`BoutenKind`, `BoutenPosition`, `Container`, `SectionKind`,
//! `HeadingKind`, `HeadingStyle`, `MarginNoteKind`, `RubySide`,
//! `DirectiveKind`) live at the top level. The attribute × scope
//! formatting model (`Format` / `ForwardAttr` / `LineFormat` /
//! `RegionFormat` / `RegionClose` and their `NonZero` parameters) lives
//! under [`mod@format`]. The owned AST node types live under
//! [`mod@ast`]; the builder under [`mod@alloc`].

#![forbid(unsafe_code)]

use core::num::NonZeroU8;

use miette::Diagnostic;
use thiserror::Error;

pub mod accent;
pub mod alloc;
pub mod ast;
pub mod degraded;
pub mod external_table_reference;
pub mod format;
pub mod lint;
pub mod node_kind;

pub use ast::GaijiMarkerPlacement;
pub use format::{
    AbsoluteSize, AccentMark, BlockPurpose, BlockStyles, CaptionScope, ColumnBlock, ColumnCount,
    EnclosureKind, FirstLineIndent, FontShift, Format, ForwardAttr, ForwardOrigin,
    HorizontalPresentation, IndentBlock, IndentLayout, Kumi, LineAlignment, LineFormat, LineWidth,
    PageVerticalPlacement, PartialLayout, QualitativeFontSize, RegionClose, RegionFormat,
    RelativePlacement, TableBlock,
};
pub use node_kind::NodeKind;

/// The typed canonical value of a gaiji reference and its building
/// blocks, re-exported from [`ab_aozora_encoding::gaiji`].
///
/// [`GaijiCanonical`] is the structured canonical value on the gaiji
/// node; [`MenKuTen`] is its
/// `第N水準P-K-T` payload and [`Resolved`] the resolved-glyph result.
pub use ab_aozora_encoding::gaiji::{GaijiCanonical, MenKuTen, Resolved};

/// Byte-range span into the original source document.
///
/// Re-exported from [`ab_aozora_spec::Span`]; see that module for the
/// canonical definition.
pub use ab_aozora_spec::Span;

/// Paired block container payload: carries only the kind descriptor.
///
/// Children live in the AST as the container node's children
/// (the `post_process` paired-container splice reparents them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Container {
    /// Which container family this open marker begins. The open
    /// [`RegionFormat`] payload is authoritative when the pair round-trips.
    pub kind: RegionFormat,
}

/// Which 傍点 (emphasis dot) or 傍線 (sideline) mark decorates a run.
///
/// Carried by both the forward-reference `ast::ForwardFormat` leaf and the
/// paired [`crate::RegionFormat::Bouten`]. The 点 (dot) vs 線 (line) split
/// ([`Self::is_line`]) is the family boundary the
/// `mismatched_bouten_container` diagnostic enforces. Each variant maps to a
/// canonical 青空文庫 keyword via [`Self::keyword`]; [`BOUTEN_KINDS`] is the
/// single declaration-order list the rest of the workspace derives from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BoutenKind {
    /// ゴマ
    Goma,
    /// 白ゴマ
    WhiteSesame,
    /// 丸
    Circle,
    /// 白丸
    WhiteCircle,
    /// 二重丸
    DoubleCircle,
    /// 蛇の目
    Janome,
    /// ばつ
    Cross,
    /// 白三角
    WhiteTriangle,
    /// 波線
    WavyLine,
    /// 傍線
    UnderLine,
    /// 二重傍線
    DoubleUnderLine,
    /// 鎖線
    ChainLine,
    /// 破線
    DashedLine,
    /// 黒三角
    BlackTriangle,
    /// 白四角
    WhiteSquare,
}

impl BoutenKind {
    /// Whether this is a 傍線 (line) variant rather than a 傍点 (dot)
    /// variant. The 点/線 split is the family boundary used by
    /// `mismatched_bouten_container`: a `［＃傍点］` range closed by a
    /// `［＃傍線終わり］` (or vice-versa) is the mismatch the diagnostic
    /// reports.
    #[must_use]
    pub const fn is_line(self) -> bool {
        matches!(
            self,
            Self::WavyLine
                | Self::UnderLine
                | Self::DoubleUnderLine
                | Self::ChainLine
                | Self::DashedLine
        )
    }

    /// Stable family tag (`"傍点"` / `"傍線"`) for diagnostics that report a
    /// mismatched bouten range pair.
    #[must_use]
    pub const fn family_str(self) -> &'static str {
        if self.is_line() { "傍線" } else { "傍点" }
    }
}

/// Every [`BoutenKind`] variant, in declaration order.
///
/// The single enumeration source the rest of the workspace derives from:
/// the parser's reverse keyword→kind lookup compares each entry's
/// [`BoutenKind::keyword`], and tests check the render / spec slug tables
/// against this list. Adding a bouten mark requires a new variant, its
/// `keyword` arm, and one row here.
pub const BOUTEN_KINDS: &[BoutenKind] = &[
    BoutenKind::Goma,
    BoutenKind::WhiteSesame,
    BoutenKind::Circle,
    BoutenKind::WhiteCircle,
    BoutenKind::DoubleCircle,
    BoutenKind::Janome,
    BoutenKind::Cross,
    BoutenKind::WhiteTriangle,
    BoutenKind::WavyLine,
    BoutenKind::UnderLine,
    BoutenKind::DoubleUnderLine,
    BoutenKind::ChainLine,
    BoutenKind::DashedLine,
    BoutenKind::BlackTriangle,
    BoutenKind::WhiteSquare,
];

/// Which side of the vertical-writing base text the bouten marks sit on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BoutenPosition {
    /// Right of (in horizontal terms, above) the base text: the
    /// default side, the bare `［＃「X」に傍点］` form.
    #[default]
    Right,
    /// Left of (below) the base text: the `左に` modifier
    /// (`［＃「X」の左に傍点］`).
    Left,
    /// Both sides of the base text: the `の両側に` modifier
    /// (`［＃「X」の両側に傍線］`).
    Both,
}

/// Source-declared placement of an emphasis dot or line decoration.
///
/// `Right` and `Left` are relative to the vertical-writing base text used by
/// ordinary Aozora bouten syntax. `Above` and `Below` retain the physical
/// placement stated by 上線 and 下線 without converting it to a writing-mode
/// relative side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum DecorationPosition {
    /// Right of the vertical-writing base text; the ordinary default.
    #[default]
    Right,
    /// Left of the vertical-writing base text.
    Left,
    /// Both relative sides of the base text.
    Both,
    /// Physically above the base text.
    Above,
    /// Physically below the base text.
    Below,
}

/// Which side of the base text a ruby reading sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum RubySide {
    /// Standard ruby: `｜base《reading》` (right of / above the base).
    #[default]
    Right,
    /// Left-side (below) ruby: `［＃「base」の左に「reading」のルビ］`, the
    /// saidoku-moji (再読文字) building block.
    Left,
}

/// Script classes used to identify implicit ruby bases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RubyBaseClass {
    /// CJK ideographs (main block + Ext A/B..F + compatibility) plus the
    /// Aozora kanji marks `々`, `〆`, `〇`, `〻`, and `ヶ`.
    Kanji,
    /// Hiragana (letters + `ゝ` / `ゞ`).
    Hiragana,
    /// Katakana (letters + `ー` / `ヽ` / `ヾ`), excluding the small `ヵ` /
    /// `ヶ` (see [`ruby_base_class`]).
    Katakana,
    /// Basic and fullwidth Latin letters plus source-encoded accent-table letters.
    Latin,
    /// Greek letters.
    Greek,
    /// Russian Cyrillic letters present in the source encoding.
    Cyrillic,
    /// ASCII and fullwidth decimal digits.
    Decimal,
}

/// Return a script or digit class, or `None` for punctuation and unsupported characters.
///
/// Aozora input rules treat `〇`, `〻`, and `ヶ` as kanji for ruby boundaries.
/// An explicit `｜` limits the base when preceding kanji must stay outside it.
/// The small katakana `ヵ` remains outside the supported implicit classes.
#[must_use]
pub const fn ruby_base_class(ch: char) -> Option<RubyBaseClass> {
    Some(match ch {
        '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{20000}'..='\u{2FFFF}'
        | '々'
        | '〆'
        | '〇'
        | '〻'
        | 'ヶ' => RubyBaseClass::Kanji,
        '\u{3041}'..='\u{3096}' | '\u{309D}'..='\u{309E}' => RubyBaseClass::Hiragana,
        // Katakana letters ァ..ヴ, phonetic ヷ..ヺ, and ー / ヽ / ヾ,
        // excluding the small ヵ(30F5) / ヶ(30F6) and the middle dot ・(30FB).
        '\u{30A1}'..='\u{30F4}' | '\u{30F7}'..='\u{30FA}' | '\u{30FC}'..='\u{30FE}' => {
            RubyBaseClass::Katakana
        }
        'A'..='Z' | 'a'..='z' | 'Ａ'..='Ｚ' | 'ａ'..='ｚ' => RubyBaseClass::Latin,
        '\u{0391}'..='\u{03A9}' | '\u{03B1}'..='\u{03C9}' => RubyBaseClass::Greek,
        'А'..='я' | 'Ё' | 'ё' => RubyBaseClass::Cyrillic,
        '0'..='9' | '０'..='９' => RubyBaseClass::Decimal,
        ch if accent::is_composed_letter(ch) => RubyBaseClass::Latin,
        _ => return None,
    })
}

/// Returns true when `ch` functions as (part of) an implicit-ruby kanji base.
/// This predicate is shared with the implicit-base classifier.
#[must_use]
pub const fn is_ruby_base_char(ch: char) -> bool {
    matches!(ruby_base_class(ch), Some(RubyBaseClass::Kanji))
}

/// Explicit physical side supplied for an editorial annotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MarginNotePosition {
    /// The source writes の左に.
    Left,
    /// The source writes の右に.
    Right,
    /// The source explicitly places the note in the page margin.
    Margin,
}

/// Position of a supplied mark along the run it accompanies.
///
/// Independent of [`MarginNotePosition`], which is the physical side. The
/// source states the two separately and can state either without the other:
/// every 傍点 sits on the right, and `の間に` states a position along the line
/// rather than a side of it. A mark with no anchor accompanies its run as a
/// whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum MarkAnchor {
    /// `「X」と「Y」の間に`: at the junction of the two adjacent runs the source
    /// quotes, and on neither of them. The source states no principal character
    /// and none is derived.
    Between,
}

/// Annotation type carried by an `ast::MarginNote`.
///
/// A note attached to a preceding run, with its source relationship retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum MarginNoteKind {
    /// 注記: an editorial gloss, with side recorded separately when supplied.
    #[default]
    Gloss,
    /// 傍記: `［＃「X」に「Y」の傍記］`, a redaction marker (典型的に ×)
    /// written beside X, used in censorship restoration.
    Marginal,
    /// 校注: an editorially supplied critical note.
    CriticalNote,
    /// A printed page label identified by a supplied section locator.
    CrossReference,
    /// The source identifies the operand as an annotation number, without a destination.
    AnnotationNumber,
    /// The source identifies the operand as an author-note marker, without a person identity.
    AuthorNote,
    /// A supplied glyph-shape assertion on an existing scalar, without glyph realization.
    GlyphShape,
    /// A literal source-positioned mark whose function is not specified.
    SuppliedMark,
}

impl MarginNoteKind {
    /// Source keyword identifying the annotation kind independently of its side.
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Gloss => "注記",
            Self::Marginal => "傍記",
            Self::CriticalNote => "校注",
            Self::SuppliedMark => "",
            Self::CrossReference | Self::GlyphShape => "は",
            Self::AnnotationNumber => "注釈番号",
            Self::AuthorNote => "自注",
        }
    }
}

/// Which section-break directive an `ast::Node::SectionBreak` carries:
/// the stronger page-structure breaks beyond the plain `［＃改ページ］`.
///
/// Each variant maps to its canonical keyword via [`Self::keyword`];
/// [`SECTION_KINDS`] is the declaration-order list the renderer derives its
/// class list from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum SectionKind {
    /// `［＃改丁］`
    Kaicho,
    /// `［＃改段］`
    Kaidan,
    /// `［＃改見開き］`
    Kaimihiraki,
}

/// Semantic kind of a break directive that also carries an instruction for
/// the content following the break.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum QualifiedBreakKind {
    /// Start the next page.
    Page,
    /// Start the next left page in vertical layout.
    Kaicho,
    /// Start the next column.
    Column,
    /// Start the next spread.
    Kaimihiraki,
}

/// Alignment requested for content following a qualified break.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FollowingPlacement {
    /// Center the following content horizontally on the page.
    HorizontalCenter,
    /// Align the following content to the left.
    Left,
}

/// A break directive with an explicit side or a layout request for the
/// content following it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct QualifiedBreak {
    /// The page, leaf, column, or spread transition.
    pub kind: QualifiedBreakKind,
    /// Whether the source explicitly requests the right side.
    pub destination_right: bool,
    /// Alignment of the following content.
    pub following_placement: Option<FollowingPlacement>,
    /// Indentation of the following content in characters.
    pub following_indent: Option<u16>,
    /// One-based number of the column that starts at a column break (`２段目`).
    pub column_number: Option<NonZeroU8>,
}

/// Heading outline level: the 大 / 中 / 小 outline rank.
///
/// Orthogonal to [`HeadingStyle`]; the two combine (同行中見出し is
/// `Medium` + `SameLine`, 窓小見出し is `Small` + `Window`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum HeadingKind {
    /// 大見出し: the top outline level (renders as `<h1>`).
    Large,
    /// 中見出し: the middle outline level (renders as `<h2>`).
    Medium,
    /// 小見出し: the lowest outline level (renders as `<h3>`).
    Small,
}

impl HeadingKind {
    /// The numeric outline level (`1` = 大, `2` = 中, `3` = 小) carried by
    /// the inline `ast::HeadingHint`'s `data-level` attribute.
    ///
    /// The single source of the 大/中/小 → 1/2/3 mapping; the renderer and the
    /// classifier both read it.
    #[must_use]
    pub const fn outline_level(self) -> u8 {
        match self {
            Self::Medium => 2,
            Self::Small => 3,
            // 大見出し and any future top level default to 1.
            _ => 1,
        }
    }
}

/// Heading style: standard, 同行 (same-line), or 窓 (window).
///
/// Orthogonal to [`HeadingKind`] (the 大 / 中 / 小 level): each style
/// pairs with any level. The 同行 style runs the title into the body on the
/// same line; 窓 is an inset title. 副見出し does not occur in the corpus and
/// has no variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum HeadingStyle {
    /// Standard heading: no 同行 / 窓 prefix. The default.
    #[default]
    Standard,
    /// 同行見出し: the title runs into the body on the same line.
    SameLine,
    /// 窓見出し: an inset ("window") title.
    Window,
}

/// Every [`SectionKind`] variant in declaration order.
///
/// The renderer derives its class list (and any codegen) from this list,
/// as with [`BOUTEN_KINDS`].
pub const SECTION_KINDS: &[SectionKind] = &[
    SectionKind::Kaicho,
    SectionKind::Kaidan,
    SectionKind::Kaimihiraki,
];

/// Every [`HeadingKind`] outline level in declaration order. See
/// [`BOUTEN_KINDS`].
pub const HEADING_KINDS: &[HeadingKind] =
    &[HeadingKind::Large, HeadingKind::Medium, HeadingKind::Small];

/// Every [`HeadingStyle`] in declaration order. See [`BOUTEN_KINDS`].
pub const HEADING_STYLES: &[HeadingStyle] = &[
    HeadingStyle::Standard,
    HeadingStyle::SameLine,
    HeadingStyle::Window,
];

// The canonical 青空文庫 keyword of each render-bearing enum is defined once,
// below (e.g. `BoutenKind::WhiteSesame` → "白ゴマ傍点"). The serializer
// (AST → annotation text) and the renderers read these keywords, and
// `ab_aozora_spec::roman_slug` derives the romaji CSS slug from them.

impl BoutenKind {
    /// Canonical 青空文庫 keyword (the body of `［＃「…」に〈keyword〉］`).
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::WhiteSesame => "白ゴマ傍点",
            Self::Circle => "丸傍点",
            Self::WhiteCircle => "白丸傍点",
            Self::DoubleCircle => "二重丸傍点",
            Self::Janome => "蛇の目傍点",
            Self::Cross => "ばつ傍点",
            Self::WhiteTriangle => "白三角傍点",
            Self::WavyLine => "波線",
            Self::UnderLine => "傍線",
            Self::DoubleUnderLine => "二重傍線",
            Self::ChainLine => "鎖線",
            Self::DashedLine => "破線",
            Self::BlackTriangle => "黒三角傍点",
            Self::WhiteSquare => "白四角傍点",
            // Goma (無印) and any future kind default to the bare 傍点.
            _ => "傍点",
        }
    }
}

impl SectionKind {
    /// Canonical 青空文庫 keyword for the section break. The match has no `_`
    /// arm, so a new variant fails to compile here until its keyword is
    /// supplied.
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Kaicho => "改丁",
            Self::Kaidan => "改段",
            Self::Kaimihiraki => "改見開き",
        }
    }
}

/// Classifies a generic `ast::Directive` annotation that no more
/// specific node recogniser matched.
///
/// [`Unknown`](Self::Unknown) is the catch-all for Aozora-shaped `［＃…］`
/// notation the parser does not model; the remaining variants tag the
/// annotations kept as raw `Directive`s (sic markers, warichu
/// delimiters, the header 凡例 `［＃］`, …) so consumers can act on them
/// without re-parsing the raw bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum DirectiveKind {
    /// The parser recognised the notation as Aozora-shaped but not registered.
    Unknown,
    /// A later declaration already associated with an earlier gaiji placeholder.
    GaijiDeclaration,
    /// Recognized operations retain source selectors without requiring target bindings.
    FormattingInstruction(ast::FormattingInstructionId),
    /// `［＃「X」はママ］`-style editorial *sic* marker (text reproduced as in the source).
    Sic,
    /// Source-text divergence note (`［＃「X」は底本では「Y」］`).
    BaseTextVariant,
    /// A ruby span that couldn't be parsed cleanly.
    InvalidRubySpan,
    /// Inline warichu opener: `［＃割り注］`.
    WarichuOpen,
    /// Inline warichu closer: `［＃割り注終わり］`.
    WarichuClose,
    /// Inline warigaki opener: `［＃割書］`.
    WarigakiOpen,
    /// Inline warigaki closer: `［＃割書終わり］`.
    WarigakiClose,
    /// An empty directive `［＃］` (or whitespace-only `［＃　］`). This is not an
    /// unrecognized notation; it is the standard symbol used in the
    /// file-header 凡例 line `［＃］：入力者注…` that prefixes nearly every
    /// 青空文庫 work. A distinct variant keeps it out of `Unknown`; its raw
    /// bytes still round-trip.
    Empty,
    /// Numbered input-typist note (`［＃入力者注(N)］`): a reference to the
    /// numbered note listed in the file's 凡例. Rendered as a visible `注N`
    /// superscript; a distinct variant keeps it out of `Unknown`.
    EditorNote,
    /// A source statement about an intervention made during transcription.
    TranscriptionNote,
    /// A source note that questions whether preceding text should be corrected.
    UncertainCorrection,
    /// A source statement that the printing is unclear at the cited location.
    SourceLegibilityNote,
    /// Printed-page side marker (`［＃左頁］` or `［＃右頁］`).
    PageSideNote,
    /// A source query whether an indentation was omitted (`一字下げ忘れか？`);
    /// the questioned indentation is not applied.
    IndentationQuestion,
    /// A source statement that notes were omitted, without supplied contents.
    OmissionNote,
    /// A source statement that the work is unfinished.
    IncompletenessNote,
    /// A source-supplied modern-language translation.
    TranslationNote,
    /// An annotation whose target is an adjacent ruby reading.
    RubyReadingAnnotation,
    /// A source-positioned explanation, without inferred target or agency.
    ExplanationNote,
    /// A quoted example of source notation, without applying that notation.
    NotationExample,
    /// A supplied external table filename, without its content or resolution.
    ExternalTableReference,
    /// A source statement that a named figure belongs at this point, supplying
    /// no filename, dimensions or availability. Distinct from `OmissionNote`:
    /// the source states that a figure belongs here, not that one was left out.
    FigureInsertion,
    /// Ruby-presence editorial note (`［＃「X」にルビ］`): records that the run
    /// `X` carries a ruby gloss in the source. The gloss text itself is not in
    /// the directive; this is a proofreading marker, not renderable ruby.
    /// Rendered as a compact visible marker; the raw bracket round-trips.
    RubyAttached,
    /// Ruby-binding editorial note (`［＃ルビは「X」にかかる］`): records that a
    /// nearby ruby applies to the run `X` (disambiguation, not renderable
    /// ruby). Rendered as a compact marker; the raw bracket round-trips.
    RubyRetarget,
    /// Left-side-ruby span opener (`［＃左にルビ付き］`): marks the start of a
    /// run that carries a left-side ruby whose reading is given on the matching
    /// [`Self::RubyPairClose`]. A raw-preserving `Direct` directive (the pair is
    /// not coupled at the splice layer, like the inline warichu pair).
    RubyPairOpen,
    /// Left-side-ruby span closer (`［＃左に「Y」のルビ付き終わり］`): gives the
    /// left-side ruby reading `Y` for the span opened by [`Self::RubyPairOpen`].
    /// `Y` is preserved verbatim in the raw bracket and shown in the marker.
    RubyPairClose,
    /// Margin-note span opener (`［＃注記付き］` / `［＃左に注記付き］`): marks the
    /// start of a run that carries a margin note whose text is given on the
    /// matching [`Self::MarginNotePairClose`]. A raw-preserving `Direct`
    /// directive (the pair is not coupled at the splice layer, like the inline
    /// warichu pair). The `左に` prefix records that the note sits on the left.
    MarginNotePairOpen,
    /// A [`Self::MarginNotePairOpen`] whose matching closer was resolved into
    /// a typed margin note. The raw marker remains available for source
    /// serialization while semantic projections suppress this boundary leaf.
    MarginNotePairOpenResolved,
    /// Margin-note span closer (`［＃「Y」の注記付き終わり］` / `［＃左に「Y」の注記
    /// 付き終わり］`): gives the margin-note text `Y` for the span opened by
    /// [`Self::MarginNotePairOpen`]. `Y` is preserved verbatim in the raw
    /// bracket (it may contain a nested `［＃…］` gaiji) and shown in the marker.
    MarginNotePairClose,
}

/// Parse- and render-time error surface for `aozora-syntax` consumers.
#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum SyntaxError {
    /// A node-kind tag string did not resolve to a known node kind. The
    /// offending tag is carried verbatim in [`kind`](Self::UnknownKind::kind)
    /// and echoed in the `未知のノード種別です` message; the diagnostic code is
    /// `aozora::syntax::unknown_kind`.
    #[error("未知のノード種別です: {kind}")]
    #[diagnostic(code(aozora::syntax::unknown_kind))]
    UnknownKind {
        /// The unrecognised tag string, as received.
        kind: Box<str>,
    },
}

/// Returns the supplied locator only for `一字下げ忘れか？N-N`.
#[must_use]
pub fn indentation_question_locator(body: &str) -> Option<&str> {
    let locator = body.strip_prefix("一字下げ忘れか？")?;
    let (page, line) = locator.split_once('-')?;
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    (digits(page) && digits(line)).then_some(locator)
}

/// Read the explicit pixel dimensions used by image insertion annotations.
#[must_use]
pub fn parse_image_dimensions(dims: &str) -> Option<(&str, &str)> {
    let (w, h) = dims.split_once('×')?;
    let w = w.strip_prefix('横')?;
    let h = h.strip_prefix('縦')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (digits(w) && digits(h)).then_some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_span_is_empty_and_zero_length() {
        let s = Span::new(42, 42);
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn span_slices_source_buffer() {
        let source = "hello world";
        let s = Span::new(6, 11);
        assert_eq!(s.slice(source), "world");
    }

    #[test]
    fn bouten_position_defaults_to_right() {
        assert_eq!(BoutenPosition::default(), BoutenPosition::Right);
    }

    #[test]
    fn ruby_side_and_heading_style_defaults() {
        assert_eq!(RubySide::default(), RubySide::Right);
        assert_eq!(HeadingStyle::default(), HeadingStyle::Standard);
    }

    #[test]
    fn bouten_keyword_is_exhaustive_and_stable() {
        // Every listed variant maps to its canonical 青空文庫 keyword; the
        // bare 傍点 (Goma) comes from the `_` default arm.
        let cases = [
            (BoutenKind::Goma, "傍点"),
            (BoutenKind::WhiteSesame, "白ゴマ傍点"),
            (BoutenKind::Circle, "丸傍点"),
            (BoutenKind::WhiteCircle, "白丸傍点"),
            (BoutenKind::DoubleCircle, "二重丸傍点"),
            (BoutenKind::Janome, "蛇の目傍点"),
            (BoutenKind::Cross, "ばつ傍点"),
            (BoutenKind::WhiteTriangle, "白三角傍点"),
            (BoutenKind::WavyLine, "波線"),
            (BoutenKind::UnderLine, "傍線"),
            (BoutenKind::DoubleUnderLine, "二重傍線"),
            (BoutenKind::ChainLine, "鎖線"),
            (BoutenKind::DashedLine, "破線"),
            (BoutenKind::BlackTriangle, "黒三角傍点"),
            (BoutenKind::WhiteSquare, "白四角傍点"),
        ];
        for (kind, kw) in cases {
            assert_eq!(kind.keyword(), kw, "keyword mismatch for {kind:?}");
        }
    }

    #[test]
    fn bouten_is_line_splits_dot_from_line_family() {
        // 線 family.
        for kind in [
            BoutenKind::WavyLine,
            BoutenKind::UnderLine,
            BoutenKind::DoubleUnderLine,
            BoutenKind::ChainLine,
            BoutenKind::DashedLine,
        ] {
            assert!(kind.is_line(), "{kind:?} should be a 線 variant");
            assert_eq!(kind.family_str(), "傍線", "family_str for {kind:?}");
        }
        // 点 family.
        for kind in [
            BoutenKind::Goma,
            BoutenKind::WhiteSesame,
            BoutenKind::Circle,
            BoutenKind::WhiteCircle,
            BoutenKind::DoubleCircle,
            BoutenKind::Janome,
            BoutenKind::Cross,
            BoutenKind::WhiteTriangle,
            BoutenKind::BlackTriangle,
            BoutenKind::WhiteSquare,
        ] {
            assert!(!kind.is_line(), "{kind:?} should be a 点 variant");
            assert_eq!(kind.family_str(), "傍点", "family_str for {kind:?}");
        }
    }

    #[test]
    fn forward_attr_keyword_is_exhaustive_and_stable() {
        use core::num::NonZeroI8;
        let cases = [
            (ForwardAttr::Bold, "太字"),
            (ForwardAttr::Italic, "斜体"),
            (ForwardAttr::SuperScript, "上付き小文字"),
            (ForwardAttr::SubScript, "下付き小文字"),
            (
                ForwardAttr::SmallScript(BoutenPosition::Right),
                "行右小書き",
            ),
            (ForwardAttr::SmallScript(BoutenPosition::Left), "行左小書き"),
            (ForwardAttr::Framed(EnclosureKind::Rule), "罫囲み"),
            (ForwardAttr::Framed(EnclosureKind::Box), "囲み"),
            (ForwardAttr::Framed(EnclosureKind::Circle), "○付き文字"),
            (
                ForwardAttr::Framed(EnclosureKind::CircleDotted),
                "点線丸囲み",
            ),
            (ForwardAttr::Framed(EnclosureKind::DoubleRule), "二重罫囲み"),
            (ForwardAttr::Horizontal, "横組み"),
            (ForwardAttr::Caption, "キャプション"),
            (ForwardAttr::CombineUpright, "縦中横"),
            (ForwardAttr::Fraction, "分数"),
            (
                ForwardAttr::FontSizeAbsolute(AbsoluteSize::ExtraLarge),
                "特大文字",
            ),
            (ForwardAttr::FontSizeAbsolute(AbsoluteSize::Large), "大文字"),
            (
                ForwardAttr::FontSizeAbsolute(AbsoluteSize::Medium),
                "中文字",
            ),
            (ForwardAttr::FontSizeAbsolute(AbsoluteSize::Small), "小文字"),
            (
                ForwardAttr::Bouten {
                    kind: BoutenKind::Goma,
                    position: DecorationPosition::Right,
                },
                "傍点",
            ),
            // FontSize carries a magnitude and takes the 太字 default arm.
            (
                ForwardAttr::FontSize(FontShift(NonZeroI8::new(3).unwrap())),
                "太字",
            ),
            // AccentDot's body is the selector grammar (serialized from the
            // interned `annotation_body`, never `keyword()`), so it also takes
            // the 太字 default: `keyword()` is never called for it.
            (ForwardAttr::AccentDot, "太字"),
            // Accent's suffix carries the bracketed mark symbol (serialized in a
            // dedicated arm, never `keyword()`), so it also takes the 太字 default.
            (ForwardAttr::Accent(AccentMark::Acute), "太字"),
        ];
        for (attr, kw) in cases {
            assert_eq!(attr.keyword(), kw, "keyword mismatch for {attr:?}");
        }
    }

    #[test]
    fn section_keyword_is_exhaustive() {
        assert_eq!(SectionKind::Kaicho.keyword(), "改丁");
        assert_eq!(SectionKind::Kaidan.keyword(), "改段");
        assert_eq!(SectionKind::Kaimihiraki.keyword(), "改見開き");
    }

    #[test]
    fn heading_kind_and_style_are_orthogonal_copies() {
        // Every level pairs with every style, and the payload structs are
        // Copy + Eq, which the AST requires.
        let heading = |level, style| Container {
            kind: RegionFormat::Heading {
                level,
                style,
                padded: true,
            },
        };
        assert_eq!(
            heading(HeadingKind::Medium, HeadingStyle::Window),
            heading(HeadingKind::Medium, HeadingStyle::Window),
            "equal Heading containers compare equal"
        );
        assert_ne!(
            heading(HeadingKind::Medium, HeadingStyle::Window),
            heading(HeadingKind::Small, HeadingStyle::Window),
            "different level ⇒ not equal"
        );
        assert_eq!(
            LineFormat::Indent {
                amount: 4,
                end_offset: None
            },
            LineFormat::Indent {
                amount: 4,
                end_offset: None
            }
        );
        assert!(matches!(
            LineFormat::AlignEnd {
                offset: 0,
                gothic: false,
                font: None
            },
            LineFormat::AlignEnd {
                offset: 0,
                gothic: false,
                font: None
            }
        ));
        assert!(matches!(
            LineFormat::Center { page: true },
            LineFormat::Center { page: true }
        ));
    }

    #[test]
    fn bouten_kinds_are_complete_and_distinct() {
        assert_eq!(BOUTEN_KINDS.len(), 15, "every BoutenKind variant listed");
        for (i, a) in BOUTEN_KINDS.iter().enumerate() {
            for b in &BOUTEN_KINDS[i + 1..] {
                assert_ne!(a, b, "duplicate variant in BOUTEN_KINDS");
                assert_ne!(a.keyword(), b.keyword(), "duplicate bouten keyword");
            }
        }
    }

    #[test]
    fn every_bouten_kind_has_a_render_slug() {
        // The renderer's `aozora-bouten-<slug>` class requires a romaji slug
        // in the spec's RENDER_SLUGS for every bouten kind; this checks the
        // spec table against `BOUTEN_KINDS`.
        for k in BOUTEN_KINDS {
            assert!(
                ab_aozora_spec::roman_slug(k.keyword()).is_some(),
                "RENDER_SLUGS missing a slug for bouten keyword {:?}",
                k.keyword()
            );
        }
    }
}
