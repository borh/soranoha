//! Source-byte decoder re-exported from the shared `ab-encoding` crate, so
//! `ab_check::encoding::` paths resolve to the decoder `ab-coverage` also uses.
pub use ab_encoding::{DecodedSource, decode_source_bytes, hex_sha256};
