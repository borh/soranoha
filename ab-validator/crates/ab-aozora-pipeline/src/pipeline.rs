//! Type-state lex pipeline.
//!
//! `Pipeline<'src, S>` makes the lex stage order enforceable at compile time.
//! The state markers [`Source`], [`Sanitized`], [`Tokenized`], [`Paired`] track
//! which stages have run; methods consume `self` and return the next state.
//! Calling `.pair()` on a `Source` is a type error; calling `.tokenize()` twice
//! is a type error; etc.
//!
//! # Two entry shapes
//!
//! - [`Pipeline::run_to_completion`]: one-shot, equivalent to [`crate::lex`].
//!   Used by `Document::parse` and the FFI / WASM / Python drivers.
//! - [`Pipeline::new`] → `.sanitize()` → `.tokenize()` → `.pair()` →
//!   `.build()`: explicit chain. Use for inspection / instrumentation: each
//!   intermediate state exposes accessors (`.sanitized_text()`, `.tokens()`,
//!   `.events()`, `.diagnostics()`) so callers can probe the partial output
//!   without re-running the pipeline.
//!
//! # State carries its own payload
//!
//! Each state marker is a field-bound struct holding exactly the stage outputs
//! it has produced (`Sanitized` carries the sanitized `String`; `Tokenized`
//! adds the token `Vec`; …). Reading `.sanitized_text()` from
//! `Pipeline<'_, Sanitized>` is a field projection on the state struct, with
//! no `Option::expect`.
//!
//! # Owned data
//!
//! Every stage operates on owned data: the sanitized text is an owned `String`,
//! the token / event lists are `Vec`s, and the classify stage builds the owned
//! AST directly into an [`Allocator`]'s [`NodeStore`], which is moved into the
//! returned [`LexOutput`].
//!
//! # `build` is the terminal transition
//!
//! The classify stage requires `&mut Allocator`, so the classify stage and the
//! normalize fold run together in the terminal `.build()` call. Each state
//! through `Paired` can be inspected; the final pass is atomic.

use std::collections::BTreeSet;

use crate::lexer::{
    ClassifiedSpan, PairEvent, PlainProvenance, PlainSpan, SpanKind, Token, classify, pair,
    sanitize, tokenize,
};
use ab_aozora_encoding::gaiji::{Resolved, parse_gaiji_body};
use ab_aozora_spec::{Diagnostic, PairKind, PairLink};

use ab_aozora_syntax::accent::{
    compose_accent_dots, decompose_fragment_sites, unresolved_accent_sites,
};
use ab_aozora_syntax::alloc::Allocator;
use ab_aozora_syntax::ast::canonicalize_classified_source_facts;
use ab_aozora_syntax::ast::{
    Content, Gaiji, LexOutput, MarginNotePair, Node, NodeStore, NonEmptySpan, Registry, Segment,
};
use ab_aozora_syntax::format::ForwardOrigin;
use ab_aozora_syntax::{DirectiveKind, ForwardAttr, GaijiMarkerPlacement, RubySide, Span};
use std::collections::BTreeMap;

use crate::fold::{Normalizer, scope_closer_matches};
use crate::lexer::classify::encoded_retrospective_gaiji_target;

// State markers (field-bound: each state carries the stage output it is
// responsible for).

/// Initial state: no stage has run yet.
#[derive(Debug, Clone, Copy)]
pub struct Source;

/// The sanitize stage has run; the sanitized text is owned.
#[derive(Debug, Clone)]
pub struct Sanitized {
    sanitized_text: String,
}

/// The tokenize stage has run; the token list is materialised.
#[derive(Debug)]
pub struct Tokenized {
    sanitized_text: String,
    tokens: Vec<Token>,
}

/// The pair stage has run; the event list and the resolved (open, close) link
/// side-table are materialised.
#[derive(Debug)]
pub struct Paired {
    sanitized_text: String,
    events: Vec<PairEvent>,
    links: Vec<PairLink>,
}

/// Type-state lex pipeline. Each state's transition method consumes `self`,
/// materialises its stage output into the next state struct, and returns a new
/// pipeline in the next state.
#[derive(Debug)]
pub struct Pipeline<'src, S> {
    source: &'src str,
    diagnostics: Vec<Diagnostic>,
    accent_scopes: Vec<Span>,
    state: S,
}

impl<'src> Pipeline<'src, Source> {
    /// Wrap a source string for type-state-driven lex. The sanitize stage has
    /// not yet run; only `source` is set.
    #[must_use]
    pub fn new(source: &'src str) -> Self {
        Self {
            source,
            diagnostics: Vec::new(),
            accent_scopes: Vec::new(),
            state: Source,
        }
    }

    /// One-shot driver: run every stage and return the final [`LexOutput`].
    /// Equivalent to [`crate::lex`].
    #[must_use]
    pub fn run_to_completion(source: &'src str) -> LexOutput {
        Self::new(source).sanitize().tokenize().pair().build()
    }

    /// Borrow the original source text.
    #[must_use]
    pub fn source(&self) -> &'src str {
        self.source
    }

    /// Run the sanitize stage, materialising the sanitized text as an owned
    /// `String`.
    #[must_use]
    pub fn sanitize(mut self) -> Pipeline<'src, Sanitized> {
        let out = sanitize(self.source);
        self.diagnostics.extend(out.diagnostics);
        Pipeline {
            source: self.source,
            diagnostics: self.diagnostics,
            accent_scopes: out.accent_scopes,
            state: Sanitized {
                sanitized_text: out.text.into_owned(),
            },
        }
    }
}

impl<'src> Pipeline<'src, Sanitized> {
    /// Parse text whose caller already owns sanitization and its source maps.
    /// Token and source-node spans index these exact bytes. The caller retains
    /// any diagnostics produced by its sanitization pass and supplies realized
    /// accent-scope extents rebased onto these bytes.
    #[must_use]
    pub fn from_sanitized(source: &'src str, accent_scopes: Vec<Span>) -> Self {
        Self {
            source,
            diagnostics: Vec::new(),
            accent_scopes,
            state: Sanitized {
                sanitized_text: source.to_owned(),
            },
        }
    }

    /// Sanitized text.
    #[must_use]
    pub fn sanitized_text(&self) -> &str {
        &self.state.sanitized_text
    }

    /// Diagnostics accumulated through the sanitize stage.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Run the tokenize stage, materialising the token list.
    #[must_use]
    pub fn tokenize(self) -> Pipeline<'src, Tokenized> {
        let tokens: Vec<Token> = tokenize(&self.state.sanitized_text).collect();
        Pipeline {
            source: self.source,
            diagnostics: self.diagnostics,
            accent_scopes: self.accent_scopes,
            state: Tokenized {
                sanitized_text: self.state.sanitized_text,
                tokens,
            },
        }
    }
}

impl<'src> Pipeline<'src, Tokenized> {
    /// Borrow the materialised token list. Useful for instrumentation.
    #[must_use]
    pub fn tokens(&self) -> &[Token] {
        &self.state.tokens
    }

