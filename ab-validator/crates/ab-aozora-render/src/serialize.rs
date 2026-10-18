//! Aozora-source serializer over the semantic AST.
//!
//! Serializes the normalized text back to Aozora source in a single forward
//! walk, dispatching each PUA sentinel through an [`LexOutput`]'s
//! [`Registry`](ab_aozora_syntax::ast::Registry) and resolving every
//! [`StrId`](ab_aozora_syntax::ast::StrId) /
//! [`ContentRange`] /
//! [`SegRange`](ab_aozora_syntax::ast::SegRange) against the [`NodeStore`].
//!
//! The container-marker spelling (`emit_container_open` / `emit_container_close`)
//! comes from [`crate::spelling::source`]; those emitters read only `Copy`
//! `RegionFormat` / `RegionClose` discriminants, so the byte spelling is
//! defined in one place. This module contains only the emitters that read the
//! AST.
//!
//! The serializer runs a decorative-rule isolate post-pass so that
//! `serialize ∘ parse` is a round-trip fixed point.

use ab_aozora_spec::NormalizedOffset;
use core::fmt::{self, Write};
use core::mem;
use std::collections::BTreeMap;

use crate::spelling::source::{
    emit_container_close, emit_container_open, emit_line, emit_section_break, heading_level_word,
    heading_style_keyword,
};
use crate::walk::{SentinelKind, WalkSink, walk};
use ab_aozora_pipeline::{
    has_long_rule_line, isolate_decorative_rules, sentinel_is_blank_line_padded,
};
use ab_aozora_syntax::accent::ACCENT_TABLE;
use ab_aozora_syntax::ast::{
    AngleQuote, ContainerEnd, Content, ContentRange, Directive, ForwardFormat, Gaiji,
    GaijiCanonicalOwned, Heading, HeadingHint, Illustration, Kunten, KuntenKind, LexOutput,
    MarginNote, Node, NodeRef, NodeStore, Ruby, Segment,
};
use ab_aozora_syntax::degraded::degraded_directive;
use ab_aozora_syntax::format::ForwardOrigin;
use ab_aozora_syntax::lint::canonical_directive;
use ab_aozora_syntax::{
    AccentMark, DecorationPosition, DirectiveKind, EnclosureKind, FollowingPlacement, ForwardAttr,
    GaijiMarkerPlacement, QualifiedBreak, QualifiedBreakKind, RubySide,
};

/// Which notation-hygiene catalogue tiers a serialize / render pass applies
/// to a `DirectiveKind::Unknown` near-miss.
///
/// The default (`Off`) is byte-identical and applies no catalogue judgement:
/// an Unknown directive round-trips its raw bytes verbatim (serialize) /
/// renders as an inert `<span class="aozora-directive" hidden>` (render), so
/// output does not depend on the catalogue.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DirectiveNormalization {
    /// Writes every directive's raw bytes verbatim.
    #[default]
    Off,
    /// Tier1 only. Rewrites verified zero-false-positive near-misses (per
    /// [`canonical_directive`]) to canonical form. Used by `fmt --fix`
    /// and `render --normalize`.
    Canonical,
    /// Tier1 and Tier2. Also reduces lossy or judgment-based degraded forms
    /// (per [`degraded_directive`]) that Tier1 does not rewrite. Only the opt-in
    /// renderer ([`crate::render_html_normalized`] via `render --degraded`)
    /// constructs this variant, and no persistent-write path does, so Tier2
    /// reductions affect only `--degraded` render output and never rewrite source.
    Degraded,
}

/// Options controlling how the AST is re-emitted to Aozora source.
///
/// With the default (`directives: Off`), every directive round-trips its
/// `raw` bytes verbatim, including `DirectiveKind::Unknown` near-misses
/// flagged by notation-hygiene lints. With `Canonical` (`aozora fmt --fix`),
/// the serializer rewrites those flagged near-misses to their canonical
/// spelling through [`canonical_directive`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SerializeOptions {
    /// Which notation-hygiene tiers to apply to `DirectiveKind::Unknown`
    /// near-misses. Only the opt-in renderer passes `Degraded`.
    pub directives: DirectiveNormalization,
}

/// Serialize an [`LexOutput`] back to Aozora source text.
///
/// `serialize ∘ parse` reaches a fixed point after one pass. The
/// decorative-rule isolate post-pass, which runs on every call
/// (`has_long_rule_line` pre-check, then `isolate_decorative_rules`),
/// normalizes decorative rule lines so a second pass re-parses and
/// re-serializes to identical bytes.
///
/// # Panics
///
/// Does not panic: writing to a `String` through [`Write`] cannot fail.
#[must_use]
pub fn serialize(out: &LexOutput) -> String {
    serialize_with(out, SerializeOptions::default())
}

