//! Single-file binary corpus archive.
//!
//! Stores the entire corpus in one binary file, pre-packed and
//! optionally pre-decoded and/or zstd-compressed, in place of a
//! directory of 17 435 small files read with walkdir, per-file
//! `read(2)` and `decode_sjis` on each call.
//!
//! # Format
//!
//! The reference data structures are Git pack files, SQLite page
//! files, and `Apache Arrow IPC`: a single sequentially laid-out file
//! with an offset index for O(1) random access. Aozora's read-only
//! corpus-sweep workload requires only a minimal subset of them:
//!
//! ```text
//! [Header — 16 bytes]
//!   4   magic       = b"AOZC"
//!   4   version     = u32 LE (currently 1)
//!   4   flags       = u32 LE
//!         bit 0:  payload is zstd-compressed per-entry
//!         bit 1:  payload is pre-decoded UTF-8 (otherwise raw SJIS)
//!   4   entry_count = u32 LE
//!
//! [Index — entry_count records, packed back-to-back, variable size]
//!   8   payload_offset   : u64 LE  (from start of file)
//!   4   payload_len      : u32 LE  (bytes in file, possibly compressed)
//!   4   decoded_len      : u32 LE  (bytes after decompress; for raw
//!                                   payloads equals payload_len)
//!   8   source_mtime_ns  : i64 LE  (for incremental-pack diff)
//!   32  source_blake3    : [u8;32] (for incremental-pack diff)
//!   4   label_len        : u32 LE
//!   ?   label_bytes      : variable UTF-8
//!
//! [Payload]
//!   Concatenated entry bytes, each at its declared offset.
//! ```
//!
//! Four variants, selected by flags at build time:
//!
//! | flags  | name              | trade-off |
//! |---|---|---|
//! | `0b00` | raw SJIS          | smallest build cost, runtime SJIS decode |
//! | `0b01` | zstd SJIS         | smaller disk, runtime SJIS decode |
//! | `0b10` | pre-decoded UTF-8 | larger disk, decode skipped at runtime |
//! | `0b11` | zstd UTF-8        | lowest runtime wall-clock time (read+decompress only) |
//!
//! # Change detection uses mtime and blake3
//!
//! mtime can be stale (filesystem races, `touch`-without-content-change,
//! cache restored from backup). The blake3 hash confirms identity when
//! the mtime indicates a possible change. The per-entry hash adds 32
//! bytes to each index record (~544 KB for a 17 k-file corpus).
//!
//! # Lifetime model
//!
//! Zero-copy reads through `mmap` require `unsafe`, which the
//! workspace forbids here, so [`Archive::open`] reads the whole file
//! into a single `Vec<u8>` (one `fs::read` syscall; the kernel
//! performs one page-cache → vec memcpy).
//! `Archive::iter_borrowed` yields zero-copy `&[u8]` slices into
//! the archive's payload for raw entries; zstd-decompressed entries
//! materialise into a fresh `Vec<u8>` per call (decompression
//! intrinsically allocates).

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::missing_panics_doc,
    clippy::too_many_arguments,
    clippy::doc_markdown,
    clippy::too_many_lines,
    reason = "binary-format parser/builder module; on-disk type widths and magic-number boundaries are documented inline in the format spec at the top of this file. The casts are intentional and bounded by the spec; panics are unreachable per the spec's `try_from` checks at the API boundary."
)]

use core::error;
use core::fmt;
use core::str;
use std::fs;
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::CorpusError;

/// Magic bytes at the start of every archive — `b"AOZC"` (Aozora Corpus).
pub const MAGIC: [u8; 4] = *b"AOZC";

/// Current archive format version.
pub const FORMAT_VERSION: u32 = 1;

/// Flag bit indicating per-entry zstd compression.
pub const FLAG_ZSTD: u32 = 1 << 0;

/// Flag bit indicating the payload is pre-decoded UTF-8 text. When
/// unset, the payload is raw Shift-JIS bytes — same shape as
/// [`crate::CorpusItem::bytes`] in the directory-walker path.
pub const FLAG_UTF8: u32 = 1 << 1;

/// Length of a per-entry index record's fixed header. The label
/// (variable-length) follows.
const INDEX_FIXED_LEN: usize = 8 + 4 + 4 + 8 + 32 + 4;

/// Length of the file header.
const HEADER_LEN: usize = 4 + 4 + 4 + 4;

/// Maximum per-entry decompressed size, in bytes.
///
/// Real-world Aozora files are at most ~10 MB. The parser rejects a
/// `decoded_len` field above 256 MiB instead of allocating the buffer,
/// as a denial-of-service bound: an archive can declare `u32::MAX`
/// (4 GB), and `Vec::with_capacity(u32::MAX as usize)` would exhaust
/// memory before the zstd stream produced its first byte.
pub const MAX_DECODED_LEN_PER_ENTRY: u32 = 256 * 1024 * 1024;

/// Errors specific to archive format parsing / IO. Wraps generic
/// [`CorpusError::Io`] for filesystem failures and adds format-level
/// distinctions (bad magic, unsupported version, truncated index).
#[derive(Debug)]
#[non_exhaustive]
pub enum ArchiveError {
    /// I/O failed (read / write / open).
    Io(io::Error),
    /// File does not start with the archive magic bytes.
    BadMagic,
    /// File version is newer than this binary supports.
    UnsupportedVersion(u32),
    /// File is shorter than the declared header / index / payload
    /// would require.
    Truncated {
        /// The structure being read (e.g. `"index entry 14233"`).
        what: &'static str,
    },
    /// Per-entry zstd decompression failed.
    Decompress(io::Error),
    /// Label bytes are not valid UTF-8.
    BadLabel,
    /// A header field declares a size that cannot fit in the
    /// archive (entry count too large, declared decoded length
    /// exceeds the per-entry budget, etc.). Distinct from
    /// [`Self::Truncated`]: the file may be intact while the *header
    /// value* exceeds its bound. Without this check, an archive
    /// declaring a 4 GB decoded length on a 10-byte payload would
    /// exhaust memory.
    InvalidSize {
        /// Which field overflowed the bound (`"entry count"`,
        /// `"decoded_len"`, …).
        what: &'static str,
        /// The rejected value, for diagnostics.
        value: u64,
    },
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "archive IO: {e}"),
            Self::BadMagic => f.write_str("file does not start with the AOZC magic bytes"),
            Self::UnsupportedVersion(v) => write!(
                f,
                "archive format version {v} is not supported by this binary"
            ),
            Self::Truncated { what } => write!(f, "archive truncated while reading {what}"),
            Self::Decompress(e) => write!(f, "zstd decompression failed: {e}"),
            Self::BadLabel => f.write_str("entry label is not valid UTF-8"),
            Self::InvalidSize { what, value } => {
                write!(
                    f,
                    "archive header field {what} is unreasonably large ({value})"
                )
            }
        }
    }
}

