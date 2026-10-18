//! Source-stated witness alternatives, editorial notes, and concealment.

use std::{num::NonZeroU32, ops::Range};

use ab_aozora_encoding::gaiji::{
    is_mencode_shaped, is_page_line_shaped, recognize_gaiji_body, reference_clauses,
};

/// Content addressed by a quoted base-text note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextVariantTarget {
    /// The reading attached to a base character sequence.
    RubyReading,
    /// An unqualified quotation, matched against principal text first.
    Text,
}

/// Edition described by a retained editorial statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditionNoteKind {
    /// The edition used as the source of the transcription.
    BaseEdition,
    /// The work's first publication, without asserting it is the base edition.
    FirstPublication,
    /// The source statement refers to the original text without identifying an edition.
    OriginalText,
}

/// Boundary of a source-stated range in the base edition, not a presentation command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditionRangeBoundary {
    /// The range begins after this marker.
    Start,
    /// The range ends before this marker.
    End,
}

/// Location asserted for the bounded source content in the base edition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditionRangeLocation {
    /// The upper part identified by the source as 上段.
    Upper,
    /// The lower part identified by the source as 下段.
    Lower,
}

/// Kind of filename-free figure stated at a location in the base edition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocatedFigureKind {
    /// A general figure or diagram (`図`).
    Figure,
    /// A map (`地図`).
    Map,
}

impl LocatedFigureKind {
    /// Stable AAT/parser-IR spelling.
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Figure => "figure",
            Self::Map => "map",
        }
    }
}

/// Printed-page position supplied for a filename-free figure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocatedFigurePosition {
    /// Upper part of the page.
    Top,
    /// Upper-right part of the page.
    TopRight,
    /// Upper-left part of the page.
    TopLeft,
    /// Left part of the page without a vertical qualifier.
    Left,
}

impl LocatedFigurePosition {
    /// Stable AAT/parser-IR and TEI `place` spelling.
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::TopRight => "top-right",
            Self::TopLeft => "top-left",
            Self::Left => "left",
        }
    }
}

/// A statement that the base edition contains a figure at a supplied page location.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocatedFigureNote<'s> {
    /// Page number exactly as supplied, without `底本` or `頁`.
    pub page: &'s str,
    /// Optional position within that page.
    pub position: Option<LocatedFigurePosition>,
    /// Whether the source calls the object a map or a general figure.
    pub kind: LocatedFigureKind,
    /// Optional description preceding `図あり`.
    pub description: Option<&'s str>,
}

/// Parse a filename-free figure statement located in the base edition.
#[must_use]
pub fn located_figure_note(body: &str) -> Option<LocatedFigureNote<'_>> {
    let rest = body.strip_prefix("底本")?;
    let (page, rest) = rest.split_once('頁')?;
    if page.is_empty()
        || !page
            .chars()
            .all(|character| character.is_ascii_digit() || ('０'..='９').contains(&character))
    {
        return None;
    }
    let rest = rest.strip_prefix('、').unwrap_or(rest);
    let (position, rest) = if let Some(rest) = rest.strip_prefix("右上に") {
        (Some(LocatedFigurePosition::TopRight), rest)
    } else if let Some(rest) = rest.strip_prefix("左上に") {
        (Some(LocatedFigurePosition::TopLeft), rest)
    } else if let Some(rest) = rest.strip_prefix("上に") {
        (Some(LocatedFigurePosition::Top), rest)
    } else if let Some(rest) = rest.strip_prefix("左に") {
        (Some(LocatedFigurePosition::Left), rest)
    } else {
        (None, rest.strip_prefix('に')?)
    };
    let rest = rest.strip_prefix('、').unwrap_or(rest);
    if rest == "地図あり" {
        return Some(LocatedFigureNote {
            page,
            position,
            kind: LocatedFigureKind::Map,
            description: None,
        });
    }
    let description = rest.strip_suffix("図あり")?;
    if description.contains(['「', '」', '［', '］', '\r', '\n']) {
        return None;
    }
    Some(LocatedFigureNote {
        page,
        position,
        kind: LocatedFigureKind::Figure,
        description: (!description.is_empty()).then_some(description),
    })
}

/// One explicitly supplied boundary marker of an edition-only annotation range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditionRangeMarker {
    /// Which endpoint the supplied marker identifies.
    pub boundary: EditionRangeBoundary,
    /// The stated base-edition location, used to match the other endpoint.
    pub location: EditionRangeLocation,
}

/// Interpret source-stated upper/lower edition-range markers.
#[must_use]
pub fn edition_range_marker(source: &str) -> Option<EditionRangeMarker> {
    let body = source.strip_prefix("［＃")?.strip_suffix('］')?;
    let (boundary, statement) = if let Some(tail) = body.strip_prefix("ここから底本では") {
        (EditionRangeBoundary::Start, tail)
    } else {
        (
            EditionRangeBoundary::End,
            body.strip_prefix("ここまで底本では")?,
        )
    };
    let location = match statement {
        "上段" => EditionRangeLocation::Upper,
        "下段" => EditionRangeLocation::Lower,
        _ => return None,
    };
    Some(EditionRangeMarker { boundary, location })
}

fn split_attribution(body: &str, edition: EditionNoteKind) -> Option<(&str, &str)> {
    let delimiters: &[&str] = match edition {
        EditionNoteKind::BaseEdition => &[
            "」は底本では",
            "」は、底本では",
            "」底本では",
            "」では底本では",
            "」は底本で「",
            "」は底本は「",
            "」は底本「",
        ],
        EditionNoteKind::FirstPublication => &["」は初出では", "」は、初出では"],
        EditionNoteKind::OriginalText => &["」は原文では", "」は、原文では"],
    };
    if delimiters
        .iter()
        .map(|delimiter| body.matches(delimiter).count())
        .sum::<usize>()
        != 1
    {
        return None;
    }
    let (current, witness) = delimiters.iter().find_map(|delimiter| {
        let (current, _) = body.split_once(delimiter)?;
        let separator = delimiter.strip_suffix('「').unwrap_or(delimiter);
        Some((current, &body[current.len() + separator.len()..]))
    })?;
    Some((
        current,
        witness.strip_prefix(['、', ' ']).unwrap_or(witness),
    ))
}

/// A supplied reading or text and its explicitly stated base-text alternative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextVariant<'s> {
    /// Authority explicitly identified for the alternative witness.
    pub witness_kind: EditionNoteKind,
    /// Whether the source explicitly identifies the reading or leaves the target unqualified.
    pub target: TextVariantTarget,
    /// The spelling supplied in the Aozora text.
    pub current: &'s str,
    /// Witness fragment before any separate continuation; empty for explicit absence.
    pub base_text: &'s str,
    /// UTF-8 byte range of the quoted current fragment within the annotation.
    pub current_span: Range<usize>,
    /// UTF-8 byte range of the witness fragment; absent for stated omission.
    pub base_span: Option<Range<usize>>,
    /// Shared literal continuation supplied after the witness quotation.
    pub base_continuation: Option<WitnessContinuation<'s>>,
    /// Complete editorial statement when the note makes an additional assertion.
    pub editorial_statement: Option<&'s str>,
    /// Supplied page-line citation for the witness statement.
    pub source_locator: Option<&'s str>,
}

/// A correction statement that quotes only the superseded base-edition text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImplicitBaseCorrection<'s> {
    /// Reading explicitly attributed to the base edition.
    pub base_text: &'s str,
    /// UTF-8 byte range of that reading within the annotation.
    pub base_span: Range<usize>,
}

/// Confidence stated by an omission annotation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OmissionNoteCertainty {
    /// The source states that material was omitted.
    Asserted,
    /// The source questions whether material was omitted.
    Uncertain,
}

/// Classify a complete omission-annotation body and its stated confidence.
#[must_use]
pub fn omission_note_certainty(body: &str) -> Option<OmissionNoteCertainty> {
    let asserted = matches!(
        body,
        "「註」略"
            | "省略"
            | "図省略"
            | "図は省略"
            | "Ａ、Ｂ、Ｃの図省略"
            | "紙片の図、図省略"
            | "王家の紙幣の図、図省略"
            | "「ねませ和子よの譜」の表題付きの楽譜入る（略）"
            | "この後、改ページに続いて「VI.　文例」の章があるが、著作権の状態が不明なため、省略する。"
            | "目次のページ数および「解題（大内兵衛）」「追記」は省略しました"
    ) || body
        .strip_prefix('「')
        .and_then(|body| body.strip_suffix("」省略"))
        .is_some_and(|label| !label.is_empty() && !label.contains(['「', '」']))
        || body
            .strip_prefix("著者註の「")
            .and_then(|body| body.strip_suffix("」が欠字"))
            .is_some_and(|mark| !mark.is_empty() && !mark.contains(['「', '」']))
        || body
            .strip_prefix('「')
            .and_then(|body| body.strip_suffix("」は欠字"))
            .is_some_and(|blank| !blank.is_empty() && blank.chars().all(|ch| ch == '　'))
        || body
            .strip_prefix("図が入るが省略。底本")
            .and_then(|body| body.strip_suffix("ページ"))
            .is_some_and(|page| !page.is_empty() && page.bytes().all(|b| b.is_ascii_digit()));
    if asserted {
        return Some(OmissionNoteCertainty::Asserted);
    }

    let quoted = body
        .strip_prefix('「')
        .and_then(|body| body.split_once('」'))
        .is_some_and(|(text, suffix)| {
            !text.is_empty()
                && !text.contains(['「', '」'])
                && matches!(suffix, "が脱落か？" | "が欠如か")
        });
    (quoted || body == "句点が抜けていると考えられる").then_some(OmissionNoteCertainty::Uncertain)
}

