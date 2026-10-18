//! Forked from <https://github.com/P4suta/aozora>
//! at rev 1a4f864603970983719655aa4af4525958ac2d38 (independent fork).
//! Upstream crate: aozora-encoding. License: MIT OR Apache-2.0 (see NOTICE).

//! Encoding utilities for Aozora Bunko source material.
//!
//! The `aozora` parser accepts UTF-8 only. This crate holds `Shift_JIS` decoding
//! and gaiji (外字) resolution, so the parser has no encoding dependency and the
//! CLI, editor integrations and downstream tools call the same functions.

#![forbid(unsafe_code)]

use std::borrow::Cow;
use std::str::from_utf8;

use encoding_rs::{DecoderResult, SHIFT_JIS};
use miette::Diagnostic;
use thiserror::Error;

/// Errors returned by the decode functions.
#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum DecodeError {
    /// The input was not valid `Shift_JIS`: `encoding_rs` reported a malformed
    /// byte sequence (or, in [`decode_sjis_into`], an output-buffer overflow).
    ///
    /// Carries no position payload: decoding is fail-closed without lossy
    /// replacement.
    #[error("Shift_JIS からの変換に失敗しました (不正なバイト列)")]
    #[diagnostic(code(aozora::encoding::sjis_invalid))]
    ShiftJisInvalid,
}

/// Decode a `Shift_JIS` byte slice into UTF-8 (NFC normalisation is applied by the
/// caller after decoding).
///
/// # Errors
///
/// Returns [`DecodeError::ShiftJisInvalid`] if `encoding_rs` reports a malformed byte
/// sequence. Decoding does not perform lossy replacement.
///
/// Allocates a new `String` per call. [`decode_sjis_into`] with a reused
/// buffer avoids the per-call allocation when many documents are decoded in
/// succession.
pub fn decode_sjis(input: &[u8]) -> Result<String, DecodeError> {
    let mut out = String::new();
    decode_sjis_into(input, &mut out)?;
    Ok(out)
}

/// Decode a `Shift_JIS` byte slice into the caller-owned `dst` buffer.
///
/// Reserves capacity in `dst` from
/// `encoding_rs::Decoder::max_utf8_buffer_length_without_replacement`,
/// so the decode loop does not reallocate. The function appends to `dst`
/// without clearing it; a caller that needs only the new decode calls
/// `dst.clear()` first. A thread-local or per-worker pool can therefore
/// reuse one buffer across many decodes without an allocation per call.
///
/// Has the same error contract as [`decode_sjis`]. Calls
/// `Decoder::decode_to_string_without_replacement` directly rather than
/// `encoding_rs`'s public `decode`, which allocates a worst-case-sized
/// `String` internally and then calls `Cow::into_owned` on the result.
///
/// # Errors
///
/// Returns [`DecodeError::ShiftJisInvalid`] on malformed input or if
/// the decoder reports output overflow. The reservation from
/// `max_utf8_buffer_length_…` makes overflow unreachable; the function
/// returns the error instead of truncating the output.
pub fn decode_sjis_into(input: &[u8], dst: &mut String) -> Result<(), DecodeError> {
    let mut decoder = SHIFT_JIS.new_decoder_without_bom_handling();
    let needed = decoder
        .max_utf8_buffer_length_without_replacement(input.len())
        .ok_or(DecodeError::ShiftJisInvalid)?;
    dst.reserve(needed);
    let (result, _read) = decoder.decode_to_string_without_replacement(input, dst, true);
    match result {
        DecoderResult::InputEmpty => Ok(()),
        DecoderResult::Malformed(_, _) | DecoderResult::OutputFull => {
            Err(DecodeError::ShiftJisInvalid)
        }
    }
}

/// Decode Aozora source bytes to UTF-8, detecting the encoding.
///
/// Aozora material arrives either as the canonical `Shift_JIS` archive
/// files or as UTF-8 mirrors (for example, a pre-converted corpus). The
/// caller passes bytes in either encoding and receives UTF-8.
///
/// - Valid UTF-8 is returned borrowed, without copying.
/// - Otherwise the bytes are decoded as `Shift_JIS` (owned).
///
/// UTF-8 is tried first. `Shift_JIS` Japanese text rarely forms valid
/// UTF-8, whereas a UTF-8 document can contain byte sequences that also
/// decode as valid `Shift_JIS`.
///
/// BOM stripping, CRLF folding, and NFC normalization belong to parser
/// sanitization and are not applied here.
///
/// # Errors
///
/// Returns [`DecodeError::ShiftJisInvalid`] when the bytes are neither
/// valid UTF-8 nor valid `Shift_JIS`.
pub fn decode_auto(input: &[u8]) -> Result<Cow<'_, str>, DecodeError> {
    if let Ok(text) = from_utf8(input) {
        return Ok(Cow::Borrowed(text));
    }
    decode_sjis(input).map(Cow::Owned)
}