    /// Run the pair stage, materialising a paired-event stream and the resolved
    /// link side-table. The pair stage's diagnostics are drained into the
    /// pipeline's diagnostic accumulator immediately.
    #[must_use]
    pub fn pair(mut self) -> Pipeline<'src, Paired> {
        let Tokenized {
            sanitized_text,
            tokens,
        } = self.state;
        let mut pair_stream = pair(tokens.into_iter());
        let events: Vec<PairEvent> = (&mut pair_stream).collect();
        self.diagnostics.extend(pair_stream.take_diagnostics());
        let links = pair_stream.take_links();
        Pipeline {
            source: self.source,
            diagnostics: self.diagnostics,
            accent_scopes: self.accent_scopes,
            state: Paired {
                sanitized_text,
                events,
                links,
            },
        }
    }
}

impl Pipeline<'_, Paired> {
    /// Borrow the materialised pair-event list. Useful for inspection before
    /// `.build()`.
    #[must_use]
    pub fn events(&self) -> &[PairEvent] {
        &self.state.events
    }

    /// Borrow the resolved (open, close) pair side-table.
    #[must_use]
    pub fn links(&self) -> &[PairLink] {
        &self.state.links
    }

    /// Complete multiline accent scopes retained as source text rather than decoded.
    /// Ordinary tortoiseshell punctuation without accent digraphs is excluded.
    #[must_use]
    pub fn retained_multiline_accent_spans(&self) -> Vec<Span> {
        let mut annotations = self
            .state
            .links
            .iter()
            .filter(|link| {
                link.kind == PairKind::Bracket
                    && self.state.sanitized_text[link.open.start as usize..].starts_with("［＃")
            })
            .map(|link| link.open.start as usize..link.close.end as usize)
            .collect::<Vec<_>>();
        annotations.sort_unstable_by_key(|range| range.start);
        let non_text = sanitize::accent_non_text_ranges(
            &self.state.sanitized_text,
            &annotations,
            &self.state.links,
        );
        self.state
            .links
            .iter()
            .filter_map(|link| {
                if link.kind != PairKind::Tortoise {
                    return None;
                }
                let body =
                    &self.state.sanitized_text[link.open.end as usize..link.close.start as usize];
                if sanitize::scope_fragments(
                    link.open.end as usize,
                    link.close.start as usize,
                    &annotations,
                )
                .iter()
                .any(|range| self.state.sanitized_text[range.clone()].contains('〔'))
                {
                    return None;
                }
                (body.contains(['\r', '\n'])
                    && (!decompose_fragment_sites(body).is_empty()
                        || (!unresolved_accent_sites(body).is_empty()
                            && sanitize::accent_scope_has_latin_text(
                                &self.state.sanitized_text,
                                link.open.end as usize,
                                link.close.start as usize,
                                &non_text,
                            ))))
                .then(|| Span::new(link.open.start, link.close.end))
            })
            .collect()
    }

    /// Drive the classify stage + the owned normalizer fold and return the final
    /// [`LexOutput`]. Terminal transition.
    ///
    /// # Diagnostic order
    ///
    /// Sanitize stage → pair stage (unclosed/unmatched) → classify stage
    /// (unknown annotations etc.) → normalizer (mismatched container close).
    ///
    /// # Panics
    ///
    /// Panics if the sanitized source exceeds `u32::MAX` bytes (the lexer's
    /// `Span` width contract). In practice unreachable.
    #[must_use]
    pub fn build(mut self) -> LexOutput {
        let Paired {
            sanitized_text,
            events,
            links,
        } = self.state;
        let sanitized_len =
            u32::try_from(sanitized_text.len()).expect("sanitized source exceeds u32::MAX bytes");

        let mut alloc = Allocator::new();

        let (normalized, recorder, container_pairs, classify_diagnostics, norm_diagnostics, store) = {
            let mut normalizer = Normalizer::new(&sanitized_text, sanitized_text.len() / 64);

            // Drain the pair events through the streaming `classify` Iterator
            // path; collect the classified spans and the classify diagnostics,
            // then drop the stream so its `&mut alloc` borrow is released before
            // the normalize (lowering) pass creates its canonical core nodes.
            let mut events_iter = events.into_iter();
            let mut classify_stream = classify(&mut events_iter, &sanitized_text, &mut alloc);
            let mut spans: Vec<ClassifiedSpan> = (&mut classify_stream).collect();
            let mut classify_diagnostics: Vec<Diagnostic> = classify_stream.take_diagnostics();
            drop(classify_stream);
            resolve_margin_note_pairs(&mut spans, &sanitized_text, &mut alloc);
            spans = apply_detached_formatted_gaiji_declarations(spans, &sanitized_text, &mut alloc);
            spans = apply_persistent_gaiji_declarations(spans, &sanitized_text, &mut alloc);
            spans = apply_ruled_box_gaiji_declarations(spans, &sanitized_text, &mut alloc);
            spans = apply_retrospective_accent_dots(spans, &sanitized_text, &mut alloc);
            spans = resolve_accent_scope_dots(
                spans,
                &sanitized_text,
                &mut alloc,
                &links,
                &self.accent_scopes,
            );
            let (mut lowered, ruby_base_decorated) =
                lower_spans(spans, &sanitized_text, &mut alloc);
            resolve_adjacent_note_targets(&mut lowered, &sanitized_text, alloc.store(), &links);
            resolve_adjacent_ruby_dots(&mut lowered, &sanitized_text, &mut alloc, &links);
            classify_diagnostics.retain(|diagnostic| {
                classify_diagnostic_survives_lowering(diagnostic, &lowered, &ruby_base_decorated)
            });
            for span in &lowered {
                normalizer.emit(span);
            }
            normalizer.finish();
            // Move the owned products out, ending the normalizer's borrow of
            // `sanitized_text` so it can be moved into the output below.
            let Normalizer {
                out,
                recorder,
                container_pairs,
                diagnostics: norm_diagnostics,
                ..
            } = normalizer;
            let store = alloc.into_store();
            (
                out,
                recorder,
                container_pairs,
                classify_diagnostics,
                norm_diagnostics,
                store,
            )
        };

        // Classify-stage diagnostics first, then the normalizer's (post-classify)
        // set, so the final vector stays in pipeline-stage order.
        self.diagnostics.extend(classify_diagnostics);
        self.diagnostics.extend(norm_diagnostics);
        let intern_stats = store.interner.stats;

        // Classifier emits in source order, so the recorder's entries are already
        // sorted by position; `from_sorted_slice` skips the redundant sort.
        let registry = Registry::from_sorted_slice(&recorder.entries);
        let classified_source_facts =
            canonicalize_classified_source_facts(recorder.classified_source_facts);

        LexOutput::new(
            normalized,
            sanitized_text,
            registry,
            self.diagnostics,
            sanitized_len,
            links,
            recorder.source_nodes,
            classified_source_facts,
            container_pairs,
            intern_stats,
            store,
        )
    }
}