/// Serialize an [`LexOutput`] back to Aozora source text with explicit
/// [`SerializeOptions`].
///
/// With the default options this is identical to [`serialize()`]. With
/// `directives` not `Off`, `DirectiveKind::Unknown` near-misses are rewritten
/// to canonical form (see [`SerializeOptions`]); the rewrite is idempotent
/// because a canonical body parses to a recognized (non-`Unknown`) node and so
/// is never revisited on a second pass.
///
/// # Panics
///
/// Does not panic: writing to a `String` through [`Write`] cannot fail.
#[must_use]
pub fn serialize_with(out: &LexOutput, opts: SerializeOptions) -> String {
    let mut s = String::with_capacity(out.normalized.len().saturating_mul(2));
    serialize_into_with(out, &mut s, opts).expect("writing to a String never fails");
    let raw = s;
    if has_long_rule_line(&raw) {
        isolate_decorative_rules(&raw)
    } else {
        raw
    }
}

/// Serialize an [`LexOutput`] into the given writer.
///
/// # Errors
///
/// Propagates write errors from `writer`.
///
/// # Panics
///
/// Panics if the normalized text exceeds `u32::MAX` bytes, the width of the
/// lexer's `Span` offsets; the sanitize stage caps input below that length.
pub fn serialize_into<W: Write>(out: &LexOutput, writer: &mut W) -> fmt::Result {
    serialize_into_with(out, writer, SerializeOptions::default())
}

/// Serialize an [`LexOutput`] into the given writer with explicit
/// [`SerializeOptions`].
///
/// # Errors
///
/// Propagates write errors from `writer`.
///
/// # Panics
///
/// Panics if the normalized text exceeds `u32::MAX` bytes, the width of the
/// lexer's `Span` offsets; the sanitize stage caps input below that length.
pub fn serialize_into_with<W: Write>(
    out: &LexOutput,
    writer: &mut W,
    opts: SerializeOptions,
) -> fmt::Result {
    let mut sink = SerializeSink {
        store: &out.store,
        out: writer,
        directives: opts.directives,
        source_boundaries: source_boundary_emissions(out),
        held_newlines: 0,
        drop_newlines: 0,
    };
    walk(out, &mut sink)
}

/// Preserve physical source boundaries while omitting purely structural endpoints.
fn source_boundary_emissions(out: &LexOutput) -> BTreeMap<NormalizedOffset, &str> {
    let mut emissions = BTreeMap::new();
    for pair in &out.container_pairs {
        if matches!(pair.source_end, ContainerEnd::SourceReplacement(_)) {
            emissions.insert(pair.close, "");
        }
    }
    for pair in out.container_pairs.windows(2) {
        if let ContainerEnd::ClosingMarker(span) = pair[0].source_end
            && pair[1].source_end == pair[0].source_end
        {
            emissions
                .entry(pair[0].close)
                .or_insert_with(|| span.slice(&out.sanitized));
            emissions.insert(pair[1].close, "");
        }
    }
    emissions
}

/// [`WalkSink`] that re-emits Aozora source text from the AST,
/// passing the [`NodeStore`], which resolves interned payloads, to every AST
/// emitter.
struct SerializeSink<'a, W: Write> {
    source_boundaries: BTreeMap<NormalizedOffset, &'a str>,
    store: &'a NodeStore,
    out: &'a mut W,
    /// Which notation-hygiene tiers to apply to `DirectiveKind::Unknown`
    /// near-misses (`Off` = verbatim; `Canonical` = Tier1; `Degraded` = Tier1+Tier2).
    directives: DirectiveNormalization,
    /// Newlines read from the normalized text and not yet written, held so
    /// the next block marker can remove the blank line the normalizer inserted
    /// before it. See [`Self::unpad`].
    held_newlines: usize,
    /// Newlines still to be dropped from the front of the next plain run: the
    /// blank line the normalizer inserted after a block marker.
    drop_newlines: usize,
}

impl<W: Write> SerializeSink<'_, W> {
    fn write_held(&mut self) -> fmt::Result {
        for _ in 0..mem::take(&mut self.held_newlines) {
            self.out.write_char('\n')?;
        }
        Ok(())
    }

    /// Remove the normalizer's blank-line padding around one marker.
    ///
    /// `lex` writes `\n\n` on each side of a block sentinel so that the
    /// line-oriented recognizers have a boundary to work with, whether or not
    /// the source had one. Those newlines are not source. Writing them out
    /// makes `serialize` insert blank lines the author never wrote, and on
    /// input whose brackets do not balance it makes it insert *parse-relevant*
    /// ones: the pair stage expires an open bracket at a newline, so a bracket
    /// that the first parse kept open has expired when the next parse reaches
    /// the closing `］`, and a directive that was at top level is parsed
    /// inside a body.
    fn unpad(&mut self, padded: bool) {
        if padded {
            self.held_newlines = self.held_newlines.saturating_sub(2);
            self.drop_newlines = 2;
        } else {
            self.drop_newlines = 0;
        }
    }

    fn emit(&mut self, kind: SentinelKind, node: NodeRef) -> fmt::Result {
        match (kind, node) {
            (SentinelKind::Inline, NodeRef::Inline(n))
            | (SentinelKind::BlockLeaf, NodeRef::BlockLeaf(n)) => {
                emit_aozora(n, self.store, self.out, self.directives)
            }
            (SentinelKind::BlockOpen, NodeRef::BlockOpen(open)) => {
                emit_container_open(open, self.store, self.out)
            }
            (SentinelKind::BlockClose, NodeRef::BlockClose(close)) => {
                emit_container_close(close, self.store, self.out)
            }
            // A sentinel without a corresponding registry entry, or a
            // kind/variant mismatch, emits nothing.
            _ => Ok(()),
        }
    }
}

