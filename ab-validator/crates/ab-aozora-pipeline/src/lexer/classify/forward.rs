//! Forward-reference recognisers.
//!
//! The `recognize_annotation` cascade and the per-construct
//! forward-reference recognisers (bouten, 縦中横, heading, emphasis,
//! left-ruby / side-note, caption-figure) plus the source-scanned
//! `FORWARD_TARGET_INDEX` that records whether a target occurs earlier
//! in the source. These need the pair-stage event stream, unlike the
//! body-keyword `directive` classifier. Recognised nodes build on the
//! shared `RecogniseCtx`.

#[cfg(feature = "classify-instrument")]
use super::super::instrumentation::{Subsystem, SubsystemGuard};

use std::cell::RefCell;
use std::collections::HashMap;

use core::num::{NonZeroI8, NonZeroU8};

use ab_aozora_encoding::gaiji as gaiji_resolve;
use ab_aozora_spec::Diagnostic;
use ab_aozora_syntax::accent::{compose_accent, compose_accent_dots, supplied_mark_target_start};
use ab_aozora_syntax::ast::{Content, FormattingEffect, KuntenKind, Node, NonEmptySpan};
use ab_aozora_syntax::format::ForwardOrigin;
use ab_aozora_syntax::lint::canonical_directive;
use ab_aozora_syntax::{
    AbsoluteSize, AccentMark, BoutenKind, BoutenPosition, DecorationPosition, DirectiveKind,
    EnclosureKind, FontShift, ForwardAttr, MarginNoteKind, MarginNotePosition, QualitativeFontSize,
    Resolved, Span, indentation_question_locator,
};

use crate::text_variant::{formatted_text_variant, formatting_edition_note};
use crate::transcribed_notes;

use super::super::pair::{PairEvent, PairKind};
use super::super::token::TriggerKind;
use super::directive::{
    AnnotationBody, bouten_kind_from_suffix, classify_annotation_body, classify_general_image_body,
    editorial_note_kind, is_gloss_body, is_return_mark, parse_decimal_u8_prefix, parse_end_spacing,
    parse_heading_keyword,
};
use super::{AnnotationMatch, BodyView, EmitKind, RecogniseCtx};

thread_local! {
    /// First source positions of quoted targets in annotation-dense documents.
    /// Unindexed operands still require source lookup; absence from this cache
    /// is not evidence that a target is absent from the document.
    static FORWARD_TARGET_INDEX: RefCell<ForwardTargetState> = RefCell::default();
}

#[derive(Default)]
struct ForwardTargetState {
    installed: bool,
    first_position: HashMap<String, u32>,
}

/// Drop the per-classify forward-target index.
fn clear_forward_target_index() {
    FORWARD_TARGET_INDEX.with(|cell| {
        let mut state = cell.borrow_mut();
        state.installed = false;
        state.first_position.clear();
    });
}

/// Most corpus documents do not install the index (they are below the
/// 64-quote threshold), so `installed` is still false from the previous
/// document and the `clear()` call would do nothing. This wrapper skips
/// the `borrow_mut()` + `HashMap::clear()` pair in that case, which saves
/// ~10 ns per parse on the common path.
fn clear_forward_target_index_if_installed() {
    FORWARD_TARGET_INDEX.with(|cell| {
        let mut state = cell.borrow_mut();
        if state.installed {
            state.installed = false;
            state.first_position.clear();
        }
    });
}

/// Below this many distinct `「…」` quote bodies, the source-byte pre-pass
/// costs more to build than the substring scans it saves.
/// The median corpus doc has < 100 quote bodies and skips the index
/// entirely; the pathological annotation-dense doc has thousands.
const FORWARD_QUOTE_BODY_THRESHOLD: usize = 64;

/// Build the forward-reference target index by scanning raw source
/// bytes for `「…」` quote pairs and recording the first byte position
/// of each unique body. Event-free: runs before the streaming
/// pipeline starts.
pub(super) fn install_forward_target_index_from_source(source: &str) {
    use memchr::memmem;

    // `「` is U+300C, UTF-8 = E3 80 8C; `」` is U+300D, UTF-8 = E3 80 8D.
    const QUOTE_OPEN: &[u8] = b"\xE3\x80\x8C";
    const QUOTE_CLOSE: &[u8] = b"\xE3\x80\x8D";

    #[cfg(feature = "classify-instrument")]
    let _classify_guard = SubsystemGuard::new(Subsystem::ForwardIndexInstall);

    let bytes = source.as_bytes();
    // Count quote opens, stopping at the threshold, so the >99 % of
    // corpus documents with fewer than 64 quote opens allocate no
    // offset `Vec<usize>`. memmem::find_iter is internally
    // SIMD-vectorised, so the count loop is bandwidth-bound; stopping
    // at 64 bounds the work by the threshold, not by the document
    // length.
    let mut count = 0usize;
    for _ in memmem::find_iter(bytes, QUOTE_OPEN) {
        count += 1;
        if count >= FORWARD_QUOTE_BODY_THRESHOLD {
            break;
        }
    }
    if count < FORWARD_QUOTE_BODY_THRESHOLD {
        clear_forward_target_index_if_installed();
        return;
    }
    let opens: Vec<usize> = memmem::find_iter(bytes, QUOTE_OPEN).collect();

    // Pair each `「` with its balanced `」` via a quote-only stack, as
    // the pair stage does (`PairKind::Quote`, pair.rs), so a quote body
    // is sliced at the same extent the recognisers see through
    // `view.links`. A target whose text embeds a nested `「…」` (a
    // forward-referenced gaiji base such as `冢※［＃「土へん＋冢」…］`) is
    // captured whole, not truncated at the inner `」`: a first-`」` scan
    // records `冢※［＃「土へん＋冢`, a body no recogniser queries, so the
    // target would be absent from the index and every installed-document
    // lookup would return "not preceded". When quote depth never exceeds
    // 1 (every non-nested document) the innermost open is always the
    // immediately preceding one, so the body set equals that of a
    // first-`」` scan.
    //
    // The stored value is the body's first occurrence as a substring of
    // the source (`memmem::find` over the whole source), the same
    // question the `source[..cutoff].contains(target)` fallback answers,
    // not the position of the `「` that introduced this body. The
    // canonical reference is `語句［＃「語句」に傍点］`, whose referent is
    // bare text before the bracket while the only `「語句」` pair is
    // inside the directive. Because the position is a body-keyed global
    // substring find, it is independent of which `「` inserted the entry,
    // so the stack pairing does not shift a stored position.
    let mut first_positions: HashMap<String, u32> = HashMap::with_capacity(opens.len());
    let mut open_stack: Vec<usize> = Vec::new();
    let mut opens_iter = opens.iter().copied().peekable();
    for close_pos in memmem::find_iter(bytes, QUOTE_CLOSE) {
        // Push every `「` that opens before this `」`.
        while opens_iter.peek().is_some_and(|&op| op < close_pos) {
            open_stack.push(opens_iter.next().expect("peeked Some"));
        }
        // Match this `」` to the innermost still-open `「`; a stray close
        // with an empty stack has nothing to index.
        let Some(open_pos) = open_stack.pop() else {
            continue;
        };
        let body_start = open_pos + QUOTE_OPEN.len();
        let body = &source[body_start..close_pos];
        if body.is_empty() {
            continue;
        }
        first_positions.entry(body.to_owned()).or_insert_with(|| {
            // First substring occurrence anywhere in the source. The body
            // provably occurs at `body_start` (its own quote), so `find`
            // never returns `None`; the fallback keeps it total.
            let first = memmem::find(bytes, body.as_bytes()).unwrap_or(body_start);
            u32::try_from(first).expect("source positions fit in u32 per sanitize-stage cap")
        });
    }

    if first_positions.len() < FORWARD_QUOTE_BODY_THRESHOLD {
        clear_forward_target_index();
        return;
    }

    FORWARD_TARGET_INDEX.with(|cell| {
        let mut state = cell.borrow_mut();
        state.installed = true;
        state.first_position = first_positions;
    });
}