fn resolve_margin_note_pairs(spans: &mut [ClassifiedSpan], source: &str, alloc: &mut Allocator) {
    use ab_aozora_syntax::{MarginNoteKind, MarginNotePosition};

    let mut openers = Vec::<(usize, Option<MarginNotePosition>)>::new();
    for close_index in 0..spans.len() {
        let SpanKind::Aozora(Node::Directive(directive)) = spans[close_index].kind else {
            continue;
        };
        if directive.kind == DirectiveKind::MarginNotePairOpen {
            let raw = alloc.store().resolve_str(directive.raw);
            let position = raw
                .starts_with("［＃左に")
                .then_some(MarginNotePosition::Left);
            openers.push((close_index, position));
            continue;
        }
        if directive.kind != DirectiveKind::MarginNotePairClose {
            continue;
        }
        let Some(payload) = directive.pair_payload else {
            continue;
        };
        let raw = alloc.store().resolve_str(directive.raw);
        let position = raw
            .starts_with("［＃左に")
            .then_some(MarginNotePosition::Left);
        let Some(&(open_index, open_position)) = openers.last() else {
            continue;
        };
        if open_position != position {
            openers.clear();
            continue;
        }
        let Some(target_span) = NonEmptySpan::new(Span::new(
            spans[open_index].source_span.end,
            spans[close_index].source_span.start,
        )) else {
            continue;
        };
        let target = target_span.span();
        let target_source = &source[target.start as usize..target.end as usize];
        if target_source.contains('\n') {
            continue;
        }
        let [note] = alloc.store().resolve_content_range(payload.content) else {
            continue;
        };
        let note = *note;
        let base = alloc.content_plain(target_source);
        let mut resolved = alloc.side_note(
            MarginNoteKind::Gloss,
            position,
            base,
            note,
            Some(payload.source_span.span()),
        );
        let Node::MarginNote(ref mut margin_note) = resolved else {
            unreachable!("side_note returns a margin note")
        };
        margin_note.target_span = Some(target_span);
        margin_note.pair = NonEmptySpan::new(spans[open_index].source_span).map(|open_span| {
            alloc.margin_note_pair(MarginNotePair {
                open_span,
                close_raw: directive.raw,
            })
        });
        margin_note.origin = ForwardOrigin::Referenced;
        openers.pop();
        spans[close_index].kind = SpanKind::Aozora(resolved);
        if let SpanKind::Aozora(Node::Directive(ref mut opener)) = spans[open_index].kind {
            opener.kind = DirectiveKind::MarginNotePairOpenResolved;
        }
    }
}

fn apply_retrospective_accent_dots(
    mut spans: Vec<ClassifiedSpan>,
    source: &str,
    alloc: &mut Allocator,
) -> Vec<ClassifiedSpan> {
    let Some(marker_index) = spans.iter().position(|span| {
        (match span.kind {
            SpanKind::Aozora(Node::Directive(directive)) => {
                directive.kind == DirectiveKind::Unknown
            }
            SpanKind::Aozora(Node::Gaiji(gaiji)) => gaiji.standalone && gaiji.replaced.is_none(),
            _ => false,
        }) && source
            .get(span.source_span.start as usize..span.source_span.end as usize)
            .and_then(|raw| raw.strip_prefix("［＃"))
            .and_then(|body| body.strip_suffix('］'))
            .is_some_and(|body| compose_accent_dots("tair", body).as_deref() == Some("ṭair"))
    }) else {
        return spans;
    };
    let marker_span = spans[marker_index].source_span;
    let raw = &source[marker_span.start as usize..marker_span.end as usize];
    let Some(body) = raw
        .strip_prefix("［＃")
        .and_then(|body| body.strip_suffix('］'))
    else {
        return spans;
    };
    let prefix = &source[..marker_span.start as usize];
    let run_end = prefix.trim_end().strip_suffix('.').map(str::len);
    let Some(run_end) = run_end else {
        return spans;
    };
    let run_start = prefix[..run_end]
        .char_indices()
        .rev()
        .find(|(_, ch)| !ch.is_ascii_alphabetic() && *ch != '-')
        .map_or(0, |(index, ch)| index + ch.len_utf8());
    let run = &source[run_start..run_end];
    if compose_accent_dots(run, body).is_none() {
        return spans;
    }
    let Some(plain_index) = spans[..marker_index].iter().rposition(|span| {
        matches!(span.kind, SpanKind::Plain(_))
            && span.source_span.start <= u32::try_from(run_start).expect("source offset fits u32")
            && u32::try_from(run_end).expect("source offset fits u32") <= span.source_span.end
    }) else {
        return spans;
    };
    let plain = spans.remove(plain_index);
    let marker_index = marker_index - usize::from(plain_index < marker_index);
    spans.remove(marker_index);
    let run_span = Span::new(
        u32::try_from(run_start).expect("source offset fits u32"),
        u32::try_from(run_end).expect("source offset fits u32"),
    );
    let target = alloc.content_plain(run);
    let Node::Format(detached) = alloc.accent_dot(target, body, ForwardOrigin::Detached) else {
        unreachable!()
    };
    let mut referenced = detached;
    referenced.origin = ForwardOrigin::Referenced;
    let mut replacements = Vec::with_capacity(5);
    if plain.source_span.start < run_span.start {
        replacements.push(ClassifiedSpan {
            kind: plain.kind.clone(),
            source_span: Span::new(plain.source_span.start, run_span.start),
        });
    }
    replacements.push(ClassifiedSpan {
        kind: SpanKind::Aozora(Node::Format(detached)),
        source_span: run_span,
    });
    if run_span.end < marker_span.start {
        replacements.push(ClassifiedSpan {
            kind: plain.kind,
            source_span: Span::new(run_span.end, marker_span.start),
        });
    }
    replacements.push(ClassifiedSpan {
        kind: SpanKind::Aozora(Node::Format(referenced)),
        source_span: marker_span,
    });
    spans.splice(plain_index..plain_index, replacements);
    spans
}

fn apply_detached_formatted_gaiji_declarations(
    mut spans: Vec<ClassifiedSpan>,
    source: &str,
    alloc: &mut Allocator,
) -> Vec<ClassifiedSpan> {
    const TARGET: &str = "大※";
    const FORMATTED_SOURCE: &str = "大※［＃「大※」に傍点］";
    let Some(marker_index) = spans.iter().position(|span| {
        let SpanKind::Aozora(Node::Gaiji(gaiji)) = span.kind else {
            return false;
        };
        gaiji.standalone
            && alloc.store().resolve_str(gaiji.hint) == "郷－即のへん"
            && gaiji.resolve(alloc.store()) == Some(Resolved::Char('\u{28675}'))
    }) else {
        return spans;
    };
    let marker_span = spans[marker_index].source_span;
    let Some(format_index) = marker_index.checked_sub(1) else {
        return spans;
    };
    let SpanKind::Aozora(Node::Format(mut format)) = spans[format_index].kind else {
        return spans;
    };
    let format_span = spans[format_index].source_span;
    if format_span.end != marker_span.start
        || source.get(format_span.start as usize..format_span.end as usize)
            != Some(FORMATTED_SOURCE)
        || alloc.store().content_range_as_plain(format.target) != Some(TARGET)
    {
        return spans;
    }
    let SpanKind::Aozora(Node::Gaiji(declaration)) = spans[marker_index].kind else {
        unreachable!("marker predicate selected a gaiji");
    };
    let target_start = format_span.start + u32::try_from("大".len()).expect("literal fits u32");
    let target_span = Span::new(
        target_start,
        target_start + u32::try_from('※'.len_utf8()).expect("literal fits u32"),
    );
    let replaced = alloc.content_plain("※");
    let gaiji = alloc.gaiji_payload_declared_at(declaration, replaced, marker_span);
    let segments = [
        alloc.seg_text("大"),
        alloc.seg_gaiji(
            gaiji,
            target_span,
            marker_span,
            GaijiMarkerPlacement::Detached,
        ),
    ];
    let content = alloc.content_segments(&segments);
    format.target = alloc.content_range(content);
    format.origin = ForwardOrigin::Referenced;
    let Content::Plain(annotation_body) = alloc.content_plain("「大※」に傍点") else {
        unreachable!("a nonempty annotation body is plain content");
    };
    format.annotation_body = Some(annotation_body);
    let formatting_start = target_span.end;
    spans.splice(
        format_index..=format_index,
        [
            ClassifiedSpan {
                kind: SpanKind::Plain(PlainSpan {
                    provenance: PlainProvenance::Text,
                }),
                source_span: Span::new(format_span.start, target_span.start),
            },
            ClassifiedSpan {
                kind: SpanKind::Aozora(alloc.gaiji(gaiji)),
                source_span: target_span,
            },
            ClassifiedSpan {
                kind: SpanKind::Aozora(Node::Format(format)),
                source_span: Span::new(formatting_start, format_span.end),
            },
        ],
    );
    let marker_index = marker_index + 2;
    let raw = &source[marker_span.start as usize..marker_span.end as usize];
    spans[marker_index].kind = SpanKind::Aozora(Node::Directive(
        alloc.make_directive(raw, DirectiveKind::GaijiDeclaration),
    ));
    spans
}

