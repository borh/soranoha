# Source accountability

Aozora Bunko marks up its texts with its own notation: `《》` for ruby,
`※［＃...］` for characters that could not be typed, `［＃...］` for layout and
emphasis. Source accountability answers one question: how much of that
notation did the conversion account for? It does not cover rights, which
[assessment evaluation](assessment-evaluation.md) does, or whether the TEI is
valid, which [TEI validation](tei-validation.md) does.

## Two reports

The build writes two reports for every work, beside the exported TEI:

- `source-accountability.json` lists every piece of notation in the source
  text. A scanner that does not use the parser finds them by matching the text
  against a table of Aozora syntax. For each one it records the exact spelling,
  the byte span in the decoded UTF-8 text, the region of the file it sits in,
  and the syntax families it matched. Markers the table does not recognise, and
  text the decoder could not decode, are listed as well.
- `interpretation-coverage.json` compares that list with what the parser
  claims to have interpreted. A claim counts only when it covers the same byte
  span as a scanned occurrence and a compatible family. For each family the
  report counts occurrences as `interpreter_claimed`, `unaccounted` or
  `outside_claim_vocabulary`, and it lists in full every occurrence with an
  unaccounted or unclassified part.

Because the scanner does not use the parser, a change to the parser cannot
alter the list its claims are compared with.

## What the reports do not show

The counts show that a marker was recognised, not that it was converted
correctly. Correctness is checked by conformance tests on specific examples and
by schema validation. An empty list of unaccounted occurrences therefore does
not mean the conversion is complete. Where the parser reports a problem over a
span, a claim over the same span does not cancel it; the problem stays in the
published TEI as an `interpretation-problem` note.

Both reports stay with the build. They are not published with the work; of
the build's reports, only `tei-validation.json` is.

## Exported representations

TEI retains ruby, gaiji declarations, headings, layout and source apparatus;
plaintext and the principal-body view retain visible body text only. Structural
XML is indented for inspection while mixed content preserves lexical whitespace.

- **Gaiji and iteration marks**: Resolved gaiji place Unicode text directly in
  `g` (including inside ruby bases) and keep source mappings in `ref` and
  `charDecl`. Complete iteration notation (`／＼`, `／″＼`) keeps its source
  spelling in `choice/orig` and realizes `〱` or `〲` in `choice/reg` without
  expanding the repeated reading. Iteration marks inside a ruby reading stay part
  of the ruby annotation; malformed notation and literal half-marks stay literal
  text.
- **Kunten and supplied diacritics**: Supplied kunten uses `note type="kunten"`
  (`subtype="return-mark"` subscript, `subtype="okurigana"` superscript, with
  nested `hi` when okurigana carries 傍点 or 傍線) and is excluded from
  plaintext. Supplied diacritic annotations keep source spelling in `choice/orig`
  and composed Unicode in `choice/reg`; interior-letter selectors retain the
  original marker as source apparatus, and unresolved selectors retain explicit
  uncertainty.
- **Emphasis and between-operand marks**: Exclusion selectors such as
  `［＃「自律・自由・人格・性格」の「・」を除く部分に傍点］` mark every non-excluded
  span while leaving excluded runs as unmarked body text; unmappable exclusions
  remain unread. Junction markers such as `［＃「自」と「然」の間に白三角傍点］`
  emit a supplied mark anchored between adjacent operands rather than emphasis
  over a span.
- **Block geometry and alignment**: Indentation, continuation indentation, line
  width (`jizume`), and foot offset (`chitsuki`) are retained independently.
  Paragraph-leading indentation uses CSS `text-indent`; heading and source-note
  continuation indentation uses `padding-inline-start`. Supplied line counts use
  `rend="line-count(N)"`, and page centering follows
  [Aozora's page-centering convention](https://www.aozora.gr.jp/annotation/layout_3.html)
  until the next page break. Consecutive indentation instructions replace the
  preceding indentation scope; a closing marker that restates a measure closes
  only a matching opener, and unmatched closers stay retained source. Relative
  placement links to its anchor's source span via `corresp` and records
  `placement-below`, `anchor-kind(...)`, and `anchor-offset-chars(N)` in `rend`.
- **Frames, tables, quotations and letters**: Frame styles (`solid`, `dashed`,
  `double`) and co-applied typography share the `@rend` vocabulary on `div` and
  `hi`. Supplied table scopes map to `div type="table"` (`図表` to
  `type="figure-table"`), keeping enclosed lines verbatim without inferring row
  or cell structure; `罫無し` maps to `rend="table-rules(none)"`, and `罫仕切り`
  remains an explanation note with literal `----` divider lines. Quotation and
  letter openers (`引用文`, `手紙文`) map to `div type="quotation"` or
  `div type="letter"` alongside their indentation and close only on a matching
  role closer; unmodeled clauses (`３行アキ`, `はじめの「一」のみ２字下げ`) stay
  retained source.
- **Font size and baseline placement**: Qualitative size comparisons (`字が小さい`,
  `ポイントを下げて`, with optional `やや` or `ひとまわり`) map to
  `font-size qualitative(smaller|larger)` and `qualifier(...)`, including when
  combined with a line-scoped foot offset. Bare `下付き` maps to
  `baseline-lowered` (distinct from `下付き小文字`) and `指数` maps to `exponent`
  while keeping the target glyphs unchanged in plaintext and Markdown.
