# Citing Soranoha

Soranoha's encoding of each work, and the metadata around it, are dedicated to
the public domain under CC0, so citing Soranoha is requested as a scholarly
norm rather than enforced as a licence condition. Where the underlying work is
under CC BY, attribution to that work is a separate licence obligation (see
[rights](rights.md)).

## Cite a release, not "the corpus"

Soranoha is versioned, and the corpus changes between releases: works are
added as their assessment completes, encoding improves, and a work can be
withdrawn. A citation that gives only "Soranoha" does not identify the bytes
you read.

Every release is identified two ways, and a citation should carry the ones
its context allows:

| Identifier | Form | Where |
|---|---|---|
| Release head | 64-character hex | `https://soranoha.org/releases/HEAD` |
| Version DOI | `10.5281/zenodo.<n>` | The quarterly Zenodo snapshot, when the release you read is one |

The head is the identifier that always exists, and it is what a reader uses to
re-verify the chain independently. Deposits are quarterly rather than per
release, so a release published between two snapshots has no DOI of its own;
carry the head alone in that case. When the release you read does carry a DOI,
put it in the bibliography too: it resolves to exactly the bytes cited, and it
remains resolvable if the site is gone.

The head is the sha256 of the release manifest's own canonical bytes, so it
identifies the manifest as well as the release: it identifies `releases/<head>.json`
in the chain and is the `manifest_id` the publishing command reports. There is
no third identifier to cite.

