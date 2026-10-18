//! Forked from <https://github.com/P4suta/aozora>
//! at rev 1a4f864603970983719655aa4af4525958ac2d38 (independent fork).
//! Upstream crate: aozora-spec. License: MIT OR Apache-2.0 (see NOTICE).

//! Aozora Bunko notation specification crate.
//!
//! This crate defines the types shared across crates:
//!
//! - **PUA sentinel codepoints**: the four `U+E001..U+E004` markers the
//!   lexer injects into normalized text (see [`sentinels`]).
//! - **[`Span`]**: `(u32, u32)` byte-range over a UTF-8 source.
//! - **[`TriggerKind`]**: the set of Aozora notation marker characters
//!   (`｜《》≪≫［］＃※〔〕「」`) plus [`classify_trigger_bytes`], which
//!   maps a UTF-8 trigger byte sequence to its kind.
//! - **[`PairKind`]**: categories of balanced open/close delimiters.
//! - **[`Diagnostic`]**: non-fatal observations emitted by pipeline stages.
//!
//! ## Dependency boundary
//!
//! `ab-aozora-syntax`, `ab-aozora-pipeline`, `ab-aozora-render` and
//! `ab-aozora-facade` reference these shared types without depending on the
//! lexer, which depends on the whole pipeline (sanitize → tokenize →
//! pair → classify). This crate therefore depends on no other
//! `ab-aozora-*` crate, only on `miette`/`thiserror`. Every other crate
//! may depend on `ab-aozora-spec` and re-export from it.

#![forbid(unsafe_code)]

pub mod diagnostic;
pub mod offset;
pub mod pair;
pub mod sentinels;
pub mod slugs;
pub mod span;
pub mod trigger;

pub use diagnostic::{
    Diagnostic, DiagnosticInfo, DiagnosticSource, InternalCheckCode, Severity, codes,
};
pub use offset::{NormalizedOffset, SourceOffset};
pub use pair::{PairKind, PairLink};
pub use sentinels::{
    ALL_SENTINELS, BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL,
    Sentinel,
};
pub use slugs::{
    RENDER_SLUGS, RenderSlug, SLUGS, SlugEntry, SlugFamily, canonicalise_slug, roman_slug,
};
pub use span::Span;
pub use trigger::{TriggerKind, classify_trigger_bytes};
