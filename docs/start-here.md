# Start here

Soranoha publishes TEI P5 editions of Japanese texts from Aozora Bunko. Each
work comes in four formats, with a signed record of exactly which bytes were
published.

This guide covers fetching one work, downloading the whole corpus, and what is
in each format.

> **Pre-genesis**: The URLs below are those of the upcoming first public
> release. Identifiers, schemas and licences are fixed. To build locally before
> then, see [the worked example](worked-example.md).

## What you get

Every work is published in four files, all reachable from its identifier:

| File | What it is |
|---|---|
| `tei` | TEI P5 XML edition containing ruby, gaiji, emphasis, indentation, headings, the source colophon, and byte offsets into the Aozora Bunko source. |
| `plaintext` | Reading text alone in UTF-8. Ruby readings, editorial notes, and apparatus remain in the TEI edition rather than this projection. |
| `markdown` | CommonMark containing the reading text with headings, ruby as inline HTML `<ruby>`, and CSS emphasis marks. |
| `tei-validation` | JSON report recording each validation layer's result, every rule finding with its severity, and the sha256 of the TEI profile the file was checked against. |

The TEI edition is the source of the other three.

## Get one text

Every work has an identifier of the form `<work-id>_<card-directory>`: six
digits, an underscore, six digits. Akutagawa Ryūnosuke's 蜘蛛の糸 is
`000092_000879`.

```sh
curl -o 000092_000879.xml https://soranoha.org/works/000092_000879/tei
curl -o 000092_000879.txt https://soranoha.org/works/000092_000879/plaintext
```

The URL ends in the artifact type and has no extension, so give `curl` an
output name. The site also serves each file under a readable name, which is
what a browser save or `curl -O` writes to disk:

```sh
curl -O https://soranoha.org/works/000092_000879/Akutagawa_Ryunosuke-kumono_ito-000092_000879.xml
```

The filename is `<author>-<stem>-<identifier>.<ext>`, where `<stem>` is Aozora
Bunko's archive stem. The identifier at the end distinguishes the 2,357 works
in the Aozora Bunko catalog that share an author and title with another work.
The filename is provided for individual downloads; the identifier is what to
cite. Both are listed on each work's page.

To read a work rather than download it, open
`https://soranoha.org/works/000092_000879/` for the bibliography and
`https://soranoha.org/works/000092_000879/read` for the text with ruby, in
horizontal or vertical setting. You can link into the text: each paragraph and
each segment has an HTML id equal to its TEI source reference (for example
`.../read#source-694-733`), which is the UTF-8 byte span of that passage in the
source text. A paragraph's id covers the whole paragraph; a paragraph without
inline markup has just that one id.

If you have an Aozora Bunko card URL, you already have the identifier. The card
`https://www.aozora.gr.jp/cards/000879/card92.html` corresponds to work id `92` and card
directory `000879`; pad each to six digits and join them, work id first:
`000092_000879`.

Identifiers remain constant between releases, designating the work itself rather
than a particular file. Details are given in
[Work identifiers](../soranoha/docs/work-identifiers.md).

## Find a text

- Search by title, kana title reading, author (Japanese or romanised), or
  identifier at `https://soranoha.org/`.
- Browse by author at `https://soranoha.org/authors/`, by title at
  `https://soranoha.org/titles/` under the first kana of the reading, or by NDC
  class at `https://soranoha.org/ndc/`.
- Fetch the bibliography of every work in the current release as one JSON file
  at `https://soranoha.org/catalog.json`. This file is part of the signed
  record.

Each catalog entry lists the identifier, the title and reading, the author and
other contributors, the first-publication details, the orthography (such as
新字新仮名), the NDC class, the Aozora Bunko card URL, the printed source
edition, and the length of the text as `body_character_count` (Unicode characters
in the plaintext, excluding line breaks, indentation, and other whitespace). Use
this count to select subsets by size or weight samples by length without
downloading the texts.

```python
import json, urllib.request

def name(person):
    # A contributor can carry one name part rather than two: 30 people in the
    # catalog have a family name and no given name (ヴォルテール, 兼好法師),
    # and the missing part is null rather than an empty string.
    return "".join(part for part in (person["family_name"],
                                     person["given_name"]) if part)

catalog = json.load(urllib.request.urlopen("https://soranoha.org/catalog.json"))
for work in catalog["works"][:5]:
    authors = [c for c in work["contributors"] if c["relation_to_work"] == "著者"]
    print(work["slug"], work["title"], "／".join(name(a) for a in authors))
```

## Get all of them

The whole corpus comes as two archives:

- `https://soranoha.org/bulk/soranoha-tei.zip`
- `https://soranoha.org/bulk/soranoha-plaintext.zip`

Author pages, NDC class pages, three-digit NDC subclass pages (such as
`https://soranoha.org/ndc/912` for 戯曲) and the children's material page
`https://soranoha.org/ndc/K` also offer TEI and plaintext ZIP archives of
their own works. So there are five sizes of archive: the corpus, a person, a
class, a subclass and the children's books.

Files within each archive are stored under human-readable names,
accompanied by `catalog.csv` at the root recording bibliographic details for
each work:

    identifier, title, subtitle, title_reading, author,
    orthographic_style, ndc, first_published, body_character_count,
    source_edition_title, source_edition_publisher, source_edition_year,
    filename, source_content_hash, url, release, doi

