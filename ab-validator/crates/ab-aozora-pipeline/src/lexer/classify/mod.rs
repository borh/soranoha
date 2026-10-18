//! Classify stage: classify the pair-stage event stream into [`Node`] spans.
//!
//! Walks the cross-linked [`PairEvent`] stream produced by the pair stage and
//! produces a contiguous vector of [`ClassifiedSpan`] whose
//! `source_span` values tile every byte of the sanitized source
//! end-to-end, in byte-offset order.
//!
//! The span kinds are:
//!
//! * [`SpanKind::Plain`]: a run of text that carries no Aozora
//!   construct. Adjacent events with equal [`PlainProvenance`] are merged;
//!   recovery boundaries remain visible while the normalize stage emits all
//!   plain spans verbatim.
//! * [`SpanKind::Aozora`]: a classified Aozora construct, carrying the
//!   concrete [`Node`] that the normalize stage will replace
//!   with a PUA placeholder sentinel (see [`crate::INLINE_SENTINEL`] and the
//!   other sentinel constants).
//! * [`SpanKind::Newline`]: a `\n` in the sanitized text, kept as its
//!   own span kind because block-level annotations (normalize-stage block
//!   sentinel substitution) depend on line boundaries.
//!
//! ## Span-coverage invariant
//!
//! When `source.len() > 0`:
//!
//! 1. `spans[0].source_span.start == 0`
//! 2. `spans[i].source_span.end == spans[i + 1].source_span.start`
//! 3. `spans[last].source_span.end == source.len()`
//!
//! When `source.is_empty()`, `spans` is empty.
//!
//! The normalize stage relies on this invariant to emit `normalized` text
//! without ever re-scanning `source`.
//!
//! ## Recogniser layout
//!
//! Every recogniser is a narrow function that inspects a
//! `&[PairEvent]` slice (often one pair's `body_events`) plus the
//! sanitized source. The driver loop's `Classifier::try_recognize`
//! dispatches based on the leading event kind:
//!
//! * Ruby (`｜X《Y》` explicit, trailing-kanji implicit)
//! * Bracket annotations, dispatched on the body keyword:
//!   fixed keyword (`改ページ` / `地付き` / ...), kaeriten
//!   (`一`/`二`/... plus okurigana `（X）`), indent / align-end
//!   (`N字下げ` / `地からN字上げ`), sashie (`挿絵`), forward-ref
//!   bouten, forward-ref TCY, paired-container open / close, and
//!   an `Directive{Unknown}` catch-all.
//! * Gaiji: `※［＃...］` reference-mark + bracket combos.
//! * Double-angle quotation `≪…≫` (displayed as `《…》`) (`AngleQuote`).
//!
//! The catch-all makes every well-formed `［＃…］` bracket produce
//! a `Node`, so the Tier-A canary (no bare `［＃` in the
//! HTML output outside an `aozora-directive` wrapper) holds regardless
//! of which specialised recogniser recognises the bracket.
//!
//! ## Inlining note
//!
//! Recogniser-wrapping dispatchers rely on compiler inlining at `-O3`
//! with LTO. Forcing `#[inline]` on dispatch wrappers increases code size
//! and degrades instruction-cache locality without throughput benefit.

#[cfg(feature = "classify-instrument")]
use super::instrumentation::{
    Subsystem, SubsystemGuard, YieldKind, record_pending_size, record_replay_body_size,
    record_yield,
};
use ab_aozora_encoding::gaiji::{gaiji_resolutions, parse_gaiji_body};
use ab_aozora_syntax::ast::KuntenKind;
use core::mem;
use core::ops::Range;
use std::collections::{BTreeMap, VecDeque};

// The classify stage builds the owned AST directly via `Allocator`'s
// inherent methods (each string interned once); the produced `Node`s are
// stored in the lex output's `NodeStore`.
use ab_aozora_syntax::alloc::Allocator;
use ab_aozora_syntax::ast::{
    Content, Directive, DirectivePairPayload, IterationMark, MarginNote, Node, NonEmptySpan,
    Segment,
};
use ab_aozora_syntax::{
    DirectiveKind, ForwardAttr, ForwardOrigin, MarginNoteKind, RegionClose, RegionFormat,
    RubyBaseClass, Span, is_ruby_base_char, ruby_base_class,
};

use super::pair::{PairEvent, PairKind};
use super::token::TriggerKind;
use ab_aozora_spec::Diagnostic;

mod directive;
mod formatting_instruction;
mod forward;
pub(crate) use forward::encoded_retrospective_gaiji_target;
mod gaiji;
mod kaeriten;
pub use directive::build_body_dispatcher;
pub(crate) use directive::prewarm;
use forward::install_forward_target_index_from_source;
use kaeriten::{KaeritenObs, classify_kaeriten_mark, family_index, looks_like_kana_prose};

/// One classified slice of the sanitized source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedSpan {
    /// What the slice is (plain run, newline, or a concrete Aozora
    /// construct / container marker). Selects which sentinel, if any,
    /// the normalizer emits.
    pub kind: SpanKind,
    /// Half-open sanitized-source byte range this slice covers.
    /// Consecutive spans tile the source contiguously (see the
    /// span-coverage invariant in the module docs).
    pub source_span: Span,
}

/// Classifier-local origin of bytes emitted verbatim as plain source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlainProvenance {
    /// Ordinary source text accepted as visible prose.
    Text,
    /// Source syntax preserved byte-exactly after recognition declined.
    RecoveredVerbatim,
}

/// Metadata carried by a plain classified span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlainSpan {
    /// Whether the bytes are ordinary text or byte-exact recovery.
    pub provenance: PlainProvenance,
}

/// Classification of a [`ClassifiedSpan`].
///
/// The normalize stage (the `Normalizer` walk in `crate::lex`) maps the
/// variants to PUA sentinels as follows:
///
/// | variant        | sentinel              | `post_process` role |
/// |----------------|-----------------------|-------------------|
/// | `Plain`        | verbatim source bytes | — |
/// | `Newline`      | verbatim `\n`         | — |
/// | `Aozora(n)`    | `E001` if inline, `E002` if block-leaf | splice Aozora node into the AST |
/// | `BlockOpen`    | `E003`                | pair with matching `BlockClose` |
/// | `BlockClose`   | `E004`                | close nearest unclosed `BlockOpen` |
///
/// The `BlockOpen` / `BlockClose` split exists because paired
/// containers (`ここから字下げ` … `ここで字下げ終わり`) span arbitrary
/// content between the two markers. The lexer emits both markers as
/// independent spans and lets `post_process` walk the AST to wrap
/// sibling nodes in the container.
///
/// # Memory layout
///
/// The `Aozora(Node)` variant is not boxed;
/// `Node` is `Copy` and small (a handful of machine words), so
/// storing it inline keeps `SpanKind` to `Aozora`-variant size without
/// a `Box` indirection.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpanKind {
    /// Source bytes that carry no Aozora construct. Emitted verbatim
    /// by the normalizer.
    Plain(PlainSpan),
    /// Classified Aozora construct (inline span or block-leaf line).
    /// The normalizer replaces the source span with an `E001` (inline)
    /// or `E002` (block-leaf) sentinel and records the node in the
    /// placeholder registry keyed at the sentinel's normalized
    /// position.
    Aozora(Node),
    /// Paired-container opener: `［＃ここから字下げ］`, `［＃罫囲み］`,
    /// etc. The normalizer emits an `E003` sentinel line; `post_process`
    /// matches it to the corresponding `BlockClose` via a balanced
    /// stack walk of the AST.
    BlockOpen(RegionFormat),
    /// Paired-container closer: `［＃ここで字下げ終わり］`,
    /// `［＃罫囲み終わり］`, etc. The normalizer emits an `E004`
    /// sentinel line; the carried [`RegionClose`] is a hint used by
    /// `post_process` to diagnose `［＃罫囲み終わり］` closing an
    /// `Indent` opener (kind mismatch).
    BlockClose(RegionClose),
    /// One supplied marker identifies two adjacent scopes, in closing order.
    BlockCloses {
        /// Supplied targets in inner-to-outer closing order.
        targets: [RegionClose; 2],
        /// Exact directive retained when the complete boundary cannot be established.
        fallback: Node,
    },
    /// A `\n` in the sanitized text. Retained as its own span kind
    /// because block-level recognizers need line boundaries.
    Newline,
}

/// Classify a streaming pair-stage [`PairEvent`] iterator against the
/// sanitized source.
///
/// Returns a [`ClassifyStream`] iterator yielding one [`ClassifiedSpan`]
/// per call to [`Iterator::next`]. After exhaustion, call
/// [`ClassifyStream::take_diagnostics`] to drain non-fatal observations
/// accumulated during recognition. The upstream pair stream's
/// diagnostics are not forwarded automatically; the caller is
/// responsible for calling `pair_stream.take_diagnostics()` after the
/// classify stream is dropped (the fused pipeline in `aozora-pipeline` does
/// this).
///
/// Pure function; no I/O. The yielded spans byte-contiguously cover
/// `source`; see the module-level span-coverage invariant.
#[must_use]
pub fn classify<'src, 'al, I>(
    events: I,
    source: &'src str,
    alloc: &'al mut Allocator,
) -> ClassifyStream<'src, 'al, I::IntoIter>
where
    I: IntoIterator<Item = PairEvent>,
{
    // Pre-pass: scan raw source bytes for `「…」` quote bodies and
    // record the first byte position of each unique body. The streaming
    // pipeline does not materialise a `Vec<PairEvent>`, so the index is
    // built from source bytes, at the cost of one extra `memmem` sweep
    // per document before classification starts. The index is installed
    // only when the source has enough quote bodies to amortise the build:
    // the median corpus doc skips it, and on the annotation-dense
    // 252-occurrence doc an index of the same forward-reference targets
    // reduced classify time from 170 ms to 20 ms.
    install_forward_target_index_from_source(source);
    let (detached_targets, detached_markers) = detached_gaiji_index(source);
    ClassifyStream::new(
        events.into_iter(),
        source,
        alloc,
        detached_targets,
        detached_markers,
    )
}

#[derive(Clone)]
struct DetachedGaijiSpec {
    description: String,
    quoted: bool,
    mencode: Option<String>,
    marker: Span,
}

fn detached_gaiji_index(
    source: &str,
) -> (
    BTreeMap<u32, DetachedGaijiSpec>,
    BTreeMap<u32, DetachedGaijiSpec>,
) {
    let mut targets = BTreeMap::new();
    let mut markers = BTreeMap::new();
    for resolution in gaiji_resolutions(source) {
        if !source[resolution.start..].starts_with("［＃") {
            continue;
        }
        let before = &source[..resolution.start];
        let target = if resolution.description == "さんずい＋自"
            && before.ends_with("※芙藍")
            && source[resolution.end..].starts_with("《さふらん》")
        {
            Some(resolution.start - "※芙藍".len())
        } else if !resolution.description.starts_with("「※」は") {
            None
        } else if before.ends_with("※々") {
            Some(resolution.start - "※々".len())
        } else {
            let line_start = before.rfind('\n').map_or(0, |position| position + 1);
            before[line_start..]
                .rfind('※')
                .map(|relative| line_start + relative)
                .filter(|&position| {
                    let gap = &source[position + '※'.len_utf8()..resolution.start];
                    gap.starts_with('《')
                        && gap.ends_with('》')
                        && !gap['《'.len_utf8()..gap.len() - '》'.len_utf8()]
                            .contains(['《', '》', '\n'])
                })
        };
        let Some(target) = target else {
            continue;
        };
        if source[..target].ends_with('｜') {
            continue;
        }
        let body = &source[resolution.start + "［＃".len()..resolution.end - "］".len()];
        let spec = DetachedGaijiSpec {
            description: resolution.description,
            quoted: parse_gaiji_body(body).quoted,
            mencode: resolution.mencode,
            marker: Span::new(
                u32::try_from(resolution.start).expect("source offset fits native span"),
                u32::try_from(resolution.end).expect("source offset fits native span"),
            ),
        };
        targets.insert(
            u32::try_from(target).expect("source offset fits native span"),
            spec.clone(),
        );
        markers.insert(spec.marker.start, spec);
    }
    (targets, markers)
}

/// Streaming classify-stage classifier.
///
/// Owns the upstream [`PairEvent`] iterator and consumes it lazily,
/// yielding one [`ClassifiedSpan`] per [`Iterator::next`] call. The
/// classifier maintains its own per-pair frame stack: when a top-level
/// `PairOpen` arrives, all subsequent events accumulate into a smallvec
/// body buffer until the matching `PairClose`; recognition then runs
/// against the buffer and yields a single span (or, in the rare
/// gaiji+ref-mark case, consumes a buffered `Solo(RefMark)` from the
/// previous emission and folds it into the bracket span).
///
/// State:
/// * `pending_outputs`: queue of complete `ClassifiedSpan`s waiting to
///   be returned by `next()`. A single input event can produce multiple
///   outputs (e.g. flush a pending Plain run + emit a recognised span);
///   draining this queue first keeps `next` simple.
/// * `frame`: current outermost open frame, if any. Inside a frame all
///   incoming events are appended to the body buffer; nested
///   `PairOpen`/`PairClose` adjust the buffer-local stack so `close_idx`
///   slots can be patched and the outer pair can be detected as
///   "matching close at depth 0".
/// * `pending_plain`: adjacent provenance segments for the current top-level
///   plain run. Equal adjacent provenance merges before emission.
/// * `pending_refmark`: a top-level `Solo(RefMark)` waiting to be
///   consumed by the next `PairOpen(Bracket)` (gaiji shape). If the
///   following event is anything else the refmark is folded into the
///   pending Plain run.
/// * `diagnostics`: non-fatal observations accumulated during the pass.
#[allow(
    missing_debug_implementations,
    reason = "the &mut Allocator field cannot derive Debug; the iterator is opaque to consumers"
)]
pub struct ClassifyStream<'src, 'al, I>
where
    I: Iterator<Item = PairEvent>,
{
    events: I,
    source: &'src str,
    source_len: u32,
    alloc: &'al mut Allocator,
    /// Buffered ready-to-yield spans drained one-per-`next()` by the
    /// consumer. `VecDeque` (not `SmallVec`) because the consumer pulls
    /// from the front and `SmallVec::remove(0)` is `O(N)`: the
    /// `replay_unrecognised_body` path can push thousands of spans at
    /// once for top-level unrecognised paired containers (e.g. doc 49178
    /// in the corpus emits ~16k pending spans), and the per-yield
    /// front-pop becomes quadratic in memmove. `VecDeque` is a ring
    /// buffer with `O(1)` `push_back` / `pop_front` and no back-shift.
    pending_outputs: VecDeque<ClassifiedSpan>,
    frame: Option<Frame>,
    /// Stream-through state for top-level pair kinds that have no
    /// recogniser (Quote, Tortoise, or a bracket without `＃`). When
    /// `Some((kind, depth))`, `process_event` bypasses frame buffering:
    /// events stream directly through `handle_stream_event`, with no
    /// `SmallVec` push per event and no replay walk. The depth counter
    /// tracks nested opens of the same kind so the outer close is
    /// unambiguous.
    ///
    /// Doc 49178 (corpus outlier) wraps ~24k inner events in two
    /// top-level Quote pairs; buffering would push each event to the
    /// body smallvec and read it back in replay, where stream-through
    /// walks forward once.
    streaming: Option<StreamingFrame>,
    pending_plain: VecDeque<PendingPlain>,
    pending_refmark: Option<Span>,
    /// A just-recognised gaiji held back one step so an immediately-
    /// following `《…》` ruby can take it as its base (`※［＃…］《みは》`).
    /// A gaiji resolves to a glyph distinct from its `※［＃…］` source and
    /// is emitted as its own node, so a ruby cannot attach to it once it
    /// is yielded; the emit is deferred here instead. Flushed as a
    /// standalone gaiji span by `process_event` / `finalize` when the next
    /// event is not the adjacent ruby that would consume it.
    pending_ruby_base: Option<PendingRubyBase>,
    detached_targets: BTreeMap<u32, DetachedGaijiSpec>,
    detached_markers: BTreeMap<u32, DetachedGaijiSpec>,
    diagnostics: Vec<Diagnostic>,
    /// Bracketed kaeriten observed in document order, drained by
    /// [`Self::finalize_kaeriten`] in [`Self::take_diagnostics`] for the
    /// document-wide base-presence pairing check and the outside-kanbun
    /// heuristic.
    kaeriten_obs: Vec<KaeritenObs>,
    finished: bool,
}

/// Active stream-through frame for a top-level Quote / Tortoise pair.
/// Carries no event buffer: only the outer pair kind and a nested-
/// open depth counter. Each `PairOpen` of the same kind increments,
/// each `PairClose` of the same kind decrements; reaching zero ends
/// stream-through.
#[derive(Debug, Clone, Copy)]
struct StreamingFrame {
    kind: PairKind,
    depth: u32,
}

#[derive(Debug, Clone, Copy)]
struct PendingPlain {
    source_span: Span,
    provenance: PlainProvenance,
}

/// Outcome of feeding one event into the active body-buffer frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameStep {
    /// The frame is still accumulating body events.
    Open,
    /// The outermost pair just closed: run recognition on the body.
    Closed,
    /// The pair stage force-expired the frame's own open (newline expiry
    /// of a line-spanning `［`, or the EOF drain): replay the body as
    /// plain and return to top-level classification.
    Abandoned,
}

/// A gaiji segment, the span of any `｜` run before its ruby-base prefix, and
/// the span of that prefix.
type GaijiEmit = (PendingBaseSegment, Option<Span>, Option<Span>);

/// A deferred native construct and its original standalone source span.
/// A following ruby adopts the segment; otherwise the span is replayed.
struct PendingBaseSegment {
    span: ClassifiedSpan,
    payload: Segment,
}

/// A contiguous ruby base containing native constructs and intervening text.
/// Gaiji can establish an implicit base, and so can a supplied-diacritic note
/// together with the letter run it reclaims when a reading follows the note
/// directly; other annotations require an explicit `｜`, so that ordinary
/// annotations do not create a ruby attachment.
///
/// `prefix` holds leading base text; `bar` holds its explicit marker outside
/// the preceding plain run. Adoption consumes the marker while retaining its
/// source span. Without a reading, every held span is replayed in source order.
struct PendingRubyBase {
    segs: smallvec::SmallVec<[PendingBaseSegment; 2]>,
    bar: Option<Span>,
    prefix: Option<Span>,
}

impl PendingRubyBase {
    fn single(segment: PendingBaseSegment, bar: Option<Span>, prefix: Option<Span>) -> Self {
        Self {
            segs: smallvec::smallvec![segment],
            bar,
            prefix,
        }
    }

    /// Whether `text` is the space that separates a base ending in a single
    /// gaiji from its reading, as in `※［＃「足へん＋宛」、第3水準1-92-36］ 《もが》`.
    fn spaced_from_reading(&self, text: &str) -> bool {
        self.bar.is_none()
            && matches!(
                self.segs[..],
                [PendingBaseSegment {
                    payload: Segment::Gaiji { .. },
                    ..
                }]
            )
            && !text.is_empty()
            && text.chars().all(|ch| matches!(ch, ' ' | '\u{3000}'))
    }

    /// Source offset where the base starts, including a held marker or prefix.
    fn start(&self) -> u32 {
        self.bar
            .or(self.prefix)
            .map_or(self.segs[0].span.source_span.start, |b| b.start)
    }

    /// Whether text or a further construct after the base can extend it. A
    /// supplied-diacritic, illustration or noted-target base without `｜` ends
    /// at its annotation.
    fn extensible(&self) -> bool {
        self.bar.is_some()
            || self.segs.iter().any(|segment| {
                !matches!(
                    segment.payload,
                    Segment::Format { .. }
                        | Segment::Illustration { .. }
                        | Segment::MarginNote { .. }
                )
            })
    }

    /// End of the last construct, anchoring adjacent text and annotations.
    fn end(&self) -> u32 {
        self.segs
            .last()
            .expect("a deferred ruby base contains a native construct")
            .span
            .source_span
            .end
    }
}

