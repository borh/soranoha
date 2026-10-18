//! HTML rendering for individual AST nodes.
//!
//! Emits per-node HTML by reading an owned [`Node`] and resolving every
//! [`StrId`](ab_aozora_syntax::ast::StrId) /
//! [`ContentRange`] /
//! [`SegRange`](ab_aozora_syntax::ast::SegRange) against a [`NodeStore`].
//!
//! This module contains only the emitters that read AST payloads. The five
//! pure-scalar leaf variants ([`Node::Line`] / [`Node::PageBreak`] /
//! [`Node::BodyEnd`] / [`Node::ForcedBreak`] / [`Node::SectionBreak`]) emit
//! their fixed markup directly (e.g. the section-break slug table). The
//! heading tag writers, line renderer, and text escaper come from
//! [`crate::spelling::html`].

use core::fmt::{self, Write};

use ab_aozora_syntax::GaijiCanonical;
use ab_aozora_syntax::accent::{compose_accent, compose_accent_dots};
use ab_aozora_syntax::ast::{
    AngleQuote, Content, ContentRange, Directive, ForwardAttrs, ForwardFormat, Gaiji,
    GaijiCanonicalOwned, Heading, HeadingHint, Illustration, Kunten, KuntenKind, MarginNote, Node,
    NodeStore, Ruby, Segment, StrId,
};
use ab_aozora_syntax::format::ForwardOrigin;
use ab_aozora_syntax::{AccentMark, DirectiveKind, EnclosureKind, ForwardAttr, RubySide};

use crate::classes;
use crate::serialize::emit_content_as_plain_one;
use crate::spelling::html::{escape_text, render_line, write_heading_close, write_heading_open};

