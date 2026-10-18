//! Pair stage: streaming balanced-stack pairing over the tokenize-stage token stream.
//!
//! Consumes the [`Token`] iterator produced by the tokenize stage and emits a
//! parallel [`PairEvent`] iterator: [`Token::Text`] / [`Token::Newline`]
//! pass through unchanged, and each [`Token::Trigger`] is classified
//! into [`PairEvent::PairOpen`] / [`PairEvent::PairClose`] /
//! [`PairEvent::Solo`] / [`PairEvent::Unmatched`] / [`PairEvent::Unclosed`].
//!
//! ## Pairing precedes classification
//!
//! Aozora annotation bodies nest:
//!
//! ```text
//! ［＃「青空」に傍点］:       quoted literal nested inside bracket body
//! ［＃底本では「旧字」］:      same shape, different keyword
//! ［＃「X［＃「Y」に傍点］Z」は底本では「W」］:   doubly nested
//! ```
//!
//! A scan for the next `］` finds the first `］` even when it closes an
//! inner bracket, which truncates the body. This stage runs a balanced
//! stack, so a body's extent is fixed before any classifier parses it.
//!
//! ## Mismatch policy
//!
//! * **Unclosed open**: left on the stack at end-of-input. The original
//!   `PairOpen` event is already streamed downstream when the stage
//!   reaches EOF with the open still on the stack, so at EOF the stage
//!   emits a synthetic [`PairEvent::Unclosed`] for each still-open frame
//!   and pushes a [`Diagnostic::UnclosedBracket`]. The classify stage's
//!   stack-aware classifier reads the trailing `Unclosed` as "the matching
//!   open never closed; treat its accumulated body events as plain".
//! * **Stray close** (empty stack or kind-mismatched top): emitted as
//!   [`PairEvent::Unmatched`] with a [`Diagnostic::UnmatchedClose`].
//!   The stack is not popped, so a well-formed outer pair like `［...］`
//!   still closes when a stray `》` occurs inside the body.
//! * **Bracket is a hard pairing scope** (refines the stray-close rule
//!   for `］` only): a `］` closes the *nearest enclosing* `［`, even when
//!   non-bracket opens are stacked above it, force-resolving those opens
//!   as [`PairEvent::Unclosed`] (innermost-first) before the
//!   [`PairEvent::PairClose`]. An unbalanced `「` inside a directive body
//!   (an image caption `［＃「…（fig）入る］`, a composed-glyph gaiji
//!   `［＃「口＋「皐」…］`, a typo-note quoting literal quotes) therefore does
//!   not prevent closure at `］`; otherwise the bracket would never close and
//!   the rest of the document would become plain text. A balanced body does
//!   not reach this rule, except for a closing delimiter quoted as a single
//!   glyph; that glyph is text when its quote closer immediately follows it.
//! * **A non-bracket closer is a hard scope for plain brackets opened inside
//!   it**, but only inside an annotation body: when the nearest enclosing open
//!   of that kind is itself inside a bracket and has only plain brackets
//!   stacked above it. This resolves a `［` quoted verbatim in a body
//!   (`［＃「［あ」は底本では「（あ」］`), which has no closer of its own and
//!   would otherwise pair with the annotation's `］`. A nested `［＃` is excluded:
//!   it is an annotation in its own right and closes at its own `］`. Outside
//!   an annotation body a closer still cannot cross a bracket downward.
//! * **A plain `［` opened directly in a directive body is text**, with no
//!   quote around it to scope it (`［＃［愛」に「ママ」の注記］`, a source typo
//!   for `「愛」`). It has no closer of its own, so pairing it would close the
//!   stray opener at the annotation's `］`. The two readings are
//!   indistinguishable at that `］`, so the decision is taken at the open,
//!   where one trigger of lookahead separates a nested `［＃` from a stray `［`.

use core::iter::Peekable;
use core::mem;

use ab_aozora_syntax::Span;
use smallvec::SmallVec;

use super::token::{Token, TriggerKind};
use ab_aozora_spec::Diagnostic;

// `PairKind` is defined in `aozora-spec` and re-exported here. `PairLink`
// is the resolved (open, close) view recorded during the pair pass.
pub use ab_aozora_spec::{PairKind, PairLink};

/// How many newlines a `［＃` directive bracket may span before the
/// newline expiry reclaims it. Multi-line directives occur but are
/// bounded (the corpus maximum is a 12-line ［＃入力者註：…］ editorial
/// note); a stray unclosed ［＃ typo spans at most this many lines, not
/// the rest of the document.
const MULTILINE_DIRECTIVE_NEWLINE_ALLOWANCE: u8 = 32;