/// A source-supplied continuation whose spelling also ends the current quotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitnessContinuation<'s> {
    /// Literal spelling supplied after the closing quote.
    pub text: &'s str,
    /// Exact UTF-8 range within the annotation, excluding the preceding quote.
    pub span: Range<usize>,
}

/// One source identified by an apparatus reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedWitness<'s> {
    /// Source label exactly as written, including bibliographic details.
    pub label: &'s str,
    /// UTF-8 byte range of the label within the annotation.
    pub span: Range<usize>,
}

/// One alternative reading and every source that shares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedApparatusReading<'s> {
    /// Sources carrying this reading.
    pub witnesses: Vec<NamedWitness<'s>>,
    /// Reading exactly as quoted.
    pub text: &'s str,
    /// UTF-8 byte range of the reading within the annotation.
    pub text_span: Range<usize>,
}

/// A quoted current reading compared with one or more explicitly identified sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedTextApparatus<'s> {
    /// Current text quoted by the annotation.
    pub current: &'s str,
    /// UTF-8 byte range of the current text within the annotation.
    pub current_span: Range<usize>,
    /// Alternative readings, grouped by their shared reading.
    pub readings: Vec<NamedApparatusReading<'s>>,
}

fn split_named_witnesses(source: &str, offset: usize) -> Option<Vec<NamedWitness<'_>>> {
    if source.is_empty() || source.contains(['［', '］', '\n', '\r']) {
        return None;
    }

    let mut witnesses = Vec::new();
    let mut cursor = 0;
    if source.starts_with('『') {
        loop {
            let rest = &source[cursor..];
            let title_end = rest.find('』')? + '』'.len_utf8();
            let mut end = cursor + title_end;
            if source[end..].starts_with('（') {
                let details_end = source[end..].find('）')? + '）'.len_utf8();
                end += details_end;
            }
            witnesses.push(NamedWitness {
                label: &source[cursor..end],
                span: offset + cursor..offset + end,
            });
            if end == source.len() {
                break;
            }
            if !source[end..].starts_with('と') {
                return None;
            }
            cursor = end + 'と'.len_utf8();
            if !source[cursor..].starts_with('『') {
                return None;
            }
        }
    } else {
        if matches!(source, "底本" | "初出" | "原文") || source.contains(['「', '」', '『', '』'])
        {
            return None;
        }
        witnesses.push(NamedWitness {
            label: source,
            span: offset..offset + source.len(),
        });
    }
    Some(witnesses)
}

fn named_reading_end(source: &str) -> Option<usize> {
    let mut bracket_depth = 0_u32;
    for (offset, ch) in source.char_indices() {
        match ch {
            '［' => bracket_depth = bracket_depth.checked_add(1)?,
            '］' => bracket_depth = bracket_depth.checked_sub(1)?,
            '」' if bracket_depth == 0 => return Some(offset),
            '\n' | '\r' => return None,
            _ => {}
        }
    }
    None
}

/// Parse named-source apparatus of the forms `Aでは「Y」` and
/// `Aでは「Y」、Bでは「Z」`.
#[must_use]
pub fn named_text_apparatus(source: &str) -> Option<NamedTextApparatus<'_>> {
    let statement = source.strip_prefix("［＃「")?.strip_suffix('］')?;
    let current_end = named_reading_end(statement)?;
    let (current, rest) = statement.split_at(current_end);
    let mut clauses = rest.strip_prefix("」は")?;
    // The current text may quote gaiji, but no other notation.
    if current.is_empty()
        || current.matches('［').count() != current.matches("※［＃").count()
        || current.contains(['\n', '\r'])
    {
        return None;
    }
    let current_start = "［＃「".len();
    let clauses_start = current_start + current.len() + "」は".len();
    let mut consumed = 0;
    if clauses.starts_with('、') {
        consumed += '、'.len_utf8();
        clauses = &clauses['、'.len_utf8()..];
    }
    let mut readings = Vec::new();
    loop {
        let (witness_source, after_witness) = clauses.split_once("では「")?;
        let witness_start = clauses_start + consumed;
        let witnesses = split_named_witnesses(witness_source, witness_start)?;
        let reading_start = witness_start + witness_source.len() + "では「".len();
        let reading_end = named_reading_end(after_witness)?;
        let text = &after_witness[..reading_end];
        if text.is_empty() {
            return None;
        }
        readings.push(NamedApparatusReading {
            witnesses,
            text,
            text_span: reading_start..reading_start + text.len(),
        });
        let tail = &after_witness[reading_end + '」'.len_utf8()..];
        if tail.is_empty() {
            break;
        }
        clauses = tail.strip_prefix('、')?;
        consumed = source.len() - '］'.len_utf8() - clauses.len() - clauses_start;
    }
    Some(NamedTextApparatus {
        current,
        current_span: current_start..current_start + current.len(),
        readings,
    })
}

/// A source note that questions whether the preceding printed text is an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UncertainCorrection<'s> {
    /// Printed text identified by the note, when the marker quotes it.
    pub target_text: Option<&'s str>,
    /// UTF-8 byte range of the quoted target within the directive body.
    pub target_span: Option<Range<usize>>,
    /// Text proposed by the note as the likely correction.
    pub proposed_text: &'s str,
    /// UTF-8 byte range of the proposed text within the directive body.
    pub proposed_span: Range<usize>,
    /// Printed page-line location carried by the note.
    pub source_locator: Option<&'s str>,
}

/// Parse a questioned correction without applying the proposed text.
#[must_use]
pub fn uncertain_correction(body: &str) -> Option<UncertainCorrection<'_>> {
    let quoted = body.strip_prefix('「')?;
    if let Some((proposed_text, source_locator)) = quoted.split_once("」の誤り？、") {
        if !valid_correction_text(proposed_text) || !is_page_line_shaped(source_locator) {
            return None;
        }
        let start = '「'.len_utf8();
        return Some(UncertainCorrection {
            target_text: None,
            target_span: None,
            proposed_text,
            proposed_span: start..start + proposed_text.len(),
            source_locator: Some(source_locator),
        });
    }
    let (target_text, proposed_text, separator) = if let Some((target_text, rest)) =
        quoted.split_once("」余分か、それとも「")
    {
        (
            target_text,
            rest.strip_suffix("」か？")?,
            "」余分か、それとも「",
        )
    } else if let Some((target_text, rest)) = quoted.split_once("」は「") {
        (target_text, questioned_error(rest)?, "」は「")
    } else {
        let proposed_text = questioned_error(quoted).filter(|text| valid_correction_text(text))?;
        let start = '「'.len_utf8();
        return Some(UncertainCorrection {
            target_text: None,
            target_span: None,
            proposed_text,
            proposed_span: start..start + proposed_text.len(),
            source_locator: None,
        });
    };
    if !valid_correction_text(target_text) || !valid_correction_text(proposed_text) {
        return None;
    }
    let target_start = '「'.len_utf8();
    let proposed_start = target_start + target_text.len() + separator.len();
    Some(UncertainCorrection {
        target_text: Some(target_text),
        target_span: Some(target_start..target_start + target_text.len()),
        proposed_text,
        proposed_span: proposed_start..proposed_start + proposed_text.len(),
        source_locator: None,
    })
}

fn questioned_error(text: &str) -> Option<&str> {
    [
        "」の誤記か",
        "」の誤りか",
        "」の誤植か",
        "」の誤記と思われる",
        "」の誤植と思われる",
        "」の誤り？",
    ]
    .into_iter()
    .find_map(|suffix| text.strip_suffix(suffix))
}

fn valid_correction_text(text: &str) -> bool {
    !text.is_empty() && !text.contains(['「', '」', '［', '］', '\r', '\n'])
}

/// Parse a source-positioned statement that the printing is unclear.
#[must_use]
pub fn source_legibility_note(body: &str) -> Option<&str> {
    let locator = body.strip_prefix("印刷不鮮明、")?;
    is_page_line_shaped(locator).then_some(locator)
}

/// A source annotation attached to the reading of the immediately preceding
/// ruby rather than to its visible base text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RubyReadingAnnotation<'s> {
    /// Reading text quoted by the directive.
    pub target: &'s str,
    /// Annotation content supplied for that reading.
    pub note: &'s str,
}

