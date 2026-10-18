# 0004: snh-catalog/2: the length of each text

Status: accepted, 2026-10-18. Owner decision, taken pre-genesis. No public lineage carries
`snh-catalog/1`: the publication architecture requires a fresh reviewed genesis
for a public lineage, so this version replaces the first rather than succeeding
it, as `snh-manifest/3` replaced `snh-manifest/2`. Verifiers conforming to
this specification accept a single catalog version. The experimental lineage at
the current chain origin holds `snh-catalog/1` blobs, which are no longer decoded
at this revision.

## Context

A catalog entry carries what a reader needs to find and cite a work without
fetching its TEI, and nothing about its size. A reader choosing a subset by
length, weighting a sample by it, or estimating the cost of a corpus-scale task
had to download every plaintext and count. The consumer is a university
course: one class multiplies measured minutes per item by the size of the
corpus a student chooses, two others sample paragraphs across an author's
works and across the plays in proportion to length. The plaintext archive is
248 MB.

The catalog schema is closed with `additionalProperties: false`, so a new
member is a new wire version.

## Decision

Wire version 2, with one structural change. Each `works[]` entry includes
`body_character_count`, required, an integer of at least 0: the number of
Unicode code points in the work's published plaintext projection that are not
White_Space characters.

- Whitespace is excluded because line breaks, indentation and the ideographic
  space are layout. The plaintext projection restores first-line indentation
  that the TEI encodes as an attribute, and a layout-only change to the
  projection must not change a work's recorded length. Under this definition
  the count over the plaintext equals the count over the `body-v1` text view
  the projection is produced from, so the catalog, the plaintext and the
  annotation view agree.
- U+FFFC, which is substituted for an unresolved glyph in the projection, is
  counted. It stands for one character of the text that could not be encoded,
  and dropping it would make a work with many gaiji shorter than it is.
- Code points, not UTF-8 bytes or UTF-16 units: a count of characters is what
  a reader asks for, and a byte count already exists as the artifact's size.
- The count is calculated during the plaintext stage while the text is in memory
  and recorded in its projection report, a small JSON blob in the content store.
  The assembler accesses that report by hash, matching how metadata records are
  read, without reading payload blobs: publication must not materialize the
  corpus, a contract verified by test. Because the report and published
  plaintext derive from the same stage output, the two remain in exact
  agreement.

In `catalog.csv` within each bulk archive, the same column is added after
`first_published`.

## What does not change

Every other member, the sort and uniqueness rules, the manifest's reference to
the catalog by content id, and the verifier's cross-checks are unchanged.
`trailing_bytes_after_archive` stays optional. The plaintext projection itself
is unchanged; the count is a reading of it. The plaintext stage version
moves from 7 to 8 with the addition of this member to its projection report.

## Alternatives rejected

- **A sidecar file beside the catalog.** The reader fetches two files and must
  join them, and the manifest would need a slot for the second, which is a
  manifest wire version. The catalog is designed so that a single fetch
  provides this information.
- **Count the source bytes.** Although inexpensive given that the source hash is
  already recorded, the source text includes ruby readings, notes, and markup
  absent from the published text; the resulting number would therefore not match
  the downloaded text.
- **Count whitespace too.** Equals the plaintext's length in code points,
  which is simpler to state, but it changes whenever the projection's layout
  changes and differs from the `body-v1` view by the restored indentation.
- **Leave it to the TEI header's `extent`.** One fetch per work, which is the
  cost this field removes.