#[derive(Clone, Copy)]
struct PersistentGaijiSpec {
    target: &'static str,
    target_len: u32,
    first_target_start: u32,
    marker: Span,
    gaiji: Gaiji,
}

fn apply_persistent_gaiji_declarations(
    mut spans: Vec<ClassifiedSpan>,
    source: &str,
    alloc: &mut Allocator,
) -> Vec<ClassifiedSpan> {
    let specs = spans
        .iter()
        .filter_map(|span| {
            let SpanKind::Aozora(Node::Gaiji(gaiji)) = span.kind else {
                return None;
            };
            if !gaiji.standalone || gaiji.replaced.is_some() {
                return None;
            }
            let target = match (
                alloc.store().resolve_str(gaiji.hint),
                gaiji.resolve(alloc.store()),
            ) {
                ("この二重括弧は一文字、以下同様に使用", Some(Resolved::Char('｟'))) => {
                    "（（"
                }
                ("この二重括弧は一文字、以下同様に使用", Some(Resolved::Char('｠'))) => {
                    "））"
                }
                _ => return None,
            };
            let target_len = u32::try_from(target.len()).ok()?;
            let first_target_start = span.source_span.start.checked_sub(target_len)?;
            (source.get(first_target_start as usize..span.source_span.start as usize)
                == Some(target))
            .then_some(PersistentGaijiSpec {
                target,
                target_len,
                first_target_start,
                marker: span.source_span,
                gaiji,
            })
        })
        .collect::<Vec<_>>();

    for spec in specs {
        let target_starts = source
            .match_indices(spec.target)
            .filter_map(|(start, _)| {
                let start = u32::try_from(start).ok()?;
                (start == spec.first_target_start || start >= spec.marker.end).then_some(start)
            })
            .collect::<Vec<_>>();
        for target_start in target_starts {
            let target = Span::new(target_start, target_start + spec.target_len);
            let Some(index) = spans.iter().position(|span| {
                matches!(span.kind, SpanKind::Plain(_))
                    && span.source_span.start <= target.start
                    && target.end <= span.source_span.end
            }) else {
                continue;
            };
            let plain = spans.remove(index);
            let mut replacements = Vec::with_capacity(3);
            if plain.source_span.start < target.start {
                replacements.push(ClassifiedSpan {
                    kind: plain.kind.clone(),
                    source_span: Span::new(plain.source_span.start, target.start),
                });
            }
            let replaced = alloc.content_plain(spec.target);
            let gaiji = alloc.gaiji_payload_declared_at(spec.gaiji, replaced, spec.marker);
            replacements.push(ClassifiedSpan {
                kind: SpanKind::Aozora(alloc.gaiji(gaiji)),
                source_span: target,
            });
            if target.end < plain.source_span.end {
                replacements.push(ClassifiedSpan {
                    kind: plain.kind,
                    source_span: Span::new(target.end, plain.source_span.end),
                });
            }
            spans.splice(index..index, replacements);
        }
        if let Some(marker) = spans.iter_mut().find(|span| {
            span.source_span == spec.marker && matches!(span.kind, SpanKind::Aozora(Node::Gaiji(_)))
        }) {
            let raw = &source[spec.marker.start as usize..spec.marker.end as usize];
            marker.kind = SpanKind::Aozora(Node::Directive(
                alloc.make_directive(raw, DirectiveKind::GaijiDeclaration),
            ));
        }
    }
    spans
}

/// `［＃罫内の「※」は「※［＃…］」］` on the line after a ruled box declares the
/// glyph of the one `※` inside that box: the `※` becomes the declared gaiji
/// and the note its declaration.
fn apply_ruled_box_gaiji_declarations(
    mut spans: Vec<ClassifiedSpan>,
    source: &str,
    alloc: &mut Allocator,
) -> Vec<ClassifiedSpan> {
    let notes = spans
        .iter()
        .filter_map(|span| {
            let SpanKind::Aozora(Node::Directive(directive)) = span.kind else {
                return None;
            };
            let raw = source.get(span.source_span.start as usize..span.source_span.end as usize)?;
            let body = raw
                .strip_prefix("［＃罫内の「※」は「※［＃")?
                .strip_suffix("］」］")?;
            (directive.kind == DirectiveKind::Unknown && !body.contains(['［', '］']))
                .then_some((span.source_span, body))
        })
        .collect::<Vec<_>>();
    for (marker, body) in notes {
        let Some(target) = ruled_box_refmark(source, marker.start as usize) else {
            continue;
        };
        let Some(index) = spans.iter().position(|span| {
            matches!(span.kind, SpanKind::Plain(_))
                && span.source_span.start <= target.start
                && target.end <= span.source_span.end
        }) else {
            continue;
        };
        let reference = parse_gaiji_body(body);
        if reference.description.is_empty() {
            continue;
        }
        let declaration = alloc.make_gaiji(
            reference.description,
            reference.quoted,
            reference.mencode,
            true,
        );
        let plain = spans.remove(index);
        let mut replacements = Vec::with_capacity(3);
        if plain.source_span.start < target.start {
            replacements.push(ClassifiedSpan {
                kind: plain.kind.clone(),
                source_span: Span::new(plain.source_span.start, target.start),
            });
        }
        let replaced = alloc.content_plain("※");
        let gaiji = alloc.gaiji_payload_declared_at(declaration, replaced, marker);
        replacements.push(ClassifiedSpan {
            kind: SpanKind::Aozora(alloc.gaiji(gaiji)),
            source_span: target,
        });
        if target.end < plain.source_span.end {
            replacements.push(ClassifiedSpan {
                kind: plain.kind,
                source_span: Span::new(target.end, plain.source_span.end),
            });
        }
        spans.splice(index..index, replacements);
        if let Some(note) = spans.iter_mut().find(|span| span.source_span == marker) {
            let raw = &source[marker.start as usize..marker.end as usize];
            note.kind = SpanKind::Aozora(Node::Directive(
                alloc.make_directive(raw, DirectiveKind::GaijiDeclaration),
            ));
        }
    }
    spans
}