/// One event in the pair-stage stream.
///
/// `PairOpen` and `PairClose` carry only their `kind` and `span`.
/// Body cross-link information (which `PairOpen` matches which
/// `PairClose` inside a body buffer) is maintained out-of-band by
/// the classify stage in a parallel `pair_links` side-table inside the
/// classifier's `BodyView`, so no `PairEvent` field has one meaning in
/// pair-stage emission and another in classify-stage internal patching.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PairEvent {
    /// A source text run, including a quoted scalar delimiter glyph.
    Text {
        /// Sanitized-source byte span of the run; may be empty.
        range: Span,
    },

    /// A trigger with no opposing pair on its own (`｜`, `＃`, `※`).
    Solo {
        /// The standalone trigger's role.
        kind: TriggerKind,
        /// Sanitized-source byte span of the trigger.
        span: Span,
    },

    /// Matched open delimiter. The classify stage pushes a new body-buffer
    /// frame onto its own stack on this event. The matching close's
    /// body-local index is recorded in the parallel `links` side-table
    /// once the close arrives.
    PairOpen {
        /// Bracket family this open belongs to.
        kind: PairKind,
        /// Sanitized-source byte span of the opening delimiter.
        span: Span,
    },

    /// Matched close delimiter. The classify stage pops the corresponding
    /// body frame on this event and runs recognition on the buffered body.
    /// The matching open's body-local index lives in the parallel
    /// `links` side-table.
    PairClose {
        /// Bracket family this close belongs to (matches its open).
        kind: PairKind,
        /// Sanitized-source byte span of the closing delimiter.
        span: Span,
    },

    /// End-of-stream synthetic event indicating that an earlier
    /// [`PairEvent::PairOpen`] of the carried `kind` was never closed.
    /// The classify stage treats the corresponding body buffer as having no
    /// matching close and re-emits the buffered events as plain.
    Unclosed {
        /// Bracket family of the open that never closed.
        kind: PairKind,
        /// Sanitized-source byte span of the original (still-open) open
        /// delimiter, not an end-of-input position.
        span: Span,
    },

    /// Close delimiter that met an empty stack or a kind-mismatched
    /// stack top. Classifier treats the span as plain text.
    Unmatched {
        /// Bracket family the stray close would have closed.
        kind: PairKind,
        /// Sanitized-source byte span of the stray close delimiter.
        span: Span,
    },

    /// Unchanged from [`Token::Newline`]; kept so the classify stage can
    /// attach line structure to block-level annotations.
    Newline {
        /// Sanitized-source byte offset of the `\n`.
        pos: u32,
    },
}

impl PairEvent {
    /// Source byte-range span of this event, or `None` for
    /// [`PairEvent::Newline`] (which has only a single position, not a
    /// range).
    #[must_use]
    pub const fn span(&self) -> Option<Span> {
        Some(match *self {
            Self::Text { range } => range,
            Self::Solo { span, .. }
            | Self::PairOpen { span, .. }
            | Self::PairClose { span, .. }
            | Self::Unclosed { span, .. }
            | Self::Unmatched { span, .. } => span,
            Self::Newline { .. } => return None,
        })
    }
}

/// Run the streaming balanced-stack pass over a tokenize-stage token stream.
///
/// The returned [`PairStream`] is an iterator yielding one
/// [`PairEvent`] per call to [`Iterator::next`]. After the iterator is
/// exhausted, call [`PairStream::take_diagnostics`] to drain any
/// non-fatal observations that accumulated during the pass
/// (unclosed opens, unmatched closes).
#[must_use]
pub fn pair<I>(tokens: I) -> PairStream<I>
where
    I: Iterator<Item = Token>,
{
    PairStream::new(tokens)
}

/// Stream of [`PairEvent`]s produced from an upstream [`Token`]
/// iterator. Internal state:
///
/// * `tokens`: upstream token producer; tokens are pulled lazily.
/// * `stack`: smallvec of open `PairKind`s with their open spans.
///   Inline capacity 8 covers the 99th-percentile bracket nesting in
///   real Aozora text (corpus profile).
/// * `diagnostics`: collected non-fatal observations.
/// * `links`: resolved `(open, close)` pairs accumulated as the
///   stack matches.
/// * `pending`: FIFO queue of events a single trigger produced beyond
///   the one it returns directly. A `］` that closes a bracket beneath
///   unclosed non-bracket opens yields several events at once
///   (`Unclosed`… then `PairClose`); the head is returned and the tail
///   is buffered here, drained front-first before the next token.
/// * `eof_drain`: cursor through the residual stack at end-of-input
///   used to emit one `Unclosed` event per remaining open frame.
/// * `finished`: terminal flag set after the eof drain completes,
///   so subsequent `next()` calls return `None` without re-walking
///   the stack.
#[derive(Debug)]
pub struct PairStream<I>
where
    I: Iterator<Item = Token>,
{
    tokens: Peekable<I>,
    stack: SmallVec<[(PairKind, Span, u8); 8]>,
    diagnostics: Vec<Diagnostic>,
    /// Resolved (open, close) pairs collected as the stack matches.
    links: Vec<PairLink>,
    /// Events queued by the current trigger for later `next()` calls,
    /// drained front-first. Empty on the fast paths.
    pending: SmallVec<[PairEvent; 4]>,
    eof_drain: bool,
    finished: bool,
}