/// Try to recognize a `［＃keyword…］` annotation at
/// `events[open_idx]`.
///
/// Requires the immediately-next event to be a [`TriggerKind::Hash`]
/// [`PairEvent::Solo`]: the shape `［` `＃` `body` `］`. Bodies
/// without a hash (plain `［…］`) are not annotations; bodies with a
/// hash whose keyword no specialised recogniser matches fall through
/// to the `Directive { Unknown }` catch-all so the bracket is
/// always consumed into some `Node`.
impl RecogniseCtx<'_, '_> {
    /// Forward-reference dispatch for a well-formed `［＃…］` bracket.
    ///
    /// Resolves complete separately transcribed note groups first. Otherwise
    /// tries the body-keyword classifier, then a fixed cascade of
    /// forward-reference recognisers, falling through to the
    /// `Directive{Unknown}` catch-all. Cascade order:
    ///
    /// 1. body keyword: `classify_annotation_body`
    /// 2. bouten (single): `「X」に<kind>`
    /// 3. bouten (range): `「X」～「Y」に<kind>`
    /// 4. left-ruby: `「X」の左に「Y」のルビ`
    /// 5. side-note: `「X」の左に「Y」の注記` / `「X」に「Y」の傍記`
    /// 6. 縦中横: `「X」は縦中横`
    /// 7. heading: `「X」は…見出し`
    /// 8. emphasis: `「X」は太字` / `斜体`
    /// 9. caption-figure: `「cap」のキャプション付きの…（file）入る`
    /// 10. general image: `<desc>（file）入る`
    /// 11. empty / editorial-note / `Directive{Unknown}` catch-all
    ///
    /// # Ordering contract
    ///
    /// Most adjacent recognisers are keyword-disjoint: their bodies
    /// carry mutually exclusive particles/keywords, so reordering them
    /// leaves the output unchanged. These are not pinned by dedicated
    /// order tests (such a test would be vacuous):
    ///
    /// * bouten vs left-ruby vs side-note: `のルビ`, `の注記`, and `の傍記`
    ///   are not bouten kinds, so each declines before the next is tried.
    /// * 縦中横 / heading / emphasis: `縦中横`, `…見出し`, and
    ///   `太字`/`斜体` are mutually exclusive after the shared `は`
    ///   particle (縦中横's diagnostic threading is the one exception,
    ///   below).
    /// * bouten single vs range: separated by the `～` / `〜` infix.
    ///
    /// Orderings where sequence affects output each have a regression test:
    ///
    /// * caption-figure ≺ general-image: both end in `（file）入る`; the
    ///   caption form is more specific and must be tried first to keep
    ///   its figcaption. Pinned by `caption_before_figure_recognised`.
    /// * target-bearing recogniser ≺ editorial-note: `「ママ」に傍点`
    ///   must be claimed as bouten before the `ママ` editorial note would
    ///   type it as `Sic`. Pinned by
    ///   `mama_target_with_bouten_stays_bouten`.
    /// * editorial-note ≺ `Unknown`: the editorial kinds refine the
    ///   catch-all, which would otherwise claim every body. Pinned by
    ///   `editorial_notes_type_as_asis_and_textual_note`.
    /// * 縦中横 compound ≺ small-script range: `「X」は縦中横、行右小書き`
    ///   assigns both attributes to one target. The compound must be
    ///   recognized before the single-attribute small-script classifier.
    /// * 縦中横 `ShapedNoTarget` diagnostics are carried through the fall-through into
    ///   source-preserving formatting instructions via `tcy_pending`. Pinned
    ///   by `tcy_target_not_found_fires_as_warning`.
    #[allow(
        clippy::too_many_lines,
        reason = "a flat dispatch chain over the forward-reference recognisers \
                  (body / bouten / 縦中横 / heading / emphasis); each block is \
                  the same shape and splitting them would scatter the ordered \
                  fall-through to the Directive{Unknown} catch-all"
    )]
    pub(super) fn recognize_annotation(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
        formatting_text_start: Option<u32>,
    ) -> Option<AnnotationMatch> {
        #[cfg(feature = "classify-instrument")]
        let _classify_guard = SubsystemGuard::new(Subsystem::Directive);
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

        if let Some(node) = transcribed_notes::classify(
            self.source,
            Span::new(open_span.start, close_span.end),
            self.alloc,
        ) {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start: open_span.start,
                consume_end: close_span.end,
                pending_diagnostic: None,
            });
        }

        // The next event must be `＃`. `open_idx + 1 < close_idx` is
        // guaranteed whenever the hash exists, and `close_idx > open_idx`
        // always holds for a paired PairOpen.
        let hash_end = match events.get(open_idx + 1)? {
            PairEvent::Solo {
                kind: TriggerKind::Hash,
                span,
            } => span.end,
            _ => return None,
        };

        // Body bytes are everything between `＃` and `］`. Trim leading /
        // trailing ASCII whitespace to accept malformed input like
        // `［＃ 改ページ  ］`; the Aozora specification does not allow such
        // whitespace, but the corpus contains instances.
        let body = self.source[hash_end as usize..close_span.start as usize].trim();

        // Body-keyword classifier. Each step needs the same `&mut alloc`
        // borrow, so this classifier and the forward recognisers run
        // sequentially with explicit early returns, not as an `or_else`
        // chain.
        let body_start = hash_end
            + u32::try_from(
                self.source[hash_end as usize..close_span.start as usize].len()
                    - self.source[hash_end as usize..close_span.start as usize]
                        .trim_start()
                        .len(),
            )
            .expect("source fits native span");
        let directive_span = Span::new(open_span.start, close_span.end);
        if let Some(
            kind @ (DirectiveKind::UncertainCorrection | DirectiveKind::SourceLegibilityNote),
        ) = editorial_note_kind(body)
        {
            return Some(self.typed_annotation_match(directive_span, kind));
        }
        // A body that opens with a stray `「` and quotes nothing is read as
        // the body after the quote; the published XHTML applies
        // `［＃「地付き］` as `［＃地付き］`.
        let unquoted = body
            .strip_prefix('「')
            .filter(|rest| !rest.contains(['「', '」']));
        if let Some((emit, annotation_payload)) = classify_annotation_body(
            &AnnotationBody {
                text: body,
                start: body_start,
                view,
            },
            self.alloc,
        )
        .or_else(|| {
            classify_annotation_body(
                &AnnotationBody {
                    text: unquoted?,
                    start: body_start + u32::try_from('「'.len_utf8()).ok()?,
                    view,
                },
                self.alloc,
            )
        }) {
            return Some(AnnotationMatch {
                emit,
                // For Warichu open / close the body classifier returns
                // a payload alongside the node; the body-builder uses it to
                // wrap as a `Segment::Directive` with the correct
                // `WarichuOpen` / `WarichuClose` kind instead of the
                // catch-all `Unknown`. Other body-keyword families
                // (PageBreak, Indent, …) leave the payload as `None`, and
                // the body-builder then uses its `Directive{Unknown}`
                // synthesis path.
                annotation_payload,
                consume_start: open_span.start,
                consume_end: close_span.end,
                pending_diagnostic: None,
            });
        }
        // Forward-reference warnings point at the whole `［＃…］` directive,
        // not at the (possibly pulled-back) consume span.
        if let Some((node, consume_start, diag)) =
            self.classify_forward_bouten(view, open_idx, close_idx)
        {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start,
                consume_end: close_span.end,
                pending_diagnostic: diag.into_diagnostic(directive_span),
            });
        }
        // Range bouten `「X」～「Y」に<kind>`: applies the marks to the whole
        // preceding run from X to Y.
        if let Some((node, consume_start)) =
            self.classify_forward_bouten_range(view, open_idx, close_idx)
        {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start,
                consume_end: close_span.end,
                pending_diagnostic: None,
            });
        }
        // `「X」の左に「Y」のルビ`: left-side ruby (saidoku building block). The
        // `の左に` prefix overlaps left-side bouten, but its `のルビ` keyword is
        // not a bouten kind, so the bouten classifier already returned `None`.
        if let Some((node, consume_start)) =
            self.classify_forward_left_ruby(view, open_idx, close_idx)
        {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start,
                consume_end: close_span.end,
                pending_diagnostic: None,
            });
        }
        // `「X」…の注記` / `「X」に「Y」の傍記`: side annotation (注記 / 傍記).
        // The `の注記` / `の傍記` keywords are disjoint from `のルビ` and every
        // bouten kind, so the bouten and left-ruby classifiers above have
        // already declined.
        if let Some((node, consume_start)) =
            self.classify_forward_side_note(view, open_idx, close_idx)
        {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start,
                consume_end: close_span.end,
                pending_diagnostic: None,
            });
        }
        if let Some(mark) = self.recognize_return_mark(view, open_idx, close_idx) {
            return Some(mark);
        }
        // 縦中横 is 3-state: a shape-matched directive whose target has no
        // referent (`ShapedNoTarget`) carries a warning down the fall-through
        // path while the generic instruction retains its selector and effect.
        let tcy_pending = match self.classify_forward_tcy(view, open_idx, close_idx) {
            ForwardTcy::Recognised(node, consume_start, diag) => {
                return Some(AnnotationMatch {
                    emit: EmitKind::Aozora(node),
                    annotation_payload: None,
                    consume_start,
                    consume_end: close_span.end,
                    pending_diagnostic: diag.into_diagnostic(directive_span),
                });
            }
            ForwardTcy::ShapedNoTarget => Some(Diagnostic::tcy_target_not_found(directive_span)),
            ForwardTcy::NotTcy => None,
        };
        if let Some((node, consume_start)) =
            self.classify_forward_heading(view, open_idx, close_idx)
        {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start,
                consume_end: close_span.end,
                pending_diagnostic: tcy_pending,
            });
        }
        // `「X」は「□」囲み`: box-character enclosure (§6.7). Its `は「…」囲み`
        // suffix is disjoint from every emphasis keyword, so its position
        // relative to emphasis does not affect output; it is adjacent as
        // another `は`-form leaf.
        if let Some((node, consume_start, diag)) =
            self.classify_forward_box_enclosure(view, open_idx, close_idx)
        {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start,
                consume_end: close_span.end,
                pending_diagnostic: diag.into_diagnostic(directive_span).or(tcy_pending),
            });
        }
        if let Some((node, consume_start, diag)) =
            self.classify_forward_emphasis(view, open_idx, close_idx)
        {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start,
                consume_end: close_span.end,
                pending_diagnostic: diag.into_diagnostic(directive_span).or(tcy_pending),
            });
        }

        // `<run>［＃mは上ドット付き］` and `［＃「e」は「´」付き］`: a mark
        // supplied to a Latin or Greek letter of the preceding run. The note
        // may address a bare letter (no `「X」` quote), so it runs its own
        // reclaim, not `extract_forward_quote_targets`.
        if let Some((node, consume_start)) = self.classify_forward_accent_dot(view, open_idx, body)
        {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start,
                consume_end: close_span.end,
                pending_diagnostic: tcy_pending,
            });
        }

        // `「caption」のキャプション付きの(図|挿絵)（file）入る`: illustration
        // with a quoted caption reference. Emits an Illustration (consumes the whole
        // bracket); checked here, after the styling recognisers.
        if let Some(node) = self.classify_caption_figure(view, open_idx, close_idx) {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start: open_span.start,
                consume_end: close_span.end,
                pending_diagnostic: tcy_pending,
            });
        }

        // General image form `［＃<説明>（file［、横W×縦H］）入る］` (graphics.html)
        // The free leading description (図 / 地図 / コンドル博士の図 …) is the
        // alt. Its description is arbitrary so it has no prefix needle in the
        // body dispatcher; checked here, after the more-specific 「caption」
        // form so it retains the quoted reference, and before the Unknown catch-all.
        if let Some(emit) = classify_general_image_body(
            body,
            u32::try_from(body.as_ptr().addr() - self.source.as_ptr().addr())
                .expect("source offset fits u32"),
            self.alloc,
        ) {
            return Some(AnnotationMatch {
                emit,
                annotation_payload: None,
                consume_start: open_span.start,
                consume_end: close_span.end,
                pending_diagnostic: None,
            });
        }

        // Empty / placeholder directives from the file-header 凡例 that
        // prefixes nearly every work: `［＃］` (入力者注), `［＃…］` (返り点),
        // `［＃（…）］` (訓点送り仮名). The body is empty (post-trim) or an
        // ellipsis placeholder: a de-facto-standard symbol, not unrecognised
        // notation. Type it as `Empty` rather than the `Unknown` catch-all.
        if body.is_empty() || body == "…" || body == "（…）" {
            return Some(self.typed_annotation_match(directive_span, DirectiveKind::Empty));
        }
        // Input-editor notes (`「X」はママ`, `「X」は底本では「Y」`, …). These
        // do not restyle their target (X stays in the text), so they emit a
        // typed `Directive` consuming only the bracket, exactly like the
        // Unknown catch-all but with the correct kind. Checked here, after the
        // specialised recognisers, so `「ママ」に傍点` etc. are already claimed.
        if let Some(kind) = editorial_note_kind(body) {
            return Some(self.typed_annotation_match(directive_span, kind));
        }

        if indentation_question_locator(body).is_some()
            && self.source[..directive_span.start as usize]
                .rsplit('\n')
                .next()
                .is_some_and(|prefix| prefix.trim().is_empty())
            && self.source[directive_span.end as usize..]
                .split('\n')
                .next()
                .is_some_and(|suffix| !suffix.trim().is_empty())
        {
            return Some(
                self.typed_annotation_match(directive_span, DirectiveKind::IndentationQuestion),
            );
        }

        if let Some(mut instruction) = super::formatting_instruction::parse(body, body_start) {
            super::formatting_instruction::bind_following_lines(
                &mut instruction,
                self.source,
                close_span.end,
            );
            super::formatting_instruction::bind_following_line_rest(
                &mut instruction,
                self.source,
                close_span.end,
            );
            if let Some(start) = formatting_text_start.filter(|start| *start <= open_span.start) {
                super::formatting_instruction::bind(
                    &mut instruction,
                    self.source,
                    Span::new(start, open_span.start),
                );
            }
            let unresolved_tcy = instruction.operations.iter().any(|operation| {
                matches!(
                    operation.effect,
                    FormattingEffect::Attribute(ForwardAttr::CombineUpright)
                ) && operation.selection.is_none()
            });
            let raw = &self.source[open_span.start as usize..close_span.end as usize];
            let directive = self.alloc.formatting_instruction(raw, instruction);
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(Node::Directive(directive)),
                annotation_payload: Some(directive),
                consume_start: open_span.start,
                consume_end: close_span.end,
                pending_diagnostic: unresolved_tcy.then(|| {
                    tcy_pending.unwrap_or_else(|| Diagnostic::tcy_target_not_found(directive_span))
                }),
            });
        }

        // Standalone external-character: a no-`※` `［＃…］` whose body
        // is a gaiji description with a trailing mencode / 底本ページ-行
        // (`「※」は「祿－示」、第3水準1-84-27、144-上-9`, `「比」の「ヒ」に代えて「く」、
        // 第4水準2-1-23`). Checked after the editorial notes so a `底本では` note
        // with a coincidental code-like tail stays editorial, and immediately before
        // the Unknown catch-all. `classify_standalone_gaiji` requires a
        // resolvable glyph or a mencode/page-line tail, so an ordinary
        // `［＃「…」］` note is never claimed.
        if let Some((node, unresolved, consume_start)) =
            self.classify_standalone_gaiji(view, open_idx, formatting_text_start)
        {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start,
                consume_end: close_span.end,
                pending_diagnostic: unresolved
                    .then(|| Diagnostic::unresolved_gaiji(directive_span)),
            });
        }

        if is_gloss_body(body)
            && canonical_directive(body).is_none()
            && self.source[..directive_span.start as usize]
                .chars()
                .next_back()
                .is_some_and(|character| !character.is_whitespace())
        {
            return Some(
                self.typed_annotation_match(directive_span, DirectiveKind::ExplanationNote),
            );
        }

        // No specialised recogniser claimed the bracket; fall back to the
        // `Directive{Unknown}` catch-all.
        Some(self.unknown_annotation_match(directive_span, body, tcy_pending))
    }

    /// Standalone (no-`※`) external-character recogniser. Reuses the
    /// gaiji body parser with the bracket open as the consume start, and
    /// declines unless the body resolves to a glyph or carries a mencode /
    /// 底本ページ-行 tail, so an ordinary `［＃「…」］` note is never claimed.
    /// Returns the gaiji node and whether it stayed unresolved.
    fn classify_standalone_gaiji(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        pending_plain_start: Option<u32>,
    ) -> Option<(Node, bool, u32)> {
        let &PairEvent::PairOpen {
            span: open_span, ..
        } = view.events.get(open_idx)?
        else {
            return None;
        };
        let m = self.recognize_gaiji_core(view, open_span.start, true, open_idx, false)?;
        // Standalone has no `※` disambiguator, so an unqualified `［＃「…」］`
        // is an ordinary note. Require a resolved glyph or a mencode /
        // page-line tail before claiming it as a gaiji.
        if !m.payload.canonical.has_mencode() && m.payload.resolve(self.alloc.store()).is_none() {
            return None;
        }
        let unresolved = m.payload.resolve(self.alloc.store()).is_none();
        let Some((target, target_start)) = retrospective_gaiji_target(
            self.source,
            open_span.start,
            self.alloc.store().resolve_str(m.payload.hint),
            m.payload.resolve(self.alloc.store()),
            pending_plain_start,
        ) else {
            return Some((self.alloc.gaiji(m.payload), unresolved, open_span.start));
        };
        let target = self.alloc.content_plain(target);
        Some((
            self.alloc.gaiji_replacing(m.payload, target),
            unresolved,
            target_start,
        ))
    }

    /// Catch-all for any well-formed `［＃…］` whose body no specialised
    /// recogniser claimed, including empty bodies (`［＃］`), which real
    /// Aozora corpora occasionally use as illustrative glyphs. Emitting
    /// `Directive{Unknown}` with the raw source slice keeps the Tier-A
    /// canary (no bare `［＃` in HTML output) intact. `directive_span` is the
    /// `open.start..close.end` extent of the bracket.
    ///
    /// `tcy_pending` carries a 縦中横-target-not-found warning down from the
    /// fall-through path; otherwise a `ここから…` opener that no
    /// `ContainerKind` claimed is reported as `unrecognised_container_directive`
    /// (`罫囲み` / `割り注` openers don't use the `ここから` prefix and are
    /// handled upstream, so the prefix uniquely flags a stray container open).
    fn unknown_annotation_match(
        &mut self,
        directive_span: Span,
        body: &str,
        tcy_pending: Option<Diagnostic>,
    ) -> AnnotationMatch {
        let raw = &self.source[directive_span.start as usize..directive_span.end as usize];
        // One payload for `emit`, one for `annotation_payload`, so the
        // body-builder can re-wrap without re-interning the raw string.
        let payload = self.alloc.make_directive(raw, DirectiveKind::Unknown);
        let node = self.alloc.annotation(payload);
        let payload_for_seg = self.alloc.make_directive(raw, DirectiveKind::Unknown);
        // Notation-hygiene lint: a body that is a verified near-miss of a
        // recognized directive (kept as Unknown) gets a canonical-form
        // suggestion. Catalogue bodies are disjoint from the ここから / 縦中横
        // cases below, so the priority order does not affect them and at
        // most one diagnostic is emitted.
        let pending_diagnostic = canonical_directive(body)
            .map(|canonical| Diagnostic::non_canonical_directive(directive_span, canonical))
            .or(tcy_pending)
            .or_else(|| {
                body.starts_with("ここから")
                    .then(|| Diagnostic::unrecognised_container_directive(directive_span))
            });
        AnnotationMatch {
            emit: EmitKind::Aozora(node),
            annotation_payload: Some(payload_for_seg),
            consume_start: directive_span.start,
            consume_end: directive_span.end,
            pending_diagnostic,
        }
    }

    /// Emit a `［＃…］` as an `Directive` carrying a specific kind
    /// (`Sic`, `BaseTextVariant`, …) rather than the `Unknown` catch-all.
    /// Same span discipline as [`Self::unknown_annotation_match`]: the
    /// whole bracket is consumed and the target text is left in place, so
    /// the raw round-trips and the HTML output is unchanged (these kinds
    /// render hidden, as Unknown does); only the typed `DirectiveKind` in
    /// the AST / wire output differs.
    fn typed_annotation_match(
        &mut self,
        directive_span: Span,
        kind: DirectiveKind,
    ) -> AnnotationMatch {
        let raw = &self.source[directive_span.start as usize..directive_span.end as usize];
        let payload = self.alloc.make_directive(raw, kind);
        let node = self.alloc.annotation(payload);
        let payload_for_seg = self.alloc.make_directive(raw, kind);
        AnnotationMatch {
            emit: EmitKind::Aozora(node),
            annotation_payload: Some(payload_for_seg),
            consume_start: directive_span.start,
            consume_end: directive_span.end,
            pending_diagnostic: None,
        }
    }
}

