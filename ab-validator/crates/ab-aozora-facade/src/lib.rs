//! Forked from <https://github.com/P4suta/aozora>
//! at rev 1a4f864603970983719655aa4af4525958ac2d38 (independent fork).
//! Upstream crate: aozora. License: MIT OR Apache-2.0 (see NOTICE).

//! `aozora` — the public meta crate.
//!
//! The entry point for parsing Aozora Bunko notation. A downstream
//! consumer depends on this crate alone: it re-exports the public types,
//! and [`Document`] and [`Tree`] give access to the parse.
//!
//! ```
//! use ab_aozora_facade::Document;
//!
//! let doc = Document::new("｜青空《あおぞら》文庫");
//! let tree = doc.parse();
//! let html = tree.to_html();
//! assert!(html.contains("青空")); // the HTML contains the ruby base
//! ```
//!
//! Parse options are set through the builder:
//!
//! ```
//! use ab_aozora_facade::{Document, DiagnosticPolicy};
//!
//! let doc = Document::options()
//!     .diagnostic_policy(DiagnosticPolicy::DropInternal)
//!     .build("｜青梅《おうめ》");
//! let tree = doc.parse();
//! assert!(!tree.to_source().is_empty());
//! ```
//!
//! # Architecture
//!
//! [`Document`] owns the source buffer plus a `Copy` diagnostic
//! policy. [`Document::parse`] returns a [`Tree`] whose `&self`
//! lifetime tracks only that source borrow — the AST data itself is
//! owned, lifetime-free, and `Send + Sync` (an `LexOutput` backed
//! by a flat `NodeStore`: a string interner plus content / segment
//! pools addressed by `u32` handles). The interner deduplicates
//! repeated string content; dropping the tree frees the store in one
//! step, with no per-node `Drop`.
//!
//! The component crates are available through [`pipeline`], [`syntax`],
//! [`render`], and [`encoding`]. Wire projections require the corresponding
//! feature flags.
//!
//! The following README examples also run as doctests.
#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
// On docs.rs, and on the GitHub Pages build when RUSTDOCFLAGS contains
// `--cfg docsrs`, rustdoc emits "Available on crate feature `…`" badges. On
// stable `docsrs` is unset, so the attribute does not enable `feature(doc_cfg)`.
#![cfg_attr(docsrs, feature(doc_cfg))]

use core::ops::Range;

/// Crate version, re-exported for identity strings (ab-aat's
/// `adapter_version` embeds it as the `facade` coordinate).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub use ab_aozora_pipeline::{LexOutput, NodeRef, SourceNode, lex};
pub use ab_aozora_spec::{
    ALL_SENTINELS, BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, Diagnostic,
    DiagnosticInfo, DiagnosticSource, INLINE_SENTINEL, InternalCheckCode, NormalizedOffset,
    PairKind, PairLink, RENDER_SLUGS, RenderSlug, SLUGS, Sentinel, Severity, SlugEntry, SlugFamily,
    SourceOffset, Span, TriggerKind, canonicalise_slug, codes, roman_slug,
};
/// Owned-AST node types that editor features match on (LSP inlay hints, hover,
/// completion, code actions, semantic tokens). Re-exported so that an external
/// consumer depends on `aozora` alone, not on `aozora-syntax`.
pub use ab_aozora_syntax::{
    BlockStyles, BoutenKind, BoutenPosition, CaptionScope, ColumnBlock, ColumnCount,
    DecorationPosition, DirectiveKind, EnclosureKind, FollowingPlacement, FontShift, Format,
    ForwardAttr, ForwardOrigin, HeadingKind, HeadingStyle, IndentBlock, IndentLayout, Kumi,
    LineFormat, LineWidth, NodeKind, QualifiedBreak, QualifiedBreakKind, RegionClose, RegionFormat,
    RubySide, SectionKind,
    ast::{Content, Node, NodeStore},
};

mod diagnostics_text;
mod document;
mod incremental;
mod splice;

