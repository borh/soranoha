//! Streaming-API semantics for [`ab_aozora_pipeline::Pipeline`] and the
//! [`ab_aozora_pipeline::lexer`] streaming building blocks (`tokenize` / `pair` /
//! `classify`).
//!
//! These tests pin the observable behaviour of the streaming pipeline:
//!
//! * Iterators may be dropped at any point without a panic, and
//!   accumulated diagnostics remain readable.
//! * Iteration is lazy: `.next()` does not consume its upstream beyond
//!   the byte position of the event it emits.
//! * `.take(N)` and `.collect().into_iter().take(N)` agree.
//! * EOF-time `Unclosed` synthesis is observable on the public stream.
//! * The type-state [`Pipeline`] composition is byte-for-byte
//!   equivalent to the one-shot [`lex`] entry point.
//! * Diagnostics observed at intermediate states still surface in the
//!   final [`LexOutput::diagnostics`].
//!
//! The tests inspect only behaviour visible to a downstream caller;
//! `Pipeline` exposes no fields beyond its public surface.

use ab_aozora_pipeline::lexer::{PairEvent, PairKind, classify, pair, tokenize};
use ab_aozora_pipeline::{Pipeline, lex};
use ab_aozora_spec::{Diagnostic, Sentinel};
use ab_aozora_syntax::alloc::Allocator;
// 1. Drop / early-termination

/// Construct a `PairStream`, take a couple of events via `next()`, then
/// drop the stream. Must not panic and `take_diagnostics` (called on a
/// fresh stream over the same input) must be callable without panic.
#[test]
fn pair_stream_drop_partway_does_not_panic_and_diagnostics_remain_readable() {
    // The test asserts only that an early drop does not panic, not the
    // event sequence.
    let src = "[X][Y][Z]";
    let mut stream = pair(tokenize(src));
    let _e0 = stream.next();
    let _e1 = stream.next();
    drop(stream);

    // A second, independent stream over the same source must still
    // behave exactly like the first would have.
    let mut stream2 = pair(tokenize(src));
    while stream2.next().is_some() {}
    // `take_diagnostics` must not panic, whatever the diagnostic count.
    let _diags = stream2.take_diagnostics();
}

/// Same shape for `ClassifyStream`. Drop midway, then run an
/// independent one over the same input and check that no global state
/// (for example the forward-target index) was corrupted.
#[test]
fn classify_stream_drop_partway_does_not_corrupt_global_state() {
    let src = "abc｜D《e》fgh";
    let mut alloc = Allocator::new();

    {
        let mut pair_stream = pair(tokenize(src));
        let mut classify_stream = classify(&mut pair_stream, src, &mut alloc);
        let _s0 = classify_stream.next();
        // Drop both streams here: `classify_stream` first (last
        // declared), then `pair_stream`.
    }

    // A fresh end-to-end run over the same source must still behave
    // identically to a never-interrupted run.
    let oneshot = lex(src);
    assert_eq!(
        oneshot.registry.count_kind(Sentinel::Inline),
        1,
        "ruby span survived"
    );
}

// 2. Lazy iteration semantics

/// `PairStream` emits events approximately in step with the underlying
/// source position. A direct laziness check would need a counting
/// wrapper, because `tokenize` is itself lazy and `pair` may peek; the
/// test checks instead that the byte position of the Nth emitted event
/// grows in proportion to N.
///
/// The test takes the first 10 events from a long input and checks that
/// the maximum source byte position observed is far below the input
/// length, which shows that the stream did not consume its upstream to
/// EOF on the first `next()`.
#[test]
fn pair_stream_take_n_does_not_exhaust_underlying_source() {
    // Aozora full-width brackets are 3 bytes each, hash 3 bytes.
    // `［＃A］` = 12 bytes; 256 of them = 3 072 bytes. The stream emits
    // 4 events per group: PairOpen, Solo(Hash), Text("A"), PairClose.
    let unit = "［＃A］";
    let src: String = unit.repeat(256);
    let total_len = src.len();
    let mut stream = pair(tokenize(&src));
    let first_10: Vec<PairEvent> = (0..10).filter_map(|_| stream.next()).collect();
    assert_eq!(first_10.len(), 10, "events: {first_10:?}");
    // The first 10 events cover roughly the first 2-3 groups (2.5 ×
    // 12 bytes ≈ 30 bytes). The ceiling of 1/8 of the input length
    // allows for variation in event packing and still detects a drain
    // of the whole input.
    let max_end: u32 = first_10
        .iter()
        .filter_map(|e| e.span().map(|s| s.end))
        .max()
        .expect("at least one event has a span");
    assert!(
        (max_end as usize) < total_len / 8,
        "first 10 events should not drive the upstream beyond ~1/8 of \
         the source, got max_end={max_end} (total source length: {total_len})"
    );
}

// 3. .take(N) equivalence

