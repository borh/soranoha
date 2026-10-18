//! HTML rendering over the semantic AST.
//!
//! Renders the normalized text in a single forward walk driving a block-level
//! `RenderState`, dispatching each PUA sentinel through an [`LexOutput`]'s
//! [`Registry`](ab_aozora_syntax::ast::Registry) and resolving its
//! interned payloads against the [`NodeStore`].
//!
//! The block-structure functions (`RenderState`'s paragraph / container logic)
//! and the plain-run escaper (`escape_text_chunk`) come from
//! `crate::spelling::html`; they read only `Copy` `RegionFormat` / `RegionClose` /
//! `bool` scalars, so container HTML is generated in one place. The private
//! `render_node` module contains only the per-node emitters that read the AST.

use ab_aozora_spec::NormalizedOffset;
use core::{cmp::Reverse, fmt, mem::discriminant};
use std::collections::BTreeMap;

use ab_aozora_syntax::ast::{ContainerPair, LexOutput, Node, NodeRef, NodeStore};
use ab_aozora_syntax::{DirectiveKind, RegionClose, RegionFormat};

use crate::render_node::render;
use crate::serialize::{DirectiveNormalization, SerializeOptions, serialize_with};
use crate::spelling::html::{RenderState, escape_text_chunk};
use crate::walk::{SentinelKind, WalkSink, walk};

/// Options controlling the opt-in HTML render path.
///
/// The default (`directives: Off`) render is byte-identical and applies no
/// catalogue judgement: an `Unknown` directive body the parser did not
/// recognise renders as an inert `<span class="aozora-directive" hidden>`, so
/// output does not depend on the notation-hygiene catalogue.
///
/// [`render_html_normalized`] rewrites near-misses to their canonical spelling
/// through the formatter rewrite, which reads the catalogue; this module holds
/// no copy of it. A known 揺れ then renders as an active HTML element instead
/// of a hidden directive span. `Canonical` applies Tier1 only
/// (`render --normalize`); `Degraded` also reduces lossy or judgment-based
/// Tier2 forms (`render --degraded`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderOptions {
    /// Which notation-hygiene tiers to apply to `DirectiveKind::Unknown`
    /// near-misses when rendering.
    pub directives: DirectiveNormalization,
}

/// Render an [`LexOutput`] into HTML after normalising its `Unknown`
/// directive near-misses at the given [`DirectiveNormalization`] level.
///
/// The function serializes the tree through the formatter rewrite, which
/// applies the notation-hygiene catalogue, re-parses the resulting canonical
/// source, and renders it with [`render_html`]. The rewritten source is a
/// temporary buffer: neither the caller's source nor `out` is modified. A body
/// with no catalogue entry is copied verbatim and renders as it does under
/// [`render_html`].
///
/// `level` is `Canonical` (Tier1) or `Degraded` (Tier1 + Tier2). Only this
/// render path accepts `Degraded`, so the lossy / judgment Tier2 reductions
/// apply only to the temporary render buffer and never to serialized source.
#[must_use]
pub fn render_html_normalized(out: &LexOutput, level: DirectiveNormalization) -> String {
    let normalized = serialize_with(out, SerializeOptions { directives: level });
    render_html(&ab_aozora_pipeline::lex(&normalized))
}

/// Render an [`LexOutput`] into a fresh `String`.
///
/// Allocates roughly `2 × normalized.len()` upfront. [`render_html_into`]
/// writes to a caller-supplied writer without the intermediate `String`.
///
/// # Panics
///
/// Does not panic: writing to a `String` through [`fmt::Write`] cannot fail,
/// and the internal `expect` covers that unreachable error.
#[must_use]
pub fn render_html(out: &LexOutput) -> String {
    let mut s = String::with_capacity(out.normalized.len().saturating_mul(2));
    render_html_into(out, &mut s).expect("writing to String never fails");
    s
}

/// Render an [`LexOutput`] into the given writer.
///
/// # Errors
///
/// Propagates write errors from `writer`.
///
/// # Panics
///
/// Panics if the normalized text exceeds `u32::MAX` bytes, the width of the
/// lexer's `Span` offsets; the sanitize stage caps input below that length.
pub fn render_html_into<W: fmt::Write>(out: &LexOutput, writer: &mut W) -> fmt::Result {
    let (scope_opens, scope_closes) = independent_scope_events(out);
    let mut sink = HtmlSink {
        scope_opens,
        scope_closes,
        store: &out.store,
        out: writer,
        state: RenderState::default(),
    };
    walk(out, &mut sink)
}

