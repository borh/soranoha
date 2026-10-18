# 0001: snh-manifest/2: a signed catalog, published markdown, and a rights grant

Status: accepted, 2026-09-08. Pre-genesis; no public chain carries
`snh-manifest/1`.

The snh protocol specification requires a permanent architectural decision
record for any change to its contracts. This is the first, and it establishes
`docs/design/adr/` as their location. Nothing else belongs there: a record is
written here only for a change to the protocol's contracts that has been made,
because the chain is append-only and a verifier needs the reason permanently.
Every other design decision is recorded in the issue tracker while it is open
and in the code once it is settled.

## Context

The release manifest could not describe its own contents. A work entry carried
`slug`, `source_content_hash` and artifact ids; title, author, classification
and source edition existed only inside each work's TEI. Finding a text meant
downloading a 8.95 MB manifest and then fetching up to 17308 TEI files.

The manifest had two smaller gaps. Markdown was already built for every work and
withheld from publication, although it is the most consumable format for the
audience the corpus is for. And nothing in the published bytes stated what a
reader may do with them.

The type registry is closed and explicitly versioned, so each of the three
requires the same change: one wire version increment.

## Decision

Move to `snh-manifest/2`, carrying all three changes:

1. A new release-level `catalog` artifact type (`snh-catalog/1`), referenced by
   a new required `catalog` field, describing exactly the manifest's works.
2. `markdown` becomes a per-work artifact type. Every work now publishes four
   artifacts rather than three.
3. A new required `rights` object stating the standing of the underlying works
   and the licence over Soranoha's own encoding, with a URL for the full
   statement.

The catalog is a canonical-JSON protocol object with a frozen schema, decoded
through the same boundary operation as the other four. It is release-level and
unsharded.

## Why the catalog is inside the protocol

The alternative was an index derived from TEI headers at serving or activation
time: no protocol change, ships immediately, revisable at will.

The decision rests on the archival argument. Release manifests are deposited
in Zenodo, and a manifest that cannot list its own contents is a weak archival
object. An unsigned serving index is outside the verified chain, so it
carries no archival guarantee and cannot be cited as part of a release. The
catalog is the part of the record a reader is most likely to cite and least
able to re-derive.

Size: roughly 14 MB of canonical JSON for 17308 works, projected from the
bibliographic fields the pinned catalog supplies, against the 8.95 MB manifest
that prompted this. The catalog is larger than the manifest it accompanies, and
that size determines the packaging: carried inline, those fields would be
downloaded again with every manifest, while a separate artifact is fetched only
by readers who request it.

## Why all three changes share one version increment

A second version increment on a live append-only public chain is far more
expensive than the first, and this is the last moment the first one is cheap.
Deferring markdown and rights to keep the verified surface smaller now would
probably have required a second increment later.

## Accepted consequences

- The verified surface grows: the verifier fetches and decodes one more object
  per release and checks it against the manifest's works.
- A withdrawal now publishes a new catalog blob, because a catalog that
  still described a withdrawn work would defeat the takedown. An
  event-amendment changes no work, so it reuses the head's catalog.
- The rights grant is read from the rights policy that already authorizes
  publication, and the policy's hash is already in the manifest. A policy that
  authorizes a release without stating terms now publishes nothing. Deployments
  must add `:rights-statement` to their policy document.
- `rights` joins the governance projection, so a governance event can no longer
  change the licence as a side effect of a withdrawal.

## What the catalog does not carry

Facts, not renderings. No download filename, no citation string, no DOI, no
manifest id, no release ordinal.

A rendered filename stored beside the fields it renders from can disagree with
them, in a record that cannot be corrected; filename rules
and citation forms are both derivable from what the catalog
does carry, and they live in the serving layer where formatting can be corrected. A
`manifest_id` inside the catalog would be circular, since the manifest identifies
the catalog. Content addressing also lets
consecutive releases with an unchanged corpus share one catalog blob, which an
embedded release ordinal would prevent. Keeping archive-provider references out
leaves the closed schema provider-neutral.

## Alternatives rejected

- **Unsigned serving-layer index.** Ships immediately and does not block
  genesis, but is not part of the citable record.
- **The catalog as CSV bytes**, like `tei` and `plaintext`. The file needed for
  bulk consumers would then be the signed artifact itself, but it would be the first
  release-level type with no schema and no boundary decode, governed by prose
  alone. The CSV is a serving projection instead.
- **Sharded catalog artifacts.** Permanent shard-boundary rules in the wire
  format, with no present consumer needing partial fetch.
- **Catalog-only version increment, deferring markdown and rights.** Smaller
  verified surface now, at the cost of a probable second increment later.

## Migration

None. `snh-manifest/1` never reached a public chain; public release anchors a
fresh genesis. The repository therefore ships one manifest schema, not two, and
the verifier accepts `snh-manifest/2` only. This is why the version
increment is practical today and would not be after genesis.