/// Outcome of [`ClassifyStream::try_ruby_over_pending_base`].
enum PendingBaseRuby {
    /// No adjacent deferred base: continue to the plain-base path.
    NotApplicable,
    /// A ruby adopted the deferred base.
    Emitted(ClassifiedSpan),
    /// The empty or malformed reading left the base as standalone spans.
    Declined,
}

/// Body window passed to recogniser helpers.
///
/// `events` is a contiguous body slice (between matched
/// `PairOpen`/`PairClose`); `links[i]` gives the body-local index of
/// the matching `PairOpen`/`PairClose` for `events[i]` if it's a
/// paired event (`u32::MAX` otherwise). Both slices are the same
/// length and are constructed by [`ClassifyStream`]'s frame buffers.
///
/// The split keeps [`PairEvent`] free of cross-link fields (the pair stage
/// can stream events one-at-a-time without back-patching) while still
/// giving recogniser helpers O(1) access to the matching delimiter via
/// the parallel side-table.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BodyView<'b> {
    pub events: &'b [PairEvent],
    pub links: &'b [u32],
}

/// Per-recognise-call shared context.
///
/// Bundles the (allocator, sanitized source) pair that every classify-stage
/// recogniser / classifier helper needs but doesn't vary per call
/// within a single recognise pass. Threading it as a single `&mut`
/// argument keeps each helper's signature at ≤4 args (project-rule
/// `clippy.toml::too-many-arguments-threshold = 4`) without losing
/// the per-call positional clarity of the body-window indices.
///
/// The two lifetimes stay distinct:
/// - `'al`: the borrow lifetime of the `&mut alloc` reference
/// - `'s`: the sanitized source lifetime
///
/// Strings the recognisers intern are owned by the allocator's
/// `NodeStore` (not borrowed from the source); keeping `'al` and `'s`
/// separate avoids over-constraining helpers that pass synthetic
/// source slices through `Cow`.
pub(crate) struct RecogniseCtx<'al, 's> {
    pub alloc: &'al mut Allocator,
    pub source: &'s str,
    /// Non-fatal diagnostics raised while building nested content
    /// (a gaiji inside a ruby / annotation reading). The owning
    /// `ClassifyStream` drains this into its own sink after each
    /// recognise call: a `RecogniseCtx` is a short-lived per-call view,
    /// so an owned `Vec` avoids threading a `&mut` sink (and a fourth
    /// lifetime) through every recogniser.
    pub diagnostics: Vec<Diagnostic>,
    /// Byte offset where the enclosing top-level pending plain run began,
    /// or `None` when no plain run is open (or this is a nested-content
    /// view, where forward references never resolve). Read by the forward
    /// recognizers to locate a non-adjacent referent inside that run and
    /// splice a styled decoration at it.
    pub pending_plain_start: Option<u32>,
    /// Source-owned selected leaves and their disjoint spans in the pending plain run.
    /// `try_bracket_emit` validates the complete selection before splicing any leaf.
    /// Empty when the recognizer has not established an interior selection.
    pub pending_decorations: Vec<(Node, Span)>,
}

/// One outermost open-pair frame currently being buffered.
///
/// `body` holds every event seen between the open and the matching
/// close (inclusive of nested Pair events). The parallel `links`
/// smallvec records, for each entry of `body`, the body-local index
/// of the matching `PairOpen` / `PairClose` (or `u32::MAX` for
/// non-paired entries and unmatched delimiters). The recognise
/// helpers (`recognize_ruby` / `recognize_annotation` /
/// `recognize_gaiji` / `try_angle_quote`) consume the buffer as a
/// [`BodyView`] with `open_idx = 0` and `close_idx = body.len() - 1`.
///
/// `inner_stack` tracks the per-buffer mini-stack of nested opens:
/// `(kind, body_index)`, so that each nested close can locate the
/// matching open in the body buffer and patch the `links` table.
struct Frame {
    body: smallvec::SmallVec<[PairEvent; 16]>,
    links: smallvec::SmallVec<[u32; 16]>,
    inner_stack: smallvec::SmallVec<[(PairKind, usize); 8]>,
    /// `true` when the outer open follows a `Solo(RefMark)` and the
    /// frame should be recognised as gaiji rather than a generic
    /// bracket annotation.
    gaiji_refmark: Option<Span>,
}

/// Recognize visible reference-sign forms; an unexplained prose sign stays opaque.
fn literal_reference_sign(source: &str, span: Span) -> bool {
    let before = &source[..span.start as usize];
    let after = &source[span.end as usize..];
    if before.ends_with('※') || after.starts_with(['※', '［', '[']) {
        return false;
    }
    // A pending explicit ruby base needs ownership of the sign as well as its
    // following text; accepting the sign alone would hide a truncated base.
    if before
        .chars()
        .rev()
        .find(|ch| matches!(ch, '｜' | '》' | '\n'))
        == Some('｜')
    {
        return false;
    }
    if after.starts_with("記号") || after.starts_with("番号") {
        return true;
    }
    if after.starts_with(|ch: char| ch.is_ascii_digit() || ('０'..='９').contains(&ch)) {
        return true;
    }
    let numbered =
        after.trim_start_matches(['一', '二', '三', '四', '五', '六', '七', '八', '九', '十']);
    if numbered.len() != after.len()
        && before.ends_with(['（', '(', '［', '['])
        && numbered.starts_with(['）', ')', '］', ']'])
    {
        return true;
    }
    before
        .rsplit('\n')
        .next()
        .is_some_and(|line| line.trim().is_empty())
        && after
            .split('\n')
            .next()
            .is_some_and(|line| line.trim().is_empty())
}

/// Span of the first ruby (`《…》`) opening inside the body
/// event range `lo..hi`, if any. Used by [`ClassifyStream::try_ruby_emit`]
/// to flag `nested_ruby`: `build_content_from_body` folds only nested
/// gaiji / annotation, so an inner ruby open would otherwise remain raw text.
fn first_nested_ruby_open(events: &[PairEvent], lo: usize, hi: usize) -> Option<Span> {
    events[lo..hi].iter().find_map(|e| match e {
        PairEvent::PairOpen {
            kind: PairKind::Ruby,
            span,
        } => Some(*span),
        _ => None,
    })
}

/// When an explicit-base ruby (`｜base《》`) has an empty reading, the span
/// of the whole `｜base《》` to flag as `empty_ruby_reading`. Returns `None`
/// when there is no explicit `｜` base (a bare `《》` is literal text)
/// or when the `《…》` reading is non-empty.
fn empty_explicit_ruby_span(
    bar_byte_offset: Option<usize>,
    preceding_start: u32,
    open_span: Span,
    close_span: Span,
) -> Option<Span> {
    let bar_off = bar_byte_offset?;
    if open_span.end < close_span.start {
        return None; // reading carries bytes: not empty
    }
    let bar_pos = preceding_start + u32::try_from(bar_off).ok()?;
    Some(Span::new(bar_pos, close_span.end))
}

/// The synthetic event stream, its parallel link table, and the index of
/// the synthetic `《` open, as returned by [`build_synth_ruby_view`]. Both
/// buffers are inline-16 `SmallVec`s, so a ruby (~5 events) allocates no
/// heap.
type SynthRubyView = (
    smallvec::SmallVec<[PairEvent; 16]>,
    smallvec::SmallVec<[u32; 16]>,
    usize,
);

/// Build the synthetic event / link stream `recognize_ruby` expects in the
/// streaming model: `[optional Solo(Bar), Text(base), ...body events...]`,
/// with links shifted to account for the prepended prefix. Returns the
/// stream, its parallel link table, and the index of the synthetic `《`
/// open. `prev_text_range` is the preceding plain run (`start` = its byte
/// offset, `end` = the `《` open); `bar_byte_offset` is the position of a
/// `｜` inside it for the explicit form. Returns `None` when an explicit
/// `｜` base would leave the base text empty.
///
/// The two buffers are `SmallVec<[_; 16]>`: a ruby is `[Text(base),
/// PairOpen, reading…, PairClose]` (~5 events), so the synth stays inline
/// and allocates no heap for the ~200 rubies per file that dominate the
/// parse; with `Vec::with_capacity`, these two buffers accounted for ~67%
/// of all owned-pipeline heap allocations (two mallocs per ruby). The `[16]` inline size matches
/// `Frame::body`, whose contents this is a superset of.
fn build_synth_ruby_view(
    body: BodyView<'_>,
    prev_text_range: Span,
    bar_byte_offset: Option<usize>,
) -> Option<SynthRubyView> {
    let mut synth: smallvec::SmallVec<[PairEvent; 16]> =
        smallvec::SmallVec::with_capacity(body.events.len() + 2);
    let mut synth_links: smallvec::SmallVec<[u32; 16]> =
        smallvec::SmallVec::with_capacity(body.events.len() + 2);
    let synth_open_idx = if let Some(bar_off) = bar_byte_offset {
        let bar_pos = prev_text_range.start + u32::try_from(bar_off).expect("bar offset fits");
        let bar_span = Span::new(bar_pos, bar_pos + u32::try_from('｜'.len_utf8()).unwrap());
        synth.push(PairEvent::Solo {
            kind: TriggerKind::Bar,
            span: bar_span,
        });
        synth_links.push(u32::MAX);
        if bar_span.end >= prev_text_range.end {
            return None; // explicit base would be empty
        }
        synth.push(PairEvent::Text {
            range: Span::new(bar_span.end, prev_text_range.end),
        });
        synth_links.push(u32::MAX);
        2
    } else {
        synth.push(PairEvent::Text {
            range: prev_text_range,
        });
        synth_links.push(u32::MAX);
        1
    };
    // Append the body events verbatim, shifting their links past the prefix.
    let shift = u32::try_from(synth.len()).expect("synth prefix fits u32");
    synth.extend(body.events.iter().cloned());
    synth_links.extend(
        body.links
            .iter()
            .map(|&l| if l == u32::MAX { u32::MAX } else { l + shift }),
    );
    Some((synth, synth_links, synth_open_idx))
}