impl<W: Write> WalkSink for SerializeSink<'_, W> {
    // Serialization copies `\n` verbatim, so it is not a structural event.
    const WANTS_NEWLINES: bool = false;

    fn on_text(&mut self, mut text: &str) -> fmt::Result {
        while self.drop_newlines > 0 {
            match text.strip_prefix('\n') {
                Some(rest) => {
                    text = rest;
                    self.drop_newlines -= 1;
                }
                None => self.drop_newlines = 0,
            }
        }
        let body = text.trim_end_matches('\n');
        // A run that is only newlines adds all of them to the held count, so
        // padding on both sides of an empty run cancels exactly.
        if body.is_empty() {
            self.held_newlines += text.len();
            return Ok(());
        }
        self.write_held()?;
        self.out.write_str(body)?;
        self.held_newlines = text.len() - body.len();
        Ok(())
    }

    fn on_node_at(
        &mut self,
        position: NormalizedOffset,
        kind: SentinelKind,
        node: NodeRef,
    ) -> fmt::Result {
        self.unpad(sentinel_is_blank_line_padded(node));
        self.write_held()?;
        if let Some(source) = self.source_boundaries.get(&position) {
            self.out.write_str(source)
        } else {
            self.emit(kind, node)
        }
    }

    fn on_node(&mut self, kind: SentinelKind, node: NodeRef) -> fmt::Result {
        self.unpad(sentinel_is_blank_line_padded(node));
        self.write_held()?;
        self.emit(kind, node)
    }

    fn finish(&mut self) -> fmt::Result {
        self.write_held()
    }
}

/// Dispatch an owned [`Node`] to its per-variant source emitter.
fn emit_aozora<W: Write>(
    node: Node,
    store: &NodeStore,
    out: &mut W,
    directives: DirectiveNormalization,
) -> fmt::Result {
    match node {
        Node::Ruby(r) => emit_ruby(&r, store, out),
        Node::Format(f) => emit_format(&f, store, out),
        Node::Gaiji(g) => emit_gaiji(&g, store, out),
        Node::Kunten(k) => emit_kunten(k, store, out),
        Node::IterationMark(mark) => out.write_str(mark.source()),
        Node::Directive(a) => emit_annotation(a, store, out, directives),
        Node::AngleQuote(d) => emit_angle_quote(d, store, out),
        Node::MarginNote(s) => emit_side_note(&s, store, out),
        Node::TranscribedNotes(id) => write!(
            out,
            "［＃{}］",
            store.resolve_str(store.resolve_transcribed_notes(id).body)
        ),
        Node::PageBreak => out.write_str("［＃改ページ］"),
        Node::BodyEnd => out.write_str("［＃本文終わり］"),
        Node::ForcedBreak => out.write_str("［＃改行］"),
        Node::SectionBreak(kind) => emit_section_break(kind, out),
        Node::QualifiedBreak(break_) => emit_qualified_break(break_, out),
        Node::Line(lf) => emit_line(lf, out),
        Node::Illustration(id) => emit_sashie(&store.resolve_illustration(id), store, out),
        Node::HeadingHint(h) => emit_heading_hint(h, store, out),
        Node::Heading(h) => emit_aozora_heading(&h, store, out),
        // Variants not covered inline: Container is routed through the
        // open/close sentinel path; Warichu reaches this arm and is written as
        // a diagnostic placeholder.
        _ => {
            out.write_str("<!-- unsupported-aozora: ")?;
            out.write_str(node.xml_node_name())?;
            out.write_str(" -->")
        }
    }
}

fn emit_qualified_break<W: Write>(break_: QualifiedBreak, out: &mut W) -> fmt::Result {
    if let Some(number) = break_.column_number {
        return write!(out, "［＃{number}段目］");
    }
    let keyword = match break_.kind {
        QualifiedBreakKind::Page => "改ページ",
        QualifiedBreakKind::Kaicho => "改丁",
        QualifiedBreakKind::Column => "改段",
        QualifiedBreakKind::Kaimihiraki => "改見開き",
    };
    write!(out, "［＃{keyword}")?;
    if break_.destination_right {
        out.write_str("右")?;
    }
    if let Some(placement) = break_.following_placement {
        match placement {
            FollowingPlacement::HorizontalCenter => out.write_str("、ページの左右中央に")?,
            FollowingPlacement::Left => out.write_str("、左寄せで")?,
        }
    }
    if let Some(indent) = break_.following_indent {
        write!(out, "、{indent}字下げで")?;
    }
    out.write_str("］")
}