/// Render a single owned [`Node`] into `writer`.
///
/// Every inline / leaf node emits its markup unconditionally: there is no
/// `entering` flag, since `RenderState` handles containers and they are not
/// passed to this function. `store` resolves the node's interned payloads.
///
/// # Errors
///
/// Propagates formatter write errors.
pub(crate) fn render<W: Write>(node: Node, store: &NodeStore, out: &mut W) -> fmt::Result {
    match node {
        Node::Ruby(r) => render_ruby(&r, store, out),
        Node::Format(f) => render_format(&f, store, out),
        Node::MarginNote(s) => render_side_note(&s, store, out),
        // The source-layout renderer already displays the separately transcribed note lines.
        Node::TranscribedNotes(_) => Ok(()),
        Node::Gaiji(g) => render_gaiji(&g, store, out),
        // Pure-scalar leaves render through the lifetime-free helpers and
        // inline byte spellings; the section-break slug table is keyed by the
        // canonical keyword.
        Node::Line(lf) => render_line(lf, out),
        Node::PageBreak => out.write_str(r#"<div class="aozora-page-break"></div>"#),
        Node::BodyEnd => out.write_str(r#"<div class="aozora-body-end"></div>"#),
        Node::ForcedBreak => out.write_str("<br />"),
        Node::SectionBreak(k) => {
            let slug = ab_aozora_spec::roman_slug(k.keyword()).unwrap_or("other");
            write!(
                out,
                r#"<div class="aozora-section-break aozora-section-break-{slug}"></div>"#,
            )
        }
        Node::QualifiedBreak(break_) => {
            let kind = match break_.kind {
                ab_aozora_syntax::QualifiedBreakKind::Page => "page",
                ab_aozora_syntax::QualifiedBreakKind::Kaicho => "kaicho",
                ab_aozora_syntax::QualifiedBreakKind::Column => "column",
                ab_aozora_syntax::QualifiedBreakKind::Kaimihiraki => "kaimihiraki",
            };
            write!(
                out,
                r#"<div class="aozora-layout-break aozora-layout-break-{kind}""#
            )?;
            if break_.destination_right {
                out.write_str(r#" data-destination-side="right""#)?;
            }
            if let Some(placement) = break_.following_placement {
                let placement = match placement {
                    ab_aozora_syntax::FollowingPlacement::HorizontalCenter => "horizontal-center",
                    ab_aozora_syntax::FollowingPlacement::Left => "left",
                };
                write!(out, r#" data-following-placement="{placement}""#)?;
            }
            if let Some(indent) = break_.following_indent {
                write!(out, r#" data-following-indent="{indent}""#)?;
            }
            if let Some(number) = break_.column_number {
                write!(out, r#" data-column-number="{number}""#)?;
            }
            out.write_str("></div>")
        }
        Node::Directive(a) => render_annotation(a, store, out),
        Node::Kunten(k) => render_kunten(k, store, out),
        Node::IterationMark(mark) => out.write_char(mark.character()),
        Node::AngleQuote(d) => render_angle_quote(d, store, out),
        Node::Illustration(id) => render_sashie(&store.resolve_illustration(id), store, out),
        Node::Heading(h) => render_aozora_heading(&h, store, out),
        Node::HeadingHint(h) => render_heading_hint(h, store, out),
        // Other variants (`Warichu`, `Container`, future non-exhaustive
        // additions) — emit a fallback `<!-- name -->` comment so the rendered
        // HTML identifies the unrendered node; the node's `xml_node_name`
        // supplies the name.
        _ => write!(out, "<!-- {} -->", node.xml_node_name()),
    }
}

/// Render a [`ContentRange`] run (length 1 by construction) by emitting the
/// HTML of each resolved [`Content`].
fn render_content_range<W: Write>(
    range: ContentRange,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    for c in store.resolve_content_range(range) {
        render_content_one(*c, store, out)?;
    }
    Ok(())
}

/// Render a single [`Content`]. A `Plain` run escapes to text; a
/// `Segments` run walks its segments, escaping text and nesting gaiji +
/// directive markup.
fn render_content_one<W: Write>(c: Content, store: &NodeStore, out: &mut W) -> fmt::Result {
    match c {
        Content::Plain(id) => escape_text(store.resolve_str(id), out),
        Content::Segments(range) => {
            for seg in store.resolve_seg_range(range) {
                match *seg {
                    Segment::Text(id) => escape_text(store.resolve_str(id), out)?,
                    Segment::Gaiji { value, .. } => render_gaiji(&value, store, out)?,
                    Segment::Directive { value, .. } => render_annotation(value, store, out)?,
                    Segment::Format { value, .. } => render_format(&value, store, out)?,
                    Segment::Kunten { value, .. } => render_kunten(value, store, out)?,
                    Segment::IterationMark { value, .. } => out.write_char(value.character())?,
                    Segment::Illustration { value, .. } => {
                        render_sashie(&store.resolve_illustration(value), store, out)?;
                    }
                    Segment::MarginNote { value, .. } => render_side_note(&value, store, out)?,
                    // `Segment` is `#[non_exhaustive]`; a variant added later
                    // renders nothing.
                    _ => {}
                }
            }
            Ok(())
        }
        // `Content` is `#[non_exhaustive]`; a variant added later renders
        // nothing.
        _ => Ok(()),
    }
}

/// Render a ruby node to a `<ruby>` element (a left-side ruby classes its
/// `<rt>` for below-the-line placement).
///
/// When `base_emphasis` is set — a declined forward directive
/// `［＃「X」に傍点/罫囲み/…］` selected this ruby's base as its unique referent —
/// the base is wrapped in that attribute's emphasis element inside the `<ruby>`,
/// before the `<rt>`, so the emphasis marks the base glyphs and not the reading.
/// `render_styled_content` decorates the owned base directly.
fn render_ruby<W: Write>(r: &Ruby, store: &NodeStore, out: &mut W) -> fmt::Result {
    out.write_str("<ruby>")?;
    match r.base_emphasis {
        Some(attr) => render_styled_content(&ForwardAttrs::One(attr), r.base, None, store, out)?,
        None => render_content_range(r.base, store, out)?,
    }
    // A left-side ruby (saidoku building block) marks its `<rt>` with a class
    // so a stylesheet can place the reading below; the right-side `<rt>`
    // has no class.
    out.write_str(match r.side {
        RubySide::Left => r#"<rp>(</rp><rt class="aozora-ruby-left">"#,
        _ => "<rp>(</rp><rt>",
    })?;
    render_content_range(r.reading, store, out)?;
    out.write_str("</rt><rp>)</rp></ruby>")
}

/// Render source-role, shape and supplied-mark statements as operand metadata, and glosses as ruby annotations.
/// Referenced targets are already present in the body and are not repeated.
fn render_side_note<W: Write>(s: &MarginNote, store: &NodeStore, out: &mut W) -> fmt::Result {
    if let Some((class, attribute)) = match s.kind {
        ab_aozora_syntax::MarginNoteKind::GlyphShape => {
            Some(("aozora-glyph-shape", "data-shape-assertion"))
        }
        ab_aozora_syntax::MarginNoteKind::SuppliedMark => {
            Some(("aozora-supplied-mark", "data-supplied-mark"))
        }
        _ => None,
    } {
        write!(out, "<span class=\"{class}\" {attribute}=\"")?;
        let mut statement = String::new();
        for content in store.resolve_content_range(s.note) {
            emit_content_as_plain_one(*content, store, &mut statement)?;
        }
        escape_text(&statement, out)?;
        if let Some(position) = s.position {
            out.write_str(match position {
                ab_aozora_syntax::MarginNotePosition::Left => "\" data-place=\"left",
                ab_aozora_syntax::MarginNotePosition::Right => "\" data-place=\"right",
                ab_aozora_syntax::MarginNotePosition::Margin => "\" data-place=\"margin",
            })?;
        }
        out.write_str("\">")?;
        if s.origin != ForwardOrigin::Referenced {
            render_content_range(s.base, store, out)?;
        }
        return out.write_str("</span>");
    }
    if let Some(role) = match s.kind {
        ab_aozora_syntax::MarginNoteKind::AnnotationNumber => Some("annotation-number"),
        ab_aozora_syntax::MarginNoteKind::AuthorNote => Some("author-note"),
        _ => None,
    } {
        if s.origin != ForwardOrigin::Referenced {
            write!(
                out,
                "<span class=\"aozora-source-role\" data-source-role=\"{role}\">"
            )?;
            render_content_range(s.base, store, out)?;
            out.write_str("</span>")?;
        }
        return Ok(());
    }

    if s.origin == ForwardOrigin::Referenced {
        out.write_str("<span class=\"aozora-margin-note\">")?;
        render_content_range(s.note, store, out)?;
        return out.write_str("</span>");
    }
    out.write_str("<ruby>")?;
    render_content_range(s.base, store, out)?;
    out.write_str(r#"<rp>(</rp><rt class="aozora-margin-note">"#)?;
    render_content_range(s.note, store, out)?;
    out.write_str("</rt><rp>)</rp></ruby>")
}

/// Render a forward-reference emphasis (bouten / combine-upright / font-size /
/// italic / span / bold) to its HTML element.
///
/// A reference operand owns no principal text and emits nothing. Reclaimed
/// and detached nodes render the source text already transferred into them.
fn render_format<W: Write>(f: &ForwardFormat, store: &NodeStore, out: &mut W) -> fmt::Result {
    if matches!(f.origin, ForwardOrigin::Referenced) {
        return Ok(());
    }
    render_styled_content(&f.attrs, f.target, f.annotation_body, store, out)
}

fn render_styled_content<W: Write>(
    attrs: &ForwardAttrs,
    target: ContentRange,
    annotation_body: Option<StrId>,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    let Some(attr) = attrs.single() else {
        out.write_str("<span class=\"")?;
        for (index, attr) in store.resolve_forward_attrs(attrs).iter().enumerate() {
            if index > 0 {
                out.write_char(' ')?;
            }
            let class = match attr {
                ForwardAttr::CombineUpright => "aozora-combine-upright",
                ForwardAttr::SmallScript(_) => {
                    write!(
                        out,
                        "aozora-{}",
                        ab_aozora_spec::roman_slug(attr.keyword())
                            .expect("small script has a spec slug")
                    )?;
                    continue;
                }
                _ => "aozora-unsupported-format",
            };
            out.write_str(class)?;
        }
        out.write_str("\">")?;
        render_content_range(target, store, out)?;
        return out.write_str("</span>");
    };
    match attr {
        ForwardAttr::Bouten { kind, position } => {
            write!(
                out,
                r#"<em class="aozora-bouten aozora-bouten-{kind} aozora-bouten-{pos}">"#,
                kind = classes::bouten_kind_slug(kind),
                pos = classes::bouten_position_slug(position),
            )?;
            render_content_range(target, store, out)?;
            out.write_str("</em>")
        }
        ForwardAttr::CombineUpright => {
            out.write_str(r#"<span class="aozora-combine-upright">"#)?;
            render_content_range(target, store, out)?;
            out.write_str("</span>")
        }
        // 文字サイズ carries a magnitude, so its open tag is dynamic.
        ForwardAttr::FontSize(shift) => {
            let class = if shift.larger() {
                "aozora-font-larger"
            } else {
                "aozora-font-smaller"
            };
            write!(
                out,
                r#"<span class="{class}" data-steps="{}">"#,
                shift.magnitude()
            )?;
            render_content_range(target, store, out)?;
            out.write_str("</span>")
        }
        // 分数: split the target on a slash — ASCII `/` or fullwidth `／` (the
        // corpus uses both) — into a `<sup>`/`<sub>` fraction joined by the
        // fraction slash U+2044. The target is plain math text, so it is
        // read through `content_range_as_plain`.
        ForwardAttr::Fraction => {
            let slug = ab_aozora_spec::roman_slug("分数").unwrap_or("bunsu");
            write!(out, r#"<span class="aozora-{slug}">"#)?;
            match store.content_range_as_plain(target) {
                Some(t) => match t.split_once(['/', '／']) {
                    Some((num, den)) => {
                        out.write_str("<sup>")?;
                        escape_text(num, out)?;
                        out.write_str("</sup>⁄<sub>")?;
                        escape_text(den, out)?;
                        out.write_str("</sub>")?;
                    }
                    // No slash (not attested in the corpus): emit the target
                    // verbatim, without a numerator / denominator.
                    None => escape_text(t, out)?,
                },
                // A structured (non-plain) target can't be split; render it
                // as-is so no content is dropped.
                None => render_content_range(target, store, out)?,
            }
            out.write_str("</span>")
        }
        // Enclosures drawn as a CSS-styled span — the glyph / keyword
        // identifies the kind (serialize-only) and the stylesheet draws the
        // frame, one class per kind: 「□」囲み / ○付き文字 / 点線丸囲み / 二重罫囲み. 罫囲み has no
        // dedicated span class (`None`) and keeps the slug-keyed
        // `aozora-keigakomi-inline` via the semantic fall-through.
        ForwardAttr::Framed(kind) => match framed_span_class(kind) {
            Some(class) => {
                write!(out, r#"<span class="{class}">"#)?;
                render_content_range(target, store, out)?;
                out.write_str("</span>")
            }
            None => render_forward_semantic(target, attr, store, out),
        },
        // ドット付き: compose the addressed letters of the reclaimed run
        // into their precomposed dotted glyphs (ṁ / ṣ) — see `render_accent_dot`.
        ForwardAttr::AccentDot => render_accent_dot(target, annotation_body, store, out),
        // アクサン / ウムラウト: compose the single target letter with its accent
        // mark into the precomposed glyph (é / ö) — see `render_accent`. The
        // separate arm keeps it out of the bold catch-all below; falling
        // through to that catch-all is a known defect class.
        ForwardAttr::Accent(mark) => render_accent(target, mark, store, out),
        // 文末より N字上げ揃え: end-align the run. Uses the line form's
        // `aozora-align-end` class / `data-offset`, so the two scopes have
        // identical styling; without this arm the run would fall through to
        // the bold default below.
        ForwardAttr::AlignEnd { offset } => {
            write!(
                out,
                r#"<span class="aozora-align-end" data-offset="{offset}">"#
            )?;
            render_content_range(target, store, out)?;
            out.write_str("</span>")
        }
        // The HTML element is semantic; the `aozora-*` slug comes from the
        // spec slug table, keyed by the canonical keyword.
        attr => render_forward_semantic(target, attr, store, out),
    }
}

/// The dedicated `aozora-*` span class for an enclosure that the stylesheet
/// draws around its target, or `None` for [`EnclosureKind::Rule`], which keeps
/// the slug-keyed `aozora-keigakomi-inline` semantic rendering. The match is
/// exhaustive, so the compiler reports a new enclosure kind here instead of
/// assigning it the ruled-frame class.
pub(crate) const fn framed_span_class(kind: EnclosureKind) -> Option<&'static str> {
    match kind {
        EnclosureKind::Rule => None,
        EnclosureKind::Unspecified => Some("aozora-keigakomi-inline"),
        EnclosureKind::DashedRule => Some("aozora-enclosure-dashed-rule"),
        EnclosureKind::Box => Some("aozora-keigakomi-box"),
        EnclosureKind::Circle => Some("aozora-enclosure-circle"),
        EnclosureKind::CircleDotted => Some("aozora-enclosure-circle-dotted"),
        EnclosureKind::DoubleRule => Some("aozora-enclosure-double-rule"),
    }
}

/// Render a forward attribute that maps to a plain semantic element keyed by
/// its canonical-keyword slug (太字 / 斜体 / 上下付き / 小書き / 絶対サイズ / …).
/// The parameterized and special-case attributes (bouten / font-size / fraction /
/// box / accent-dot / align-end) are handled by their own arms in
/// [`render_format`]; this is the catch-all for the simple styled runs.
fn render_forward_semantic<W: Write>(
    target: ContentRange,
    attr: ForwardAttr,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    let (el, close) = match attr {
        ForwardAttr::Italic => ("i", "</i>"),
        ForwardAttr::SuperScript => ("sup", "</sup>"),
        ForwardAttr::SubScript => ("sub", "</sub>"),
        ForwardAttr::Lowered
        | ForwardAttr::Raised { .. }
        | ForwardAttr::Exponent
        | ForwardAttr::SmallScript(_)
        | ForwardAttr::Framed(_)
        | ForwardAttr::Horizontal
        | ForwardAttr::Vertical
        | ForwardAttr::Caption
        | ForwardAttr::QualitativeFontSize(_)
        | ForwardAttr::FontSizeAbsolute(_) => ("span", "</span>"),
        // Bold and any future weight default to the bold element.
        _ => ("b", "</b>"),
    };
    let slug = match attr {
        ForwardAttr::Lowered => "lowered",
        ForwardAttr::Raised { .. } => "raised",
        ForwardAttr::Exponent => "exponent",
        _ => ab_aozora_spec::roman_slug(attr.keyword()).unwrap_or("futoji"),
    };
    write!(out, r#"<{el} class="aozora-{slug}">"#)?;
    render_content_range(target, store, out)?;
    out.write_str(close)
}

/// Render a dotted-letter forward: compose the addressed letters of the
/// reclaimed run into their precomposed glyphs inside an `aozora-accent-dot`
/// span. The interned `annotation_body` holds the selector; the composer, which
/// the classifier also uses for validation, produces the visible run. The class
/// is a literal, not derived from the `slugs.rs` / Hepburn table; a body-less or
/// structured target renders the run verbatim.
fn render_accent_dot<W: Write>(
    target: ContentRange,
    annotation_body: Option<StrId>,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    out.write_str(r#"<span class="aozora-accent-dot">"#)?;
    match (store.content_range_as_plain(target), annotation_body) {
        (Some(run), Some(body_id)) => match compose_accent_dots(run, store.resolve_str(body_id)) {
            Some(composed) => escape_text(&composed, out)?,
            // Unreachable after classification; render the run so no content
            // is dropped.
            None => escape_text(run, out)?,
        },
        // A structured / body-less target can't be composed; emit as-is.
        _ => render_content_range(target, store, out)?,
    }
    out.write_str("</span>")
}

/// Render an accent-mark forward reference: compose the single target letter
/// with its accent `mark` into the precomposed glyph (é / ö) inside an
/// `aozora-accent` span. The composer, which the classifier also uses for
/// validation, defines the composition; a structured or non-composable target,
/// unreachable after classification, renders verbatim so no content is dropped.
fn render_accent<W: Write>(
    target: ContentRange,
    mark: AccentMark,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    out.write_str(r#"<span class="aozora-accent">"#)?;
    match store.content_range_as_plain(target) {
        Some(run) => match run.chars().next().and_then(|c| compose_accent(c, mark)) {
            Some(glyph) => out.write_char(glyph)?,
            // Unreachable after classification (validated single composable
            // letter); emit the run so no content is dropped.
            None => escape_text(run, out)?,
        },
        // A structured target can't be composed; emit as-is.
        None => render_content_range(target, store, out)?,
    }
    out.write_str("</span>")
}

/// Render a gaiji node to a `<span class="aozora-gaiji">`.
///
/// Reconstructs an [`ab_aozora_syntax::GaijiCanonical`] (the `Unresolved` tail's
/// [`StrId`](ab_aozora_syntax::ast::StrId) resolves against `store`) and calls
/// its `resolve`, which performs the JIS-table lookup. A resolved gaiji emits
/// its `data-codepoint` + glyph; an unresolved one emits its escaped `hint` as
/// `data-description` + body. `standalone` is ignored in HTML (serialize-only).
fn render_gaiji<W: Write>(g: &Gaiji, store: &NodeStore, out: &mut W) -> fmt::Result {
    let hint = store.resolve_str(g.hint);
    let canonical = match g.canonical {
        GaijiCanonicalOwned::MenKuTen(m) => GaijiCanonical::MenKuTen(m),
        GaijiCanonicalOwned::Unicode(c) => GaijiCanonical::Unicode(c),
        GaijiCanonicalOwned::Unresolved { mencode } => GaijiCanonical::Unresolved {
            mencode: mencode.map(|id| store.resolve_str(id)),
        },
    };
    if let Some(resolved) = canonical.resolve(hint) {
        out.write_str(r#"<span class="aozora-gaiji" data-codepoint=""#)?;
        // Write `Resolved` into a small `String` buffer to iterate its scalars
        // without matching the Char/Multi variants here.
        let mut buf = String::with_capacity(8);
        resolved
            .write_to(&mut buf)
            .expect("Resolved::write_to into String never fails");
        let mut first = true;
        for c in buf.chars() {
            if !first {
                out.write_char(' ')?;
            }
            first = false;
            write!(out, "U+{:04X}", c as u32)?;
        }
        out.write_str(r#"">"#)?;
        resolved.write_to(out)?;
    } else {
        out.write_str(r#"<span class="aozora-gaiji" data-description=""#)?;
        escape_text(hint, out)?;
        out.write_str(r#"">"#)?;
        if let Some(replaced) = g.replaced {
            render_content_range(replaced, store, out)?;
        } else {
            escape_text(hint, out)?;
        }
    }
    out.write_str("</span>")
}

/// Render a directive: warichu open/close to `<span class="aozora-warichu">` /
/// `</span>`, an editor note to a visible `注N` superscript, and any other
/// directive to a hidden `<span class="aozora-directive">` of its raw text.
fn render_annotation<W: Write>(a: Directive, store: &NodeStore, out: &mut W) -> fmt::Result {
    match a.kind {
        DirectiveKind::GaijiDeclaration | DirectiveKind::MarginNotePairOpenResolved => {
            return Ok(());
        }
        // `RenderState` balances top-level inline warichu tags; these arms run
        // only for a warichu directive nested in `Segment::Directive`, which
        // does not occur.
        DirectiveKind::WarichuOpen => return out.write_str(r#"<span class="aozora-warichu">"#),
        DirectiveKind::WarichuClose => return out.write_str("</span>"),
        DirectiveKind::EditorNote => {
            // ［＃入力者注(N)］ → a visible 注N superscript. `a.raw` is the whole
            // bracketed directive; recover N (the classifier guaranteed the shape).
            let raw = store.resolve_str(a.raw);
            let n = raw
                .strip_prefix("［＃入力者注(")
                .and_then(|r| r.strip_suffix(")］"))
                .unwrap_or(raw);
            out.write_str(r#"<sup class="aozora-editor-note">注"#)?;
            escape_text(n, out)?;
            return out.write_str("</sup>");
        }
        // Ruby-placement editorial notes render as a compact visible marker
        // instead of a hidden span, without the annotated run `X`: `X` is
        // typically the immediately preceding text, so emitting it again would
        // render it twice. The raw bracket (with `X`) round-trips on
        // serialize; the reader sees only the marker.
        DirectiveKind::RubyAttached | DirectiveKind::RubyRetarget => {
            return out.write_str(r#"<sup class="aozora-ruby-note">ルビ</sup>"#);
        }
        DirectiveKind::RubyPairOpen => {
            return out.write_str(r#"<sup class="aozora-ruby-note">左ルビ</sup>"#);
        }
        DirectiveKind::RubyPairClose => {
            // ［＃左に「Y」のルビ付き終わり］ → show the left-ruby reading Y. Y is
            // the gloss (not surrounding text), so it does not double-render;
            // recover it from the raw bracket the classifier guaranteed.
            let raw = store.resolve_str(a.raw);
            let y = raw
                .strip_prefix("［＃左に「")
                .and_then(|r| r.strip_suffix("」のルビ付き終わり］"))
                .unwrap_or(raw);
            out.write_str(r#"<sup class="aozora-ruby-note">左ルビ「"#)?;
            escape_text(y, out)?;
            return out.write_str("」</sup>");
        }
        DirectiveKind::MarginNotePairOpen => {
            // ［＃注記付き］ / ［＃左に注記付き］ → a compact marker at the span
            // start; the note text appears on the matching close. The `左に`
            // form is placed on the left and has a distinct marker label.
            let raw = store.resolve_str(a.raw);
            let label = if raw.starts_with("［＃左に") {
                "左注記"
            } else {
                "注記"
            };
            out.write_str(r#"<sup class="aozora-margin-note">"#)?;
            out.write_str(label)?;
            return out.write_str("</sup>");
        }
        DirectiveKind::MarginNotePairClose => {
            // ［＃「Y」の注記付き終わり］ / ［＃左に「Y」の注記付き終わり］ → show the
            // margin-note text Y. Y is the note (not surrounding text), so it
            // does not double-render; recover it from the raw bracket the
            // classifier guaranteed (may hold a nested ［＃…］ gaiji, rendered
            // as literal notation).
            let raw = store.resolve_str(a.raw);
            let left = raw.starts_with("［＃左に「");
            let (label, prefix) = if left {
                ("左注記", "［＃左に「")
            } else {
                ("注記", "［＃「")
            };
            let y = raw
                .strip_prefix(prefix)
                .and_then(|r| r.strip_suffix("」の注記付き終わり］"))
                .unwrap_or(raw);
            out.write_str(r#"<sup class="aozora-margin-note">"#)?;
            out.write_str(label)?;
            out.write_str("「")?;
            escape_text(y, out)?;
            return out.write_str("」</sup>");
        }
        _ => {}
    }
    out.write_str(r#"<span class="aozora-directive" hidden>"#)?;
    escape_text(store.resolve_str(a.raw), out)?;
    out.write_str("</span>")
}

/// Render supplied annotations in their source-established positions.
fn render_kunten<W: Write>(k: Kunten, store: &NodeStore, out: &mut W) -> fmt::Result {
    let (tag, class) = match k.kind {
        KuntenKind::ReturnMark => ("sub", "aozora-kaeriten"),
        KuntenKind::Okurigana => ("sup", "aozora-okurigana"),
    };
    write!(out, "<{tag} class=\"{class}\">")?;
    escape_text(store.resolve_str(k.text), out)?;
    write!(out, "</{tag}>")
}

/// Render an angle-quote as `<span class="aozora-angle-quote">《…》</span>`.
fn render_angle_quote<W: Write>(d: AngleQuote, store: &NodeStore, out: &mut W) -> fmt::Result {
    out.write_str(r#"<span class="aozora-angle-quote">《"#)?;
    render_content_range(d.content, store, out)?;
    out.write_str("》</span>")
}

/// Render an illustration as a `<figure class="aozora-illustration">` with an
/// `<img>` (optional width/height from the dimensions, alt from the
/// description or quoted caption reference). The visible caption is supplied
/// separately by the source.
fn render_sashie<W: Write>(s: &Illustration, store: &NodeStore, out: &mut W) -> fmt::Result {
    out.write_str(r#"<figure class="aozora-illustration"><img src=""#)?;
    escape_text(store.resolve_str(s.file), out)?;
    out.write_char('"')?;
    if let Some((w, h)) = s
        .dimensions
        .map(|id| store.resolve_str(id))
        .and_then(ab_aozora_syntax::parse_image_dimensions)
    {
        write!(out, r#" width="{w}" height="{h}""#)?;
    }
    // The leading description, as in `挿絵１` or `コンドル博士の図`, is the alt.
    out.write_str(r#" alt=""#)?;
    if let Some(description) = s.description {
        escape_text(store.resolve_str(description), out)?;
    }
    if let Some(caption) = s.caption {
        let mut reference = String::new();
        emit_content_as_plain_one(caption, store, &mut reference)?;
        escape_text(&reference, out)?;
    }
    out.write_str(r#"" />"#)?;
    out.write_str("</figure>")
}

/// Render an Aozora heading by wrapping its text with the shared
/// `write_heading_open` / `write_heading_close` writers from
/// [`crate::spelling::html`], so the `<hN>` / `<div>` spelling is defined in
/// one place.
fn render_aozora_heading<W: Write>(h: &Heading, store: &NodeStore, out: &mut W) -> fmt::Result {
    write_heading_open(h.kind, h.style, out)?;
    render_content_range(h.text, store, out)?;
    write_heading_close(h.kind, h.style, out)
}

/// Render a heading hint (`［＃「X」は中見出し］`).
///
/// An unpromoted hint retains its target as hidden reference metadata.
fn render_heading_hint<W: Write>(h: HeadingHint, store: &NodeStore, out: &mut W) -> fmt::Result {
    write!(
        out,
        r#"<span class="aozora-heading-hint" data-level="{level}""#,
        level = h.level.outline_level(),
    )?;
    // `data-style` is emitted only for a non-standard style.
    if let Some(style) = classes::heading_style_slug(h.style) {
        write!(out, r#" data-style="{style}""#)?;
    }
    // Hidden marker: the heading text is in the (promotable) referent run.
    out.write_str(r#" data-target=""#)?;
    escape_text(store.resolve_str(h.target), out)?;
    out.write_str(r#"" hidden></span>"#)
}
