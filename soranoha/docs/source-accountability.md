# Source accountability and validation

`source-accountability.json` records lexical occurrences found directly in the
retained source. It binds the original bytes, decoded UTF-8 text and coverage
matrix by hash. Each occurrence retains its exact spelling, decoded byte span,
source region and recognized syntax families. Editorial legend examples remain
visible as front matter. Unrecognized markers and lossy decoding remain explicit.

This scanner runs independently of the parser. Its stage identity binds the
scanner executable and matrix; changes confined to the parser stage do not
invalidate its evidence. Changes to shared scanner dependencies can invalidate it.
The native command is `ab-source-inventory --source source.txt --matrix matrix.toml
--output-json accountability.json`. The publication wrapper supplies
`AB_SOURCE_INVENTORY_BIN` and `AB_AOZORA_SYNTAX_MATRIX`.

Lexical recognition does not establish a correct interpretation. Interpreter
claims describe what the parser recognized; missing claims remain
unaccounted. Neither source preservation nor an empty problem list proves semantic
coverage. Source conformance tests exercise expected values and annotation
placement for specific examples. TEI profile validation checks the exported
XML against the profile. These are separate kinds of evidence.

A report about one edition does not compare its exported values independently
against every source construction. Conformance tests support the examples they
exercise; structural validation and interpreter claims are not equivalent to a
per-edition source/export fidelity certificate.

`interpretation-coverage.json` joins this lexical evidence to explicit native
interpretation facts. Family counts distinguish `interpreter_claimed` from
`unaccounted` and from `outside_claim_vocabulary`; a claimed occurrence has at
least one explicit compatible aspect, not necessarily every aspect. Each claim
must match the exact span and a compatible family of an independently scanned
occurrence or nested component.
Nested markers retain their own component spans. Source apparatus is kept
separate. Lossy decoding prevents positive claim accounting. Reported
interpretation problems retain their original influence bounds alongside positive
claims. A positive claim is not evidence that an overlapping or document-wide
problem has been resolved.

The report states the counts and lists the exceptions. `families` and
`interpretation_problems` cover every occurrence; `occurrence_count` and
`unclassified_occurrence_count` give the totals; `unaccounted_occurrences` holds,
whole, each occurrence with an unaccounted family or unclassified syntax in the
outer marker or a nested component. Valid partial claims remain in the report;
they do not remove unclassified components from that list. Each containing occurrence is counted
once, regardless of how many components are unclassified.

Families incompatible with every interpretation fact for their marker kind are
counted under `outside_claim_vocabulary`, rather than `unaccounted`. They do not
by themselves cause an occurrence to be listed; unclassified components still do.

## Review exports

From the repository root, set these variables to absolute paths. The corpus must
be a clean Git checkout and the export directory must not exist:

```sh
nix run .#soranoha-kernel -- build \
  --root "$BUILD_STORE" \
  --aozora-root "$CORPUS_CHECKOUT" \
  --assets-root "$PWD/soranoha" \
  --out "$EXPORT_DIR"
```

Each work directory contains `tei.xml`, `plain.txt`, `text.md`,
`tei-validation.json`, `plaintext-projection.json`, `markdown-projection.json`,
`source-accountability.json` and `interpretation-coverage.json`. A top-level
`build.json` records artifact hashes and build results. Omitting `--out` retains
outputs in the computation store. Building exports does not publish them or accept
an assessment.

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

Validation provenance binds the ODD, RNG and Schematron hashes through
`schemas/tei-profile-generation.json`. See [TEI validation](tei-validation.md).
