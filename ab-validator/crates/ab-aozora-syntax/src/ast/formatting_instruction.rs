use std::num::NonZeroU32;

use crate::{ForwardAttr, Span};

/// All spans address the owning document's sanitized UTF-8 source.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FormattingInstruction {
    /// Source order is retained, including repeated selectors and effects.
    pub operations: Vec<FormattingOperation>,
}

/// Recognizing an operation does not establish its body targets.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FormattingOperation {
    /// Clause extent; an inherited selector can precede this range.
    pub source_span: Span,
    /// Operand extents refer to annotation text, not selected body text.
    pub selector: FormattingSelector,
    /// Supplied effect, retained even when its selector cannot be resolved.
    pub effect: FormattingEffect,
    /// Absent when body ownership or selector matching has not established a selection.
    pub selection: Option<FormattingSelection>,
}

/// Quoted spelling may denote a source template rather than a literal body occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FormattingSelector {
    /// The source supplies a selector relation but its operand is absent.
    Unresolved,
    /// A suffix instruction applies each supplied effect to the preceding text window.
    PrecedingText,
    /// No occurrence qualifier was supplied.
    Quoted(Span),
    /// An unquoted parenthesized token that refers to itself, including its parentheses.
    Parenthesized(Span),
    /// An unquoted descriptive phrase that refers to a source-defined part
    /// without establishing its concrete body extent.
    Named(Span),
    /// The selected operand is stated within a separately supplied enclosing phrase.
    Subtarget {
        /// Supplied enclosing phrase.
        enclosing: Span,
        /// Supplied inner phrase selected for formatting.
        operand: Span,
    },
    /// Supplied operands are neither deduplicated nor corrected against the body.
    EachQuoted(Vec<Span>),
    /// The source's universal quantifier does not establish its scope.
    AllQuoted(Span),
    /// The source refers to digits without specifying their character repertoire or scope.
    Digits,
    /// Select Arabic digits by their preceding characters; scope remains separate.
    ArabicDigitsAfter(FormattingPredecessor),
    /// Occurrences are counted within a separately established scope.
    Occurrence {
        /// One-based ordinal as supplied, not a byte offset.
        ordinal: NonZeroU32,
        /// Quoted text whose occurrences are counted.
        operand: Span,
    },
    /// Position refers to characters rather than occurrences of the quoted text.
    CharacterPosition {
        /// One-based position as supplied, not a byte offset.
        ordinal: NonZeroU32,
        /// Supplied character spelling, retained even if it disagrees with the body.
        operand: Span,
    },
    /// Every source line governed by a document-level layout instruction.
    EachLine,
    /// Source text after the instruction, with no supplied end boundary.
    FollowingText,
    /// A supplied number of physical source lines immediately after the instruction.
    FollowingLines {
        /// Number of following physical source lines selected by the instruction.
        lines: NonZeroU32,
    },
    /// The complete source section that the quoted heading identifies.
    QuotedWhole(Span),
}

/// Character conditions retain their source operands before any body matching.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FormattingPredecessor {
    /// The source supplies the alphabetic class rather than individual letters.
    Alphabetic,
    /// Source order and repeated character operands are retained.
    NamedCharacters(Vec<Span>),
}

