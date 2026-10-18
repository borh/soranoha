# Archival, and resolving a citation through the archive

The protocol makes an archived copy of the publication repository part of what
a citation rests on. [The specification](design/snh-protocol-v1.md) defines
`archive_verification(archived_view, C, pinned_keys)`, runs it with the
Software Heritage snapshot as the sole repository view, and states that
citation eligibility requires a successful observation. It also requires a
documented recipe for mapping a published identifier to its archived location.
This is that recipe and the procedure that produces the observation.

## The archive is a second view of the same bytes, read independently

`verify_repository_at` reads every commit, tree, manifest, signature, event and
artifact through one non-fallback view. An archival observation supplies the
archived snapshot as that view and nothing else, so verification of an incomplete
archive fails rather than reading the missing objects from the live origin. What a successful
report claims is repository-closure completeness plus the public invariants,
and nothing more; it makes no claim about transcription or assessment.

The report is disposable. It records the snapshot identifier, the commit, the
pinned key fingerprints, the verifier version and the result, and it is neither
signed nor schema-frozen. Reports have no ordering contract, so there is no
"latest" one, and a later failed observation does not by itself invalidate an
earlier successful one.

## The chain repository must be publicly cloneable

Software Heritage archives an origin by cloning a provided URL. The Git
repository holding the signed chain must be reachable and anonymously
cloneable at a stable public URL.

This requirement is distinct from public HTTP serving of corpus artifacts.
The Git URL serves as the archive's origin identity across all SWHID qualifiers
and must remain stable.

The URL is:

```
https://github.com/borh/soranoha-chain
```

Anyone can clone it anonymously over HTTPS. Publishers write to the same
repository over authenticated SSH, and the repository's branch rules reject
force pushes and branch deletion, so the origin enforces the fast-forward-only
push the kernel requires. This URL
serves as the stable origin qualifier for every archived SWHID; changing it
invalidates previously recorded identifiers.

An archival observation records the authoritative origin that serializes
releases; an observation of a secondary mirror does not substitute for it.

## Making an observation

Use Software Heritage's Save Code Now and Vault, following Software Heritage's
procedures, under the project's account. Four requirements are specific to
Soranoha and fixed.

**Archive the origin, not a copy of it.** Submit the public chain URL. A
snapshot of some other clone is not an observation of the published chain.

**Cook the snapshot as a `git-bare` bundle.** The Vault produces three bundle
types, and `git-bare` is the one that reconstructs a bare Git
repository with its branches, which is the form the verifier reads.

**Check that the visit was full, not partial.** Software Heritage records a
visit as full, partial or failed, and a partial visit still produces a
snapshot. Read the visit status; the existence of a snapshot does not
establish a full visit. The time a submission takes to reach a full visit is
unmeasured.

**Verify the materialized bundle, and record the snapshot identifier with the
report.** Unpack the bundle, then run the kernel against it:

```sh
nix run .#soranoha-kernel -- archive-verify \
  --archive /absolute/path/to/unpacked-bare-repo \
  --commit "$C" \
  --release-pub /absolute/path/to/release.pub \
  --governance-pub /absolute/path/to/governance.pub
```

`--commit` is the publication commit being observed, as a full lowercase 40
character object id. The kernel's report identifies the view by its local
path; replace that path with the snapshot identifier when the report is filed.

A view that cannot be constructed at all is a failure to perform the
observation rather than an observation, and the kernel throws instead of
reporting. A readable view always produces a report, successful or failed.

Verifying the materialized bundle is what establishes that every published
artifact is present in the archive with the right bytes: the verifier reads
each one at its prescribed path in the archived view and checks it against the
manifest that lists it.

## Resolving a citation through the archive

A citation identifies a release head and a work. Reaching the archived bytes
takes three steps, none of which need the live site, and one distinction to
keep.

**Map the identifier to an in-repo path.** The chain's layout is fixed:

| Identifier | Path in the repository |
|---|---|
| Release head `<hex>` | `releases/<hex>.json`, with its signature at `releases/<hex>.sig` |
| Artifact id `snh:1:<type>:<sha256>` | `blobs/sha256/<first two characters>/<sha256>` |
| Governance event `<hex>` | `governance/<hex>.json` |
| Current head pointer | `releases/HEAD` |