/// Classify a `［＃「target」に<bouten-kind>］` forward-reference
/// bouten annotation.
///
/// Uses the event-stream layout to find the target quote pair, so a
/// target whose text contains nested `「…」` is not truncated at the
/// first `」`. The pair stage has already balanced the
/// quotes so the target's extent is unambiguous.
///
/// Expected event layout for a valid forward bouten:
///
/// ```text
/// open_idx         PairOpen(Bracket)
/// open_idx + 1     Solo(Hash)                [already verified]
/// open_idx + 2     PairOpen(Quote, close=Q)
/// …                body events               [usually only Text]
/// Q                PairClose(Quote)
/// Q+1..close_idx   suffix events             [usually Text("に…")]
/// close_idx        PairClose(Bracket)
/// ```
impl RecogniseCtx<'_, '_> {
    /// Classify a range bouten `「X」～「Y」に<kind>` (also `〜`), or
    /// `「X」から「Y」まで<kind>`: apply the marks to the whole preceding run
    /// from the start of X to the end of Y, which ends at the bracket. Returns `(node, consume_start)` with
    /// `consume_start` at X so the styled span is the run's sole rendered copy.
    fn classify_forward_bouten_range(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(Node, u32)> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        // Extraction stops at the `～`, so the first quote is X; the suffix is
        // `～「Y」に<kind>` (or `〜「Y」…`).
        let [start_target] = extracted.targets.as_slice() else {
            return None;
        };
        let (rest, until) = match extracted.suffix.strip_prefix("から「") {
            Some(rest) => (rest, true),
            None => (
                extracted
                    .suffix
                    .strip_prefix("～「")
                    .or_else(|| extracted.suffix.strip_prefix("〜「"))?,
                false,
            ),
        };
        let (end_target, after) = rest.split_once('」')?;
        if start_target.is_empty() || end_target.is_empty() {
            return None;
        }
        let after = if until {
            after.strip_prefix("まで")?
        } else {
            after
        };
        let (position, kind_suffix) = if until && bouten_kind_from_suffix(after).is_some() {
            (DecorationPosition::Right, after)
        } else if let Some(r) = after.strip_prefix("に") {
            (DecorationPosition::Right, r)
        } else if let Some(r) = after.strip_prefix("の左に") {
            (DecorationPosition::Left, r)
        } else {
            (DecorationPosition::Both, after.strip_prefix("の両側に")?)
        };
        let kind = bouten_kind_from_suffix(kind_suffix)?;
        let &PairEvent::PairOpen {
            span: open_span, ..
        } = view.events.get(open_idx)?
        else {
            return None;
        };
        // The run ends at the bracket with Y; its start is the last X before Y.
        let cutoff = open_span.start as usize;
        let prefix = &self.source[..cutoff];
        if !prefix.ends_with(end_target) {
            return None;
        }
        let y_start = cutoff - end_target.len();
        let x_start = self.source[..y_start].rfind(start_target)?;
        let phrase = &self.source[x_start..cutoff];
        let content = self.alloc.content_plain(phrase);
        Some((
            self.alloc
                .bouten(kind, content, position, ForwardOrigin::Reclaimed),
            u32::try_from(x_start).ok()?,
        ))
    }

    fn classify_forward_bouten(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(Node, u32, ForwardDiag)> {
        let (mut node, start, diagnostic) =
            self.classify_forward_bouten_inner(view, open_idx, close_idx)?;
        if let (PairEvent::PairOpen { span: open, .. }, PairEvent::PairClose { span: close, .. }) =
            (&view.events[open_idx], &view.events[close_idx])
        {
            let marker = &self.source[open.start as usize..close.end as usize];
            if (formatting_edition_note(marker).is_some() || physical_line_marker(marker).is_some())
                && let Node::Format(format) = &mut node
                && let Content::Plain(body) = self
                    .alloc
                    .content_plain(&marker["［＃".len()..marker.len() - '］'.len_utf8()])
            {
                format.annotation_body = Some(body);
            }
        }
        Some((node, start, diagnostic))
    }

    fn classify_forward_bouten_inner(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(Node, u32, ForwardDiag)> {
        if let Some(selection) = extract_bouten_selection(view, self.source, open_idx, close_idx) {
            // `の間に` locates a mark at a junction rather than selecting a
            // range to decorate, so it produces a note rather than emphasis.
            if let Some(mark) = between_mark_keyword(selection.suffix) {
                return self
                    .classify_between_mark(view, open_idx, close_idx, &selection, mark)
                    .map(|(node, start)| (node, start, ForwardDiag::None));
            }
            return self.classify_selected_bouten(view, open_idx, close_idx, &selection);
        }
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        let (PairEvent::PairOpen { span: open, .. }, PairEvent::PairClose { span: close, .. }) =
            (&view.events[open_idx], &view.events[close_idx])
        else {
            return None;
        };
        let marker = &self.source[open.start as usize..close.end as usize];
        let suffix = formatting_edition_note(marker).map_or(extracted.suffix, |(suffix, _)| suffix);
        // Shape 1: `に<kind>`: default right-side placement.
        // Shape 2: `の左に<kind>`: left-side placement (position flipped).
        // Shape 3: `の両側に<kind>`: both sides.
        let (position, kind) = if let Some(position) = physical_line_suffix(suffix) {
            (position, BoutenKind::UnderLine)
        } else if let Some(rest) = suffix.strip_prefix("に") {
            (DecorationPosition::Right, bouten_kind_from_suffix(rest)?)
        } else if let Some(rest) = suffix.strip_prefix("の左に") {
            (DecorationPosition::Left, bouten_kind_from_suffix(rest)?)
        } else if let Some(rest) = suffix.strip_prefix("の両側に") {
            (DecorationPosition::Both, bouten_kind_from_suffix(rest)?)
        } else if matches!(suffix, "は傍点" | "の傍点" | "傍点") {
            (DecorationPosition::Right, BoutenKind::Goma)
        } else {
            return None;
        };
        let &PairEvent::PairOpen {
            span: open_span, ..
        } = view.events.get(open_idx)?
        else {
            return None;
        };
        let [only] = extracted.targets.as_slice() else {
            return None;
        };
        let only = if *only == "※" {
            preceding_refmark_reference(self.source, open_span.start).unwrap_or(only)
        } else {
            only
        };
        if !forward_target_is_preceded(view.events, self.source, open_idx, only) {
            return Some(self.resolve_forward_format(
                view,
                open_idx,
                open_span.start,
                ForwardAttr::Bouten { kind, position },
                only,
            ));
        }
        let adjacent =
            find_immediate_predecessor_target_position(view.events, self.source, open_idx, only)
                .is_some();
        let (node, consume_start, diag) = self.resolve_forward_format(
            view,
            open_idx,
            open_span.start,
            ForwardAttr::Bouten { kind, position },
            only,
        );
        let diag = if !adjacent
            && matches!(diag, ForwardDiag::None)
            && self.source[..open_span.start as usize]
                .matches(only)
                .count()
                >= 2
        {
            ForwardDiag::Ambiguous
        } else {
            diag
        };
        Some((node, consume_start, diag))
    }

    /// Classify `［＃「X」と「Y」の間に<mark>］`: a mark the source places at the
    /// junction of two adjacent runs.
    ///
    /// The result is a note rather than emphasis, because the source states a
    /// position and not a range: there is nothing for a decoration to cover.
    /// The two runs together become the base, so the position stays
    /// addressable, and no principal character is derived from a marker that
    /// identifies neither.
    fn classify_between_mark(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
        selection: &BoutenSelection<'_>,
        mark: &str,
    ) -> Option<(Node, u32)> {
        let PairEvent::PairOpen { span: open, .. } = view.events[open_idx] else {
            return None;
        };
        let PairEvent::PairClose { .. } = view.events[close_idx] else {
            return None;
        };
        let start = self.pending_plain_start?;
        let spans = selection.resolve(
            self.source,
            Span::new(start, open.start),
            SelectionSense::Named,
        )?;
        // A junction exists only where the runs meet. Runs the source quotes
        // with anything between them locate no single position, so the
        // recogniser declines the marker rather than choosing one of the gaps.
        let [first, second] = spans.as_slice() else {
            return None;
        };
        if first.end != second.start {
            return None;
        }
        let base_span = Span::new(first.start, second.end);
        // The mark keyword is a literal substring of the marker, so the note
        // text is source text with an exact extent rather than a label
        // supplied here.
        let note_start = mark.as_ptr().addr() - self.source.as_ptr().addr();
        let note_span = Span::new(
            u32::try_from(note_start).ok()?,
            u32::try_from(note_start + mark.len()).ok()?,
        );
        let left = self
            .alloc
            .content_plain(&self.source[first.start as usize..first.end as usize]);
        let right = self
            .alloc
            .content_plain(&self.source[second.start as usize..second.end as usize]);
        let note = self.alloc.content_plain(mark);
        let mut result = self.alloc.between_mark(left, right, note, note_span);
        if let Node::MarginNote(note) = &mut result {
            note.target_span =
                Some(NonEmptySpan::new(base_span).expect("resolved junction base is nonempty"));
        }
        Some((result, base_span.start))
    }

    fn classify_selected_bouten(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
        selection: &BoutenSelection<'_>,
    ) -> Option<(Node, u32, ForwardDiag)> {
        let PairEvent::PairOpen { span: open, .. } = view.events[open_idx] else {
            return None;
        };
        let PairEvent::PairClose { span: close, .. } = view.events[close_idx] else {
            return None;
        };
        let (sense, suffix) = match selection.suffix.strip_prefix("を除く部分に") {
            Some(rest) => (SelectionSense::Complement, rest),
            None => (SelectionSense::Named, selection.suffix.strip_prefix("に")?),
        };
        let kind = bouten_kind_from_suffix(suffix)?;
        let start = self.pending_plain_start?;
        let spans = selection.resolve(self.source, Span::new(start, open.start), sense)?;
        let attr = ForwardAttr::Bouten {
            kind,
            position: DecorationPosition::Right,
        };
        Some(self.decorate_selected_ranges(open, close, attr, &spans))
    }

    fn decorate_selected_ranges(
        &mut self,
        open: Span,
        close: Span,
        attr: ForwardAttr,
        spans: &[Span],
    ) -> (Node, u32, ForwardDiag) {
        let contents: Vec<_> = spans
            .iter()
            .map(|span| {
                self.alloc
                    .content_plain(&self.source[span.start as usize..span.end as usize])
            })
            .collect();
        let body = &self.source[(open.end as usize + '＃'.len_utf8())..close.start as usize];
        let reference = self.alloc.selected_forward_format(attr, &contents, body);
        for (index, &span) in spans.iter().enumerate() {
            let mut detached = reference;
            detached.origin = ForwardOrigin::Detached;
            detached.target.start += u32::try_from(index).expect("target index fits arena");
            detached.target.len = 1;
            self.pending_decorations
                .push((Node::Format(detached), span));
        }
        (Node::Format(reference), open.start, ForwardDiag::None)
    }
}

fn physical_line_marker(marker: &str) -> Option<DecorationPosition> {
    let body = marker.strip_prefix("［＃「")?.strip_suffix('］')?;
    let (_, suffix) = body.split_once('」')?;
    physical_line_suffix(suffix)
}

fn physical_line_suffix(suffix: &str) -> Option<DecorationPosition> {
    if suffix == "に下線" {
        return Some(DecorationPosition::Below);
    }
    if suffix == "は上線（￣）付き" {
        return Some(DecorationPosition::Above);
    }
    let locator = suffix.strip_prefix("は下線（_）付き、")?;
    valid_table_line_locator(locator).then_some(DecorationPosition::Below)
}

fn valid_table_line_locator(locator: &str) -> bool {
    let Some((page, line)) = locator.split_once("-表組") else {
        return false;
    };
    let Some(line) = line.strip_suffix("行目") else {
        return false;
    };
    !page.is_empty()
        && !line.is_empty()
        && page
            .chars()
            .all(|ch| ch.is_ascii_digit() || ('０'..='９').contains(&ch))
        && line
            .chars()
            .all(|ch| ch.is_ascii_digit() || ('０'..='９').contains(&ch))
}

/// The quoted operands select source ranges; they do not supply new text.
struct BoutenSelection<'a> {
    targets: BoutenTargets<'a>,
    suffix: &'a str,
}

enum BoutenTargets<'a> {
    Disjoint(Vec<&'a str>),
    Within { context: &'a str, target: &'a str },
}

/// Which side of the inner operand the source asked for.
///
/// The connector establishes that one quoted operand lies inside the other;
/// it does not establish whether the inner one is the selection or the
/// cut-out. Only the suffix states that, so both senses are read from it.
#[derive(Clone, Copy)]
enum SelectionSense {
    /// `の「Y」に…`: the range Y quotes.
    Named,
    /// `の「Y」を除く部分に…`: the context apart from what Y quotes.
    Complement,
}

impl BoutenSelection<'_> {
    fn resolve(&self, source: &str, pending: Span, sense: SelectionSense) -> Option<Vec<Span>> {
        let text = &source[pending.start as usize..pending.end as usize];
        if text.contains(['\n', '\r']) {
            return None;
        }
        if let BoutenTargets::Within { context, target } = &self.targets {
            let context_start = text.strip_suffix(context)?.len();
            let base = pending.start + u32::try_from(context_start).ok()?;
            return match sense {
                SelectionSense::Named => {
                    let offset = unique_selected_offset(context, target)?;
                    let start = base + u32::try_from(offset).ok()?;
                    Some(vec![Span::new(
                        start,
                        start + u32::try_from(target.len()).ok()?,
                    )])
                }
                SelectionSense::Complement => complement_spans(base, context, target),
            };
        }
        if matches!(sense, SelectionSense::Complement) {
            return None;
        }
        let BoutenTargets::Disjoint(targets) = &self.targets else {
            return None;
        };
        text.strip_suffix(*targets.last()?)?;
        let mut spans = Vec::with_capacity(targets.len());
        let mut before = text.len();
        for target in targets.iter().rev() {
            let start = text[..before].rfind(target)?;
            let end = start + target.len();
            spans.push(Span::new(
                pending.start + u32::try_from(start).ok()?,
                pending.start + u32::try_from(end).ok()?,
            ));
            before = start;
        }
        spans.reverse();
        let repeated = targets
            .iter()
            .any(|target| text.match_indices(target).nth(1).is_some());
        if repeated
            && spans.windows(2).any(|pair| {
                text[pair[0].end as usize - pending.start as usize
                    ..pair[1].start as usize - pending.start as usize]
                    .chars()
                    .any(char::is_alphanumeric)
            })
        {
            return None;
        }
        Some(spans)
    }
}