impl error::Error for ArchiveError {}

impl From<io::Error> for ArchiveError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<ArchiveError> for CorpusError {
    fn from(value: ArchiveError) -> Self {
        match value {
            ArchiveError::Io(source) => Self::Io {
                path: PathBuf::new(),
                source,
            },
            other => Self::Io {
                path: PathBuf::new(),
                source: io::Error::other(other.to_string()),
            },
        }
    }
}

/// Per-entry metadata held in the archive's index. Each field
/// corresponds to one field of the on-disk index record, and its
/// documentation gives the on-disk semantics.
#[derive(Debug, Clone)]
pub struct EntryMeta {
    /// Offset in bytes from the start of the archive file at which
    /// this entry's payload begins.
    pub payload_offset: u64,
    /// Number of payload bytes in the file. For zstd-compressed
    /// archives this is the compressed length; for raw archives it
    /// equals `decoded_len`.
    pub payload_len: u32,
    /// Number of bytes after decompression. For raw archives this
    /// equals `payload_len`. Callers can use it to pre-allocate a
    /// destination buffer before decoding.
    pub decoded_len: u32,
    /// Source-file `mtime` in nanoseconds since UNIX epoch. The
    /// incremental pack diff compares it to select files that may have
    /// changed, then confirms a change with the full-content `blake3`
    /// hash.
    pub source_mtime_ns: i64,
    /// blake3 hash of the source file bytes (raw SJIS bytes for
    /// archives built from a directory of `.txt` files). Provides the
    /// content-addressed identity used by the incremental pack diff.
    pub source_blake3: [u8; 32],
    /// Human-readable label (typically the path relative to the
    /// corpus root).
    pub label: String,
}

/// A loaded corpus archive — header + parsed index + payload bytes
/// held in memory.
#[derive(Debug)]
pub struct Archive {
    flags: u32,
    entries: Vec<EntryMeta>,
    /// Raw archive bytes from offset 0 to end. Per-entry payloads
    /// are slices into this buffer at `entry.payload_offset ..
    /// entry.payload_offset + entry.payload_len`.
    bytes: Vec<u8>,
}