/// Parse `ルビの「X」にYの注記`, `ルビの「X」に「Y」の注記` or
/// `ルビ「X」の下に「Y」の注記` without asserting that a matching ruby exists.
#[must_use]
pub fn ruby_reading_annotation(body: &str) -> Option<RubyReadingAnnotation<'_>> {
    let (target, note) = match body.strip_prefix("ルビの「") {
        Some(rest) => rest.split_once("」に")?,
        None => body.strip_prefix("ルビ「")?.split_once("」の下に")?,
    };
    let note = note.strip_suffix("の注記")?;
    let note = note
        .strip_prefix('「')
        .and_then(|quoted| quoted.strip_suffix('」'))
        .unwrap_or(note);
    (!target.is_empty()
        && !note.is_empty()
        && !target.contains(['「', '」', '［', '］', '\r', '\n'])
        && !note.contains(['「', '」', '［', '］', '\r', '\n']))
    .then_some(RubyReadingAnnotation { target, note })
}

/// A sic note with a quoted target: `「X」はママ` for printed text,
/// `ルビの「X」はママ` for a ruby reading. `は底本のまま`, `は底本ママ` and
/// `は底本のママ` are the same note; a stray `」` after `ママ` is ignored.
///
/// `「X」は誤訳で本当は「Y」` states that the printed X is a mistranslation of Y.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SicTarget<'s> {
    /// The quoted text, reproduced as printed.
    pub target: &'s str,
    /// UTF-8 byte range of the quoted target within the directive body.
    pub target_span: Range<usize>,
    /// Whether the target is the reading of the preceding ruby.
    pub ruby_reading: bool,
    /// UTF-8 byte range of the correct text stated by the note, within the directive body.
    pub correction_span: Option<Range<usize>>,
}

/// Parse a sic note's quoted target without asserting that it occurs.
#[must_use]
pub fn sic_target(body: &str) -> Option<SicTarget<'_>> {
    let (ruby_reading, rest) = match body.strip_prefix("ルビの「") {
        Some(rest) => (true, rest),
        None => (false, body.strip_prefix('「')?),
    };
    let (target, correction_span) = if !ruby_reading
        && let Some((target, correction)) = rest.split_once("」は誤訳で本当は「")
    {
        let correction = correction
            .strip_suffix('」')
            .filter(|text| valid_correction_text(text))?;
        let end = body.len() - '」'.len_utf8();
        (target, Some(end - correction.len()..end))
    } else {
        let target = [
            "」はママ",
            "」はママ」",
            "」は底本のまま",
            "」は底本ママ",
            "」は底本のママ",
        ]
        .into_iter()
        .find_map(|suffix| rest.strip_suffix(suffix))?;
        (target, None)
    };
    if target.is_empty() || target.contains(['「', '」', '［', '］', '\r', '\n']) {
        return None;
    }
    let start = body.len() - rest.len();
    Some(SicTarget {
        target,
        target_span: start..start + target.len(),
        ruby_reading,
        correction_span,
    })
}

fn witness_continuation<'s>(
    witness: &'s str,
    current: &str,
    start: usize,
) -> Option<WitnessContinuation<'s>> {
    let (quoted, text) = witness.rsplit_once('」')?;
    if !quoted
        .strip_prefix('「')
        .is_some_and(|interior| !interior.is_empty() && !interior.contains(['「', '」']))
        || text.is_empty()
        || !text.chars().all(char::is_alphanumeric)
        || current.strip_suffix(text).is_none_or(str::is_empty)
    {
        return None;
    }
    let end = start + witness.len();
    Some(WitnessContinuation {
        text,
        span: end - text.len()..end,
    })
}

/// Reading alternatives qualified by the same parenthesized principal continuation.
/// The consumer must verify that continuation against the addressed ruby's source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextualReading<'s> {
    /// Supplied reading without the parenthesized continuation.
    pub current: &'s str,
    /// Base-edition reading without the same continuation.
    pub base_text: &'s str,
    /// Exact principal text required between the ruby and annotation.
    pub continuation: &'s str,
}

/// Separate a shared continuation without asserting that a source target exists.
#[must_use]
pub fn contextual_reading<'s>(
    current: &'s str,
    base_text: &'s str,
) -> Option<ContextualReading<'s>> {
    let split = |text: &'s str| {
        let (reading, continuation) = text.strip_suffix('）')?.rsplit_once('（')?;
        (!reading.is_empty()
            && !continuation.is_empty()
            && !reading.contains(['（', '）', '\n', '\r'])
            && !continuation.contains(['（', '）', '\n', '\r']))
        .then_some((reading, continuation))
    };
    let (current, continuation) = split(current)?;
    let (base_text, witness_continuation) = split(base_text)?;
    (continuation == witness_continuation).then_some(ContextualReading {
        current,
        base_text,
        continuation,
    })
}

/// Recognize a quoted current target and its explicitly attributed base-text alternative.
/// Other editorial prose remains unclassified by this recognizer.
#[must_use]
pub fn text_variant(source: &str) -> Option<TextVariant<'_>> {
    if let Some(variant) = literal_bracket_text_variant(source) {
        return Some(variant);
    }
    if let Some(variant) = explicit_base_correction(source) {
        return Some(variant);
    }
    let statement = source.strip_prefix("［＃")?.strip_suffix('］')?;
    if let Some(variant) = leading_base_edition_variant(source, statement) {
        return Some(variant);
    }
    // An unpaired `「` before `ルビの` leaves the ruby-reading note as the only reading.
    let (target, body) = if let Some(body) = statement
        .strip_prefix("ルビの「")
        .or_else(|| statement.strip_prefix("「ルビの「"))
    {
        (TextVariantTarget::RubyReading, body)
    } else {
        (TextVariantTarget::Text, statement.strip_prefix('「')?)
    };
    let (witness_kind, current, witness) =
        [EditionNoteKind::BaseEdition, EditionNoteKind::OriginalText]
            .into_iter()
            .find_map(|kind| {
                split_attribution(body, kind).map(|(current, witness)| (kind, current, witness))
            })?;
    let witness_start = source.len() - '］'.len_utf8() - witness.len();
    let current_start = source.len() - '］'.len_utf8() - body.len();
    variant_parts(
        statement,
        witness_kind,
        target,
        current,
        current_start,
        witness,
        witness_start,
    )
}

/// `底本では「A」は「B」`: the attribution leads, and A and B are the current
/// and the base-edition text, as in `「A」は底本では「B」`. A statement that
/// continues after B is left to the other readings.
fn leading_base_edition_variant<'s>(
    source: &'s str,
    statement: &'s str,
) -> Option<TextVariant<'s>> {
    const RELATION: &str = "」は「";
    let body = statement.strip_prefix("底本では「")?;
    if body.matches(RELATION).count() != 1 || !body.ends_with('」') {
        return None;
    }
    let (current, _) = body.split_once(RELATION)?;
    let witness = &body[current.len() + "」は".len()..];
    variant_parts(
        statement,
        EditionNoteKind::BaseEdition,
        TextVariantTarget::Text,
        current,
        source.len() - '］'.len_utf8() - body.len(),
        witness,
        source.len() - '］'.len_utf8() - witness.len(),
    )
}

fn explicit_base_correction(source: &str) -> Option<TextVariant<'_>> {
    let statement = source.strip_prefix("［＃")?.strip_suffix('］')?;
    let (base_text, current, base_offset, current_offset) =
        explicit_base_correction_parts(statement)?;
    let statement_start = "［＃".len();
    let base_start = statement_start + base_offset;
    let current_start = statement_start + current_offset;
    Some(TextVariant {
        witness_kind: EditionNoteKind::BaseEdition,
        target: TextVariantTarget::Text,
        current,
        base_text,
        current_span: current_start..current_start + current.len(),
        base_span: Some(base_start..base_start + base_text.len()),
        base_continuation: None,
        editorial_statement: Some(statement),
        source_locator: None,
    })
}

/// Recognize a base-edition correction whose replacement is not stated.
#[must_use]
pub fn implicit_base_correction(source: &str) -> Option<ImplicitBaseCorrection<'_>> {
    let statement = source.strip_prefix("［＃")?.strip_suffix('］')?;
    let (base_text, offset) = implicit_base_correction_parts(statement)?;
    let start = "［＃".len() + offset;
    Some(ImplicitBaseCorrection {
        base_text,
        base_span: start..start + base_text.len(),
    })
}

fn implicit_base_correction_parts(statement: &str) -> Option<(&str, usize)> {
    let quoted = statement
        .strip_prefix("底本の「")
        .or_else(|| statement.strip_prefix("底本「"))?;
    let base_text = ["」を訂正", "」を変更", "」を修正", "」を削除"]
        .into_iter()
        .find_map(|verb| quoted.strip_suffix(verb))?;
    if base_text.is_empty() || base_text.contains(['「', '」', '［', '］', '\n', '\r']) {
        return None;
    }
    Some((base_text, statement.len() - quoted.len()))
}

