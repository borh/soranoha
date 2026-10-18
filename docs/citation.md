# Citing Soranoha

Soranoha's encoding of each work, and the metadata around it, are public
domain under CC0, so citing Soranoha is asked for as scholarly practice, not
required by licence. Where the underlying work is under CC BY, crediting that
work is a separate licence obligation (see [rights](rights.md)).

## Cite a release, not "the corpus"

The corpus changes between releases: works are added as their assessment
completes, encoding improves, and a work can be withdrawn. A citation consisting
only of "Soranoha" does not identify which version of a text you used.

Every release has two identifiers; give the ones your context allows:

| Identifier | Form | Where |
|---|---|---|
| Release head | 64-character hex | `https://soranoha.org/releases/HEAD` |
| Version DOI | `10.5281/zenodo.<n>` | The quarterly Zenodo snapshot, when the release you read is one |

The head always exists, and it is what a reader uses to verify the chain
independently. Deposits are made quarterly, not per release, so a release
published between two snapshots has no DOI of its own; give the head alone in
that case. When the release you read does have a DOI, put it in the
bibliography as well: it resolves to exactly the bytes cited, and it still
resolves if the site is gone.

The head is the sha256 of the release manifest's canonical bytes, so it
identifies the manifest as well as the release: it is `releases/<head>.json`
in the chain and the `manifest_id` the publishing command reports. There is no
third identifier.

The forms below are templates showing where each part goes. The same forms
filled in for the release you are reading are on the served
[citation page](https://soranoha.org/citation), and every work page has its
own, ready to copy.

## A release is citable once it has been archived

A release becomes citable once an independent archival observation succeeds
on its publication commit. The archival observation inspects only the Software
Heritage copy of the publication repository, rerunning full chain verification
against it.

Verification walks the whole chain back to genesis through the archived copy,
so an observation that succeeds at one commit confirms that every release up to
that commit is in the archive and verifies. A release published after the last
observation is not yet covered; the next observation of any later commit
covers it, with nothing redeposited.

Observation reports are not ordered, so a later failed observation does not by
itself cancel an earlier successful one. [Archival](archival.md) describes how
an observation is made and how to resolve a citation through the archive
without the live site.

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
machine-readable form; GitHub-compatible forges and Zenodo read it directly. It
describes the corpus, not one release, and has neither the head nor the
version DOI; fill those in from the release you read.

To cite the corpus across releases rather than one of them, use the Zenodo
*concept* DOI, which always resolves to the latest version. Do not use it when
reproducibility matters: it does not identify specific bytes.

## Citing one work

A work needs both a readable description and its identifier. In the Aozora
Bunko catalog this release builds from, 2,357 works remain ambiguous by author
and title alone; even when combining author, title, subtitle (副題), orthography
(文字遣い種別), source edition (底本名), and first publication (初出), 16 works
remain indistinguishable without a specific identifier. Those counts change with
the upstream catalog. The work identifier, such as `000092_000879`, distinguishes
all of them and does not change between releases; see
[work identifiers](../soranoha/docs/work-identifiers.md).

Every work page carries both forms below, ready to copy, and the same record
as CSL-JSON and BibLaTeX. All of them are generated from the work's TEI
header, which is the bibliographic record; the catalog and every citation
format are derived from it.

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

Citations give the 12-character release prefix, which resolves at
`https://soranoha.org/releases/short/<prefix>.json` to the same manifest as
the full 64-character name, for example
`https://soranoha.org/releases/short/8b259c55e63a.json`. The machine-readable
forms carry the full 64-character hash: `version` in BibLaTeX, the `Release`
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

The identifier and the release are never optional. The other parts describe
the work; those two identify it.

### Machine-readable forms

Each work serves the same record in the formats reference managers read:

| Route | Format | For |
|---|---|---|
| `/works/<id>/citation.json` | CSL-JSON | Zotero, Pandoc, citeproc |
| `/works/<id>/citation.bib` | BibLaTeX | LaTeX written directly |

Work pages also embed [COinS](https://en.wikipedia.org/wiki/COinS), so the
Zotero browser connector saves a correctly typed record in one click with no
site-specific setup.

All three record a work as an item inside its 底本, not as a book of its own:
CSL `chapter`, BibLaTeX `@incollection`, COinS `genre=bookitem`. A
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

The identifier uses BibLaTeX's archive fields: `eprinttype` holds the
collection and `eprint` the item in it, and standard styles print both.

The release appears in two fields. `version` holds the whole head hash, which
biber and Zotero import but standard styles do not print for an
`@incollection`. `addendum` holds the 12-character prefix, which every
standard driver prints at the end of the entry. The note also repeats the
original title of a translated work, which `origtitle` imports but standard
styles do not print.

In CSL-JSON, 校訂者 is mapped to `contributor`; in BibLaTeX, it is represented as
`editora` with `editoratype = {reviser}`. The Japanese role is shown on each work
page and in its TEI header.

For working with multiple works at once, `catalog.csv` is included at the root
of each [bulk archive](start-here.md) with these fields as columns, ready for
reference management or tabular analysis.

### The edition year

`date = {1986}` is extracted, not recorded. Aozora Bunko's 底本初版発行年 is a
free-form publication history, not a year: 17747 of the 18780 recorded values
look like `1981（昭和56）年3月20日`, and of the 1033 that do not, 667 add a
printing history, as in
`1948（昭和23）年5月15日、1963（昭和38）年5月16日第20刷改版`. The first
Gregorian year is taken; where several years appear, the earliest is the first
edition's, which is what the field means. Work pages show the recorded string
in full; only the machine-readable fields carry the extracted year.

### What CSL cannot carry

CSL has no field for kana readings, and CSL and Zotero support for several
scripts is limited, so 作品名読み goes in `note`. In BibLaTeX, it is stored in
`titleaddon`. The TEI header and catalog JSON remain the record.

### Romanization

Citation formats provide titles in Japanese. Romanize them according to your
style guide (such as Hepburn or ALA-LC). Download filenames use Aozora Bunko's
curated archive stems. Orthographic classifications (`文字遣い種別`) remain in
Japanese.

## Citing an exact byte sequence

For work that must be reproducible against specific bytes rather than a
release, cite the artifact by its content address. Every published artifact
has an id of the form `snh:1:<type>:<sha256>`, and the manifest that lists it
is signed:

```
Soranoha Aozora Bunko TEI Corpus, work 000092_000879, TEI artifact
snh:1:tei:6f0b1c…, in release
<release head, 64 hex characters>.
```

This is the most precise citation available. A reader can fetch the artifact,
hash it, find that hash in the manifest, and verify the manifest's signature
against the release key in the trust anchor deposit. The procedure is in
[`docs/design/snh-protocol-v1.md`](design/snh-protocol-v1.md); the contents of
each deposit and the verification procedure confirming that a DOI points to the
release specified in its deposit are described in
[Zenodo deposits and ORCID anchoring](zenodo-deposits.md); and the path
mapping for reaching the same artifact through the archive instead of this site
is given in [archival](archival.md).

## Citing the tooling

The conversion and publication code is licensed separately from the corpus
(see [rights](rights.md#the-toolchain)). Cite the tooling when your research
depends on the conversion pipeline or parser rather than on the corpus text.