#[cfg(feature = "entries")]
#[cfg_attr(docsrs, doc(cfg(feature = "entries")))]
pub mod json;

/// Plain-text diagnostic rendering (`miette`-free, every target).
pub use diagnostics_text::diagnostics_text;
pub use document::{DiagnosticPolicy, Document, ParseOptions, Tree};
/// Source-region ownership and minimal-diff source splicing.
pub use splice::{CoupledKind, Coupling, Region, RegionRole, SpliceError, SpliceSafety};

pub use incremental::{DiagBaseRef, DiagSplice, PieceSeq, SanitizedSrc};

/// Incrementally reparses diagnostics without rebuilding the full tree.
///
/// The LSP calls this on each keystroke. It splices the maintained
/// [`PieceSeq`] (the next edit's
/// region-find base, from which the LSP flattens this edit's diagnostics) from
/// the store-free [`DiagBaseRef`] of the prior parse, without building an
/// [`LexOutput`]: no normalized/sanitized string rebuild, no store
/// clone/graft, no registry or container-pairs rebuild, and no whole-table
/// re-materialization. Cost is `O(region + #pieces)`: the maintained sequence is
/// spliced (prefix/suffix pieces shared by `Arc`), not rebuilt, versus a full
/// parse's `O(doc)`.
///
/// Returns [`DiagSplice`], or `None` for any edit whose locality it cannot prove
/// from the cached tables (the caller then runs a full parse). That the edit
/// changes only bytes inside `edit_old` is a caller precondition, checked only
/// in debug builds. The `corpus_incremental_merge` differential test checks that
/// the returned diagnostics are byte-identical to a full parse's. The full tree
/// is built only when a structural request (rename) requires it.
///
/// This is exposed for the in-workspace LSP consumer only; its shape may change
/// without a major version bump until v0.5.0.
#[must_use]
#[allow(
    clippy::needless_pass_by_value,
    reason = "the lightweight DiagBaseRef (a sanitized source plus one PieceSeq borrow) is taken by value so the in-workspace LSP caller passes its temporary `DiagBaseRef { .. }` literal unchanged; it is forwarded by reference to the generic engine"
)]
pub fn reparse_incremental_diagnostics_only(
    base: DiagBaseRef<'_>,
    new_sanitized: &str,
    edit_old: Range<usize>,
) -> Option<DiagSplice> {
    incremental::reparse_incremental_diagnostics_only(&base, &new_sanitized, edit_old)
}

/// Runs the diagnostics-only incremental path over a generic byte source.
///
/// This variant of [`reparse_incremental_diagnostics_only`] accepts any
/// [`SanitizedSrc`] byte source `S`
/// rather than the `&str`-backed [`DiagBaseRef`]. The in-workspace LSP passes
/// its `ropey`-backed sanitized buffer through this entry, which splices the
/// rope without flattening it to a `String` on each edit; the
/// `corpus_incremental_merge` differential test and the other callers use the
/// `&str` [`reparse_incremental_diagnostics_only`].
///
/// Same contract and fallbacks as [`reparse_incremental_diagnostics_only`]:
/// returns [`DiagSplice`], or `None` for any edit whose locality it cannot prove
/// from the cached tables (the caller then runs a full parse). That the edit
/// changes only bytes inside `edit_old` is a caller precondition, checked only
/// in debug builds. Like that function it is
/// exposed for the in-workspace LSP consumer only; its shape may change without a
/// major version bump until v0.5.0.
///
/// The base is taken by reference (not by value like the `&str` entry) because a
/// rope source holds a cursor and is not `Copy`; callers pass `&DiagBaseRef { .. }`.
#[must_use]
pub fn reparse_incremental_diagnostics_only_in<S: SanitizedSrc>(
    base: &DiagBaseRef<'_, S>,
    new_sanitized: &S,
    edit_old: Range<usize>,
) -> Option<DiagSplice> {
    incremental::reparse_incremental_diagnostics_only(base, new_sanitized, edit_old)
}