fn explicit_base_correction_parts(statement: &str) -> Option<(&str, &str, usize, usize)> {
    let quoted = statement
        .strip_prefix("底本の「")
        .or_else(|| statement.strip_prefix("底本「"))?;
    let (base_text, current) = quoted.split_once("」を「")?;
    let (current, verb) = ["」に訂正", "」に変更"]
        .into_iter()
        .find_map(|verb| Some((current.strip_suffix(verb)?, verb)))?;
    if [base_text, current]
        .iter()
        .any(|text| text.is_empty() || text.contains(['「', '」', '［', '］', '\n', '\r']))
    {
        return None;
    }
    let base_offset = statement.len() - quoted.len();
    let current_offset = statement.len() - verb.len() - current.len();
    Some((base_text, current, base_offset, current_offset))
}

/// Parse the corpus convention that encloses a literal quotation mark in `【…】`.
fn literal_bracket_text_variant(source: &str) -> Option<TextVariant<'_>> {
    let statement = source.strip_prefix("［＃")?.strip_suffix('］')?;
    let body = statement.strip_prefix('【')?;
    let (current, witness) = body.split_once("】は原文では【")?;
    let (witness, tail) = witness.split_once('】')?;
    let (assertion, locator) = tail.rsplit_once('、')?;
    if assertion != "と誤記"
        || !is_page_line_shaped(locator)
        || current.is_empty()
        || witness.is_empty()
        || current.contains(['【', '】', '\n', '\r'])
        || witness.contains(['【', '】', '\n', '\r'])
    {
        return None;
    }
    let current_start = "［＃【".len();
    let witness_start =
        source.len() - '］'.len_utf8() - tail.len() - '】'.len_utf8() - witness.len();
    Some(TextVariant {
        witness_kind: EditionNoteKind::OriginalText,
        target: TextVariantTarget::Text,
        current,
        base_text: witness,
        current_span: current_start..current_start + current.len(),
        base_span: Some(witness_start..witness_start + witness.len()),
        base_continuation: None,
        editorial_statement: Some(statement),
        source_locator: Some(locator),
    })
}

/// Separate an edition clause from a shared-target formatting suffix.
/// The caller must interpret the returned formatting suffix in its own closed vocabulary.
#[must_use]
pub fn formatted_text_variant(source: &str) -> Option<(&str, TextVariant<'_>)> {
    let statement = source.strip_prefix("［＃")?.strip_suffix('］')?;
    let (formatted, suffix) = statement.strip_prefix('「')?.split_once("」は")?;
    if formatted.is_empty() || formatted.contains(['「', '」', '\n']) {
        return None;
    }
    let (formatting, current, current_start, witness) =
        if let Some((formatting, witness)) = suffix.split_once("、底本では") {
            (formatting, formatted, "［＃「".len(), witness)
        } else {
            let (formatting, assertion) = suffix.split_once("、「")?;
            let (current, witness) = assertion.split_once("」が底本では")?;
            if current.contains(['「', '」', '\n']) {
                return None;
            }
            let start = source.len() - '］'.len_utf8() - assertion.len();
            (formatting, current, start, witness)
        };
    let witness_start = source.len() - '］'.len_utf8() - witness.len();
    Some((
        formatting,
        variant_parts(
            statement,
            EditionNoteKind::BaseEdition,
            TextVariantTarget::Text,
            current,
            current_start,
            witness,
            witness_start,
        )?,
    ))
}

/// Locate a source assertion about an incorrectly printed base-edition glyph.
/// The assertion remains documentary text; it does not supply another Unicode value.
#[must_use]
pub fn gaiji_edition_note(source: &str) -> Option<&str> {
    let body = source.strip_prefix("※［＃")?.strip_suffix('］')?;
    let parsed = recognize_gaiji_body(body)?;
    let mut clauses = reference_clauses(parsed.mencode?).peekable();
    while let Some(clause) = clauses.next() {
        let statement = clause.trim();
        if clauses.peek().is_some() {
            if !is_mencode_shaped(statement) && !is_page_line_shaped(statement) {
                return None;
            }
            continue;
        }
        let description = statement
            .strip_prefix("底本はこの字を「")?
            .strip_suffix("」と作字上の誤り")?;
        return (!description.is_empty() && !description.contains(['\n', '［', '］']))
            .then_some(statement);
    }
    None
}

/// Preserve an edition statement about lettering inside an explicitly inserted image.
#[must_use]
pub fn image_edition_note(body: &str) -> Option<(&str, &str)> {
    let (image, statement) = body.split_once("入る。")?;
    let quoted = statement.strip_prefix('「')?;
    let (current, witness) = quoted.split_once("」とあるのは底本では「")?;
    let witness = witness.strip_suffix('」')?;
    if current.is_empty()
        || witness.is_empty()
        || current.contains(['「', '」', '\n'])
        || witness.contains(['「', '」', '\n'])
    {
        return None;
    }
    Some((&body[..image.len() + "入る".len()], statement))
}

/// Separate a formatting suffix from an edition statement.
///
/// The statement is explicitly attributed, or it is `伏字を起こした文字`: the
/// formatted characters restore text an earlier edition concealed. The
/// formatting caller must still recognize its complete suffix.
#[must_use]
pub fn formatting_edition_note(source: &str) -> Option<(&str, &str)> {
    const RESTORED_CONCEALMENT: &str = "伏字を起こした文字";
    let body = source.strip_prefix("［＃「")?.strip_suffix('］')?;
    let (target, suffix) = body.split_once('」')?;
    if target.is_empty() || target.contains(['「', '\n']) {
        return None;
    }
    if let Some(formatting) = suffix
        .strip_suffix(RESTORED_CONCEALMENT)
        .and_then(|head| head.strip_suffix('、'))
    {
        return Some((
            formatting,
            &suffix[suffix.len() - RESTORED_CONCEALMENT.len()..],
        ));
    }
    let (formatting, statement) = suffix.split_once("、底本では")?;
    let statement = &suffix[suffix.len() - statement.len() - "底本では".len()..];
    edition_statement(statement)?;
    Some((formatting, statement))
}

fn witness_statement_tail<'s>(witness: &'s str, current: &str) -> Option<&'s str> {
    if let Some((quoted, reference)) = witness.rsplit_once('、')
        && (is_edition_locator_shaped(reference) || reference == "正誤表による訂正")
    {
        return Some(quoted);
    }
    if let Some((quoted, repeated)) = witness.rsplit_once("」と「")
        && let Some(repeated) = repeated.strip_suffix("」が重複")
        && !repeated.is_empty()
        && !repeated.contains(['「', '」', '［', '］', '\n'])
    {
        return Some(&witness[..quoted.len() + '」'.len_utf8()]);
    }
    if let Some((quoted, explanation)) = witness.rsplit_once("」。")
        && explanation.starts_with('【')
        && explanation.ends_with("に合わせました。")
        && !explanation.contains("［＃")
    {
        return Some(&witness[..quoted.len() + '」'.len_utf8()]);
    }
    if let Some((quoted, supplied)) = witness.rsplit_once("（本文は「")
        && supplied.strip_suffix("」）") == Some(current)
    {
        return Some(quoted);
    }
    None
}

fn is_edition_locator_shaped(source: &str) -> bool {
    !source.is_empty()
        && source
            .split('-')
            .all(|part| matches!(part, "上段" | "中段" | "下段") || is_page_line_shaped(part))
}

fn variant_parts<'s>(
    statement: &'s str,
    witness_kind: EditionNoteKind,
    target: TextVariantTarget,
    current: &'s str,
    current_start: usize,
    witness: &'s str,
    witness_start: usize,
) -> Option<TextVariant<'s>> {
    let source_locator = witness
        .rsplit_once('、')
        .and_then(|(_, locator)| is_edition_locator_shaped(locator).then_some(locator));
    let with_statement_tail = witness_statement_tail(witness, current);
    let witness = with_statement_tail.unwrap_or(witness);
    let asserted = [
        "と誤記",
        "と誤植",
        "となっている",
        "となっている。誤記か",
        "と欠字",
        "と記載",
        "と旧仮名遣い",
    ]
    .iter()
    .find_map(|suffix| {
        witness
            .strip_suffix(suffix)
            .filter(|quoted| quoted.ends_with('」'))
    });
    let editorial_statement =
        (asserted.is_some() || with_statement_tail.is_some()).then_some(statement);
    let witness = asserted.unwrap_or(witness);
    let base_continuation = (target == TextVariantTarget::Text)
        .then(|| witness_continuation(witness, current, witness_start))
        .flatten();
    let witness = base_continuation.as_ref().map_or(witness, |continuation| {
        &witness[..witness.len() - continuation.text.len()]
    });
    let (base_text, quote_prefix) = match witness {
        "欠落" | "なし" | "無し" | "脱落" | "欠如" | "欠" | "脱字" => ("", 0),
        glyph
            if glyph
                .strip_prefix("※［＃")
                .and_then(|text| text.strip_suffix('］'))
                .and_then(recognize_gaiji_body)
                .is_some() =>
        {
            (glyph, 0)
        }
        quoted => {
            let text = quoted.strip_prefix('「')?.strip_suffix('」')?;
            if text.is_empty() {
                return None;
            }
            (text, '「'.len_utf8())
        }
    };
    if current.is_empty() || current.contains('\n') || base_text.contains('\n') {
        return None;
    }
    let base_span = (!base_text.is_empty()).then(|| {
        let start = witness_start + quote_prefix;
        start..start + base_text.len()
    });
    let (target, current, base_text, current_start, base_span) = if target
        == TextVariantTarget::Text
        && let (Some(current), Some(base_text), Some(span)) = (
            current
                .strip_prefix('《')
                .and_then(|text| text.strip_suffix('》')),
            base_text
                .strip_prefix('《')
                .and_then(|text| text.strip_suffix('》')),
            base_span.as_ref(),
        )
        && !current.is_empty()
        && !base_text.is_empty()
    {
        (
            TextVariantTarget::RubyReading,
            current,
            base_text,
            current_start + '《'.len_utf8(),
            Some(span.start + '《'.len_utf8()..span.end - '》'.len_utf8()),
        )
    } else {
        (target, current, base_text, current_start, base_span)
    };
    Some(TextVariant {
        witness_kind,
        target,
        current,
        base_text,
        current_span: current_start..current_start + current.len(),
        base_span,
        base_continuation,
        editorial_statement,
        source_locator,
    })
}