impl<I> PairStream<I>
where
    I: Iterator<Item = Token>,
{
    fn new(tokens: I) -> Self {
        Self {
            tokens: tokens.peekable(),
            stack: SmallVec::new(),
            diagnostics: Vec::new(),
            links: Vec::new(),
            pending: SmallVec::new(),
            eof_drain: false,
            finished: false,
        }
    }

    /// Drain accumulated diagnostics. Should be called after the
    /// iterator is exhausted (otherwise EOF unclosed-bracket
    /// diagnostics will not yet have been emitted).
    pub fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        mem::take(&mut self.diagnostics)
    }

    /// Borrow accumulated diagnostics in place. Same caveat as
    /// [`Self::take_diagnostics`]: only complete after exhaustion.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Drain the resolved [`PairLink`] side-table. Same exhaustion
    /// caveat as [`Self::take_diagnostics`].
    pub fn take_links(&mut self) -> Vec<PairLink> {
        mem::take(&mut self.links)
    }

    /// Borrow the resolved [`PairLink`] list in place. Same caveat
    /// applies: only complete after exhaustion.
    #[must_use]
    pub fn links(&self) -> &[PairLink] {
        &self.links
    }

    /// Close the open frame at `open_pos`, force-resolving everything stacked
    /// above it as `Unclosed` first.
    ///
    /// Pops top-first so the innermost unclosed open is emitted first, matching
    /// the EOF-drain order in `next`. `pending` is left holding
    /// [`PairEvent::Unclosed`]… (innermost-first) then
    /// [`PairEvent::PairClose`]; the head is returned and `next` drains the
    /// tail in order.
    fn close_through(&mut self, kind: PairKind, open_pos: usize, close: Span) -> PairEvent {
        while self.stack.len() > open_pos + 1 {
            let (k, open_span, _) = self.stack.pop().expect("len > open_pos + 1");
            self.diagnostics
                .push(Diagnostic::unclosed_bracket(open_span, k));
            self.pending.push(PairEvent::Unclosed {
                kind: k,
                span: open_span,
            });
        }
        let (_, open_span, _) = self.stack.pop().expect("open frame at open_pos");
        self.links.push(PairLink::new(kind, open_span, close));
        self.pending
            .push(PairEvent::PairClose { kind, span: close });
        self.pending.remove(0)
    }

    fn classify_trigger(&mut self, kind: TriggerKind, span: Span) -> PairEvent {
        if let Some(pair_kind) = open_kind_of(kind) {
            // A plain `［` opened directly in a directive body is data, not
            // structure. Bodies quote source text, and a source that writes
            // `［愛」` where it meant `「愛」` leaves an opener with no partner.
            // Pushed on the stack, it becomes the nearest enclosing bracket, so
            // the annotation's own `］` closes it, the annotation stays open
            // until the newline expiry, and the expiry resolves every marker
            // within its allowance as plain text. The rule below does not apply
            // to this shape: no quote surrounds the stray opener to scope it,
            // and the closing `］` cannot distinguish the two readings, since
            // they differ only in whether a second `］` follows before the line
            // ends.
            //
            // Deciding at the open needs no lookahead beyond the next trigger
            // and excludes no attested reading. Across 17,877 corpus archives
            // this is the only position where an unquoted `［` opens directly
            // inside a directive body, and that one instance is the malformed
            // one; a balanced pair in this position is unattested. A nested
            // `［＃` is excluded, being an annotation in its own right, and a
            // `［` opened inside a quote in the body is handled by the rule
            // below.
            if pair_kind == PairKind::Bracket
                && self
                    .stack
                    .last()
                    .is_some_and(|&(top, _, allowance)| top == PairKind::Bracket && allowance > 0)
                && !matches!(self.tokens.peek(), Some(Token::Trigger {
                    kind: TriggerKind::Hash, span: hash,
                }) if hash.start == span.end)
            {
                return PairEvent::Text { range: span };
            }
            self.stack.push((pair_kind, span, 0));
            return PairEvent::PairOpen {
                kind: pair_kind,
                span,
            };
        }

        if let Some(pair_kind) = close_kind_of(kind) {
            if pair_kind != PairKind::Quote
                && self
                    .stack
                    .last()
                    .is_some_and(|&(top, open, _)| top == PairKind::Quote && open.end == span.start)
                && matches!(self.tokens.peek(), Some(Token::Trigger {
                    kind: TriggerKind::QuoteClose, span: close,
                }) if close.start == span.end)
            {
                return PairEvent::Text { range: span };
            }

            if let Some(&(top, open_span, _)) = self.stack.last()
                && top == pair_kind
            {
                self.stack.pop();
                self.links.push(PairLink::new(pair_kind, open_span, span));
                return PairEvent::PairClose {
                    kind: pair_kind,
                    span,
                };
            }

            // A `］` treats its bracket as a hard pairing scope: it closes
            // the nearest enclosing `［`, force-resolving any non-bracket opens
            // stacked above that bracket as `Unclosed` (innermost-first) before
            // the close. An unbalanced `「` inside a directive body (e.g.
            // `［＃「…（fig）入る］` or the composed-glyph gaiji `［＃「口＋「皐」…］`)
            // therefore does not hide the `］`; otherwise the bracket would
            // never close and the classifier would treat the rest of the
            // document as plain. A quoted scalar closer is already retained as
            // text above.
            if pair_kind == PairKind::Bracket
                && let Some(bracket_pos) = self
                    .stack
                    .iter()
                    .rposition(|&(k, _, _)| k == PairKind::Bracket)
            {
                // Everything above `bracket_pos` is non-bracket by construction
                // (it is the top-most bracket).
                return self.close_through(pair_kind, bracket_pos, span);
            }

            // The counterpart of that rule, for the one shape it does not
            // reach: a quoted `［` inside an annotation body. Bodies quote source
            // text verbatim (`［＃「［あ」は底本では「（あ」］`), so a `［` the quote
            // opened is data and has no closer of its own. Left on the stack, it
            // becomes the nearest enclosing bracket, so the rule above closes it
            // instead of the annotation, and the annotation's own bracket stays
            // open until the newline expiry, which resolves every marker within
            // its allowance as plain text.
            //
            // A non-bracket closer is therefore a hard scope for brackets
            // opened inside it, under the conditions that make that reading
            // unambiguous. The opener is itself inside a bracket, which makes
            // this an annotation body rather than running text, so a crossing
            // in prose (`「あ［い」う］`) is left unchanged. And the only opens
            // above it are plain brackets, carrying no directive newline
            // allowance: a nested `［＃` is an annotation in its own right and
            // closes at its own `］`, so a body that quotes a chain of gaiji
            // annotations (`［＃「※［＃「麾」の「毛」に代えて「手」」、42-8］…」に白丸傍点］`)
            // keeps its pairing.
            if let Some(open_pos) = self.stack.iter().rposition(|&(k, _, _)| k == pair_kind)
                && self.stack[open_pos + 1..]
                    .iter()
                    .all(|&(k, _, allowance)| k == PairKind::Bracket && allowance == 0)
                && self.stack[..open_pos]
                    .iter()
                    .any(|&(k, _, _)| k == PairKind::Bracket)
            {
                return self.close_through(pair_kind, open_pos, span);
            }

            self.diagnostics
                .push(Diagnostic::unmatched_close(span, pair_kind));
            return PairEvent::Unmatched {
                kind: pair_kind,
                span,
            };
        }

        // Trigger is neither open nor close (Bar / Hash / RefMark). A
        // `＃` immediately after a bracket open marks that open as a
        // directive bracket (`［＃`): unlike a plain gloss `［`, it may span
        // lines (multi-line 入力者註 editorial notes), so the newline expiry
        // skips it while it has newline allowance.
        if kind == TriggerKind::Hash
            && let Some(top) = self.stack.last_mut()
            && top.0 == PairKind::Bracket
            && top.1.end == span.start
        {
            top.2 = MULTILINE_DIRECTIVE_NEWLINE_ALLOWANCE;
        }
        PairEvent::Solo { kind, span }
    }
}