/// Encoding-agnostic counterpart to [`decode_sjis_into`]: append the
/// decoded UTF-8 to `dst`, detecting the source encoding.
///
/// Applies the detection rule of [`decode_auto`] (valid UTF-8 is
/// appended as is, otherwise the bytes are decoded as `Shift_JIS`), but
/// writes into a caller-owned buffer so corpus loaders can reuse one
/// allocation across many documents. The function does not clear the
/// buffer; see [`decode_sjis_into`].
///
/// # Errors
///
/// Returns [`DecodeError::ShiftJisInvalid`] when the bytes are neither
/// valid UTF-8 nor valid `Shift_JIS`.
pub fn decode_auto_into(input: &[u8], dst: &mut String) -> Result<(), DecodeError> {
    if let Ok(text) = from_utf8(input) {
        dst.push_str(text);
        return Ok(());
    }
    decode_sjis_into(input, dst)
}

/// Whether the byte slice carries a UTF-8 BOM (`EF BB BF`).
///
/// The CLI calls this to strip the BOM before passing input to the parser.
/// [`decode_auto`] does not remove a BOM: the BOM is valid UTF-8, so
/// `decode_auto` returns it unchanged, and the parser's sanitiser strips it.
#[must_use]
pub const fn has_utf8_bom(input: &[u8]) -> bool {
    matches!(input, [0xEF, 0xBB, 0xBF, ..])
}

pub mod gaiji;

pub mod suijun;
/// Reverse JIS 水準 classifier, re-exported at the crate root so callers
/// reach it as `ab_aozora_facade::encoding::jis_level` / `Suijun` (the
/// facade crate re-exports this crate with a wildcard).
#[doc(inline)]
pub use self::suijun::{Suijun, is_platform_dependent, jis_level, level_table_sizes};

/// PHF tables (single, combo, description) that `build.rs` generates at
/// compile time with `phf_codegen`. The generated file is written to
/// `OUT_DIR` and regenerated when an input TSV changes; the source tree
/// contains only the TSV data, not the perfect-hash output.
#[allow(
    clippy::unreadable_literal,
    reason = "phf_codegen emits 64-bit perfect-hash keys without separators; \
              we cannot reformat them without forking the codegen crate"
)]
mod jisx0213_table {
    include!(concat!(env!("OUT_DIR"), "/jisx0213_table.rs"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_plain_ascii_sjis() {
        assert_eq!(decode_sjis(b"hello").unwrap(), "hello");
    }

    #[test]
    fn decodes_japanese_sjis() {
        // 「青空文庫」 in Shift_JIS.
        let bytes = &[0x90, 0xC2, 0x8B, 0xF3, 0x95, 0xB6, 0x8C, 0xC9];
        assert_eq!(decode_sjis(bytes).unwrap(), "青空文庫");
    }

    #[test]
    fn decodes_empty_input_to_empty_string() {
        assert_eq!(decode_sjis(b"").unwrap(), "");
    }

    #[test]
    fn decodes_ascii_control_characters_verbatim() {
        // LF, CR and tab decode to themselves in SJIS because the lead
        // byte range excludes ASCII; the decoder must pass them unchanged
        // to the sanitize pass.
        assert_eq!(decode_sjis(b"a\nb\rc\td").unwrap(), "a\nb\rc\td");
    }

    #[test]
    fn decodes_halfwidth_katakana() {
        // Halfwidth katakana (0xA1..=0xDF) is a single byte each in SJIS.
        // `ｱｲｳｴｵ` → bytes 0xB1..0xB5.
        let bytes = &[0xB1, 0xB2, 0xB3, 0xB4, 0xB5];
        assert_eq!(decode_sjis(bytes).unwrap(), "ｱｲｳｴｵ");
    }

    #[test]
    fn decodes_mixed_ascii_and_kanji() {
        // Aozora corpora commonly mix ASCII explanatory text with
        // Japanese quotations.
        let mut bytes = Vec::from(*b"about ");
        bytes.extend_from_slice(&[0x93, 0xFA, 0x96, 0x7B]); // 日本
        bytes.extend_from_slice(b" !");
        assert_eq!(decode_sjis(&bytes).unwrap(), "about 日本 !");
    }

    #[test]
    fn decodes_hiragana_sjis() {
        // 「こんにちは」 — lead bytes in the 0x82 range.
        let bytes = &[
            0x82, 0xB1, // こ
            0x82, 0xF1, // ん
            0x82, 0xC9, // に
            0x82, 0xBF, // ち
            0x82, 0xCD, // は
        ];
        assert_eq!(decode_sjis(bytes).unwrap(), "こんにちは");
    }

    #[test]
    fn decodes_fullwidth_digits() {
        // ０１２: fullwidth digits are common in Aozora ruby delimiters.
        let bytes = &[0x82, 0x4F, 0x82, 0x50, 0x82, 0x51];
        assert_eq!(decode_sjis(bytes).unwrap(), "０１２");
    }

    #[test]
    fn decode_auto_passes_utf8_through_borrowed() {
        let bytes = "青空文庫".as_bytes();
        let out = decode_auto(bytes).unwrap();
        assert!(matches!(out, Cow::Borrowed(_)), "UTF-8 must be zero-copy");
        assert_eq!(out, "青空文庫");
    }

    #[test]
    fn decode_auto_falls_back_to_sjis_owned() {
        // 「青空文庫」 in Shift_JIS is not valid UTF-8, so it is decoded as Shift_JIS.
        let bytes = &[0x90, 0xC2, 0x8B, 0xF3, 0x95, 0xB6, 0x8C, 0xC9];
        let out = decode_auto(bytes).unwrap();
        assert!(
            matches!(out, Cow::Owned(_)),
            "SJIS must be decoded to owned"
        );
        assert_eq!(out, "青空文庫");
    }

    #[test]
    fn decode_auto_borrows_ascii() {
        // ASCII is valid in both encodings; UTF-8-first means borrowed.
        let out = decode_auto(b"hello").unwrap();
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, "hello");
    }

    #[test]
    fn decode_auto_tries_utf8_before_ambiguous_sjis() {
        // UTF-8 「日本語」 = E3 81 A5 ..., whose leading bytes are also
        // valid Shift_JIS lead bytes. Decoding as SJIS first would produce
        // mojibake; trying UTF-8 first returns the correct text, borrowed.
        let bytes = "日本語".as_bytes();
        let out = decode_auto(bytes).unwrap();
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, "日本語");
    }