**Find the publication commit.** A manifest is read at the commit that
published it, which is the one whose `releases/HEAD` contains its head. Walk
parents back from the archived branch tip until `releases/HEAD` reads the head
you want. That the walk terminates at the right place is checkable as you go:
each manifest's `prev_manifest` records the head its parent commit carried, so
the two walks agree at every step or the chain is broken.

**Identify the archived object.** With the commit `C`, the snapshot `S` and the
origin URL, a fully qualified identifier for one file is:

```
swh:1:cnt:<git blob id>;origin=https://github.com/borh/soranoha-chain;visit=swh:1:snp:<S>;anchor=swh:1:rev:<C>;path=/blobs/sha256/8b/8b2595...
```

**Do not confuse the two digests.** A Soranoha artifact id carries the sha256
of the artifact's own bytes. A Software Heritage content identifier carries a
Git blob hash, which is a sha1 over a different byte sequence. The two never
match and neither is derivable from the other. The sha256 selects the path; the
archive resolves the path to its own identifier.

## Resolving the upstream provenance pointers

Every manifest records where the source corpus came from as
`corpus.upstream_origin` and `corpus.upstream_rev`. Because the upstream GitHub
repository is not served, those fields resolve through the Software Heritage
archive.

The origin URL serves as the archive's repository identifier rather than an active
endpoint:

| Manifest field | Software Heritage identifier |
|---|---|
| `corpus.upstream_origin` | `swh:1:ori:<sha1 of the URL>`, resolved by `/api/1/origin/<url>/get/` |
| `corpus.upstream_rev` | `swh:1:rev:<upstream_rev>`, the same hex, resolved by `/api/1/revision/<rev>/` |

The revision identifier needs no translation. Git revision hashes *are*
Software Heritage revision identifiers, unlike the artifact ids above, whose
sha256 has to go through a path.

For the upstream corpus this project pins, that resolves today to
`swh:1:ori:6adb5c82f6b8412b1a978506696d9288eac300ab`, whose full visits reach
2026-08-14, and `swh:1:rev:0e9ea3e586eb0aa34039fabfc85a407d2f98b165`, archived
with its tree at `swh:1:dir:dcf629b24001d78ff4e324d9766948e867b89f0e`.

A qualified identifier for one upstream file, which is what a reader following
a provenance pointer usually needs, uses the revision as its anchor:

```
swh:1:cnt:<git blob id>;origin=https://github.com/aozorabunko/aozorabunko;anchor=swh:1:rev:<upstream_rev>;path=/cards/000148/files/1046_ruby_4521.zip
```

The origin qualifier is a persistent archive identifier that remains valid after
the repository stops serving. The manifest field records the original origin
URL, and the recorded revision identifies the exact source the corpus was
compiled from.

## Where this sits in the genesis sequence

An archival observation needs a publication commit to observe, so it follows
the first signed release rather than preceding it. The
[key ceremony](key-ceremony.md) and the trust anchor both come first: they
establish which keys the verifier pins.

The order is: keys, anchor deposit, ORCID entry, first signed release, public
chain URL, archival observation. Only the archival observation makes a release
citation-eligible under the specification, and it requires the public chain
URL, so that URL is required at launch.

## When to observe again

An observation identifies one commit but covers the chain behind it. Verification
walks from that commit back to genesis through the archived view, so a single
successful observation establishes every release up to it, and a release
published afterwards becomes covered by the next observation rather than
needing one of its own. Observations are therefore periodic rather than
per-release.

Observing the head at each [quarterly deposit](zenodo-deposits.md) pairs the
two independent records: the deposit holds one release's manifest
bytes and the archive holds the whole repository they came from. The interval
sets how long a new release waits to become citable.

## See also

- [The snh protocol specification](design/snh-protocol-v1.md), whose archival
  section defines the observation and the eligibility rule.
- [Zenodo deposits and ORCID anchoring](zenodo-deposits.md), the other
  independent record. Its deposits hold loose files, and `archive-verify`
  reads a repository.
- [Citing Soranoha](citation.md), for what a citation carries.