impl Archive {
    /// Open and fully load an archive from disk. Reads the file in
    /// one [`fs::read`] call; subsequent [`Self::iter`] /
    /// `Self::iter_borrowed` calls slice into the in-memory buffer.
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError`] if the file is missing, malformed, or
    /// uses a version this binary does not support.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ArchiveError> {
        let bytes = fs::read(path)?;
        Self::from_bytes(bytes)
    }

    /// Parse an archive from an already-loaded byte buffer.
    ///
    /// # Errors
    ///
    /// Same conditions as [`Self::open`], except the I/O case.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, ArchiveError> {
        if bytes.len() < HEADER_LEN {
            return Err(ArchiveError::Truncated { what: "header" });
        }
        if bytes[0..4] != MAGIC {
            return Err(ArchiveError::BadMagic);
        }
        let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if version != FORMAT_VERSION {
            return Err(ArchiveError::UnsupportedVersion(version));
        }
        let flags = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let count = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;

        // Hard cap: every index record needs at least INDEX_FIXED_LEN
        // bytes (the variable label adds more). A `count` larger than
        // `(file_size - HEADER_LEN) / INDEX_FIXED_LEN` indicates a corrupted or
        // adversarial header; reject it before allocating Vec<EntryMeta>.
        let count_ceiling = bytes.len().saturating_sub(HEADER_LEN) / INDEX_FIXED_LEN;
        if count > count_ceiling {
            return Err(ArchiveError::InvalidSize {
                what: "entry count",
                value: count as u64,
            });
        }

        let mut entries = Vec::with_capacity(count);
        let mut cursor = HEADER_LEN;
        for _ in 0..count {
            if cursor + INDEX_FIXED_LEN > bytes.len() {
                return Err(ArchiveError::Truncated {
                    what: "index entry header",
                });
            }
            let payload_offset =
                u64::from_le_bytes(bytes[cursor..cursor + 8].try_into().expect("8-byte slice"));
            let payload_len = u32::from_le_bytes(
                bytes[cursor + 8..cursor + 12]
                    .try_into()
                    .expect("4-byte slice"),
            );
            let decoded_len = u32::from_le_bytes(
                bytes[cursor + 12..cursor + 16]
                    .try_into()
                    .expect("4-byte slice"),
            );
            // Reject a decoded_len above the per-entry maximum at parse
            // time so `payload_at` allocates from a bounded budget.
            // Aozora documents are far smaller than
            // `MAX_DECODED_LEN_PER_ENTRY` (256 MiB); a larger value
            // indicates a decompression bomb.
            if decoded_len > MAX_DECODED_LEN_PER_ENTRY {
                return Err(ArchiveError::InvalidSize {
                    what: "decoded_len",
                    value: u64::from(decoded_len),
                });
            }
            let source_mtime_ns = i64::from_le_bytes(
                bytes[cursor + 16..cursor + 24]
                    .try_into()
                    .expect("8-byte slice"),
            );
            let mut source_blake3 = [0u8; 32];
            source_blake3.copy_from_slice(&bytes[cursor + 24..cursor + 56]);
            let label_len = u32::from_le_bytes(
                bytes[cursor + 56..cursor + 60]
                    .try_into()
                    .expect("4-byte slice"),
            ) as usize;
            cursor += INDEX_FIXED_LEN;
            if cursor + label_len > bytes.len() {
                return Err(ArchiveError::Truncated {
                    what: "index entry label",
                });
            }
            let label = str::from_utf8(&bytes[cursor..cursor + label_len])
                .map_err(|_| ArchiveError::BadLabel)?
                .to_owned();
            cursor += label_len;

            // The declared payload range must lie within the file
            // before any reader slices it.
            let payload_end = payload_offset.checked_add(u64::from(payload_len)).ok_or(
                ArchiveError::Truncated {
                    what: "index entry payload range overflow",
                },
            )?;
            if payload_end > bytes.len() as u64 {
                return Err(ArchiveError::Truncated {
                    what: "index entry payload past end of file",
                });
            }
            entries.push(EntryMeta {
                payload_offset,
                payload_len,
                decoded_len,
                source_mtime_ns,
                source_blake3,
                label,
            });
        }

        Ok(Self {
            flags,
            entries,
            bytes,
        })
    }

    /// Whether the archive's payloads are zstd-compressed per entry.
    #[must_use]
    pub fn is_zstd(&self) -> bool {
        self.flags & FLAG_ZSTD != 0
    }

    /// Whether the archive's payloads are pre-decoded UTF-8 (rather
    /// than raw Shift-JIS bytes).
    #[must_use]
    pub fn is_utf8(&self) -> bool {
        self.flags & FLAG_UTF8 != 0
    }

    /// Number of entries in the archive.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the archive is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The archive's flags word, for callers that report the variant
    /// (for example "this archive is zstd-compressed UTF-8").
    #[must_use]
    pub fn flags(&self) -> u32 {
        self.flags
    }

    /// Borrow every entry's metadata. Iterating this is much cheaper
    /// than [`Self::iter`] when the caller only needs labels /
    /// lengths.
    #[must_use]
    pub fn entries(&self) -> &[EntryMeta] {
        &self.entries
    }

    /// Random-access counterpart to `Self::iter_borrowed` — yield
    /// the payload at `index` directly, no iteration cost.
    ///
    /// Used by parallel callers that fan out via `(0..len).into_par_iter()`
    /// — calling `iter_borrowed().nth(i)` inside the parallel body
    /// would be O(n) per item and turn the whole loop O(n²);
    /// `payload_at(i)` is O(1).
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError::Decompress`] if the entry is
    /// zstd-compressed and decompression fails.
    ///
    /// # Panics
    ///
    /// Panics if `index >= self.len()`.
    pub fn payload_at(&self, index: usize) -> Result<ArchivePayload<'_>, ArchiveError> {
        let entry = &self.entries[index];
        let raw = self.raw_payload(index);
        if self.is_zstd() {
            let bytes = decompress_payload(raw, entry.decoded_len as usize)
                .map_err(ArchiveError::Decompress)?;
            Ok(ArchivePayload::Decompressed(bytes))
        } else {
            Ok(ArchivePayload::Borrowed(raw))
        }
    }

    /// Borrow the raw payload slice for a given entry — without
    /// decompression. Returns the on-disk bytes (potentially
    /// zstd-compressed). For the high-level reader use [`Self::iter`].
    ///
    /// # Panics
    ///
    /// Panics if `index >= self.len()`.
    #[must_use]
    pub fn raw_payload(&self, index: usize) -> &[u8] {
        let entry = &self.entries[index];
        let start = entry.payload_offset as usize;
        let end = start + entry.payload_len as usize;
        &self.bytes[start..end]
    }

    /// Iterate entries, decompressing each payload on demand if
    /// needed. Yields owned `(label, bytes)` pairs: raw
    /// archives yield a copy of the in-memory slice;
    /// zstd archives yield freshly decompressed bytes.
    ///
    /// Use `Self::iter_borrowed` when borrowed slices suffice
    /// to avoid copying.
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError::Decompress`] if a per-entry zstd
    /// decompression fails. Failures do not abort iteration; each
    /// item is yielded as `Result`.
    pub fn iter(&self) -> impl Iterator<Item = Result<(String, Vec<u8>), ArchiveError>> + '_ {
        self.iter_borrowed().map(|item| {
            let (label, payload) = item?;
            Ok((label.to_owned(), payload.into_owned()))
        })
    }

    /// Zero-copy iterator.
    ///
    /// Yields `(label_borrow, payload_borrow_or_decompress)` pairs
    /// where:
    ///
    /// - `label_borrow: &'a str` borrows directly from the archive's
    ///   in-memory index — no allocation per entry.
    /// - `payload` is an [`ArchivePayload<'a>`]: for raw archives it
    ///   borrows the in-memory payload slice (zero-copy); for zstd
    ///   archives it materialises the decompressed bytes into a fresh
    ///   `Vec<u8>` (decompression intrinsically allocates, so there
    ///   is no zero-copy path for compressed payloads — the
    ///   `ArchivePayload` enum exposes both shapes uniformly).
    ///
    /// Compared with [`Self::iter`], this avoids one `Vec<u8>`
    /// allocation and memcpy per entry on raw archives, a measurable
    /// reduction in allocator pressure on a 17 k-entry corpus.
    ///
    /// # Errors
    ///
    /// Same as [`Self::iter`] — per-entry zstd decompression failures
    /// surface as `Err` items.
    pub fn iter_borrowed(
        &self,
    ) -> impl Iterator<Item = Result<(&str, ArchivePayload<'_>), ArchiveError>> + '_ {
        self.entries.iter().enumerate().map(move |(i, entry)| {
            let payload = self.payload_at(i)?;
            Ok((entry.label.as_str(), payload))
        })
    }
}

/// Per-entry payload yielded by `Archive::iter_borrowed`.
///
/// Unifies the two shapes the archive can produce — a borrowed slice
/// into the in-memory archive (raw variants) or a freshly
/// decompressed owned `Vec<u8>` (zstd variants) — in one type, so
/// callers need not branch on `archive.is_zstd()`.
///
/// Both variants implement [`Self::as_bytes`] for borrow-only use,
/// and [`Self::into_owned`] for the cases where ownership is required
/// (e.g. building a `String` via `String::from_utf8`).
#[derive(Debug)]
pub enum ArchivePayload<'a> {
    /// Raw payload — slice into the archive's in-memory bytes. No
    /// per-entry allocation; lifetime tied to the [`Archive`].
    Borrowed(&'a [u8]),
    /// Freshly decompressed payload. Owned because zstd
    /// decompression intrinsically allocates a destination buffer.
    Decompressed(Vec<u8>),
}