The forms below are templates: they show where each component goes. The same
forms filled in for the release you are reading are on the served
[citation page](https://soranoha.org/citation), and every work page carries
its own, ready to copy.

## A release is citable once it has been archived

A release is citable once a successful independent archival observation
covers its publication commit. The observation reads the Software Heritage
archived copy of the publication repository as its only source and re-runs the
full chain verification against it.

An observation covers more than the one release it identifies. Verification walks
the whole chain back to genesis through the archived copy, so an observation
that succeeds at one commit establishes that every release up to that commit is
in the archive and passes verification. A release published after the last observation
is not yet covered; it becomes covered by the next observation of any later
commit, without anything being redeposited.

Because the observation reports carry no ordering contract, a later failed
observation does not by itself revoke an earlier successful one.
[Archival](archival.md) describes how an observation is made and how to resolve
a citation through the archive without the live site.

## Citing the whole corpus

```
Hodošček, Bor. Soranoha Aozora Bunko TEI Corpus. Release
<release head, 64 hex characters>.
https://doi.org/10.5281/zenodo.<version deposit>
```

BibLaTeX:

```bibtex
@dataset{soranoha,
  author  = {Hodošček, Bor},
  title   = {Soranoha Aozora Bunko TEI Corpus},
  version = {<release head, 64 hex characters>},
  doi     = {10.5281/zenodo.<version deposit>},
  url     = {https://soranoha.org},
}
```

`CITATION.cff` in the repository root carries the corpus-level record in
machine-readable form, and GitHub-compatible forges and Zenodo read it
directly. It describes the corpus rather than one release and carries neither
the head nor the version DOI; fill them in from the release you read.

To cite the corpus across releases rather than one of them, use the Zenodo
*concept* DOI, which always resolves to the latest version. Do not use it when
reproducibility matters: it does not identify specific bytes.

## Citing one work

A work needs both a readable description and its identifier: measured over the
Aozora Bunko catalog this release builds from, author plus title leaves 2357 works
ambiguous, and author, title, 副題, 文字遣い種別, 底本名 and 初出 together
still leave 16. Those counts change with the upstream catalog. The work
identifier, such as `000092_000879`, disambiguates all of them and is stable
across releases; see
[work identifiers](../soranoha/docs/work-identifiers.md).

Every work page carries both forms below, ready to copy, and offers the same
record as CSL-JSON and BibLaTeX. Everything shown here is generated from the
work's TEI header, which is the authoritative bibliographic record; the
catalog and every citation format are projections of it.

### For a Japanese bibliography

```
芥川 竜之介「蜘蛛の糸」（新字新仮名）、底本『芥川龍之介全集2』ちくま文庫、筑摩書房、
1986年。Soranoha Aozora Bunko TEI Corpus, 000092_000879,
release <first 12 characters of the head>.
https://doi.org/10.5281/zenodo.<version deposit>
```

### For an English bibliography

```
Akutagawa Ryunosuke. “蜘蛛の糸” (新字新仮名). In 芥川龍之介全集2.
ちくま文庫、筑摩書房, 1986. Soranoha Aozora Bunko TEI Corpus, 000092_000879,
release <first 12 characters of the head>.
https://doi.org/10.5281/zenodo.<version deposit>
```

The author is in Latin script, from the catalog's romanized name parts; the
titles stay in Japanese (see [Romanization](#romanization)).

Citations cite the 12-character release prefix, which resolves at
`https://soranoha.org/releases/short/<prefix>.json` to the same manifest as
the full 64-character name: for example,
`https://soranoha.org/releases/short/8b259c55e63a.json`. The machine-readable
forms record the full 64-character hash: `version` in BibLaTeX, the `Release`
sentence in the CSL note, and the `release` column of `catalog.csv`.

### What each component is for

| Component | Example | Why it is there |
|---|---|---|
| Author | `芥川 竜之介` / `Akutagawa Ryunosuke` | Who wrote it. Not sufficient to identify the work. |
| Title, with 副題 | `蜘蛛の糸` | What it is. Not sufficient either: 2357 works share an author-and-title pair with another work. |
| 文字遣い種別 | `新字新仮名` | Which orthographic transcription. Aozora Bunko often publishes the same work in two, and they are different texts. |
| 底本 | `『芥川龍之介全集2』ちくま文庫、筑摩書房、1986年` | The printed book the transcription was made from. Two transcriptions of one work from different 底本 are different texts. |
| Corpus name | `Soranoha Aozora Bunko TEI Corpus` | Which corpus, distinguishing this encoding from Aozora Bunko's own files. |
| **Work identifier** | `000092_000879` | **Identifies the work.** Stable across releases. The one component that makes the citation unambiguous. |
| **Release** | `release <first 12 characters of the head>` | **Identifies the bytes.** The corpus is versioned; a citation without it does not identify what was read. |
| Version DOI | `https://doi.org/10.5281/zenodo.<version deposit>` | Resolves to exactly those bytes, and remains resolvable if the site is gone. Present only when the release you read is a quarterly snapshot. |

The identifier and the release are the two components that are never
optional. The other components describe the work; those two identify it.

### Machine-readable forms

Each work serves the same record in the formats a reference manager reads:

| Route | Format | For |
|---|---|---|
| `/works/<id>/citation.json` | CSL-JSON | Zotero, Pandoc, citeproc |
| `/works/<id>/citation.bib` | BibLaTeX | LaTeX written directly |

Work pages also embed [COinS](https://en.wikipedia.org/wiki/COinS), so the
Zotero browser connector saves a correctly typed record in one click without
site-specific configuration.

All three type a work as an item inside its 底本 rather than as a book of its
own: CSL `chapter`, BibLaTeX `@incollection`, COinS `genre=bookitem`. A
bibliography built from any of them keeps the source edition.

```bibtex
@incollection{soranoha-000092_000879,
  author = {芥川, 竜之介},
  title = {蜘蛛の糸},
  titleaddon = {くものいと},
  booktitle = {芥川龍之介全集2},
  publisher = {ちくま文庫、筑摩書房},
  date = {1986},
  language = {japanese},
  langid = {japanese},
  eprinttype = {Soranoha Aozora Bunko TEI Corpus},
  eprint = {000092_000879},
  version = {<release head, 64 hex characters>},
  addendum = {Release <release head, first 12 characters>},
  url = {https://soranoha.org/works/000092_000879/},
  doi = {10.5281/zenodo.<version deposit>},
  note = {新字新仮名; 初出: 「赤い鳥」1918（大正7）年7月},
}
```

The identifier uses BibLaTeX's archive idiom: `eprinttype` gives the
collection and `eprint` the item within it, and standard bibliography styles
print both.

The release appears in two fields. `version` holds the whole head hash, which
biber and Zotero import; standard styles do not print it for an
`@incollection`. `addendum` holds the 12-character release prefix, which every
standard driver prints at the end of the entry. The note also repeats the
original title of a translated work, which `origtitle` imports but standard
styles do not print.

CSL-JSON records 校訂者 as `contributor`, and BibLaTeX records it as `editora`
with `editoratype = {reviser}`. The Japanese role is on the work page and in
the TEI header.

For many works at once, every [bulk archive](start-here.md) carries a
`catalog.csv` holding the same fields as columns, so a whole selection
becomes a bibliography in a spreadsheet.

### The edition year

`date = {1986}` is extracted, not recorded. Aozora Bunko's 底本初版発行年 is a
free-form publication history rather than a year: 17747 of the 18780 recorded
values read like `1981（昭和56）年3月20日`, and of the 1033 that do not, 667
append a printing history, as in
`1948（昭和23）年5月15日、1963（昭和38）年5月16日第20刷改版`. The first
Gregorian year is taken, which is the field's own meaning: where several years
appear, the earliest is the first edition's.
Work pages show the recorded string in full; only the machine-readable fields
carry the extracted year.

### What CSL cannot carry

CSL has no field for kana readings, and multiscript support in CSL and Zotero
remains limited; 作品名読み is recorded in `note`. BibLaTeX stores it in
`titleaddon`. The authoritative copy remains in the TEI header and catalog JSON.

### Romanization

Citation formats give titles in Japanese. Romanize the title according to the
target style guide (Hepburn, ALA-LC). Download filenames use Aozora Bunko's
hand-curated archive stems. Orthographic classifications (`文字遣い種別`) are
retained in Japanese.

## Citing an exact byte sequence

For work that must be reproducible against specific bytes rather than a
release, cite the artifact by content address. Every published artifact has an
id of the form `snh:1:<type>:<sha256>`, and the manifest that lists it is
signed:

```
Soranoha Aozora Bunko TEI Corpus, work 000092_000879, TEI artifact
snh:1:tei:6f0b1c…, in release
<release head, 64 hex characters>.
```

This is the most precise form of citation available. A reader can fetch the artifact, hash
it, find that hash in the manifest, and verify the manifest's signature
against the release key published in the trust anchor deposit. The procedure
is specified in [`docs/design/snh-protocol-v1.md`](design/snh-protocol-v1.md),
and [Zenodo deposits and ORCID anchoring](zenodo-deposits.md) describes what
each deposit contains and how to check that a DOI identifies the release its deposit states.
To reach the same artifact through the archive rather than through this site,
[archival](archival.md) gives the path mapping.

## Citing the tooling

The conversion and publication codebase is licensed separately from the corpus
(see [rights](rights.md#the-toolchain)). Cite the tooling when research depends
on the conversion pipeline or parser implementation rather than the corpus text.