/// The refmark glyph reference `※［＃…］` that ends at `end`. A quoted `※`
/// directly after such a reference refers to that glyph.
fn preceding_refmark_reference(source: &str, end: u32) -> Option<&str> {
    let end = end as usize;
    let (start, span_end) = gaiji_resolve::find_span(source, end.checked_sub(1)?)?;
    (span_end == end && source[start..].starts_with('※')).then(|| &source[start..end])
}

/// The mark keyword of a `の間に<mark>` suffix, as a substring of the source.
///
/// Only spellings the mark vocabulary already recognises are read. The keyword
/// is returned rather than its parsed kind because the note carries the
/// source's own word, with the exact extent it occupies in the marker.
fn between_mark_keyword(suffix: &str) -> Option<&str> {
    let mark = suffix.strip_prefix("の間に")?;
    bouten_kind_from_suffix(mark).map(|_| mark)
}

/// The ranges of `context` that `excluded` does not cover.
///
/// The source states what to leave out rather than what to mark, and one
/// statement covers every occurrence: `「自律・自由・人格・性格」の「・」を除く部分`
/// leaves out all three separators, not the first one only. A gap of no
/// length contributes no range, so an excluded run at either edge, or two of
/// them in a row, adds nothing to select. A cut-out the context does not
/// contain declines the marker rather than selecting the whole context.
fn complement_spans(base: u32, context: &str, excluded: &str) -> Option<Vec<Span>> {
    if excluded.is_empty() || !context.contains(excluded) {
        return None;
    }
    let mut spans = Vec::new();
    let mut cursor = 0usize;
    for piece in context.split(excluded) {
        if !piece.is_empty() {
            let start = base + u32::try_from(cursor).ok()?;
            spans.push(Span::new(start, start + u32::try_from(piece.len()).ok()?));
        }
        cursor += piece.len() + excluded.len();
    }
    (!spans.is_empty()).then_some(spans)
}

fn unique_selected_offset(text: &str, target: &str) -> Option<usize> {
    let start = text.find(target)?;
    let next = start + text[start..].chars().next()?.len_utf8();
    (!text[next..].contains(target)).then_some(start)
}

/// Read only complete native quote pairs, with one consistent relation between operands.
fn extract_bouten_selection<'a>(
    view: BodyView<'_>,
    source: &'a str,
    open_idx: usize,
    close_idx: usize,
) -> Option<BoutenSelection<'a>> {
    let PairEvent::PairClose { span: close, .. } = view.events[close_idx] else {
        return None;
    };
    if !matches!(
        view.events.get(open_idx + 2),
        Some(PairEvent::PairOpen {
            kind: PairKind::Quote,
            ..
        })
    ) {
        return None;
    }
    let mut cursor = open_idx + 2;
    let mut targets = Vec::new();
    let mut previous_end = None;
    let mut relation = None;
    let mut last_end = 0;
    while cursor < close_idx {
        let event = &view.events[cursor];
        let PairEvent::PairOpen {
            kind: PairKind::Quote,
            span: open,
        } = event
        else {
            cursor += 1;
            continue;
        };
        if let Some(end) = previous_end {
            let connector = &source[end..open.start as usize];
            if !matches!(connector, "" | "と" | "の" | "、") {
                break;
            }
            if relation.is_some_and(|previous| previous != connector) {
                return None;
            }
            relation = Some(connector);
        }
        let end_index = usize::try_from(*view.links.get(cursor)?).ok()?;
        if end_index >= close_idx {
            return None;
        }
        let PairEvent::PairClose { span: end, .. } = view.events[end_index] else {
            return None;
        };
        let target = &source[open.end as usize..end.start as usize];
        if target.is_empty() {
            return None;
        }
        targets.push(target);
        last_end = end.end as usize;
        previous_end = Some(last_end);
        cursor = end_index + 1;
    }
    if targets.len() < 2 {
        return None;
    }
    let targets = if relation == Some("の") {
        let [context, target] = targets.as_slice() else {
            return None;
        };
        BoutenTargets::Within { context, target }
    } else {
        BoutenTargets::Disjoint(targets)
    };
    Some(BoutenSelection {
        targets,
        suffix: &source[last_end..close.start as usize],
    })
}

/// Outcome of [`RecogniseCtx::classify_forward_tcy`].
///
/// Distinguishes a recognised 縦中横 from a directive whose `は縦中横`
/// shape matched but whose target is absent from the look-back (which
/// still warrants a `tcy_target_not_found` warning), and from a bracket
/// that is not a 縦中横 directive at all (fall-through without a diagnostic).
enum ForwardTcy {
    /// A 縦中横 with a located target: the node, its consume start, and the
    /// directive-level diagnostic (`NotStylable` for a declined referent).
    Recognised(Node, u32, ForwardDiag),
    /// `は縦中横` shape matched but the target has no preceding referent.
    ShapedNoTarget,
    /// Not a 縦中横 directive.
    NotTcy,
}

impl RecogniseCtx<'_, '_> {
    fn recognize_return_mark(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<AnnotationMatch> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        let [target] = extracted.targets.as_slice() else {
            return None;
        };
        if extracted.suffix != "は返り点" || !is_return_mark(target) {
            return None;
        }
        let start =
            find_immediate_predecessor_target_position(view.events, self.source, open_idx, target)?;
        if start < self.pending_plain_start? {
            return None;
        }
        let PairEvent::PairClose { span, .. } = view.events[close_idx] else {
            return None;
        };
        Some(AnnotationMatch {
            emit: EmitKind::Aozora(self.alloc.kunten(KuntenKind::ReturnMark, target)),
            annotation_payload: None,
            consume_start: start,
            consume_end: span.end,
            pending_diagnostic: None,
        })
    }

    /// Classify a `［＃「target」は縦中横］` forward-reference
    /// tate-chu-yoko (horizontal-in-vertical) annotation.
    ///
    /// Same event-layout expectations as forward bouten, except the
    /// suffix uses the particle `は` and the keyword `縦中横`. Paired
    /// form (`［＃縦中横］…［＃縦中横終わり］`) is handled by the
    /// paired-container classifier and not matched here.
    ///
    /// The compound form adds right-side small script to the same target. An
    /// edition assertion is retained with that formatting; unrecognized clauses
    /// and multiple formatting targets remain raw directives.
    fn classify_forward_tcy(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> ForwardTcy {
        let (PairEvent::PairOpen { span: opener, .. }, PairEvent::PairClose { span: closer, .. }) =
            (&view.events[open_idx], &view.events[close_idx])
        else {
            return ForwardTcy::NotTcy;
        };
        let marker = &self.source[opener.start as usize..closer.end as usize];
        let Some(extracted) = extract_forward_quote_targets(view, self.source, open_idx, close_idx)
            .or_else(|| {
                let target = marker.strip_prefix("［＃")?.strip_suffix("は縦中横］")?;
                let body = target.strip_prefix('（')?.strip_suffix('）')?;
                if body.is_empty() || body.contains(['（', '）', '［', '］', '\n', '\r']) {
                    return None;
                }
                Some(ForwardTargetExtract {
                    targets: smallvec::smallvec![target],
                    suffix: "は縦中横",
                })
            })
        else {
            return ForwardTcy::NotTcy;
        };
        let edition = formatted_text_variant(marker);
        let suffix = edition.as_ref().map_or_else(
            || {
                extracted
                    .suffix
                    .strip_prefix('は')
                    .unwrap_or(extracted.suffix)
            },
            |(formatting, _)| *formatting,
        );
        let attrs = match suffix {
            // `縦中横組み` and a stray closing `」` after `縦中横` are attested
            // spellings of the same instruction.
            "縦中横" | "縦中横組み" | "縦中横」" | "一字" | "１マスに横並び" => {
                &[ForwardAttr::CombineUpright][..]
            }
            "縦中横、行右小書き" => &[
                ForwardAttr::CombineUpright,
                ForwardAttr::SmallScript(BoutenPosition::Right),
            ][..],
            _ => return ForwardTcy::NotTcy,
        };
        // Single target only: a multi-quote `「A」「B」は縦中横` is not an
        // Aozora shape and declines to `Directive{Unknown}`, as emphasis
        // does; keeping only the first target would drop data.
        let [first] = extracted.targets.as_slice() else {
            return ForwardTcy::NotTcy;
        };
        // The shape is a 縦中横 directive. If its target has no referent in
        // the preceding text the styling is meaningless; flag it (the
        // caller turns this into `tcy_target_not_found`) and let the generic
        // instruction path preserve the unresolved selector and effect.
        if !forward_target_is_preceded(view.events, self.source, open_idx, first) {
            return ForwardTcy::ShapedNoTarget;
        }
        // Resolve the target position, shared with the emphasis / box
        // families: `昭和64［＃「64」は縦中横］年` (adjacent) pulls `64` back into a
        // `Reclaimed` tcy; a non-adjacent plain referent splices a `Detached`
        // tcy decoration; a declined referent stays `Referenced` + reports it.
        let Some(&PairEvent::PairOpen {
            span: open_span, ..
        }) = view.events.get(open_idx)
        else {
            return ForwardTcy::NotTcy;
        };
        let (mut node, consume_start, diag) =
            self.resolve_forward_formats(view, open_idx, open_span.start, attrs, first);
        if edition.is_some()
            && let Node::Format(format) = &mut node
            && let Content::Plain(body) = self
                .alloc
                .content_plain(&marker["［＃".len()..marker.len() - '］'.len_utf8()])
        {
            format.annotation_body = Some(body);
        }
        ForwardTcy::Recognised(node, consume_start, diag)
    }
}

/// Check whether `target` appears somewhere in the source preceding the
/// `［` event at `open_idx`. Used by forward-reference recognisers to
/// suppress `［＃「X」…］` spans whose target has no referent.
///
/// Returns `false` if the event shape isn't the expected `PairOpen`
/// (the caller selects a valid bracket, so this branch is reached only
/// if that invariant fails).
fn forward_target_is_preceded(
    events: &[PairEvent],
    source: &str,
    open_idx: usize,
    target: &str,
) -> bool {
    #[cfg(feature = "classify-instrument")]
    let _classify_guard = SubsystemGuard::new(Subsystem::ForwardTargetCheck);
    let Some(PairEvent::PairOpen { span, .. }) = events.get(open_idx) else {
        return false;
    };
    let cutoff = span.start;

    // The index contains quoted targets. Unquoted operands use the source
    // lookup when no cached position exists.
    let indexed = FORWARD_TARGET_INDEX.with(|cell| {
        let state = cell.borrow();
        if !state.installed {
            return None;
        }
        state
            .first_position
            .get(target)
            .map(|&first_pos| first_pos < cutoff)
    });
    if let Some(decided) = indexed {
        return decided;
    }

    source[..cutoff as usize].contains(target)
}

fn forward_target_is_preceded_in_visible_text(
    view: BodyView<'_>,
    source: &str,
    open_idx: usize,
    target: &str,
) -> bool {
    let Some(&PairEvent::PairOpen { span, .. }) = view.events.get(open_idx) else {
        return false;
    };
    let prefix = &source[..span.start as usize];

    // Drop every `《reading》` span, explicit-base `｜` and `［＃…］` marker. An
    // unclosed `《` or `［＃` drops the tail, so the check can miss a
    // preceding target but does not report one that is absent.
    let mut stripped = String::with_capacity(prefix.len());
    let mut in_ruby = false;
    let mut marker_depth = 0_usize;
    let mut chars = prefix.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '［' if marker_depth > 0 => marker_depth += 1,
            '］' if marker_depth > 0 => marker_depth -= 1,
            _ if marker_depth > 0 => {}
            '［' if chars.peek() == Some(&'＃') => marker_depth = 1,
            '《' => in_ruby = true,
            '》' => in_ruby = false,
            '｜' => {}
            _ if in_ruby => {}
            _ => stripped.push(ch),
        }
    }
    stripped.contains(target)
}

/// Like [`forward_target_is_preceded`] but stricter: returns the byte
/// position where `target` begins only if that target ends directly
/// at the `［` event at `open_idx`. Used by the
/// `classify_forward_*` family to compute the `consume_start` they
/// return to `try_bracket_emit`, so the flush truncates the
/// pending plain run immediately past the matched literal and the styled
/// span becomes its sole rendered copy.
///
/// Returns `None` when:
/// - the event at `open_idx` is not a `PairOpen` (defensive),
/// - the bracket is at byte offset < `target.len()` (no room),
/// - or the bytes immediately before the bracket differ from
///   `target` (the target is not the immediately preceding source text).
fn find_immediate_predecessor_target_position(
    events: &[PairEvent],
    source: &str,
    open_idx: usize,
    target: &str,
) -> Option<u32> {
    let &PairEvent::PairOpen { span, .. } = events.get(open_idx)? else {
        return None;
    };
    let cutoff = span.start as usize;
    let len = target.len();
    if cutoff < len {
        return None;
    }
    let candidate_start = cutoff - len;
    // Compare bytes; `target` is already canonical UTF-8 (extracted
    // from `extract_forward_quote_targets`), so equality of byte
    // slices is equivalent to equality of strings.
    if &source.as_bytes()[candidate_start..cutoff] == target.as_bytes() {
        Some(u32::try_from(candidate_start).ok()?)
    } else {
        None
    }
}