impl<I> Iterator for PairStream<I>
where
    I: Iterator<Item = Token>,
{
    type Item = PairEvent;

    fn next(&mut self) -> Option<PairEvent> {
        if self.finished {
            return None;
        }
        // A single `］` may have produced several events (dangling-open
        // unwind + the bracket close); return the buffered tail before
        // pulling the next token so ordering is preserved.
        if !self.pending.is_empty() {
            return Some(self.pending.remove(0));
        }
        if self.eof_drain {
            // Drain residual stack entries as Unclosed events. Pop from
            // the back, so innermost (last-pushed) opens are emitted
            // first.
            if let Some((kind, span, _)) = self.stack.pop() {
                self.diagnostics
                    .push(Diagnostic::unclosed_bracket(span, kind));
                return Some(PairEvent::Unclosed { kind, span });
            }
            self.finished = true;
            return None;
        }

        match self.tokens.next() {
            Some(Token::Text { range }) => Some(PairEvent::Text { range }),
            Some(Token::Newline { pos }) => {
                // Aozora annotations are single-line: a `［` still open at
                // the newline can never close as a directive, and leaving
                // it on the stack makes the classifier buffer the rest of
                // the document (one stray gloss bracket would hold the
                // markers of every later line in its body). Expire the
                // lowest expirable open bracket and everything nested above
                // it, innermost-first, as the EOF drain does, then emit the
                // newline. Opens below the bracket (a multi-paragraph 「)
                // stay on the stack, as does anything beneath a directive
                // bracket that still has newline allowance.
                for entry in &mut self.stack {
                    if entry.0 == PairKind::Bracket && entry.2 > 0 {
                        entry.2 -= 1;
                    }
                }
                // A directive bracket that still has newline allowance
                // keeps every frame below it on the stack. The stack can
                // only be popped from the top, so expiring a plain `［`
                // underneath a live multi-line directive would also pop
                // the directive, whose allowance covers this newline. The
                // exemption is bounded: the directive's own allowance was
                // decremented above, so a stray gloss bracket beneath a
                // directive is still reclaimed within
                // `MULTILINE_DIRECTIVE_NEWLINE_ALLOWANCE` newlines.
                let protected = self
                    .stack
                    .iter()
                    .rposition(|&(k, _, allowance)| k == PairKind::Bracket && allowance > 0)
                    .map_or(0, |index| index + 1);
                if let Some(lowest) = self.stack[protected..]
                    .iter()
                    .position(|&(k, _, allowance)| k == PairKind::Bracket && allowance == 0)
                    .map(|index| index + protected)
                {
                    while self.stack.len() > lowest {
                        let (k, open_span, _) = self.stack.pop().expect("len > lowest");
                        self.diagnostics
                            .push(Diagnostic::unclosed_bracket(open_span, k));
                        self.pending.push(PairEvent::Unclosed {
                            kind: k,
                            span: open_span,
                        });
                    }
                    self.pending.push(PairEvent::Newline { pos });
                    return Some(self.pending.remove(0));
                }
                Some(PairEvent::Newline { pos })
            }
            Some(Token::Trigger { kind, span }) => Some(self.classify_trigger(kind, span)),
            None => {
                // Upstream exhausted. Switch into EOF-drain mode and
                // recurse to either yield the first Unclosed or
                // terminate.
                self.eof_drain = true;
                self.next()
            }
        }
    }
}

/// Map a trigger to the [`PairKind`] it opens, if any.
const fn open_kind_of(kind: TriggerKind) -> Option<PairKind> {
    Some(match kind {
        TriggerKind::BracketOpen => PairKind::Bracket,
        TriggerKind::RubyOpen => PairKind::Ruby,
        TriggerKind::AngleQuoteOpen => PairKind::AngleQuote,
        TriggerKind::TortoiseOpen => PairKind::Tortoise,
        TriggerKind::QuoteOpen => PairKind::Quote,
        _ => return None,
    })
}