/// Emit a single [`Content`] verbatim (plain text, or each segment's
/// text / gaiji / directive).
fn emit_content_one<W: Write>(c: Content, store: &NodeStore, out: &mut W) -> fmt::Result {
    match c {
        Content::Plain(id) => out.write_str(store.resolve_str(id)),
        Content::Segments(range) => {
            for seg in store.resolve_seg_range(range) {
                emit_segment(*seg, store, out)?;
            }
            Ok(())
        }
        // `Content` is `#[non_exhaustive]`; a variant added later emits
        // nothing.
        _ => Ok(()),
    }
}

fn emit_segment<W: Write>(segment: Segment, store: &NodeStore, out: &mut W) -> fmt::Result {
    match segment {
        Segment::Text(id) => out.write_str(store.resolve_str(id)),
        Segment::Gaiji { value, .. } => emit_gaiji(&value, store, out),
        Segment::Directive { value, .. } => out.write_str(store.resolve_str(value.raw)),
        Segment::Format { value, .. } => emit_format(&value, store, out),
        Segment::Kunten { value, .. } => emit_kunten(value, store, out),
        Segment::IterationMark { value, .. } => out.write_str(value.source()),
        Segment::Illustration { value, .. } => {
            emit_sashie(&store.resolve_illustration(value), store, out)
        }
        Segment::MarginNote { value, .. } => emit_side_note(&value, store, out),
        _ => Ok(()),
    }
}

/// Emit a [`ContentRange`] run (length 1 by construction) by serializing each
/// resolved [`Content`].
fn emit_content_range<W: Write>(
    range: ContentRange,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    for c in store.resolve_content_range(range) {
        emit_content_one(*c, store, out)?;
    }
    Ok(())
}

/// Emit a single [`Content`] as plain text: a gaiji segment writes its
/// `hint`, not its glyph form.
pub(crate) fn emit_content_as_plain_one<W: Write>(
    c: Content,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    match c {
        Content::Plain(id) => out.write_str(store.resolve_str(id)),
        Content::Segments(range) => {
            for seg in store.resolve_seg_range(range) {
                match *seg {
                    Segment::Text(id) => out.write_str(store.resolve_str(id))?,
                    Segment::Gaiji { value, .. } => out.write_str(store.resolve_str(value.hint))?,
                    Segment::Directive { value, .. } => {
                        out.write_str(store.resolve_str(value.raw))?;
                    }
                    Segment::Format { value, .. } => emit_format(&value, store, out)?,
                    Segment::Kunten { value, .. } => emit_kunten(value, store, out)?,
                    Segment::IterationMark { value, .. } => out.write_str(value.source())?,
                    Segment::Illustration { value, .. } => {
                        emit_sashie(&store.resolve_illustration(value), store, out)?;
                    }
                    Segment::MarginNote { value, .. } => emit_side_note(&value, store, out)?,
                    // `Segment` is `#[non_exhaustive]`; a variant added later
                    // emits nothing.
                    _ => {}
                }
            }
            Ok(())
        }
        // `Content` is `#[non_exhaustive]`; a variant added later emits
        // nothing.
        _ => Ok(()),
    }
}

/// Emit a [`ContentRange`] run as plain text.
fn emit_content_as_plain_range<W: Write>(
    range: ContentRange,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    for c in store.resolve_content_range(range) {
        emit_content_as_plain_one(*c, store, out)?;
    }
    Ok(())
}

/// Serialize a ruby node: a left-side ruby to its
/// `base［＃「base」の左に「reading」のルビ］` form; a right-side ruby to
/// `｜base《reading》`, retaining an explicit boundary for every base.
fn emit_ruby<W: Write>(r: &Ruby, store: &NodeStore, out: &mut W) -> fmt::Result {
    if matches!(r.side, RubySide::Left) {
        emit_content_range(r.base, store, out)?;
        out.write_str("［＃「")?;
        emit_content_range(r.base, store, out)?;
        out.write_str("」の左に「")?;
        emit_content_range(r.reading, store, out)?;
        return out.write_str("」のルビ］");
    }
    if !content_range_has_detached_gaiji(r.base, store) {
        out.write_char('｜')?;
    }
    emit_content_range(r.base, store, out)?;
    out.write_char('《')?;
    emit_ruby_reading(r.reading, store, out)?;
    out.write_char('》')?;
    emit_postposed_reading_markers(r.reading, store, out)
}

fn content_range_has_detached_gaiji(range: ContentRange, store: &NodeStore) -> bool {
    store.resolve_content_range(range).iter().any(|content| {
        let Content::Segments(segments) = content else {
            return false;
        };
        store.resolve_seg_range(*segments).iter().any(|segment| {
            matches!(
                segment,
                Segment::Gaiji {
                    placement: GaijiMarkerPlacement::Detached,
                    ..
                }
            )
        })
    })
}

fn emit_ruby_reading<W: Write>(range: ContentRange, store: &NodeStore, out: &mut W) -> fmt::Result {
    for content in store.resolve_content_range(range) {
        match *content {
            Content::Segments(segments) => {
                for segment in store.resolve_seg_range(segments) {
                    match *segment {
                        Segment::Gaiji {
                            value,
                            placement: GaijiMarkerPlacement::AfterRuby,
                            ..
                        } => {
                            if let Some(replaced) = value.replaced {
                                emit_content_range(replaced, store, out)?;
                            }
                        }
                        other => emit_segment(other, store, out)?,
                    }
                }
            }
            other => emit_content_one(other, store, out)?,
        }
    }
    Ok(())
}