/// The one `※` inside the ruled box that ends on the line before the note at
/// `note_start`; `None` unless only whitespace precedes the note on its line
/// and the box holds exactly one `※`.
fn ruled_box_refmark(source: &str, note_start: usize) -> Option<Span> {
    let line_start = source[..note_start]
        .rfind('\n')
        .map_or(0, |offset| offset + 1);
    if !source[line_start..note_start].trim().is_empty() {
        return None;
    }
    let mut lines = source[..line_start.checked_sub(1)?].rsplit('\n');
    let bottom = lines.next()?;
    let mut box_start = line_start - 1 - bottom.len();
    let bottom = bottom.trim();
    if !(bottom.starts_with('└') && bottom.ends_with('┘')) {
        return None;
    }
    loop {
        let line = lines.next()?;
        box_start = box_start.checked_sub(1 + line.len())?;
        let line = line.trim();
        if line.starts_with('┌') && line.ends_with('┐') {
            break;
        }
        if !(line.starts_with('│') && line.ends_with('│')) {
            return None;
        }
    }
    let mut refmarks = source[box_start..line_start].match_indices('※');
    let (offset, mark) = refmarks.next()?;
    if refmarks.next().is_some() {
        return None;
    }
    let start = u32::try_from(box_start + offset).ok()?;
    Some(Span::new(start, start + u32::try_from(mark.len()).ok()?))
}

/// Whether a classify diagnostic still describes the lowered output: lowering
/// can supply an accent dot for an unresolved gaiji or reclaim a forward referent.
fn classify_diagnostic_survives_lowering(
    diagnostic: &Diagnostic,
    lowered: &[ClassifiedSpan],
    ruby_base_decorated: &[Span],
) -> bool {
    if matches!(diagnostic, Diagnostic::UnresolvedGaiji { .. }) {
        let diagnostic_span = diagnostic.span();
        let supplied_diacritic = lowered.iter().any(|span| {
            span.source_span == diagnostic_span
                && matches!(
                    span.kind,
                    SpanKind::Aozora(Node::Format(format))
                        if format.origin == ForwardOrigin::Referenced
                            && format.attrs.single() == Some(ForwardAttr::AccentDot)
                )
        });
        if supplied_diacritic {
            return false;
        }
    }
    if !matches!(diagnostic, Diagnostic::ForwardReferentNotStylable { .. }) {
        return true;
    }
    let diagnostic_span = diagnostic.span();
    let reclaimed = lowered.iter().any(|span| {
        matches!(
            &span.kind,
            SpanKind::Aozora(Node::Format(format))
                if format.origin == ForwardOrigin::Reclaimed
                    && span.source_span.start < diagnostic_span.start
                    && span.source_span.end == diagnostic_span.end
        )
    });
    !reclaimed && !ruby_base_decorated.contains(&diagnostic_span)
}

/// Normalize (lowering) pass over the materialized classified-span list.
///
/// The pass drops superset spans by source byte range: when a later span's
/// source span is a proper superset of an earlier one (a backward pull-back,
/// e.g. a promoted 大/中/小 heading reclaiming its referent line `序章\n`, or a
/// forward node reclaiming its predecessor literal), the subsumed earlier span
/// is dropped, so the normalizer (which appends in source order) does not emit
/// the reclaimed text twice. Overlap truncation keeps the round trip stable;
/// the [`ForwardOrigin`] retained on each forward leaf is necessary provenance.
///
/// Returns the lowered spans and the set of forward-directive spans the
/// ruby-base emphasis step decorated; the builder suppresses those
/// directives' `forward_referent_not_stylable` warnings.
fn lower_spans(
    spans: Vec<ClassifiedSpan>,
    source: &str,
    alloc: &mut Allocator,
) -> (Vec<ClassifiedSpan>, Vec<Span>) {
    // Attach a declaration immediately after a ruby to the final character
    // of its reading when the source, descriptor, and resolved value agree.
    let spans = adopt_postposed_reading_gaiji(spans, source, alloc);
    // Resolve forward heading hints whose referent is the bare line
    // directly above the directive into promoted `Heading` nodes.
    let spans = promote_headings(spans, source, alloc);
    let mut out: Vec<ClassifiedSpan> = Vec::with_capacity(spans.len());
    for mut span in spans {
        while let Some(back) = out.last() {
            let (bs, be) = (back.source_span.start, back.source_span.end);
            let back_is_plain = matches!(back.kind, SpanKind::Plain(_));
            let (ss, se) = (span.source_span.start, span.source_span.end);
            if ss <= bs && be <= se && (ss < bs || se > be) {
                // Full superset. `span` reclaimed all of `back` (for example, a promoted
                // heading subsuming its referent line). Drop `back`.
                out.pop();
            } else if back_is_plain && bs < ss && ss < be {
                // Partial overlap. When `span` (a `Reclaimed` forward node) pulls its
                // source region back into the tail of a committed plain run,
                // truncate the plain run so the literal is emitted once by the node,
                // preventing duplicate emission and unbounded growth.
                if let Some(last) = out.last_mut() {
                    last.source_span.end = ss;
                }
                break;
            } else {
                break;
            }
        }
        resolve_relative_text_anchor(&mut span, &out, source, alloc.store());
        out.push(span);
    }
    // Apply a declined forward emphasis to the preceding ruby base that is
    // its unique referent.
    let decorated = decorate_ruby_bases(&mut out, source, alloc.store());
    resolve_warichu_closer_aliases(&mut out);
    (out, decorated)
}

fn adopt_postposed_reading_gaiji(
    mut spans: Vec<ClassifiedSpan>,
    source: &str,
    alloc: &mut Allocator,
) -> Vec<ClassifiedSpan> {
    let mut index = 0;
    while index + 1 < spans.len() {
        let (ruby, gaiji) = match (&spans[index].kind, &spans[index + 1].kind) {
            (SpanKind::Aozora(Node::Ruby(ruby)), SpanKind::Aozora(Node::Gaiji(gaiji)))
                if ruby.side == RubySide::Right
                    && gaiji.standalone
                    && gaiji.replaced.is_none()
                    && spans[index].source_span.end == spans[index + 1].source_span.start =>
            {
                (*ruby, *gaiji)
            }
            _ => {
                index += 1;
                continue;
            }
        };
        let hint = alloc.store().resolve_str(gaiji.hint);
        let Some(target) = encoded_retrospective_gaiji_target(hint, gaiji.resolve(alloc.store()))
            .map(str::to_owned)
        else {
            index += 1;
            continue;
        };
        let Some(reading) = alloc.store().content_range_as_plain(ruby.reading) else {
            index += 1;
            continue;
        };
        if !reading.ends_with(&target) {
            index += 1;
            continue;
        }
        let ruby_end = spans[index].source_span.end as usize;
        let Some(reading_end) = ruby_end.checked_sub('》'.len_utf8()) else {
            index += 1;
            continue;
        };
        let Some(target_start) = reading_end.checked_sub(target.len()) else {
            index += 1;
            continue;
        };
        if source.get(target_start..reading_end) != Some(target.as_str())
            || source.get(reading_end..ruby_end) != Some("》")
        {
            index += 1;
            continue;
        }

        let reading_prefix = reading[..reading.len() - target.len()].to_owned();
        let replaced = alloc.content_plain(&target);
        let gaiji = alloc.gaiji_payload_replacing(gaiji, replaced);
        let target_span = Span::new(
            u32::try_from(target_start).expect("source offset fits u32"),
            u32::try_from(reading_end).expect("source offset fits u32"),
        );
        let marker_span = spans[index + 1].source_span;
        let mut segments = Vec::with_capacity(2);
        if !reading_prefix.is_empty() {
            segments.push(alloc.seg_text(&reading_prefix));
        }
        segments.push(alloc.seg_gaiji(
            gaiji,
            target_span,
            marker_span,
            GaijiMarkerPlacement::AfterRuby,
        ));
        let reading = alloc.content_segments(&segments);
        if let SpanKind::Aozora(Node::Ruby(ref mut ruby)) = spans[index].kind {
            ruby.reading = alloc.content_range(reading);
        }
        spans[index].source_span.end = marker_span.end;
        spans.remove(index + 1);
        index += 1;
    }
    spans
}

