# Glossary

Terms used across Soranoha's files, release records and encoding.

## admission

The decision to include a work in a release. A work is admitted when its
rights standing has been established on a recorded basis and that basis still
holds when the release is published.

Admission and validation are separate: a work can pass validation without
being admitted, and an admitted work is not guaranteed to be encoded well. The
evidence supporting admission is recorded in each release manifest.

## assessment

The evaluation that produces a work's admission basis: what is claimed about
its rights standing, on what evidence, from what date, and in which
jurisdiction (always Japan).

Assessments record facts together with their premises, rather than permanent
verdicts. Each fact specifies what was established, from which evidence, and as
of which date. If the evidence can no longer be reached, the fact becomes
unavailable, so a work can drop out of a release without anyone making a new
claim against it. See [reliance](#reliance) for the basis most works use.

## reliance

The usual admission basis: Aozora Bunko's own copyright-expired classification
of a specific edition, recorded as Aozora Bunko's assertion together with the
raw bytes of the pages it was read from. A recorded exception or a conflicting
reviewed finding prevents reliance.

Reliance is checked again against the live Aozora Bunko pages at release time.
The retained bytes do not replace that check.

## fidelity

How much of the source markup survives into the encoding. Fidelity is about
markup only: rights come from assessment and schema conformance from
validation.

Each build produces two fidelity reports:

- **source accountability**: a scan of the raw Aozora Bunko text for
  recognisable markup, made without the parser, with the exact byte span of
  each occurrence.
- **interpretation coverage**: a comparison of that scan with the converted
  output, counting claimed and unaccounted annotations per family and listing
  every unaccounted one in full.

Fidelity reporting verifies that a marker was present and recognised, not that
it was interpreted correctly. Ambiguities are recorded in the published TEI as
`interpretation-problem` notes.

## work, edition, document

Three different things:

- **work**: the text as an intellectual work (for example 蜘蛛の糸). Works
  are designated by Soranoha identifiers.
- **edition**: a particular source of the work, such as a printed edition or
  the Aozora Bunko transcription made from it. `source_content_hash` in the
  manifest and `<sourceDesc>` in each TEI file identify the edition.
- **document**: a particular byte sequence, identified by its SHA-256 digest.
  Two releases of the same work from the same edition may produce different
  documents as the conversion tooling changes.

Cite the identifier for the work, the identifier plus `source_content_hash`
for the edition, and the artifact id for exact bytes. See
[work identifiers](../soranoha/docs/work-identifiers.md) and
[citing Soranoha](citation.md).

## artifact

One published file, identified by its content as `snh:1:<type>:<sha256>`. Each
work has four: `tei`, `plaintext`, `markdown` and `tei-validation`.

An artifact ID can be checked by computing the SHA-256 of the file locally.

## manifest

The signed record of one release. In the manifest, every admitted work is
listed with its identifier, `source_content_hash`, and the ID and byte length of
each of its four artifacts, together with the rights grant, admission evidence,
release catalog and digest of the parent manifest.

The manifest is the authoritative record; site pages, indexes and reading
views are generated from it. `https://soranoha.org/releases/latest` serves the
current manifest.

## chain

The sequence of manifests, each referring to the one before it, back to the
first release, the genesis. Nothing is removed from it.

Earlier releases do not change. A withdrawal removes a work from current
distribution; it does not alter earlier manifests.

Publication and governance use different signing keys, and the governance keys
are kept offline. See the [protocol specification](design/snh-protocol-v1.md).

## release

One published state of the corpus: a manifest, its signature and the artifacts
it lists. A release is identified by the SHA-256 of its manifest, the release
head; citing the head specifies that exact state.

## catalog

The bibliography of every work in a release: identifiers, titles, readings,
contributors, first publication, orthographic style, NDC class, Aozora Bunko
card URL and source edition. Published at `https://soranoha.org/catalog.json`.

Bibliographic facts are recorded in the catalog, excluding presentation-level
elements such as download filenames or formatted citations.

## withdrawal

Removing a work from current distribution by a signed governance event that
records a reason code and a public statement.

A withdrawn work is removed from `/works/<identifier>/` and from the active
catalog. Earlier signed manifests continue to list it, and its artifacts remain
addressable by hash. The withdrawal record is served at
`https://soranoha.org/withdrawn/<identifier>.json`.

For rights inquiries or withdrawal requests, see [rights](rights.md).

## body-v1

The reading policy that defines the plain reading text. The plaintext
projection, the site's reading view and external annotation layers all use it.

The policy specifies deterministic resolution across the TEI body: selecting
the apparatus lemma rather than the variant, the corrected or regularised
reading rather than the source original, the base text rather than ruby
readings, and omitting editorial notes and figure descriptions. Unresolved
gaiji are excluded from annotatable spans rather than replaced.

Because the name includes a version designation, annotations can specify the
exact policy used to determine their offsets and be verified accordingly.

## gaiji (外字)

A character the Aozora Bunko source file could not encode, written as a
description in words, usually with a JIS X 0213 code point:
`※［＃「特のへん＋廴＋聿」、第3水準1-87-71］`.

The published TEI keeps both the resolved character and the original marker
verbatim, so a resolution can be checked or disputed. See
[the worked example](worked-example.md#encodingdesc-taxonomies-and-gaiji-declarations).

## ruby (ルビ)

A reading printed beside or above base characters (furigana). Written
`蓮池《はすいけ》` in the source; in TEI, `<ruby>` containing `<rb>` (base) and
`<rt>` (reading).

Ruby readings are present in the TEI edition and excluded from plaintext
projections, which retain base text only.

## source offset

A byte range into the UTF-8 decoding of the Aozora Bunko source text, recorded
for almost every element of the published TEI.

The text the offsets index is identified once per document: `primary-text-hash`
in `<sourceDesc>` is the SHA-256 of the Aozora Bunko text file as distributed,
and the offsets index that file's UTF-8 decoding. If the hash does not match,
the offsets do not index those bytes.

## snh

The publication protocol: the format of manifests, governance events,
admission evidence and artifact IDs, and the rules a verifier applies. Also the
prefix of the extension vocabulary in published TEI
(`https://w3id.org/soranoha/ns/tei`).

Use it to verify the corpus yourself or to build tools that conform. See the
[protocol specification](design/snh-protocol-v1.md) and the
[TEI extension vocabulary](../soranoha/docs/tei-vocabulary.md).
