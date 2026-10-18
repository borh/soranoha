//! Dependency-free plain-text diagnostic rendering.
//!
//! [`diagnostics_text`] formats a parse's diagnostics into a
//! human-readable string using only the public [`Diagnostic`] accessors
//! — no `miette` / terminal dependency. It therefore compiles for
//! every target (including `wasm32-unknown-unknown`, where `miette`'s
//! `fancy` graphical renderer does not build), so the WASM / Python / FFI
//! bindings can return human-readable diagnostics in addition to the
//! [`json`](crate::json) envelope.
//!
//! The CLI renders a `miette` graphical report on a TTY. This format
//! corresponds to the CLI's one-line `short` view, with the offending
//! source slice added.

use std::fmt::{self, Write as _};

use ab_aozora_pipeline::lexer::sanitize;

use crate::Diagnostic;

/// Render `diagnostics` over `source` as a plain-text report.
///
/// One block per diagnostic — a `<severity> [<code>] @ <start>..<end>:
/// <message>` header followed by the offending source slice — joined by
/// newlines. Returns the empty string when `diagnostics` is empty.
///
/// Spans are in sanitized coordinates (the lexer strips the BOM, folds
/// CRLF→LF, and decomposes 〔…〕 accent digraphs, each of which shifts
/// byte offsets), so `source` is sanitized internally before the
/// offending slice is cut, as in the CLI's graphical renderer.
///
/// # Panics
///
/// Does not panic in normal use: `String` cannot fail as a
/// [`Write`](std::fmt::Write) sink.
#[must_use]
pub fn diagnostics_text(source: &str, diagnostics: &[Diagnostic]) -> String {
    let mut out = String::new();
    // Writing into a `String` cannot fail; the fallible inner helper
    // permits `write!` with `?` instead of discarding each line's result,
    // as in the `serialize` renderer.
    write_report(&mut out, source, diagnostics).expect("String write is infallible");
    out
}

fn write_report(out: &mut String, source: &str, diagnostics: &[Diagnostic]) -> fmt::Result {
    if diagnostics.is_empty() {
        return Ok(());
    }
    let sanitized = sanitize(source).text;
    for diag in diagnostics {
        let span = diag.span();
        let (start, end) = (span.start as usize, span.end as usize);
        writeln!(
            out,
            "{severity} [{code}] @ {start}..{end}: {diag}",
            severity = diag.severity().as_json_str(),
            code = diag.code(),
        )?;
        if let Some(slice) = sanitized.get(start..end) {
            writeln!(out, "  > {slice:?}")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::Document;

    #[test]
    fn empty_when_no_diagnostics() {
        let doc = Document::new("｜青空《あおぞら》");
        let tree = doc.parse();
        let text = super::diagnostics_text(doc.source(), tree.diagnostics());
        assert!(text.is_empty(), "clean parse renders nothing: {text:?}");
    }

    #[test]
    fn renders_code_and_offending_slice() {
        // The inner `《ん》` inside the outer reading is nested ruby.
        let doc = Document::new("｜漢《か《ん》じ》");
        let tree = doc.parse();
        let diagnostics = tree.diagnostics();
        assert!(!diagnostics.is_empty(), "nested ruby yields a diagnostic");
        let text = super::diagnostics_text(doc.source(), diagnostics);
        assert!(
            text.contains("aozora::") && text.contains('@'),
            "header carries the code + span: {text}"
        );
        assert!(
            text.contains("  > \""),
            "report carries the source slice: {text}"
        );
    }

    #[test]
    fn one_block_per_diagnostic() {
        let doc = Document::new("｜《おうめ》");
        let tree = doc.parse();
        let diagnostics = tree.diagnostics();
        let text = super::diagnostics_text(doc.source(), diagnostics);
        // Each diagnostic contributes one header line, which starts with its severity.
        let headers = text.lines().filter(|l| l.contains("] @ ")).count();
        assert_eq!(
            headers,
            diagnostics.len(),
            "one header per diagnostic: {text}"
        );
    }
}