impl<'src, 'al, I> ClassifyStream<'src, 'al, I>
where
    I: Iterator<Item = PairEvent>,
{
    fn new(
        events: I,
        source: &'src str,
        alloc: &'al mut Allocator,
        detached_targets: BTreeMap<u32, DetachedGaijiSpec>,
        detached_markers: BTreeMap<u32, DetachedGaijiSpec>,
    ) -> Self {
        Self {
            events,
            source,
            source_len: u32::try_from(source.len()).expect("sanitize asserts fit in u32"),
            alloc,
            pending_outputs: VecDeque::new(),
            frame: None,
            streaming: None,
            pending_plain: VecDeque::new(),
            pending_refmark: None,
            pending_ruby_base: None,
            detached_targets,
            detached_markers,
            diagnostics: Vec::new(),
            kaeriten_obs: Vec::new(),
            finished: false,
        }
    }

    /// Drain accumulated diagnostics. Should be called after the
    /// iterator is exhausted (otherwise the trailing Plain flush has
    /// not yet recorded any final-span observations).
    pub fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        self.finalize_kaeriten();
        mem::take(&mut self.diagnostics)
    }

    /// Document-final kaeriten checks, run once the event stream is
    /// exhausted: the document-wide base-presence pairing check
    /// (`bracketed_kaeriten_no_pair`) and the conservative
    /// outside-kanbun heuristic (`kaeriten_outside_kanbun`). Both work
    /// off `kaeriten_obs`, accumulated during classification.
    ///
    /// The pairing rule is document-wide base presence: a ladder mark
    /// of rank ≥ 2 is reported only when its family's base (`一` / `上` / `甲`)
    /// is absent from the entire document. The rule is calibrated against
    /// the 青空文庫 corpus: kanbun return-mark groups routinely span
    /// `、` / `。` and line boundaries, and 上下点 skips `中`, so a narrower
    /// scope or a stricter ladder reports valid kanbun (a strict per-clause
    /// rule: 586 false positives across 337 corpus files; this rule: 2). It
    /// matches the catalogue's literal wording: "a `［＃二］` with no `［＃一］`".
    fn finalize_kaeriten(&mut self) {
        let obs = mem::take(&mut self.kaeriten_obs);
        if obs.is_empty() {
            return;
        }
        // Conservative outside-kanbun heuristic: the document contains a
        // single kaeriten whose surroundings are kana prose, which
        // indicates a stray annotation rather than a 返り点.
        if let [only] = obs.as_slice()
            && looks_like_kana_prose(self.source, only.span)
        {
            self.diagnostics
                .push(Diagnostic::kaeriten_outside_kanbun(only.span));
        }
        // Document-wide base presence per ladder family.
        let mut has_base = [false; 3];
        for o in obs.iter().filter(|o| o.is_ladder && o.rank == 1) {
            has_base[family_index(o.family)] = true;
        }
        for o in obs.iter().filter(|o| o.is_ladder && o.rank > 1) {
            if !has_base[family_index(o.family)] {
                self.diagnostics
                    .push(Diagnostic::bracketed_kaeriten_no_pair(o.span));
            }
        }
    }

    fn push_output(&mut self, span: ClassifiedSpan) {
        #[cfg(feature = "classify-instrument")]
        record_yield(match &span.kind {
            SpanKind::Plain(PlainSpan {
                provenance: PlainProvenance::Text,
            }) => YieldKind::PlainText,
            SpanKind::Plain(PlainSpan {
                provenance: PlainProvenance::RecoveredVerbatim,
            }) => YieldKind::PlainRecoveredVerbatim,
            SpanKind::Newline => YieldKind::Newline,
            SpanKind::Aozora(_) => YieldKind::Aozora,
            SpanKind::BlockOpen(_) => YieldKind::BlockOpen,
            SpanKind::BlockClose(_) | SpanKind::BlockCloses { .. } => YieldKind::BlockClose,
        });
        self.pending_outputs.push_back(span);
    }

    fn pending_plain_start(&self) -> Option<u32> {
        self.pending_plain
            .front()
            .map(|plain| plain.source_span.start)
    }

    fn push_plain(&mut self, source_span: Span, provenance: PlainProvenance) {
        if source_span.start >= source_span.end {
            return;
        }
        if let Some(back) = self.pending_plain.back_mut()
            && back.provenance == provenance
            && back.source_span.end == source_span.start
        {
            back.source_span.end = source_span.end;
            return;
        }
        self.pending_plain.push_back(PendingPlain {
            source_span,
            provenance,
        });
    }

    fn fold_pending_refmark(&mut self) {
        if let Some(rm) = self.pending_refmark.take() {
            self.flush_plain_up_to(rm.start);
            let provenance = if literal_reference_sign(self.source, rm) {
                PlainProvenance::Text
            } else {
                PlainProvenance::RecoveredVerbatim
            };
            self.push_plain(rm, provenance);
            self.flush_plain_up_to(rm.end);
        }
    }

    /// Emit any pending top-level plain run whose end is `end`. The
    /// pending refmark, if any, is folded into the plain run's coverage
    /// (its span is contiguous with the surrounding text).
    fn flush_plain_up_to(&mut self, end: u32) {
        #[cfg(feature = "classify-instrument")]
        let _classify_guard = SubsystemGuard::new(Subsystem::FlushPlain);
        // A pending refmark contributes its bytes to the plain run.
        self.fold_pending_refmark();
        while self
            .pending_plain
            .front()
            .is_some_and(|plain| plain.source_span.start < end)
        {
            let mut plain = self.pending_plain.pop_front().expect("checked Some");
            let emitted_end = plain.source_span.end.min(end);
            self.emit_plain_run(
                Span::new(plain.source_span.start, emitted_end),
                plain.provenance,
            );
            if emitted_end < plain.source_span.end {
                plain.source_span.start = emitted_end;
                self.pending_plain.push_front(plain);
                break;
            }
        }
    }

    fn emit_plain_run(&mut self, span: Span, provenance: PlainProvenance) {
        let mut start = span.start;
        if provenance == PlainProvenance::Text {
            for (offset, _) in
                self.source[span.start as usize..span.end as usize].match_indices('／')
            {
                let position = span.start + u32::try_from(offset).expect("source fits native span");
                let Some(mark) =
                    IterationMark::at_start(&self.source[position as usize..span.end as usize])
                else {
                    continue;
                };
                if start < position {
                    self.push_output(ClassifiedSpan {
                        kind: SpanKind::Plain(PlainSpan { provenance }),
                        source_span: Span::new(start, position),
                    });
                }
                start = position
                    + u32::try_from(mark.source().len()).expect("notation fits native span");
                self.push_output(ClassifiedSpan {
                    kind: SpanKind::Aozora(Node::IterationMark(mark)),
                    source_span: Span::new(position, start),
                });
            }
        }
        if start < span.end {
            self.push_output(ClassifiedSpan {
                kind: SpanKind::Plain(PlainSpan { provenance }),
                source_span: Span::new(start, span.end),
            });
        }
    }

    /// Splice a styled decoration leaf into the middle of the pending plain
    /// run at an interior referent. Where [`Self::flush_plain_up_to`]
    /// only truncates the tail, this splits the run at the referent:
    /// `Plain[pending_start, deco.start]` · `deco` · then restarts the pending
    /// run at `deco.end` so the tail (up to the bracket's `consume_start`)
    /// flushes normally afterward. Pre: the caller has verified the pending
    /// run is open and `pending_start <= deco_span.start`.
    fn splice_plain_around(&mut self, deco: Node, deco_span: Span) {
        // Head plain run before the referent (often empty → emits nothing).
        self.flush_plain_up_to(deco_span.start);
        // `flush_plain_up_to` may have split one provenance segment at the
        // referent start. Remove every queued piece covered by the decoration,
        // retaining the provenance of a segment tail that begins after it.
        while self
            .pending_plain
            .front()
            .is_some_and(|plain| plain.source_span.start < deco_span.end)
        {
            let mut plain = self.pending_plain.pop_front().expect("checked Some");
            if plain.source_span.end > deco_span.end {
                plain.source_span.start = deco_span.end;
                self.pending_plain.push_front(plain);
                break;
            }
        }
        self.push_output(ClassifiedSpan {
            kind: SpanKind::Aozora(deco),
            source_span: deco_span,
        });
        // The unconsumed tail remains queued and is flushed by the caller's
        // `flush_plain_up_to`.
    }

    /// Open a new top-level frame. `gaiji_refmark` is `Some(span)` when
    /// the outer open was preceded by a `Solo(RefMark)` waiting to be
    /// consumed (the gaiji shape).
    fn open_frame(&mut self, open_event: PairEvent, gaiji_refmark: Option<Span>) {
        #[cfg(feature = "classify-instrument")]
        let _classify_guard = SubsystemGuard::new(Subsystem::OpenFrame);
        let mut body: smallvec::SmallVec<[PairEvent; 16]> = smallvec::SmallVec::new();
        let mut links: smallvec::SmallVec<[u32; 16]> = smallvec::SmallVec::new();
        // Inner stack tracks nested opens; the outer open is at body[0],
        // where its position is recorded.
        let mut inner_stack = smallvec::SmallVec::new();
        let &PairEvent::PairOpen { kind, .. } = &open_event else {
            unreachable!("open_frame called with non-PairOpen event");
        };
        inner_stack.push((kind, 0_usize));
        body.push(open_event);
        links.push(u32::MAX);
        self.frame = Some(Frame {
            body,
            links,
            inner_stack,
            gaiji_refmark,
        });
    }

    /// Append an event to the current frame's body, updating the
    /// inner-stack and patching the parallel `links` side-table as
    /// needed. Returns `true` if the appended event closed the
    /// outermost pair (i.e. `inner_stack` became empty), signalling
    /// that the caller should run recognition on the now-complete
    /// buffer.
    fn append_to_frame(&mut self, event: PairEvent) -> FrameStep {
        #[cfg(feature = "classify-instrument")]
        let _classify_guard = SubsystemGuard::new(Subsystem::FrameAppend);
        let frame = self
            .frame
            .as_mut()
            .expect("append_to_frame requires an active frame");
        let body_idx = frame.body.len();

        match &event {
            PairEvent::PairOpen { kind, .. } => {
                frame.inner_stack.push((*kind, body_idx));
                frame.body.push(event);
                frame.links.push(u32::MAX);
            }
            PairEvent::PairClose { kind, .. } => {
                // Find the matching open via the inner stack. The pair stage
                // guarantees that a PairClose only arrives when the
                // top of the global stack matches its kind, but the body
                // buffer may hold nested opens of several kinds; the
                // nearest matching open is patched.
                if let Some(pos) = frame.inner_stack.iter().rposition(|&(k, _)| k == *kind) {
                    let (_, open_body_idx) = frame.inner_stack.remove(pos);
                    frame.body.push(event);
                    let body_idx_u32 = u32::try_from(body_idx)
                        .expect("body_idx fits u32 (corpus body lengths are bounded)");
                    let open_body_idx_u32 = u32::try_from(open_body_idx)
                        .expect("body_idx fits u32 (corpus body lengths are bounded)");
                    frame.links.push(open_body_idx_u32);
                    frame.links[open_body_idx] = body_idx_u32;
                } else {
                    // No matching open in this buffer. The pair stage's
                    // stack-balance contract delivers a PairClose only when
                    // the outer stack matches; the event is appended
                    // without a link.
                    frame.body.push(event);
                    frame.links.push(u32::MAX);
                }
            }
            PairEvent::Unclosed { kind, .. } => {
                // The pair stage force-resolved an unclosed open. When a
                // hard-scope `］` unwinds a non-bracket open stacked above the
                // bracket (see `pair.rs`), drop its inner-stack entry so the
                // buffer can still close on the following `PairClose(Bracket)`.
                // The event stays in the body with no link (`u32::MAX`), so
                // forward recognisers see an unresolved pair and decline:
                // the body round-trips as raw bytes.
                //
                // Position semantics: the frame's outermost open is at
                // inner-stack position 0 and never closes via a hard-scope
                // unwind (that pops opens stacked above the bracket). A
                // position-0 match means the pair stage force-expired the
                // frame's own open (the EOF drain, or the newline expiry
                // of a line-spanning `［`), so the frame is abandoned: its
                // buffered body replays to plain (the caller's abandon
                // path), and later events classify at top level again
                // instead of buffering forever.
                if let Some(pos) = frame.inner_stack.iter().rposition(|&(k, _)| k == *kind) {
                    if pos == 0 {
                        frame.body.push(event);
                        frame.links.push(u32::MAX);
                        return FrameStep::Abandoned;
                    }
                    frame.inner_stack.remove(pos);
                }
                frame.body.push(event);
                frame.links.push(u32::MAX);
            }
            _ => {
                frame.body.push(event);
                frame.links.push(u32::MAX);
            }
        }

        if frame.inner_stack.is_empty() {
            FrameStep::Closed
        } else {
            FrameStep::Open
        }
    }

    /// Run recognition on the current frame's body buffer and emit the
    /// resulting span. Called when the outermost pair has just closed.
    fn recognize_and_emit(&mut self) {
        #[cfg(feature = "classify-instrument")]
        let _classify_guard = SubsystemGuard::new(Subsystem::RecognizeAndEmit);
        let frame = self
            .frame
            .take()
            .expect("recognize_and_emit requires an active frame");
        let body = frame.body;
        let links = frame.links;
        debug_assert!(body.len() >= 2, "frame body must contain open + close");
        debug_assert_eq!(body.len(), links.len(), "links must parallel body");

        // The frame's outer open is at body[0], the matching close
        // at body[body.len() - 1].
        let open_idx = 0usize;
        let close_idx = body.len() - 1;

        // Pull open span / kind for emission and pending-plain truncation.
        let PairEvent::PairOpen {
            kind: open_kind, ..
        } = body[open_idx]
        else {
            unreachable!("frame body[0] must be PairOpen");
        };

        let view = BodyView {
            events: &body,
            links: &links,
        };

        if self.try_emit_detached_declaration(view, open_kind) {
            return;
        }

        match open_kind {
            PairKind::Ruby => {
                if let Some(span) = self.try_ruby_emit(view, open_idx, close_idx) {
                    self.push_output(span);
                    return;
                }
            }
            PairKind::AngleQuote => {
                if self.quote_spans_block_note(view) {
                    self.classify_quote_interior(&body);
                    return;
                }
                if let Some(span) = self.try_angle_quote_emit(view, open_idx, close_idx) {
                    self.push_output(span);
                    return;
                }
                // Empty `≪≫` falls through to `replay_unrecognised_body`
                // so the bytes return to the pending plain run.
            }
            PairKind::Bracket => {
                let refmark = frame.gaiji_refmark;
                if let Some(rm_span) = refmark {
                    // Pass the original body and links directly with
                    // `bracket_open_idx = 0`: `recognize_gaiji` takes the
                    // bracket entry point via `bracket_open_idx` and
                    // `refmark_span` as a separate argument, so no
                    // synthetic Solo(RefMark) prefix is needed. Copying the
                    // body into a SmallVec to prepend one accounted for the
                    // memcpy_memmove buckets of 25.13 % (doc 50685) and
                    // 22.63 % (doc 49178) on this hot path.
                    let (emitted, uninterpreted) =
                        self.refmark_gaiji(view, open_idx, close_idx, rm_span);
                    if let Some((gaiji, bar, prefix)) = emitted {
                        // Defer the emit one step: an immediately-following
                        // `《…》` ruby can adopt this gaiji as its base
                        // (`※［＃…］《みは》`). When this gaiji is
                        // source-adjacent to an already-pending run, extend
                        // that run instead of replacing it, so an ateji
                        // spanning several gaiji (`※…※…《かいがい》`) forms one
                        // multi-segment base. `process_event` / `finalize`
                        // flush the run as standalone spans when the next
                        // event neither continues nor adopts it.
                        let gaiji_start = gaiji.span.source_span.start;
                        let adjacent = self.pending_ruby_base.as_ref().is_some_and(|p| {
                            p.end() == gaiji_start
                                || prefix.is_some_and(|text| text.start == p.end())
                        });
                        if adjacent {
                            self.pending_ruby_base
                                .as_mut()
                                .expect("adjacent implies Some")
                                .segs
                                .push(gaiji);
                        } else {
                            if let Some(old) = self.pending_ruby_base.take() {
                                self.emit_pending_base(old);
                            }
                            self.pending_ruby_base =
                                Some(PendingRubyBase::single(gaiji, bar, prefix));
                        }
                        return;
                    }
                    // Gaiji recognition declined. A run held for a possible
                    // ruby base must flush first so it precedes this non-gaiji
                    // directive in source order (the held refmark looked like
                    // a continuation until recognition declined). Then fold
                    // the refmark bytes into the pending plain run and attempt
                    // a normal bracket annotation on the original body.
                    if let Some(pending) = self.pending_ruby_base.take() {
                        self.emit_pending_base(pending);
                    }
                    if !matches!(
                        view.events.get(open_idx + 1),
                        Some(PairEvent::Solo {
                            kind: TriggerKind::Hash,
                            ..
                        })
                    ) {
                        self.replay_unrecognised_body(body, Some(rm_span));
                        return;
                    }
                    // Before an annotation that a recognizer interprets, the
                    // refmark is a literal reference mark.
                    let provenance = if uninterpreted == Some(false) {
                        PlainProvenance::Text
                    } else {
                        PlainProvenance::RecoveredVerbatim
                    };
                    self.push_plain(rm_span, provenance);
                    if self.try_bracket_emit(view, open_idx, close_idx).is_some() {
                        return;
                    }
                    // Both gaiji and bracket annotation declined: replay
                    // the body and fold the refmark span into plain.
                    self.replay_unrecognised_body(body, None);
                    return;
                }
                if self.try_bracket_emit(view, open_idx, close_idx).is_some() {
                    return;
                }
            }
            // Tortoise / Quote at top level have no built-in
            // recogniser; the bracket bytes pass through as plain.
            _ => {}
        }

        // Recognition declined: every event in the body becomes plain.
        // Replay the buffered events through the per-event acceptor so
        // that any Newlines inside are emitted as their own spans and the
        // surrounding bytes attach to a top-level Plain run. If the
        // frame was opened in gaiji-mode, the refmark span is also
        // folded back to plain.
        self.replay_unrecognised_body(body, frame.gaiji_refmark);
    }

    fn abandoned_annotation_boundary(&self, frame: &Frame) -> Option<usize> {
        if !matches!(
            frame.body.first(),
            Some(PairEvent::PairOpen {
                kind: PairKind::Bracket,
                ..
            })
        ) || !matches!(
            frame.body.get(1),
            Some(PairEvent::Solo {
                kind: TriggerKind::Hash,
                ..
            })
        ) {
            return None;
        }
        let mut depth = 0;
        for (index, event) in frame.body.iter().enumerate() {
            match event {
                PairEvent::PairOpen { .. } => depth += 1,
                PairEvent::PairClose { .. } => depth -= 1,
                PairEvent::Newline { .. } | PairEvent::Unclosed { .. } => return None,
                PairEvent::Unmatched {
                    kind: PairKind::Quote,
                    span,
                } if depth == 1 => {
                    let mut events = frame.body[..=index].to_vec();
                    let mut links = frame.links[..=index].to_vec();
                    events[index] = PairEvent::PairClose {
                        kind: PairKind::Bracket,
                        span: *span,
                    };
                    links[0] = u32::try_from(index).expect("body index fits native span");
                    links[index] = 0;
                    let view = BodyView {
                        events: &events,
                        links: &links,
                    };
                    let mut alloc = Allocator::new();
                    let mut ctx = RecogniseCtx {
                        alloc: &mut alloc,
                        source: self.source,
                        diagnostics: Vec::new(),
                        pending_plain_start: self.pending_plain_start(),
                        pending_decorations: Vec::new(),
                    };
                    let recognized = if let Some(refmark) = frame.gaiji_refmark {
                        ctx.recognize_gaiji(view, refmark, 0).is_some()
                    } else {
                        ctx.recognize_annotation(view, 0, index, self.pending_plain_start())
                            .is_some_and(|annotation| {
                                annotation.pending_diagnostic.is_none()
                                    && !matches!(
                                        annotation.emit,
                                        EmitKind::Aozora(
                                            Node::Directive(_)
                                                | Node::MarginNote(_)
                                                | Node::TranscribedNotes(_)
                                        )
                                    )
                            })
                    };
                    if recognized && ctx.diagnostics.is_empty() {
                        return Some(index);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn replay_abandoned_frame(&mut self, mut frame: Frame) {
        let Some(boundary) = self.abandoned_annotation_boundary(&frame) else {
            self.replay_unrecognised_body(frame.body, frame.gaiji_refmark);
            return;
        };
        let uncertain_reading = frame.gaiji_refmark.and_then(|_| {
            let mut index = boundary + 1;
            while let Some(PairEvent::Text { range }) = frame.body.get(index) {
                if !self.source[range.start as usize..range.end as usize]
                    .chars()
                    .all(is_ruby_base_char)
                {
                    return None;
                }
                index += 1;
            }
            if !matches!(
                frame.body.get(index),
                Some(PairEvent::PairOpen {
                    kind: PairKind::Ruby,
                    ..
                })
            ) {
                return None;
            }
            let close = usize::try_from(*frame.links.get(index)?).ok()?;
            (close < frame.body.len() && close > index)
                .then_some((index - boundary - 1, close - index + 1))
        });
        if matches!(frame.body.last(), Some(PairEvent::Unclosed { span, .. })
            if Some(*span) == frame.body.first().and_then(PairEvent::span))
        {
            frame.body.pop();
        }
        if let Some(refmark) = frame.gaiji_refmark
            && let Some(plain) = self.pending_plain.back()
            && plain.source_span.end == refmark.start
            && &self.source[plain.source_span.start as usize..plain.source_span.end as usize]
                == "｜"
        {
            frame.gaiji_refmark = Some(Span::new(plain.source_span.start, refmark.end));
            self.pending_plain.pop_back();
        }
        let mut events = frame.body.into_iter();
        let annotation = events.by_ref().take(boundary + 1).collect();
        self.replay_unrecognised_body(annotation, frame.gaiji_refmark);
        let mut events = events.enumerate();
        while let Some((index, event)) = events.next() {
            if let Some((_, count)) = uncertain_reading.filter(|(start, _)| *start == index) {
                let mut reading = smallvec::SmallVec::new();
                reading.push(event);
                reading.extend(events.by_ref().take(count - 1).map(|(_, event)| event));
                self.replay_unrecognised_body(reading, None);
            } else {
                self.process_event(event);
            }
        }
    }

    /// Replay the events from a frame whose recognition declined.
    /// The whole declined construct remains recovered source, including its
    /// interior text. Newlines retain their independent structural spans.
    ///
    /// `refmark` is `Some(span)` when the frame was opened in
    /// gaiji-mode (Bracket preceded by `※`). The refmark bytes need
    /// to be re-folded into plain since gaiji recognition declined.
    ///
    /// `Unclosed` events are skipped during replay: they are
    /// synthetic EOF markers carrying the same span as the original
    /// `PairOpen` (which is also in `body`), and re-adding their span
    /// to the pending plain run would double-count bytes already
    /// covered by the open's `body[0]` entry.
    fn replay_unrecognised_body(
        &mut self,
        body: smallvec::SmallVec<[PairEvent; 16]>,
        refmark: Option<Span>,
    ) {
        #[cfg(feature = "classify-instrument")]
        let _classify_guard = SubsystemGuard::new(Subsystem::ReplayBody);
        #[cfg(feature = "classify-instrument")]
        record_replay_body_size(body.len() as u64);
        if let Some(pending) = self.pending_ruby_base.take() {
            self.emit_pending_base(pending);
        }
        let start = refmark.or_else(|| body.first().and_then(PairEvent::span));
        let end = body
            .iter()
            .filter_map(PairEvent::span)
            .map(|span| span.end)
            .max();
        // Preserve the failed construct's boundary even next to another
        // recovered token, such as a literal closing bracket.
        if let Some(span) = start {
            self.flush_plain_up_to(span.start);
        }
        if let Some(rm) = refmark {
            self.push_plain(rm, PlainProvenance::RecoveredVerbatim);
        }
        for ev in body {
            if matches!(ev, PairEvent::Unclosed { .. }) {
                continue;
            }
            if matches!(ev, PairEvent::Text { .. }) {
                // Text inside a declined syntax frame is part of that failed
                // construct, not independently accepted visible prose.
                self.push_plain(
                    ev.span().expect("text span"),
                    PlainProvenance::RecoveredVerbatim,
                );
            } else {
                self.handle_top_level(ev, /*replay=*/ true);
            }
        }
        if let Some(end) = end {
            self.flush_plain_up_to(end);
        }
    }

    /// Handle a top-level event (no active frame) in either streaming
    /// mode (`replay = false`) or replay mode (`replay = true`, which
    /// suppresses the frame-open path so a residual nested `PairOpen` in
    /// a declined body does not re-open a sub-frame).
    fn handle_top_level(&mut self, event: PairEvent, replay: bool) {
        match event {
            PairEvent::Newline { pos } => {
                self.flush_plain_up_to(pos);
                self.push_output(ClassifiedSpan {
                    kind: SpanKind::Newline,
                    source_span: Span::new(pos, pos + 1),
                });
            }
            PairEvent::Solo {
                kind: TriggerKind::RefMark,
                span,
            } if !replay => {
                // Hold the refmark pending the next event. If a flush
                // is requested before the next event arrives the
                // refmark is folded into the plain run.
                self.pending_refmark = Some(span);
            }
            PairEvent::PairOpen { kind, span, .. } if !replay => {
                // Literal delimiters remain text, while notation inside them
                // still reaches its recognizer. Replaying a whole literal
                // bracket as plain would hide nested gaiji and annotations.
                if matches!(kind, PairKind::Quote | PairKind::Tortoise)
                    || (self.pending_refmark.is_none() && self.is_literal_bracket(kind, span))
                {
                    // Fold any pending refmark into plain first, then
                    // start the new plain run at the open's first byte.
                    let pre_open = self
                        .pending_refmark
                        .take()
                        .map_or(span.start, |rm| rm.start);
                    // A bracket after an explicit `｜` is held open as a
                    // possible continuation of the base, because that is what
                    // `｜漢字［＃「漢字」に傍点］` is. A literal bracket ends
                    // that possibility, and the base's spans start before the
                    // plain run this is about to flush: emitting the plain
                    // first would put the base's own text after text that
                    // follows it in the source.
                    if let Some(pending) = self.pending_ruby_base.take() {
                        self.emit_pending_base(pending);
                    }
                    self.flush_plain_up_to(pre_open);
                    self.push_plain(span, PlainProvenance::RecoveredVerbatim);
                    self.streaming = Some(StreamingFrame { kind, depth: 1 });
                    return;
                }
                // Opening a top-level pair may flush the pending plain
                // up to (but not including) the open's start. The
                // refmark, if any and only when this open is a
                // Bracket, is consumed by the frame; for any other
                // pair kind the refmark is folded into plain first.
                //
                // Ruby and AngleQuote consume the preceding text
                // (explicit `｜base《reading》` or implicit
                // trailing-kanji). `pending_plain_start` is not flushed
                // here, so `try_ruby_emit` can walk the preceding source
                // bytes and determine how much of the plain run the ruby
                // consumes.
                //
                // Bracket is handled the same way: forward-reference
                // `［＃「X」に傍点］` / `…は縦中横］` classifiers may move
                // `consume_start` back over the preceding target
                // literal. `try_bracket_emit` then calls
                // `flush_plain_up_to(consume_start)` itself, with the
                // same effect for recognised brackets and a single fused
                // Plain run (literal + raw bracket bytes) for the
                // unrecognised path's replay, which renders identically
                // to separate literal and bracket spans.
                let gaiji_refmark = if matches!(kind, PairKind::Bracket) {
                    self.pending_refmark.take()
                } else {
                    None
                };
                let preserve_pending_plain = matches!(
                    kind,
                    PairKind::Ruby | PairKind::AngleQuote | PairKind::Bracket
                );
                if !preserve_pending_plain {
                    let truncate_to = gaiji_refmark.map_or(span.start, |rm| rm.start);
                    self.flush_plain_up_to(truncate_to);
                }
                self.open_frame(PairEvent::PairOpen { kind, span }, gaiji_refmark);
            }
            other => {
                // Catch-all: every non-Newline event carries a span
                // and folds into the pending plain run.
                let Some(span) = other.span() else {
                    return;
                };
                let provenance = if matches!(other, PairEvent::Text { .. }) {
                    PlainProvenance::Text
                } else {
                    PlainProvenance::RecoveredVerbatim
                };
                self.push_plain(span, provenance);
            }
        }
    }

    fn is_literal_bracket(&self, kind: PairKind, span: Span) -> bool {
        kind == PairKind::Bracket && !self.source[span.end as usize..].starts_with('＃')
    }

    /// Fold a held `※` refmark into the plain run unless the very next
    /// event is the `［` that consumes it (the gaiji shape). The streaming
    /// mirror of the top-level refmark block in `process_event`, which is
    /// skipped while streaming because the stream-through check routes to
    /// `handle_stream_event` first, so an orphan `※` inside a quote
    /// round-trips as plain, exactly as at top level.
    fn fold_held_refmark(&mut self, event: &PairEvent) {
        if self.pending_refmark.is_some()
            && !matches!(
                event,
                PairEvent::PairOpen {
                    kind: PairKind::Bracket,
                    ..
                }
            )
        {
            self.fold_pending_refmark();
        }
    }

    /// Handle one event while in stream-through mode (top-level
    /// Quote / Tortoise pair, no recogniser candidate). Mirrors the
    /// `replay = true` behaviour of [`Self::handle_top_level`] but
    /// (a) reads from the live event stream rather than a buffered
    /// `SmallVec`, (b) tracks nested-open depth so the outer close
    /// unambiguously exits the mode, and (c) skips the inner-frame
    /// open path (a nested `Bracket` / `Ruby` / `AngleQuote` inside
    /// an unrecognised `Quote` folds into the surrounding plain run,
    /// as in buffered replay).
    fn handle_stream_event(&mut self, event: PairEvent) {
        self.fold_held_refmark(&event);

        // A nested Ruby / AngleQuote / Bracket inside the streamed
        // Quote/Tortoise is recognised without changing the streaming depth,
        // so it is handled before the streaming frame is borrowed. This
        // makes `「駄目《だめ》」` form a ruby, a nested `≪…≫` resolve, and an
        // in-quote directive / gaiji (`「…［＃「X」に傍点］…」`,
        // `「…※［＃…水準…］《みは》…」`) resolve instead of remaining literal
        // text: the sub-frame runs the same recogniser as at top level, and
        // `pending_plain_start` (kept live through the quote) supplies the
        // ruby base or the forward-reference target. `process_event` checks
        // `frame` before `streaming`, so the sub-frame's body buffers
        // correctly and streaming resumes once it closes.
        if let PairEvent::PairOpen { kind, span } = &event
            && matches!(
                kind,
                PairKind::Ruby | PairKind::AngleQuote | PairKind::Bracket
            )
            && (self.pending_refmark.is_some() || !self.is_literal_bracket(*kind, *span))
        {
            // None of the three flush `pending_plain_start`, so the
            // recogniser can pull `consume_start` back over the preceding
            // text. Only a Bracket consumes a held `※` refmark (the gaiji
            // shape); Ruby / AngleQuote do not. Routing an in-quote gaiji
            // through this Bracket sub-frame lets `try_gaiji_emit` clear
            // `pending_plain_start`, the precondition
            // `try_ruby_over_pending_base` requires to adopt an adjacent `《…》`.
            let gaiji_refmark = if matches!(kind, PairKind::Bracket) {
                self.pending_refmark.take()
            } else {
                None
            };
            self.open_frame(
                PairEvent::PairOpen {
                    kind: *kind,
                    span: *span,
                },
                gaiji_refmark,
            );
            return;
        }
        // Called only when `streaming` is `Some`.
        let stream = self
            .streaming
            .as_mut()
            .expect("handle_stream_event without streaming state");
        match event {
            PairEvent::Newline { pos } => {
                self.flush_plain_up_to(pos);
                self.push_output(ClassifiedSpan {
                    kind: SpanKind::Newline,
                    source_span: Span::new(pos, pos + 1),
                });
            }
            PairEvent::PairOpen { kind, span } if kind == stream.kind => {
                stream.depth = stream.depth.saturating_add(1);
                self.push_plain(span, PlainProvenance::RecoveredVerbatim);
            }
            PairEvent::PairClose { kind, span } if kind == stream.kind => {
                stream.depth = stream.depth.saturating_sub(1);
                let streaming_finished = stream.depth == 0;
                self.push_plain(span, PlainProvenance::RecoveredVerbatim);
                if streaming_finished {
                    self.streaming = None;
                }
            }
            PairEvent::Unclosed { kind, .. } => {
                // The pair stage emits synthetic Unclosed events when EOF
                // arrives mid-frame, one per still-open pair. Each
                // one's span aliases its original PairOpen; those
                // bytes were already folded into the plain run when
                // the PairOpen arrived (or emitted by an intervening
                // Newline flush). Re-folding here would set
                // `pending_plain_start` to a position behind the
                // cursor and break the tiling invariant.
                //
                // For Unclosed of the streaming kind, decrement depth
                // so the count mirrors the matching PairOpens (one
                // increment per open, one Unclosed per still-open).
                // When depth reaches zero the outer pair is gone, so
                // exit streaming. Other-kind Unclosed events (a
                // nested Bracket / Ruby / AngleQuote that streaming
                // mode never opened a frame for) are ignored.
                if kind == stream.kind {
                    stream.depth = stream.depth.saturating_sub(1);
                    if stream.depth == 0 {
                        self.streaming = None;
                    }
                }
            }
            PairEvent::Solo {
                kind: TriggerKind::RefMark,
                span,
            } => {
                // Hold the `※` pending the next event: the streaming
                // mirror of the top-level `handle_top_level` refmark arm.
                // The block at the top of this method folds it to plain if
                // no `［` follows, or the Bracket sub-frame consumes it as
                // the gaiji shape.
                self.pending_refmark = Some(span);
            }
            other => {
                let Some(span) = other.span() else {
                    return;
                };
                let provenance = if matches!(other, PairEvent::Text { .. }) {
                    PlainProvenance::Text
                } else {
                    PlainProvenance::RecoveredVerbatim
                };
                self.push_plain(span, provenance);
            }
        }
    }

    /// Adopt the deferred native segments and adjacent text as a ruby base.
    /// Its reading uses the same native body recognizers as a plain-base ruby.
    fn try_ruby_over_pending_base(
        &mut self,
        body: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> PendingBaseRuby {
        let PairEvent::PairOpen {
            span: open_span, ..
        } = body.events[open_idx]
        else {
            return PendingBaseRuby::NotApplicable;
        };
        let adjacent_base = self.pending_ruby_base.as_ref().is_some_and(|p| {
            let between = &self.source[p.end() as usize..open_span.start as usize];
            p.end() == open_span.start
                || (p.extensible()
                    && self.pending_plain_start() == Some(p.end())
                    && (p.bar.is_some()
                        || between.chars().all(is_ruby_base_char)
                        || p.spaced_from_reading(between)))
        });
        if !adjacent_base {
            return PendingBaseRuby::NotApplicable;
        }
        let pending = self.pending_ruby_base.take().expect("checked Some");
        let PairEvent::PairClose {
            span: close_span, ..
        } = body.events[close_idx]
        else {
            self.emit_pending_base(pending);
            return PendingBaseRuby::Declined;
        };
        if open_span.end >= close_span.start {
            // An empty reading cannot adopt a base; replay the original spans.
            self.emit_pending_base(pending);
            return PendingBaseRuby::Declined;
        }
        let reading = {
            let mut ctx = RecogniseCtx {
                alloc: self.alloc,
                source: self.source,
                diagnostics: Vec::new(),
                pending_plain_start: None,
                pending_decorations: Vec::new(),
            };
            let reading = ctx.build_content_from_body(
                body,
                &BodyWindow {
                    events: open_idx + 1..close_idx,
                    bytes: open_span.end..close_span.start,
                },
            );
            self.diagnostics.append(&mut ctx.diagnostics);
            reading
        };
        let base_start = pending.start();
        let mut segs = Vec::new();
        if let Some(prefix) = pending.prefix {
            push_text_segment(&mut segs, self.source, prefix.start..prefix.end, self.alloc);
        }
        let mut previous_end = None;
        for segment in &pending.segs {
            if let Some(start) =
                previous_end.filter(|start| *start < segment.span.source_span.start)
            {
                push_text_segment(
                    &mut segs,
                    self.source,
                    start..segment.span.source_span.start,
                    self.alloc,
                );
            }
            segs.push(self.bind_ruby_base_instruction(
                segment.payload,
                previous_end.or_else(|| pending.prefix.map(|span| span.start)),
            ));
            previous_end = Some(segment.span.source_span.end);
        }
        if pending.end() < open_span.start {
            push_text_segment(
                &mut segs,
                self.source,
                pending.end()..open_span.start,
                self.alloc,
            );
            self.pending_plain.clear();
        }
        let base = self.alloc.content_segments(&segs);
        let node = self.alloc.ruby(base, reading);
        PendingBaseRuby::Emitted(ClassifiedSpan {
            kind: SpanKind::Aozora(node),
            // The consumed base marker remains part of the ruby's source span.
            source_span: Span::new(base_start, close_span.end),
        })
    }

    fn bind_ruby_base_instruction(&mut self, segment: Segment, start: Option<u32>) -> Segment {
        let Segment::Directive { value, source_span } = segment else {
            return segment;
        };
        let DirectiveKind::FormattingInstruction(id) = value.kind else {
            return segment;
        };
        let Some(start) = start else {
            return segment;
        };
        let mut instruction = self
            .alloc
            .store()
            .resolve_formatting_instruction(id)
            .clone();
        formatting_instruction::bind(
            &mut instruction,
            self.source,
            Span::new(start, source_span.start),
        );
        if &instruction == self.alloc.store().resolve_formatting_instruction(id) {
            return segment;
        }
        let raw = &self.source[source_span.start as usize..source_span.end as usize];
        Segment::Directive {
            value: self.alloc.formatting_instruction(raw, instruction),
            source_span,
        }
    }

    fn try_ruby_emit(
        &mut self,
        body: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<ClassifiedSpan> {
        #[cfg(feature = "classify-instrument")]
        let _classify_guard = SubsystemGuard::new(Subsystem::TryRubyEmit);
        // Ruby recognition uses the preceding text (if any) as the
        // base, but in the streaming model that text is not in the body
        // buffer; it is located through `pending_outputs` and
        // `pending_plain_start`.
        //
        // The recogniser receives a synthesised body slice with a single
        // preceding Text event for the current
        // `pending_plain_start..open_span.start` range, plus a
        // `Solo(Bar)` if the explicit-ruby shape applies, as
        // `recognize_ruby` expects:
        //   events[open_idx - 1] = Text { range: ... preceding ... }
        //   events[open_idx - 2] = optional Solo(Bar)
        let PairEvent::PairOpen {
            span: open_span, ..
        } = body.events[open_idx]
        else {
            return None;
        };

        // Nested ruby: a `《…》` opening inside the reading
        // body is an authoring error. Flag the first one (caret on the
        // inner `《`); the outer ruby still parses best-effort. Recorded
        // before `ctx` reborrows `self.alloc`, so the borrows do not conflict.
        if let Some(inner_open) = first_nested_ruby_open(body.events, open_idx + 1, close_idx) {
            self.diagnostics.push(Diagnostic::nested_ruby(inner_open));
        }

        // A structured base takes precedence over the trailing plain fragment.
        match self.try_ruby_over_pending_base(body, open_idx, close_idx) {
            PendingBaseRuby::Emitted(span) => return Some(span),
            PendingBaseRuby::Declined => return None,
            PendingBaseRuby::NotApplicable => {}
        }

        // Determine the preceding plain run (the ruby base lives here) and
        // detect the explicit `｜` form by scanning it for a bar. The
        // streaming model has no preceding events, so they are synthesised.
        let preceding_start = self.pending_plain_start().unwrap_or(open_span.start);
        if preceding_start >= open_span.start {
            return None;
        }
        let prev_text_range = Span::new(preceding_start, open_span.start);
        let preceding_bytes = &self.source[preceding_start as usize..open_span.start as usize];
        let bar_byte_offset = preceding_bytes.rfind('｜');

        let (synth, synth_links, synth_open_idx) =
            build_synth_ruby_view(body, prev_text_range, bar_byte_offset)?;
        let synth_close_idx = synth_open_idx + (close_idx - open_idx);
        let synth_view = BodyView {
            events: synth.as_slice(),
            links: synth_links.as_slice(),
        };
        let mut ctx = RecogniseCtx {
            alloc: self.alloc,
            source: self.source,
            diagnostics: Vec::new(),
            pending_plain_start: None,
            pending_decorations: Vec::new(),
        };
        let Some(m) = ctx.recognize_ruby(synth_view, synth_open_idx, synth_close_idx) else {
            // `recognize_ruby` rejects an empty `《》` reading; flag the
            // explicit-base `｜base《》` shape (a bare `《》` produces no
            // diagnostic). The bytes fall through to plain replay either
            // way. `ctx`'s reborrow of `self.alloc` ended at the call above,
            // so pushing onto the disjoint `self.diagnostics` does not
            // conflict with it.
            if let PairEvent::PairClose {
                span: close_span, ..
            } = body.events[close_idx]
                && let Some(span) = empty_explicit_ruby_span(
                    bar_byte_offset,
                    preceding_start,
                    open_span,
                    close_span,
                )
            {
                self.diagnostics.push(Diagnostic::empty_ruby_reading(span));
            }
            return None;
        };
        // Drain diagnostics raised while building the nested ruby reading.
        self.diagnostics.append(&mut ctx.diagnostics);
        // Truncate any in-progress plain run to end exactly where the ruby
        // takes over.
        self.flush_plain_up_to(m.consume_start);
        let base_start =
            open_span.start - u32::try_from(m.base.len()).expect("base fits native span");
        let mut base_segments = Vec::new();
        push_text_segment(
            &mut base_segments,
            self.source,
            base_start..open_span.start,
            self.alloc,
        );
        let base_content = self.alloc.content_segments(&base_segments);
        let node = self.alloc.ruby(base_content, m.reading);
        self.pending_plain.clear();
        Some(ClassifiedSpan {
            kind: SpanKind::Aozora(node),
            source_span: Span::new(m.consume_start, m.consume_end),
        })
    }

    /// Whether a `≪…≫` frame has a block container note on a line of its own
    /// with every nested pair closed inside the frame. Such a quote does not
    /// contain the block: the published XHTML prints ≪ and ≫ as text there.
    fn quote_spans_block_note(&self, body: BodyView<'_>) -> bool {
        let (Some(open), Some(close)) = (
            body.events.first().and_then(PairEvent::span),
            body.events.last().and_then(PairEvent::span),
        ) else {
            return false;
        };
        let balanced = body
            .events
            .iter()
            .zip(body.links)
            .all(|(event, link)| match event {
                PairEvent::PairOpen { .. } | PairEvent::PairClose { .. } => *link != u32::MAX,
                PairEvent::Unclosed { .. } | PairEvent::Unmatched { .. } => false,
                _ => true,
            });
        balanced
            && self.source[open.end as usize..close.start as usize]
                .split('\n')
                .skip(1)
                .any(|line| line.starts_with("［＃ここから") || line.starts_with("［＃ここで"))
    }

    /// Classify the interior of a declined `≪…≫` frame as top-level source,
    /// with the delimiters as text.
    fn classify_quote_interior(&mut self, body: &[PairEvent]) {
        let (Some(open), Some(close)) = (
            body.first().and_then(PairEvent::span),
            body.last().and_then(PairEvent::span),
        ) else {
            return;
        };
        if let Some(pending) = self.pending_ruby_base.take() {
            self.emit_pending_base(pending);
        }
        self.flush_plain_up_to(open.start);
        self.push_plain(open, PlainProvenance::Text);
        for event in &body[1..body.len() - 1] {
            self.process_event(event.clone());
        }
        if let Some(pending) = self.pending_ruby_base.take() {
            self.emit_pending_base(pending);
        }
        self.flush_plain_up_to(close.start);
        self.push_plain(close, PlainProvenance::Text);
    }

    /// Attempt to classify the buffered body as a `AngleQuote` node.
    ///
    /// Returns `None` when the body content is empty (`≪≫` with
    /// no payload); the caller falls through to plain replay so the
    /// bytes appear as literal source. Emitting a `AngleQuote` span
    /// here would violate the `NonEmpty` invariant on the
    /// `Content` payload.
    fn try_angle_quote_emit(
        &mut self,
        body: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<ClassifiedSpan> {
        let PairEvent::PairOpen {
            span: open_span, ..
        } = body.events[open_idx]
        else {
            unreachable!("body[open_idx] must be PairOpen");
        };
        let PairEvent::PairClose {
            span: close_span, ..
        } = body.events[close_idx]
        else {
            unreachable!("body[close_idx] must be PairClose");
        };
        let mut ctx = RecogniseCtx {
            alloc: self.alloc,
            source: self.source,
            diagnostics: Vec::new(),
            pending_plain_start: None,
            pending_decorations: Vec::new(),
        };
        let content = ctx.build_content_from_body(
            body,
            &BodyWindow {
                events: open_idx + 1..close_idx,
                bytes: open_span.end..close_span.start,
            },
        );
        // Drain diagnostics raised while building the nested angle-quote body.
        self.diagnostics.append(&mut ctx.diagnostics);
        // Empty `≪≫` is not a valid AngleQuote: the bytes pass
        // through as plain text. The caller's fall-through path
        // (`replay_unrecognised_body`) handles the plain emission.
        let content_is_empty = match content {
            Content::Plain(s) => self.alloc.store().resolve_str(s).is_empty(),
            Content::Segments(segs) => segs.len == 0,
            _ => false,
        };
        if content_is_empty {
            return None;
        }
        self.flush_plain_up_to(open_span.start);
        let node = self.alloc.angle_quote(content);
        self.pending_plain.clear();
        Some(ClassifiedSpan {
            kind: SpanKind::Aozora(node),
            source_span: Span::new(open_span.start, close_span.end),
        })
    }

    fn try_bracket_emit(
        &mut self,
        body: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<()> {
        #[cfg(feature = "classify-instrument")]
        let _classify_guard = SubsystemGuard::new(Subsystem::TryBracketEmit);
        let pending_plain_start = self.pending_plain_start();
        let mut ctx = RecogniseCtx {
            alloc: self.alloc,
            source: self.source,
            diagnostics: Vec::new(),
            // The forward recognizers resolve a non-adjacent referent inside
            // the current pending plain run; pass them its start.
            pending_plain_start,
            pending_decorations: Vec::new(),
        };
        let mut m = ctx.recognize_annotation(body, open_idx, close_idx, pending_plain_start)?;
        ctx.attach_margin_note_pair_payload(body, open_idx, close_idx, &mut m);
        // End the recognizer's allocator borrow before emitting its selected leaves.
        self.diagnostics.append(&mut ctx.diagnostics);
        let decorations = mem::take(&mut ctx.pending_decorations);
        if !matches!(
            m.emit,
            EmitKind::Aozora(
                Node::Kunten(_)
                    | Node::Directive(_)
                    | Node::Format(_)
                    | Node::Illustration(_)
                    | Node::MarginNote(_)
            )
        ) && let Some(pending) = self.pending_ruby_base.take()
        {
            self.emit_pending_base(pending);
        }
        // Validate the complete ordered selection against the retained plain window.
        // Failure leaves all source text untouched and the reference unassociated.
        let mut previous_end = self.pending_plain_start();
        let all_owned = decorations.iter().all(|(_, span)| {
            let valid = previous_end.is_some_and(|start| start <= span.start)
                && span.start < span.end
                && span.end <= m.consume_start;
            previous_end = Some(span.end);
            valid
        });
        let accent_word = all_owned
            .then(|| self.referenced_accent_word(&m, &decorations))
            .flatten();
        if let Some(word) = accent_word {
            self.hold_accent_word_base(word, &decorations);
        } else if all_owned {
            for (decoration, span) in decorations {
                self.splice_plain_around(decoration, span);
            }
        }
        let kind = match m.emit {
            EmitKind::Aozora(node) => SpanKind::Aozora(node),
            EmitKind::BlockOpen(container) => SpanKind::BlockOpen(container),
            EmitKind::BlockClose(container) => SpanKind::BlockClose(container),
            EmitKind::BlockCloses { targets, fallback } => {
                SpanKind::BlockCloses { targets, fallback }
            }
        };
        // Record any non-fatal warning the recogniser attached
        // (unrecognised container directive / 縦中横 target not found /
        // ambiguous bouten target). The emitted node is unaffected; for
        // the catch-all cases it is still `Directive{Unknown}`, so the
        // Tier-A "no bare ［＃" canary holds. `ctx`'s reborrow of
        // `self.alloc` ended at `recognize_annotation`, so pushing onto the
        // disjoint `self.diagnostics` does not conflict with it.
        if let Some(diag) = m.pending_diagnostic {
            self.diagnostics.push(diag);
        }
        // Record bracketed kaeriten for the end-of-document pairing /
        // context checks (`finalize_kaeriten`). The directive span is the
        // whole `［＃…］`.
        if let SpanKind::Aozora(Node::Kunten(k)) = kind
            && k.kind == KuntenKind::ReturnMark
        {
            let span = Span::new(m.consume_start, m.consume_end);
            let (family, rank, is_ladder) =
                classify_kaeriten_mark(self.alloc.store().resolve_str(k.text));
            self.kaeriten_obs.push(KaeritenObs {
                family,
                rank,
                is_ladder,
                span,
            });
        }
        let span = ClassifiedSpan {
            kind,
            source_span: Span::new(m.consume_start, m.consume_end),
        };
        if accent_word.is_some()
            && let (SpanKind::Aozora(Node::Format(value)), Some(pending)) =
                (&span.kind, self.pending_ruby_base.as_mut())
        {
            pending.segs.push(PendingBaseSegment {
                payload: Segment::Format {
                    value: *value,
                    source_span: span.source_span,
                },
                span,
            });
        } else if !self.defer_explicit_base_annotation(&span) {
            if let Some(pending) = self.pending_ruby_base.take() {
                self.emit_pending_base(pending);
            }
            self.flush_plain_up_to(m.consume_start);
            self.pending_plain.clear();
            self.push_output(span);
        }
        Some(())
    }

    /// Start of the Latin word that a supplied-accent note quoting letters
    /// inside it follows, when a reading follows the note directly: in
    /// `Venus［＃「e」はアクサン（´）付き］《ヴェニュス》` the reading belongs to
    /// `Venus`, as the published XHTML gives.
    fn referenced_accent_word(
        &self,
        m: &AnnotationMatch,
        decorations: &[(Node, Span)],
    ) -> Option<u32> {
        let EmitKind::Aozora(Node::Format(format)) = m.emit else {
            return None;
        };
        if format.origin != ForwardOrigin::Referenced
            || !matches!(format.attrs.single(), Some(ForwardAttr::Accent(_)))
            || !self.source[m.consume_end as usize..].starts_with('《')
            || self.pending_ruby_base.is_some()
        {
            return None;
        }
        let start = self.pending_plain_start()?;
        let before = self.source.get(start as usize..m.consume_start as usize)?;
        let offset = trailing_ruby_base_start(before);
        let first = before[offset..].chars().next()?;
        let word = start + u32::try_from(offset).expect("source offset fits u32");
        (ruby_base_class(first) == Some(RubyBaseClass::Latin)
            && !decorations.is_empty()
            && decorations.iter().all(|(_, span)| word <= span.start))
        .then_some(word)
    }

    /// Hold the Latin word from `word` with its accent `decorations` as a
    /// pending ruby base; the accent note joins it after classification.
    fn hold_accent_word_base(&mut self, word: u32, decorations: &[(Node, Span)]) {
        self.flush_plain_up_to(word);
        self.pending_plain.clear();
        let prefix =
            (word < decorations[0].1.start).then(|| Span::new(word, decorations[0].1.start));
        let segs = decorations
            .iter()
            .filter_map(|&(decoration, source_span)| {
                let Node::Format(value) = decoration else {
                    return None;
                };
                Some(PendingBaseSegment {
                    span: ClassifiedSpan {
                        kind: SpanKind::Aozora(decoration),
                        source_span,
                    },
                    payload: Segment::Format { value, source_span },
                })
            })
            .collect();
        self.pending_ruby_base = Some(PendingRubyBase {
            segs,
            bar: None,
            prefix,
        });
    }

    fn defer_explicit_base_annotation(&mut self, span: &ClassifiedSpan) -> bool {
        let payload = match span.kind {
            SpanKind::Aozora(Node::Format(value)) => Segment::Format {
                value,
                source_span: span.source_span,
            },
            SpanKind::Aozora(Node::Kunten(value)) => Segment::Kunten {
                value,
                source_span: span.source_span,
            },
            SpanKind::Aozora(Node::Directive(value)) => Segment::Directive {
                value,
                source_span: span.source_span,
            },
            SpanKind::Aozora(Node::Illustration(value)) => Segment::Illustration {
                value,
                source_span: span.source_span,
            },
            SpanKind::Aozora(Node::MarginNote(value)) => Segment::MarginNote {
                value,
                source_span: span.source_span,
            },
            _ => return false,
        };
        let adjacent = self.pending_ruby_base.as_ref().is_some_and(|pending| {
            pending.bar.is_some()
                && pending.end() <= span.source_span.start
                && (pending.end() == span.source_span.start
                    || self.pending_plain_start() == Some(pending.end()))
        });
        if adjacent {
            self.pending_plain.clear();
            self.pending_ruby_base
                .as_mut()
                .expect("adjacent base exists")
                .segs
                .push(PendingBaseSegment {
                    span: span.clone(),
                    payload,
                });
            return true;
        }
        let Some(start) = self.pending_plain_start() else {
            return self.defer_reclaimed_target_base(span, payload, "")
                || self.defer_supplied_diacritic_base(span, payload)
                || self.defer_figure_or_note_base(span, payload, "");
        };
        // A reclaimed target can precede the pending plain fragment. Only an
        // ordered fragment containing its own bar establishes this deferred base.
        let Some(before) = self
            .source
            .get(start as usize..span.source_span.start as usize)
        else {
            return false;
        };
        let Some(offset) = before.rfind('｜') else {
            return self.defer_implicit_base_annotation(span, payload, start, before)
                || self.defer_reclaimed_target_base(span, payload, before)
                || self.defer_supplied_diacritic_base(span, payload)
                || self.defer_figure_or_note_base(span, payload, before);
        };
        if before[offset..].contains('\n') {
            return self.defer_supplied_diacritic_base(span, payload);
        }
        let bar_start = start + u32::try_from(offset).expect("source offset fits u32");
        let bar = Span::new(bar_start, bar_start + 3);
        let prefix = (bar.end < span.source_span.start)
            .then_some(Span::new(bar.end, span.source_span.start));
        self.flush_plain_up_to(bar.start);
        self.pending_plain.clear();
        self.pending_ruby_base = Some(PendingRubyBase::single(
            PendingBaseSegment {
                span: span.clone(),
                payload,
            },
            Some(bar),
            prefix,
        ));
        true
    }

    /// Hold a note that a reading follows directly, with the implicit base
    /// run before the note, as the base of that reading: in
    /// `鈍［＃「鈍」は底本では「鋭」］《にぶ》` the reading belongs to `鈍`.
    fn defer_implicit_base_annotation(
        &mut self,
        span: &ClassifiedSpan,
        payload: Segment,
        start: u32,
        before: &str,
    ) -> bool {
        if !matches!(payload, Segment::Directive { .. })
            || !self.source[span.source_span.end as usize..].starts_with('《')
        {
            return false;
        }
        let offset = trailing_ruby_base_start(before);
        if offset == before.len() {
            return false;
        }
        if let Some(pending) = self.pending_ruby_base.take() {
            self.emit_pending_base(pending);
        }
        let prefix = Span::new(
            start + u32::try_from(offset).expect("source offset fits u32"),
            span.source_span.start,
        );
        self.flush_plain_up_to(prefix.start);
        self.pending_plain.clear();
        self.pending_ruby_base = Some(PendingRubyBase::single(
            PendingBaseSegment {
                span: span.clone(),
                payload,
            },
            None,
            Some(prefix),
        ));
        true
    }

    /// Hold a style note with the target it reclaims as the base of a reading
    /// that follows the note directly: in
    /// `公事根源［＃「公事根源」は斜体］《くじこんげん》` the reading belongs to
    /// the styled target, as the published XHTML gives. A target
    /// that continues a base run in `before`, or follows a mark that can be a
    /// base, leaves the extent of the base open.
    fn defer_reclaimed_target_base(
        &mut self,
        span: &ClassifiedSpan,
        payload: Segment,
        before: &str,
    ) -> bool {
        let Segment::Format { value, .. } = payload else {
            return false;
        };
        let target = &self.source[span.source_span.start as usize..];
        if value.origin != ForwardOrigin::Reclaimed
            || value.attrs.single() == Some(ForwardAttr::AccentDot)
            || !self.source[span.source_span.end as usize..].starts_with('《')
        {
            return false;
        }
        let (Some(first), previous) = (target.chars().next(), before.chars().next_back()) else {
            return false;
        };
        if previous.is_some_and(|previous| {
            is_symbol_ruby_base(previous)
                || ruby_base_class(previous)
                    .is_some_and(|class| ruby_base_class(first) == Some(class))
        }) {
            return false;
        }
        if let Some(pending) = self.pending_ruby_base.take() {
            self.emit_pending_base(pending);
        }
        self.flush_plain_up_to(span.source_span.start);
        self.pending_plain.clear();
        self.pending_ruby_base = Some(PendingRubyBase::single(
            PendingBaseSegment {
                span: span.clone(),
                payload,
            },
            None,
            None,
        ));
        true
    }

    /// Hold an illustration, or a note with the target it reclaims, as the
    /// base of a reading that follows the annotation directly: in
    /// `［＃梵字（fig1317_15.png、横18×縦23）入る］《ラン》` the reading belongs to
    /// the figure and in `起誓［＃「起誓」に「ママ」の注記］《きしょう》` to the
    /// noted target, as the published XHTML gives. A target that continues a
    /// base run in `before`, or follows a mark that can be a base, leaves the
    /// extent of the base open, except a mark that a glyph-shape note quotes
    /// as containing the target: in `√３［＃「３」は「√」の記号の中に入っている］《ルートサン》`
    /// the base is `√３`.
    fn defer_figure_or_note_base(
        &mut self,
        span: &ClassifiedSpan,
        payload: Segment,
        before: &str,
    ) -> bool {
        if !self.source[span.source_span.end as usize..].starts_with('《') {
            return false;
        }
        let prefix = match payload {
            Segment::Illustration { .. } => None,
            Segment::MarginNote { value, .. }
                if value.origin == ForwardOrigin::Reclaimed && value.pair.is_none() =>
            {
                let Some(first) = self.source[span.source_span.start as usize..]
                    .chars()
                    .next()
                else {
                    return false;
                };
                match before.chars().next_back() {
                    Some(previous) if self.note_quotes_container(&value, previous) => {
                        let start = span.source_span.start
                            - u32::try_from(previous.len_utf8()).expect("character fits u32");
                        Some(Span::new(start, span.source_span.start))
                    }
                    Some(previous)
                        if is_symbol_ruby_base(previous)
                            || ruby_base_class(previous)
                                .is_some_and(|class| ruby_base_class(first) == Some(class)) =>
                    {
                        return false;
                    }
                    _ => None,
                }
            }
            _ => return false,
        };
        if let Some(pending) = self.pending_ruby_base.take() {
            self.emit_pending_base(pending);
        }
        self.flush_plain_up_to(prefix.map_or(span.source_span.start, |prefix| prefix.start));
        self.pending_plain.clear();
        self.pending_ruby_base = Some(PendingRubyBase::single(
            PendingBaseSegment {
                span: span.clone(),
                payload,
            },
            None,
            prefix,
        ));
        true
    }

    /// Whether a glyph-shape `note` quotes `mark` as the sign that contains its
    /// target, as `「３」は「√」の記号の中に入っている` quotes `√`.
    fn note_quotes_container(&self, note: &MarginNote, mark: char) -> bool {
        note.kind == MarginNoteKind::GlyphShape
            && self
                .alloc
                .store()
                .content_range_as_plain(note.note)
                .is_some_and(|text| text == format!("「{mark}」の記号の中に入っている"))
    }

    /// Hold a supplied-diacritic note with its reclaimed letter run as the
    /// base of a reading that may follow the note directly.
    fn defer_supplied_diacritic_base(&mut self, span: &ClassifiedSpan, payload: Segment) -> bool {
        let Segment::Format { value, .. } = payload else {
            return false;
        };
        if value.attrs.single() != Some(ForwardAttr::AccentDot)
            || value.origin != ForwardOrigin::Reclaimed
        {
            return false;
        }
        if let Some(pending) = self.pending_ruby_base.take() {
            self.emit_pending_base(pending);
        }
        self.flush_plain_up_to(span.source_span.start);
        self.pending_plain.clear();
        self.pending_ruby_base = Some(PendingRubyBase::single(
            PendingBaseSegment {
                span: span.clone(),
                payload,
            },
            None,
            None,
        ));
        true
    }

    /// Whether the annotation recognizers leave the bracket at `open_idx`
    /// uninterpreted, falling back to `Directive{Unknown}` without a
    /// diagnostic, once the refmark at `refmark_span` has joined the plain
    /// run. The trial uses a scratch allocator and leaves the stream untouched.
    fn annotation_uninterpreted(
        &self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
        refmark_span: Span,
    ) -> bool {
        let pending_plain_start = self.pending_plain_start().or(Some(refmark_span.start));
        let mut alloc = Allocator::new();
        let mut ctx = RecogniseCtx {
            alloc: &mut alloc,
            source: self.source,
            diagnostics: Vec::new(),
            pending_plain_start,
            pending_decorations: Vec::new(),
        };
        let uninterpreted = ctx
            .recognize_annotation(view, open_idx, close_idx, pending_plain_start)
            .is_some_and(|m| {
                m.pending_diagnostic.is_none()
                    && matches!(
                        m.emit,
                        EmitKind::Aozora(Node::Directive(directive))
                            if directive.kind == DirectiveKind::Unknown
                    )
            });
        uninterpreted && ctx.diagnostics.is_empty()
    }

    /// The gaiji the refmark at `refmark_span` and the bracket at `open_idx`
    /// form, and, when the strict recognizer declines, whether the
    /// annotation recognizers leave the bracket uninterpreted.
    fn refmark_gaiji(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
        refmark_span: Span,
    ) -> (Option<GaijiEmit>, Option<bool>) {
        if let Some(emitted) = self.try_gaiji_emit(view, open_idx, refmark_span, false) {
            return (Some(emitted), None);
        }
        let uninterpreted = self.annotation_uninterpreted(view, open_idx, close_idx, refmark_span);
        let emitted = uninterpreted
            .then(|| self.try_gaiji_emit(view, open_idx, refmark_span, true))
            .flatten();
        (emitted, Some(uninterpreted))
    }

    /// `described` admits a description in any wording; see
    /// [`RecogniseCtx::recognize_described_gaiji`].
    fn try_gaiji_emit(
        &mut self,
        body: BodyView<'_>,
        bracket_open_idx: usize,
        refmark_span: Span,
        described: bool,
    ) -> Option<GaijiEmit> {
        let mut ctx = RecogniseCtx {
            alloc: self.alloc,
            source: self.source,
            diagnostics: Vec::new(),
            pending_plain_start: None,
            pending_decorations: Vec::new(),
        };
        let m = if described {
            ctx.recognize_described_gaiji(body, refmark_span, bracket_open_idx)
        } else {
            ctx.recognize_gaiji(body, refmark_span, bracket_open_idx)
        }?;
        // An explicit `｜` (U+FF5C) before the kanji prefix or gaiji is a
        // base-start marker for a following ruby. Hold it out of the plain
        // run so `try_ruby_over_pending_base` can drop the redundant marker on
        // adoption (the gaiji is unambiguously the base), or `emit_pending_base`
        // can re-emit it as plain when the gaiji stands alone. Consume the
        // whole trailing `｜` run, not only the last bar: dropping one bar
        // per parse would leave the next `｜` adjacent to the gaiji, so
        // re-serialising `｜｜※…《…》` would remove one bar per pass and
        // never reach a fixed point (fmt-idempotence).
        let prefix = self.pending_plain_start().and_then(|start| {
            let before = &self.source[start as usize..m.consume_start as usize];
            if self
                .pending_ruby_base
                .as_ref()
                .is_some_and(|base| base.bar.is_some())
            {
                return Some(Span::new(start, m.consume_start));
            }
            before
                .char_indices()
                .rev()
                .take_while(|(_, ch)| is_ruby_base_char(*ch))
                .last()
                .map(|(offset, _)| {
                    Span::new(
                        start + u32::try_from(offset).expect("source offset fits u32"),
                        m.consume_start,
                    )
                })
        });
        let prefix_start = prefix.map_or(m.consume_start, |p| p.start);
        let before = &self.source[..prefix_start as usize];
        let bar_start = before.trim_end_matches('\u{ff5c}').len();
        let bar = (bar_start < before.len()).then(|| {
            Span::new(
                u32::try_from(bar_start).expect("bar-run start is within the source (u32)"),
                prefix_start,
            )
        });
        self.flush_plain_up_to(bar.map_or(prefix_start, |b| b.start));
        let node = self.alloc.gaiji(m.payload);
        self.pending_plain.clear();
        // The gaiji still renders best-effort (as its description text)
        // when resolution misses; report the miss, because the glyph does
        // not appear. `m.payload` is a `Copy` value and the
        // `ctx` reborrow of `self.alloc` ended at the `gaiji()` call above,
        // so reading `ucs` and pushing onto `self.diagnostics` does not
        // conflict with the borrow. This site reports top-level `※［＃…］`;
        // gaiji nested in a ruby reading / annotation body are resolved and
        // rendered by `build_content_from_body`, whose `try_emit_gaiji_at`
        // reports the same diagnostic. A gaiji inside a forward-reference
        // quote target (nested `［＃…］` breaks pairing) falls to `Unknown`.
        if m.payload.resolve(self.alloc.store()).is_none() {
            self.diagnostics
                .push(Diagnostic::unresolved_gaiji(Span::new(
                    m.consume_start,
                    m.consume_end,
                )));
        }
        Some((
            PendingBaseSegment {
                span: ClassifiedSpan {
                    kind: SpanKind::Aozora(node),
                    source_span: Span::new(m.consume_start, m.consume_end),
                },
                payload: self.alloc.seg_gaiji(
                    m.payload,
                    Span::new(m.consume_start, m.consume_end),
                    Span::new(m.consume_start, m.consume_end),
                    ab_aozora_syntax::GaijiMarkerPlacement::Inline,
                ),
            },
            bar,
            prefix,
        ))
    }

    /// Replay an unadopted base, including its marker and intervening text.
    fn emit_pending_base(&mut self, pending: PendingRubyBase) {
        if let Some(bar) = pending.bar {
            self.push_output(ClassifiedSpan {
                kind: SpanKind::Plain(PlainSpan {
                    provenance: PlainProvenance::RecoveredVerbatim,
                }),
                source_span: bar,
            });
        }
        if let Some(prefix) = pending.prefix {
            self.push_output(ClassifiedSpan {
                kind: SpanKind::Plain(PlainSpan {
                    provenance: PlainProvenance::Text,
                }),
                source_span: prefix,
            });
        }
        let mut previous_end = None;
        for segment in pending.segs {
            if let Some(start) =
                previous_end.filter(|start| *start < segment.span.source_span.start)
            {
                self.push_output(ClassifiedSpan {
                    kind: SpanKind::Plain(PlainSpan {
                        provenance: PlainProvenance::Text,
                    }),
                    source_span: Span::new(start, segment.span.source_span.start),
                });
            }
            previous_end = Some(segment.span.source_span.end);
            self.push_output(segment.span);
        }
    }

    /// Final flush: emit any trailing Plain run covering the source
    /// tail. Called once when the upstream iterator returns None.
    fn finalize(&mut self) {
        self.fold_pending_refmark();
        let end = self.source_len;
        self.flush_plain_up_to(end);
    }
}

impl<I> Iterator for ClassifyStream<'_, '_, I>
where
    I: Iterator<Item = PairEvent>,
{
    type Item = ClassifiedSpan;

    fn next(&mut self) -> Option<ClassifiedSpan> {
        #[cfg(feature = "classify-instrument")]
        let _classify_guard = SubsystemGuard::new(Subsystem::IterDispatch);
        loop {
            if let Some(span) = self.pending_outputs_pop_front() {
                return Some(span);
            }
            if self.finished {
                return None;
            }
            #[cfg(feature = "classify-instrument")]
            let events_next_guard = SubsystemGuard::new(Subsystem::EventsNext);
            let next_event = self.events.next();
            #[cfg(feature = "classify-instrument")]
            drop(events_next_guard);
            if let Some(event) = next_event {
                #[cfg(feature = "classify-instrument")]
                let _classify_loop_guard = SubsystemGuard::new(Subsystem::LoopBody);
                self.process_event(event);
            } else {
                // Upstream exhausted. A deferred gaiji with no following
                // ruby is a standalone span: flush it first (it precedes
                // any unclosed frame's body in source order). Then close
                // any active frame as unclosed (its body events fold back
                // to plain; a gaiji-mode refmark is also folded into plain),
                // then run final flush.
                if let Some(pending) = self.pending_ruby_base.take() {
                    self.emit_pending_base(pending);
                }
                if let Some(frame) = self.frame.take() {
                    self.replay_abandoned_frame(frame);
                }
                self.finalize();
                self.finished = true;
            }
        }
    }
}

impl<I> ClassifyStream<'_, '_, I>
where
    I: Iterator<Item = PairEvent>,
{
    fn pending_outputs_pop_front(&mut self) -> Option<ClassifiedSpan> {
        #[cfg(feature = "classify-instrument")]
        {
            // Record pending_outputs.len() before the pop so the
            // distribution histogram tracks pre-pop sizes.
            let len = self.pending_outputs.len() as u64;
            if len > 0 {
                record_pending_size(len);
            }
        }
        self.pending_outputs.pop_front()
    }

    fn continues_pending_base(&self, pending: &PendingRubyBase, event: &PairEvent) -> bool {
        let end = pending.end();
        match event {
            PairEvent::PairOpen {
                kind: PairKind::Ruby,
                span,
            } => {
                span.start == end
                    || (pending.extensible()
                        && self.pending_plain_start() == Some(end)
                        && self
                            .pending_plain
                            .back()
                            .is_some_and(|p| p.source_span.end == span.start))
            }
            PairEvent::Text { range, .. } => {
                pending.extensible()
                    && (range.start == end
                        || self
                            .pending_plain
                            .back()
                            .is_some_and(|p| p.source_span.end == range.start))
                    && if pending.bar.is_some() {
                        !self.source[range.start as usize..range.end as usize].contains('\n')
                    } else {
                        let text = &self.source[range.start as usize..range.end as usize];
                        text.chars().all(is_ruby_base_char) || pending.spaced_from_reading(text)
                    }
            }
            PairEvent::Solo {
                kind: TriggerKind::RefMark,
                span,
            } => {
                pending.extensible()
                    && (span.start == end
                        || self.pending_plain_start() == Some(end)
                            && self
                                .pending_plain
                                .back()
                                .is_some_and(|p| p.source_span.end == span.start))
            }
            PairEvent::PairOpen {
                kind: PairKind::Bracket,
                span,
            } => {
                self.detached_markers.contains_key(&span.start)
                    || pending.bar.is_some()
                    || self.pending_refmark.is_some_and(|rm| {
                        rm.start == end
                            || (self.pending_plain_start() == Some(end)
                                && self
                                    .pending_plain
                                    .back()
                                    .is_some_and(|p| p.source_span.end == rm.start))
                    })
            }
            _ => false,
        }
    }

    fn process_event(&mut self, event: PairEvent) {
        // Frame buffering takes precedence. It is checked before `streaming`
        // so that a sub-frame opened mid-stream for nested ruby or angle quotes
        // (`《…》` / `≪…≫` inside a `「…」` quote; see
        // `handle_stream_event`) accumulates its body instead of
        // the quote's stream-through path consuming it. Once the
        // sub-frame's outer pair closes, recognition runs and the frame
        // clears, so `streaming` resumes on the next event.
        if self.frame.is_some() {
            // A pending refmark cannot coexist with an open frame: it is
            // consumed into the frame's `gaiji_refmark` or folded to
            // plain before the frame opens.
            debug_assert!(
                self.pending_refmark.is_none(),
                "a pending refmark should have been absorbed or flushed before frame entry"
            );
            match self.append_to_frame(event) {
                FrameStep::Closed => self.recognize_and_emit(),
                FrameStep::Abandoned => {
                    let frame = self.frame.take().expect("abandoned frame is active");
                    self.replay_abandoned_frame(frame);
                }
                FrameStep::Open => {}
            }
            return;
        }

        if self.try_defer_detached_target(&event) {
            return;
        }

        // Only adjacent events continue a deferred base. Explicit bases admit
        // text and annotations; implicit gaiji bases retain their kanji boundary.
        // Check after the frame guard so a buffered reading cannot flush its base.
        let flush_gaiji = self
            .pending_ruby_base
            .as_ref()
            .is_some_and(|pending| !self.continues_pending_base(pending, &event));
        if flush_gaiji {
            let pending = self.pending_ruby_base.take().expect("checked Some");
            self.emit_pending_base(pending);
        }

        // Stream-through path for top-level Quote / Tortoise: see
        // `StreamingFrame` for the rationale. Body events pass directly
        // through, except a nested Ruby / AngleQuote opens a sub-frame
        // (buffered by the frame check above).
        if self.streaming.is_some() {
            self.handle_stream_event(event);
            return;
        }

        // Top level. A `※` refmark held from the previous event is
        // consumed only by an immediately-following Bracket open (gaiji);
        // otherwise it folds into the plain run (its span is picked up
        // because `pending_plain_start` is set to `rm.start` first).
        if self.pending_refmark.is_some()
            && !matches!(
                event,
                PairEvent::PairOpen {
                    kind: PairKind::Bracket,
                    ..
                }
            )
        {
            self.fold_pending_refmark();
        }

        self.handle_top_level(event, /*replay=*/ false);
    }

    fn try_emit_detached_declaration(&mut self, body: BodyView<'_>, open_kind: PairKind) -> bool {
        let Some(PairEvent::PairOpen {
            span: open_span, ..
        }) = body.events.first()
        else {
            return false;
        };
        let Some(close_span) = body.events.last().and_then(PairEvent::span) else {
            return false;
        };
        if !matches!(open_kind, PairKind::Bracket) {
            return false;
        }
        let Some(spec) = self.detached_markers.remove(&open_span.start) else {
            return false;
        };
        if spec.marker.end != close_span.end {
            self.detached_markers.insert(spec.marker.start, spec);
            return false;
        }
        let raw = &self.source[spec.marker.start as usize..spec.marker.end as usize];
        let directive = self
            .alloc
            .make_directive(raw, DirectiveKind::GaijiDeclaration);
        let belongs_to_pending_base = self.pending_ruby_base.as_ref().is_some_and(|pending| {
            self.pending_plain_start() == Some(pending.end())
                && self
                    .pending_plain
                    .back()
                    .is_some_and(|plain| plain.source_span.end == spec.marker.start)
        });
        if belongs_to_pending_base {
            self.pending_plain.clear();
            self.pending_ruby_base
                .as_mut()
                .expect("checked pending base")
                .segs
                .push(PendingBaseSegment {
                    span: ClassifiedSpan {
                        kind: SpanKind::Aozora(Node::Directive(directive)),
                        source_span: spec.marker,
                    },
                    payload: Segment::Directive {
                        value: directive,
                        source_span: spec.marker,
                    },
                });
            return true;
        }
        self.flush_plain_up_to(spec.marker.start);
        self.push_output(ClassifiedSpan {
            kind: SpanKind::Aozora(Node::Directive(directive)),
            source_span: spec.marker,
        });
        true
    }

    fn try_defer_detached_target(&mut self, event: &PairEvent) -> bool {
        let PairEvent::Solo {
            kind: TriggerKind::RefMark,
            span,
        } = event
        else {
            return false;
        };
        let Some(spec) = self.detached_targets.remove(&span.start) else {
            return false;
        };
        self.defer_detached_gaiji(*span, &spec);
        true
    }

    fn defer_detached_gaiji(&mut self, target: Span, spec: &DetachedGaijiSpec) {
        if let Some(old) = self.pending_ruby_base.take() {
            self.emit_pending_base(old);
        }
        let prefix = self.pending_plain_start().and_then(|start| {
            self.source[start as usize..target.start as usize]
                .char_indices()
                .rev()
                .take_while(|(_, ch)| is_ruby_base_char(*ch))
                .last()
                .map(|(offset, _)| {
                    Span::new(
                        start + u32::try_from(offset).expect("source offset fits native span"),
                        target.start,
                    )
                })
        });
        let prefix_start = prefix.map_or(target.start, |span| span.start);
        self.flush_plain_up_to(prefix_start);
        self.pending_plain.clear();
        let payload = self.alloc.make_gaiji(
            &spec.description,
            spec.quoted,
            spec.mencode.as_deref(),
            true,
        );
        let replaced = self.alloc.content_plain("※");
        let payload = self
            .alloc
            .gaiji_payload_declared_at(payload, replaced, spec.marker);
        if payload.resolve(self.alloc.store()).is_none() {
            self.diagnostics
                .push(Diagnostic::unresolved_gaiji(spec.marker));
        }
        let node = self.alloc.gaiji(payload);
        self.pending_ruby_base = Some(PendingRubyBase::single(
            PendingBaseSegment {
                span: ClassifiedSpan {
                    kind: SpanKind::Aozora(node),
                    source_span: target,
                },
                payload: self.alloc.seg_gaiji(
                    payload,
                    target,
                    spec.marker,
                    ab_aozora_syntax::GaijiMarkerPlacement::Detached,
                ),
            },
            None,
            prefix,
        ));
    }
}

/// Intermediate result of `recognize_ruby`. `base` stays borrowed
/// (both handled forms, explicit `｜X《Y》` and implicit
/// trailing-kanji, both come from a single [`PairEvent::Text`] event
/// with no nested structure). `reading`, on the other hand, can carry
/// embedded gaiji (`※［＃…］`) or annotations (`［＃ママ］`), so it is
/// already resolved into a `Content` via `build_content_from_body`.
///
/// Resolving the reading in the lexer keeps the [`Node`] payload
/// self-contained: the normalize stage writes one PUA sentinel over the whole `｜…《…》` source
/// span, and the inner gaiji/annotation never reach the top-level
/// `spans` list or downstream consumers.
struct RubyMatch<'s> {
    base: &'s str,
    reading: Content,
    consume_start: u32,
    consume_end: u32,
}

/// Try to recognize a Ruby span at `events[open_idx]`.
///
/// Two shapes per the Aozora annotation manual
/// (<https://www.aozora.gr.jp/annotation/ruby.html>):
///
/// * **Explicit**: `｜X《Y》`. A [`TriggerKind::Bar`] `Solo` two
///   events before the [`PairKind::Ruby`] open marks the full base.
///   Any Text, not only kanji, may be the base.
/// * **Implicit**: `…X《Y》` where the preceding Text event ends in
///   a run of ideographs. The base is the trailing kanji run of that
///   Text; any non-kanji prefix remains plain.
///
/// The `《…》` reading body is walked with `build_content_from_body`
/// so nested `※［＃…］` gaiji and `［＃…］` annotations fold into the
/// returned `Content` as `Segment::Gaiji` / `Segment::Directive`.
/// Pure-text readings collapse back to `Content::Plain` via
/// `Content::from_segments`.
///
/// Returns `None` if neither shape applies (empty reading, no
/// preceding Text, no kanji for implicit).
impl<'s> RecogniseCtx<'_, 's> {
    fn recognize_ruby(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<RubyMatch<'s>> {
        #[cfg(feature = "classify-instrument")]
        let _classify_guard = SubsystemGuard::new(Subsystem::Ruby);
        let events = view.events;
        let PairEvent::PairOpen {
            span: open_span, ..
        } = events[open_idx]
        else {
            return None;
        };
        let PairEvent::PairClose {
            span: close_span, ..
        } = events[close_idx]
        else {
            return None;
        };
        if open_span.end >= close_span.start {
            // Empty reading: the `《…》` body has no bytes.
            return None;
        }
        if open_idx == 0 {
            return None;
        }
        let PairEvent::Text {
            range: prev_range, ..
        } = events[open_idx - 1]
        else {
            return None;
        };
        let prev_text = &self.source[prev_range.start as usize..prev_range.end as usize];

        let reading = self.build_content_from_body(
            view,
            &BodyWindow {
                events: open_idx + 1..close_idx,
                bytes: open_span.end..close_span.start,
            },
        );

        // Explicit form: Solo(Bar) two events before the open, with the
        // Text between them acting as the base.
        if open_idx >= 2
            && let PairEvent::Solo {
                kind: TriggerKind::Bar,
                span: bar_span,
            } = events[open_idx - 2]
        {
            if prev_text.is_empty() {
                return None;
            }
            return Some(RubyMatch {
                base: prev_text,
                reading,
                consume_start: bar_span.start,
                consume_end: close_span.end,
            });
        }

        let base_offset = trailing_ruby_base_start(prev_text);
        if base_offset == prev_text.len() {
            return None;
        }
        let consume_start =
            prev_range.start + u32::try_from(base_offset).expect("base offset fits in u32");
        Some(RubyMatch {
            base: &prev_text[base_offset..],
            reading,
            consume_start,
            consume_end: close_span.end,
        })
    }
}

/// Half-open window into a [`PairEvent`] stream. Bundles the event-
/// index range with the matching byte-offset range so
/// `build_content_from_body` can flush text segments using source
/// byte slices without dereferencing event spans on every iteration.
///
/// The two ranges are redundant in principle: `bytes.start` always
/// equals `events[events.start]`'s leading edge, but caching them
/// avoids a branch when the range is empty and explicitly declares
/// the helper's required bounds.
struct BodyWindow {
    events: Range<usize>,
    bytes: Range<u32>,
}

/// Immutable per-call body-walk context shared across the
/// `build_content_from_body` orchestrator and its per-shape helpers
/// (`try_emit_gaiji_at` / `try_emit_annotation_at`). Bundling `view`
/// and `window` together prevents the per-helper signatures from
/// exceeding the project's 4-arg threshold without losing positional
/// clarity at call sites.
#[derive(Clone, Copy)]
struct BodyWalkCtx<'b> {
    view: BodyView<'b>,
    window: &'b BodyWindow,
}

/// Mutable per-call build state for `build_content_from_body`.
/// Tracks the under-construction segment vector and the byte position
/// where the current pending Text run started. Threading these through
/// per-shape helpers as a single `&mut` field keeps the helper
/// signatures at ≤4 args.
struct ContentBuild {
    segments: Vec<Segment>,
    /// Byte position of the earliest source byte not yet committed
    /// to a `Segment::Text`. Each successful gaiji / annotation emit
    /// advances this past the consumed bracket.
    text_start: u32,
}

impl RecogniseCtx<'_, '_> {
    /// Walk `window` over `events` and build the corresponding
    /// `Content` in a single forward sweep.
    ///
    /// Each nested `※［＃description、mencode］` reduces to a
    /// `Segment::Gaiji` via `recognize_gaiji`; each standalone
    /// `［＃…］` reduces to a `Segment::Directive` via
    /// `recognize_annotation`. Every other byte (plain text, stray
    /// triggers, unmatched delimiters) is captured into adjacent
    /// `Segment::Text` runs by tracking a single "outstanding text
    /// start" byte offset and flushing only when a recognisable construct
    /// consumes the intervening bytes.
    ///
    /// Non-Directive Aozora emits (a paired-container opener, a block
    /// leaf, etc.) are not first-class segments and are folded back
    /// into `Directive{Unknown}` with the raw bracket bytes; this keeps
    /// the Tier-A canary intact inside a ruby body regardless of how
    /// unusual the inner annotation shape is.
    ///
    /// ## Fast path
    ///
    /// [`has_nested_candidate`] first short-circuits the body scan: when
    /// no `Solo(RefMark)` and no `PairOpen(Bracket)` appear, the body is
    /// guaranteed to be plain text (possibly containing unrelated
    /// triggers like `｜` or mismatched quotes, which are treated as
    /// text), emitted as a single `Plain`. Returning `Content::from(&str)`
    /// in that branch skips the `Vec` allocation and the `from_segments`
    /// collapse pass for the 99%+ of ruby readings that carry no embedded
    /// structure.
    ///
    /// ## Slow path
    ///
    /// The fallback is a single `O(body_events)` sweep through two
    /// per-shape recognise helpers, advancing `i` when neither applies.
    /// `text_start` tracks the earliest byte that has not yet been
    /// committed to a Text segment; only a recognised nested construct
    /// triggers a flush, so unrelated events cost a single index
    /// increment. Each recognition jumps to `close_idx + 1` using the
    /// pair stage's pre-linked pair indices, keeping the sweep strictly
    /// forward-only regardless of nesting depth.
    ///
    /// The returned value is always normalised via
    /// `Content::from_segments`, so a slow-path body that turned out to
    /// contain only text (for example because its brackets were malformed
    /// and skipped) still collapses back to `Content::Plain`.
    fn build_content_from_body(&mut self, view: BodyView<'_>, window: &BodyWindow) -> Content {
        #[cfg(feature = "classify-instrument")]
        let _classify_guard = SubsystemGuard::new(Subsystem::BuildContent);
        debug_assert!(
            window.events.start <= window.events.end,
            "body window event range must be non-inverted",
        );
        debug_assert!(
            window.bytes.start <= window.bytes.end,
            "body window byte range must be non-inverted",
        );
        debug_assert_eq!(
            view.events.len(),
            view.links.len(),
            "BodyView events/links must be parallel",
        );

        let body_events = &view.events[window.events.start..window.events.end];
        if !has_nested_candidate(body_events)
            && !self.source[window.bytes.start as usize..window.bytes.end as usize].contains('／')
        {
            // Fast path: no `※` and no `［` in the body; bytes pass
            // through verbatim. `content_plain("")` canonicalises to
            // empty `Segments(&[])`, the shape of `Content::from(&str)`.
            let text = &self.source[window.bytes.start as usize..window.bytes.end as usize];
            return self.alloc.content_plain(text);
        }

        // Slow path: at least one potential nested construct exists.
        // Pre-size the segment vector: worst case is `ceil(n / 2)` runs
        // of `Text, Construct, Text, …` plus one trailing Text.
        // `body_events.len() + 1` is an upper bound that is small
        // in practice (ruby readings almost never reach double-digit
        // events).
        let body = BodyWalkCtx { view, window };
        let mut build = ContentBuild {
            segments: Vec::with_capacity(body_events.len() + 1),
            text_start: window.bytes.start,
        };
        let mut i = window.events.start;
        while i < window.events.end {
            if let Some(next_i) = self.try_emit_gaiji_at(body, &mut build, i) {
                i = next_i;
                continue;
            }
            if let Some(next_i) = self.try_emit_annotation_at(body, &mut build, i) {
                i = next_i;
                continue;
            }
            i += 1;
        }
        push_text_segment(
            &mut build.segments,
            self.source,
            build.text_start..window.bytes.end,
            self.alloc,
        );
        self.alloc.content_segments(&build.segments)
    }

    fn attach_margin_note_pair_payload(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
        matched: &mut AnnotationMatch,
    ) {
        let EmitKind::Aozora(Node::Directive(mut directive)) = matched.emit else {
            return;
        };
        if directive.kind != DirectiveKind::MarginNotePairClose {
            return;
        }
        let raw = &self.source[matched.consume_start as usize..matched.consume_end as usize];
        let prefix = if raw.starts_with("［＃左に「") {
            "［＃左に「"
        } else if raw.starts_with("［＃「") {
            "［＃「"
        } else {
            return;
        };
        let suffix = "」の注記付き終わり］";
        let Some(note) = raw
            .strip_prefix(prefix)
            .and_then(|raw| raw.strip_suffix(suffix))
            .filter(|note| !note.is_empty())
        else {
            return;
        };
        let note_start = matched.consume_start
            + u32::try_from(prefix.len()).expect("paired note prefix fits native span");
        let note_end =
            note_start + u32::try_from(note.len()).expect("paired note content fits native span");
        let event_start = (open_idx + 1..close_idx)
            .find(|index| {
                view.events[*index]
                    .span()
                    .is_some_and(|span| span.end > note_start)
            })
            .unwrap_or(close_idx);
        let event_end = (event_start..close_idx)
            .take_while(|index| {
                view.events[*index]
                    .span()
                    .is_some_and(|span| span.start < note_end)
            })
            .last()
            .map_or(event_start, |index| index + 1);
        let content = self.build_content_from_body(
            view,
            &BodyWindow {
                events: event_start..event_end,
                bytes: note_start..note_end,
            },
        );
        directive.pair_payload = Some(DirectivePairPayload {
            content: self.alloc.content_range(content),
            source_span: NonEmptySpan::new(Span::new(note_start, note_end))
                .expect("paired note content is nonempty"),
        });
        matched.emit = EmitKind::Aozora(Node::Directive(directive));
        matched.annotation_payload = Some(directive);
    }

    /// Shape 1: `※［＃…］`: `Solo(RefMark)` immediately followed by a
    /// matched `PairOpen(Bracket)`. On a successful gaiji recognise,
    /// flush the pending Text run, push a `Segment::Gaiji`, advance
    /// `text_start`, and return the index of the first event past the
    /// bracket close. Returns `None` if the shape does not match or if
    /// the inner [`Self::recognize_gaiji`] declines.
    fn try_emit_gaiji_at(
        &mut self,
        body: BodyWalkCtx<'_>,
        build: &mut ContentBuild,
        i: usize,
    ) -> Option<usize> {
        let PairEvent::Solo {
            kind: TriggerKind::RefMark,
            span: refmark_span,
        } = body.view.events[i]
        else {
            return None;
        };
        let bracket_idx = i + 1;
        if bracket_idx >= body.window.events.end {
            return None;
        }
        let PairEvent::PairOpen {
            kind: PairKind::Bracket,
            ..
        } = body.view.events[bracket_idx]
        else {
            return None;
        };
        let close_link = body.view.links[bracket_idx];
        if close_link == u32::MAX {
            return None;
        }
        let close_idx = close_link as usize;
        if close_idx >= body.window.events.end {
            return None;
        }
        let g = self.recognize_gaiji(body.view, refmark_span, bracket_idx)?;
        push_text_segment(
            &mut build.segments,
            self.source,
            build.text_start..g.consume_start,
            self.alloc,
        );
        let source_span = Span::new(g.consume_start, g.consume_end);
        build.segments.push(self.alloc.seg_gaiji(
            g.payload,
            source_span,
            source_span,
            ab_aozora_syntax::GaijiMarkerPlacement::Inline,
        ));
        build.text_start = g.consume_end;
        // A nested gaiji whose mencode resolves nothing renders best-effort as
        // its description; report the miss so nested references match the
        // top-level `※［＃…］` behaviour. The owning `ClassifyStream`
        // drains `self.diagnostics` after the recognise call.
        if g.payload.resolve(self.alloc.store()).is_none() {
            self.diagnostics
                .push(Diagnostic::unresolved_gaiji(Span::new(
                    g.consume_start,
                    g.consume_end,
                )));
        }
        Some(close_idx + 1)
    }

    /// Shape 2: `［＃…］`: a standalone bracket annotation. Tried
    /// after [`Self::try_emit_gaiji_at`] so the `※`+bracket combination
    /// is tried first on a leading bracket. `recognize_annotation` has an
    /// `Unknown` catch-all (only returns `None` for malformed brackets
    /// with no `＃` sentinel); on success the bracket folds into a
    /// `Segment::Directive` with the recogniser's payload (or a
    /// synthetic `Unknown` payload built from the raw source bytes
    /// when the recogniser left `annotation_payload` unset). The
    /// fallback synthesis preserves the Tier-A canary: no bare `［＃`
    /// appears outside an `aozora-directive` wrapper.
    fn try_emit_annotation_at(
        &mut self,
        body: BodyWalkCtx<'_>,
        build: &mut ContentBuild,
        i: usize,
    ) -> Option<usize> {
        let PairEvent::PairOpen {
            kind: PairKind::Bracket,
            span: open_span,
        } = body.view.events[i]
        else {
            return None;
        };
        let close_link = body.view.links[i];
        if close_link == u32::MAX {
            return None;
        }
        let close_idx = close_link as usize;
        if close_idx >= body.window.events.end {
            return None;
        }
        let a = self.recognize_annotation(body.view, i, close_idx, Some(build.text_start))?;
        let PairEvent::PairClose {
            span: close_span, ..
        } = body.view.events[close_idx]
        else {
            // Pair-stage invariant: PairOpen's link always targets a
            // PairClose of the matching kind.
            unreachable!("PairOpen link must target a PairClose");
        };
        // A construct without a segment form keeps the target text it reclaimed.
        let consume_start = if a.annotation_payload.is_some()
            || matches!(a.emit, EmitKind::Aozora(Node::Gaiji(_) | Node::Kunten(_)))
        {
            a.consume_start
        } else {
            open_span.start
        };
        push_text_segment(
            &mut build.segments,
            self.source,
            build.text_start..consume_start,
            self.alloc,
        );
        // A no-`※` standalone gaiji reaches here as `Aozora(Gaiji)`;
        // wrap it as a `Segment::Gaiji` (not the `Unknown` annotation the
        // payload fallback would build) and flag an unresolved miss.
        if let EmitKind::Aozora(Node::Gaiji(g)) = a.emit {
            let marker_span = Span::new(open_span.start, close_span.end);
            let target_span = if g.replaced.is_some() {
                Span::new(a.consume_start, open_span.start)
            } else {
                marker_span
            };
            build.segments.push(self.alloc.seg_gaiji(
                g,
                target_span,
                marker_span,
                ab_aozora_syntax::GaijiMarkerPlacement::Inline,
            ));
            if g.resolve(self.alloc.store()).is_none() {
                self.diagnostics
                    .push(Diagnostic::unresolved_gaiji(Span::new(
                        a.consume_start,
                        a.consume_end,
                    )));
            }
        } else if let EmitKind::Aozora(Node::Kunten(value)) = a.emit {
            build.segments.push(Segment::Kunten {
                value,
                source_span: Span::new(a.consume_start, a.consume_end),
            });
        } else {
            let payload = if let Some(p) = a.annotation_payload {
                p
            } else {
                let raw = &self.source[open_span.start as usize..close_span.end as usize];
                self.alloc.make_directive(raw, DirectiveKind::Unknown)
            };
            build.segments.push(
                self.alloc
                    .seg_annotation(payload, Span::new(consume_start, a.consume_end)),
            );
        }
        build.text_start = a.consume_end;
        Some(close_idx + 1)
    }
}

/// Whether `body` could host a nested gaiji / annotation. The pair-stage
/// event model guarantees that:
///
/// * `※［＃…］` always emits a `Solo(RefMark)` event at its `※`.
/// * `［＃…］` always emits a `PairOpen(Bracket)` event at its `［`.
///
/// So the absence of both event shapes in the body is sufficient proof
/// that no nested construct can be recognised, allowing
/// `build_content_from_body` to take the allocation-free fast path.
fn has_nested_candidate(body: &[PairEvent]) -> bool {
    body.iter().any(|e| {
        matches!(
            e,
            PairEvent::Solo {
                kind: TriggerKind::RefMark,
                ..
            } | PairEvent::PairOpen {
                kind: PairKind::Bracket,
                ..
            }
        )
    })
}

/// Append `source[start..end]` to `segments` as a `Segment::Text` if
/// the slice is non-empty. `start == end` occurs when a
/// recognised construct is at the start of the body or
/// immediately follows a previous one; skipping those zero-length
/// flushes keeps the post-collapse invariant "no empty `Text` in a
/// `Segments` run" (see `Content::from_segments`) without a second
/// compaction pass.
#[inline]
fn push_text_segment(
    segments: &mut Vec<Segment>,
    source: &str,
    bytes: Range<u32>,
    alloc: &mut Allocator,
) {
    let mut start = bytes.start;
    for (offset, _) in source[bytes.start as usize..bytes.end as usize].match_indices('／') {
        let position = bytes.start + u32::try_from(offset).expect("source fits native span");
        let Some(value) = IterationMark::at_start(&source[position as usize..bytes.end as usize])
        else {
            continue;
        };
        if start < position {
            segments.push(alloc.seg_text(&source[start as usize..position as usize]));
        }
        start = position + u32::try_from(value.source().len()).expect("notation fits native span");
        segments.push(Segment::IterationMark {
            value,
            source_span: Span::new(position, start),
        });
    }
    if start < bytes.end {
        segments.push(alloc.seg_text(&source[start as usize..bytes.end as usize]));
    }
}

fn trailing_ruby_base_start(text: &str) -> usize {
    let word = text.trim_end_matches(' ');
    if word.len() < text.len() {
        return spaced_latin_word_start(word)
            .or_else(|| latin_phrase_start(word, 2))
            .unwrap_or(text.len());
    }
    if let Some(phrase) = text.strip_suffix('!')
        && let Some(start) = latin_phrase_start(phrase, 1)
    {
        return start;
    }
    let mut characters = text.char_indices().rev().peekable();
    let Some((mut start, last)) = characters.next() else {
        return text.len();
    };
    if is_symbol_ruby_base(last) {
        while let Some((index, _)) = characters.next_if(|&(_, character)| character == last) {
            start = index;
        }
        return start;
    }
    let latin_abbreviation = matches!(last, '.' | '\'')
        && characters
            .peek()
            .is_some_and(|&(_, previous)| ruby_base_class(previous) == Some(RubyBaseClass::Latin));
    let Some(base_class) =
        ruby_base_class(last).or_else(|| latin_abbreviation.then_some(RubyBaseClass::Latin))
    else {
        return text.len();
    };
    while let Some((index, character)) = characters.next() {
        let same_class = ruby_base_class(character) == Some(base_class);
        let internal_latin_punctuation = base_class == RubyBaseClass::Latin
            && matches!(character, '\'' | '-' | '.')
            && characters
                .peek()
                .is_some_and(|&(_, previous)| ruby_base_class(previous) == Some(base_class));
        if !same_class && !internal_latin_punctuation {
            break;
        }
        start = index;
    }
    start
}

/// Start of the single Latin word that `word` ends with, when spaces separate
/// it from the reading that follows, as in `の Zwagri 《ツワグリ》`. The
/// spaces belong to the base. A word that follows another Latin word leaves
/// the extent of the base open, so it yields `None`.
fn spaced_latin_word_start(word: &str) -> Option<usize> {
    let start = trailing_ruby_base_start(word);
    let last = word.chars().next_back()?;
    let latin = ruby_base_class(last) == Some(RubyBaseClass::Latin) || matches!(last, '.' | '\'');
    let before = word[..start].trim_end_matches([' ', '\u{3000}']);
    let follows_word = before.len() < start
        && before
            .chars()
            .next_back()
            .is_some_and(|previous| ruby_base_class(previous) == Some(RubyBaseClass::Latin));
    (start < word.len() && latin && !follows_word).then_some(start)
}

/// Start of the phrase of at least `words_min` Latin words, separated by
/// single spaces, that `text` ends with and that no Latin word precedes, as in
/// `の Hsifan territory  《シフアン・テリトリー》` or `、Der Teufel hole sie!《デル トイフェル ホオレ ジイ》`.
/// The published XHTML sets such a phrase, with a closing `!` and the spaces
/// before the reading, as the ruby base.
fn latin_phrase_start(text: &str, words_min: usize) -> Option<usize> {
    let mut start = text.len();
    let mut words = 0;
    loop {
        let word_start = trailing_ruby_base_start(&text[..start]);
        let latin = text[word_start..start]
            .chars()
            .next()
            .is_some_and(|first| ruby_base_class(first) == Some(RubyBaseClass::Latin));
        if word_start == start || !latin {
            break;
        }
        start = word_start;
        words += 1;
        let Some(before) = text[..start].strip_suffix(' ') else {
            break;
        };
        if !before
            .chars()
            .next_back()
            .is_some_and(|previous| ruby_base_class(previous) == Some(RubyBaseClass::Latin))
        {
            break;
        }
        start = before.len();
    }
    (words >= words_min).then_some(start)
}

/// Whether `character` is a mark or symbol whose run of repetitions, written
/// before a reading without `｜`, the published Aozora XHTML treats as the
/// ruby base, as in `○○《なになに》` or `＋《プラス》`. A question mark, a
/// fullwidth comma and a middle dot are such bases, as in
/// `？《インタロゲーションマーク》`, where the reading gives the mark itself.
/// Brackets, quotation marks, the full stop and the exclamation mark are not.
fn is_symbol_ruby_base(character: char) -> bool {
    matches!(character,
        '\u{2190}'..='\u{21FF}'
        | '\u{2200}'..='\u{22FF}'
        | '\u{25A0}'..='\u{25FF}'
        | '\u{2600}'..='\u{26FF}'
        | '×' | '÷' | '＋' | '－' | '＝' | '＜' | '＞' | '？' | '，' | '・')
}

/// Intermediate result of `recognize_annotation`.
///
/// `emit` selects which [`SpanKind`] the driver pushes for the
/// top-level case. `annotation_payload` is `Some` exactly when the
/// recogniser produced an `Directive{…}` payload; the
/// `build_content_from_body` caller uses it to wrap the same payload
/// as a `Segment::Directive` without reconstructing it. The emit
/// variants `BlockOpen` / `BlockClose` and non-`Directive` `Aozora`
/// nodes leave `annotation_payload` as `None`, so the body-builder
/// falls back to its `Directive{Unknown}` synthesis path.
struct AnnotationMatch {
    emit: EmitKind,
    annotation_payload: Option<Directive>,
    consume_start: u32,
    consume_end: u32,
    /// A non-fatal warning to report for this bracket, if any:
    /// `unrecognised_container_directive`, `tcy_target_not_found`, or
    /// `bouten_target_ambiguous`. The caller drains it into the diagnostic
    /// stream; the emitted node (often the `Directive{Unknown}` catch-all,
    /// canary intact) is unaffected.
    pending_diagnostic: Option<Diagnostic>,
}

/// What to emit for a matched annotation.
enum EmitKind {
    /// Inline or block-leaf: becomes [`SpanKind::Aozora`].
    Aozora(Node),
    /// Paired-container opener: becomes [`SpanKind::BlockOpen`]. Carries the
    /// authoritative open [`RegionFormat`].
    BlockOpen(RegionFormat),
    /// Paired-container closer: becomes [`SpanKind::BlockClose`]. Carries the
    /// [`RegionClose`] discriminant (the open payload stays authoritative).
    BlockClose(RegionClose),
    /// One supplied marker identifies two adjacent scopes, in closing order.
    BlockCloses {
        /// Supplied targets in inner-to-outer closing order.
        targets: [RegionClose; 2],
        /// Exact directive retained when the complete boundary cannot be established.
        fallback: Node,
    },
}

// `ruby_base_class` is defined in `ab_aozora_syntax` (single source of truth
// shared with the serializer's bare-vs-`｜` decision); imported at the top
// of this module and driven here by `trailing_ruby_base_start`.

#[cfg(test)]
mod tests {
    //! Owned-native classify-stage tests.
    //!
    //! The classifier builds the owned AST directly, so these tests assert
    //! on `Node` spans and resolve their payloads through the
    //! `Allocator`'s `NodeStore`. End-to-end byte-identity of the rendered
    //! output is pinned separately by the conformance vectors, the corpus
    //! verbatim gate, and the render byte-identity gates, which are the
    //! frozen authority.

    use super::*;
    use ab_aozora_syntax::ast::{Content, ContentRange, Node, NodeStore, Segment, StrId};
    use ab_aozora_syntax::{
        BoutenKind, DecorationPosition, ForwardAttr, ForwardOrigin, SectionKind,
    };

    use crate::lexer::pair::pair;
    use crate::lexer::tokenize::tokenize;

    /// Materialised classify output plus the backing store the owned payloads
    /// resolve against.
    #[derive(Debug)]
    struct TestClassifyOutput {
        spans: Vec<ClassifiedSpan>,
        diagnostics: Vec<Diagnostic>,
        store: NodeStore,
    }

    impl TestClassifyOutput {
        /// Resolve a length-1 content run to its plain text (`None` for a
        /// `Segments`/multi run), the owned analogue of `Content::as_plain`.
        fn plain(&self, range: ContentRange) -> Option<&str> {
            self.store.content_range_as_plain(range)
        }

        /// Resolve a `StrId` to its interned bytes.
        fn s(&self, id: StrId) -> &str {
            self.store.resolve_str(id)
        }

        /// Resolve a content run to its `Content` slice.
        fn contents(&self, range: ContentRange) -> Vec<Content> {
            self.store.resolve_content_range(range).to_vec()
        }

        /// The single `SpanKind::Aozora` node, panicking if not exactly one.
        fn only_aozora(&self) -> Node {
            let mut found = None;
            for span in &self.spans {
                if let SpanKind::Aozora(node) = span.kind {
                    assert!(
                        found.is_none(),
                        "more than one Aozora span: {:?}",
                        self.spans
                    );
                    found = Some(node);
                }
            }
            found.unwrap_or_else(|| panic!("no Aozora span in {:?}", self.spans))
        }
    }

    /// Materialise a fresh owned classify run, binding `out` to a
    /// [`TestClassifyOutput`].
    macro_rules! run {
        ($name:ident, $src:expr) => {
            let mut alloc = Allocator::new();
            let mut pair_stream = pair(tokenize($src));
            let mut spans: Vec<ClassifiedSpan> = Vec::new();
            let classify_diagnostics: Vec<Diagnostic> = {
                let mut stream = classify(&mut pair_stream, $src, &mut alloc);
                for span in &mut stream {
                    spans.push(span);
                }
                stream.take_diagnostics()
            };
            let mut diagnostics = pair_stream.take_diagnostics();
            diagnostics.extend(classify_diagnostics);
            let $name = TestClassifyOutput {
                spans,
                diagnostics,
                store: alloc.into_store(),
            };
        };
    }

    #[test]
    fn malformed_annotation_closers_preserve_following_ruby() {
        for (prefix, marker) in [("前", "［＃改丁」"), ("山", "［＃「山」に傍点」")]
        {
            let source = format!("{prefix}{marker}後《あと》。\n次《つぎ》。");
            run!(out, &source);
            let recovered: Vec<_> = out
                .spans
                .iter()
                .filter(|span| {
                    matches!(
                        span.kind,
                        SpanKind::Plain(PlainSpan {
                            provenance: PlainProvenance::RecoveredVerbatim,
                        })
                    )
                })
                .map(|span| &source[span.source_span.start as usize..span.source_span.end as usize])
                .collect();
            assert_eq!(recovered, vec![marker], "{source}");
            let ruby: Vec<_> = out
                .spans
                .iter()
                .filter_map(|span| {
                    if let SpanKind::Aozora(Node::Ruby(ruby)) = span.kind {
                        Some((out.plain(ruby.base), out.plain(ruby.reading)))
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(
                ruby,
                vec![(Some("後"), Some("あと")), (Some("次"), Some("つぎ"))]
            );
            assert_eq!(
                out.spans
                    .iter()
                    .filter(|span| matches!(span.kind, SpanKind::Aozora(_)))
                    .count(),
                2
            );
            assert!(!out.diagnostics.is_empty());
        }
    }

    #[test]
    fn malformed_closer_recovery_does_not_split_note_content() {
        for source in [
            "前［＃入力者註：以下を修正した。\n「甲」→「乙」］末《すえ》。",
            "前［＃改丁」についての注\n後《あと》。］末《すえ》。",
            "前［＃「改丁」についての注\n後《あと》。］末《すえ》。",
        ] {
            run!(out, source);
            let readings: Vec<_> = out
                .spans
                .iter()
                .filter_map(|span| match span.kind {
                    SpanKind::Aozora(Node::Ruby(ruby)) => out.plain(ruby.reading),
                    _ => None,
                })
                .collect();
            assert_eq!(readings, vec!["すえ"], "{source}");
        }
        for source in [
            "前［＃未知」後《あと》。",
            "前［＃入力者註：改丁」後《あと》。",
            "前［＃「改丁」後《あと》。",
        ] {
            run!(out, source);
            assert!(
                !out.spans
                    .iter()
                    .any(|span| matches!(span.kind, SpanKind::Aozora(Node::Ruby(_)))),
                "{source}"
            );
        }
    }

    #[test]
    fn malformed_gaiji_does_not_narrow_the_following_ruby_base() {
        for prefix in ["半", "｜"] {
            let source = format!(
                "{prefix}※［＃「※」は「ひへん＋向」、第3水準1-85-25、44-2」計《はんときばか》り。次《つぎ》。"
            );
            run!(out, &source);
            let readings: Vec<_> = out
                .spans
                .iter()
                .filter_map(|span| match span.kind {
                    SpanKind::Aozora(Node::Ruby(ruby)) => out.plain(ruby.reading),
                    _ => None,
                })
                .collect();
            assert_eq!(readings, vec!["つぎ"]);
            assert!(out.spans.iter().any(|span| {
                matches!(
                    span.kind,
                    SpanKind::Plain(PlainSpan {
                        provenance: PlainProvenance::RecoveredVerbatim
                    })
                ) && &source[span.source_span.start as usize..span.source_span.end as usize]
                    == "《はんときばか》"
            }));
            assert!(out.spans.iter().any(|span| {
                matches!(
                    span.kind,
                    SpanKind::Plain(PlainSpan {
                        provenance: PlainProvenance::Text
                    })
                ) && &source[span.source_span.start as usize..span.source_span.end as usize] == "計"
            }));
        }
    }

    #[test]
    fn empty_input_produces_empty_span_vector() {
        run!(out, "");
        assert!(out.spans.is_empty());
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn plain_ascii_becomes_single_plain_span() {
        run!(out, "hello");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(out.spans[0].kind, SpanKind::Plain(_)));
        assert_eq!(out.spans[0].source_span, Span::new(0, 5));
    }

    #[test]
    fn newline_in_middle_splits_into_three_spans() {
        run!(out, "a\nb");
        assert_eq!(out.spans.len(), 3);
        assert!(matches!(out.spans[0].kind, SpanKind::Plain(_)));
        assert_eq!(out.spans[1].kind, SpanKind::Newline);
        assert!(matches!(out.spans[2].kind, SpanKind::Plain(_)));
    }

    #[test]
    fn explicit_ruby_collapses_to_plain_base_and_reading() {
        run!(out, "｜青梅《おうめ》");
        let Node::Ruby(r) = out.only_aozora() else {
            panic!("expected Ruby, got {:?}", out.only_aozora());
        };
        assert_eq!(out.plain(r.base), Some("青梅"));
        assert_eq!(out.plain(r.reading), Some("おうめ"));
    }

    #[test]
    fn ruby_reading_with_embedded_gaiji_produces_segments() {
        run!(out, "｜日本《に※［＃「ほ」、第3水準1-85-54］ん》");
        let Node::Ruby(r) = out.only_aozora() else {
            panic!("expected Ruby");
        };
        assert_eq!(out.plain(r.base), Some("日本"));
        let reading = out.contents(r.reading);
        let [Content::Segments(seg_range)] = reading[..] else {
            panic!("expected a single Segments reading, got {reading:?}");
        };
        let segs = out.store.resolve_seg_range(seg_range).to_vec();
        assert_eq!(segs.len(), 3);
        assert!(matches!(segs[0], Segment::Text(t) if out.s(t) == "に"));
        assert!(matches!(segs[1], Segment::Gaiji { .. }));
        assert!(matches!(segs[2], Segment::Text(t) if out.s(t) == "ん"));
    }

    #[test]
    fn documented_kanji_marks_extend_ruby_unless_a_bar_limits_it() {
        for (source, expected) in [
            ("松《まつ》ヶ枝《え》", vec![("松", "まつ"), ("ヶ枝", "え")]),
            (
                "鳥《てう》ヶ森《もり》",
                vec![("鳥", "てう"), ("ヶ森", "もり")],
            ),
            (
                "鳥《てう》ヶ｜森《もり》",
                vec![("鳥", "てう"), ("森", "もり")],
            ),
            ("六ヶ《むつか》", vec![("六ヶ", "むつか")]),
            (
                "〇五〇〇《まるごうまるまる》",
                vec![("〇五〇〇", "まるごうまるまる")],
            ),
            (
                "〇｜五〇〇《ごうまるまる》",
                vec![("五〇〇", "ごうまるまる")],
            ),
            ("益〻《ますます》", vec![("益〻", "ますます")]),
        ] {
            run!(out, source);
            let actual: Vec<_> = out
                .spans
                .iter()
                .filter_map(|span| {
                    if let SpanKind::Aozora(Node::Ruby(r)) = &span.kind {
                        Some((
                            out.plain(r.base).expect("plain base"),
                            out.plain(r.reading).expect("plain reading"),
                        ))
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(actual, expected, "{source}");
        }
    }

    #[test]
    fn implicit_ruby_keeps_leading_gaiji_and_kanji_in_one_base() {
        let source = "※［＃「特のへん＋廴＋聿」、第3水準1-87-71］陀多《かんだた》";
        run!(out, source);
        let Node::Ruby(r) = out.only_aozora() else {
            panic!("expected Ruby");
        };
        assert_eq!(out.spans.len(), 1);
        assert_eq!(
            out.spans[0].source_span,
            Span::new(
                0,
                u32::try_from(source.len()).expect("fixture length fits u32")
            )
        );
        assert_eq!(out.plain(r.reading), Some("かんだた"));
        let base = out.contents(r.base);
        let [Content::Segments(range)] = base[..] else {
            panic!("expected structured ruby base");
        };
        let segments = out.store.resolve_seg_range(range);
        assert_eq!(segments.len(), 2);
        assert!(matches!(segments[0], Segment::Gaiji { .. }));
        assert!(matches!(segments[1], Segment::Text(t) if out.s(t) == "陀多"));
    }

    #[test]
    fn a_reading_after_a_space_belongs_to_the_gaiji_before_it() {
        for (source, expected) in [
            (
                "其間に※［＃「足へん＋宛」、第3水準1-92-36］ 《もが》いて",
                "※［＃「足へん＋宛」、第3水準1-92-36］ 《もが》",
            ),
            (
                "喧噪と雑※［＃「二点しんにょう＋鰥のつくり」、第4水準2-89-93］ 《ざつたふ》とを",
                "雑※［＃「二点しんにょう＋鰥のつくり」、第4水準2-89-93］ 《ざつたふ》",
            ),
            (
                "呉江の沈※［＃「王＋旬」、第3水準1-87-93］\u{3000}《ちんしゅん》という",
                "沈※［＃「王＋旬」、第3水準1-87-93］\u{3000}《ちんしゅん》",
            ),
        ] {
            run!(out, source);
            let ruby = out
                .spans
                .iter()
                .find_map(|span| match span.kind {
                    SpanKind::Aozora(Node::Ruby(_)) => Some(span.source_span),
                    _ => None,
                })
                .expect("ruby over the gaiji");
            assert_eq!(&source[ruby.start as usize..ruby.end as usize], expected);
        }
        run!(out, "※［＃「足へん＋宛」、第3水準1-92-36］ という");
        assert!(
            !out.spans
                .iter()
                .any(|span| matches!(span.kind, SpanKind::Aozora(Node::Ruby(_))))
        );
    }

    #[test]
    fn implicit_ruby_includes_kanji_before_gaiji() {
        for (prefix, marker, reading) in [
            ("袁", "※［＃「にんべん＋參」、第4水準2-1-79］", "えんさん"),
            (
                "幽",
                "※［＃「帚」の「冖／巾」に代えて「火」、第3水準1-87-36］",
                "ゆうれい",
            ),
        ] {
            let source = format!("の{prefix}{marker}《{reading}》");
            run!(out, &source);
            let Node::Ruby(r) = out.only_aozora() else {
                panic!("expected Ruby")
            };
            assert_eq!(out.spans.len(), 2);
            assert_eq!(out.spans[0].source_span, Span::new(0, 3));
            assert_eq!(out.spans[1].source_span.start, 3);
            assert_eq!(out.plain(r.reading), Some(reading));
            let base = out.contents(r.base);
            let [Content::Segments(range)] = base[..] else {
                panic!("expected structured base")
            };
            let segments = out.store.resolve_seg_range(range);
            assert_eq!(segments.len(), 2);
            assert!(matches!(segments[0], Segment::Text(t) if out.s(t) == prefix));
            assert!(matches!(segments[1], Segment::Gaiji { .. }));
        }
    }

    #[test]
    fn ruby_base_retains_gaiji_separated_by_kanji() {
        for (source, expected) in [
            (
                "八※［＃「にんべん＋（八がしら／月）」、第3水準1-14-20］之※［＃「にんべん＋舞」、第4水準2-3-4］《やつらのまい》",
                "八佾之儛",
            ),
            (
                "※［＃「口＋奄」、第3水準1-15-6］阿謨※［＃「口＋云」、第3水準1-14-87］《おんあもうん》",
                "唵阿謨呍",
            ),
        ] {
            run!(out, source);
            let Node::Ruby(ruby) = out.only_aozora() else {
                panic!("expected one ruby across the complete base");
            };
            let [Content::Segments(range)] = out.contents(ruby.base)[..] else {
                panic!("expected structured base");
            };
            let visible: String = out
                .store
                .resolve_seg_range(range)
                .iter()
                .map(|seg| match seg {
                    Segment::Text(text) => out.s(*text).to_owned(),
                    Segment::Gaiji { value: gaiji, .. } => gaiji
                        .resolve(&out.store)
                        .expect("fixture glyph resolves")
                        .as_char()
                        .expect("single fixture scalar")
                        .to_string(),
                    _ => panic!("unexpected base segment"),
                })
                .collect();
            assert_eq!(visible, expected);
            assert_eq!(out.spans.len(), 1);
            assert_eq!(
                out.spans[0].source_span,
                Span::new(0, u32::try_from(source.len()).unwrap())
            );
        }
    }

    #[test]
    fn mixed_gaiji_run_restores_every_span_when_no_ruby_adopts_it() {
        for suffix in ["", "。", "《》", "《み", "\n本文", "かな字《じ》"] {
            let source = format!(
                "八※［＃「にんべん＋（八がしら／月）」、第3水準1-14-20］之※［＃「にんべん＋舞」、第4水準2-3-4］{suffix}"
            );
            run!(out, &source);
            let mut end = 0;
            for span in &out.spans {
                assert_eq!(span.source_span.start, end, "{source}");
                end = span.source_span.end;
            }
            assert_eq!(usize::try_from(end).unwrap(), source.len());
            assert_eq!(
                out.spans
                    .iter()
                    .filter(|span| matches!(span.kind, SpanKind::Aozora(Node::Gaiji(_))))
                    .count(),
                2
            );
        }
    }

    #[test]
    fn deferred_kanji_prefix_preserves_source_without_a_reading() {
        for suffix in ["", "。", "《》", "《えんさん", "\n本文"] {
            let source = format!("の袁※［＃「にんべん＋參」、第4水準2-1-79］{suffix}");
            run!(out, &source);
            assert!(
                !out.spans
                    .iter()
                    .any(|s| matches!(s.kind, SpanKind::Aozora(Node::Ruby(_))))
            );
            let mut end = 0;
            for span in &out.spans {
                assert_eq!(span.source_span.start, end);
                end = span.source_span.end;
            }
            assert_eq!(end as usize, source.len());
        }
    }

    #[test]
    fn deferred_gaiji_with_kanji_preserves_non_ruby_source_order() {
        for suffix in ["陀多", "陀多《》", "陀多《かんだた", "陀多。", "陀多\n本文"]
        {
            let source = format!("※［＃「特のへん＋廴＋聿」、第3水準1-87-71］{suffix}");
            run!(out, &source);
            assert!(matches!(
                out.spans[0].kind,
                SpanKind::Aozora(Node::Gaiji(_))
            ));
            assert!(
                !out.spans
                    .iter()
                    .any(|span| matches!(span.kind, SpanKind::Aozora(Node::Ruby(_))))
            );
            let mut end = 0;
            for span in &out.spans {
                assert_eq!(span.source_span.start, end, "{source}");
                end = span.source_span.end;
            }
            assert_eq!(end as usize, source.len());
        }
    }

    #[test]
    fn non_kanji_after_gaiji_starts_a_separate_implicit_ruby_base() {
        run!(
            out,
            "※［＃「特のへん＋廴＋聿」、第3水準1-87-71］かな陀多《だた》"
        );
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::Aozora(Node::Gaiji(_))
        ));
        let SpanKind::Aozora(Node::Ruby(r)) = out.spans.last().unwrap().kind else {
            panic!("expected trailing ruby");
        };
        assert_eq!(out.plain(r.base), Some("陀多"));
    }

    #[test]
    fn official_kanji_like_marks_form_implicit_ruby_bases() {
        for (source, expected) in [
            ("々《くりかえし》", "々"),
            ("仝《おなじ》", "仝"),
            ("〆《しめ》", "〆"),
            ("〇《れい》", "〇"),
            ("〻《くりかえし》", "〻"),
            ("ヶ《か》", "ヶ"),
        ] {
            run!(out, source);
            let Node::Ruby(ruby) = out.only_aozora() else {
                panic!("expected implicit ruby for {source}");
            };
            assert_eq!(out.plain(ruby.base), Some(expected));
        }
    }

    #[test]
    fn symbol_runs_and_latin_abbreviations_form_implicit_ruby_bases() {
        for (source, expected) in [
            ("おケシも出た。○○《なになに》", "○○"),
            ("還□《かへらせ》", "□"),
            ("生活＋《プラス》", "＋"),
            ("になり、×《タイム》", "×"),
            ("Pestalozzi Str.《シユトラセ》", "Str."),
            ("U.S.A.《アメリカ》", "U.S.A."),
            ("RYTHMES《リトム》 D'AMOUR'《ダムウル》", "D'AMOUR'"),
            ("……　？《インタロゲーションマーク》", "？"),
            ("赤で・《てん》", "・"),
            ("英語の『，《こんま》", "，"),
            ("一分流の Zwagri 《ツワグリ》", "Zwagri "),
            (
                "Hsifan territory  《シフアン・テリトリー》",
                "Hsifan territory  ",
            ),
            (
                "蒙古人は Kuso-Bhakator-Nor 《クーゾ・バカトル・ノール》",
                "Kuso-Bhakator-Nor ",
            ),
        ] {
            run!(out, source);
            let SpanKind::Aozora(Node::Ruby(ruby)) = out.spans.last().unwrap().kind else {
                panic!("expected implicit ruby for {source}");
            };
            assert_eq!(out.plain(ruby.base), Some(expected), "{source}");
        }
        for source in [
            "来る。《六月十日》",
            "出ろ！《ヴアツ》",
            "「日本斎忌考」《ゼ・タブー》",
            "城上は.《ああ》",
            "漢字 《かんじ》",
        ] {
            run!(out, source);
            assert!(
                !out.spans
                    .iter()
                    .any(|span| matches!(span.kind, SpanKind::Aozora(Node::Ruby(_)))),
                "{source}"
            );
        }
    }

    #[test]
    fn a_reading_after_a_note_belongs_to_the_base_run_before_the_note() {
        run!(out, "光が鈍［＃「鈍」は底本では「鋭」］《にぶ》い");
        let ruby = out
            .spans
            .iter()
            .find_map(|span| match span.kind {
                SpanKind::Aozora(Node::Ruby(ruby)) => Some((ruby, span.source_span)),
                _ => None,
            })
            .expect("ruby over the noted base");
        assert_eq!(
            &"光が鈍［＃「鈍」は底本では「鋭」］《にぶ》い"
                [ruby.1.start as usize..ruby.1.end as usize],
            "鈍［＃「鈍」は底本では「鋭」］《にぶ》"
        );
        run!(out, "光が。［＃「。」はママ］《にぶ》い");
        assert!(
            !out.spans
                .iter()
                .any(|span| matches!(span.kind, SpanKind::Aozora(Node::Ruby(_))))
        );
    }

    #[test]
    fn a_reading_after_a_style_note_belongs_to_the_styled_target() {
        for (source, base) in [
            (
                "儀式。公事根源［＃「公事根源」は斜体］《くじこんげん》には",
                "公事根源［＃「公事根源」は斜体］《くじこんげん》",
            ),
            (
                "公事根源［＃「公事根源」に傍点］《くじこんげん》には",
                "公事根源［＃「公事根源」に傍点］《くじこんげん》",
            ),
        ] {
            run!(out, source);
            let ruby = out
                .spans
                .iter()
                .find_map(|span| match span.kind {
                    SpanKind::Aozora(Node::Ruby(_)) => Some(span.source_span),
                    _ => None,
                })
                .expect(source);
            assert_eq!(&source[ruby.start as usize..ruby.end as usize], base);
        }
        for source in [
            "これは漢字［＃「字」に傍点］《かんじ》です",
            "美津子は√３［＃「３」は太字］《ルートサン》と",
        ] {
            run!(out, source);
            assert!(
                !out.spans
                    .iter()
                    .any(|span| matches!(span.kind, SpanKind::Aozora(Node::Ruby(_)))),
                "{source}"
            );
        }
    }

    #[test]
    fn top_level_gaiji_resolves() {
        run!(out, "※［＃「木＋吶のつくり」、第3水準1-85-54］");
        let Node::Gaiji(g) = out.only_aozora() else {
            panic!("expected Gaiji");
        };
        assert_eq!(out.s(g.hint), "木＋吶のつくり");
        assert!(g.resolve(&out.store).is_some());
    }

    #[test]
    fn refmark_disambiguates_description_only_gaiji() {
        for description in ["底本では欠字", "偏が「宛」で旁が「りっとう」"] {
            let source = format!("※［＃{description}］");
            run!(out, &source);
            let Node::Gaiji(g) = out.only_aozora() else {
                panic!("expected Gaiji for {description}");
            };
            assert_eq!(out.s(g.hint), description);
            assert!(g.resolve(&out.store).is_none());
        }
    }

    #[test]
    fn forward_bouten_reclaims_adjacent_literal() {
        run!(out, "青空［＃「青空」に傍点］");
        let Node::Format(f) = out.only_aozora() else {
            panic!("expected Format(Bouten)");
        };
        assert_eq!(
            f.attrs.single().unwrap(),
            ForwardAttr::Bouten {
                kind: BoutenKind::Goma,
                position: DecorationPosition::Right,
            }
        );
        assert_eq!(out.plain(f.target), Some("青空"));
        assert_eq!(f.origin, ForwardOrigin::Reclaimed);
    }

    #[test]
    fn paired_container_emits_open_and_close() {
        run!(out, "［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］");
        assert!(
            out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::BlockOpen(_)))
        );
        assert!(
            out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::BlockClose(_)))
        );
    }

    #[test]
    fn historical_ika_indent_opens_a_typed_block() {
        run!(out, "［＃以下3字下げ］\n本文\n［＃字下げ終わり］");
        assert!(out.spans.iter().any(|span| {
            matches!(
                span.kind,
                SpanKind::BlockOpen(RegionFormat::Indent(block)) if block.amount == 3
            )
        }));
        assert!(
            out.spans
                .iter()
                .any(|span| matches!(span.kind, SpanKind::BlockClose(RegionClose::Indent { .. })))
        );
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    }

    #[test]
    fn kaeriten_is_classified() {
        run!(out, "天［＃（レ）］");
        let has_kaeriten = out
            .spans
            .iter()
            .any(|s| matches!(s.kind, SpanKind::Aozora(Node::Kunten(_))));
        assert!(has_kaeriten, "expected a Kunten span: {:?}", out.spans);
    }

    #[test]
    fn unknown_annotation_is_directive_not_bare_bracket() {
        run!(out, "［＃まったく未知の注記です］");
        let Node::Directive(d) = out.only_aozora() else {
            panic!("expected a Directive, got {:?}", out.only_aozora());
        };
        assert_eq!(d.kind, DirectiveKind::Unknown);
        assert!(out.s(d.raw).starts_with("［＃"));
    }

    #[test]
    fn page_break_and_section_break_classified() {
        run!(out, "［＃改ページ］");
        assert!(matches!(out.only_aozora(), Node::PageBreak));
        run!(out2, "［＃改丁］");
        assert!(matches!(
            out2.only_aozora(),
            Node::SectionBreak(SectionKind::Kaicho)
        ));
    }

    #[test]
    fn angle_quote_classified() {
        run!(out, "≪重要≫");
        let Node::AngleQuote(d) = out.only_aozora() else {
            panic!("expected AngleQuote, got {:?}", out.only_aozora());
        };
        assert_eq!(out.plain(d.content), Some("重要"));
    }

    #[test]
    fn unresolved_gaiji_emits_diagnostic() {
        run!(out, "※［＃「謎の字」、未知の注記］");
        assert!(
            out.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::UnresolvedGaiji { .. })),
            "expected an UnresolvedGaiji diagnostic, got {:?}",
            out.diagnostics
        );
    }

    #[test]
    fn spans_tile_source_contiguously() {
        run!(out, "前｜青梅《おうめ》後");
        let mut cursor = 0u32;
        for span in &out.spans {
            assert_eq!(span.source_span.start, cursor, "gap before {span:?}");
            cursor = span.source_span.end;
        }
    }
}
