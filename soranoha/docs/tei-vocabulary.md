# Soranoha TEI extension vocabulary

Namespace IRI: `https://w3id.org/soranoha/ns/tei`
Conventional prefix: `snh:`
Current vocabulary version: `3`

Published TEI documents bind this namespace IRI on the root element.

Soranoha publishes TEI P5. Where the source markup carries information TEI has
no exact representation for, that information uses this namespace.
Every published TEI root binds the
namespace and declares the vocabulary version:

```xml
<TEI xmlns="http://www.tei-c.org/ns/1.0"
     xmlns:snh="https://w3id.org/soranoha/ns/tei"
     snh:vocab-version="3">
```

These extensions are derived audit projections. They record what the parser
established about the Aozora Bunko source; they do not replace the source records
from which they were derived, and a consumer may ignore them without losing the
transcription.

## Versioning

The namespace IRI identifies the extension and never changes. The vocabulary's
major version is not implied by that IRI, so `snh:vocab-version`
carries it separately. A breaking vocabulary change increments this version, whereas an
additive attribute may keep the same major version when compatibility is
documented.

The profile hashes recorded in a work's `tei-validation.json` (`odd_hash`,
`rng_hash`, `schematron_hash`) serve a different purpose. They change on any
profile edit, additive ones included, so they identify which profile validated
a document but do not indicate whether an existing `snh:*` reader is still
correct. That is what the vocabulary version is for.

Version `1` contains attributes only. Custom elements in this namespace
are not part of it. Version `2` adds `snh:gaiji-reference` within `tei:char`;
each element represents one reference clause in source order, with its exact
spelling in `original`. Version `3` adds `snh:formatting-instruction` within
`tei:note`, retaining ordered operations, selectors, and effects. Its operand
source references identify annotation bytes, not resolved body targets.
The ODD defines the permitted semantic fields.

## Attributes

All are optional and may appear on any element, subject to the constraints
below. Only the first three are emitted by the current pipeline; the rest are
declared for projections that are specified but not yet produced.

| Attribute | Value | Meaning |
|---|---|---|
| `snh:vocab-version` | `1`, `2`, `3` | Vocabulary major version. Required on the root of any document carrying `snh:*` elements or attributes; `snh:gaiji-reference` requires `2` or `3`; `snh:formatting-instruction` requires `3`. |
| `snh:layout-kind` | token, or space-separated tokens when scopes co-apply | Layout fact projected verbatim from the parser IR. Paragraph layouts are `burasage`, `chitsuki`, `jisage`, `jizume`, `line-jisage`; inline scopes are `baseline-position`, `chitsuki`, `emphasis`, `exponent`, `font-size`, `fraction`, `keigakomi`, `line-jisage`, `small-script`, `tcy`, `yokogumi`. |
| `snh:layout-params` | `key=value` pairs joined by `;` | Parameters of the layout fact, for example `indent=2` or `align=right;offset-from-end=1`. |
| `snh:preservation-record` | `r` followed by six digits | Record id in the publication-preservation sidecar. |
| `snh:ir-pointer` | JSON Pointer | Location in the parser IR of the projected fact. |
| `snh:source-pointer` | pointer string | Location in the source or AAT of the projected fact. |
| `snh:source-marker` | string | Original Aozora Bunko marker, kept when a TEI-native rendering normalizes it away. |
| `snh:accent-code` | string | Aozora Bunko accent notation code. |
| `snh:style-source` | string | Aozora Bunko style source marker. |
| `snh:figure-class` | string | Aozora Bunko figure class. |

Permitted values of `snh:vocab-version` are declared in
[the ODD](../schemas/tei-profile.odd) and enforced by the generated Relax NG schema.
The initial version is `1`.

## Source spans

Most elements in a published body carry `@source`, pointing at a `note` of
type `source-span` in the back matter. Those notes are the offset table: they
state which run of the source text an element came from.

A note is empty, because everything about the extent is in its reference:

```xml
<note type="source-span" xml:id="source-694-733" n="20"/>
```

- `xml:id` has the form `source-START-END`. `START` and `END` are offsets in
  UTF-8 bytes into the decoded primary text, which `sourceDesc` identifies by
  its `idno` of type `primary-text-hash`, after the emendations declared in
  `editorialDecl/correction` are applied. Decode the source archive's text
  member, apply each emendation by replacing its `sic` text with its `corr`
  text at its own extent, encode the result as UTF-8, and slice.
- `n` is the line the extent begins on. It is absent when the source did not
  record one, rather than carrying a placeholder.

The header declares this unit in a `refsDecl` pointing back at this document.

Notes with `xml:id` prefixed by `parser-source` belong to parser diagnostics
rather than body elements; they carry their byte extent as JSON in element content.