/// The supplied placeholder explicitly identified as concealed text, without a quantity.
#[must_use]
pub fn concealed_placeholder(source: &str) -> Option<&str> {
    let target = source
        .strip_prefix("［＃「")?
        .strip_suffix("」は伏せ字］")?;
    (!target.is_empty() && !target.contains(['「', '」', '［', '］', '\n', '\r'])).then_some(target)
}

/// Number of concealed characters explicitly attributed to the base edition.
/// Placeholder identity and quantity agreement are established at the target.
#[must_use]
pub fn base_edition_concealed_characters(source: &str) -> Option<NonZeroU32> {
    let digits = source.strip_prefix("［＃底本")?.strip_suffix("字伏字］")?;
    let count = digits.chars().try_fold(0u32, |count, ch| {
        let digit = match ch {
            '0'..='9' => u32::from(ch) - u32::from('0'),
            '０'..='９' => u32::from(ch) - u32::from('０'),
            _ => return None,
        };
        count.checked_mul(10)?.checked_add(digit)
    })?;
    NonZeroU32::new(count)
}

/// Preserve edition statements as apparatus without applying their descriptions
/// to the principal text. Mixed transcription instructions and malformed quoted
/// alternatives remain outside this grammar.
#[must_use]
pub fn edition_note(source: &str) -> Option<(EditionNoteKind, &str)> {
    edition_statement(source.strip_prefix("［＃")?.strip_suffix('］')?)
}

fn quoted_spatial_description(description: &str) -> bool {
    let Some((glyph, relation)) = description
        .strip_prefix('「')
        .and_then(|s| s.split_once('」'))
    else {
        return false;
    };
    !glyph.is_empty()
        && !glyph.contains(['「', '［', '］', '\n'])
        && (relation == "の転倒" || relation == "の右横に付く" || relation == "が左に90度回転")
}

fn located_base_edition_statement(body: &str) -> bool {
    body.strip_prefix("底本")
        .and_then(|s| s.split_once("では"))
        .is_some_and(|(locations, statement)| {
            !locations.is_empty()
                && reference_clauses(locations).all(|location| is_page_line_shaped(location.trim()))
                && !statement.trim().is_empty()
        })
}

/// `欄外に「X」の校注あり`, `上部欄外に「X」`, or a sentence ending in such a clause,
/// as in `「＊」は注釈記号。欄外に「X」の注`: X is printed in the base edition's margin.
fn marginal_statement(body: &str) -> bool {
    let Some(start) = body.find("欄外に「") else {
        return false;
    };
    let context = &body[..start];
    let Some((note, tail)) = body[start + "欄外に「".len()..].split_once('」') else {
        return false;
    };
    (matches!(context, "" | "上部") || context.ends_with('。'))
        && !note.is_empty()
        && !note.contains(['「', '［', '］', '\r', '\n'])
        && matches!(tail, "" | "の校注あり" | "の注")
}

fn retained_base_edition_statement(body: &str) -> bool {
    body.strip_prefix('「')
        .and_then(|s| s.split_once("」は底本どおり"))
        .is_some_and(|(current, continuation)| {
            !current.is_empty()
                && !current.contains(['「', '」', '\r', '\n'])
                && (continuation.is_empty() || continuation.starts_with(['、', '。']))
        })
}

/// Interpret one source-owned edition statement, including a clause within a layout marker.
#[must_use]
pub fn edition_statement(body: &str) -> Option<(EditionNoteKind, &str)> {
    if explicit_base_correction_parts(body).is_some()
        || implicit_base_correction_parts(body).is_some()
    {
        return Some((EditionNoteKind::BaseEdition, body));
    }
    if let Some(statement) = body.strip_prefix("入力者註：")
        && statement.starts_with("底本では")
        && statement.len() > "底本では".len()
    {
        return Some((EditionNoteKind::BaseEdition, body));
    }
    if let Some((context, statement)) = body.split_once("。底本では")
        && !context.is_empty()
        && !context.contains("［＃")
        && !context.starts_with(['「', '［'])
        && !["ルビの", "ここから", "ここまで", "ここで"]
            .iter()
            .any(|prefix| context.starts_with(prefix))
        && !statement.is_empty()
    {
        return Some((EditionNoteKind::BaseEdition, body));
    }
    if body
        .strip_prefix("底本にはない「")
        .and_then(|text| text.strip_suffix("」を補った"))
        .is_some_and(|text| {
            !text.is_empty() && !text.contains(['「', '」', '［', '］', '\n', '\r'])
        })
    {
        return Some((EditionNoteKind::BaseEdition, body));
    }
    let statement = body.strip_prefix("ただし").unwrap_or(body);
    if statement
        .strip_prefix("底本では")
        .or_else(|| statement.strip_prefix("底本で"))
        .or_else(|| statement.strip_prefix("底本は"))
        .or_else(|| statement.strip_prefix("底本のルビは"))
        .or_else(|| statement.strip_prefix("底本ルビは"))
        .or_else(|| {
            statement
                .strip_prefix("底本には")
                .filter(|text| !text.starts_with("ない「"))
        })
        .or_else(|| statement.strip_prefix("底本の本文に"))
        .is_some_and(|text| !text.trim().is_empty())
        || located_base_edition_statement(statement)
        || retained_base_edition_statement(statement)
        || marginal_statement(statement)
    {
        return Some((EditionNoteKind::BaseEdition, body));
    }
    if let Some(quoted) = body
        .strip_prefix("ルビの「")
        .or_else(|| body.strip_prefix('「'))
        && let Some(kind) = quoted_edition_statement(quoted)
    {
        return Some((kind, body));
    }
    if let Some((subject, description)) = body
        .split_once("は底本では")
        .or_else(|| body.split_once("は、底本では"))
        && let Some((quoted, qualification)) =
            subject.strip_prefix('「').and_then(|s| s.split_once('」'))
        && !quoted.is_empty()
        && qualification.starts_with('の')
        && qualification.len() > 'の'.len_utf8()
        && !qualification.contains(['「', '」', '［', '］', '\n'])
        && !description.is_empty()
    {
        return Some((EditionNoteKind::BaseEdition, body));
    }
    if let Some((subject, description)) = body
        .split_once("は底本では")
        .or_else(|| body.split_once("は、底本では"))
        && let Some((qualifier, quoted)) = subject.split_once('「')
        && !qualifier.is_empty()
        && !qualifier.starts_with("ここから")
        && !qualifier.starts_with("ここまで")
        && !qualifier.contains(['、', '」', '［', '］', '\n'])
        && (qualifier != "ルビの" || quoted.matches('「').count() > 0)
        && quoted.contains('」')
        && !description.is_empty()
        && !description.contains("は底本では")
    {
        return Some((EditionNoteKind::BaseEdition, body));
    }
    if let Some((subject, description)) = body
        .split_once("は底本では")
        .or_else(|| body.split_once("、底本では"))
        && !subject.is_empty()
        && !subject.contains(['、', '「', '」', '［', '］', '\n'])
        && !subject.starts_with("ここから")
        && !subject.starts_with("ここまで")
        && !subject.starts_with("ここで")
        && !description.is_empty()
    {
        return Some((EditionNoteKind::BaseEdition, body));
    }
    None
}