fn emit_postposed_reading_markers<W: Write>(
    range: ContentRange,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    for content in store.resolve_content_range(range) {
        let Content::Segments(segments) = *content else {
            continue;
        };
        for segment in store.resolve_seg_range(segments) {
            if let Segment::Gaiji {
                value,
                placement: GaijiMarkerPlacement::AfterRuby,
                ..
            } = *segment
            {
                emit_gaiji_marker(&value, store, out)?;
            }
        }
    }
    Ok(())
}

/// Serialize a forward-format node to its `［＃…］` bracket form. A `Reclaimed`
/// origin first re-emits the target literal; a bouten attribute uses the
/// `…に<keyword>` (or `の左に`) shape; a font-size attribute writes its
/// `N段階大きな/小さな文字` magnitude; every other attribute uses
/// `［＃「target」は<keyword>］`.
fn emit_format<W: Write>(f: &ForwardFormat, store: &NodeStore, out: &mut W) -> fmt::Result {
    if matches!(f.origin, ForwardOrigin::Reclaimed | ForwardOrigin::Detached) {
        emit_format_target(f, store, out)?;
    }
    // A `Detached` decoration is the styled-literal half of a
    // non-adjacent forward split: it serializes as the bare literal above,
    // because the `［＃…］` directive is a separate `Referenced` node that
    // emits the bracket itself. Return before the bracket-emitting block.
    if matches!(f.origin, ForwardOrigin::Detached) {
        return Ok(());
    }
    if let Some(body) = f.annotation_body {
        out.write_str("［＃")?;
        out.write_str(store.resolve_str(body))?;
        return out.write_char('］');
    }
    let Some(attr) = f.attrs.single() else {
        out.write_str("［＃「")?;
        emit_content_as_plain_range(f.target, store, out)?;
        out.write_str("」は")?;
        for (index, attr) in store.resolve_forward_attrs(&f.attrs).iter().enumerate() {
            if index > 0 {
                out.write_char('、')?;
            }
            out.write_str(attr.keyword())?;
        }
        return out.write_char('］');
    };
    if let ForwardAttr::Bouten { kind, position } = attr {
        out.write_str("［＃")?;
        emit_bouten_targets(store.resolve_content_range(f.target), store, out)?;
        if position == DecorationPosition::Below {
            return out.write_str("に下線］");
        }
        if position == DecorationPosition::Above {
            return out.write_str("は上線（￣）付き］");
        }
        match position {
            DecorationPosition::Left => out.write_str("の左に")?,
            DecorationPosition::Both => out.write_str("の両側に")?,
            _ => out.write_char('に')?,
        }
        out.write_str(kind.keyword())?;
        return out.write_char('］');
    }
    if matches!(attr, ForwardAttr::Framed(EnclosureKind::Box)) {
        // 「□」囲み: the keyword embeds the quoted glyph, so it can't come from
        // `keyword()`. □ (U+25A1) is the canonical spelling of the Box kind.
        out.write_str("［＃「")?;
        emit_content_as_plain_range(f.target, store, out)?;
        return out.write_str("」は「□」囲み］");
    }
    out.write_str("［＃「")?;
    emit_content_as_plain_range(f.target, store, out)?;
    out.write_str("」は")?;
    if let ForwardAttr::FontSize(shift) = attr {
        let word = if shift.larger() {
            "大きな"
        } else {
            "小さな"
        };
        write!(out, "{}段階{word}文字", shift.magnitude())?;
    } else if let ForwardAttr::AlignEnd { offset } = attr {
        // Anchor is not distinguished in the model (like LineFormat::AlignEnd), so
        // canonicalise: 0 → 地付き (the zero-lift spelling), else 文末より…字上げ揃え.
        // Both re-parse to the same offset.
        if offset == 0 {
            out.write_str("地付き")?;
        } else {
            write!(out, "文末より{offset}字上げ揃え")?;
        }
    } else if let ForwardAttr::Accent(mark) = attr {
        // アクサン / ウムラウト: the suffix carries the bracketed mark symbol, not a
        // bare keyword (so `keyword()` returns its 太字 default) — re-emit the
        // exact source suffix for a byte-exact round-trip.
        out.write_str(accent_suffix(mark))?;
    } else {
        out.write_str(attr.keyword())?;
    }
    out.write_char('］')
}

/// Reconstruct accent encoding when its scope keeps a supplied selector attached.
fn emit_format_target<W: Write>(f: &ForwardFormat, store: &NodeStore, out: &mut W) -> fmt::Result {
    if f.attrs.single() == Some(ForwardAttr::AccentDot)
        && let Some(target) = store.content_range_as_plain(f.target)
        && !target.contains(['〔', '〕'])
        && target
            .chars()
            .any(|ch| ACCENT_TABLE.iter().any(|(_, glyph)| *glyph == ch))
    {
        out.write_char('〔')?;
        for ch in target.chars() {
            if let Some((spelling, _)) = ACCENT_TABLE.iter().find(|(_, glyph)| *glyph == ch) {
                out.write_str(spelling)?;
            } else {
                out.write_char(ch)?;
            }
        }
        return out.write_char('〕');
    }
    emit_content_as_plain_range(f.target, store, out)
}