/// Eagerly initialise the parser's process-global lazy tables.
///
/// The first [`Document::parse`] then does not include the one-time build
/// cost.
///
/// Without this call the tables are built on first use, so a consumer that
/// never parses, or parses no annotations, does not build them. Call it once,
/// early, from a latency-sensitive front end (e.g. a WASM editor that
/// initialises the parser before the first keystroke). It is idempotent and
/// thread-safe; a repeated call does not rebuild the tables.
///
/// It warms the SIMD trigger-scan backend selection (tokenize stage) and
/// the annotation-classifier Aho-Corasick DFA (classify stage).
///
/// ```
/// ab_aozora_facade::prewarm();
/// let doc = ab_aozora_facade::Document::new("｜青梅《おうめ》");
/// let _ = doc.parse().to_html();
/// ```
pub fn prewarm() {
    ab_aozora_pipeline::prewarm();
}

/// Re-export of [`ab_aozora_pipeline`] under a stable name.
///
/// Editor integrations that require per-phase access
/// (`pipeline::lexer::*` for the phase functions, `pipeline::Pipeline`
/// for the type-state machine) use this module, so that the workspace
/// depends on `aozora` alone. The `aozora-pipeline` crate is published
/// separately but is normally used through this re-export.
pub mod pipeline {
    pub use ab_aozora_pipeline::*;
}

/// Re-export of [`ab_aozora_syntax`] — owned AST node types, the
/// `NodeStore`, and the string interner.
///
/// External callers normally use [`Document`] / [`Tree`] for the
/// parsed AST; this module exposes the underlying types to callers
/// that construct nodes directly (custom renderers, owned-tree
/// transforms).
pub mod syntax {
    pub use ab_aozora_syntax::*;
}

/// Re-export of [`ab_aozora_render`] — owned-AST HTML / source emitters.
///
/// `Tree::to_html` / `Tree::to_source` cover the common cases; custom
/// downstream renderers (EPUB, plain text, LaTeX, …) walk the owned
/// `LexOutput` (its `source_nodes` + `NodeStore`) and can reuse the
/// shared byte-spelling helpers re-exported through this module.
pub mod render {
    pub use ab_aozora_render::*;
}

/// Re-export of [`ab_aozora_encoding`] — `Shift_JIS` decoding and gaiji
/// resolution.
///
/// The sanitize stage of the lex pipeline runs encoding detection
/// first; a caller that decodes without parsing uses this module.
pub mod encoding {
    pub use ab_aozora_encoding::*;
}

/// Aozora-shaped `proptest` strategies.
///
/// Downstream renderer / visitor authors writing their own property
/// tests use this module instead of depending on
/// `ab-notation-strategies`. Enabled by the `proptest` Cargo feature on the `aozora`
/// crate; both `ab_aozora_facade::proptest::*` and the `proptest` crate
/// itself are then in scope for the consumer.
///
/// The generators produce the same input shapes as the workspace's
/// `tests/property_*` suites.
#[cfg(feature = "proptest")]
#[cfg_attr(docsrs, doc(cfg(feature = "proptest")))]
pub mod proptest {
    pub use ab_notation_strategies::*;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_parse_returns_a_tree() {
        let doc = Document::new("hello, world");
        let tree = doc.parse();
        assert_eq!(tree.to_source(), "hello, world");
    }

    #[test]
    fn document_parse_handles_ruby() {
        let doc = Document::new("｜青梅《おうめ》");
        let tree = doc.parse();
        // The serializer writes `｜` before every right-side ruby base that
        // contains no detached gaiji; `sanitized` keeps the source bytes.
        assert_eq!(tree.to_source(), "｜青梅《おうめ》");
        assert_eq!(tree.sanitized(), "｜青梅《おうめ》");
    }

    #[test]
    fn document_to_html_renders_plain_text() {
        let doc = Document::new("hello");
        let tree = doc.parse();
        let html = tree.to_html();
        assert!(html.contains("hello"), "html: {html}");
    }
}