/// Map a trigger to the [`PairKind`] it closes, if any.
const fn close_kind_of(kind: TriggerKind) -> Option<PairKind> {
    Some(match kind {
        TriggerKind::BracketClose => PairKind::Bracket,
        TriggerKind::RubyClose => PairKind::Ruby,
        TriggerKind::AngleQuoteClose => PairKind::AngleQuote,
        TriggerKind::TortoiseClose => PairKind::Tortoise,
        TriggerKind::QuoteClose => PairKind::Quote,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::lexer::tokenize::tokenize;

    /// Materialise the full stream + diagnostics for tests.
    fn run(src: &str) -> (Vec<PairEvent>, Vec<Diagnostic>) {
        let mut stream = pair(tokenize(src));
        let events: Vec<PairEvent> = (&mut stream).collect();
        let diagnostics = stream.take_diagnostics();
        (events, diagnostics)
    }

    #[test]
    fn quoted_closing_glyph_belongs_to_the_quote_not_the_annotation_boundary() {
        let source = "［＃「〕」は底本では「］」］";
        let mut stream = pair(tokenize(source));
        let events = (&mut stream).collect::<Vec<_>>();
        assert!(
            stream.diagnostics().is_empty(),
            "{:?}",
            stream.diagnostics()
        );
        let bracket = stream
            .links()
            .iter()
            .filter(|p| p.kind == PairKind::Bracket)
            .collect::<Vec<_>>();
        assert_eq!(bracket.len(), 1);
        assert_eq!(bracket[0].open.start, 0);
        assert_eq!(usize::try_from(bracket[0].close.end).unwrap(), source.len());
        assert!(events.iter().any(
            |event| matches!(event, PairEvent::Text { range } if range.slice(source) == "］")
        ));
    }

    #[test]
    fn a_quoted_opening_bracket_does_not_absorb_the_annotation_close() {
        // The body quotes source text that contains a `［` with no partner. The
        // annotation's own `］` has to close the annotation, not the quoted
        // glyph; otherwise the annotation bracket stays open until the
        // newline expiry, which resolves every later marker within its
        // allowance as plain text.
        let source = "［＃「［あ」は底本では「（あ」］";
        let (events, _) = run(source);
        let close = events
            .iter()
            .filter_map(|event| match *event {
                PairEvent::PairClose {
                    kind: PairKind::Bracket,
                    span,
                } => Some(span),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(close.len(), 1, "{events:?}");
        assert_eq!(usize::try_from(close[0].end).unwrap(), source.len());
    }

    #[test]
    fn a_balanced_quoted_bracket_pair_still_closes_as_itself() {
        // The same shape with the quoted bracket balanced: the inner pair
        // resolves on its own before the quote closes, so the quoted-opener
        // rule does not apply and both brackets close at their own closers.
        let source = "［＃「［あ］」は底本では「（あ）」］";
        let (events, diagnostics) = run(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let close = events
            .iter()
            .filter_map(|event| match *event {
                PairEvent::PairClose {
                    kind: PairKind::Bracket,
                    span,
                } => Some(usize::try_from(span.end).unwrap()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(close, vec!["［＃「［あ］".len(), source.len()]);
    }

    #[test]
    fn a_nested_annotation_in_a_body_still_closes_at_its_own_bracket() {
        // Each gaiji description here ends in a doubled quote closer, so the
        // outer body's quote closes while the nested annotation's bracket is
        // still open. That bracket is a directive and resolves at its own `］`,
        // so the closer does not force-resolve it.
        let source = "［＃「※［＃「麾」の「毛」に代えて「手」」、42-8］」に白丸傍点］";
        let (events, _) = run(source);
        let close = events
            .iter()
            .filter_map(|event| match *event {
                PairEvent::PairClose {
                    kind: PairKind::Bracket,
                    span,
                } => Some(usize::try_from(span.end).unwrap()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            close,
            vec![
                "［＃「※［＃「麾」の「毛」に代えて「手」」、42-8］".len(),
                source.len()
            ],
            "{events:?}"
        );
    }

    #[test]
    fn a_quoted_opening_bracket_outside_an_annotation_body_is_left_alone() {
        // Running text, not an annotation body: the quote's opener is not
        // inside a bracket, so the crossing keeps its pairing, with the
        // `］` closing the `［` and the `」` unmatched.
        let source = "「あ［い」う］";
        let (events, _) = run(source);
        assert!(
            events.iter().any(|event| matches!(
                event,
                PairEvent::Unmatched {
                    kind: PairKind::Quote,
                    ..
                }
            )),
            "{events:?}"
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                PairEvent::PairClose {
                    kind: PairKind::Bracket,
                    ..
                }
            )),
            "{events:?}"
        );
    }

    #[test]
    fn an_unquoted_opening_bracket_in_a_body_does_not_absorb_the_annotation_close() {
        // Work 000662 writes `［愛」` where it meant `「愛」`. The stray opener
        // has no quote around it, so the rule for quoted openers cannot scope
        // it; pairing it would close it at the annotation's `］`, and the
        // newline expiry would then expire the annotation together with the
        // markers on the lines it spans.
        let source = "［＃［愛」に「ママ」の注記］";
        let (events, _) = run(source);
        let close = events
            .iter()
            .filter_map(|event| match *event {
                PairEvent::PairClose {
                    kind: PairKind::Bracket,
                    span,
                } => Some(usize::try_from(span.end).unwrap()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(close, vec![source.len()], "{events:?}");
        assert!(
            events.iter().any(
                |event| matches!(event, PairEvent::Text { range } if range.slice(source) == "［")
            ),
            "{events:?}"
        );
    }

    #[test]
    fn a_nested_directive_open_in_a_body_is_still_a_bracket() {
        // The same position, but the opener is `［＃`: an annotation in its own
        // right, closing at its own `］`. One trigger of lookahead separates it
        // from the stray opener above.
        let source = "［＃注［＃「あ」に傍点］］";
        let (events, diagnostics) = run(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let close = events
            .iter()
            .filter_map(|event| match *event {
                PairEvent::PairClose {
                    kind: PairKind::Bracket,
                    span,
                } => Some(usize::try_from(span.end).unwrap()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(close, vec!["［＃注［＃「あ」に傍点］".len(), source.len()]);
    }

    #[test]
    fn a_plain_bracket_nested_in_a_plain_bracket_still_pairs() {
        // Not a directive body: the enclosing `［` carries no newline
        // allowance, so the rule reaches neither running text nor a gloss.
        let source = "［あ［い］う］";
        let (events, diagnostics) = run(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let closes = events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    PairEvent::PairClose {
                        kind: PairKind::Bracket,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(closes, 2, "{events:?}");
    }

    #[test]
    fn quoted_scalar_closers_do_not_disable_malformed_quote_recovery() {
        for glyph in ["］", "〕", "》", "≫"] {
            let source = format!("［＃「{glyph}」］");
            let (_, diagnostics) = run(&source);
            assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}");
        }
        for source in ["［＃「未完］後", "［＃「口＋「皐」］後"] {
            let (events, diagnostics) = run(source);
            assert!(events.iter().any(|event| matches!(
                event,
                PairEvent::PairClose {
                    kind: PairKind::Bracket,
                    ..
                }
            )));
            assert!(!diagnostics.is_empty());
        }
        let source = "［＃「］」";
        let (events, diagnostics) = run(source);
        assert!(events.iter().any(|event| matches!(
            event,
            PairEvent::Unclosed {
                kind: PairKind::Bracket,
                ..
            }
        )));
        assert!(!diagnostics.is_empty());
    }

    fn pair_kinds(events: &[PairEvent]) -> Vec<(&'static str, PairKind)> {
        events
            .iter()
            .filter_map(|e| match *e {
                PairEvent::PairOpen { kind, .. } => Some(("open", kind)),
                PairEvent::PairClose { kind, .. } => Some(("close", kind)),
                PairEvent::Unclosed { kind, .. } => Some(("unclosed", kind)),
                PairEvent::Unmatched { kind, .. } => Some(("unmatched", kind)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn newline_expires_open_bracket_so_later_lines_pair_fresh() {
        // Aozora annotations are single-line: a stray gloss `［` left open
        // on one line (e.g. `彼の所有［当然` in a German-textbook work) must
        // not place every later line's directives in one unterminated body.
        let (events, diagnostics) = run("彼の［当然\n※［＃「口＋世」、U+546D］猫\n");
        assert_eq!(
            pair_kinds(&events),
            vec![
                ("open", PairKind::Bracket),
                ("unclosed", PairKind::Bracket),
                ("open", PairKind::Bracket),
                ("open", PairKind::Quote),
                ("close", PairKind::Quote),
                ("close", PairKind::Bracket),
            ]
        );
        // The expiry is observable: exactly one unclosed-bracket
        // diagnostic, anchored on the stray open, before the newline.
        assert_eq!(diagnostics.len(), 1);
        // The second line's directive resolved as a pair.
        let unclosed_pos = events
            .iter()
            .position(|e| matches!(e, PairEvent::Unclosed { .. }))
            .expect("unclosed event");
        let newline_pos = events
            .iter()
            .position(|e| matches!(e, PairEvent::Newline { .. }))
            .expect("newline event");
        assert!(unclosed_pos < newline_pos);
    }

    #[test]
    fn newline_keeps_multi_line_directive_bracket_open() {
        // A `［＃…` directive spans lines in rare editorial
        // notes (北條民雄全集's ［＃入力者註：…9 lines…］). Only plain `［`
        // opens (which can never become directives) expire at the
        // newline.
        let (events, diagnostics) = run("［＃入力者註：以下を修正した。\n「甲」→「乙」］\n本文\n");
        assert!(
            pair_kinds(&events)
                .iter()
                .any(|&(what, kind)| what == "close" && kind == PairKind::Bracket)
        );
        assert!(
            !pair_kinds(&events)
                .iter()
                .any(|&(what, _)| what == "unclosed")
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn directive_bracket_expires_after_bounded_newline_allowance() {
        // A stray unclosed ［＃ (a mistyped nested marker like
        // ［＃［愛」に「ママ」の注記］) may span lines, but only up to the
        // bounded allowance; beyond it the expiry reclaims it so the
        // rest of the document still classifies.
        let mut src = String::from("可愛［＃タイポ\n");
        for _ in 0..40 {
            src.push_str("本文の一行。\n");
        }
        let (events, diagnostics) = run(&src);
        assert!(
            pair_kinds(&events)
                .iter()
                .any(|&(what, kind)| what == "unclosed" && kind == PairKind::Bracket)
        );
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn live_directive_protects_a_stray_bracket_beneath_it_from_the_newline() {
        // A stack pops only from the top, so expiring the stray gloss `［`
        // would also pop the multi-line directive nested above it, whose
        // allowance covers this newline. The directive closes
        // at its own `］`; the stray drains at EOF.
        let (events, diagnostics) =
            run("彼の［［＃入力者註：以下を修正した。\n「甲」→「乙」］\n本文\n");
        assert_eq!(
            pair_kinds(&events),
            vec![
                ("open", PairKind::Bracket),
                ("open", PairKind::Bracket),
                ("open", PairKind::Quote),
                ("close", PairKind::Quote),
                ("open", PairKind::Quote),
                ("close", PairKind::Quote),
                ("close", PairKind::Bracket),
                ("unclosed", PairKind::Bracket),
            ]
        );
        // Only the stray is reported, and only once the directive is gone.
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn a_protected_stray_bracket_is_still_reclaimed_within_the_allowance() {
        // The exemption is bounded: the directive's own allowance decreases
        // across the newlines it spans, and when it reaches zero the expiry
        // closes both frames, so a stray gloss bracket cannot hold the rest
        // of the document in its body.
        let mut src = String::from("彼の［［＃タイポ\n");
        for _ in 0..40 {
            src.push_str("本文の一行。\n");
        }
        let (events, diagnostics) = run(&src);
        assert_eq!(
            pair_kinds(&events),
            vec![
                ("open", PairKind::Bracket),
                ("open", PairKind::Bracket),
                ("unclosed", PairKind::Bracket),
                ("unclosed", PairKind::Bracket),
            ]
        );
        assert_eq!(diagnostics.len(), 2);
    }

    #[test]
    fn newline_keeps_multi_line_quote_open() {
        // Prose 「 spans paragraphs; only bracket opens (and
        // anything nested above them) expire at the newline.
        let (events, diagnostics) = run("「話しはじめた\nそして続く」\n");
        assert_eq!(
            pair_kinds(&events),
            vec![("open", PairKind::Quote), ("close", PairKind::Quote)]
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn empty_input_yields_no_events() {
        let (events, diagnostics) = run("");
        assert!(events.is_empty());
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn plain_text_passes_through_as_text_event() {
        let (events, diagnostics) = run("hello");
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], PairEvent::Text { .. }));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn simple_bracket_pair_emits_open_and_close() {
        let (events, diagnostics) = run("［body］");
        // Events: PairOpen(Bracket), Text("body"), PairClose(Bracket).
        assert_eq!(events.len(), 3);
        assert!(matches!(
            events[0],
            PairEvent::PairOpen {
                kind: PairKind::Bracket,
                ..
            }
        ));
        assert!(matches!(
            events[2],
            PairEvent::PairClose {
                kind: PairKind::Bracket,
                ..
            }
        ));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn nested_brackets_pair_inner_before_outer() {
        let (events, diagnostics) = run("［＃外［＃内］終］");
        // 0 PairOpen Bracket, 1 Solo Hash, 2 Text "外",
        // 3 PairOpen Bracket, 4 Solo Hash, 5 Text "内",
        // 6 PairClose Bracket, 7 Text "終", 8 PairClose Bracket.
        assert_eq!(events.len(), 9);
        assert!(matches!(
            events[0],
            PairEvent::PairOpen {
                kind: PairKind::Bracket,
                ..
            }
        ));
        assert!(matches!(
            events[3],
            PairEvent::PairOpen {
                kind: PairKind::Bracket,
                ..
            }
        ));
        assert!(matches!(
            events[6],
            PairEvent::PairClose {
                kind: PairKind::Bracket,
                ..
            }
        ));
        assert!(matches!(
            events[8],
            PairEvent::PairClose {
                kind: PairKind::Bracket,
                ..
            }
        ));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ruby_pair_emits_ruby_kinds() {
        let (events, diagnostics) = run("《かんじ》");
        assert_eq!(
            pair_kinds(&events),
            vec![("open", PairKind::Ruby), ("close", PairKind::Ruby)]
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn angle_quote_is_its_own_pair_kind() {
        let (events, _diagnostics) = run("≪X≫");
        assert_eq!(
            pair_kinds(&events),
            vec![
                ("open", PairKind::AngleQuote),
                ("close", PairKind::AngleQuote),
            ]
        );
    }

    #[test]
    fn tortoise_pair_emits_tortoise_kinds() {
        let (events, _) = run("〔e^〕");
        assert_eq!(
            pair_kinds(&events),
            vec![("open", PairKind::Tortoise), ("close", PairKind::Tortoise)]
        );
    }

    #[test]
    fn quote_pair_standalone_emits_quote_kinds() {
        let (events, _) = run("「台詞」");
        assert_eq!(
            pair_kinds(&events),
            vec![("open", PairKind::Quote), ("close", PairKind::Quote)]
        );
    }

    #[test]
    fn solo_bar_hash_refmark_remain_solo() {
        let (events, _) = run("｜＃※");
        assert_eq!(events.len(), 3);
        for ev in &events {
            assert!(
                matches!(ev, PairEvent::Solo { .. }),
                "expected all Solo, got {ev:?}"
            );
        }
    }

    #[test]
    fn newline_passes_through_unchanged() {
        let (events, _) = run("a\nb");
        assert_eq!(events.len(), 3);
        assert!(matches!(events[1], PairEvent::Newline { .. }));
    }

    #[test]
    fn unclosed_bracket_appends_synthetic_unclosed_event() {
        let (events, diagnostics) = run("［＃unclosed");
        // Stream: PairOpen, Solo(Hash), Text, ...then EOF appends Unclosed.
        assert!(
            events.iter().any(|e| matches!(
                e,
                PairEvent::Unclosed {
                    kind: PairKind::Bracket,
                    ..
                }
            )),
            "expected an Unclosed Bracket event in {events:?}"
        );
        assert!(diagnostics.iter().any(|d| matches!(
            d,
            Diagnostic::UnclosedBracket {
                kind: PairKind::Bracket,
                ..
            }
        )));
    }

    #[test]
    fn unmatched_close_emits_diagnostic_without_affecting_stack() {
        let (events, diagnostics) = run("stray］text");
        assert!(events.iter().any(|e| matches!(
            e,
            PairEvent::Unmatched {
                kind: PairKind::Bracket,
                ..
            }
        )));
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn mismatched_close_inside_bracket_does_not_pop_outer() {
        let (events, diagnostics) = run("［body》more］");
        let kinds = pair_kinds(&events);
        assert_eq!(
            kinds,
            vec![
                ("open", PairKind::Bracket),
                ("unmatched", PairKind::Ruby),
                ("close", PairKind::Bracket),
            ]
        );
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn event_count_matches_token_count_plus_eof_unclosed() {
        // Every input Token maps to exactly one event, plus one synthetic
        // Unclosed for each still-open frame at EOF. Downstream position
        // tracking relies on this sum.
        let src = "［＃「a」に］plain《b》〔c〕";
        let token_count = tokenize(src).count();
        let (events, _diagnostics) = run(src);
        assert_eq!(events.len(), token_count, "no unclosed in this src");
    }

    #[test]
    fn span_accessor_returns_range_for_text_and_trigger_events() {
        let (events, _) = run("a｜b《c》");
        for ev in &events {
            match ev {
                PairEvent::Newline { .. } => {
                    assert!(ev.span().is_none(), "Newline must have no span");
                }
                _ => {
                    assert!(ev.span().is_some(), "non-Newline event must carry a span");
                }
            }
        }
    }

    #[test]
    fn span_accessor_returns_none_for_newline() {
        let (events, _) = run("\n");
        assert_eq!(events.len(), 1);
        assert!(events[0].span().is_none());
    }

    /// Three nested unclosed `［＃` opens reach EOF together. The
    /// EOF-drain loop must surface them innermost-first (`stack.pop()`
    /// from the back), and emit one `UnclosedBracket` diagnostic per
    /// frame in the same order. Pins the diagnostic ordering callers
    /// rely on for spans rendering.
    #[test]
    fn pair_stream_eof_drains_innermost_first_after_multiple_unclosed() {
        let (events, diagnostics) = run("［＃［＃［＃");
        // Filter Unclosed events out: they should be the last three
        // events of the stream (after Open/Solo/Open/Solo/Open/Solo).
        let unclosed: Vec<&PairEvent> = events
            .iter()
            .filter(|e| matches!(e, PairEvent::Unclosed { .. }))
            .collect();
        assert_eq!(unclosed.len(), 3, "events were {events:?}");

        // The opens have monotonically increasing source start
        // positions; the EOF drain pops innermost (last-pushed) first, so
        // the span start of the first Unclosed event must be the largest
        // of the three (innermost = last in source order).
        let starts: Vec<u32> = unclosed
            .iter()
            .map(|e| e.span().expect("Unclosed has a span").start)
            .collect();
        assert!(
            starts[0] > starts[1] && starts[1] > starts[2],
            "EOF drain order should be innermost-first; got starts={starts:?}"
        );

        // Diagnostic ordering: same innermost-first, one per frame.
        let bracket_diag_count = diagnostics
            .iter()
            .filter(|d| matches!(d, Diagnostic::UnclosedBracket { .. }))
            .count();
        assert_eq!(bracket_diag_count, 3);
    }

    /// `take_diagnostics` on a partly-driven stream returns whatever
    /// has accumulated so far (could be 0); the same call after the
    /// stream is exhausted must return the empty Vec because the prior
    /// drain emptied the buffer.
    #[test]
    fn pair_stream_take_diagnostics_only_complete_after_exhaustion() {
        let mut stream = pair(tokenize("stray］more text［＃tail"));
        // Drive partway: pull 4 events. The unmatched `］` close
        // produces one diagnostic eagerly; the unclosed `［＃` is
        // reported only after EOF.
        for _ in 0..4 {
            let _ = stream.next();
        }
        let mid = stream.take_diagnostics();
        // 0 or more diagnostics: the exact count depends on tokenisation;
        // the test requires only that the call does not panic and
        // returns what was accumulated so far.
        let _ = mid.len(); // observably non-panicking access

        while stream.next().is_some() {}
        let after = stream.take_diagnostics();
        // Diagnostics drained at `mid` are removed; diagnostics emitted
        // after the first `take_diagnostics` (e.g. the EOF unclosed) are
        // in `after`. `take_diagnostics` drains, so a second immediate
        // take must yield an empty Vec.
        let again = stream.take_diagnostics();
        assert!(
            again.is_empty(),
            "second take_diagnostics must return empty after the prior drain, got {again:?}"
        );
        // At least one diagnostic was emitted across the two drains (the
        // unclosed-bracket synthesis), so the assertion above tests drain
        // behaviour, not an absence of diagnostics.
        assert!(
            !after.is_empty() || mid.iter().any(|_| true),
            "expected at least one diagnostic across the two drains for this input"
        );
    }

    /// A purely textual input emits exactly one `Text` event covering
    /// every byte. Exercises the tokenize → pair pass-through path.
    #[test]
    fn pair_stream_text_event_byte_coverage() {
        let (events, diagnostics) = run("abcdef");
        assert_eq!(events.len(), 1, "got {events:?}");
        match events[0] {
            PairEvent::Text { range } => {
                assert_eq!(range, Span::new(0, 6));
            }
            ref other => panic!("expected single Text event, got {other:?}"),
        }
        assert!(diagnostics.is_empty());
    }

    proptest! {
        /// Output is a pure function of input: running the same source
        /// twice must produce identical event sequences.
        #[test]
        fn proptest_pair_is_deterministic(src in source_strategy()) {
            let (a, _) = run(&src);
            let (b, _) = run(&src);
            prop_assert_eq!(a, b);
        }

        /// Every PairOpen of `kind` is eventually balanced either by a
        /// matching PairClose of the same `kind` or by an Unclosed of the
        /// same `kind`. No "lost" opens.
        #[test]
        fn proptest_every_open_resolves(src in source_strategy()) {
            let (events, _) = run(&src);
            // Replay the stream maintaining a stack: every push must be
            // matched by a Close or an Unclosed of the same kind.
            let mut stack: Vec<PairKind> = Vec::new();
            for ev in &events {
                match *ev {
                    PairEvent::PairOpen { kind, .. } => stack.push(kind),
                    PairEvent::PairClose { kind, .. } => {
                        let top = stack.pop();
                        prop_assert_eq!(top, Some(kind));
                    }
                    PairEvent::Unclosed { kind, .. } => {
                        let top = stack.pop();
                        prop_assert_eq!(top, Some(kind));
                    }
                    _ => {}
                }
            }
            prop_assert!(stack.is_empty(), "leftover opens in stack: {stack:?}");
        }
    }

    fn source_strategy() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop_oneof![
                Just('a'),
                Just('あ'),
                Just('漢'),
                Just('｜'),
                Just('《'),
                Just('》'),
                Just('［'),
                Just('］'),
                Just('＃'),
                Just('※'),
                Just('〔'),
                Just('〕'),
                Just('「'),
                Just('」'),
                Just('\n'),
            ],
            0..40,
        )
        .prop_map(|chars| chars.into_iter().collect())
    }
}