/// The edition a quoted statement attributes its reading to, given the text after the opening quote.
fn quoted_edition_statement(quoted: &str) -> Option<EditionNoteKind> {
    if let Some((current, witness)) = split_attribution(quoted, EditionNoteKind::FirstPublication)
        && !current.is_empty()
        && !current.contains(['「', '」', '\n'])
        && witness.starts_with('「')
        && witness.ends_with('」')
    {
        return Some(EditionNoteKind::FirstPublication);
    }
    if let Some((current, witness)) = split_attribution(quoted, EditionNoteKind::BaseEdition)
        && !current.is_empty()
        && witness.starts_with('「')
        && let Some((base, bibliography)) = witness.split_once("」。以下の本では「")
        && base.len() > '「'.len_utf8()
        && bibliography.contains('『')
        && bibliography.contains('』')
    {
        return Some(EditionNoteKind::BaseEdition);
    }
    if let Some((current, description)) = split_attribution(quoted, EditionNoteKind::BaseEdition)
        && !current.is_empty()
        && !description.is_empty()
        && (!description.starts_with(['「', '※']) || quoted_spatial_description(description))
    {
        return Some(EditionNoteKind::BaseEdition);
    }
    None
}

/// Templates for source statements about a layout that the text cannot carry:
/// braces joining lines, set-off placements, row and column labels. In each
/// template `「○○」` stands for one quoted operand and `N` for a run of digits.
const LAYOUT_STATEMENT_TEMPLATES: &[&str] = &[
    "「○○」と「○○」の句の上には、この二つの句を括る波括弧あり",
    "「○○」は前のN行にわたる",
    "「○○」は前のN行にわたる」",
    "「○○」と「○○」の中間に「○○」",
    "「○○」は「○○」と「○○」の二行の中央",
    "以下「○○」と「○○」は「○○」の下で二行に分かれ、「○○」「○○」の下に上向きのくくり記号",
    "「○○」は前N行の中央、下に",
    "「○○」はN行の中央、括弧はN行にわたる波括弧",
    "「○○」は天地左右中央",
    "上記の詞書は、ポイントを下げて中央やや下がり目に",
    "段組み適用外",
    "N行目",
    "横書き、「○○」はアクセント（∨）付き",
    "横書き、「○○」の「○○」と、「○○」の「○○」と「○○」の間にアクセント（∨）、「○○」に上線",
    "この行は下に横組みで",
    "この行は枠の上に横書き",
    "左図の解説文終わり",
    "右図の解説文終わり",
    "（）内の文字全てに傍点、ただし読点をのぞく",
    "以下の「○○」は罫で囲む",
    "「○○」は罫囲み、手紙の中では上記の印",
    "ここからページ上部横組み",
    "ページ上部横組み終わり",
    "ここからページ下部縦組み",
    "ページ下部縦組み終わり",
    "ここから本文外に横書き",
    "ここで本文外横書き終わり",
    "前の行とはN.N行アキ、「○○」はゴシック体",
];

/// Templates for clauses of a layout marker that state what the scope holds
/// or how it is set off, without a geometry the marker can carry.
const LAYOUT_CLAUSE_TEMPLATES: &[&str] = &[
    "プログラム",
    "----で挾まれた部分が一つの仕切り内",
    "N行アキ",
    "はじめの「○○」のみN字下げ",
    "（「○○」と注記した箇所を除く）。上段に演目、下段に国名や出演者名など。演目は大文字",
];

/// Whether `body` is a source statement about a layout the text cannot carry.
#[must_use]
pub fn layout_statement(body: &str) -> bool {
    LAYOUT_STATEMENT_TEMPLATES
        .iter()
        .any(|template| matches_layout_template(template, body))
}

/// Whether one clause of a layout marker is a statement kept as an explanation.
#[must_use]
pub fn layout_clause_statement(clause: &str) -> bool {
    layout_statement(clause)
        || LAYOUT_CLAUSE_TEMPLATES
            .iter()
            .any(|template| matches_layout_template(template, clause))
}

fn matches_layout_template(template: &str, body: &str) -> bool {
    const OPERAND: &str = "「○○」";
    if let Some(rest) = template.strip_prefix(OPERAND) {
        let Some(contents) = body.strip_prefix('「') else {
            return false;
        };
        return contents.char_indices().any(|(offset, character)| {
            character == '」'
                && offset > 0
                && !contents[..offset].contains(['「', '」', '［', '］', '\r', '\n'])
                && matches_layout_template(rest, &contents[offset + '」'.len_utf8()..])
        });
    }
    if let Some(rest) = template.strip_prefix('N') {
        let digits = body
            .char_indices()
            .take_while(|(_, character)| {
                character.is_ascii_digit() || ('０'..='９').contains(character)
            })
            .map(|(offset, character)| offset + character.len_utf8())
            .last();
        return digits.is_some_and(|end| matches_layout_template(rest, &body[end..]));
    }
    template
        .chars()
        .next()
        .map_or(body.is_empty(), |character| {
            body.strip_prefix(character).is_some_and(|body| {
                matches_layout_template(&template[character.len_utf8()..], body)
            })
        })
}

#[cfg(test)]
mod tests {
    use super::{
        EditionNoteKind, TextVariantTarget, base_edition_concealed_characters,
        named_text_apparatus, source_legibility_note, text_variant, uncertain_correction,
    };

    #[test]
    fn named_apparatus_groups_sources_by_reading() {
        let source = "［＃「ことは」は、『谷崎潤一郎全集　第十九巻』（中央公論新社2015年6月10日初版発行）と『谷崎潤一郎全集　第十五卷』（中央公論社1968年1月25日発行）では「ことを」］";
        let apparatus = named_text_apparatus(source).unwrap();
        assert_eq!(apparatus.current, "ことは");
        assert_eq!(&source[apparatus.current_span], "ことは");
        assert_eq!(apparatus.readings.len(), 1);
        assert_eq!(apparatus.readings[0].witnesses.len(), 2);
        assert_eq!(apparatus.readings[0].text, "ことを");
        assert_eq!(&source[apparatus.readings[0].text_span.clone()], "ことを");
        for witness in &apparatus.readings[0].witnesses {
            assert_eq!(&source[witness.span.clone()], witness.label);
        }
    }

    #[test]
    fn named_apparatus_preserves_distinct_readings() {
        let source = "［＃「、」は、『谷崎潤一郎全集　第十九巻』（中央公論新社2015年6月10日初版発行）では「。」、『谷崎潤一郎全集　第十五卷』（中央公論社1968年1月25日発行）では「、」］";
        let apparatus = named_text_apparatus(source).unwrap();
        assert_eq!(
            apparatus
                .readings
                .iter()
                .map(|reading| reading.text)
                .collect::<Vec<_>>(),
            ["。", "、"]
        );
        assert!(named_text_apparatus("［＃「字」は筑摩版では末尾の「字」なし］").is_none());
        assert!(named_text_apparatus("［＃「字」は以下の本では「別字」。説明］").is_none());
    }

    #[test]
    fn named_apparatus_reading_can_contain_source_markup() {
        for (source, expected) in [
            (
                "［＃「海蓴（こも）の柄」は本文の書き下し文では「海※［＃「くさかんむり／溥のつくり」、U+84AA、59-本文-4］（こも）の柄」］",
                "海※［＃「くさかんむり／溥のつくり」、U+84AA、59-本文-4］（こも）の柄",
            ),
            (
                "［＃「纒向の日代の宮」は本文の書き下し文では「纏向《まきむく》の日代《ひしろ》の宮」］",
                "纏向《まきむく》の日代《ひしろ》の宮",
            ),
        ] {
            let apparatus = named_text_apparatus(source).unwrap_or_else(|| panic!("{source}"));
            assert_eq!(apparatus.readings[0].witnesses[0].label, "本文の書き下し文");
            assert_eq!(apparatus.readings[0].text, expected);
            assert_eq!(&source[apparatus.readings[0].text_span.clone()], expected);
        }
        assert!(
            named_text_apparatus("［＃「字」は本文の書き下し文では「※［＃未閉じの指定」］")
                .is_none()
        );
    }

    #[test]
    fn uncertain_corrections_and_legibility_notes_keep_their_locators() {
        let correction = uncertain_correction("「霙」の誤り？、400-7").unwrap();
        assert_eq!(correction.target_text, None);
        assert_eq!(correction.proposed_text, "霙");
        assert_eq!(correction.source_locator, Some("400-7"));
        assert_eq!(&"「霙」の誤り？、400-7"[correction.proposed_span], "霙");
        let correction =
            uncertain_correction("「勧《つと》めた」は「勤《つと》めた」の誤記か").unwrap();
        assert_eq!(correction.target_text, Some("勧《つと》めた"));
        assert_eq!(correction.proposed_text, "勤《つと》めた");
        assert_eq!(correction.source_locator, None);
        assert_eq!(
            &"「勧《つと》めた」は「勤《つと》めた」の誤記か"[correction.target_span.unwrap()],
            "勧《つと》めた"
        );
        assert_eq!(
            &"「勧《つと》めた」は「勤《つと》めた」の誤記か"[correction.proposed_span],
            "勤《つと》めた"
        );
        assert_eq!(source_legibility_note("印刷不鮮明、87-14"), Some("87-14"));
        assert!(uncertain_correction("「霙」の誤り、400-7").is_none());
    }