/// The exact `は`-suffix source for a forward accent [`AccentMark`], for a
/// byte-exact round-trip: fullwidth parens (U+FF08 / U+FF09) wrapping the mark
/// symbol (´ U+00B4 / ｀ U+FF40 / ¨ U+00A8).
const fn accent_suffix(mark: AccentMark) -> &'static str {
    match mark {
        AccentMark::Acute => "アクサン（´）付き",
        AccentMark::Grave => "アクサン（｀）付き",
        AccentMark::Umlaut => "ウムラウト（¨）付き",
    }
}

/// Serialize the bouten target(s) as quoted `「…」` runs. A single `Plain`
/// target becomes one `「text」`; a segmented target is split on `、` into one
/// `「part」` per piece (falling back to an empty `「」`). Operates on the
/// resolved target run (always length 1).
fn emit_bouten_targets<W: Write>(run: &[Content], store: &NodeStore, out: &mut W) -> fmt::Result {
    if let [Content::Plain(id)] = run {
        out.write_char('「')?;
        out.write_str(store.resolve_str(*id))?;
        return out.write_char('」');
    }
    let mut any = false;
    for c in run {
        if let Content::Segments(seg_range) = c {
            for seg in store.resolve_seg_range(*seg_range) {
                if let Segment::Text(id) = *seg {
                    let t = store.resolve_str(id);
                    for part in t.split('、').filter(|p| !p.is_empty()) {
                        out.write_char('「')?;
                        out.write_str(part)?;
                        out.write_char('」')?;
                        any = true;
                    }
                }
            }
        }
    }
    if !any {
        out.write_char('「')?;
        out.write_char('」')?;
    }
    Ok(())
}

/// Serialize a gaiji node to its `［＃「hint」…］` bracket form (prefixed with
/// `※` unless `standalone`), appending the mencode tail when the canonical
/// form carries one.
fn emit_gaiji<W: Write>(g: &Gaiji, store: &NodeStore, out: &mut W) -> fmt::Result {
    if let Some(replaced) = g.replaced {
        emit_content_range(replaced, store, out)?;
    }
    if g.declaration_span.is_some() {
        return Ok(());
    }
    emit_gaiji_marker(g, store, out)
}

fn emit_gaiji_marker<W: Write>(g: &Gaiji, store: &NodeStore, out: &mut W) -> fmt::Result {
    if !g.standalone {
        out.write_char('※')?;
    }
    out.write_str("［＃")?;
    let hint = store.resolve_str(g.hint);
    if g.quoted {
        out.write_char('「')?;
        out.write_str(hint)?;
        out.write_char('」')?;
    } else {
        out.write_str(hint)?;
    }
    if gaiji_has_mencode(g.canonical) {
        out.write_char('、')?;
        write_gaiji_mencode(g.canonical, store, out)?;
    }
    out.write_char('］')
}

/// `GaijiCanonical::has_mencode` for the owned payload.
const fn gaiji_has_mencode(c: GaijiCanonicalOwned) -> bool {
    !matches!(c, GaijiCanonicalOwned::Unresolved { mencode: None })
}

/// `GaijiCanonical::write_mencode` for the owned payload; resolves the
/// `Unresolved` tail's [`StrId`](ab_aozora_syntax::ast::StrId) against `store`.
fn write_gaiji_mencode<W: Write>(
    c: GaijiCanonicalOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    match c {
        GaijiCanonicalOwned::MenKuTen(m) => write!(out, "{m}"),
        GaijiCanonicalOwned::Unicode(ch) => write!(out, "U+{:04X}", ch as u32),
        GaijiCanonicalOwned::Unresolved { mencode } => {
            mencode.map_or(Ok(()), |id| out.write_str(store.resolve_str(id)))
        }
    }
}

/// Serialize a kaeriten mark to its `［＃<mark>］` bracket form.
fn emit_kunten<W: Write>(k: Kunten, store: &NodeStore, out: &mut W) -> fmt::Result {
    out.write_str("［＃")?;
    if k.kind == KuntenKind::Okurigana {
        out.write_char('（')?;
    }
    out.write_str(store.resolve_str(k.text))?;
    if k.kind == KuntenKind::Okurigana {
        out.write_char('）')?;
    }
    out.write_char('］')
}