impl ArchivePayload<'_> {
    /// Borrow the payload bytes irrespective of variant.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Borrowed(s) => s,
            Self::Decompressed(v) => v,
        }
    }

    /// Take ownership — copies the bytes if this is a `Borrowed`
    /// variant, returns the `Vec` directly if `Decompressed`.
    #[must_use]
    pub fn into_owned(self) -> Vec<u8> {
        match self {
            Self::Borrowed(s) => s.to_vec(),
            Self::Decompressed(v) => v,
        }
    }

    /// Length of the payload in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.as_bytes().len()
    }

    /// Whether the payload is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.as_bytes().is_empty()
    }
}

/// Decompress a zstd-compressed payload into a fresh `Vec`.
///
/// `expected_len` is a hint used to pre-size the destination
/// buffer; it is clamped to [`MAX_DECODED_LEN_PER_ENTRY`], so an
/// archive header declaring a huge decoded length on a tiny compressed
/// payload cannot cause an out-of-memory pre-allocation. Growth of the
/// vector beyond `expected_len` during `read_to_end` is amortised and
/// bounded by the actual stream content rather than by an integer in
/// the index header.
fn decompress_payload(src: &[u8], expected_len: usize) -> io::Result<Vec<u8>> {
    let cap = expected_len.min(MAX_DECODED_LEN_PER_ENTRY as usize);
    let mut out = Vec::with_capacity(cap);
    let decoder = zstd::stream::read::Decoder::new(src)?;
    // `Read::take` bounds the decoded stream as well: a stream that
    // expands past the budget returns an error instead of growing the
    // allocation further.
    let limit = u64::from(MAX_DECODED_LEN_PER_ENTRY).saturating_add(1);
    let mut limited = decoder.take(limit);
    limited.read_to_end(&mut out)?;
    if out.len() as u64 > u64::from(MAX_DECODED_LEN_PER_ENTRY) {
        return Err(io::Error::other(
            "zstd payload expanded past per-entry decode budget",
        ));
    }
    Ok(out)
}

/// Convert a [`SystemTime`] into a UNIX-epoch nanosecond count
/// suitable for [`EntryMeta::source_mtime_ns`]. Pre-epoch times, which
/// do not occur in real corpora, map to 0.
#[must_use]
pub fn system_time_to_ns(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH).map_or_else(
        |_| 0,
        |d| {
            // u128 → i64 saturating cast; corpus timestamps are well
            // within i64::MAX nanoseconds for any plausible decade.
            let ns = d.as_nanos();
            i64::try_from(ns.min(i64::MAX as u128)).unwrap_or(i64::MAX)
        },
    )
}

/// Inverse of [`system_time_to_ns`], for debug printing and equality
/// checks. Negative values saturate to the epoch.
#[must_use]
pub fn ns_to_system_time(ns: i64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_nanos(ns.max(0) as u64)
}

/// Streaming archive builder.
///
/// Append entries via [`push_entry`] (or [`push_already_encoded`] for
/// pre-encoded payloads, or [`push_prebuilt`] for verbatim copies
/// from a previous archive); then finalise via [`finish`] which
/// writes header + index + payload to the supplied path atomically
/// (write-to-tmpfile-then-rename).
///
/// [`push_entry`]: ArchiveBuilder::push_entry
/// [`push_already_encoded`]: ArchiveBuilder::push_already_encoded
/// [`push_prebuilt`]: ArchiveBuilder::push_prebuilt
/// [`finish`]: ArchiveBuilder::finish
#[derive(Debug)]
pub struct ArchiveBuilder {
    flags: u32,
    /// Per-entry metadata accumulated so far. `payload_offset` is set
    /// when the entry is pushed (offsets are computed against the
    /// growing `payload` Vec).
    entries: Vec<EntryMeta>,
    /// Concatenated payload bytes (already encoded — zstd-compressed
    /// if `flags & FLAG_ZSTD`, raw otherwise).
    payload: Vec<u8>,
    /// zstd compression level used when `flags & FLAG_ZSTD` is set.
    /// 19 is the long-mode default that gives the best ratio on
    /// Aozora's text profile (~5:1 on UTF-8 input).
    zstd_level: i32,
}

impl ArchiveBuilder {
    /// New empty builder. `flags` is the bitwise OR of [`FLAG_ZSTD`]
    /// and/or [`FLAG_UTF8`]; both default off.
    #[must_use]
    pub fn new(flags: u32) -> Self {
        Self {
            flags,
            entries: Vec::new(),
            payload: Vec::new(),
            zstd_level: 19,
        }
    }

    /// Override the zstd compression level (1..=22). The default
    /// (19) is the highest compression whose build time stays at a few
    /// seconds for the full corpus on a modern CPU. Has no effect when
    /// [`FLAG_ZSTD`] is unset.
    pub fn zstd_level(&mut self, level: i32) -> &mut Self {
        self.zstd_level = level;
        self
    }