fn retrospective_gaiji_target<'a>(
    source: &'a str,
    marker_start: u32,
    hint: &str,
    resolved: Option<Resolved>,
    pending_plain_start: Option<u32>,
) -> Option<(&'a str, u32)> {
    fn digit_value(ch: char) -> Option<u32> {
        match ch {
            '1'..='7' => Some(u32::from(ch) - u32::from('0')),
            '１'..='７' => Some(u32::from(ch) - u32::from('０')),
            _ => None,
        }
    }

    let end = usize::try_from(marker_start).ok()?;
    let before = source.get(..end)?;
    let target = {
        let rest = hint.strip_prefix('「')?;
        let declared = rest.chars().next()?;
        if let Some(digit) = digit_value(declared) {
            let rest = &rest[declared.len_utf8()..];
            let suffix = rest.strip_prefix("」はローマ数字")?;
            if !suffix.is_empty()
                && (suffix.chars().count() != 1
                    || suffix.chars().next().and_then(digit_value) != Some(digit))
            {
                return None;
            }
            let expected = match digit {
                1 => 'Ⅰ',
                2 => 'Ⅱ',
                3 => 'Ⅲ',
                4 => 'Ⅳ',
                5 => 'Ⅴ',
                6 => 'Ⅵ',
                7 => 'Ⅶ',
                _ => return None,
            };
            if resolved != Some(Resolved::Char(expected)) {
                return None;
            }
            let actual = before.chars().next_back()?;
            (digit_value(actual) == Some(digit))
                .then_some(&before[before.len() - actual.len_utf8()..])?
        } else {
            let close = rest.find('」')?;
            let declared = &rest[..close];
            let suffix = &rest[close + '」'.len_utf8()..];
            let eligible = match (declared, suffix) {
                (
                    "?!" | "!!" | "？！" | "！？" | "！！",
                    "は横１文字" | "は一文字" | "は１マスに横並び",
                ) => resolved.is_some(),
                ("!!!", "は一文字") => true,
                _ if encoded_retrospective_gaiji_target(hint, resolved) == Some(declared) => {
                    resolved.is_some()
                }
                _ => return None,
            };
            if !eligible {
                return None;
            }
            before
                .strip_suffix(declared)
                .map(|_| &before[before.len() - declared.len()..])?
        }
    };
    let start = end.checked_sub(target.len())?;
    let start = u32::try_from(start).ok()?;
    if pending_plain_start.is_none_or(|pending| pending > start) {
        return None;
    }
    Some((&source[start as usize..end], start))
}

/// Exact source target declared by an encoded retrospective character note.
/// The descriptor and resolved JIS value must both agree; a matching quoted
/// character or code alone is insufficient.
pub(crate) fn encoded_retrospective_gaiji_target(
    hint: &str,
    resolved: Option<Resolved>,
) -> Option<&str> {
    for (body, target, expected) in [
        ("「E」はアキュートアクセント付きE", "E", Resolved::Char('É')),
        (
            "「プ」は小書き半濁点付き片仮名フ",
            "プ",
            Resolved::Multi("ㇷ゚"),
        ),
        ("「ム」は小書き片仮名ム", "ム", Resolved::Char('ㇺ')),
    ] {
        if hint == body && resolved == Some(expected) {
            return Some(target);
        }
    }
    None
}

/// Where a forward-reference target `X` resolves relative to its `［`, given
/// the current pending plain run: the three-way generalisation of
/// [`find_immediate_predecessor_target_position`] for interior referents.
///
/// - [`Adjacent`](ForwardReferent::Adjacent): `X` ends at the bracket; pull it
///   back into a `Reclaimed` node (case A).
/// - [`Interior`](ForwardReferent::Interior): `X` is a plain occurrence inside
///   the pending run but not adjacent; the caller materialises a `Detached`
///   decoration at `[start, end)` and keeps the bracket `Referenced` (case B).
/// - [`Unresolvable`](ForwardReferent::Unresolvable): `X` is present in the
///   look-back but not in the pending run (a ruby base, an earlier line, or
///   inside a prior construct): keep it `Referenced`, declined.
///
/// The window `[pending_plain_start, ［)` is pure plain source by construction
/// (every completed node resets `pending_plain_start` to a byte past itself,
/// every newline flushes it), so a window `rfind` selects exactly the §7.5
/// most-recent-preceding base-text occurrence when one is representable and
/// returns `Unresolvable` otherwise, so the ruby-base and cross-line declines
/// need no separate detector.
enum ForwardReferent {
    Adjacent(u32),
    Interior { start: u32, end: u32 },
    Unresolvable,
}

/// The directive-level diagnostic a forward recognizer returns for the
/// dispatch to attach. Independent of the emitted node.
enum ForwardDiag {
    /// No diagnostic.
    None,
    /// A detached styled target that occurs more than once in the look-back:
    /// the chosen run may be unintended (`bouten_target_ambiguous`).
    Ambiguous,
    /// The target is present but not stylable in place: a ruby base, an
    /// earlier line, a prior construct, or one of several targets
    /// (`forward_referent_not_stylable`).
    NotStylable,
}

impl ForwardDiag {
    /// Build the concrete directive-span diagnostic this signal identifies, if any.
    fn into_diagnostic(self, directive_span: Span) -> Option<Diagnostic> {
        match self {
            Self::None => None,
            Self::Ambiguous => Some(Diagnostic::bouten_target_ambiguous(directive_span)),
            Self::NotStylable => Some(Diagnostic::forward_referent_not_stylable(directive_span)),
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "Resolution inputs (events, target, range) are independent."
)]
fn resolve_forward_referent(
    events: &[PairEvent],
    source: &str,
    open_idx: usize,
    target: &str,
    pending_plain_start: Option<u32>,
) -> ForwardReferent {
    let Some(&PairEvent::PairOpen { span, .. }) = events.get(open_idx) else {
        return ForwardReferent::Unresolvable;
    };
    let cutoff = span.start as usize;
    let len = target.len();
    // A: byte-adjacent: the most-recent occurrence by definition. `target`
    // is canonical UTF-8, so a byte-slice compare is a string compare.
    if cutoff >= len && &source.as_bytes()[cutoff - len..cutoff] == target.as_bytes() {
        return u32::try_from(cutoff - len)
            .map_or(ForwardReferent::Unresolvable, ForwardReferent::Adjacent);
    }
    // B: most-recent occurrence within the current pending plain run.
    let Some(window_start) = pending_plain_start.map(|p| p as usize) else {
        return ForwardReferent::Unresolvable;
    };
    if window_start >= cutoff {
        return ForwardReferent::Unresolvable;
    }
    source[window_start..cutoff]
        .rfind(target)
        .map_or(ForwardReferent::Unresolvable, |rel| {
            let start = window_start + rel;
            match (u32::try_from(start), u32::try_from(start + len)) {
                (Ok(start), Ok(end)) => ForwardReferent::Interior { start, end },
                _ => ForwardReferent::Unresolvable,
            }
        })
}

/// Source-identified targets and the remaining formatting instruction.
/// Quoted targets exclude their quotation delimiters; an unquoted
/// parenthesized target retains its supplied parentheses.
struct ForwardTargetExtract<'s> {
    /// Inline capacity 4 covers the corpus 99th percentile: most
    /// forward-reference annotations have a single quoted target,
    /// the long tail rarely exceeds 2-3.
    targets: smallvec::SmallVec<[&'s str; 4]>,
    suffix: &'s str,
}

/// Shared helper for the `［＃「X」…<particle><keyword>］` shape.
///
/// Walks consecutive quote pairs immediately after the `＃` and
/// stops when the next event is not another `PairOpen(Quote)`.
/// Returns the collected target list together with the trimmed
/// suffix so callers can match on the particle + keyword portion.
///
/// Returns `None` if any shape assumption fails: no adjacent quote
/// pair, first quote empty, or the initial quote crossing out of the
/// bracket. Subsequent empty quote bodies are skipped (`「」`
/// placeholders occur in the corpora) rather than aborting the
/// recognition. `「」」` quotes the closing punctuation itself.
fn extract_forward_quote_targets<'s>(
    view: BodyView<'_>,
    source: &'s str,
    open_idx: usize,
    close_idx: usize,
) -> Option<ForwardTargetExtract<'s>> {
    let events = view.events;
    let &PairEvent::PairClose {
        span: bracket_close_span,
        ..
    } = events.get(close_idx)?
    else {
        return None;
    };

    if let Some(PairEvent::PairOpen {
        kind: PairKind::Quote,
        span,
    }) = events.get(open_idx + 2)
    {
        let quoted = &source[span.start as usize..bracket_close_span.start as usize];
        if let Some(suffix) = quoted.strip_prefix("「」」") {
            return Some(ForwardTargetExtract {
                targets: smallvec::smallvec![&quoted['「'.len_utf8().."「」".len()]],
                suffix: suffix.trim(),
            });
        }
    }

    let mut targets: smallvec::SmallVec<[&'s str; 4]> = smallvec::SmallVec::new();
    let mut cursor = open_idx + 2; // skip `［` and `＃`
    let mut last_quote_end: u32 = 0;

    while let Some(&PairEvent::PairOpen {
        kind: PairKind::Quote,
        span: quote_open_span,
    }) = events.get(cursor)
    {
        // Look up the quote's matching close via the side-table. An
        // unmatched/orphan PairOpen has `links[cursor] == u32::MAX`,
        // which is treated as "not nested inside this bracket", and
        // recognition declines.
        let quote_close_link = *view.links.get(cursor)?;
        if quote_close_link == u32::MAX {
            return None;
        }
        let quote_close_idx = quote_close_link as usize;
        // The quote must close before the bracket: a cross-boundary
        // close would mean the quote is not nested inside the bracket.
        if quote_close_idx >= close_idx {
            return None;
        }
        let Some(&PairEvent::PairClose {
            span: quote_close_span,
            ..
        }) = events.get(quote_close_idx)
        else {
            return None;
        };
        // Empty quotes are tolerated in-position but not added to the
        // target list; they carry no semantic content.
        let body = &source[quote_open_span.end as usize..quote_close_span.start as usize];
        if !body.is_empty() {
            targets.push(body);
        }
        last_quote_end = quote_close_span.end;
        cursor = quote_close_idx + 1;
    }

    if targets.is_empty() {
        return None;
    }
    let suffix = source[last_quote_end as usize..bracket_close_span.start as usize].trim();
    Some(ForwardTargetExtract { targets, suffix })
}

/// Classify a `［＃「target」は(大|中|小)見出し］` forward-reference
/// heading annotation.
///
/// Shares the event-stream extraction helper with `classify_forward_bouten`:
/// the quote-delimited target and the trailing keyword occur in the same
/// `［＃「X」…］` shape. The suffix uses `は`, or the corpus-attested `に`, and
/// the keyword selects the heading level: `大見出し` → 1, `中見出し` → 2,
/// `小見出し` → 3.
///
/// When the (single) referent is the bare line immediately above the
/// directive, the line is promoted in place to a block
/// `Heading` (大→`<h1>` / 中→`<h2>` / 小→`<h3>`): the
/// consume span is pulled back over that line so the heading element is
/// its sole rendered copy. When the referent is not a clean preceding
/// line, the classifier keeps the inline `HeadingHint` marker
/// at the directive position (information-preserving, never promoted to
/// an empty or misplaced heading).
///
/// Quoted operands identify source text. An unpromoted hint retains the
/// reference without inserting its quoted operand into principal text.
impl RecogniseCtx<'_, '_> {
    fn classify_forward_heading(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(Node, u32)> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        let rest = extracted
            .suffix
            .strip_prefix("は")
            .or_else(|| extracted.suffix.strip_prefix("に"))?;
        let (style, kind) = parse_heading_keyword(rest)?;

        let &PairEvent::PairOpen {
            span: open_span, ..
        } = view.events.get(open_idx)?
        else {
            return None;
        };

        // Whether `target` appears in the look-back. See `classify_forward_bouten`
        // for the same rationale. Exact look-back first (cheap; uses the AC index
        // when installed); on a miss, retry against a ruby-stripped copy of the
        // look-back, since a heading title carrying ruby (`両頭《りやうとう》`) has
        // its ruby-stripped target (`両頭`) quoted in the directive and so is not a
        // contiguous source substring.
        let preceded = |target: &str| {
            forward_target_is_preceded(view.events, self.source, open_idx, target)
                || forward_target_is_preceded_in_visible_text(view, self.source, open_idx, target)
        };

        if extracted.targets.len() > 1
            && extracted
                .targets
                .iter()
                .any(|target| !target.is_empty() && !preceded(target))
        {
            return None;
        }

        // Concatenate targets in the (rare) multi-quote case so the hint content
        // is the full quoted run. The 同行 / 窓 styles run into the body on their
        // own line, so they are also emitted here as hints.
        let combined: String = extracted.targets.iter().copied().collect();
        if combined.is_empty() {
            return None;
        }
        Some((
            self.alloc.heading_hint(kind, style, &combined),
            open_span.start,
        ))
    }
}

