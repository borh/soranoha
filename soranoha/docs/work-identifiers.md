# Work identifiers

Every published work has one identifier of the form
`<work-id>_<card-directory>`: six digits, an underscore, six digits.

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

Both parts come from Aozora Bunko. `000092` is the catalog's 作品ID (work id)
and `000879` is the contributor card directory the source archive is filed
under, as in `cards/000879/files/`. Together they point to the Aozora Bunko
card the text was taken from:

```
https://www.aozora.gr.jp/cards/000879/card92.html
```

The card URL drops the leading zeros from the work id. Nothing else differs.

## What the identifier guarantees

**It is stable across releases.** The identifier is computed from the source's
own coordinates and nothing else. Adding or removing works, reprocessing the
corpus, or changing the converter or this project's tooling leaves it as it
was.

**The identifier designates the work, rather than a particular file.** If
Aozora Bunko repackages the archive, proofreads the text again or publishes a corrected edition, the
identifier stays the same and the text under it changes. In the manifest,
`source_content_hash` is recorded for each work, computed over the canonical
identity of the source bundle rather than the raw archive bytes, allowing two
releases to be compared exactly. Cite the identifier for the work; cite the
identifier plus `source_content_hash` when the exact source edition matters.

**Withdrawal changes where it resolves.** The publication history is
append-only, but a work can be withdrawn. A withdrawn identifier no longer
appears under `/works/` and appears instead under `/withdrawn/<identifier>.json`,
the signed governance event with the public reason. The releases that included
the work keep their manifests, signatures and content; withdrawal removes the
work from the current release and never rewrites a past one.

The identifier would change if Aozora Bunko refiled the same work under a
different contributor card, since the card directory is part of it. Only that
work would be affected.

## What the identifier omits

The identifier excludes person IDs, archive filename stems, and format
variants (`ruby`/`txt`), which describe how a source was acquired rather than
which work it is.

The card directory is the Aozora Bunko contributor card folder the source
archive is filed under. For 146 works that card belongs to a contributor other
than the first author.

In the catalog this release builds from, no work id is filed under more than
one contributor card. If one were, the card directory would distinguish them.

## The identifier and the download filename

Every artifact is served at two locations. Citations use
`/works/<identifier>/<type>`, which you can build from the identifier alone.
Browser downloads and `curl -O` receive
`/works/<identifier>/<author>-<stem>-<identifier>.<ext>`. Both paths resolve to
the same bytes.

The identifier is the citable reference. The filename is a convenience that
the serving layer builds and a later release can correct. Its author part is
one contributor of a work that may have several, and its middle part is Aozora
Bunko's own archive stem. In the signed catalog, the filename components
(`archive_stem` and the romanized name parts) are recorded rather than the
assembled filename.

Filenames follow `<author>-<stem>-<identifier>.<ext>` so that works sharing an
author and title do not collide: 470 author-and-title pairs are shared by 2357
of the 17810 works in the Aozora Bunko catalog. Splitting a filename on its
hyphens yields the identifier as the last part; it is likewise included in
`catalog.csv` within the bulk archives.
