//! Fuzz target: `aozora_encoding::decode_sjis` on arbitrary bytes.
//!
//! A decode error (non-Shift_JIS input) ends the iteration without a
//! failure. The target asserts that the decoder does not panic on
//! adversarial input (truncated trail bytes, lead byte at EOF, malformed
//! multi-byte sequences) and that a successful decode is valid UTF-8.

#![no_main]

use aozora_encoding::decode_sjis;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = decode_sjis(data) else {
        return;
    };
    // `decode_sjis` returns `String`, which is UTF-8 by type; the
    // assertion fails if a change to the return type admits non-UTF-8
    // output.
    assert!(
        std::str::from_utf8(text.as_bytes()).is_ok(),
        "decode_sjis returned non-UTF-8 String for input bytes = {data:?}",
    );
});