/// A bare closer can end the nearest generic warichu scope. Bare pairs retain
/// their inline representation; intervening scopes cannot be crossed.
fn resolve_warichu_closer_aliases(spans: &mut [ClassifiedSpan]) {
    use ab_aozora_syntax::{DirectiveKind, RegionClose, RegionFormat};

    // None denotes a bare inline opener; Some retains a generic scope's kind.
    let mut openers = Vec::new();
    for span in spans {
        match span.kind {
            SpanKind::BlockOpen(kind) => openers.push(Some(kind)),
            SpanKind::BlockClose(close) => {
                if openers
                    .last()
                    .is_some_and(|open| open.is_some_and(|kind| scope_closer_matches(kind, close)))
                {
                    openers.pop();
                } else {
                    openers.clear();
                }
            }
            SpanKind::BlockCloses { targets, .. } => {
                if openers.len() >= targets.len()
                    && openers.iter().rev().zip(targets).all(|(open, close)| {
                        open.is_some_and(|kind| scope_closer_matches(kind, close))
                    })
                {
                    openers.truncate(openers.len() - targets.len());
                }
            }
            SpanKind::Aozora(Node::Directive(directive)) => match directive.kind {
                DirectiveKind::WarichuOpen | DirectiveKind::WarigakiOpen => openers.push(None),
                DirectiveKind::WarichuClose | DirectiveKind::WarigakiClose => {
                    match openers.last() {
                        Some(Some(RegionFormat::Warichu)) => {
                            span.kind = SpanKind::BlockClose(RegionClose::Warichu);
                            openers.pop();
                        }
                        Some(None) => {
                            openers.pop();
                        }
                        _ => {
                            openers.clear();
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

/// Retain normalization-owned targets without treating arbitrary punctuation as a word joiner.
fn resolve_accent_scope_dots(
    spans: Vec<ClassifiedSpan>,
    source: &str,
    alloc: &mut Allocator,
    links: &[PairLink],
    scopes: &[Span],
) -> Vec<ClassifiedSpan> {
    if scopes.is_empty() {
        return spans;
    }
    let mut scopes_by_end = BTreeMap::new();
    for scope in scopes {
        scopes_by_end.entry(scope.end).or_insert(scope);
    }
    let pairs: BTreeMap<_, _> = links.iter().map(|pair| (pair.close.end, pair)).collect();
    let mut out: Vec<ClassifiedSpan> = Vec::with_capacity(spans.len());
    for mut span in spans {
        let dot = matches!(span.kind, SpanKind::Aozora(Node::Format(f)) if f.attrs.single()==Some(ForwardAttr::AccentDot));
        let unknown = matches!(span.kind, SpanKind::Aozora(Node::Directive(d)) if d.kind==DirectiveKind::Unknown);
        if !(dot || unknown) {
            out.push(span);
            continue;
        }
        let Some(pair) = pairs
            .get(&span.source_span.end)
            .filter(|p| p.kind == PairKind::Bracket)
        else {
            out.push(span);
            continue;
        };
        let Some(scope) = scopes_by_end.get(&pair.open.start) else {
            out.push(span);
            continue;
        };
        let Some(body) =
            source[pair.open.end as usize..pair.close.start as usize].strip_prefix('＃')
        else {
            out.push(span);
            continue;
        };
        let plain_scope = out
            .iter()
            .rev()
            .take_while(|prior| prior.source_span.end > scope.start)
            .all(|prior| matches!(prior.kind, SpanKind::Plain(_) | SpanKind::Newline));
        let run = &source[scope.start as usize..scope.end as usize];
        if plain_scope && compose_accent_dots(run, body).is_some() {
            let target = alloc.content_plain(run);
            span.kind = SpanKind::Aozora(alloc.accent_dot(target, body, ForwardOrigin::Reclaimed));
            span.source_span.start = scope.start;
        } else if dot {
            use crate::lexer::classify::{PlainProvenance, PlainSpan};
            out.push(ClassifiedSpan {
                source_span: Span::new(span.source_span.start, pair.open.start),
                kind: SpanKind::Plain(PlainSpan {
                    provenance: PlainProvenance::Text,
                }),
            });
            span.source_span.start = pair.open.start;
            span.kind = SpanKind::Aozora(Node::Directive(
                alloc.make_directive(body, DirectiveKind::Unknown),
            ));
        }
        out.push(span);
    }
    out
}

/// Apply supplied dot selectors to an adjacent typed ruby base, never its reading.
fn resolve_adjacent_ruby_dots(
    spans: &mut [ClassifiedSpan],
    source: &str,
    alloc: &mut Allocator,
    links: &[PairLink],
) {
    if !spans.windows(2).any(|pair| {
        matches!(pair[0].kind, SpanKind::Aozora(Node::Ruby(_)))
            && matches!(pair[1].kind, SpanKind::Aozora(Node::Directive(_)))
    }) {
        return;
    }
    let pairs: BTreeMap<_, _> = links.iter().map(|pair| (pair.close.end, pair)).collect();
    for index in 1..spans.len() {
        let SpanKind::Aozora(Node::Directive(directive)) = spans[index].kind else {
            continue;
        };
        if directive.kind != DirectiveKind::Unknown {
            continue;
        }
        let SpanKind::Aozora(Node::Ruby(mut ruby)) = spans[index - 1].kind else {
            continue;
        };
        if spans[index - 1].source_span.end != spans[index].source_span.start {
            continue;
        }
        let (Some(ruby_pair), Some(marker)) = (
            pairs.get(&spans[index - 1].source_span.end),
            pairs.get(&spans[index].source_span.end),
        ) else {
            continue;
        };
        if ruby_pair.kind != PairKind::Ruby || marker.kind != PairKind::Bracket {
            continue;
        }
        let Some(body) =
            source[marker.open.end as usize..marker.close.start as usize].strip_prefix('＃')
        else {
            continue;
        };
        let Some(base) = alloc.store().content_range_as_plain(ruby.base) else {
            continue;
        };
        if compose_accent_dots(base, body).is_none() {
            continue;
        }
        let base = base.to_owned();
        let mut base_start = spans[index - 1].source_span.start;
        if source[base_start as usize..].starts_with('｜') {
            base_start += 3;
        }
        let target = alloc.content_plain(&base);
        let Node::Format(format) = alloc.accent_dot(target, body, ForwardOrigin::Detached) else {
            unreachable!()
        };
        let content = alloc.content_segments(&[Segment::Format {
            value: format,
            source_span: Span::new(base_start, ruby_pair.open.start),
        }]);
        ruby.base = alloc.content_range(content);
        spans[index - 1].kind = SpanKind::Aozora(Node::Ruby(ruby));
        let mut reference = format;
        reference.origin = ForwardOrigin::Referenced;
        spans[index].kind = SpanKind::Aozora(Node::Format(reference));
    }
}

/// Bind a unique literal in the immediately preceding nonempty source paragraph.
fn resolve_relative_text_anchor(
    span: &mut ClassifiedSpan,
    prior: &[ClassifiedSpan],
    source: &str,
    store: &NodeStore,
) {
    use ab_aozora_syntax::{RegionFormat, RelativePlacement};
    let SpanKind::BlockOpen(RegionFormat::RelativePlacement(RelativePlacement::BelowText {
        reference,
        offset_chars,
        ..
    })) = span.kind
    else {
        return;
    };
    let reference_text = store.resolve_str(reference);
    let before = &source[..span.source_span.start as usize];
    let trimmed = before.trim_end();
    let start = trimmed.rfind('\n').map_or(0, |at| at + 1);
    let mut matches = trimmed[start..].match_indices(reference_text);
    let Some((offset, _)) = matches.next() else {
        return;
    };
    if matches.next().is_some() {
        return;
    }
    let start = u32::try_from(start + offset).expect("native source fits u32");
    let end = start + u32::try_from(reference_text.len()).expect("native source fits u32");
    if !prior
        .iter()
        .rev()
        .take_while(|span| span.source_span.end > start)
        .any(|span| {
            matches!(span.kind, SpanKind::Plain(_))
                && span.source_span.start <= start
                && end <= span.source_span.end
        })
    {
        return;
    }
    span.kind = SpanKind::BlockOpen(RegionFormat::RelativePlacement(
        RelativePlacement::BelowText {
            reference,
            offset_chars,
            anchor: Some(Span::new(start, end)),
        },
    ));
}

/// Resolve source quotations within the immediately preceding typed base.
fn resolve_adjacent_note_targets(
    spans: &mut [ClassifiedSpan],
    source: &str,
    store: &NodeStore,
    links: &[PairLink],
) {
    if !spans.iter().any(|span| {
        matches!(span.kind,
        SpanKind::Aozora(Node::MarginNote(note)) if note.target_span.is_none())
    }) {
        return;
    }
    let pairs: BTreeMap<_, _> = links.iter().map(|pair| (pair.close.end, pair)).collect();
    for index in 1..spans.len() {
        let SpanKind::Aozora(Node::MarginNote(mut note)) = spans[index].kind else {
            continue;
        };
        if note.target_span.is_some() {
            continue;
        }
        let previous = &spans[index - 1];
        if previous.source_span.end != spans[index].source_span.start {
            continue;
        }
        let expected_pair = match previous.kind {
            SpanKind::Aozora(Node::Ruby(_)) => PairKind::Ruby,
            SpanKind::Aozora(Node::Format(_)) => PairKind::Bracket,
            _ => continue,
        };
        let Some(pair) = pairs.get(&previous.source_span.end) else {
            continue;
        };
        if pair.kind != expected_pair {
            continue;
        }
        let mut start = previous.source_span.start as usize;
        let end = pair.open.start as usize;
        if source
            .get(start..end)
            .is_some_and(|text| text.starts_with('｜'))
        {
            start += '｜'.len_utf8();
        }
        let (Some(base), Some(target)) = (
            source.get(start..end),
            store.content_range_as_plain(note.base),
        ) else {
            continue;
        };
        let mut occurrences = base.match_indices(target);
        let resolved = occurrences.next().and_then(|(offset, _)| {
            (!target.is_empty() && occurrences.next().is_none())
                .then_some((start + offset, start + offset + target.len()))
        });
        let resolved = resolved.or_else(|| {
            let SpanKind::Aozora(Node::Ruby(ruby)) = previous.kind else {
                return None;
            };
            let ruby_base = store.content_range_as_plain(ruby.base)?;
            let prefix = target.strip_suffix(ruby_base)?;
            if prefix.is_empty() {
                return None;
            }
            let ruby_start = previous.source_span.start as usize;
            let from = ruby_start.checked_sub(prefix.len())?;
            (source.get(from..ruby_start) == Some(prefix) && !source.get(from..end)?.contains('\n'))
                .then_some((from, previous.source_span.end as usize))
        });
        let Some((from, to)) = resolved else {
            continue;
        };
        let from = u32::try_from(from).expect("source offset fits u32");
        let to = u32::try_from(to).expect("source offset fits u32");
        note.target_span = NonEmptySpan::new(Span::new(from, to));
        spans[index].kind = SpanKind::Aozora(Node::MarginNote(note));
    }
}

/// Whether a forward attribute decorates a whole run as a single emphasis
/// wrapper, so it can style a ruby base. Excludes the sub-character /
/// target-splitting attributes: [`ForwardAttr::AccentDot`] (addresses letters
/// via an interned directive body the ruby cannot carry),
/// [`ForwardAttr::Accent`] (composes a single Latin letter), and
/// [`ForwardAttr::Fraction`] (splits the target on a slash), which are never
/// meaningful over a kanji base and stay declined.
const fn attr_decorates_ruby_base(attr: ForwardAttr) -> bool {
    !matches!(
        attr,
        ForwardAttr::AccentDot | ForwardAttr::Accent(_) | ForwardAttr::Fraction
    )
}

/// Apply a declined forward emphasis directive (`［＃「X」に傍点/罫囲み/…］`, a
/// [`ForwardOrigin::Referenced`] leaf) onto a preceding ruby whose base is the
/// unique referent quoted as `X`. The classifier declines these because a
/// ruby base cannot be pulled into a plain forward leaf (bouten-over-ruby is not
/// representable); this pass sets that ruby's `base_emphasis` instead, and the
/// renderer wraps the base in the attribute's emphasis element. The directive leaf stays
/// `Referenced` (serializes the bracket verbatim, renders nothing), so serialize
/// stays byte-identical.
///
/// Target resolution requires strict uniqueness rather than nearest-ruby
/// matching. The target must match exactly one preceding ruby base and
/// no preceding plain-text run anywhere in the look-back (a plain copy that
/// precedes the ruby, cross-line or same-line out of the classifier's reset
/// window, is a competing referent). If a competing referent exists, the
/// directive stays declined and the `forward_referent_not_stylable` warning
/// is retained. Returns the directive
/// spans decorated, so the builder can suppress exactly those warnings.
fn decorate_ruby_bases(out: &mut [ClassifiedSpan], source: &str, store: &NodeStore) -> Vec<Span> {
    let detached: BTreeSet<_> = out
        .iter()
        .filter_map(|span| {
            let SpanKind::Aozora(Node::Format(format)) = span.kind else {
                return None;
            };
            (format.origin == ForwardOrigin::Detached)
                .then_some((format.target.start, format.target.len))
        })
        .collect();
    let mut decorated: Vec<Span> = Vec::new();
    for idx in 0..out.len() {
        // Act only on a declined (Referenced) forward directive with a
        // whole-run-decoratable attribute.
        let SpanKind::Aozora(Node::Format(f)) = out[idx].kind else {
            continue;
        };
        if !matches!(f.origin, ForwardOrigin::Referenced)
            || detached.contains(&(f.target.start, f.target.len))
            || !f.attrs.single().is_some_and(attr_decorates_ruby_base)
        {
            continue;
        }
        // A ruby base is a single `Plain` run; a structured target never matches.
        let Some(target) = store.content_range_as_plain(f.target) else {
            continue;
        };
        // Scan the whole look-back for the unique referent.
        let mut ruby_match: Option<usize> = None;
        let mut ambiguous = false;
        for j in 0..idx {
            match &out[j].kind {
                SpanKind::Aozora(Node::Ruby(r)) => {
                    if store.content_range_as_plain(r.base) == Some(target) {
                        if ruby_match.is_some() {
                            ambiguous = true;
                            break;
                        }
                        ruby_match = Some(j);
                    }
                }
                // Any preceding plain run that carries the target text (before
                // the ruby, so invisible to the classifier's reset window) is a
                // competing referent that forces a decline.
                SpanKind::Plain(_) => {
                    let s =
                        &source[out[j].source_span.start as usize..out[j].source_span.end as usize];
                    if s.contains(target) {
                        ambiguous = true;
                        break;
                    }
                }
                _ => {}
            }
        }
        if ambiguous {
            continue;
        }
        let Some(ruby_idx) = ruby_match else {
            continue;
        };
        let Some(attr) = f.attrs.single() else {
            continue;
        };
        let directive_span = out[idx].source_span;
        if let SpanKind::Aozora(Node::Ruby(ref mut r)) = out[ruby_idx].kind {
            r.base_emphasis = Some(attr);
            decorated.push(directive_span);
        }
    }
    decorated
}

/// Byte position where `target` begins, only if it is the bare line
/// immediately preceding the `［` at `bracket_start`. `None` → the hint stays
/// inline.
fn find_heading_predecessor_position_at(
    source: &str,
    bracket_start: u32,
    target: &str,
) -> Option<u32> {
    let bytes = source.as_bytes();
    let cutoff = bracket_start as usize;
    if cutoff == 0 || bytes[cutoff - 1] != b'\n' {
        return None;
    }
    let text_end = cutoff - 1;
    let len = target.len();
    if text_end < len {
        return None;
    }
    let candidate_start = text_end - len;
    if &bytes[candidate_start..text_end] != target.as_bytes() {
        return None;
    }
    if candidate_start != 0 && bytes[candidate_start - 1] != b'\n' {
        return None;
    }
    u32::try_from(candidate_start).ok()
}

/// Promote each forward heading hint whose target is the bare line directly
/// above it into a `Heading`, reaching the span back over `target\n` so the
/// superset-drop pass reclaims the referent line.
fn promote_headings(
    mut spans: Vec<ClassifiedSpan>,
    source: &str,
    alloc: &mut Allocator,
) -> Vec<ClassifiedSpan> {
    for span in &mut spans {
        let SpanKind::Aozora(Node::HeadingHint(hint)) = span.kind else {
            continue;
        };
        // Resolve the interned target to an owned string so the `&store` borrow
        // does not overlap the `&mut alloc` builder calls below.
        let target = alloc.store().resolve_str(hint.target).to_owned();
        let Some(referent_start) =
            find_heading_predecessor_position_at(source, span.source_span.start, &target)
        else {
            continue;
        };
        let text = alloc.content_plain(&target);
        span.kind = SpanKind::Aozora(alloc.aozora_heading(hint.level, hint.style, text));
        span.source_span.start = referent_start;
    }
    spans
}

#[cfg(test)]
mod tests {
    use ab_aozora_syntax::ast::NodeRef;
    use core::ptr;

    use super::*;

    #[test]
    fn type_state_chain_compiles() {
        let _final = Pipeline::new("｜青梅《おうめ》")
            .sanitize()
            .tokenize()
            .pair()
            .build();
    }

    #[test]
    fn run_to_completion_matches_chain() {
        let chain = Pipeline::new("｜青梅《おうめ》")
            .sanitize()
            .tokenize()
            .pair()
            .build();
        let oneshot = Pipeline::run_to_completion("｜青梅《おうめ》");
        assert_eq!(chain.normalized, oneshot.normalized);
        assert_eq!(chain.sanitized_len, oneshot.sanitized_len);
        assert_eq!(
            chain.registry.count_kind(ab_aozora_spec::Sentinel::Inline),
            oneshot
                .registry
                .count_kind(ab_aozora_spec::Sentinel::Inline)
        );
    }

    #[test]
    fn intermediate_inspection_at_sanitized() {
        let p = Pipeline::new("plain text").sanitize();
        assert_eq!(p.sanitized_text(), "plain text");
        assert!(p.diagnostics().is_empty());
        drop(p.tokenize().pair().build());
    }

    #[test]
    fn intermediate_inspection_at_tokenized() {
        let p = Pipeline::new("a｜b《c》").sanitize().tokenize();
        assert!(p.tokens().len() >= 5);
        drop(p.pair().build());
    }

    #[test]
    fn intermediate_inspection_at_paired() {
        let p = Pipeline::new("a｜b《c》").sanitize().tokenize().pair();
        assert!(!p.events().is_empty());
        drop(p.build());
    }

    #[test]
    fn sanitize_accent_diagnostic_propagates() {
        let out = Pipeline::run_to_completion("abc〔cafe'〕def");
        assert!(
            out.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::AccentDecompositionApplied { .. })),
            "expected AccentDecompositionApplied, got {:?}",
            out.diagnostics
        );
    }

    #[test]
    fn empty_source_round_trips() {
        let out = Pipeline::run_to_completion("");
        assert!(out.normalized.is_empty());
        assert!(out.registry.is_empty());
        assert_eq!(out.sanitized_len, 0);
    }

    #[test]
    fn source_accessor_returns_original() {
        let s = "the original";
        let p = Pipeline::new(s);
        assert!(ptr::eq(p.source(), s));
    }

    #[test]
    fn paired_margin_note_resolves_across_structured_base() {
        let open = "［＃注記付き］";
        let target = "碧梧桐［＃「碧梧桐」は罫囲み］";
        let close = "［＃「何とかいう俳画家」の注記付き終わり］";
        let source = format!("{open}{target}{close}");
        let out = Pipeline::run_to_completion(&source);
        let span = |start: usize, end: usize| {
            Span::new(u32::try_from(start).unwrap(), u32::try_from(end).unwrap())
        };
        let note = out
            .source_nodes
            .iter()
            .find_map(|source_node| match source_node.node {
                NodeRef::Inline(Node::MarginNote(note)) => Some(note),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            out.store
                .resolve_margin_note_pair(note.pair.unwrap())
                .open_span
                .span(),
            span(0, open.len())
        );
        assert_eq!(
            note.target_span.unwrap().span(),
            span(open.len(), open.len() + target.len())
        );
        assert_eq!(
            note.note_span.unwrap().span(),
            span(
                open.len() + target.len() + "［＃「".len(),
                source.len() - "」の注記付き終わり］".len(),
            )
        );
        assert!(out.source_nodes.iter().any(|source_node| {
            matches!(
                source_node.node,
                NodeRef::Inline(Node::Directive(directive))
                    if directive.kind == DirectiveKind::MarginNotePairOpenResolved
            )
        }));
    }
}