/// Classify a `「X」の左に「Y」のルビ` forward-reference left-side ruby: the
/// building block of a 再読文字 (saidoku-moji). The target `X` is pulled back
/// (mirroring `classify_forward_bouten`); the reading `Y` attaches on the left.
/// Single-target only; the `の左に「…」のルビ` suffix shape is unique, so a
/// non-ruby `の左に…` (left-side bouten) never reaches here.
impl RecogniseCtx<'_, '_> {
    fn classify_forward_left_ruby(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(Node, u32)> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        let [target] = extracted.targets.as_slice() else {
            return None;
        };
        // suffix == の左に「<reading>」のルビ
        let reading_text = extracted
            .suffix
            .strip_prefix("の左に「")?
            .strip_suffix("」のルビ")?;
        if reading_text.is_empty() {
            return None;
        }
        let &PairEvent::PairOpen {
            span: open_span, ..
        } = view.events.get(open_idx)?
        else {
            return None;
        };
        let consume_start =
            find_immediate_predecessor_target_position(view.events, self.source, open_idx, target)
                .unwrap_or(open_span.start);
        let base = self.alloc.content_plain(target);
        let reading = self.alloc.content_plain(reading_text);
        Some((self.alloc.left_ruby(base, reading), consume_start))
    }
}

fn page_reference_location<'a>(target: &str, suffix: &'a str) -> Option<&'a str> {
    let number = target
        .strip_suffix("頁")
        .or_else(|| target.strip_suffix("ページ"))?;
    if number.is_empty()
        || !number.chars().all(|c| {
            c.is_ascii_digit()
                || matches!(
                    c,
                    '０'
                        ..='９'
                            | '〇'
                            | '一'
                            | '二'
                            | '三'
                            | '四'
                            | '五'
                            | '六'
                            | '七'
                            | '八'
                            | '九'
                            | '十'
                            | '百'
                            | '千'
                )
        })
    {
        return None;
    }
    let location = suffix.strip_prefix('は')?;
    let mut remaining = location;
    loop {
        let quoted = remaining.strip_prefix('「')?;
        let (title, rest) = quoted.split_once('」')?;
        if title.is_empty() || title.contains('「') {
            return None;
        }
        if rest.is_empty() {
            return Some(location);
        }
        remaining = rest.strip_prefix('の')?;
    }
}

fn note_role_suffix(suffix: &str) -> Option<(MarginNoteKind, &str)> {
    let word = suffix.strip_prefix('は')?;
    match word {
        "注釈番号" => Some((MarginNoteKind::AnnotationNumber, word)),
        "自注" => Some((MarginNoteKind::AuthorNote, word)),
        _ => None,
    }
}

fn edition_verified_gloss(suffix: &str) -> Option<&str> {
    let statement = suffix.strip_prefix('に')?;
    let (gloss, provenance) = statement.strip_prefix("校注、「")?.split_once('」')?;
    (!gloss.is_empty()
        && !gloss.contains(['「', '\n', '\r'])
        && provenance == "、ただし底本では校注が脱落、底本の親本にて確認")
        .then_some(statement)
}

fn parenthesized_note_role_target<'s>(
    view: BodyView<'_>,
    source: &'s str,
    open_idx: usize,
    close_idx: usize,
) -> Option<ForwardTargetExtract<'s>> {
    let PairEvent::PairOpen { span: open, .. } = view.events.get(open_idx)? else {
        return None;
    };
    let PairEvent::PairClose { span: close, .. } = view.events.get(close_idx)? else {
        return None;
    };
    let body = source
        .get(open.start as usize..close.end as usize)?
        .strip_prefix("［＃")?
        .strip_suffix('］')?;
    let (inside, _) = body.strip_prefix('（')?.split_once('）')?;
    if inside.is_empty() || inside.contains(['（', '［', '］', '\n']) {
        return None;
    }
    let target_end = '（'.len_utf8() + inside.len() + '）'.len_utf8();
    let suffix = &body[target_end..];
    note_role_suffix(suffix)?;
    Some(ForwardTargetExtract {
        targets: smallvec::smallvec![&body[..target_end]],
        suffix,
    })
}

/// The unquoted subject of a glyph description, as `焔［＃焔の火へんを…］`,
/// which must repeat the character immediately before the note.
fn unquoted_glyph_description_target<'s>(
    view: BodyView<'_>,
    source: &'s str,
    open_idx: usize,
    close_idx: usize,
) -> Option<ForwardTargetExtract<'s>> {
    let PairEvent::PairOpen { span: open, .. } = view.events.get(open_idx)? else {
        return None;
    };
    let PairEvent::PairClose { span: close, .. } = view.events.get(close_idx)? else {
        return None;
    };
    let body = source
        .get(open.start as usize..close.end as usize)?
        .strip_prefix("［＃")?
        .strip_suffix('］')?;
    let before = source.get(..open.start as usize)?;
    if let Some(statement) = component_statement(body) {
        // `癇［＃やまいだれの中は「間」］` describes a component of the character before it.
        let written = before.chars().next_back().filter(|ch| is_ideograph(*ch))?;
        let start = open.start as usize - written.len_utf8();
        return Some(ForwardTargetExtract {
            targets: smallvec::smallvec![&source[start..open.start as usize]],
            suffix: statement,
        });
    }
    let subject = body.chars().next()?;
    let target = &body[..subject.len_utf8()];
    let suffix = &body[target.len()..];
    (before.ends_with(subject) && glyph_shape_statement(target, suffix).is_some()).then(|| {
        ForwardTargetExtract {
            targets: smallvec::smallvec![target],
            suffix,
        }
    })
}

/// A subjectless statement about one component of a character, as
/// `やまいだれの中は「間」` or `右下の部分は「蝎」の右下部と同形`.
fn component_statement(body: &str) -> Option<&str> {
    let (component, rest) = body
        .split_once("の中は「")
        .or_else(|| body.split_once("の部分は「"))?;
    let (glyph, tail) = rest.split_once('」')?;
    let quoted_char = glyph.chars().count() == 1 && is_ideograph(glyph.chars().next()?);
    let tail_ok = tail.is_empty()
        || tail
            .strip_prefix('の')
            .and_then(|part| part.strip_suffix("部と同形"))
            .is_some_and(|part| !part.is_empty() && !part.contains(['「', '」', '、', '。']));
    (!component.is_empty()
        && !component.contains(['「', '」', '、', '。', '［', '］'])
        && quoted_char
        && tail_ok)
        .then_some(body)
}

fn is_ideograph(ch: char) -> bool {
    matches!(ch, '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '\u{20000}'..='\u{3FFFF}')
}

fn parenthesized_note_payload(suffix: &str) -> Option<&str> {
    let note = suffix.strip_prefix('に')?.strip_suffix("の注記")?;
    let inside = note.strip_prefix('（')?.strip_suffix('）')?;
    (!inside.is_empty() && !inside.contains(['（', '）', '\n', '\r'])).then_some(note)
}

fn author_note_payload(suffix: &str) -> Option<&str> {
    let note = suffix.strip_prefix("に「")?.strip_suffix("」の著者註")?;
    (!note.is_empty() && !note.contains(['「', '」', '\n', '\r'])).then_some(note)
}

fn unquoted_mama_note_payload(suffix: &str) -> Option<&str> {
    let note = suffix.strip_prefix('に')?;
    let note = note
        .strip_suffix("の注記")
        .or_else(|| note.strip_suffix("と傍書"))?;
    matches!(note, "ママ" | "〔ママ〕").then_some(note)
}

fn marginal_critical_note_payload(suffix: &str) -> Option<&str> {
    let note = suffix
        .strip_prefix("に欄外に校注、「")?
        .strip_suffix('」')?;
    (!note.is_empty() && !note.contains(['「', '」', '\n', '\r'])).then_some(note)
}

fn supplied_right_mark(suffix: &str) -> Option<&str> {
    let mark = suffix.strip_prefix("の右に「")?.strip_suffix('」')?;
    (mark == "）").then_some(mark)
}

/// The small kana before the note when the note quotes its full-size form,
/// as `ソヴェ［＃「エ」は小さい「ヱ」］` quotes エ for the small ェ.
fn small_kana_written_for<'s>(
    source: &'s str,
    open_idx: usize,
    view: BodyView<'_>,
    target: &str,
) -> Option<&'s str> {
    const PAIRS: &[(char, char)] = &[
        ('ア', 'ァ'),
        ('イ', 'ィ'),
        ('ウ', 'ゥ'),
        ('エ', 'ェ'),
        ('オ', 'ォ'),
        ('ツ', 'ッ'),
        ('ヤ', 'ャ'),
        ('ユ', 'ュ'),
        ('ヨ', 'ョ'),
        ('ワ', 'ヮ'),
    ];
    let &PairEvent::PairOpen { span, .. } = view.events.get(open_idx)? else {
        return None;
    };
    let before = source.get(..span.start as usize)?;
    let written = before.chars().next_back()?;
    let mut quoted = target.chars();
    let full = quoted.next().filter(|_| quoted.next().is_none())?;
    PAIRS
        .contains(&(full, written))
        .then(|| &before[before.len() - written.len_utf8()..])
}

/// The wording of a note that describes the glyph of its quoted target
/// without supplying a replacement character.
fn glyph_shape_statement<'a>(target: &str, suffix: &'a str) -> Option<&'a str> {
    let one_character = target.chars().count() == 1;
    if one_character && let Some(statement) = component_statement(suffix) {
        return Some(statement);
    }
    if let Some(statement) = suffix.strip_prefix('の') {
        return ((one_character && statement == "右上に小さな四角あり")
            || (target == "焔"
                && statement
                    == "火へんを炎にしたうえで、へんとつくりをいれかえた字、焔の正字と同字"))
            .then_some(statement);
    }
    let statement = suffix.strip_prefix('は')?;
    let quoted = |prefix: &str, suffix: &str| {
        statement
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix(suffix))
            .filter(|glyph| glyph.chars().count() == 1)
    };
    let quoted_glyph = |prefix: &str, suffix: &str| quoted(prefix, suffix).is_some();
    // `´ ｀ ¨ ^` supply a letter diacritic and belong to the supplied-mark grammar.
    let attached_mark = quoted("「", "」付き").is_some_and(|mark| !"´｀¨^".contains(mark));
    let number = |text: &str| {
        !text.is_empty()
            && text
                .chars()
                .all(|ch| ch.is_ascii_digit() || ('０'..='９').contains(&ch))
    };
    let roman = !target.is_empty() && target.chars().all(|ch| "IVXLCDMivxlcdm".contains(ch));
    ((one_character && statement == "上に「⌒」付き")
        || (target == "♂" && statement == "矢印が下向き")
        || (one_character && attached_mark)
        || (one_character && quoted_glyph("小さい「", "」"))
        || (one_character && quoted_glyph("「", "」のくずし字"))
        || statement == "崩し字"
        || (one_character && statement == "「√」の記号の中に入っている")
        || (target.chars().all(|ch| ch == '□')
            && statement.strip_suffix("倍の長方形").is_some_and(number))
        || (roman
            && (statement
                .strip_suffix("を表すローマ数字の小文字")
                .is_some_and(number)
                || statement.strip_prefix("ローマ数字の").is_some_and(number))))
    .then_some(statement)
}

fn side_note_payload<'a>(
    target: &str,
    suffix: &'a str,
) -> Option<(MarginNoteKind, Option<MarginNotePosition>, &'a str)> {
    Some(if let Some((kind, word)) = note_role_suffix(suffix) {
        (kind, None, word)
    } else if let Some(note) = author_note_payload(suffix) {
        (MarginNoteKind::AuthorNote, None, note)
    } else if let Some(note) = unquoted_mama_note_payload(suffix) {
        (MarginNoteKind::Gloss, None, note)
    } else if let Some(note) = marginal_critical_note_payload(suffix) {
        (
            MarginNoteKind::CriticalNote,
            Some(MarginNotePosition::Margin),
            note,
        )
    } else if let Some(statement) = edition_verified_gloss(suffix) {
        (MarginNoteKind::Gloss, None, statement)
    } else if let Some(note) = parenthesized_note_payload(suffix) {
        (MarginNoteKind::Gloss, None, note)
    } else if let Some(mark) = supplied_right_mark(suffix) {
        (
            MarginNoteKind::SuppliedMark,
            Some(MarginNotePosition::Right),
            mark,
        )
    } else if let Some(statement) = glyph_shape_statement(target, suffix) {
        (MarginNoteKind::GlyphShape, None, statement)
    } else if let Some(location) = page_reference_location(target, suffix) {
        (MarginNoteKind::CrossReference, None, location)
    } else {
        let (kind, inner) = if let Some(inner) = [
            "」の注記",
            "」注記",
            "」と注記",
            "」の注記がある",
            "」の注",
            "」の表記",
        ]
        .into_iter()
        .find_map(|keyword| suffix.strip_suffix(keyword))
        {
            (MarginNoteKind::Gloss, inner)
        } else {
            (MarginNoteKind::Marginal, suffix.strip_suffix("」の傍記")?)
        };
        let (position, note_text) = if let Some(note) = inner.strip_prefix("の左に「") {
            (Some(MarginNotePosition::Left), note)
        } else if let Some(note) = inner.strip_prefix("の右に「") {
            (Some(MarginNotePosition::Right), note)
        } else {
            (None, inner.strip_prefix("に「")?)
        };
        (kind, position, note_text)
    })
}