/// Serialize a directive by writing its `raw` bytes verbatim (they already
/// include the `［＃…］` brackets).
///
/// When `directives` is not `Off`, an `Unknown` directive whose trimmed body is
/// a verified near-miss is rewritten to `［＃<canonical>］` instead. Tier1
/// ([`canonical_directive`]) is tried first; at the `Degraded` level a Tier1
/// miss then tries the lossy / judgment Tier2 reductions ([`degraded_directive`]).
/// The rewrite is idempotent: the resolved body parses to a recognized
/// (non-`Unknown`) node, so a second pass never re-enters this branch. Only
/// the temporary `Degraded` render buffer reaches Tier2, and no
/// persistent-write path does, so a Tier2 reduction does not rewrite source.
fn emit_annotation<W: Write>(
    a: Directive,
    store: &NodeStore,
    out: &mut W,
    directives: DirectiveNormalization,
) -> fmt::Result {
    let raw = store.resolve_str(a.raw);
    if directives != DirectiveNormalization::Off && a.kind == DirectiveKind::Unknown {
        let body = raw
            .strip_prefix("［＃")
            .and_then(|s| s.strip_suffix('］'))
            .unwrap_or(raw)
            .trim();
        let resolved = canonical_directive(body).or_else(|| {
            (directives == DirectiveNormalization::Degraded)
                .then(|| degraded_directive(body))
                .flatten()
        });
        if let Some(canonical) = resolved {
            out.write_str("［＃")?;
            out.write_str(canonical.as_ref())?;
            return out.write_char('］');
        }
    }
    out.write_str(raw)
}

/// Serialize an angle-quote to its `≪…≫` form.
fn emit_angle_quote<W: Write>(d: AngleQuote, store: &NodeStore, out: &mut W) -> fmt::Result {
    out.write_char('≪')?;
    emit_content_range(d.content, store, out)?;
    out.write_char('≫')
}

/// Serialize a margin note to its `base［＃「base」…］` form, using the kind's
/// connector / suffix affixes around the note text.
fn emit_side_note<W: Write>(s: &MarginNote, store: &NodeStore, out: &mut W) -> fmt::Result {
    if let Some(pair) = s.pair {
        return out.write_str(store.resolve_str(store.resolve_margin_note_pair(pair).close_raw));
    }
    if s.origin != ForwardOrigin::Referenced {
        emit_content_range(s.base, store, out)?;
    }
    out.write_str("［＃「")?;
    if s.anchor == Some(ab_aozora_syntax::MarkAnchor::Between) {
        // The base holds the two runs the source quoted, in that order, so the
        // marker round-trips as the pair it was written as.
        let [left, right] = store.resolve_content_range(s.base) else {
            return Err(fmt::Error);
        };
        let (left, right) = (*left, *right);
        emit_content_one(left, store, out)?;
        out.write_str("」と「")?;
        emit_content_one(right, store, out)?;
        out.write_str("」の間に")?;
        emit_content_range(s.note, store, out)?;
        return out.write_char('］');
    }
    emit_content_range(s.base, store, out)?;
    if s.kind == ab_aozora_syntax::MarginNoteKind::CriticalNote
        && s.position == Some(ab_aozora_syntax::MarginNotePosition::Margin)
    {
        out.write_str("」に欄外に校注、「")?;
        emit_content_range(s.note, store, out)?;
        return out.write_str("」］");
    }
    if matches!(
        s.kind,
        ab_aozora_syntax::MarginNoteKind::CrossReference
            | ab_aozora_syntax::MarginNoteKind::AnnotationNumber
            | ab_aozora_syntax::MarginNoteKind::AuthorNote
            | ab_aozora_syntax::MarginNoteKind::GlyphShape
    ) {
        out.write_str("」は")?;
        emit_content_range(s.note, store, out)?;
        return out.write_char('］');
    }
    out.write_str(match s.position {
        Some(ab_aozora_syntax::MarginNotePosition::Left) => "」の左に「",
        Some(ab_aozora_syntax::MarginNotePosition::Right) => "」の右に「",
        Some(ab_aozora_syntax::MarginNotePosition::Margin) => "」の欄外に「",
        None => "」に「",
    })?;
    emit_content_range(s.note, store, out)?;
    out.write_char('」')?;
    if s.kind != ab_aozora_syntax::MarginNoteKind::SuppliedMark {
        out.write_char('の')?;
        out.write_str(s.kind.keyword())?;
    }
    out.write_char('］')
}

/// Serialize an illustration to its `［＃…（file）…入る］` bracket form (a
/// description, or `挿絵` + optional number; optional dimensions and caption).
fn emit_sashie<W: Write>(s: &Illustration, store: &NodeStore, out: &mut W) -> fmt::Result {
    out.write_str("［＃")?;
    if let Some(body) = s.annotation_body {
        out.write_str(store.resolve_str(body))?;
        return out.write_char('］');
    }
    if let Some(description) = s.description {
        out.write_str(store.resolve_str(description))?;
    } else {
        out.write_str("挿絵")?;
        if let Some(number) = s.number {
            out.write_str(store.resolve_str(number))?;
        }
    }
    out.write_char('（')?;
    out.write_str(store.resolve_str(s.file))?;
    if let Some(dims) = s.dimensions {
        out.write_char('、')?;
        out.write_str(store.resolve_str(dims))?;
    }
    out.write_char('）')?;
    if let Some(caption) = s.caption {
        out.write_char('「')?;
        emit_content_as_plain_one(caption, store, out)?;
        out.write_char('」')?;
    }
    out.write_str("入る］")
}