    /// Append one entry to the archive in construction. `bytes` is
    /// the *unencoded* source bytes (raw SJIS for `!FLAG_UTF8`
    /// archives, UTF-8 for `FLAG_UTF8` archives); the builder
    /// compresses if needed and computes the blake3 hash.
    ///
    /// `source_mtime_ns` and the computed hash flow into the on-disk
    /// index for incremental-pack diff.
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError::Decompress`] if zstd encoding fails;
    /// zstd does not fail on valid input.
    pub fn push_entry(
        &mut self,
        label: impl Into<String>,
        bytes: &[u8],
        source_mtime_ns: i64,
    ) -> Result<(), ArchiveError> {
        let source_blake3: [u8; 32] = blake3::hash(bytes).into();
        let decoded_len = u32::try_from(bytes.len()).expect("entry larger than u32 unsupported");
        let payload_offset_in_section = u64::try_from(self.payload.len())
            .expect("archive payload section larger than u64 unsupported");

        if self.flags & FLAG_ZSTD != 0 {
            use zstd::stream::copy_encode;
            let mut compressed = Vec::with_capacity(bytes.len() / 4);
            copy_encode(bytes, &mut compressed, self.zstd_level)
                .map_err(ArchiveError::Decompress)?;
            let payload_len =
                u32::try_from(compressed.len()).expect("compressed entry larger than u32");
            self.payload.extend_from_slice(&compressed);
            self.entries.push(EntryMeta {
                // payload_offset is fixed up to absolute file offset
                // in `finish`; until then it stores the offset within
                // the payload section.
                payload_offset: payload_offset_in_section,
                payload_len,
                decoded_len,
                source_mtime_ns,
                source_blake3,
                label: label.into(),
            });
        } else {
            self.payload.extend_from_slice(bytes);
            self.entries.push(EntryMeta {
                payload_offset: payload_offset_in_section,
                payload_len: decoded_len,
                decoded_len,
                source_mtime_ns,
                source_blake3,
                label: label.into(),
            });
        }
        Ok(())
    }

    /// Append an entry whose payload has already been encoded
    /// (compressed if the archive is `FLAG_ZSTD`, raw otherwise) by
    /// the caller — typically because the encode happened in a
    /// parallel work loop and only the assembly step is sequential.
    /// The caller has already computed `decoded_len` and the source
    /// hash; the builder records them unchanged.
    pub fn push_already_encoded(
        &mut self,
        label: &str,
        payload: &[u8],
        decoded_len: u32,
        source_mtime_ns: i64,
        source_blake3: [u8; 32],
    ) {
        let payload_offset_in_section = u64::try_from(self.payload.len())
            .expect("archive payload section larger than u64 unsupported");
        let payload_len =
            u32::try_from(payload.len()).expect("payload longer than u32 unsupported");
        self.payload.extend_from_slice(payload);
        self.entries.push(EntryMeta {
            payload_offset: payload_offset_in_section,
            payload_len,
            decoded_len,
            source_mtime_ns,
            source_blake3,
            label: label.to_owned(),
        });
    }

    /// Append an entry whose compressed/raw payload is *already*
    /// encoded. Used by the incremental-pack path to copy unchanged
    /// entries verbatim from a previous archive without
    /// recompressing.
    pub fn push_prebuilt(&mut self, meta: EntryMeta, payload: &[u8]) {
        let payload_offset_in_section = u64::try_from(self.payload.len())
            .expect("archive payload section larger than u64 unsupported");
        self.payload.extend_from_slice(payload);
        self.entries.push(EntryMeta {
            payload_offset: payload_offset_in_section,
            ..meta
        });
    }

    /// Finalise the archive: assemble header + index + payload and
    /// write atomically to `path`. The write goes to `path.tmp`
    /// first, then `rename(2)` to `path`, so a partially written
    /// archive never appears at `path`.
    ///
    /// Returns the total file size in bytes, for reporting compression
    /// ratios.
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError::Io`] on filesystem failures.
    pub fn finish(self, path: impl AsRef<Path>) -> Result<u64, ArchiveError> {
        let path = path.as_ref();

        // Compute index section size to know the absolute payload
        // offset (so each entry's `payload_offset` can be promoted
        // from "offset within payload section" to "offset within
        // file").
        let index_section_len: usize = self
            .entries
            .iter()
            .map(|e| INDEX_FIXED_LEN + e.label.len())
            .sum();
        let payload_section_offset = HEADER_LEN + index_section_len;

        // Pre-size the output vector so the final `fs::write` is one
        // sequential memcpy with no growth realloc.
        let total_len = payload_section_offset + self.payload.len();
        let mut out = Vec::with_capacity(total_len);

        // Header
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&self.flags.to_le_bytes());
        out.extend_from_slice(
            &u32::try_from(self.entries.len())
                .expect("entry count larger than u32 unsupported")
                .to_le_bytes(),
        );

        // Index
        for entry in &self.entries {
            let absolute_offset = u64::try_from(payload_section_offset)
                .expect("file offsets fit in u64")
                + entry.payload_offset;
            out.extend_from_slice(&absolute_offset.to_le_bytes());
            out.extend_from_slice(&entry.payload_len.to_le_bytes());
            out.extend_from_slice(&entry.decoded_len.to_le_bytes());
            out.extend_from_slice(&entry.source_mtime_ns.to_le_bytes());
            out.extend_from_slice(&entry.source_blake3);
            let label_len_u32 =
                u32::try_from(entry.label.len()).expect("label longer than u32 unsupported");
            out.extend_from_slice(&label_len_u32.to_le_bytes());
            out.extend_from_slice(entry.label.as_bytes());
        }

        // Payload
        out.extend_from_slice(&self.payload);
        debug_assert_eq!(out.len(), total_len, "archive layout invariant");

        let tmp = path.with_extension(format!(
            "{}.tmp",
            path.extension().and_then(|e| e.to_str()).unwrap_or("aozc")
        ));
        // Atomic-publish pattern: write to tmp -> fsync -> rename.
        // Without the fsync, a crash between `write` and `rename` —
        // or between `rename` and the kernel flushing pages — can
        // leave the target archive present-but-corrupt: the rename
        // is committed to the directory entry, but the file's data
        // pages haven't reached the block device. fsync forces the
        // pages out before the rename publishes the file, so a
        // post-rename crash leaves either the old archive or the
        // fully-written new one — never a half-written one.
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&out)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)?;
        Ok(total_len as u64)
    }
}

#[cfg(test)]
mod tests {
    use core::ptr;

    use super::*;

    fn roundtrip(flags: u32, entries: &[(&str, &[u8])]) -> Archive {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("corpus.aozc");
        let mut builder = ArchiveBuilder::new(flags);
        for (label, bytes) in entries {
            builder
                .push_entry(*label, bytes, 0)
                .expect("push_entry must succeed");
        }
        let _ = builder.finish(&path).expect("finish must succeed");
        Archive::open(&path).expect("open must succeed")
    }