/// `pair(tokenize(s)).take(N).collect()` must equal
/// `pair(tokenize(s)).collect().into_iter().take(N).collect()` for
/// several inputs and several N. Detects per-iteration side effects
/// that make streamed results differ from index-based access.
#[test]
fn pair_stream_take_n_matches_collect_then_take() {
    let inputs: &[&str] = &[
        "",
        "plain",
        "［＃改ページ］",
        "［＃ここから2字下げ］\nbody\n［＃ここで字下げ終わり］",
        "｜青梅《おうめ》街道沿いの古い宿",
        "「a」「b」「c」「d」「e」",
    ];
    let ns: &[usize] = &[0, 1, 2, 5, 100];

    for src in inputs {
        // Full materialisation as the reference.
        let full: Vec<PairEvent> = pair(tokenize(src)).collect();
        for &n in ns {
            let via_take: Vec<PairEvent> = pair(tokenize(src)).take(n).collect();
            let via_collect_take: Vec<PairEvent> = full.iter().take(n).cloned().collect();
            assert_eq!(
                via_take, via_collect_take,
                "src={src:?} n={n}: take(n) must agree with collect().take(n)"
            );
        }
    }
}

// 4. EOF Unclosed isolation

/// One unclosed bracket → exactly one trailing `PairEvent::Unclosed`
/// of `PairKind::Bracket` and exactly one corresponding diagnostic.
#[test]
fn pair_stream_emits_one_unclosed_event_for_one_unclosed_bracket() {
    let src = "［＃unclosed";
    let mut stream = pair(tokenize(src));
    let events: Vec<PairEvent> = (&mut stream).collect();
    let diagnostics = stream.take_diagnostics();

    // The trailing event must be the Unclosed for the outer bracket.
    let unclosed_count = events
        .iter()
        .filter(|e| {
            matches!(
                e,
                PairEvent::Unclosed {
                    kind: PairKind::Bracket,
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        unclosed_count, 1,
        "expected exactly one Unclosed Bracket event, got {events:?}"
    );

    let bracket_diag_count = diagnostics
        .iter()
        .filter(|d| {
            matches!(
                d,
                Diagnostic::UnclosedBracket {
                    kind: PairKind::Bracket,
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        bracket_diag_count, 1,
        "expected exactly one UnclosedBracket diagnostic, got {diagnostics:?}"
    );
}

// 5. Pipeline state-transition smoke tests

/// For several non-trivial inputs the explicit chain
/// `Pipeline::new(s).sanitize().tokenize().pair().build()`
/// must produce results identical to `lex(s)`.
#[test]
fn pipeline_chain_matches_lex_into_arena_for_corpus_shapes() {
    let inputs: &[&str] = &[
        "",
        "plain text",
        "｜青梅《おうめ》",
        "［＃改ページ］",
        "［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］",
        "明治の頃｜青梅《おうめ》街道沿いに、※［＃「木＋吶のつくり」、第3水準1-85-54］\n\
         なる珍しき木が立つ。［＃ここから2字下げ］\n\
         その下で人々は語らひ、［＃「青空」に傍点］\n\
         ［＃ここで字下げ終わり］",
    ];
    for src in inputs {
        let chain = Pipeline::new(src).sanitize().tokenize().pair().build();
        let oneshot = lex(src);
        assert_eq!(
            chain.normalized, oneshot.normalized,
            "normalized text drift for input {src:?}"
        );
        assert_eq!(
            chain.sanitized_len, oneshot.sanitized_len,
            "sanitized_len drift for input {src:?}"
        );
        assert_eq!(
            chain.diagnostics.len(),
            oneshot.diagnostics.len(),
            "diagnostic count drift for input {src:?}"
        );
        assert_eq!(
            chain.registry.count_kind(Sentinel::Inline),
            oneshot.registry.count_kind(Sentinel::Inline),
            "inline registry drift for input {src:?}"
        );
        assert_eq!(
            chain.registry.count_kind(Sentinel::BlockLeaf),
            oneshot.registry.count_kind(Sentinel::BlockLeaf),
            "block_leaf registry drift for input {src:?}"
        );
        assert_eq!(
            chain.registry.count_kind(Sentinel::BlockOpen),
            oneshot.registry.count_kind(Sentinel::BlockOpen),
            "block_open registry drift for input {src:?}"
        );
        assert_eq!(
            chain.registry.count_kind(Sentinel::BlockClose),
            oneshot.registry.count_kind(Sentinel::BlockClose),
            "block_close registry drift for input {src:?}"
        );
    }
}

// 6. Diagnostics persist through intermediate inspection

/// A sanitize-stage diagnostic visible at the [`Sanitized`] state must
/// also appear in the final [`LexOutput::diagnostics`] after
/// `.build()`. This pins the contract that intermediate inspection
/// does not consume diagnostics.
#[test]
fn pipeline_phase0_diagnostic_observed_at_sanitized_also_present_after_build() {
    let src = "abc〔cafe'〕def";

    let sanitized = Pipeline::new(src).sanitize();
    let phase0_count_at_sanitized = sanitized
        .diagnostics()
        .iter()
        .filter(|d| matches!(d, Diagnostic::AccentDecompositionApplied { .. }))
        .count();
    assert_eq!(
        phase0_count_at_sanitized,
        1,
        "expected one AccentDecompositionApplied at Sanitized state, \
         got: {:?}",
        sanitized.diagnostics()
    );

    // Drive to completion. The sanitize-stage diagnostic must still be there.
    let final_out = sanitized.tokenize().pair().build();
    let phase0_count_at_build = final_out
        .diagnostics
        .iter()
        .filter(|d| matches!(d, Diagnostic::AccentDecompositionApplied { .. }))
        .count();
    assert_eq!(
        phase0_count_at_build, 1,
        "expected the same AccentDecompositionApplied to survive build, \
         got: {:?}",
        final_out.diagnostics
    );
}
