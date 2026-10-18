# Research annotation layers

The base TEI keeps the source paragraphs and inline nodes as transcribed.
Sentence segmentation lives in separately chosen analysis layers; it does not
split source nodes or add sentence wrappers to the base TEI. This is enforced by
the profile: any element of TEI's analysis module inside `tei:text` is reported
unless declared in the header.

Analysis runs on a named text view of the TEI, not on an export with its
presentation whitespace. The `body-v1` view takes ruby bases and corrected or
regularized readings, drops notes, figure descriptions and running matter, and
keeps semantic line and page breaks and block separators. Offsets are UTF-8
bytes. An unresolved empty glyph is U+FFFC and lies outside the eligible
ranges; it is never deleted without a report. The view identity covers the
exact text and the reading policy. Its alignment to the DOM is local to the
source document and is not part of the analyzer input. Image metadata is not
body text: `figDesc` keeps the description and a `note` of type
`caption-reference` keeps any quoted caption reference, while the separately
transcribed visible caption stays body text, following the
[Aozora Bunko image convention](https://www.aozora.gr.jp/annotation/graphics.html).
Correcting descriptions that were previously included changes the affected
text identities; readings without images keep their existing `body-v1`
identity.

A `soranoha-annotation-layer/1` JSON file holds one producer's claims. The
envelope has the view identity, a vocabulary content hash, the producer's named
input hashes, the actual dependency hashes, the eligible spans, the execution
status and the records. A record has an identifier, a half-open byte range, a
label and string-valued features. The vocabulary and producer identities fix
what the claims mean; validating the envelope does not check whether the
claims are linguistically right.

Every offset must fall on a UTF-8 character boundary inside an eligible range.
Producer inputs include the code, model, dictionary and configuration actually
used, where each applies. The eligibility mask is an input, so changing it can
invalidate an analysis even when the text is unchanged. A completed run with no
records is distinct from `failed`, `partial` and `not-run`; a `failed` or
`not-run` layer carries no claims. Competing tokenizations or entity labels are
separate layers and do not replace each other's records.

The computation identity excludes the resulting claims and the source edition
the layer is attached to, whereas the layer identity encompasses the claims and
the execution outcome. An assembly selects exact layer artifacts. When the source
correspondence changes but the analyzer inputs do not, the same layer can be
attached to the new edition through its current TEI alignment.

Enriched TEI keeps the transcription, inserts anchors and appends stand-off
span groups. Each group refers to its layer by content identity; the record
label goes in `span/@n` and the features as JSON text inside the span. These
are analysis claims, not statements about the source. The base TEI and
plaintext stages do not depend on the chosen layers; enrichment takes only the
base TEI and the ordered layer blob identities.

An anchor may split a text node; the concatenated text is unchanged. A TEI
glyph is atomic, so an analysis boundary inside a multi-codepoint glyph is
rejected rather than splitting the glyph. Existing XML identifiers are kept; an
identifier collision or an incompatible text view is an error. The file
commands never overwrite an existing output.

## Markdown consumer

The `markdown/2` profile produces horizontal CommonMark with inline HTML. The
reference consumer is the flake-pinned cmark with HTML enabled (`--unsafe`).
Tests check that text, ruby nesting, line breaks and the generated `span`
attributes survive that consumer; they do not test a browser's typography or a
hosting service's sanitizer. Consuming tools and rendering pipelines must
permit `ruby`, `rb`, `rt`, `strong`, `em`, `br`, and `span` with its generated
`style` and `data-tei-rend` attributes. Source text is escaped for both HTML
and CommonMark, including inside inline HTML.

The profile keeps visible body text and ruby readings. Explicit TEI renditions
become:

| Rendition | Horizontal projection |
| --- | --- |
| Bold and italic | `strong` and `em` |
| Named sesame, circle, triangle, double-circle, bullseye and cross dots | CSS text emphasis preserving the named shape; source-right marks above, source-left below |
| Solid, double and wavy side lines | CSS text decoration; source-right below, source-left above, both on both sides |
| Lowered baseline and exponent | `vertical-align: sub` and `vertical-align: super`, both at the inherited font size, because the source states a position and not a size |
| Inline horizontal text and vertical text combination | `writing-mode: horizontal-tb` and `text-combine-upright: all`; already horizontal text stays horizontal |
| An unqualified inline box | A solid one-pixel border; no claim of the source's exact stroke width |

Dot shapes and side-line kinds follow the
[Aozora Bunko emphasis notation](https://www.aozora.gr.jp/annotation/emphasis.html),
whose horizontal XHTML examples underline default side lines and overline
left-side lines. Dot placement follows the
[CSS text-emphasis convention](https://www.w3.org/TR/css-text-decor-3/#text-emphasis-position-property).
These are choices for horizontal display, not a rotation of the vertical source
typography. Each generated span keeps the original `rend` in `data-tei-rend`,
and the projection report classifies it as `transformed`.

The profile omits source apparatus, alternative readings and layout such as
exact pagination. Generic `emphasis` has no visual style and is unsupported.
Unknown rendition tokens, other TEI inline styles, unsupported ruby placement
and unresolved glyphs are listed in the report, and their text is kept. A
`complete-for-profile` result means complete for this projection; it does not
mean the parser was correct or the edition faithful.

## Source correspondence and reading variants

The TEI attaches decoded UTF-8 source ranges through local `@source`
references. In the referenced note, the range record and, when available, the
content hash of the primary text are preserved. Equal range records share one
identifier. Parser-text offsets are a different coordinate system and cannot
stand in for a missing source range.

A supplied reading that differs from the base text is an `app`: `lem` for the
supplied reading, `rdg type="base-text"` for the base text. This does not
assert that the base text is erroneous. In both the body reading view and the
Markdown projection, the supplied reading is selected, with the omitted
alternative recorded in the Markdown report.

A positive interpretation fact identifies the exact source marker resolved
during interpretation, without encompassing every nested marker in the node's
provenance range. A paired scope claims its opener and closer separately when
both are known. The independent lexical scan joins those facts by exact decoded
byte range.

Structured findings about syntax problems are preserved in
`interpretation-problem` notes. Where a problem affects textual content, the
entire body is excluded from analysis until its influence range in the analysis
view is established; the marker's own source range is not sufficient. A problem
affecting layout only leaves the text eligible. Eligibility means usable under
these known-uncertainty masks; it does not mean the source is fully covered or
that no other uncertainty exists. The masks are separate from the reading
identity, so unchanged analyzer text can be reused while the layer must still
satisfy the current mask.

Warichu is `seg type="warichu" rend="two-line"`. An undivided source stays one
content sequence; nested `seg type="upper"` and `seg type="lower"` appear only
when the input establishes that division. Text projections keep the supplied reading
order and report the two-line layout as omitted.

In the TEI edition, distinctions are made for Gothic, italic, superscript and
subscript. Font sizes are recorded as either a relative step count or an absolute
size category; small writing to the right or left of a line retains its
placement. Neither is treated as horizontal superscript or subscript.
Unimplemented font and script renditions are recorded in the Markdown projection
report.