fn independent_scope_events(
    out: &LexOutput,
) -> (
    BTreeMap<NormalizedOffset, Vec<RegionFormat>>,
    BTreeMap<NormalizedOffset, Vec<RegionClose>>,
) {
    let mut groups = BTreeMap::<NormalizedOffset, Vec<&ContainerPair>>::new();
    for pair in &out.container_pairs {
        if matches!(
            pair.kind,
            RegionFormat::Horizontal(_) | RegionFormat::Framed(_)
        ) && matches!(
            out.registry.node_at(pair.open),
            Some(NodeRef::BlockOpen(
                RegionFormat::Indent(_) | RegionFormat::Columns(_)
            ))
        ) {
            groups.entry(pair.open).or_default();
        }
    }
    for pair in &out.container_pairs {
        if let Some(group) = groups.get_mut(&pair.open) {
            group.push(pair);
        }
    }
    let mut opens = BTreeMap::new();
    let mut closes = BTreeMap::<NormalizedOffset, Vec<RegionClose>>::new();
    for (position, mut pairs) in groups {
        let Some(NodeRef::BlockOpen(original)) = out.registry.node_at(position) else {
            continue;
        };
        if pairs.iter().all(|pair| pair.kind == original) {
            continue;
        }
        let mut remainder = original;
        let styles = match &mut remainder {
            RegionFormat::Indent(block) => &mut block.styles,
            RegionFormat::Columns(block) => &mut block.styles,
            _ => continue,
        };
        for pair in &pairs {
            match pair.kind {
                RegionFormat::Horizontal(_) => styles.horizontal = None,
                RegionFormat::Framed(_) => styles.frame = None,
                _ => {}
            }
        }
        let complete = pairs
            .iter()
            .any(|pair| discriminant(&pair.kind) == discriminant(&original));
        pairs.sort_by_key(|pair| Reverse(pair.close));
        let mut regions = Vec::with_capacity(pairs.len() + usize::from(!complete));
        if !complete {
            regions.push(remainder);
        }
        for pair in pairs {
            regions.push(pair.kind);
            closes
                .entry(pair.close)
                .or_default()
                .push(RegionClose::of(pair.kind));
        }
        opens.insert(position, regions);
    }
    (opens, closes)
}

/// [`WalkSink`] that emits semantic HTML5 from the AST. It passes the
/// [`NodeStore`], which resolves interned payloads, to every AST emitter and
/// uses `crate::spelling::html`'s [`RenderState`] for all block / paragraph /
/// container structure.
struct HtmlSink<'a, W: fmt::Write> {
    scope_opens: BTreeMap<NormalizedOffset, Vec<RegionFormat>>,
    scope_closes: BTreeMap<NormalizedOffset, Vec<RegionClose>>,
    store: &'a NodeStore,
    out: &'a mut W,
    state: RenderState,
}

impl<W: fmt::Write> WalkSink for HtmlSink<'_, W> {
    // HTML output treats `\n` as structural (paragraph / line break).
    const WANTS_NEWLINES: bool = true;

    fn on_node_at(
        &mut self,
        position: NormalizedOffset,
        kind: SentinelKind,
        node: NodeRef,
    ) -> fmt::Result {
        if let Some(regions) = self.scope_opens.get(&position) {
            for region in regions {
                self.state.open_container(*region, self.out)?;
            }
            return Ok(());
        }
        if let Some(regions) = self.scope_closes.get(&position) {
            for region in regions.iter().rev() {
                self.state.close_container(region.is_inline(), self.out)?;
            }
            return Ok(());
        }
        self.on_node(kind, node)
    }

    fn on_text(&mut self, text: &str) -> fmt::Result {
        self.state.ensure_in_paragraph(self.out)?;
        escape_text_chunk(text, self.out)
    }

    fn on_newline(&mut self, next: Option<u8>) -> fmt::Result {
        match next {
            // A blank line closes the current paragraph.
            Some(b'\n') => self.state.close_paragraph(self.out),
            // A lone newline inside a paragraph is a line break.
            Some(_) if self.state.in_paragraph => self.out.write_str("<br />\n"),
            // A newline outside a paragraph (e.g. between blocks) is dropped.
            Some(_) | None => Ok(()),
        }
    }

    fn on_node(&mut self, kind: SentinelKind, node: NodeRef) -> fmt::Result {
        match (kind, node) {
            (SentinelKind::Inline, NodeRef::Inline(n)) => {
                self.state.ensure_in_paragraph(self.out)?;
                // `RenderState` writes the inline warichu open / close tags and
                // keeps their depth counter, as it does for containers, so the
                // warichu span stays balanced across paragraph boundaries.
                // Every other inline node goes to `render`; the match borrows
                // `n` to read `kind` and moves `n` into `render` in the
                // fall-through arm.
                match &n {
                    Node::Directive(a) if a.kind == DirectiveKind::WarichuOpen => {
                        self.state.open_warichu(self.out)
                    }
                    Node::Directive(a) if a.kind == DirectiveKind::WarichuClose => {
                        self.state.close_warichu(self.out)
                    }
                    _ => render(n, self.store, self.out),
                }
            }
            (SentinelKind::BlockLeaf, NodeRef::BlockLeaf(n)) => {
                self.state.before_block_emit(self.out)?;
                render(n, self.store, self.out)?;
                self.state.after_block_emit();
                Ok(())
            }
            (SentinelKind::BlockOpen, NodeRef::BlockOpen(open)) => {
                self.state.open_container(open, self.out)
            }
            (SentinelKind::BlockClose, NodeRef::BlockClose(close)) => {
                // Pass the close marker's inline-ness so an unmatched inline
                // close in a paragraph gap cancels a pending reopen and does not
                // pop a block from the stack.
                self.state.close_container(close.is_inline(), self.out)
            }
            // A sentinel whose kind does not match its registry entry renders
            // nothing.
            _ => Ok(()),
        }
    }

    fn finish(&mut self) -> fmt::Result {
        // Close any region a source left open (an unbalanced ［＃ここから…］):
        // to_html must emit balanced markup even though to_source keeps the
        // imbalance verbatim. The unclosed region extends to end-of-document.
        self.state.drain_open_containers(self.out)?;
        self.state.close_paragraph(self.out)
    }
}