The fields to cite are `identifier`, `source_content_hash` and `release`. Read
`catalog.csv` with `utf-8-sig` to strip the UTF-8 byte-order mark.
`source_edition_year` is the Gregorian year extracted from Aozora Bunko's
free-form 初版発行年 string (such as `1981（昭和56）年3月20日`); the full string
appears on individual work pages.

To fetch works one by one, for example a filtered subset:

```python
import json, urllib.request, pathlib, time

catalog = json.load(urllib.request.urlopen("https://soranoha.org/catalog.json"))
out = pathlib.Path("corpus-tei")
out.mkdir(exist_ok=True)

for work in catalog["works"]:
    slug = work["slug"]
    target = out / f"{slug}.xml"
    if target.exists():
        continue
    with urllib.request.urlopen(f"https://soranoha.org/works/{slug}/tei") as r:
        target.write_bytes(r.read())
    time.sleep(0.5)
```

Please rate-limit scripted requests for single texts and use the bulk archives
for the whole corpus; the site runs on limited bandwidth with no availability
guarantee. The two archives are also deposited on Zenodo quarterly beside the
signed manifest; details on which DOI covers each deposit are provided in
[Citing Soranoha](citation.md). The permanent record is the signed release chain
and its archived copies.

## Scale and coverage

The source is Aozora Bunko's catalog of works whose copyright has expired in
Japan, limited to works with a downloadable text file: 17,655 of them in the
catalog this release builds from, by 1,167 authors, translators, editors and
collators. The numbers change as Aozora Bunko adds works and copyrights expire.
The work count for each release is stated on the landing page and in
`/catalog.json`; each release represents a fixed set of works, not a live view.

Contributors keep their Aozora Bunko roles, such as 著者, 翻訳者, 校訂者 or
編者. A work can have several contributors, and contributors are indexed under
each role they hold, rather than by authorship alone.

Two principles govern what a release contains:

**A work is published only after its rights are assessed.** A work is
admitted to a release only when rights evaluation confirms its eligibility
based on recorded evidence, by default Aozora Bunko's catalog metadata. See
[admission and assessment](user-glossary.md#admission).

**Illustrations are recorded by reference.** Where the source refers to an
illustration, the TEI record notes the reference and its caption. Aozora Bunko's
image files are outside the grant and are not published.

## Licence

The underlying works are not Soranoha's to license. Almost all are out of
copyright; the rest are works whose rightsholder publishes them on Aozora Bunko
under a Creative Commons Attribution licence. Soranoha's own contributions,
including the TEI encoding, the plaintext and Markdown projections, the
validation reports, the catalog and the manifests, are dedicated to the public
domain under [CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/).
The dedication covers what Soranoha added; the text of a CC BY work keeps its
licence in every file that contains it.

You may copy, redistribute, adapt, translate, mine and republish all of it,
commercially or not, without asking and without payment. For Soranoha's own
encoding, attribution is requested, not required. Where the underlying work is under CC BY, attribution to that work is a
condition of its licence. The applicable licence is stated on each work's
page and in its TEI header.

[Rights](rights.md) is the full statement, including what applies if you
redistribute the software rather than the corpus, and how a rights holder asks
for a work to be withdrawn. Formats for scholarly reference are provided in
[Citing Soranoha](citation.md). A citation should give both the release and the
identifier, because title and author alone do not identify a work.

## What "signed" means for a reader

Every release is a signed manifest listing each published work and the SHA-256
of each of its four files. The manifests form a chain in which each refers to
the one before, so a release cannot be altered or removed afterwards without
detection, and anyone can check that a downloaded file is the one the release
records.

Every release in the chain is listed at `https://soranoha.org/history`, newest
first, with its Aozora Bunko revisions and the counts of works added, removed
or re-encoded, computed from the published manifests.

The upstream revision a release records is a commit of Aozora Bunko's Git
repository, `https://github.com/aozorabunko/aozorabunko`. GitHub stopped
serving that repository in 2026. Software Heritage archived it in full before
then, so the revision resolves there as `swh:1:rev:<upstream_rev>`, and the
archive's last visit of the origin, snapshot
`swh:1:snp:861faf52eb29116fdb2984563a031acec1a8d6b1`, holds every revision a
release records.

To compare a release with an earlier point in Aozora Bunko's history, check
that history out and run the comparison from a checkout of the
[Soranoha repository on GitHub](https://github.com/borh/soranoha):

```sh
nix run .#soranoha-kernel -- corpus-delta \
  --chain-clone /path/to/a/clone/of/the/chain \
  --aozora-root /path/to/aozorabunko/checked/out/at/the/revision/you/mean \
  --release-pub release.pub --governance-pub governance.pub
```

This comparison lists works whose source text differs, works present in only
one revision, and works withdrawn by governance events. It examines the source
identity recorded for each work without parsing or rebuilding, verifying the
chain against published keys before checking against the local checkout.

Citing the release specifies the exact files analyzed, even after subsequent
corpus updates. Definitions of manifest, chain, and admission are given in the
[glossary](user-glossary.md); rules governing verification are set out in the
[protocol specification](design/snh-protocol-v1.md).

## Where to go next

- [A worked example](worked-example.md) traces a text from Aozora Bunko source
  to TEI and plaintext, walking through each header block with illustrative
  loader code.
- [Glossary](user-glossary.md): terms used across the project.
- [Work identifiers](../soranoha/docs/work-identifiers.md): identifier structure,
  stability across releases, and edition tracking.
- [TEI extension vocabulary](../soranoha/docs/tei-vocabulary.md): `snh:`
  attributes in published files and their validation constraints.
- [Rights](rights.md) and [Citing Soranoha](citation.md).
