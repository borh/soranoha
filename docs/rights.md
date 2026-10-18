# Rights and licensing

This statement is identified by <https://w3id.org/soranoha/rights>. That
identifier is recorded in `rights.statement_url` in every signed release
manifest, and referenced from `publicationStmt/availability` through
`licence/ptr/@target` in every published TEI file. The `w3id.org/soranoha/`
redirects must be registered before the first release; the identifiers here are
written as the signed bytes contain them.

## Permitted use in brief

You may copy, redistribute, adapt, translate, mine and republish everything
Soranoha publishes, for any purpose, commercial or not, without asking and
without paying. For almost every work, attribution is requested, not required.
For the works whose rightsholder publishes them under a Creative Commons
Attribution licence, attribution is a condition of that licence. Which of the
two applies is declared on each work's page and in its TEI header.

If you use the corpus in research, please cite it (see
[citation](citation.md)). That is scholarly practice, not a licence condition.

## Two distinct rights layers

Each published work combines two sets of rights.

**The underlying works** are texts from [Aozora
Bunko](https://www.aozora.gr.jp/), in one of two states. Almost all of them
are works whose copyright has expired. Soranoha holds no rights in those and
claims none. Aozora Bunko's [handling rules](https://www.aozora.gr.jp/guide/kijyunn.html)
allow copyright-expired files to be copied, redistributed, performed and
adapted freely, without permission or fee. The text itself may be altered
within Article 20(2)(iv) of the Copyright Act, which covers format conversion,
notation changes and modernising 旧字旧仮名. Aozora Bunko's guidelines request that contributor
credits be retained and textual changes recorded. Soranoha keeps the credits in
each TEI `<back>` and publishes every work with its validation report and
recorded divergences.

The rest are works still in copyright, published on Aozora Bunko by their
rightsholder under a Creative Commons Attribution licence. In Aozora Bunko's
catalog, the presence of subsisting copyright is noted without detailed terms,
so the applicable licence is read from the notice the rightsholder wrote into
the colophon. Soranoha publishes such a work only if attribution is its sole
condition. Works with licences stipulating NonCommercial, NoDerivatives, or
ShareAlike restrictions are excluded during selection and do not enter the corpus.
The state each work was admitted under is recorded with it: in `works[].rights`
in the manifest, in the catalog, and in the work's own TEI header.

Soranoha publishes only works its assessment finds free to publish on these
terms. The reasoning is in
[`soranoha/docs/evidence/aozora-rights-source-contract.md`](../soranoha/docs/evidence/aozora-rights-source-contract.md).

**Soranoha's encoding** (TEI markup, plain text and Markdown versions,
validation reports and release manifests) is dedicated to the public domain
under [CC0 1.0 Universal](https://creativecommons.org/publicdomain/zero/1.0/).
Any copyright or database right that arises in the encoding is waived. The
waiver covers only what Soranoha added. In a CC BY work, the text inside the
TEI, plain text and Markdown files stays under that licence, and its
attribution condition applies to any use of those files.

The catalog has two layers too. Its structure, the identifiers Soranoha
assigns, and the source-edition and validation fields are Soranoha's CC0
encoding. Its bibliographic data comes from Aozora Bunko's catalog CSV under
CC-BY-4.0, so redistributing the catalog, or lists built from it, requires
crediting Aozora Bunko as the source.

Scholarly credit is requested through `CITATION.cff` and ordinary citation
practice (see [citation](citation.md)).

## What covers what

| Scope | Licence | Where |
|---|---|---|
| Underlying works | Public domain (copyright expired), or CC BY where the rightsholder licensed it | Not Soranoha's to license; recorded per work as `works[].rights` |
| Soranoha's encoding in the TEI, plaintext, Markdown, validation reports, catalog and manifests | [CC0-1.0](../LICENSE-CC0) | Published bytes; the text of a CC BY work inside them keeps that licence |
| TEI customisation: `soranoha/schemas/tei-profile.{odd,rng,sch}` | [CC0-1.0](../LICENSE-CC0) | Repository |
| Protocol JSON Schemas: `soranoha/resources/snh/schemas/` | [CC0-1.0](../LICENSE-CC0) | Repository |
| Locally authored source code | [Apache-2.0](../LICENSE) | Repository |
| Forked parser core, eight crates | MIT OR Apache-2.0 | `ab-validator/crates/`, listed below |
| Data tables | Per-file source provenance and terms | `ab-validator/crates/*/data/` |
| Aozora Bunko catalog CSV (bibliographic metadata) | CC-BY-4.0 | Upstream metadata source |

The encoding vocabulary and protocol schemas are CC0. Soranoha's own source
code is Apache-2.0. The forked parser crates keep upstream's dual
MIT/Apache-2.0 licence.

Each data table in `ab-validator/crates/*/data/` records its source and terms in
its file header. The gaiji annotation dictionary (`外字注記辞書`) mapping is
CC0. The JIS X 0213 code tables follow Project X0213's permissive terms. The
gaiji alias mappings are Soranoha's own. The Aozora Bunko catalog CSV used for
bibliographic data is published by Aozora Bunko under CC BY 4.0.

## The toolchain

The parser core under `ab-validator/crates/` is forked from
[P4suta/aozora](https://github.com/P4suta/aozora) (revision
`1a4f864603970983719655aa4af4525958ac2d38`), licensed MIT OR Apache-2.0.
Upstream licence files, notices and source headers are kept.

Eight crates are forked: `ab-aozora-corpus`, `ab-aozora-encoding`,
`ab-aozora-facade`, `ab-aozora-pipeline`, `ab-aozora-render`, `ab-aozora-spec`,
`ab-aozora-syntax`, and `ab-notation-strategies` (forked from `aozora-proptest`).
The CLI binary `ab-aozora` is Soranoha's own.

Redistributing published corpus files is governed by the CC0 dedication of the
encoding and, for a CC BY work, that licence's attribution condition;
public-domain works carry no licence restrictions. Redistributing the toolchain
is governed by the toolchain's own licences.

## Machine-readable form

Each published file carries its own terms, so it can still be read correctly
once detached from this site.

In every signed release manifest, Soranoha's grant over its own layer is
recorded as:

```json
"rights": {
  "encoding": "CC0-1.0",
  "statement_url": "https://w3id.org/soranoha/rights"
}
```

and, beside each work, the state the work was admitted under:

```json
"works": [
  { "slug": "000092_000879", "rights": "public-domain" },
  { "slug": "054333_001657", "rights": "CC-BY-2.1-JP" }
]
```

Every published TEI file has one `licence` element per rights layer. In the
example below the underlying work is out of copyright, so the first `licence`
is the Creative Commons Public Domain Mark; for a CC BY work it identifies that
licence instead with attribution as its condition. The second `licence` is
CC0 in both cases. For a public-domain work it dedicates the encoding and the
derived files outright, as below; for a CC BY work it waives Soranoha's rights
only and records that the text in every file stays under the licence above it:

```xml
<publicationStmt>
  <!-- publisher, idno, and date omitted -->
  <availability status="free">
    <licence target="https://creativecommons.org/publicdomain/mark/1.0/">The
      underlying work is in the public domain; Soranoha asserts no rights over
      it.</licence>
    <licence target="https://creativecommons.org/publicdomain/zero/1.0/">Soranoha's
      encoding of this work and the artifacts derived from it are dedicated to
      the public domain under CC0-1.0. Attribution is requested, not required.
      Full rights statement: <ptr target="https://w3id.org/soranoha/rights"/></licence>
  </availability>
</publicationStmt>
```

Both forms come from `soranoha/data/publication-policy.edn`, which is hashed
into each manifest as `admission.policy_hash`. The signed manifest is
authoritative. Under the TEI Schematron rule `snh-publication-licence` in
`soranoha/schemas/tei-profile.sch`, any file whose
`publicationStmt/availability/licence` lacks a `@target` attribute is rejected.

## If you hold rights in a published work

To report a copyright concern or ask for a work to be removed, write to the
maintainer via <https://orcid.org/0000-0003-2246-8774> with the work
identifier (e.g. `000092_000879`) or the Aozora Bunko card number, and the
basis of the claim. Informal notifications are accepted.

A withdrawal is recorded as a signed governance event. Each event carries a
public `statement` and one of four `reason_code` values: `rights` for a claim
that copyright subsists, `takedown-request` for a removal asked for on other
grounds, and `data-defect` or `other` for withdrawals that are not claims.
Publishing the signed event creates an immediate release, without waiting for a
scheduled corpus cycle.

- The work is removed from `works[]` and the active catalog, so its files
  are no longer served at `/works/<identifier>/`: a manifest may not reference
  a catalog describing a work withdrawn in that release.
- The work is listed in `withdrawn[]`, pointing to the signed governance event
  served at `/withdrawn/<identifier>.json`.
- Earlier releases that contain the work stay in the signed chain. Withdrawal
  stops current distribution without rewriting past releases.
- A withdrawal statement can be amended with an `event-amendment`, but an
  amendment cannot undo the withdrawal.

See [`design/snh-protocol-v1.md`](design/snh-protocol-v1.md) and
`soranoha/resources/snh/schemas/snh-governance-event-1.schema.json`.

## No warranty

Soranoha publishes transcriptions alongside validation reports and recorded
divergences, without warranty of completeness or accuracy. The corpus is
provided as is; do not rely on it for legal, medical, or safety purposes
without verifying against the source edition specified in the TEI
`sourceDesc`.