/// Classify target-associated notes and source roles while preserving supplied
/// placement. Bare `に` does not establish a side. Native referent extents let
/// downstream projections attach interior notes without repeating the target.
impl RecogniseCtx<'_, '_> {
    fn classify_forward_side_note(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(Node, u32)> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)
            .or_else(|| parenthesized_note_role_target(view, self.source, open_idx, close_idx))
            .or_else(|| {
                unquoted_glyph_description_target(view, self.source, open_idx, close_idx)
            })?;
        let [quoted] = extracted.targets.as_slice() else {
            return None;
        };
        let radicand = (*quoted == "√")
            .then(|| {
                extracted
                    .suffix
                    .strip_prefix("の中に「")?
                    .strip_suffix('」')
            })
            .flatten()
            .filter(|radicand| !radicand.contains(['「', '」']));
        let combined;
        let (target, (kind, position, note_text)) = if let Some(radicand) = radicand {
            // `「√」の中に「５」` describes the sign and the radicand after it.
            combined = format!("{quoted}{radicand}");
            let statement = extracted.suffix.strip_prefix('の')?;
            (
                combined.as_str(),
                (MarginNoteKind::GlyphShape, None, statement),
            )
        } else {
            (*quoted, side_note_payload(quoted, extracted.suffix)?)
        };
        if note_text.is_empty() {
            return None;
        }
        let target = if kind == MarginNoteKind::GlyphShape && note_text.starts_with("小さい「")
        {
            small_kana_written_for(self.source, open_idx, view, target).unwrap_or(target)
        } else {
            target
        };
        if !forward_target_is_preceded(view.events, self.source, open_idx, target)
            && !forward_target_is_preceded_in_visible_text(view, self.source, open_idx, target)
        {
            return None;
        }
        let &PairEvent::PairOpen {
            span: open_span, ..
        } = view.events.get(open_idx)?
        else {
            return None;
        };
        let (consume_start, target_span, origin) = match resolve_forward_referent(
            view.events,
            self.source,
            open_idx,
            target,
            self.pending_plain_start,
        ) {
            ForwardReferent::Adjacent(start) => (
                start,
                Some(Span::new(start, open_span.start)),
                ForwardOrigin::Reclaimed,
            ),
            ForwardReferent::Interior { start, end } => (
                open_span.start,
                Some(Span::new(start, end)),
                ForwardOrigin::Referenced,
            ),
            ForwardReferent::Unresolvable => (open_span.start, None, ForwardOrigin::Referenced),
        };
        // A glyph-shape note on a word resolves like other quoted targets; on
        // one character it must follow that character.
        let single_glyph = kind == MarginNoteKind::GlyphShape && target.chars().nth(1).is_none();
        if (matches!(
            kind,
            MarginNoteKind::AnnotationNumber
                | MarginNoteKind::AuthorNote
                | MarginNoteKind::SuppliedMark
        ) || single_glyph
            || parenthesized_note_payload(extracted.suffix).is_some())
            && origin != ForwardOrigin::Reclaimed
            && author_note_payload(extracted.suffix).is_none()
        {
            return None;
        }
        let base = self.alloc.content_plain(target);
        let note = self.alloc.content_plain(note_text);
        let note_start = note_text.as_ptr().addr() - self.source.as_ptr().addr();
        let note_span = Span::new(
            u32::try_from(note_start).ok()?,
            u32::try_from(note_start + note_text.len()).ok()?,
        );
        let mut result = self
            .alloc
            .side_note(kind, position, base, note, Some(note_span));
        if let Node::MarginNote(note) = &mut result {
            note.target_span = target_span
                .map(|span| NonEmptySpan::new(span).expect("resolved target is nonempty"));
            note.origin = origin;
        }
        Some((result, consume_start))
    }

    /// Preserve the quoted caption reference as image metadata. The visible
    /// caption is transcribed separately in the body by the source convention.
    fn classify_caption_figure(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<Node> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        let [caption] = extracted.targets.as_slice() else {
            return None;
        };
        if caption.is_empty() {
            return None;
        }
        // suffix == のキャプション付きの(図|挿絵|写真)（file）入る
        let rest = extracted.suffix.strip_prefix("のキャプション付きの")?;
        let rest = rest
            .strip_prefix("図")
            .or_else(|| rest.strip_prefix("挿絵"))
            .or_else(|| rest.strip_prefix("写真"))?;
        let rest = rest.strip_prefix('（')?;
        let close_off = rest.find('）')?;
        let spec = &rest[..close_off];
        let (file, dimensions) = super::directive::illustration_file_spec(spec)?;
        if &rest[close_off + '）'.len_utf8()..] != "入る" {
            return None;
        }
        let caption_content = self.alloc.content_plain(caption);
        let result = self
            .alloc
            .sashie(file, None, dimensions, Some(caption_content));
        if let Node::Illustration(id) = result {
            let image = self.alloc.illustration_mut(id);
            let start = caption.as_ptr().addr() - self.source.as_ptr().addr();
            image.caption_span = Some(Span::new(
                u32::try_from(start).expect("source offset fits u32"),
                u32::try_from(start + caption.len()).expect("source offset fits u32"),
            ));
        }
        Some(result)
    }
}

