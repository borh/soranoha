//! AST-free byte-spelling primitives.
//!
//! Lifetime-free functions that write markers, tags and escapes, called by
//! both renderers and by the source splice layer. Each function takes only
//! `Copy` scalar payloads (`RegionFormat` / `Container` / `LineFormat` /
//! `HeadingKind` / `&str`), so the byte spelling of each construct is
//! defined in one place.
//!
//! - `html` writes HTML: the block/paragraph state machine, the container /
//!   heading / line tag writers, and the text escaper.
//! - [`source`] writes canonical Aozora source: the container /
//!   layout-directive / heading markers used by the serializer and the splice
//!   layer.

pub(crate) mod html;
pub mod source;