/// Serialize a heading hint back to its `［＃「X」は…見出し］` bracket form.
fn emit_heading_hint<W: Write>(h: HeadingHint, store: &NodeStore, out: &mut W) -> fmt::Result {
    out.write_str("［＃「")?;
    out.write_str(store.resolve_str(h.target))?;
    out.write_str("」は")?;
    out.write_str(heading_style_keyword(h.style))?;
    out.write_str(heading_level_word(h.level))?;
    out.write_str("］")
}

/// Serialize an Aozora heading to its referent line followed by the
/// `［＃「text」は<style><level>見出し］` bracket.
fn emit_aozora_heading<W: Write>(h: &Heading, store: &NodeStore, out: &mut W) -> fmt::Result {
    emit_content_range(h.text, store, out)?;
    out.write_str("\n［＃「")?;
    emit_content_range(h.text, store, out)?;
    out.write_str("」は")?;
    out.write_str(heading_style_keyword(h.style))?;
    out.write_str(heading_level_word(h.kind))?;
    out.write_str("］")
}

#[cfg(test)]
mod tests {
    use crate::serialize::serialize;

    /// `serialize ∘ parse` reaches a fixed point after one pass — the
    /// canonical round-trip contract (the conformance golden tests end-to-end
    /// byte identity).
    fn assert_parity(src: &str) {
        let first = serialize(&ab_aozora_pipeline::lex(src));
        let second = serialize(&ab_aozora_pipeline::lex(&first));
        assert_eq!(first, second, "serialize fixed point diverged for {src:?}");
    }

    #[test]
    fn annotation_side_is_preserved_without_inventing_a_side_for_bare_ni() {
        for source in [
            "刺［＃「刺」に「テフダ」の注記］",
            "刺［＃「刺」の左に「テフダ」の注記］",
            "刺［＃「刺」の右に「テフダ」の注記］",
            "刺［＃「刺」に「×」の傍記］",
            "土を堀り［＃「堀」に「ママ」の注記］かへす",
        ] {
            assert_eq!(serialize(&ab_aozora_pipeline::lex(source)), source);
        }
    }

    #[test]
    fn mixed_formatting_keeps_its_edition_assertion_in_source_serialization() {
        for source in [
            "１）［＃「１）」は縦中横、行右小書き、底本では欠落］",
            "１）［＃「１）」は縦中横、行右小書き、「１」が底本では欠落］",
            "２）［＃「２）」は縦中横、行右小書き、底本では「１）」］",
        ] {
            assert_eq!(serialize(&ab_aozora_pipeline::lex(source)), source);
            assert_parity(source);
        }
    }

    #[test]
    fn serialize_is_fixed_point_across_node_kinds() {
        for src in [
            "plain text",
            "｜青梅《おうめ》",
            "頃｜青梅《おうめ》",
            "再読［＃「再読」の左に「さい」のルビ］",
            "底本「青空」［＃「青空」の左に「注記」の注記］",
            "可哀想［＃「可哀想」に傍点］",
            "甲乙［＃「甲」「乙」に傍点］",
            "重要［＃「重要」は太字］",
            "X［＃「X」は3段階大きな文字］",
            "※［＃「○○」、第3水準1-85-54］",
            "一二［＃レ］",
            "≪重要≫",
            "見出し\n［＃「見出し」は大見出し］",
            "［＃挿絵（fig.png、横480×縦640）入る］",
            "［＃改ページ］",
            "本編［＃本文終わり］",
            "行頭［＃改行］行末",
            "［＃ここから2字下げ］\nA\n［＃ここで字下げ終わり］",
            "段落の文\n――――――――――――\n｜青梅《おうめ》の続き\n",
        ] {
            assert_parity(src);
        }
    }

    /// Reference serialization retains the marker without inserting its operand.
    #[test]
    fn referenced_forward_serializes_bracket_only() {
        use ab_aozora_syntax::alloc::Allocator;
        use ab_aozora_syntax::ast::Node;
        use ab_aozora_syntax::{ForwardAttr, ForwardOrigin};

        let mut a = Allocator::new();
        let t = a.content_plain("X");
        let node = a.forward_format(ForwardAttr::Bold, t, ForwardOrigin::Referenced);
        let Node::Format(f) = node else {
            panic!("forward_format must build a Format node");
        };
        let store = a.into_store();

        let mut s = String::new();
        super::emit_format(&f, &store, &mut s).expect("serialize into String is infallible");
        assert_eq!(s, "［＃「X」は太字］");
    }

    #[test]
    fn heading_hint_serializes_bracket_only() {
        use ab_aozora_syntax::alloc::Allocator;
        use ab_aozora_syntax::ast::Node;
        use ab_aozora_syntax::{HeadingKind, HeadingStyle};

        let mut a = Allocator::new();
        let node = a.heading_hint(HeadingKind::Medium, HeadingStyle::Standard, "序章");
        let Node::HeadingHint(h) = node else {
            panic!("heading_hint must build a HeadingHint node");
        };
        let store = a.into_store();

        let mut s = String::new();
        super::emit_heading_hint(h, &store, &mut s).expect("serialize into String is infallible");
        assert_eq!(s, "［＃「序章」は中見出し］");
    }
}