impl RecogniseCtx<'_, '_> {
    /// Shared resolution for the single-target `forward_format` families
    /// (emphasis / 縦中横 / box enclosure). The caller has already confirmed the
    /// target is preceded (not self-contained) and holds the `attr` + open
    /// span. Returns the bracket node, its consume start, and the diagnostic;
    /// for the interior case it also stores the styled `Detached` decoration
    /// in `self.pending_decorations` for `try_bracket_emit` to splice.
    #[allow(
        clippy::too_many_arguments,
        reason = "the caller already holds the body view, bracket index, open-span start, \
                  resolved attribute, and target text: each an independent input."
    )]
    fn resolve_forward_format(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        open_span_start: u32,
        attr: ForwardAttr,
        only: &str,
    ) -> (Node, u32, ForwardDiag) {
        self.resolve_forward_formats(view, open_idx, open_span_start, &[attr], only)
    }

    fn resolve_forward_formats(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        open_span_start: u32,
        attrs: &[ForwardAttr],
        only: &str,
    ) -> (Node, u32, ForwardDiag) {
        let text = self.alloc.content_plain(only);
        match resolve_forward_referent(
            view.events,
            self.source,
            open_idx,
            only,
            self.pending_plain_start,
        ) {
            ForwardReferent::Adjacent(consume_start) => (
                self.alloc
                    .forward_formats(attrs, text, ForwardOrigin::Reclaimed),
                consume_start,
                ForwardDiag::None,
            ),
            ForwardReferent::Interior { start, end } => {
                let deco = self
                    .alloc
                    .forward_formats(attrs, text, ForwardOrigin::Detached);
                self.pending_decorations.push((deco, Span::new(start, end)));
                let Node::Format(mut reference) = deco else {
                    unreachable!("forward format allocation produces a format node");
                };
                reference.origin = ForwardOrigin::Referenced;
                (Node::Format(reference), open_span_start, ForwardDiag::None)
            }
            ForwardReferent::Unresolvable => (
                self.alloc
                    .forward_formats(attrs, text, ForwardOrigin::Referenced),
                open_span_start,
                ForwardDiag::NotStylable,
            ),
        }
    }

    fn classify_selected_style(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(Node, u32, ForwardDiag)> {
        let selection = extract_bouten_selection(view, self.source, open_idx, close_idx)?;
        let attr = match selection.suffix {
            "の部分はイタリック体" | "の部分は斜体" => ForwardAttr::Italic,
            "はゴシック体" => ForwardAttr::Gothic,
            _ => return None,
        };
        let PairEvent::PairOpen { span: open, .. } = view.events[open_idx] else {
            return None;
        };
        let PairEvent::PairClose { span: close, .. } = view.events[close_idx] else {
            return None;
        };
        let spans = self.pending_plain_start.and_then(|start| {
            selection.resolve(
                self.source,
                Span::new(start, open.start),
                SelectionSense::Named,
            )
        });
        if let Some(spans) = spans {
            return Some(self.decorate_selected_ranges(open, close, attr, &spans));
        }
        let raw = &self.source[open.start as usize..close.end as usize];
        let payload = self.alloc.make_directive(raw, DirectiveKind::Unknown);
        Some((
            self.alloc.annotation(payload),
            open.start,
            ForwardDiag::NotStylable,
        ))
    }

    fn classify_forward_emphasis(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(Node, u32, ForwardDiag)> {
        if let Some(selected) = self.classify_selected_style(view, open_idx, close_idx) {
            return Some(selected);
        }
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        // The particle is tied to the decoration. `は` is the dominant emphasis
        // form (`「X」は太字`). The frame decoration also takes the "applied to"
        // particle `に` (`「X」に枠囲み`), but 太字/斜体/… stay `は`-only, so a `に`
        // suffix is accepted only when it resolves to `Framed`. Bouten runs
        // earlier in the cascade and already claims `に〈bouten-kind〉`. The
        // serializer canonicalises both particles to `」は`. `の部分は` and a
        // particle-free `ゴシック体` state the same style of the quoted text.
        let attr = if let Some(rest) = extracted.suffix.strip_prefix("は") {
            forward_attr_from_suffix(rest)?
        } else if extracted.suffix == "の小文字" {
            ForwardAttr::FontSizeAbsolute(AbsoluteSize::Small)
        } else if matches!(extracted.suffix, "の部分はイタリック体" | "の部分は斜体")
        {
            ForwardAttr::Italic
        } else if extracted.suffix == "ゴシック体" {
            ForwardAttr::Gothic
        } else {
            let rest = extracted.suffix.strip_prefix("に")?;
            match forward_attr_from_suffix(rest)? {
                framed @ ForwardAttr::Framed(_) => framed,
                _ => return None,
            }
        };
        let [only] = extracted.targets.as_slice() else {
            return None;
        };
        // Forward accent (`「e」はアクサン（´）付き` → é): the quoted target must be a
        // single Latin letter composable with this mark. Word-qualified /
        // positional / multi-letter forms would misrender, so decline them to a
        // byte-exact `Directive{Unknown}` here (mirrors how
        // `classify_forward_accent_dot` validates via `compose_accent_dots`).
        if let ForwardAttr::Accent(mark) = attr {
            let mut cs = only.chars();
            match (cs.next(), cs.next()) {
                (Some(letter), None) if compose_accent(letter, mark).is_some() => {}
                _ => return None,
            }
        }
        let &PairEvent::PairOpen {
            span: open_span, ..
        } = view.events.get(open_idx)?
        else {
            return None;
        };
        Some(self.resolve_forward_format(view, open_idx, open_span.start, attr, only))
    }
}

/// Classify a `［＃「X」は「□」囲み］` box-character enclosure: the 「□」 box
/// member of the 罫囲み enclosure family ([`EnclosureKind::Box`], §6.7).
///
/// A single target `X`; the `は` particle and the quoted `□` glyph live in the
/// suffix, mirroring [`Self::classify_forward_left_ruby`] /
/// [`Self::classify_forward_side_note`]. Structurally it is an emphasis-style
/// treatment (it boxes the target run, like `「X」は太字` bolds it), so it reuses
/// the same source-target resolution as
/// [`Self::classify_forward_emphasis`]. The `は「` prefix + `」囲み` suffix shape
/// excludes every `の注記` / `のルビ` / `に…` form; only the canonical `□`
/// (U+25A1) glyph is claimed; any other glyph stays `Directive{Unknown}` until
/// a corresponding [`EnclosureKind`] member is added.
impl RecogniseCtx<'_, '_> {
    fn classify_forward_box_enclosure(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(Node, u32, ForwardDiag)> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        let [target] = extracted.targets.as_slice() else {
            return None;
        };
        let glyph = extracted
            .suffix
            .strip_prefix("は「")?
            .strip_suffix("」囲み")?;
        if glyph != "□" {
            return None;
        }
        let attr = ForwardAttr::Framed(EnclosureKind::Box);
        let &PairEvent::PairOpen {
            span: open_span, ..
        } = view.events.get(open_idx)?
        else {
            return None;
        };
        Some(self.resolve_forward_format(view, open_idx, open_span.start, attr, target))
    }
}

/// Classify a `<run>［＃mは上ドット付き］` dotted-letter directive.
///
/// The directive addresses a base Latin letter inside the immediately
/// preceding run (a bare word `Padma-sambhava` or a decomposed `〔…〕`
/// accent span) and states a combining dot above / below it. This is
/// sub-run occurrence addressing, not the `「X」は…` quote shape, so it reclaims
/// the preceding run directly (never `extract_forward_quote_targets`). The run
/// is pulled back (the styled span is the sole rendered copy, without duplicate rendering); the
/// raw body is interned for byte-exact serialize and render-time composition.
/// `ab_aozora_syntax::accent::compose_accent_dots` is the single authority for the
/// selector grammar and glyph table, shared with the renderer; a `Some`
/// result both validates the claim and (in the renderer) produces the glyphs.
impl RecogniseCtx<'_, '_> {
    fn classify_forward_accent_dot(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        body: &str,
    ) -> Option<(Node, u32)> {
        let &PairEvent::PairOpen {
            span: open_span, ..
        } = view.events.get(open_idx)?
        else {
            return None;
        };
        let bracket_start = open_span.start as usize;
        let prefix = &self.source[..bracket_start];
        let (run_start, body) = if body.ends_with("ドット付き") {
            (reclaim_accent_run_start(prefix)?, body)
        } else {
            let start = supplied_mark_target_start(prefix, body)?;
            if self
                .pending_plain_start
                .is_none_or(|plain| plain as usize > start)
            {
                return None;
            }
            // The serializer re-emits the stored body between `［＃` and `］`.
            let raw = &self.source[bracket_start..]
                .strip_prefix("［＃")?
                .split_once('］')?
                .0;
            (start, *raw)
        };
        let run = &self.source[run_start..bracket_start];
        // The shared composer declines a 段目 form and any unresolvable or
        // uncomposable letter, leaving those as `Directive{Unknown}`.
        compose_accent_dots(run, body)?;
        // Store the reclaimed run uncomposed: the renderer composes it at
        // render time and the serializer re-emits it verbatim before the raw body.
        let text = self.alloc.content_plain(run);
        let consume_start = u32::try_from(run_start).ok()?;
        let origin = ForwardOrigin::from_consume(consume_start, open_span.start);
        Some((self.alloc.accent_dot(text, body, origin), consume_start))
    }
}

/// Byte offset where the run immediately preceding a dotted-letter directive
/// begins, or `None` when a non-Latin character immediately precedes the bracket
/// (a `》` ruby close, Japanese text), which declines the directive.
///
/// A prefix ending in `〕` reclaims the whole decomposed `〔…〕` accent span
/// (sanitize keeps its brackets, so it is a contiguous run); otherwise the
/// maximal trailing run of Latin letters / Latin-Extended glyphs / `-` word
/// joiners is reclaimed.
fn reclaim_accent_run_start(prefix: &str) -> Option<usize> {
    if prefix.ends_with('〕') {
        // `〔…〕` does not nest, so the matching open is the last `〔`.
        return prefix.rfind('〔');
    }
    let mut start = prefix.len();
    for (i, ch) in prefix.char_indices().rev() {
        if is_latin_run_char(ch) {
            start = i;
        } else {
            break;
        }
    }
    (start < prefix.len()).then_some(start)
}

/// A character that participates in a reclaimable Latin run: ASCII letters,
/// the Latin-1 / Latin-Extended-A ranges (accented vowels like ā, ç produced
/// by `〔…〕` decomposition), the Latin-Extended-Additional range (dotted
/// glyphs like ṁ), and the `-` word joiner (`Nara-sinha`).
fn is_latin_run_char(ch: char) -> bool {
    ch.is_ascii_alphabetic()
        || ch == '-'
        || matches!(ch, '\u{00C0}'..='\u{024F}' | '\u{1E00}'..='\u{1EFF}')
}

/// Map the keyword after `は` to a forward-scope [`ForwardAttr`].
///
/// 太字 → Bold, ゴシック体 → Gothic (a distinct typeface, not a fold to
/// 太字), 斜体 → Italic (per
/// <https://www.aozora.gr.jp/annotation/emphasis.html>). ゴチック shares the
/// gothic typeface meaning. 上付き小文字 →
/// `SuperScript`, 下付き小文字 → `SubScript`, 行右小書き → `SmallScript(Right)`,
/// 行左小書き → `SmallScript(Left)`, and `N段階大きな/小さな文字` → `FontSize`
/// (per <https://www.aozora.gr.jp/annotation/etc.html>). 分数 → `Fraction`
/// (`「a/b」は分数`, the render arm typesets the `/`-split target). 罫囲み /
/// ○付き文字 / 点線丸囲み / 二重罫囲み → the corresponding [`EnclosureKind`]
/// under `Framed` (`「□」囲み` is claimed earlier, by the box recogniser). 枠囲み,
/// 枠囲い and 枠線 supply a frame without its rule style.
/// Unknown suffixes return `None` (→ `Directive{Unknown}`).
pub(super) fn forward_attr_from_suffix(s: &str) -> Option<ForwardAttr> {
    Some(match s {
        "太字" => ForwardAttr::Bold,
        "ゴシック体" | "ゴチック" => ForwardAttr::Gothic,
        "手書き文字" => ForwardAttr::Handwritten,
        "斜体" | "イタリック体" => ForwardAttr::Italic,
        "上付き小文字" => ForwardAttr::SuperScript,
        "下付き小文字" => ForwardAttr::SubScript,
        "下付き" => ForwardAttr::Lowered,
        "上部に出ている" => ForwardAttr::Raised { degrees: None },
        "指数" => ForwardAttr::Exponent,
        "行右小書き" | "小書き右寄せ" => ForwardAttr::SmallScript(BoutenPosition::Right),
        "行左小書き" => ForwardAttr::SmallScript(BoutenPosition::Left),
        "罫囲み" | "罫で囲む" => ForwardAttr::Framed(EnclosureKind::Rule),
        "枠囲み" | "枠囲い" | "枠線" => ForwardAttr::Framed(EnclosureKind::Unspecified),
        "破線枠囲み" => ForwardAttr::Framed(EnclosureKind::DashedRule),
        // Other single-target enclosures whose suffix carries no embedded glyph
        // (unlike 「□」囲み). Each has its own EnclosureKind so serialize
        // round-trips the exact keyword rather than folding onto 罫囲み.
        "○付き文字" => ForwardAttr::Framed(EnclosureKind::Circle),
        "点線丸囲み" => ForwardAttr::Framed(EnclosureKind::CircleDotted),
        "二重罫囲み" => ForwardAttr::Framed(EnclosureKind::DoubleRule),
        "横組み" | "横書き" => ForwardAttr::Horizontal,
        "キャプション" => ForwardAttr::Caption,
        // 絶対サイズ: `「X」は小文字` (corpus-attested) and its 特大/大/中 siblings.
        // Distinct from the relative `N段階…文字` (parse_font_size_suffix) and
        // from the script-glyph `上付き小文字`/`下付き小文字` (exact-match above).
        "特大文字" => ForwardAttr::FontSizeAbsolute(AbsoluteSize::ExtraLarge),
        "大文字" => ForwardAttr::FontSizeAbsolute(AbsoluteSize::Large),
        "中文字" => ForwardAttr::FontSizeAbsolute(AbsoluteSize::Medium),
        "小文字" | "小書き" => ForwardAttr::FontSizeAbsolute(AbsoluteSize::Small),
        "小さい文字" | "本文より小さいサイズの文字" | "ポイントを下げる" => {
            ForwardAttr::QualitativeFontSize(QualitativeFontSize::Smaller)
        }
        "小さめの文字" => {
            ForwardAttr::QualitativeFontSize(QualitativeFontSize::SlightlySmaller)
        }
        // 分数: `「a/b」は分数`. Only the single-target form is matched here; a
        // comma-joined compound (`「3」は上付き小文字、「1/143」は分数`) yields a
        // suffix that is not exactly `分数`, so it stays `Directive{Unknown}`.
        "分数" => ForwardAttr::Fraction,
        _ => {
            if let Some((degrees, tail)) = parse_decimal_u8_prefix(s)
                && tail == "度位右上がり"
            {
                return Some(ForwardAttr::Raised {
                    degrees: NonZeroU8::new(degrees),
                });
            }
            return parse_font_size_suffix(s)
                .or_else(|| parse_align_end_suffix(s))
                .or_else(|| parse_accent_suffix(s));
        }
    })
}

/// Parse a forward accent-mark suffix (`アクサン（´）付き` / `アクサン（｀）付き` /
/// `ウムラウト（¨）付き`) into a [`ForwardAttr::Accent`].
///
/// The mark word does not distinguish acute from grave: the bracketed symbol
/// does (´ U+00B4 vs ｀ U+FF40); ウムラウト（¨） (¨ U+00A8) is the umlaut. Only the
/// canonical fullwidth-paren spelling (U+FF08 / U+FF09) is claimed; the
/// classifier separately requires the quoted target to be a single composable
/// letter, so word-qualified / positional / half-width-paren forms decline.
/// Any other suffix returns `None` (→ `Directive{Unknown}`).
fn parse_accent_suffix(s: &str) -> Option<ForwardAttr> {
    let mark = match s {
        "アクサン（´）付き" => AccentMark::Acute,
        "アクサン（｀）付き" => AccentMark::Grave,
        "ウムラウト（¨）付き" => AccentMark::Umlaut,
        _ => return None,
    };
    Some(ForwardAttr::Accent(mark))
}

/// Parse a `地付き` / `文末より N字上げ揃え` / `行末より N字上がり` end-alignment
/// suffix into a [`ForwardAttr::AlignEnd`]: the forward-scope analogue of the
/// line-form `AlignEndParamPrefix` (§7.6). Mirrors its verb set (`字上げ` /
/// `字上がり`, optional `揃え`) and, like the line-form `LineFormat::AlignEnd`,
/// collapses the 文末 / 行末 / 地より / 地から anchor to the offset alone. `地付き`
/// is the zero-lift form (`offset: 0`, flush to the text-end edge). Returns
/// `None` for a missing/zero magnitude or any other suffix (→
/// `Directive{Unknown}`).
fn parse_align_end_suffix(s: &str) -> Option<ForwardAttr> {
    // 地付き: flush to the text-end edge (zero lift); the forward analogue of
    // the `地付き` line form (LineFormat::AlignEnd { offset: 0 }). A distinct
    // lexeme with no magnitude, so it does not take the prefix+digit path below.
    if matches!(s, "地付き" | "地付け") {
        return Some(ForwardAttr::AlignEnd { offset: 0 });
    }
    if let Some(spacing) = s.strip_prefix("地付き、地より") {
        return parse_end_spacing(spacing).map(|offset| ForwardAttr::AlignEnd { offset });
    }
    let rest = s
        .strip_prefix("文末より")
        .or_else(|| s.strip_prefix("行末より"))
        .or_else(|| s.strip_prefix("地より"))
        .or_else(|| s.strip_prefix("地から"))?;
    let (offset, tail) = parse_decimal_u8_prefix(rest)?;
    (matches!(tail, "字上げ" | "字上がり" | "字上げ揃え" | "字上がり揃え") && offset >= 1)
        .then_some(ForwardAttr::AlignEnd { offset })
}

/// Parse a `N段階大きな文字` / `N段階小さな文字` font-size suffix into a
/// [`ForwardAttr::FontSize`]. `大きな` yields a positive stage count, `小さな`
/// a negative one. Returns `None` for a missing/zero magnitude, an `i8`
/// overflow, or any other suffix (→ `Directive{Unknown}`).
fn parse_font_size_suffix(s: &str) -> Option<ForwardAttr> {
    let (magnitude, rest) = s
        .strip_prefix('一')
        .map(|rest| (1, rest))
        .or_else(|| parse_decimal_u8_prefix(s))?;
    let steps = i8::try_from(magnitude).ok()?;
    let shift = FontShift(NonZeroI8::new(steps)?);
    match rest {
        "段階大きな文字" => Some(ForwardAttr::FontSize(shift)),
        "段階小さな文字" => Some(ForwardAttr::FontSize(FontShift(NonZeroI8::new(-steps)?))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{AccentMark, EnclosureKind, ForwardAttr, forward_attr_from_suffix};

    #[test]
    fn accent_suffixes_map_to_their_marks() {
        // The bracketed symbol distinguishes acute (´) from grave (｀); ウムラウト
        // (¨) is the umlaut. Fullwidth parens (U+FF08 / U+FF09) are canonical.
        assert_eq!(
            forward_attr_from_suffix("アクサン（´）付き"),
            Some(ForwardAttr::Accent(AccentMark::Acute))
        );
        assert_eq!(
            forward_attr_from_suffix("アクサン（｀）付き"),
            Some(ForwardAttr::Accent(AccentMark::Grave))
        );
        assert_eq!(
            forward_attr_from_suffix("ウムラウト（¨）付き"),
            Some(ForwardAttr::Accent(AccentMark::Umlaut))
        );
        // Half-width parens are not the canonical spelling; declined here (the
        // corpus half-width forms are all word-qualified anyway).
        assert_eq!(forward_attr_from_suffix("アクサン(´)付き"), None);
        // A bare mark word with no bracketed symbol is not a claimable suffix.
        assert_eq!(forward_attr_from_suffix("アクサン付き"), None);
    }

    #[test]
    fn enclosure_suffixes_map_to_their_kinds() {
        // Each glyph-free enclosure suffix picks its own EnclosureKind so
        // serialize can round-trip the exact keyword.
        assert_eq!(
            forward_attr_from_suffix("○付き文字"),
            Some(ForwardAttr::Framed(EnclosureKind::Circle))
        );
        assert_eq!(
            forward_attr_from_suffix("点線丸囲み"),
            Some(ForwardAttr::Framed(EnclosureKind::CircleDotted))
        );
        assert_eq!(
            forward_attr_from_suffix("二重罫囲み"),
            Some(ForwardAttr::Framed(EnclosureKind::DoubleRule))
        );
        assert_eq!(
            forward_attr_from_suffix("罫囲み"),
            Some(ForwardAttr::Framed(EnclosureKind::Rule))
        );
        assert_eq!(
            forward_attr_from_suffix("破線枠囲み"),
            Some(ForwardAttr::Framed(EnclosureKind::DashedRule))
        );
        assert_eq!(
            forward_attr_from_suffix("枠囲み"),
            Some(ForwardAttr::Framed(EnclosureKind::Unspecified))
        );
        // A near-miss glyph-bearing form is not one of these bare suffixes.
        assert_eq!(forward_attr_from_suffix("丸囲み"), None);
    }

    #[test]
    fn fraction_suffix_matches_only_the_exact_single_form() {
        // `「a/b」は分数` → Fraction.
        assert_eq!(
            forward_attr_from_suffix("分数"),
            Some(ForwardAttr::Fraction)
        );
        // A comma-joined compound (`「3」は上付き小文字、「1/143」は分数`) reaches this
        // fn with a suffix that is not exactly `分数`, so it must not match and
        // stays `Directive{Unknown}`; a multi-directive-per-bracket grammar is
        // out of scope.
        assert_eq!(
            forward_attr_from_suffix("上付き小文字、「1/143」は分数"),
            None
        );
        assert_eq!(forward_attr_from_suffix("分数、縦中横"), None);
    }

    #[test]
    fn align_end_suffix_parses_offset_and_verb_variants() {
        // The corpus-attested form: `「訳者」は文末より１字上げ揃え`.
        assert_eq!(
            forward_attr_from_suffix("文末より１字上げ揃え"),
            Some(ForwardAttr::AlignEnd { offset: 1 })
        );
        // Verb + anchor variants mirror the line-form recogniser.
        assert_eq!(
            forward_attr_from_suffix("行末より2字上がり"),
            Some(ForwardAttr::AlignEnd { offset: 2 })
        );
        assert_eq!(
            forward_attr_from_suffix("文末より3字上げ"),
            Some(ForwardAttr::AlignEnd { offset: 3 })
        );
        // 地付き and its corpus spelling 地付け are zero-lift forms;
        // 地より / 地から are accepted anchors.
        assert_eq!(
            forward_attr_from_suffix("地付き"),
            Some(ForwardAttr::AlignEnd { offset: 0 })
        );
        assert_eq!(
            forward_attr_from_suffix("地より１字上げ"),
            Some(ForwardAttr::AlignEnd { offset: 1 })
        );
        assert_eq!(
            forward_attr_from_suffix("地より１１字上げ"),
            Some(ForwardAttr::AlignEnd { offset: 11 })
        );
        assert_eq!(
            forward_attr_from_suffix("地付け"),
            Some(ForwardAttr::AlignEnd { offset: 0 })
        );
        // Zero magnitude remains unsupported.
        assert_eq!(forward_attr_from_suffix("地より0字上げ"), None);
        assert_eq!(
            forward_attr_from_suffix("地付き、地より３字アキ"),
            Some(ForwardAttr::AlignEnd { offset: 3 })
        );
        // Zero offset and unrelated suffixes stay Unknown.
        assert_eq!(forward_attr_from_suffix("文末より0字上げ"), None);
        assert_eq!(forward_attr_from_suffix("文末より字上げ"), None);
        assert_eq!(forward_attr_from_suffix("文頭より1字上げ"), None);
    }
}