## Emendations

A work whose source contains a transcription defect that the parser corrects
before parsing declares each correction in the header:

```xml
<editorialDecl>
  <correction method="markup">
    <p xml:id="emendation-pariyori-403-ruby-bar-after-base" source="#source-START-END">
      <choice><sic>一番向｜《むか》うにある</sic><corr>一番｜向《むか》うにある</corr></choice>
    </p>
  </correction>
</editorialDecl>
```

- `sic` is the transcribed source text; `corr` is the text the parser read.
- `xml:id` is `emendation-` followed by the emendation identifier.
- `@source` refers to the `source-span` note of the `corr` text's extent,
  measured as described under [Source spans](#source-spans).
- Declarations follow the order in which the parser applied them. A work with
  no emendation has no `editorialDecl`.
- An emendation records a correction that Aozora Bunko's own publication
  establishes, in the published XHTML or in a later edition of the work. A
  defect without such a witness is retained as transcribed.

## Schematron rule identifiers

Constraint violations are published in each work's `tei-validation.json` as
`findings[].rule_id`. The identifiers are stable names, not sequence numbers.
Every identifier below comes from the Schematron layer; a Relax NG failure is
reported with the constant `rule_id` `relax-ng` instead.

| Rule ID | Severity | Requires |
|---|---|---|
| `snh-tei-header-title` | error | A main title in `teiHeader/fileDesc/titleStmt` (a `title` that is not one of the subordinate forms `sub`, `sub-reading`, `reading`, or `original`). |
| `snh-tei-header-source-work-id` | error | An Aozora Bunko work ID or source work identifier in the header. |
| `snh-header-language-declared` | error | At least one `profileDesc/langUsage/language` with a non-empty `@ident`. |
| `snh-publication-licence` | error | A rights grant in `publicationStmt/availability/licence` with a non-empty `@target`. |
| `snh-ruby-complete` | error | Ruby structures contain both base and reading components. |
| `snh-ruby-base-non-empty` | error | A ruby base contains text or a declared character. |
| `snh-ruby-reading-non-empty` | error | A ruby reading is not empty. |
| `snh-gaiji-reference` | error | Gaiji retains its original marker and either a resolved replacement or unresolved status. |
| `snh-gaiji-chardecl-resolution` | error | A gaiji `@ref` fragment resolves to a `charDecl/char` in the same document. |
| `snh-char-resolution-form` | error | A `charDecl/char` declares at least one of `mapping`, `unicodeProp`, `localProp`, `desc`. |
| `snh-figure-accessibility` | warning | A figure carries a nonempty description or caption, so the image it refers to has alternative text. Most Aozora Bunko illustration markers supply a filename and pixel dimensions and no description, so this rule reports the source's own form rather than anything the conversion dropped. |
| `snh-source-span-reference` | error | Elements claiming source spans point to valid span identifiers. |
| `snh-source-span-target-exists` | error | Each fragment id in `@source` resolves to an `@xml:id` in the same document. |
| `snh-transcription-vs-annotation` | warning | No element of TEI's analysis module appears inside `tei:text` unless the header declares it. Stand-off anchors and a `standOff` sibling are outside the rule, which is what the supported enrichment path writes. |
| `snh-vocab-version-declared` | error | A root carrying `snh:*` elements or attributes declares `snh:vocab-version`; `snh:gaiji-reference` requires `2` or `3`; `snh:formatting-instruction` requires `3`. |
| `snh-preservation-record-shape` | error | `snh:preservation-record` values have the form `r000000`. |
| `snh-layout-params-shape` | error | `snh:layout-params` uses the `key=value` grammar above. |

## Rights in the header

Every published work states its own terms in
`fileDesc/publicationStmt/availability`, with one `licence` element per rights
layer (one for the standing of the underlying work, which is public domain or a
Creative Commons Attribution licence, and one for Soranoha's CC0 grant over the
encoding, which also carries a `ptr` to the full statement). The encoding grant
is rendered from the one publication policy the release manifest hashes and the
work's standing from the same value the manifest records in `works[].rights`,
so a detached TEI file and the signed release record cannot state different
terms.
`snh-publication-licence` makes their absence a validation failure. See
[rights and licensing](../../docs/rights.md).

A schema pass does not establish source fidelity or publication rights. See
[TEI validation](tei-validation.md) for how the two validation layers run, and
[source accountability](source-accountability.md) for the separate fidelity
evidence.

## Provenance of this document

The vocabulary is defined by [`soranoha/schemas/tei-profile.odd`](../schemas/tei-profile.odd),
from which the Relax NG and Schematron are generated. Where this page and the
ODD disagree, the ODD is authoritative.
