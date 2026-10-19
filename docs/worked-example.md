# A worked example: 蜘蛛の糸

This page follows Akutagawa Ryūnosuke's 蜘蛛の糸 (1918, identifier
`000092_000879`) from its Aozora Bunko source file to the published TEI and
plaintext, and ends with Python code that reads them.

The excerpts are the pipeline's own output for the Aozora Bunko file
[`cards/000879/files/92_ruby_164.zip`](https://www.aozora.gr.jp/cards/000879/files/92_ruby_164.zip).
Running the same file through the same pipeline produces the same bytes.

## The source

Aozora Bunko distributes the text as a Shift_JIS file inside a ZIP, with its
own notation written inline. Four lines from the file illustrate the principal
features requiring conversion (the middle two are truncated here):

```
［＃８字下げ］一［＃「一」は中見出し］

　ある日の事でございます。御釈迦様《おしゃかさま》は極楽の蓮池《はすいけ》のふちを、
　するとその地獄の底に、※［＃「特のへん＋廴＋聿」、第3水準1-87-71］陀多《かんだた》と云う男が
［＃地から１字上げ］（大正七年四月十六日）
```

- `《…》` is ruby: a reading printed above or beside the preceding characters.
- `※［＃「…」、第3水準1-87-71］` is a **gaiji**, a character unrepresented in the
  file's primary character encoding. The marker provides a descriptive label along
  with its JIS X 0213 coordinates: level 3 (第3水準), plane 1, row 87, cell 71.
  Here it is 犍, the first character of the thief's name 犍陀多.
- `［＃８字下げ］…［＃「一」は中見出し］` encodes two instructions: indent eight
  characters, and mark 一 as a section heading.
- `［＃地から１字上げ］` indicates alignment to the bottom margin (in vertical
  text) or end of the line, one character from the edge.

At the end of the file, a colophon records the printed edition used for typing,
as well as the transcribers and proofreaders. In the TEI edition, this
information is preserved in back matter (`<back>`).

## The pipeline

![The pipeline](assets/pipeline.svg)

In the first three stages, the Rust component parses the Aozora Bunko
notation into an annotated syntax tree (AAT), converting it into a parser
intermediate representation (IR) in which each node includes its byte span in the
decoded source. The Clojure kernel then integrates the parser IR with catalog
metadata to produce the TEI document, from which the remaining three files are
derived.

To run it yourself, clone the
[Soranoha repository on GitHub](https://github.com/borh/soranoha) and follow
[the development guide](development.md).

## The header

The `teiHeader` for this work is 46 lines. The excerpts below are wrapped for
reading. In the published XML, elements such as `<author>`, `<taxonomy>` and
`<ruby>` are written on one line each, so no whitespace enters mixed content.

### titleStmt: work title and authorship

```xml
<titleStmt>
  <title type="main" xml:lang="ja">蜘蛛の糸</title>
  <title type="reading" xml:lang="ja-Hira">くものいと</title>
  <author>
    <persName xml:lang="ja"><surname>芥川</surname><forename>竜之介</forename><idno type="aozora-person-id">000879</idno></persName>
    <persName xml:lang="ja-Hira"><surname>あくたがわ</surname><forename>りゅうのすけ</forename></persName>
    <persName xml:lang="ja-Latn"><surname>Akutagawa</surname><forename>Ryunosuke</forename></persName>
  </author>
</titleStmt>
```

The kana reading of the title is a separate `<title type="reading">`. The
author has three `<persName>` forms, in kanji, kana and romaji, each tagged
with its script so that software can pick the one to sort or display. All
three come from Aozora Bunko's catalog with its spelling unchanged: the given
name is 竜之介 here, while the printed edition cited in the next block reads
龍之介.

The issuing authority is recorded in the `@type` attribute on `<idno>`, as in
`publicationStmt`. `aozora-person-id` is Aozora Bunko's six-digit person ID
(人物ID). An author without one receives a local identifier under
`soranoha-person-id`.

### publicationStmt: publication details and terms

```xml
<publicationStmt>
  <publisher ref="https://w3id.org/soranoha/">Soranoha</publisher>
  <idno type="soranoha-work-identifier">000092_000879</idno>
  <idno type="aozora-work-id">000092</idno>
  <idno type="aozora-card-url">https://www.aozora.gr.jp/cards/000879/card92.html</idno>
  <date when="2014-09-17">2014-09-17</date>
  <availability status="free">
    <licence target="https://creativecommons.org/publicdomain/mark/1.0/">The underlying work is in the public domain; Soranoha asserts no rights over it.</licence>
    <licence target="https://creativecommons.org/publicdomain/zero/1.0/">Soranoha's encoding of this work and the artifacts derived from it are dedicated to the public domain under CC0-1.0. Attribution is requested, not required. Full rights statement: <ptr target="https://w3id.org/soranoha/rights"/></licence>
  </availability>
</publicationStmt>
```

Three identifiers appear here, each with an issuing authority identified by its
`type`. Soranoha assigns the publication identifier (`000092_000879`), which
appears in every URL and file name for the work; Aozora Bunko assigns the work id
and card URL. Because the publication identifier remains inside the TEI file,
any saved copy can still be reliably cited.

The date is the last-modified date of Aozora Bunko's catalog entry. It is not
the date of composition or of Soranoha's processing.

Each `<licence>` covers one rights layer. The first declares the standing of the
underlying work, here out of copyright; the second dedicates Soranoha's
encoding under CC0. Both declarations are preserved in the file wherever it is
copied. See [rights](rights.md).

`<publisher>` is the publishing entity, `Soranoha`, with its persistent
identifier in `@ref`. The corpus title is in `titleStmt`.

### sourceDesc: printed edition and source byte hashes

```xml
<sourceDesc>
  <bibl><title>芥川龍之介全集2</title><publisher>ちくま文庫、筑摩書房</publisher><date>1986（昭和61）年10月28日</date><note type="input-edition">1996（平成8）年7月15日第11刷</note></bibl>
  <bibl type="first-publication">「赤い鳥」1918（大正7）年7月</bibl>
  <bibl><idno type="source-content-hash">sha256:ba7a3b91f4796022ff7b9d17712a0bdea92724e309ed2f4db9f17f151c8e0712</idno></bibl>
  <bibl><idno type="primary-text-hash">sha256:3c686946ec0a7f31f9de7a9a7231d93fe97c7323a5172c268a48f180fdb7bcac</idno></bibl>
</sourceDesc>
```

Four statements about the source, at four levels:

- the **printed edition** the volunteer typed from, and which printing;
- the **first publication**, where the text first appeared (the July 1918
  issue of 「赤い鳥」), marked `type="first-publication"` to keep it apart from
  the edition typed from;
- `source-content-hash`, the sha256 of the bundle's member listing,
  `{"construction":"abc-source-bundle-v1","members":[{"member_hash":…,"path":…}],"primary_text_member":…}`
  in RFC 8785 form, rather than of the ZIP file. Repackaging the same members
  leaves it unchanged; a reissued member changes it;
- `primary-text-hash`, the sha256 of the text member as distributed, that is,
  of the Shift_JIS bytes, not the decoded text.

The work is designated by the identifier `000092_000879`, which remains constant
across releases. The hashes verify which specific text under that identifier was
used. For exact reproducibility in scholarly citations, record both.

### encodingDesc: taxonomies and gaiji declarations

```xml
<encodingDesc>
  <styleDefDecl scheme="css"/>
  <classDecl>
    <taxonomy xml:id="ndc">
      <bibl>分類番号 as recorded by <title>青空文庫</title>, read as a class of 日本十進分類法 新訂9版. Category labels are from <title>NDC9 Linked Data</title> (日本図書館協会, CC BY); a category's IRI is <ref target="http://jla.or.jp/data/ndc9">http://jla.or.jp/data/ndc9</ref> followed by # and its n.</bibl>
      <category xml:id="ndc913" n="913">
        <catDesc xml:lang="ja">小説．物語</catDesc>
        <catDesc xml:lang="en">Fiction.Romance.Novel</catDesc>
      </category>
    </taxonomy>
    <taxonomy xml:id="aozora-shelf">
      <bibl>分類番号 as recorded by <title>青空文庫</title>: the prefix K (児童図書記号) is placed before the NDC code for children's books.</bibl>
      <category xml:id="shelf-k" n="K">
        <catDesc xml:lang="ja">児童書</catDesc>
        <catDesc xml:lang="en">Children's material</catDesc>
      </category>
    </taxonomy>
    <taxonomy xml:id="aozora-orthography"><bibl><title>青空文庫</title> 文字遣い種別: the orthographic style Aozora Bunko records for the transcription.</bibl></taxonomy>
  </classDecl>
  <refsDecl xml:id="source-spans">
    <p>A note of type source-span has an xml:id of the form source-START-END, giving the extent in UTF-8 bytes of the decoded primary text identified in sourceDesc. <ptr target="https://w3id.org/soranoha/ns/tei"/></p>
  </refsDecl>
  <charDecl>
    <char xml:id="gaiji-3-1-87-71">
      <mapping type="unicode">犍</mapping>
      <localProp name="rawMarker" value="特のへん＋廴＋聿"/>
    </char>
  </charDecl>
</encodingDesc>
```

Each gaiji in the text has one `<char>` declaration here, referred to from the
body. Both aspects of the character are recorded in the declaration:

- `<mapping type="unicode">` is the character resolved from the JIS X 0213
  code point specified in the marker.
- `rawMarker` records the source notation as written (`特のへん＋廴＋聿`, "the left
  radical of 特, plus 廴, plus 聿"). This preserves the original editorial note
  directly in the record, allowing verification of the mapping without
  re-examining the source file.

The `xml:id` is the marker's JIS X 0213 coordinates with every character
outside `[A-Za-z0-9_.-]` replaced by a hyphen, so `第3水準1-87-71` becomes
`3-1-87-71`: 3 is the level (水準), with `1-87-71` representing the plane, row,
and cell. In this way, occurrences of the same character always resolve to the
same declaration.

The declaration `<styleDefDecl scheme="css"/>` establishes that `@style`
attributes in the body contain CSS properties, recording source layout that
lacks native TEI attributes (such as character-width indents, line measures, and
text alignment).

The `@source` references on body elements are measured in the byte unit declared
in `<refsDecl>`, which also cites the vocabulary used for the remainder. The
section on source offsets below works through one.

### profileDesc: language and classification

```xml
<profileDesc>
  <langUsage><language ident="ja">日本語</language></langUsage>
  <textClass>
    <classCode scheme="#ndc">K913</classCode>
    <catRef scheme="#ndc" target="#ndc913"/>
    <catRef scheme="#aozora-shelf" target="#shelf-k"/>
    <classCode scheme="#aozora-orthography">新字新仮名</classCode>
  </textClass>
</profileDesc>
```

In the classification code `K913` recorded by Aozora Bunko, `913` is the Nippon
Decimal Classification (NDC, 9th ed.) category for Japanese fiction (小説．物語),
and `K` is the children's literature symbol (児童図書記号). Two `<catRef>`
elements connect the work to the categories declared in `<classDecl>`, recording
both its subject classification and its children's literature designation in the
TEI header. In the Soranoha browse index, the work appears under 913 with a
children's literature label (`児童書`), listed under its subject alongside adult
literature as well as in the dedicated children's literature index. Where a code
is not listed in NDC9 (such as `998`), only a `<classCode>` is recorded, without
a `<catRef>`.

The transcription's orthography is recorded as `新字新仮名` (modern kanji, modern
kana). Across the 17,655 catalog works with a text file, 10,791 are 新字新仮名,
4,569 新字旧仮名, 2,183 旧字旧仮名, 93 旧字新仮名 and 19 その他. A single title
can appear under multiple orthographies as separate works with distinct
identifiers (568 title-and-subtitle pairs, covering 1,343 works, do so in the
corpus). For example, Nomura Kodō's *Zenigata Heiji Torimonohikae 001 Kin-iro no shojo*
(銭形平次捕物控 001 金色の処女) is `054695` in 旧字旧仮名 and `056372` in 新字新仮名.

Taxonomies declared in `encodingDesc` are referenced by `scheme` attributes, with
both classifications taken from Aozora Bunko's catalog.

## The body

### Ruby

```xml
<ruby type="furigana" rend="right" source="#source-733-769">
  <rb>御釈迦様</rb><rt>おしゃかさま</rt>
</ruby>
```

Base text is placed in `<rb>` and the reading in `<rt>`, as in HTML `<ruby>`.
`@rend` indicates which side of the line the source placed the reading on. In
the plaintext projection, base text (`<rb>`) is retained while readings (`<rt>`)
are omitted.

A ruby base can contain markup of its own, as when the thief's name combines a
gaiji with ordinary characters:

```xml
<ruby type="furigana" rend="right" source="#source-1856-1942">
  <rb><g ref="#gaiji-3-1-87-71" source="#source-1856-1918">犍</g><seg source="#source-1918-1924">陀多</seg></rb>
  <rt>かんだた</rt>
</ruby>
```

To get the base text, take all text under `<rb>`, including nested `<g>` and
`<seg>`.

### Gaiji in the body

```xml
<g ref="#gaiji-3-1-87-71" source="#source-2071-2133">犍</g>
```

The element's content is the mapped character (犍), and `@ref` references the
header declaration containing the original marker.

### Headings and layout

```xml
<div type="layout" source="#source-633-694" style="padding-inline-start: 8em">
  <div>
    <head n="2" source="#source-654-690"><seg source="#source-654-657">一</seg></head>
  </div>
</div>
```

The outer `<div type="layout">` represents the source's ８字下げ instruction as
layout geometry. The inner `<head n="2">` represents the heading designated by
the source's 中見出し marker. Each derives from its own source annotation; neither
is inferred from the other.

The dateline at the end is layout only:

```xml
<div type="layout" source="#source-11439-11502" style="padding-inline-end: 1em; text-align: right">
  <p><seg source="#source-11469-11502">（大正七年四月十六日）</seg></p>
</div>
```

### Source offsets

Almost every element includes a `@source` attribute referencing a
`<note type="source-span">` in `<back>`:

```xml
<seg source="#source-694-733">ある日の事でございます。</seg>
```

```xml
<note type="source-span" xml:id="source-694-733" n="20"/>
```

The note contains no text content; the byte extent is expressed directly in its
identifier. In `source-694-733`, the start and end byte offsets in the decoded
UTF-8 source text are given as 694 and 733, with the source line number recorded
in `@n`. The source text indexed by these offsets is identified for the whole
document by `primary-text-hash` in `sourceDesc`:

```xml
<idno type="primary-text-hash">sha256:3c686946ec0a7f31f9de7a9a7231d93fe97c7323a5172c268a48f180fdb7bcac</idno>
```

The hash is of the member as distributed; the offsets index its UTF-8
decoding. To go from one to the other, decode with the encoding given in
`<note type="source-decoding">` at the end of `<back>`, re-encode as UTF-8,
and slice:

```python
import hashlib

raw = open("kumono_ito.txt", "rb").read()   # the ZIP's one member
assert hashlib.sha256(raw).hexdigest() == (
    "3c686946ec0a7f31f9de7a9a7231d93fe97c7323a5172c268a48f180fdb7bcac")
source = raw.decode("cp932")                # windows-31j, from source-decoding
assert source.encode("utf-8")[694:733].decode("utf-8") == "　ある日の事でございます。"
```

That is 39 bytes for 13 characters. The first is the ideographic space used
for indentation in the source; it is included in the span because the paragraph's
`text-indent` was derived from it. Offsets index the source, not the output.

Annotations made over the text, such as token spans, alignments or named
entities, can use these byte offsets and be checked against the source without
any Soranoha tooling.

### What is at the end

```xml
<back>
  <div type="source">
    <note type="source-attribution" source="#source-11510-11785"><seg type="source-line">底本：「芥川龍之介全集2」ちくま文庫、筑摩書房</seg><lb/>…</note>
    <note type="transcriber-note" source="#source-11785-12154"><seg type="source-line">入力：平山誠、野口英司</seg><lb/>…</note>
    <note type="source-span" xml:id="source-10080-10141" n="36"/>
    …
  </div>
  <note type="parser-completion" n="true">The n attribute is true when no parser diagnostic of error severity was raised for this source. …</note>
  <note type="source-decoding">windows-31j</note>
</back>
```

The colophon and the transcribers' note are the source's own back matter, kept
as written. The `source-span` notes form the offset table. `source-decoding`
records the character encoding used when reading the source, ensuring that byte
offsets can be reproduced accurately.

`parser-completion` is always present; its `@n` is `true` when the parser
raised no error. Where ambiguous or uninterpreted syntax was encountered during
conversion, findings are recorded in `interpretation-problem` and
`parser-diagnostic` notes.

## The plaintext

```
一
　ある日の事でございます。御釈迦様は極楽の蓮池のふちを、独りでぶらぶら御歩きになっていらっしゃいました。…
```

Reading text only: ruby readings are removed, gaiji appear as their mapped
characters, headings are on their own lines, and source indentation is
preserved. Editorial notes, apparatus and the offset table are excluded.

The plaintext is derived from the TEI under one documented policy. Where the
TEI contains alternatives, one lemma is selected from an apparatus and one
branch from an editorial choice according to that policy. The site's reading
view applies the same policy, presenting identical text.

## The other two projections

`markdown` is CommonMark with the ruby kept as inline HTML, so a reader with
no XML parser still gets the readings:

```markdown
## 一

ある日の事でございます。<ruby><rb>御釈迦様</rb><rt>おしゃかさま</rt></ruby>は極楽の<ruby><rb>蓮池</rb><rt>はすいけ</rt></ruby>のふちを、…
```

Where the source contains an emphasis mark, the Markdown projection formats it
as a `<span>` carrying the original `@rend` and matching CSS; 白ゴマ傍点, for
example, is rendered with `text-emphasis-style: open sesame`.

`tei-validation` is a JSON report detailing the hash of the profile used for
verification, the status of each validation layer (well-formedness, Relax NG, and
Schematron), and all rule findings accompanied by their identifier and severity.
A pass indicates that the XML conforms to the profile, rather than verifying the
accuracy of the underlying transcription. The scope of verification performed at
each layer is detailed in [TEI validation](../soranoha/docs/tei-validation.md).

## Loading it in Python

No dependencies; `xml.etree.ElementTree` from the standard library is enough.

```python
import xml.etree.ElementTree as ET

TEI = "{http://www.tei-c.org/ns/1.0}"
XML = "{http://www.w3.org/XML/1998/namespace}"
# The work page's TEI download is named for the author, Aozora Bunko's own stem for
# the text, and the identifier.
tree = ET.parse("Akutagawa_Ryunosuke-kumono_ito-000092_000879.xml")

def text(node):
    """All text under a node, descending into ruby bases and gaiji."""
    return "".join(node.itertext()) if node is not None else ""
```

**Bibliography, from the header:**

```python
head = tree.find(f"{TEI}teiHeader/{TEI}fileDesc")
title = head.findtext(f"{TEI}titleStmt/{TEI}title[@type='main']")
author = head.find(f"{TEI}titleStmt/{TEI}author/{TEI}persName")
edition = head.find(f"{TEI}sourceDesc/{TEI}bibl")

print(title)                                              # 蜘蛛の糸
print(author.findtext(f"{TEI}surname")
      + author.findtext(f"{TEI}forename"))                # 芥川竜之介
print(edition.findtext(f"{TEI}publisher"))                # ちくま文庫、筑摩書房
```

Pick the parts you need rather than taking all text under `<persName>`: the
Aozora Bunko person id is an `<idno>` inside it, so `text(author)` would give
`芥川竜之介000879`.

**Every ruby pair in the work:**

```python
rubies = [(text(r.find(f"{TEI}rb")), text(r.find(f"{TEI}rt")))
          for r in tree.iter(f"{TEI}ruby")]

print(len(rubies))            # 68
print(rubies[:3])             # [('御釈迦様', 'おしゃかさま'), ('蓮池', 'はすいけ'), ('蓮', 'はす')]
print(rubies[18])             # ('犍陀多', 'かんだた')  (gaiji inside the base)
```

Ruby readings and editorial markup are preserved in the TEI; the plaintext
retains base reading text only.

**Every gaiji, with the marker the source used:**

```python
chars = {c.get(f"{XML}id"): c for c in tree.iter(f"{TEI}char")}

for g in tree.iter(f"{TEI}g"):
    decl = chars[g.get("ref").lstrip("#")]
    marker = decl.find(f"{TEI}localProp[@name='rawMarker']").get("value")
    print(g.text, "from", marker)        # 犍 from 特のへん＋廴＋聿
```

**A passage and where it came from:**

```python
def extent(reference):
    _, start, end = reference.lstrip("#").split("-")
    return int(start), int(end)

# The source bytes the offsets index, opened again as in "Source offsets".
source = open("kumono_ito.txt", "rb").read().decode("cp932")
body = tree.find(f"{TEI}text/{TEI}body")
for seg in body.iter(f"{TEI}seg"):
    ref = seg.get("source")
    if ref:
        start, end = extent(ref)
        print(text(seg), source.encode("utf-8")[start:end].decode("utf-8"))
        break
```

The byte extent is given directly in the `@source` fragment. The corresponding
`source-span` notes in `<back>` record the source line number in `@n`, with
offsets preserved in their identifiers. On the body elements shown here, a
single reference appears in `@source`; however, where a span is closed by a
`<note>`, two whitespace-separated references are supplied. If widening the loop
beyond `<seg>`, split on whitespace first.

**If all you want is the text**, download the `plaintext` file instead of
deriving it. It is published beside the TEI, made under the documented reading
policy, and hashed into the same signed manifest:

```python
import urllib.request

url = "https://soranoha.org/works/000092_000879/plaintext"
body = urllib.request.urlopen(url).read().decode("utf-8")
```

## Where to go next

- [Start here](start-here.md): corpus scope and bulk downloads.
- [Glossary](user-glossary.md): admission, manifests, the release chain, and
  the difference between work, edition and document.
- [TEI extension vocabulary](../soranoha/docs/tei-vocabulary.md): the `snh:`
  attributes and the validation rule identifiers.
- [TEI validation](../soranoha/docs/tei-validation.md): the two validation
  layers and the report.