    #[test]
    fn every_attested_error_question_is_an_uncertain_correction() {
        for body in [
            "「偶」は「隅」の誤りか",
            "「縁」は「緑」の誤植か",
            "「ならぬば」は「ならねば」の誤記と思われる",
            "「み」余分か、それとも「見」か？",
        ] {
            let correction = uncertain_correction(body).unwrap_or_else(|| panic!("{body}"));
            assert_eq!(
                &body[correction.target_span.unwrap()],
                correction.target_text.unwrap()
            );
            assert_eq!(&body[correction.proposed_span], correction.proposed_text);
        }
        for body in [
            "「三千円」の誤記か",
            "「マント」の誤植と思われる",
            "「今の寛永寺なのです。」の誤り？",
        ] {
            let correction = uncertain_correction(body).unwrap_or_else(|| panic!("{body}"));
            assert_eq!(correction.target_text, None);
            assert_eq!(correction.source_locator, None);
            assert_eq!(&body[correction.proposed_span], correction.proposed_text);
        }
        assert!(uncertain_correction("「偶」は「隅」の誤り").is_none());
        assert!(uncertain_correction("「偶」は「隅」「角」の誤りか").is_none());
        assert!(uncertain_correction("「み」余分か、それとも「見」").is_none());
        assert!(source_legibility_note("印刷不鮮明、位置").is_none());
    }

    #[test]
    fn edition_attribution_requires_a_statement_outside_quoted_text() {
        for source in [
            "［＃底本は　］",
            "［＃底本7-10では　］",
            "［＃底本U+884Eでは「衎」］",
            "［＃「底本は」に傍点］",
            "［＃「で」は底本どおりではない］",
        ] {
            assert!(super::edition_note(source).is_none(), "{source}");
        }
    }

    #[test]
    fn embedded_glyph_statement_owns_only_a_complete_reference_tail() {
        let statement = "底本はこの字を「さんずい＋「仰」のつくり」と作字上の誤り";
        let source =
            format!("※［＃「※」は「さんずい＋卯」、第4水準2-78-35、17-上-9、{statement}］");
        assert_eq!(super::gaiji_edition_note(&source), Some(statement));
        for source in [
            format!("※［＃「字」、U+6CD6、{statement}、未知］"),
            format!("※［＃「字」、U+6CD6、未知、{statement}］"),
            format!("※［＃「{statement}」、U+6CD6］"),
            "※［＃「字」、U+6CD6、底本はこの字を「」と作字上の誤り］".to_owned(),
        ] {
            assert!(super::gaiji_edition_note(&source).is_none(), "{source}");
        }
    }

    #[test]
    fn concealment_quantity_retains_only_explicit_base_edition_character_counts() {
        for (source, quantity) in [
            ("［＃底本１字伏字］", 1),
            ("［＃底本２字伏字］", 2),
            ("［＃底本４字伏字］", 4),
            ("［＃底本12字伏字］", 12),
        ] {
            assert_eq!(
                base_edition_concealed_characters(source).unwrap().get(),
                quantity
            );
        }
        for source in [
            "［＃底本０字伏字］",
            "［＃底本字伏字］",
            "［＃底本2行伏字］",
            "［＃初出２字伏字］",
            "［＃底本２字伏字、復元］",
            "［＃底本4294967296字伏字］",
        ] {
            assert!(
                base_edition_concealed_characters(source).is_none(),
                "{source}"
            );
        }
    }

    #[test]
    fn quoted_alternatives_preserve_the_attribution_without_asserting_error() {
        let reading = text_variant("［＃ルビの「ざる」は底本では「さる」］").unwrap();
        assert_eq!(reading.target, TextVariantTarget::RubyReading);
        assert_eq!((reading.current, reading.base_text), ("ざる", "さる"));
        let stray = text_variant("［＃「ルビの「ヰタリテ」は底本では「ヰリリテ」］").unwrap();
        assert_eq!(stray.target, TextVariantTarget::RubyReading);
        assert_eq!((stray.current, stray.base_text), ("ヰタリテ", "ヰリリテ"));
        let text = text_variant("［＃「甍の」は底本では「薨の」］").unwrap();
        assert_eq!(text.target, TextVariantTarget::Text);
        assert_eq!((text.current, text.base_text), ("甍の", "薨の"));
    }

    #[test]
    fn explicit_base_corrections_keep_both_supplied_readings() {
        for source in [
            "［＃底本「生まれてた」を「生まれでた」に訂正］",
            "［＃底本の「低流」を「底流」に訂正］",
            "［＃底本の「お」を「を」に変更］",
        ] {
            let variant = text_variant(source).unwrap();
            assert_eq!(variant.witness_kind, EditionNoteKind::BaseEdition);
            assert_eq!(variant.target, TextVariantTarget::Text);
            assert_eq!(&source[variant.current_span], variant.current);
            assert_eq!(
                &source[variant.base_span.clone().unwrap()],
                variant.base_text
            );
            assert_eq!(
                variant.editorial_statement,
                Some(&source["［＃".len()..source.len() - '］'.len_utf8()])
            );
        }
        for source in [
            "［＃底本「旧」を「」に訂正］",
            "［＃底本「旧」を「新」へ訂正］",
            "［＃底本「旧」を訂正］",
            "［＃底本「旧」を「新」を「別」に訂正］",
        ] {
            assert!(text_variant(source).is_none(), "{source}");
        }
    }

    #[test]
    fn implicit_base_corrections_do_not_invent_a_replacement() {
        for source in [
            "［＃底本「お」を訂正］",
            "［＃底本の「低流」を訂正］",
            "［＃底本の「烏」を変更］",
            "［＃底本「ずらり」を修正］",
            "［＃底本の「。」を削除］",
        ] {
            let correction = super::implicit_base_correction(source).unwrap();
            assert_eq!(&source[correction.base_span], correction.base_text);
        }
        for source in [
            "［＃底本「」を訂正］",
            "［＃底本「旧」を「新」に訂正］",
            "［＃底本「旧」を改める］",
        ] {
            assert!(
                super::implicit_base_correction(source).is_none(),
                "{source}"
            );
        }
    }

    #[test]
    fn omission_notes_distinguish_assertions_from_questions() {
        for body in [
            "省略",
            "図省略",
            "「年表」省略",
            "著者註の「＊」が欠字",
            "「　」は欠字",
        ] {
            assert_eq!(
                super::omission_note_certainty(body),
                Some(super::OmissionNoteCertainty::Asserted)
            );
        }
        for body in [
            "「人」が脱落か？",
            "「や」が欠如か",
            "句点が抜けていると考えられる",
        ] {
            assert_eq!(
                super::omission_note_certainty(body),
                Some(super::OmissionNoteCertainty::Uncertain)
            );
        }
        for body in ["「省略」は太字", "「人」が脱落", "省略か", "「人」は欠字"]
        {
            assert_eq!(super::omission_note_certainty(body), None);
        }
    }

    #[test]
    fn ruby_reading_annotations_keep_target_and_note_separate() {
        let annotation =
            super::ruby_reading_annotation("ルビの「ハシビトノムラジオユ」に〈原〉の注記").unwrap();
        assert_eq!(annotation.target, "ハシビトノムラジオユ");
        assert_eq!(annotation.note, "〈原〉");
        let sic = super::ruby_reading_annotation("ルビの「しう」にママの注記").unwrap();
        assert_eq!(sic.target, "しう");
        assert_eq!(sic.note, "ママ");
        let quoted = super::ruby_reading_annotation("ルビの「たい」に「ママ」の注記").unwrap();
        assert_eq!((quoted.target, quoted.note), ("たい", "ママ"));
        let below =
            super::ruby_reading_annotation("ルビ「どんす」の下に「（ママ）」の注記").unwrap();
        assert_eq!((below.target, below.note), ("どんす", "（ママ）"));

        for body in [
            "ルビ「どんす」に「（ママ）」の注記",
            "ルビの「」に〈原〉の注記",
            "ルビの「読み」にの注記",
            "「読み」に〈原〉の注記",
            "ルビの「読み」に〈原〉",
        ] {
            assert!(super::ruby_reading_annotation(body).is_none(), "{body}");
        }
    }

    #[test]
    fn quotation_characters_are_target_data_and_keep_exact_fragment_extents() {
        for source in [
            "［＃「……』」は底本では「……」」］",
            "［＃「』」は、底本では「」」］",
            "［＃「』」は底本では、「」」］",
        ] {
            let variant = text_variant(source).unwrap();
            assert_eq!(&source[variant.current_span.clone()], variant.current);
            assert_eq!(
                &source[variant.base_span.clone().unwrap()],
                variant.base_text
            );
            assert!(variant.base_text.ends_with('」'));
        }
    }