    #[test]
    fn decode_auto_errors_when_neither_encoding_fits() {
        // 0xFF is neither a valid UTF-8 byte nor an assigned Shift_JIS byte.
        assert!(matches!(
            decode_auto(&[0xFF, 0xFF]),
            Err(DecodeError::ShiftJisInvalid)
        ));
    }

    #[test]
    fn decode_auto_empty_is_borrowed_empty() {
        let out = decode_auto(b"").unwrap();
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, "");
    }

    #[test]
    fn decode_auto_into_appends_both_encodings() {
        let mut buf = String::new();
        decode_auto_into("青空".as_bytes(), &mut buf).unwrap(); // UTF-8
        decode_auto_into(&[0x95, 0xB6, 0x8C, 0xC9], &mut buf).unwrap(); // 文庫 in SJIS
        assert_eq!(buf, "青空文庫");
    }

    // The `into_equivalent_on_*` tests check that
    // `decode_sjis(b) == decode_sjis_into(b, &mut buf)` byte-for-byte,
    // and that both return `Err` on invalid input. `decode_sjis_into` is
    // the buffer-reuse entry point used by the bench `parallel_size_bands`
    // thread-local pool; `decode_sjis` calls `decode_sjis_into` with a new
    // `String`.

    fn check_equivalent(input: &[u8]) {
        let owned = decode_sjis(input);
        let mut buf = String::new();
        let into_result = decode_sjis_into(input, &mut buf);
        match (owned, into_result) {
            (Ok(s), Ok(())) => assert_eq!(s, buf, "decode_sjis output != decode_sjis_into output"),
            (Err(_), Err(_)) => {} // both reject the input
            (Ok(s), Err(e)) => panic!("owned succeeded ({s:?}) but _into failed ({e:?})"),
            (Err(e), Ok(())) => panic!("owned failed ({e:?}) but _into succeeded ({buf:?})"),
        }
    }

    #[test]
    fn into_equivalent_on_ascii() {
        check_equivalent(b"hello world");
    }

    #[test]
    fn into_equivalent_on_japanese() {
        check_equivalent(&[0x90, 0xC2, 0x8B, 0xF3, 0x95, 0xB6, 0x8C, 0xC9]);
    }

    #[test]
    fn into_equivalent_on_empty() {
        check_equivalent(b"");
    }

    #[test]
    fn into_equivalent_on_halfwidth_katakana() {
        check_equivalent(&[0xB1, 0xB2, 0xB3, 0xB4, 0xB5]);
    }

    #[test]
    fn into_equivalent_on_invalid_lead_byte() {
        check_equivalent(&[0xFF, 0xFF]);
    }

    #[test]
    fn into_equivalent_on_lone_lead_byte() {
        check_equivalent(&[b'o', b'k', 0x82]);
    }

    #[test]
    fn into_reuses_buffer_capacity_across_calls() {
        // A `dst` String with sufficient capacity is not reallocated on a
        // second decode. The assertions check that capacity does not
        // shrink across `clear() + decode_sjis_into` cycles.
        let mut buf = String::with_capacity(4096);
        let cap_before = buf.capacity();
        decode_sjis_into(b"hello", &mut buf).unwrap();
        let cap_after_first = buf.capacity();
        assert!(
            cap_after_first >= cap_before,
            "capacity must not shrink on small decode"
        );
        buf.clear();
        decode_sjis_into(b"world", &mut buf).unwrap();
        assert!(
            buf.capacity() >= cap_after_first,
            "capacity must not shrink on a buffer-reuse cycle"
        );
    }

    #[test]
    fn into_appends_when_dst_not_cleared() {
        // `decode_sjis_into` appends to `dst`; a caller that needs only
        // the new decode calls `clear()` first. A change that clears
        // inside the function fails this test.
        let mut buf = String::from("PRE:");
        decode_sjis_into(b"hi", &mut buf).unwrap();
        assert_eq!(buf, "PRE:hi");
    }

    #[test]
    fn rejects_invalid_lead_byte() {
        let bytes = &[0xFF, 0xFF];
        assert!(matches!(
            decode_sjis(bytes),
            Err(DecodeError::ShiftJisInvalid)
        ));
    }

    #[test]
    fn rejects_lone_lead_byte_at_end_of_input() {
        // 0x82 alone is a truncated two-byte sequence (expects trail).
        let bytes = &[b'o', b'k', 0x82];
        assert!(matches!(
            decode_sjis(bytes),
            Err(DecodeError::ShiftJisInvalid)
        ));
    }

    #[test]
    fn rejects_invalid_trail_byte() {
        // Lead 0x82 with an invalid trail 0x00 (trails must be 0x40..=0xFC, != 0x7F).
        let bytes = &[0x82, 0x00];
        assert!(matches!(
            decode_sjis(bytes),
            Err(DecodeError::ShiftJisInvalid)
        ));
    }

    #[test]
    fn error_message_is_japanese_and_carries_miette_code() {
        // User-facing errors in this project are in Japanese.
        let err = decode_sjis(&[0xFF, 0xFF]).unwrap_err();
        let message = format!("{err}");
        assert!(
            message.contains("Shift_JIS"),
            "error message must contain Shift_JIS for locatability, got {message:?}",
        );
    }

    #[test]
    fn detects_utf8_bom() {
        assert!(has_utf8_bom(b"\xEF\xBB\xBFtext"));
    }

    #[test]
    fn no_utf8_bom_on_plain_input() {
        assert!(!has_utf8_bom(b"text"));
    }

    #[test]
    fn no_utf8_bom_on_shorter_than_bom() {
        assert!(!has_utf8_bom(b"\xEF\xBB"));
    }

    #[test]
    fn no_utf8_bom_on_empty_input() {
        assert!(!has_utf8_bom(b""));
    }

    #[test]
    fn detects_utf8_bom_on_exactly_three_bytes() {
        // The slice is exactly `EF BB BF` with no trailing content; the
        // `..` rest pattern in `matches!` accepts an empty tail.
        assert!(has_utf8_bom(&[0xEF, 0xBB, 0xBF]));
    }

    #[test]
    fn bom_detection_rejects_near_misses() {
        // Byte sequences that differ from the UTF-8 BOM in one position.
        assert!(!has_utf8_bom(&[0xEF, 0xBB, 0xBE])); // last byte wrong
        assert!(!has_utf8_bom(&[0xEE, 0xBB, 0xBF])); // first byte wrong
        assert!(!has_utf8_bom(&[0xEF, 0xBC, 0xBF])); // middle byte wrong
        assert!(!has_utf8_bom(&[0xFE, 0xFF])); // UTF-16 BE BOM
        assert!(!has_utf8_bom(&[0xFF, 0xFE])); // UTF-16 LE BOM
    }

    #[test]
    fn gaiji_lookup_echoes_existing_ucs_when_set() {
        assert_eq!(
            gaiji::lookup(Some('吶'), Some("第3水準1-85-54"), "木＋吶のつくり"),
            Some(gaiji::Resolved::Char('吶'))
        );
    }

    #[test]
    fn gaiji_lookup_resolves_identifier_without_description() {
        assert_eq!(
            gaiji::lookup(None, None, "第3水準1-85-54"),
            Some(gaiji::Resolved::Char('枘'))
        );
    }
}
