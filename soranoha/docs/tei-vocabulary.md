# Soranoha TEI extension vocabulary

Namespace IRI: `https://w3id.org/soranoha/ns/tei`
Conventional prefix: `snh:`
Current vocabulary version: `3`

Soranoha publishes TEI P5. Where the Aozora Bunko source carries information
TEI cannot represent exactly, that information goes in this namespace. Every
published TEI root binds the namespace and declares the vocabulary version:

```xml
<TEI xmlns="http://www.tei-c.org/ns/1.0"
     xmlns:snh="https://w3id.org/soranoha/ns/tei"
     snh:vocab-version="3">
```

The extensions record what the parser found in the source. They do not replace
the source records they come from, and a consumer that ignores them still has
the whole transcription.

## Versioning

The namespace IRI never changes. `snh:vocab-version` carries the major
version, which the IRI does not. A breaking change increments it; an additive
attribute may keep it when compatibility is documented.

The profile hashes in a work's `tei-validation.json` (`odd_hash`, `rng_hash`,
`schematron_hash`) change on any profile edit, additive ones included. They
identify which profile validated a document; they do not tell an `snh:*`
reader whether it is still correct. The vocabulary version does.

Version `1` has attributes only and no custom elements. Version `2` adds
`snh:gaiji-reference` inside `tei:char`: one element per reference clause, in
source order, with its exact spelling in `original`. Version `3` adds
`snh:formatting-instruction` inside `tei:note`, which keeps the ordered
operations, selectors and effects; its operand source references point at the
annotation bytes, not at the resolved body targets. Permitted semantic fields
are defined in the ODD.

## Attributes

All are optional and may appear on any element, subject to the constraints
below. Only the first three are produced by the current pipeline; the remainder
are declared for projections that are specified but not yet produced.

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
[the ODD](../schemas/tei-profile.odd) and enforced through the generated
Relax NG schema.

## Source spans

In a published body, most elements carry `@source`, referencing a `note` of type
`source-span` in the back matter. Together these notes form the offset table,
identifying the span of source text corresponding to each element.

The note is empty; the extent is in its attributes:

```xml
<note type="source-span" xml:id="source-694-733" n="20"/>
```

- `xml:id` has the form `source-START-END`. `START` and `END` are UTF-8 byte
  offsets into the decoded primary text, which `sourceDesc` identifies by its
  `idno` of type `primary-text-hash`, after the emendations declared in
  `editorialDecl/correction` are applied. To take the slice: decode the
  archive's text member, apply each emendation by replacing its `sic` text
  with its `corr` text at its own extent, encode the result as UTF-8, and cut
  from `START` to `END`.
- `n` is the line the extent begins on. It is absent when the source recorded
  no line.

The unit is declared in a `refsDecl` pointing to this document.

Notes whose `xml:id` starts with `parser-source` are parser diagnostics, not
body elements; their byte extent is JSON in the element content.

## Character declarations

Each gaiji in the text is represented by a `g` element referencing a `char`
declaration in `encodingDesc/charDecl`. Available details for the character
are recorded in the declaration:

| Child | Meaning |
|---|---|
| `mapping[@type='unicode']` | The single code point resolved from the marker's JIS X 0213 coordinate, `U+` code, or description. |
| `localProp[@name='mappingSource']` | Where the code point or variation sequence derives from the cjkvi-data table of Aozora notations rather than a JIS reference, the 外字注記辞書, or a composition rule: the URL of that table at the pinned commit. |
| `localProp[@name='variationSource']` | Present when the selector of an IVS mapping derives from the cjkvi-data tables: the URL of that table at the pinned commit. When `mappingSource` is absent, the base character derives from the official dictionary. |
| `mapping[@type='composed']` | A sequence of code points: a JIS X 0213 character represented in Unicode as a base and combining mark, a Roman numeral above twelve, or a `濁点付き…` description composed as a base followed by U+3099 or U+309A. |
| `mapping[@type='IVS']` | An Ideographic Variation Sequence (base character followed by a variation selector registered in the Unicode Ideographic Variation Database) from the cjkvi-data tables. An accompanying `unicode` mapping provides the base character. |
| `mapping[@type='IDS']` | The Ideographic Description Sequence of a character unencoded in Unicode, derived from the GlyphWiki group for Aozora Bunko gaiji. |
| `localProp[@name='glyphwiki']` | The GlyphWiki glyph name for that character, with the corresponding glyph image URL on glyphwiki.org provided in `figure/graphic/@url`. |
| `localProp[@name='rawMarker']`, `localProp[@name='sourceReference']`, `localProp[@name='unresolvedReason']` | The marker's description, its reference clause and, when nothing above applies, the reason. |
| `snh:gaiji-reference` | One element per reference clause, in source order, with its exact spelling in `original`. |