    #[test]
    fn variant_assertions_and_source_locators_preserve_whole_statements() {
        for (source, expected) in [
            ("［＃「藤井」は底本では「蔵井」、412-13］", "蔵井"),
            ("［＃「山」は底本では「出」と誤植、25-上-1］", "出"),
            ("［＃「学士」は底本では「博士」、172-上段-8］", "博士"),
            ("［＃「学士」は底本では「博士」、172-下段-2］", "博士"),
            ("［＃「平和」は底本では「価格」、正誤表による訂正］", "価格"),
            ("［＃「なんとも」は底本では「何となく」と記載］", "何となく"),
            (
                "［＃「そういう」は底本では「さういう」と旧仮名遣い、56-9］",
                "さういう",
            ),
            (
                "［＃「如何《どう》」は底本では「如何《どう》う」と「う」が重複］",
                "如何《どう》う",
            ),
        ] {
            let variant = text_variant(source).unwrap_or_else(|| panic!("{source}"));
            assert_eq!(variant.base_text, expected);
            assert_eq!(&source[variant.base_span.unwrap()], expected);
            assert_eq!(
                variant.editorial_statement,
                Some(&source["［＃".len()..source.len() - '］'.len_utf8()])
            );
        }
    }

    #[test]
    fn original_text_errors_preserve_authority_targets_and_literal_quote_escapes() {
        for (source, current, witness) in [
            (
                "［＃「しました。」は原文では「しました、」と誤記、136-2］",
                "しました。",
                "しました、",
            ),
            ("［＃【「】は原文では【」】と誤記、43-10］", "「", "」"),
        ] {
            let variant = text_variant(source).unwrap_or_else(|| panic!("{source}"));
            assert_eq!(variant.witness_kind, EditionNoteKind::OriginalText);
            assert_eq!((variant.current, variant.base_text), (current, witness));
            assert_eq!(&source[variant.current_span], current);
            assert_eq!(&source[variant.base_span.unwrap()], witness);
            assert_eq!(
                variant.editorial_statement,
                Some(&source["［＃".len()..source.len() - '］'.len_utf8()])
            );
        }
    }

    #[test]
    fn explicit_attribution_accepts_supplied_particles_and_direct_glyph_witnesses() {
        for source in [
            "［＃「ひとりっきり」では底本では「ひとりきりっ」］",
            "［＃「ことゞも」は底本で「ことヾも」］",
            "［＃「もんは」は底本は「もんと」］",
            "［＃「二三ヶ月」は底本「二三ヶ」］",
            "［＃「餡」は底本では※［＃「飮のへん＋稻のつくり」、第4水準2-92-68］］",
        ] {
            let variant = text_variant(source).unwrap_or_else(|| panic!("{source}"));
            assert_eq!(&source[variant.current_span], variant.current);
            assert_eq!(&source[variant.base_span.unwrap()], variant.base_text);
        }
        assert!(text_variant("［＃「字」は底本では※［＃未知の指示］］").is_none());
    }

    #[test]
    fn qualified_base_edition_geometry_is_documentary_apparatus() {
        for source in [
            "［＃左図の解説文、底本では横組み］",
            "［＃「differentiation」の左から２番目のtは底本では上下逆］",
            "［＃「5」の傍線は底本では欠落］",
            "［＃「士は、」の後は、底本では改行１字下げ］",
            "［＃「％」は底本では「・」の右横に付く］",
            "［＃ルビの「ゲトウ」は、底本では「ゲ」が左に90度回転］",
            "［＃底本のルビは「おのず」］",
            "［＃底本ルビは「よみが」、7-下段-15］",
            "［＃底本にはない「。」を補った］",
            "［＃底本には組み体裁上の誤りがあり］",
            "［＃ただし底本では３字下げ］",
        ] {
            let (_, statement) = super::edition_note(source).unwrap_or_else(|| panic!("{source}"));
            assert_eq!(
                statement,
                &source["［＃".len()..source.len() - '］'.len_utf8()]
            );
            assert!(text_variant(source).is_none());
        }
        assert!(super::edition_note("［＃「字」は底本では「他字］").is_none());
        assert!(super::edition_note("［＃底本にはない「」を補った］").is_none());
        assert!(super::edition_note("［＃「字」は底本では「他字。底本ではそうだ。］").is_none());
    }

    #[test]
    fn editorial_explanations_retain_witnesses_and_complete_assertions() {
        for source in [
            "［＃「１７５９２」は底本では「１７３９２」。【例題一】と同一と考えられるため、【例題一】に合わせました。］",
            "［＃「幽明の交通」は底本では「幽明交通」（本文は「幽明の交通」）］",
        ] {
            let variant = text_variant(source).unwrap_or_else(|| panic!("{source}"));
            assert_eq!(&source[variant.base_span.unwrap()], variant.base_text);
            assert!(variant.editorial_statement.is_some());
        }
        for source in [
            "［＃底本で第二十四頁にあるのは、例題七（底本では例題八）です。］",
            "［＃次の手紙は「遺書」として書かれ投函されなかった。底本では第十九巻の巻末に収録］",
            "［＃入力者註：底本では「中国」「支那」が共に使われているが、「中国」に統一した。］",
        ] {
            let (_, statement) = super::edition_note(source).unwrap_or_else(|| panic!("{source}"));
            assert_eq!(
                statement,
                &source["［＃".len()..source.len() - '］'.len_utf8()]
            );
        }
    }

    #[test]
    fn ambiguous_or_incomplete_notes_remain_unclassified() {
        for source in [
            "［＃ルビの「ざる」は底本では「さる」",
            "［＃ルビの「」は底本では「さる」］",
            "［＃ルビの「ざる」は底本では「さる」とある］",
            "［＃「字」は底本では「字」は底本では「字」］",
            "［＃ルビの「ざる」は底本では「さ\nる」］",
            "［＃「字」は底本で「他字］",
            "［＃「字」は底本「他字」の誤植か］",
        ] {
            assert!(text_variant(source).is_none(), "{source}");
        }
    }

    #[test]
    fn every_sic_spelling_keeps_the_quoted_target() {
        for body in [
            "「島田屋の」はママ」",
            "「島田屋の」はママ",
            "「島田屋の」は底本のまま",
            "「島田屋の」は底本ママ",
            "ルビの「島田屋の」は底本のママ",
        ] {
            let sic = super::sic_target(body).unwrap_or_else(|| panic!("{body}"));
            assert_eq!(
                (sic.target, &body[sic.target_span]),
                ("島田屋の", "島田屋の")
            );
            assert_eq!(sic.correction_span, None);
        }
        assert!(super::sic_target("「島田屋の」」はママ」").is_none());
    }

    #[test]
    fn a_translation_error_is_a_sic_note_with_the_stated_correction() {
        let body = "「弟」は誤訳で本当は「兄」";
        let sic = super::sic_target(body).unwrap();
        assert_eq!(&body[sic.target_span], "弟");
        assert_eq!(&body[sic.correction_span.unwrap()], "兄");
        assert!(!sic.ruby_reading);
        for body in [
            "「弟」は誤訳で本当は「」",
            "「弟」は誤訳で本当は「兄",
            "「弟」は誤訳で本当は「兄」「姉」",
            "ルビの「おとうと」は誤訳で本当は「あに」",
        ] {
            assert!(super::sic_target(body).is_none(), "{body}");
        }
    }

    #[test]
    fn located_figures_keep_page_position_kind_and_description_separate() {
        use super::{LocatedFigureKind, LocatedFigurePosition, located_figure_note};

        for (source, page, position, kind, description) in [
            (
                "底本１７７頁に地図あり",
                "１７７",
                None,
                LocatedFigureKind::Map,
                None,
            ),
            (
                "底本１９７頁上に地図あり",
                "１９７",
                Some(LocatedFigurePosition::Top),
                LocatedFigureKind::Map,
                None,
            ),
            (
                "底本１９８頁右上に図あり",
                "１９８",
                Some(LocatedFigurePosition::TopRight),
                LocatedFigureKind::Figure,
                None,
            ),
            (
                "底本２７頁、左上に図あり",
                "２７",
                Some(LocatedFigurePosition::TopLeft),
                LocatedFigureKind::Figure,
                None,
            ),
            (
                "底本１４２頁右上に、持久戦争と決戦戦争の移り変わりを示した図あり",
                "１４２",
                Some(LocatedFigurePosition::TopRight),
                LocatedFigureKind::Figure,
                Some("持久戦争と決戦戦争の移り変わりを示した"),
            ),
        ] {
            let note = located_figure_note(source).unwrap_or_else(|| panic!("{source}"));
            assert_eq!(note.page, page);
            assert_eq!(note.position, position);
            assert_eq!(note.kind, kind);
            assert_eq!(note.description, description);
        }
        for source in [
            "底本頁に図あり",
            "底本一頁に図あり",
            "底本１７７頁地図あり",
            "底本１７７頁右下に図あり",
            "底本１７７頁に図あり、詳細",
            "底本１７７頁に「図」あり",
        ] {
            assert!(located_figure_note(source).is_none(), "{source}");
        }
    }
}