/// Placement relative to a supplied base does not imply a supplied smaller font size.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FormattingEffect {
    /// Uses the same attribute vocabulary as resolved forward formatting.
    Attribute(ForwardAttr),
    /// Retains the supplied base independently of any resolved body occurrence.
    UpperRelativeTo {
        /// Quoted base operand in annotation source.
        operand: Span,
        /// Body occurrence established within the owning text window.
        target: Option<Span>,
    },
    /// Smaller type arranged in a supplied number of lines relative to supplied anchors.
    ReducedPointLines {
        /// Relationship between the arranged text and its supplied anchors.
        placement: FormattingPlacement,
        /// Number of arranged lines.
        lines: NonZeroU32,
        /// Number of lines that supplied parentheses enclose, when stated.
        parentheses_lines: Option<NonZeroU32>,
    },
    /// Center the selected run across the supplied number of source lines.
    HorizontalCenter {
        /// Number of lines whose horizontal extent defines the center.
        lines: NonZeroU32,
    },
    /// Set the selected runs as the supplied number of lines within one line;
    /// selector order is line order.
    LineSetting {
        /// Number of lines set within the line.
        lines: NonZeroU32,
        /// Number of lines that supplied parentheses enclose, when stated.
        parentheses_lines: Option<NonZeroU32>,
    },
    /// Place the selected run relative to source-owned anchors.
    RelativePlacement(FormattingPlacement),
    /// Rotate the selected characters by the supplied angle in degrees.
    Rotation {
        /// Source-supplied angle; direction is unspecified by this notation.
        degrees: NonZeroU32,
    },
    /// A declarative layout policy whose body targets need not be byte-addressable.
    Layout {
        /// Full-width characters from the policy's start reference.
        indent: u8,
        /// The instruction requests horizontal composition.
        horizontal: bool,
        /// The instruction aligns the selected content to the end edge.
        align_end: bool,
    },
    /// Insert the supplied full-width spacing measure between characters.
    LetterSpacing {
        /// Full-width characters of spacing supplied by the source.
        amount: NonZeroU32,
    },
    /// Align source-line starts by a supplied relation between lines.
    LineStartAlignment(LineStartAlignment),
    /// A heading type treatment supplied by the source, without an asserted outline level.
    HeadingTypography(HeadingTypography),
    /// A source declaration identifying return-mark glyphs and their governing scope.
    KuntenPolicy(KuntenPolicy),
    /// Style supplied for rule glyphs within the selected source lines.
    RuleStyle(FormattingRuleStyle),
    /// Print the selected letters as one ligature glyph of the supplied components.
    Ligature {
        /// Quoted component letters in source order.
        components: Vec<Span>,
    },
}

/// Source-supplied style for rule glyphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FormattingRuleStyle {
    /// A wavy rule.
    Wavy,
}

/// The source may identify either one reference line or a collective group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LineStartAlignment {
    /// Align the selected line with a line a supplied number of lines earlier.
    RelativeLine {
        /// Physical source-line distance supplied by the annotation.
        lines_before: NonZeroU32,
        /// Exact earlier line when the source document establishes it.
        reference_target: Option<Span>,
    },
    /// A group of lines, identified by one source operand, shares one start;
    /// its members are not identified.
    Collective {
        /// Source operand identifying the collectively aligned lines.
        members: Span,
    },
}

/// A declarative rule distinguishing return-mark glyphs from principal text.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct KuntenPolicy {
    /// Source spelling of the declared return marks.
    pub marks: Span,
    /// Source range governed by the declaration.
    pub scope: KuntenScope,
    /// Phrases whose specified submark is ordinary text rather than a return mark.
    pub exclusions: Vec<KuntenExclusion>,
    /// Whether the declaration explicitly continues to later matching ranges.
    pub repeat_following: bool,
    /// Whether a vertical rule is supplied to the right of the inserted mark.
    pub vertical_rule_right: bool,
}

/// Source-relative range governed by a return-mark declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum KuntenScope {
    /// The quoted source range selected by the instruction.
    QuotedRegion,
    /// Following source text selected by an `以下` instruction.
    FollowingText,
    /// The source position occupied by an explicit supplied mark.
    InsertionSite,
    /// The block opened by the same source marker.
    FollowingBlock,
    /// The block closed by the same source marker.
    PrecedingBlock,
}

/// A phrase in which a marker-shaped character remains principal text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct KuntenExclusion {
    /// Phrase containing the excluded character.
    pub enclosing: Span,
    /// Character spelling excluded within that phrase.
    pub mark: Span,
}

/// Placement of source-supplied text relative to source-supplied anchors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FormattingPlacement {
    /// The text is placed below the supplied anchor.
    Below(Span),
    /// The text is placed between two supplied anchors.
    Between {
        /// Anchor preceding the text.
        preceding: Span,
        /// Anchor following the text.
        following: Span,
    },
    /// Place the selected run below a consecutive range of day-labelled lines.
    BelowDayRange {
        /// Numeric start-day operand in the annotation.
        start_operand: Span,
        /// Numeric end-day operand in the annotation.
        end_operand: Span,
        /// Supplied first day of the inclusive range.
        start_day: NonZeroU32,
        /// Supplied last day of the inclusive range.
        end_day: NonZeroU32,
        /// Leading day number on the established first source line.
        start_target: Option<Span>,
        /// Leading day number on the established last source line.
        end_target: Option<Span>,
    },
}

/// Source-supplied heading type treatments that do not define document structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum HeadingTypography {
    /// `見出し文字`.
    Heading,
    /// `小見出し文字`.
    SmallHeading,
}

/// Targets are ordered within the source-owned enclosing scope.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FormattingSelection {
    /// Enclosing body extent within which the selector was evaluated.
    pub scope: Span,
    /// Nonempty body extents in selector order.
    pub targets: Vec<Span>,
}