For a character without an associated mapping, only its description and source span
are retained. In plaintext and Markdown output, composed sequences appear where
provided in TEI character declarations; otherwise, the original marker text appears.

## Emendations

Where transcription defects in the source are emended prior to parsing,
each correction is declared in the header:

```xml
<editorialDecl>
  <correction method="markup">
    <p xml:id="emendation-pariyori-403-ruby-bar-after-base" source="#source-START-END">
      <choice><sic>一番向｜《むか》うにある</sic><corr>一番｜向《むか》うにある</corr></choice>
    </p>
  </correction>
</editorialDecl>
```

- `sic` is the transcribed source text; `corr` is the emended reading adopted during parsing.
- `xml:id` is `emendation-` followed by the emendation identifier.
- `@source` refers to the `source-span` note of the `corr` text's extent,
  measured as described under [Source spans](#source-spans).
- Declarations follow the order in which emendations were applied. Works
  without emendations contain no `editorialDecl`.
- An emendation is made only where Aozora Bunko's own publication, the XHTML
  or a later edition of the work, has the corrected text. A defect without
  that witness is kept as transcribed.

## Schematron rule identifiers

Each work's `tei-validation.json` lists violations as `findings[].rule_id`.
The identifiers are stable names. All of those below are Schematron rules; a
Relax NG failure has the constant `rule_id` `relax-ng`.

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
| `snh-figure-accessibility` | warning | Figures must include a non-empty description or caption providing alternative text for the referenced image. Because most Aozora Bunko illustration markers specify only a filename and pixel dimensions without descriptive text, this warning reflects the source transcription as recorded. |
| `snh-source-span-reference` | error | Elements claiming source spans point to valid span identifiers. |
| `snh-source-span-target-exists` | error | Each fragment id in `@source` resolves to an `@xml:id` in the same document. |
| `snh-catref-target-exists` | error | Each fragment id in `catRef/@target` resolves to a `category/@xml:id` in the same document. |
| `snh-transcription-vs-annotation` | warning | Elements from the TEI analysis module may not appear within `tei:text` unless declared in the header. Stand-off anchors and sibling `standOff` elements generated by downstream enrichment are exempt from this restriction. |
| `snh-vocab-version-declared` | error | A root carrying `snh:*` elements or attributes declares `snh:vocab-version`; `snh:gaiji-reference` requires `2` or `3`; `snh:formatting-instruction` requires `3`. |
| `snh-preservation-record-shape` | error | `snh:preservation-record` values have the form `r000000`. |
| `snh-layout-params-shape` | error | `snh:layout-params` uses the `key=value` grammar above. |

## Rights in the header

Rights terms for each published work are recorded in
`fileDesc/publicationStmt/availability`: one `licence` element for the
underlying work (designated as public domain or released under a Creative Commons
Attribution licence), and one for Soranoha's CC0 dedication of the markup,
including a `ptr` to the full terms. The encoding grant derives from the
publication policy hashed in the release manifest, while the rights standing of
the work matches `works[].rights` in the manifest, ensuring consistency between
detached TEI files and signed release records. Validation fails under
`snh-publication-licence` when these declarations are absent. See
[rights and licensing](../../docs/rights.md).

Passing schema validation does not establish source fidelity or determine
publication rights. Details on the two validation layers appear in
[TEI validation](tei-validation.md); source fidelity is addressed in
[source accountability](source-accountability.md).

## Provenance of this document

The vocabulary is defined in
[`soranoha/schemas/tei-profile.odd`](../schemas/tei-profile.odd); the Relax NG
and Schematron are generated from it. Where this page and the ODD disagree, the
ODD is authoritative.
