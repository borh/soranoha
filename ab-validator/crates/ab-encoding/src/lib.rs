//! Canonical source-byte decoder for Aozora Bunko source files.
//!
//! Provides the decode and hash steps shared by `ab-check` and (text-only)
//! `ab-coverage`. The decoder strips a UTF-8 BOM, then tries strict UTF-8,
//! then falls back to Windows-31J (Shift_JIS) with a `windows-31j-lossy`
//! classification when bytes are invalid, as reference Aozora parsers do.

use anyhow::Result;
use encoding_rs::SHIFT_JIS;
use sha2::{Digest, Sha256};

pub mod emendation;

pub use emendation::AppliedEmendation;

/// Decoded source text plus its detected encoding and raw byte SHA-256.
///
/// `encoding` is one of: `utf-8-bom`, `utf-8`, `windows-31j`, `windows-31j-lossy`.
/// `raw_sha256` is the `sha256:<hex>` of the original bytes (pre-BOM-strip).
/// `text` is the decoded text after the source emendations in `emendations`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DecodedSource {
    pub text: String,
    pub encoding: String,
    pub raw_sha256: String,
    pub emendations: Vec<AppliedEmendation>,
}

/// Decode source bytes with UTF-8 (BOM-aware) detection and SHIFT-JIS
/// fallback, then apply the source emendation table.
///
/// # Errors
///
/// Returns an error when the bytes start with a UTF-8 BOM but the bytes that
/// follow are not valid UTF-8, when the emendation table is invalid, or when
/// two emendations overlap in the decoded text. Plain UTF-8 and Windows-31J
/// decoding never errors.
pub fn decode_source_bytes(bytes: &[u8]) -> Result<DecodedSource> {
    let raw_sha256 = format!("sha256:{}", hex_sha256(bytes));
    let (text, encoding) = decode_text(bytes)?;
    let (text, emendations) = emendation::emend(&text)?.unwrap_or((text, Vec::new()));
    Ok(DecodedSource {
        text,
        encoding: encoding.to_owned(),
        raw_sha256,
        emendations,
    })
}

/// Decode source bytes without applying the emendation table.
///
/// # Errors
///
/// Returns an error when the bytes start with a UTF-8 BOM but the bytes that
/// follow are not valid UTF-8.
pub fn decode_text(bytes: &[u8]) -> Result<(String, &'static str)> {
    if let Some(body) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return Ok((std::str::from_utf8(body)?.to_owned(), "utf-8-bom"));
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok((text.to_owned(), "utf-8"));
    }
    let (cow, _, had_errors) = SHIFT_JIS.decode(bytes);
    let encoding = if had_errors {
        "windows-31j-lossy"
    } else {
        "windows-31j"
    };
    Ok((cow.into_owned(), encoding))
}

/// Lowercase hex SHA-256 of the input bytes.
#[must_use]
pub fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf8_bom() {
        let decoded = decode_source_bytes(b"\xef\xbb\xbfabc").unwrap();
        assert_eq!(decoded.text, "abc");
        assert_eq!(decoded.encoding, "utf-8-bom");
    }

    #[test]
    fn decodes_windows_31j() {
        let bytes = [0x8c, 0xe1, 0x94, 0x79];
        let decoded = decode_source_bytes(&bytes).unwrap();
        assert_eq!(decoded.text, "吾輩");
        assert_eq!(decoded.encoding, "windows-31j");
    }

    #[test]
    fn decodes_windows_31j_lossy_like_reference_parsers() {
        let decoded = decode_source_bytes(&[0x82, 0xa0, 0xff]).unwrap();
        assert_eq!(decoded.text, "あ�");
        assert_eq!(decoded.encoding, "windows-31j-lossy");
    }

    #[test]
    fn raw_sha256_is_hex_of_original_bytes() {
        let decoded = decode_source_bytes(b"\xef\xbb\xbfabc").unwrap();
        // sha256 over the BOM+payload, not the stripped text.
        let mut h = Sha256::new();
        h.update(b"\xef\xbb\xbfabc");
        assert_eq!(decoded.raw_sha256, format!("sha256:{:x}", h.finalize()));
    }

    #[test]
    fn applies_emendations_after_decoding_and_hashes_the_raw_bytes() {
        let bytes = "前\n一番向｜《むか》うにある".as_bytes();
        let decoded = decode_source_bytes(bytes).unwrap();
        assert_eq!(decoded.text, "前\n一番｜向《むか》うにある");
        assert_eq!(decoded.raw_sha256, format!("sha256:{}", hex_sha256(bytes)));
        let ids = decoded
            .emendations
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["pariyori-403-ruby-bar-after-base"]);
    }

    #[test]
    fn errors_when_bom_prefix_but_body_is_not_utf8() {
        // A UTF-8 BOM followed by a lone 0xff, which is not valid UTF-8.
        let err = decode_source_bytes(&[0xef, 0xbb, 0xbf, 0xff]).unwrap_err();
        assert!(
            err.to_string().contains("utf-8"),
            "expected utf-8 error, got: {err}"
        );
    }
}
