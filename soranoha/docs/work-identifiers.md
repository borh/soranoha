# Work identifiers

Every published work has one identifier of the form
`<work-id>_<card-directory>`, consisting of six digits, an underscore, and six digits:

```
000092_000879
```

It appears as `works[].slug` in each signed manifest, as the directory name in
the published routes, and as the file name of a withdrawal record:

```
https://soranoha.org/works/000092_000879/tei
https://soranoha.org/works/000092_000879/markdown
https://soranoha.org/works/000092_000879/plaintext
https://soranoha.org/works/000092_000879/tei-validation
https://soranoha.org/withdrawn/000092_000879.json
```

Both components come from Aozora Bunko. `000092` is the catalog's 作品ID
(work id) and `000879` is the contributor card directory the source archive is
filed under, as in `cards/000879/files/`. Together they identify the Aozora Bunko card
the text was taken from:

```
https://www.aozora.gr.jp/cards/000879/card92.html
```

The card URL drops the leading zeros from the work id; nothing else needs
translating.

## What the identifier guarantees

**It is stable across releases.** The identifier is computed from one source's
own coordinates and nothing else, so no change elsewhere in the corpus can change
it. Adding works, removing works, reprocessing the corpus, changing the
converter, or changing this project's own tooling all leave it exactly as it
was.

**It denotes the work, not a particular file.** If Aozora Bunko repackages the
archive, re-proofreads the text, or publishes a corrected edition, the
identifier stays the same and the text under it changes. The manifest carries `source_content_hash` per work
over the canonical identity of the source bundle rather than a raw archive hash, so
two releases can be compared exactly. Cite the identifier for the work; cite
the identifier plus `source_content_hash` when you need the precise bytes you
read.

**Withdrawal changes where it resolves.** Publication is append-only, but a
work can be withdrawn. A withdrawn identifier stops appearing under `/works/`
and appears under `/withdrawn/<identifier>.json`, which identifies the signed
governance event carrying the public reason. The releases that included the
work keep their manifests, signatures and content; withdrawal removes the
work from the current release, and never rewrites a past one.

An identifier would also change if Aozora Bunko refiled the same work under a
different contributor card, since the card directory is part of it. Only that
work would be affected; every other identifier is computed independently.

## What the identifier omits

The identifier omits person IDs, archive filename stems, and format variants
(`ruby`/`txt`), which describe how a source was acquired.

The card directory identifies the Aozora Bunko contributor card folder the
source archive is filed under; for 146 works that card belongs to a
contributor other than the first author.

In the catalog this release builds from, no work id is filed under more than
one contributor card. The card directory distinguishes a work id filed under
several.

## The identifier and the download filename

Every artifact is served at two locations. Citations use `/works/<identifier>/<type>`, which can be
constructed directly from the work identifier. Browser downloads and `curl -O`
receive `/works/<identifier>/<author>-<stem>-<identifier>.<ext>`. Both paths
resolve to the same blob.

The identifier is the citable reference. The filename is a convenience that the
serving layer renders and a later release can correct. Its author component
names one contributor of a work that may record several, and its middle
component is Aozora Bunko's own archive stem. The signed catalog records the
filename's inputs (`archive_stem` and the romanized name parts) rather than
the assembled filename.

The identifier uniquely identifies the work, and filenames follow
`<author>-<stem>-<identifier>.<ext>` so that works sharing an author and title
do not collide: 470 author-and-title pairs are shared by 2357 of the 17810
works in the Aozora Bunko catalog. Splitting a filename on its hyphens
recovers the identifier as the last component; the bulk archives also carry it
in `catalog.csv`.