    #[test]
    fn roundtrip_raw_sjis_one_entry() {
        let arc = roundtrip(0, &[("a.txt", b"hello world")]);
        assert!(!arc.is_zstd());
        assert!(!arc.is_utf8());
        assert_eq!(arc.len(), 1);
        let items: Vec<_> = arc.iter().filter_map(Result::ok).collect();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0, "a.txt");
        assert_eq!(items[0].1, b"hello world");
    }

    #[test]
    fn roundtrip_raw_utf8_three_entries() {
        let arc = roundtrip(
            FLAG_UTF8,
            &[
                ("a.txt", "あおい".as_bytes()),
                ("b/c.txt", "そら".as_bytes()),
                ("d.txt", "ぶん こ".as_bytes()),
            ],
        );
        assert!(arc.is_utf8());
        assert_eq!(arc.len(), 3);
        let items: Vec<_> = arc.iter().filter_map(Result::ok).collect();
        assert_eq!(items[0].0, "a.txt");
        assert_eq!(items[0].1, "あおい".as_bytes());
        assert_eq!(items[1].0, "b/c.txt");
        assert_eq!(items[1].1, "そら".as_bytes());
        assert_eq!(items[2].0, "d.txt");
        assert_eq!(items[2].1, "ぶん こ".as_bytes());
    }

    #[test]
    fn roundtrip_zstd_compressed_payload_decompresses() {
        let big = "あ".repeat(10_000); // ~30 KB highly compressible
        let arc = roundtrip(FLAG_ZSTD | FLAG_UTF8, &[("big.txt", big.as_bytes())]);
        assert!(arc.is_zstd());
        assert!(arc.is_utf8());
        // A payload shorter on disk than its decompressed length shows
        // that the builder compressed it under FLAG_ZSTD.
        let raw_payload = arc.raw_payload(0);
        let entry = &arc.entries()[0];
        assert!(
            (raw_payload.len() as u32) < entry.decoded_len,
            "zstd payload should be smaller than decoded length"
        );
        let items: Vec<_> = arc.iter().filter_map(Result::ok).collect();
        assert_eq!(items[0].1, big.as_bytes());
    }

    #[test]
    fn iter_borrowed_returns_zero_copy_slice_for_raw_archive() {
        // For a raw archive (no zstd), iter_borrowed must yield the
        // payload as `Borrowed(&[u8])` whose pointer is *inside* the
        // archive's in-memory buffer — confirming zero-copy.
        let arc = roundtrip(0, &[("a.txt", b"hello world")]);
        let items: Vec<_> = arc.iter_borrowed().filter_map(Result::ok).collect();
        assert_eq!(items.len(), 1);
        let (label, payload) = &items[0];
        assert_eq!(*label, "a.txt");
        match payload {
            ArchivePayload::Borrowed(slice) => {
                assert_eq!(*slice, b"hello world");
                // The slice's pointer must lie inside `arc.bytes`'s
                // buffer (the in-memory copy of the archive file).
                let archive_payload_ptr = arc.raw_payload(0).as_ptr();
                assert!(
                    ptr::eq(archive_payload_ptr, slice.as_ptr()),
                    "iter_borrowed must yield the archive's own buffer slice (zero-copy)"
                );
            }
            ArchivePayload::Decompressed(_) => panic!("raw archive must yield Borrowed"),
        }
    }

    #[test]
    fn iter_borrowed_decompresses_zstd_payload() {
        let big = "あ".repeat(2_000);
        let arc = roundtrip(FLAG_ZSTD | FLAG_UTF8, &[("big.txt", big.as_bytes())]);
        let items: Vec<_> = arc.iter_borrowed().filter_map(Result::ok).collect();
        let (label, payload) = &items[0];
        assert_eq!(*label, "big.txt");
        match payload {
            ArchivePayload::Decompressed(v) => assert_eq!(v, big.as_bytes()),
            ArchivePayload::Borrowed(_) => panic!("zstd archive must yield Decompressed"),
        }
    }

    #[test]
    fn archive_payload_as_bytes_is_uniform_across_variants() {
        let body = b"some bytes".to_vec();
        let raw_payload = ArchivePayload::Borrowed(&body);
        let owned_payload = ArchivePayload::Decompressed(body.clone());
        assert_eq!(raw_payload.as_bytes(), owned_payload.as_bytes());
        assert_eq!(raw_payload.len(), 10);
        assert!(!raw_payload.is_empty());
    }

    #[test]
    fn archive_payload_into_owned_returns_vec_for_both_variants() {
        let raw_payload = ArchivePayload::Borrowed(b"abc");
        let owned_payload = ArchivePayload::Decompressed(b"def".to_vec());
        assert_eq!(raw_payload.into_owned(), b"abc");
        assert_eq!(owned_payload.into_owned(), b"def");
    }

    #[test]
    fn iter_and_iter_borrowed_yield_byte_equal_outputs() {
        // `iter` is implemented on `iter_borrowed`; both must yield
        // byte-for-byte the same payloads.
        let arc = roundtrip(
            FLAG_UTF8,
            &[("a.txt", "あおい".as_bytes()), ("b.txt", "そら".as_bytes())],
        );
        let owned: Vec<_> = arc.iter().filter_map(Result::ok).collect();
        let borrowed: Vec<_> = arc
            .iter_borrowed()
            .filter_map(Result::ok)
            .map(|(label, payload)| (label.to_owned(), payload.into_owned()))
            .collect();
        assert_eq!(owned, borrowed);
    }

    #[test]
    fn empty_archive_roundtrips() {
        let arc = roundtrip(0, &[]);
        assert_eq!(arc.len(), 0);
        assert!(arc.is_empty());
        assert!(arc.iter().next().is_none());
    }

    #[test]
    fn bad_magic_rejected() {
        let buf = vec![b'X'; 32];
        match Archive::from_bytes(buf) {
            Err(ArchiveError::BadMagic) => {}
            other => panic!("expected BadMagic, got {other:?}"),
        }
    }

    #[test]
    fn unsupported_version_rejected() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&999u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        match Archive::from_bytes(buf) {
            Err(ArchiveError::UnsupportedVersion(999)) => {}
            other => panic!("expected UnsupportedVersion(999), got {other:?}"),
        }
    }

    #[test]
    fn truncated_header_rejected() {
        let buf = vec![b'A', b'O']; // less than 16 bytes
        match Archive::from_bytes(buf) {
            Err(ArchiveError::Truncated { what: "header" }) => {}
            other => panic!("expected Truncated header, got {other:?}"),
        }
    }

    #[test]
    fn entry_metadata_records_blake3() {
        let arc = roundtrip(0, &[("a.txt", b"hello world")]);
        let expected: [u8; 32] = blake3::hash(b"hello world").into();
        assert_eq!(arc.entries()[0].source_blake3, expected);
    }

    #[test]
    fn entry_metadata_records_mtime() {
        // Built manually so the test sets the mtime.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("corpus.aozc");
        let mut b = ArchiveBuilder::new(0);
        b.push_entry("a.txt", b"hi", 12_345_678_900).unwrap();
        b.finish(&path).unwrap();
        let arc = Archive::open(&path).unwrap();
        assert_eq!(arc.entries()[0].source_mtime_ns, 12_345_678_900);
    }

    #[test]
    fn push_prebuilt_carries_through_to_iter() {
        // First archive: real entry with zstd.
        let dir = tempfile::tempdir().expect("tempdir");
        let path1 = dir.path().join("a.aozc");
        let path2 = dir.path().join("b.aozc");
        let body = "あ".repeat(2000);

        let mut b1 = ArchiveBuilder::new(FLAG_ZSTD | FLAG_UTF8);
        b1.push_entry("doc.txt", body.as_bytes(), 1).unwrap();
        b1.finish(&path1).unwrap();
        let arc1 = Archive::open(&path1).unwrap();

        // Second archive: copy the prebuilt entry verbatim into a
        // fresh builder (the incremental-pack code path).
        let mut b2 = ArchiveBuilder::new(FLAG_ZSTD | FLAG_UTF8);
        let meta = arc1.entries()[0].clone();
        let raw = arc1.raw_payload(0).to_vec();
        b2.push_prebuilt(meta, &raw);
        b2.finish(&path2).unwrap();

        let arc2 = Archive::open(&path2).unwrap();
        assert_eq!(
            arc2.entries()[0].source_blake3,
            arc1.entries()[0].source_blake3
        );
        let items: Vec<_> = arc2.iter().filter_map(Result::ok).collect();
        assert_eq!(items[0].0, "doc.txt");
        assert_eq!(items[0].1, body.as_bytes());
    }

    // Parse-time bounds checks: a hostile or corrupted archive header is
    // rejected before any allocation sized by it. The bounds are
    // documented on `MAX_DECODED_LEN_PER_ENTRY` and
    // `ArchiveError::InvalidSize`.

    /// A header that claims more entries than the file could possibly
    /// hold (each index record is at least `INDEX_FIXED_LEN` bytes)
    /// must be rejected at parse time. Without the cap, the parser
    /// would allocate `Vec::with_capacity(count)` worth of `EntryMeta`
    /// (each ≥ 80 bytes) on a small file and OOM.
    #[test]
    fn rejects_unreasonable_entry_count() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes()); // flags
        // Claim 4 billion entries in a 16-byte file.
        buf.extend_from_slice(&u32::MAX.to_le_bytes());
        match Archive::from_bytes(buf) {
            Err(ArchiveError::InvalidSize { what, value }) => {
                assert_eq!(what, "entry count");
                assert_eq!(value, u64::from(u32::MAX));
            }
            other => panic!("expected InvalidSize entry count, got {other:?}"),
        }
    }

    /// Edge case: the largest count that fits in the file. The parser
    /// must not reject a count at this exact boundary, so the cap
    /// rejects only counts the file cannot hold.
    #[test]
    fn accepts_count_exactly_at_ceiling() {
        // file_len = HEADER + 1 * INDEX_FIXED_LEN, claim count = 1.
        // The label_len field will be 0 so this constructs a valid
        // single-entry archive with empty payload.
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes()); // count = 1
        // One full INDEX_FIXED_LEN record, all zeros (label_len=0).
        // payload_offset = HEADER_LEN + INDEX_FIXED_LEN — points
        // past the file but payload_len = 0 so the slice is empty.
        let payload_offset = (HEADER_LEN + INDEX_FIXED_LEN) as u64;
        buf.extend_from_slice(&payload_offset.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes()); // payload_len
        buf.extend_from_slice(&0u32.to_le_bytes()); // decoded_len
        buf.extend_from_slice(&0i64.to_le_bytes()); // mtime
        buf.extend_from_slice(&[0u8; 32]); // blake3
        buf.extend_from_slice(&0u32.to_le_bytes()); // label_len = 0
        let arc = Archive::from_bytes(buf).expect("valid count-at-ceiling archive");
        assert_eq!(arc.len(), 1);
        assert_eq!(arc.entries()[0].label, "");
        assert_eq!(arc.entries()[0].payload_len, 0);
    }

    /// A header field declaring a `decoded_len` above the budget must be
    /// rejected at parse time, before `decompress_payload` can attempt
    /// a 4 GB allocation.
    #[test]
    fn rejects_unreasonable_decoded_len_at_parse_time() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        buf.extend_from_slice(&FLAG_ZSTD.to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes()); // count = 1
        buf.extend_from_slice(&0u64.to_le_bytes()); // payload_offset
        buf.extend_from_slice(&10u32.to_le_bytes()); // payload_len
        // decoded_len: 1 GB — past the 256 MiB budget.
        buf.extend_from_slice(&(1u32 << 30).to_le_bytes());
        buf.extend_from_slice(&0i64.to_le_bytes());
        buf.extend_from_slice(&[0u8; 32]);
        buf.extend_from_slice(&0u32.to_le_bytes());
        // Payload bytes, so that the declared payload range lies within
        // the file.
        buf.resize(buf.len() + 100, 0);
        match Archive::from_bytes(buf) {
            Err(ArchiveError::InvalidSize { what, value }) => {
                assert_eq!(what, "decoded_len");
                assert_eq!(value, u64::from(1u32 << 30));
            }
            other => panic!("expected InvalidSize decoded_len, got {other:?}"),
        }
    }

    /// `MAX_DECODED_LEN_PER_ENTRY` exactly is fine; one byte over is
    /// not. Boundary test for the parse-time cap.
    #[test]
    fn decoded_len_at_max_passes_one_over_fails() {
        for (value, should_pass) in [
            (MAX_DECODED_LEN_PER_ENTRY, true),
            (MAX_DECODED_LEN_PER_ENTRY + 1, false),
        ] {
            let mut buf = Vec::new();
            buf.extend_from_slice(&MAGIC);
            buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
            buf.extend_from_slice(&FLAG_ZSTD.to_le_bytes());
            buf.extend_from_slice(&1u32.to_le_bytes());
            // payload_offset past the file, but payload_len = 0 so the
            // payload-end check is satisfied (offset + 0 ≤ file len).
            let payload_offset = (HEADER_LEN + INDEX_FIXED_LEN) as u64;
            buf.extend_from_slice(&payload_offset.to_le_bytes());
            buf.extend_from_slice(&0u32.to_le_bytes());
            buf.extend_from_slice(&value.to_le_bytes());
            buf.extend_from_slice(&0i64.to_le_bytes());
            buf.extend_from_slice(&[0u8; 32]);
            buf.extend_from_slice(&0u32.to_le_bytes());
            let result = Archive::from_bytes(buf);
            assert_eq!(
                result.is_ok(),
                should_pass,
                "decoded_len={value} should_pass={should_pass}, got {result:?}",
            );
        }
    }

    /// The `decompress_payload` helper caps the pre-allocation to
    /// `MAX_DECODED_LEN_PER_ENTRY` independently of the parse-time
    /// check. Unit test on the internal helper; no archive is
    /// constructed.
    #[test]
    fn decompress_payload_clamps_giant_expected_len() {
        // A valid empty zstd stream — frame magic + empty content.
        let empty_input: &[u8] = &[];
        let empty: Vec<u8> =
            zstd::encode_all(io::Cursor::new(empty_input), 1).expect("zstd encode");
        // Pass an absurd expected_len; the helper must clamp.
        let out = decompress_payload(&empty, usize::MAX).expect("decompress empty");
        assert!(out.is_empty(), "empty stream must decompress to empty");
    }

    /// A successful `finish` followed by `Archive::open` must yield
    /// the same bytes. This test passes on a healthy filesystem with or
    /// without the `sync_all` call in `finish`;
    /// `finish_does_not_leave_tmp_file_behind` covers the publish step.
    #[test]
    fn finish_produces_a_readable_archive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("corpus.aozc");
        let mut b = ArchiveBuilder::new(FLAG_UTF8);
        b.push_entry("a.txt", b"hello", 0).unwrap();
        b.finish(&path).unwrap();
        let arc = Archive::open(&path).unwrap();
        assert_eq!(arc.len(), 1);
        let payload = arc.payload_at(0).unwrap();
        assert_eq!(payload.as_bytes(), b"hello");
    }

    /// After a successful `finish`, the directory holds only the archive:
    /// `finish` renames the `.tmp` file it wrote onto the target path.
    #[test]
    fn finish_does_not_leave_tmp_file_behind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("corpus.aozc");
        let mut b = ArchiveBuilder::new(0);
        b.push_entry("a.txt", b"hi", 0).unwrap();
        b.finish(&path).unwrap();
        // A leftover `.tmp` would mean the rename did not run.
        let count = fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(
            count,
            1,
            "expected only the archive in {:?}, got {} entries",
            dir.path(),
            count,
        );
    }

    // The following tests fix the on-disk format and public surface.
    // Changing any value asserted here requires updating the
    // corresponding constant.

    #[test]
    fn archive_magic_is_aozc() {
        assert_eq!(
            MAGIC, *b"AOZC",
            "MAGIC bytes are part of the on-disk format; \
             changing them is a hard-incompatible bump",
        );
    }

    #[test]
    fn format_version_is_one() {
        assert_eq!(
            FORMAT_VERSION, 1,
            "bumping FORMAT_VERSION requires a parser arm for the previous version",
        );
    }

    #[test]
    fn header_layout_matches_the_documented_spec() {
        // Header: magic(4) + version(4) + flags(4) + count(4) = 16.
        assert_eq!(HEADER_LEN, 16);
        // Index record fixed prefix: payload_offset(8) + payload_len(4)
        //                          + decoded_len(4) + mtime_ns(8)
        //                          + blake3(32)     + label_len(4) = 60.
        assert_eq!(INDEX_FIXED_LEN, 60);
    }

    #[test]
    fn flag_bits_have_fixed_values() {
        // The FLAG bits are public, and third-party tooling can read them.
        assert_eq!(FLAG_ZSTD, 0b01);
        assert_eq!(FLAG_UTF8, 0b10);
    }

    #[test]
    fn max_decoded_len_is_256_mib() {
        // Aozora docs cap at ~10 MB; 256 MiB gives ~25× headroom.
        // Lowering this bound can reject legitimate large corpora;
        // raising it widens the allocation an archive header can request.
        assert_eq!(MAX_DECODED_LEN_PER_ENTRY, 256 * 1024 * 1024);
    }

    #[test]
    fn archive_error_variants_have_nonempty_display() {
        // A new variant must be added to this list, and external pattern
        // matches against `ArchiveError` reviewed. The list is an array,
        // not a `match`, so a variant missing from it still compiles.
        for err in [
            ArchiveError::Io(io::Error::other("x")),
            ArchiveError::BadMagic,
            ArchiveError::UnsupportedVersion(0),
            ArchiveError::Truncated { what: "x" },
            ArchiveError::Decompress(io::Error::other("x")),
            ArchiveError::BadLabel,
            ArchiveError::InvalidSize {
                what: "x",
                value: 0,
            },
        ] {
            // Each variant must have a non-empty Display impl.
            let display = format!("{err}");
            assert!(!display.is_empty(), "empty Display for {err:?}");
        }
    }

    #[test]
    fn format_constants_compose_to_expected_minimum_file_size() {
        // The smallest valid archive is 16 bytes (header only, zero
        // entries); the test fails if HEADER_LEN changes.
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(buf.len(), HEADER_LEN);
        let arc = Archive::from_bytes(buf).expect("minimal archive parses");
        assert_eq!(arc.len(), 0);
    }

    #[test]
    fn system_time_ns_roundtrip() {
        use core::convert::Infallible;
        let now = SystemTime::now();
        let ns = system_time_to_ns(now);
        let back = ns_to_system_time(ns);
        let drift = back
            .duration_since(now)
            .or_else(|e| Ok::<_, Infallible>(e.duration()))
            .unwrap();
        assert!(drift.as_nanos() < 100, "round-trip drift > 100 ns");
    }
}
