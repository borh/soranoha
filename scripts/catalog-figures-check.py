#!/usr/bin/env python3
"""Verify consistency of quoted Aozora corpus and repository metrics.

Checks that figures quoted across documentation match their derivations from
the pinned corpus snapshot (or, under --quotes-only, that all citing files
agree with the recorded values).
"""

from __future__ import annotations

import argparse
import collections
import csv
import hashlib
import io
import os
import re
import statistics
import subprocess
import sys
import unicodedata
import zipfile
from collections.abc import Iterable, Sequence
from pathlib import Path
from typing import NamedTuple

ROOT = Path(__file__).resolve().parent.parent
CATALOG_ARCHIVE = "index_pages/list_person_all_extended_utf8.zip"
CARD_DIRECTORY = re.compile(r"/cards/(\d{6})/")
TEXT_ARCHIVE = re.compile(r"/cards/(\d{6})/files/(.+\.zip)$")
AUTHOR = "著者"

CATALOG = "catalog"
WITH_TEXT = "with-text"
ROWS = "rows"
# Commit history populations derived from git history rather than catalog CSV.
HISTORY = "first-parent-history"
SKIPPED = "skipped-commits"

WORK_ARCHIVES = "cards/*/files/*.zip"
CATALOG_DIRECTORY = "index_pages/*"

# Matches standard edition date format: Gregorian year, era year, month, and day.
PLAIN_EDITION_DATE = re.compile(r"^\d{4}（[^）]+）年\d+月\d+日")

# Transliteration mapping mirroring soranoha.za.naming/ascii-component.
TRANSLITERATIONS = {
    "ł": "l",
    "Ł": "L",
    "ø": "o",
    "Ø": "O",
    "æ": "ae",
    "Æ": "Ae",
    "œ": "oe",
    "Œ": "Oe",
    "ß": "ss",
    "đ": "d",
    "Đ": "D",
    "ð": "d",
    "Ð": "D",
    "þ": "th",
    "Þ": "Th",
    "ı": "i",
    "İ": "I",
    "’": "'",
}

CITATION = "docs/citation.md"
CITATION_SRC = "soranoha/src/soranoha/za/citation.clj"
EVIDENCE = "soranoha/docs/evidence/aozora-rights-source-contract.md"
IDENTIFIERS = "soranoha/docs/work-identifiers.md"
NAMING = "soranoha/src/soranoha/za/naming.clj"
NAMING_TEST = "soranoha/test/soranoha/za/naming_test.clj"
MAIN = "soranoha/src/soranoha/main.clj"
MANIFEST_3 = "docs/design/adr/0003-snh-manifest-3.md"
PERFORMANCE = "docs/performance.md"
START_HERE = "docs/start-here.md"
WORKED_EXAMPLE = "docs/worked-example.md"

Row = dict[str, str]
Work = list[Row]


class Digest(NamedTuple):
    """The pinned snapshot the figures below were derived from."""

    what: str
    sha256: str
    quoted_in: Sequence[str]


# Pinned catalog archive and CSV digests.
DIGESTS = [
    Digest(
        "the catalog archive",
        "5ea13273dd457f89af31de39f559ea9c6f5435d9bda1ae3d46d6a683b8bc3c92",
        [EVIDENCE],
    ),
    Digest(
        "the catalog CSV",
        "c0ace54c7ac037e5aebd045922c7879b9f7dc01f85b8f7483c8d4ecc29569ef4",
        [EVIDENCE],
    ),
]


class Figure(NamedTuple):
    """One quoted count: what it claims, over which population, and where."""

    key: str
    population: str
    what: str
    quoted: int
    quoted_in: Sequence[str]


FIGURES = [
    Figure("catalog-rows", ROWS, "rows in the catalog CSV", 19470, [EVIDENCE]),
    Figure("catalog-works", CATALOG, "works the catalog lists", 17810, [EVIDENCE, IDENTIFIERS]),
    Figure("catalog-persons", CATALOG, "distinct people in the catalog", 1334, [EVIDENCE, NAMING]),
    Figure(
        "works-with-text",
        CATALOG,
        "catalog works that have a downloadable text file",
        17655,
        [START_HERE, WORKED_EXAMPLE],
    ),
    Figure(
        "contributors",
        WITH_TEXT,
        "people credited on a work that has a text file",
        1167,
        [START_HERE],
    ),
    Figure(
        "author-title-works",
        CATALOG,
        "works sharing an author-and-title pair with another work",
        2357,
        [
            CITATION,
            START_HERE,
            IDENTIFIERS,
            "soranoha/src/soranoha/ori/tei_header.clj",
            CITATION_SRC,
            NAMING,
            NAMING_TEST,
            "soranoha/test/soranoha/tei_fidelity_test.clj",
        ],
    ),
    Figure(
        "author-title-pairs",
        CATALOG,
        "distinct author-and-title pairs shared by more than one work",
        470,
        [IDENTIFIERS, NAMING, NAMING_TEST],
    ),
    Figure(
        "author-title-subtitle-works",
        CATALOG,
        "works still ambiguous once the subtitle is added",
        882,
        ["soranoha/src/soranoha/ori/tei_header.clj"],
    ),
    Figure(
        "six-field-works",
        CATALOG,
        "works still ambiguous under author, title, 副題, 文字遣い種別, 底本名 and 初出",
        16,
        [CITATION, CITATION_SRC, "soranoha/test/soranoha/tei_fidelity_test.clj"],
    ),
    Figure("multi-author-works", CATALOG, "works recording more than one 著者", 422, [NAMING]),
    Figure(
        "card-not-first-author",
        CATALOG,
        "works filed under a card belonging to someone other than the first 著者",
        146,
        [IDENTIFIERS],
    ),
    Figure(
        "persons-without-given-romaji",
        CATALOG,
        "people whose given name carries no romaji",
        73,
        [NAMING],
    ),
    Figure(
        "longest-author-component",
        CATALOG,
        "characters in the longest rendered author filename component",
        38,
        [NAMING],
    ),
    Figure("orthography-shinji-shinkana", WITH_TEXT, "新字新仮名 works", 10791, [WORKED_EXAMPLE]),
    Figure("orthography-shinji-kyukana", WITH_TEXT, "新字旧仮名 works", 4569, [WORKED_EXAMPLE]),
    Figure("orthography-kyuji-kyukana", WITH_TEXT, "旧字旧仮名 works", 2183, [WORKED_EXAMPLE]),
    Figure("orthography-kyuji-shinkana", WITH_TEXT, "旧字新仮名 works", 93, [WORKED_EXAMPLE]),
    Figure("orthography-other", WITH_TEXT, "その他 works", 19, [WORKED_EXAMPLE]),
    Figure(
        "orthography-split-titles",
        WITH_TEXT,
        "title-and-subtitle pairs recorded under more than one orthography",
        568,
        [WORKED_EXAMPLE],
    ),
    Figure(
        "orthography-split-works",
        WITH_TEXT,
        "works sharing a title and subtitle with a differently-spelled sibling",
        1343,
        [WORKED_EXAMPLE],
    ),
    Figure(
        "edition-year-values",
        ROWS,
        "rows recording a 底本初版発行年 for the first edition slot",
        18780,
        [CITATION, CITATION_SRC],
    ),
    Figure(
        "edition-year-values-both-slots",
        ROWS,
        "recorded 底本初版発行年 values across both edition slots",
        18886,
        [CITATION_SRC],
    ),
    Figure(
        "edition-year-plain",
        ROWS,
        "first-slot values that are a date and nothing else",
        17747,
        [CITATION, CITATION_SRC],
    ),
    Figure(
        "edition-year-irregular",
        ROWS,
        "first-slot values that are not a date and nothing else",
        1033,
        [CITATION, CITATION_SRC],
    ),
    Figure(
        "edition-year-appended",
        ROWS,
        "irregular values that are a date followed by a printing history",
        667,
        [CITATION, CITATION_SRC],
    ),
    # Derived only; citation.clj writes the figure as prose rather than a numeral.
    Figure(
        "edition-year-not-leading",
        ROWS,
        "values whose four-digit year does not lead the string",
        1,
        [],
    ),
]

STEM_FIGURES = [
    Figure(
        "apostrophe-stems",
        CATALOG,
        "archive stems using Hepburn's apostrophe",
        204,
        [NAMING],
    ),
    Figure(
        "shared-stems", CATALOG, "archive stems claimed by more than one work", 802, [NAMING_TEST]
    ),
    Figure(
        "works-sharing-a-stem",
        CATALOG,
        "works whose archive stem is not theirs alone",
        1908,
        [NAMING_TEST],
    ),
    Figure(
        "median-stem-run",
        CATALOG,
        "median longest unbroken run of letters in an archive stem",
        8,
        [NAMING],
    ),
]


# Figures derived from commit history rather than catalog metadata.
HISTORY_FIGURES = [
    Figure(
        "first-parent-commits",
        HISTORY,
        "commits on aozorabunko's first-parent history",
        5476,
        [MAIN, MANIFEST_3, PERFORMANCE],
    ),
    Figure(
        "work-archive-commits",
        HISTORY,
        "commits touching a work archive",
        4673,
        [MAIN],
    ),
    Figure(
        "catalog-or-archive-commits",
        HISTORY,
        "commits touching a work archive or the catalog directory",
        5469,
        [MAIN],
    ),
    Figure(
        "catalog-only-commits",
        HISTORY,
        "commits touching the catalog directory but no work archive",
        796,
        [MAIN],
    ),
    Figure(
        "longest-skipped-run",
        SKIPPED,
        "longest unbroken run of commits changing no work archive",
        14,
        [MAIN],
    ),
    Figure(
        "longest-skip-wait-days",
        SKIPPED,
        "days a skipped commit waits for its covering release, at most",
        14,
        [MAIN],
    ),
    # Derived only; cited in prose as a word/bare digit.
    Figure(
        "median-skip-wait-days",
        SKIPPED,
        "days a skipped commit waits for its covering release, at the median",
        2,
        [],
    ),
]


def git(aozora_root: Path, *arguments: str) -> list[str]:
    result = subprocess.run(
        ["git", "-C", str(aozora_root), *arguments],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise SystemExit(
            f"catalog-figures: git {' '.join(arguments)} failed in {aozora_root}:"
            f" {result.stderr.strip()}"
        )
    return result.stdout.splitlines()


def measure_history(aozora_root: Path) -> dict[tuple[str, str], int]:
    """Derive commit-history and skipped-commit metrics from first-parent git log."""
    commits = [
        line.split()
        for line in git(
            aozora_root,
            "rev-list",
            "--first-parent",
            "--format=%H %ct",
            "--no-commit-header",
            "HEAD",
        )
    ]
    commits.reverse()
    moved = set(git(aozora_root, "rev-list", "--first-parent", "HEAD", "--", WORK_ARCHIVES))
    either = git(
        aozora_root, "rev-list", "--first-parent", "HEAD", "--", WORK_ARCHIVES, CATALOG_DIRECTORY
    )

    longest = current = 0
    waits: list[float] = []
    pending: list[int] = []
    for commit, timestamp in commits:
        seconds = int(timestamp)
        if commit in moved:
            waits.extend((seconds - held) / 86400 for held in pending)
            pending = []
            current = 0
        else:
            current += 1
            longest = max(longest, current)
            pending.append(seconds)

    return {
        ("first-parent-commits", HISTORY): len(commits),
        ("work-archive-commits", HISTORY): len(moved),
        ("catalog-or-archive-commits", HISTORY): len(either),
        ("catalog-only-commits", HISTORY): len(either) - len(moved),
        ("longest-skipped-run", SKIPPED): longest,
        ("longest-skip-wait-days", SKIPPED): round(max(waits)),
        ("median-skip-wait-days", SKIPPED): round(statistics.median(waits)),
    }


def check_digests(aozora_root: Path) -> list[str]:
    """Verify that the checkout matches pinned catalog digests."""
    archive_path = aozora_root / CATALOG_ARCHIVE
    archive = zipfile.ZipFile(archive_path)
    member = next(name for name in archive.namelist() if name.endswith(".csv"))
    found = [
        hashlib.sha256(archive_path.read_bytes()).hexdigest(),
        hashlib.sha256(archive.read(member)).hexdigest(),
    ]
    problems = []
    for digest, actual in zip(DIGESTS, found, strict=True):
        if digest.sha256 != actual:
            problems.append(f"{digest.what}: pinned {digest.sha256}, checkout has {actual}")
    return problems or digest_quotes()


def digest_quotes() -> list[str]:
    """Check that each file citing a pinned digest still states it."""
    return [
        f"{digest.what}: {name} no longer states {digest.sha256}"
        for digest in DIGESTS
        for name in digest.quoted_in
        if digest.sha256 not in (ROOT / name).read_text(encoding="utf-8")
    ]


def read_catalog(aozora_root: Path) -> dict[str, Work]:
    """Catalog rows grouped by 作品ID, in the order the CSV lists them."""
    archive = zipfile.ZipFile(aozora_root / CATALOG_ARCHIVE)
    member = next(name for name in archive.namelist() if name.endswith(".csv"))
    works: dict[str, Work] = collections.OrderedDict()
    for row in csv.DictReader(io.StringIO(archive.read(member).decode("utf-8-sig"))):
        works.setdefault(row["作品ID"], []).append(row)
    return works


def authors(work: Work) -> Work:
    return [row for row in work if row["役割フラグ"].strip() == AUTHOR]


def author_key(work: Work) -> tuple[str, ...]:
    """Return sorted author person IDs for a work."""
    return tuple(sorted(row["人物ID"] for row in authors(work)))


def ambiguous(works: Iterable[Work], fields: Sequence[str]) -> tuple[int, int]:
    """Works sharing a description with another, and how many descriptions."""
    groups: collections.Counter[tuple[object, ...]] = collections.Counter()
    for work in works:
        groups[(author_key(work), *(work[0][field] for field in fields))] += 1
    shared = [count for count in groups.values() if count > 1]
    return sum(shared), len(shared)


def ascii_component(value: str) -> str:
    if not value.strip():
        return ""
    folded = unicodedata.normalize("NFKD", value)
    folded = re.sub(r"[̀-ͯ]+", "", folded)
    folded = "".join(TRANSLITERATIONS.get(character, character) for character in folded)
    folded = re.sub(r"[^A-Za-z0-9']+", "_", folded)
    folded = re.sub(r"^_|_$", "", folded)[:64]
    return re.sub(r"[_']+$", "", folded)


def author_component(work: Work) -> str:
    person = authors(work)[0] if authors(work) else work[0]
    parts = [
        part
        for part in (ascii_component(person["姓ローマ字"]), ascii_component(person["名ローマ字"]))
        if part
    ]
    return "_".join(parts)


def archive_stems(aozora_root: Path, works: Iterable[Work]) -> list[str]:
    """Extract text member filename stems from work ZIP archives."""
    stems = []
    for work in works:
        match = TEXT_ARCHIVE.search(work[0]["テキストファイルURL"].strip())
        if not match:
            continue
        path = aozora_root / "cards" / match.group(1) / "files" / match.group(2)
        if not path.exists():
            continue
        try:
            members = [
                name for name in zipfile.ZipFile(path).namelist() if name.lower().endswith(".txt")
            ]
        except (zipfile.BadZipFile, OSError):
            continue
        if len(members) == 1:
            stems.append(os.path.splitext(os.path.basename(members[0]))[0])
    return stems


def longest_run(value: str) -> int:
    return max((len(part) for part in re.split(r"[^A-Za-z0-9]+", value) if part), default=0)


def measure(works: dict[str, Work]) -> dict[tuple[str, str], int]:
    every = list(works.values())
    with_text = [work for work in every if work[0]["テキストファイルURL"].strip()]
    author_title, author_title_pairs = ambiguous(every, ["作品名"])
    filed_elsewhere = 0
    for work in every:
        match = CARD_DIRECTORY.search(work[0]["テキストファイルURL"])
        written = authors(work)
        if match and written and written[0]["人物ID"].zfill(6) != match.group(1):
            filed_elsewhere += 1
    orthography = collections.Counter(work[0]["文字遣い種別"].strip() for work in with_text)
    # Group works by title and subtitle to find works split across orthographies.
    by_title: collections.defaultdict[tuple[str, str], set[str]] = collections.defaultdict(set)
    for work in with_text:
        style = work[0]["文字遣い種別"].strip()
        if style:
            by_title[(work[0]["作品名"], work[0]["副題"])].add(style)
    split_titles = {title for title, styles in by_title.items() if len(styles) > 1}
    split_works = sum(
        1 for work in with_text if (work[0]["作品名"], work[0]["副題"]) in split_titles
    )
    rows = [row for work in every for row in work]
    first_slot = [row["底本初版発行年1"].strip() for row in rows if row["底本初版発行年1"].strip()]
    both_slots = first_slot + [
        row["底本初版発行年2"].strip() for row in rows if row["底本初版発行年2"].strip()
    ]
    irregular = [value for value in first_slot if not PLAIN_EDITION_DATE.fullmatch(value)]
    return {
        ("catalog-rows", ROWS): len(rows),
        ("catalog-works", CATALOG): len(every),
        ("catalog-persons", CATALOG): len({row["人物ID"] for work in every for row in work}),
        ("works-with-text", CATALOG): len(with_text),
        ("contributors", WITH_TEXT): len({row["人物ID"] for work in with_text for row in work}),
        ("author-title-works", CATALOG): author_title,
        ("author-title-pairs", CATALOG): author_title_pairs,
        ("author-title-subtitle-works", CATALOG): ambiguous(every, ["作品名", "副題"])[0],
        ("six-field-works", CATALOG): ambiguous(
            every, ["作品名", "副題", "文字遣い種別", "底本名1", "初出"]
        )[0],
        ("multi-author-works", CATALOG): sum(1 for work in every if len(authors(work)) > 1),
        ("card-not-first-author", CATALOG): filed_elsewhere,
        ("persons-without-given-romaji", CATALOG): len(
            {row["人物ID"] for work in every for row in work if not row["名ローマ字"].strip()}
        ),
        ("longest-author-component", CATALOG): max(len(author_component(work)) for work in every),
        ("orthography-shinji-shinkana", WITH_TEXT): orthography["新字新仮名"],
        ("orthography-shinji-kyukana", WITH_TEXT): orthography["新字旧仮名"],
        ("orthography-kyuji-kyukana", WITH_TEXT): orthography["旧字旧仮名"],
        ("orthography-kyuji-shinkana", WITH_TEXT): orthography["旧字新仮名"],
        ("orthography-other", WITH_TEXT): orthography["その他"],
        ("orthography-split-titles", WITH_TEXT): len(split_titles),
        ("orthography-split-works", WITH_TEXT): split_works,
        ("edition-year-values", ROWS): len(first_slot),
        ("edition-year-values-both-slots", ROWS): len(both_slots),
        ("edition-year-plain", ROWS): len(first_slot) - len(irregular),
        ("edition-year-irregular", ROWS): len(irregular),
        ("edition-year-appended", ROWS): sum(
            1 for value in irregular if PLAIN_EDITION_DATE.match(value)
        ),
        ("edition-year-not-leading", ROWS): sum(
            1 for value in both_slots if not re.match(r"^\d{4}", value)
        ),
    }


def measure_stems(aozora_root: Path, works: dict[str, Work]) -> dict[tuple[str, str], int]:
    stems = archive_stems(aozora_root, works.values())
    claimed: collections.Counter[str] = collections.Counter(stems)
    shared = [count for count in claimed.values() if count > 1]
    return {
        ("apostrophe-stems", CATALOG): sum(1 for stem in stems if "'" in stem),
        ("shared-stems", CATALOG): len(shared),
        ("works-sharing-a-stem", CATALOG): sum(shared),
        ("median-stem-run", CATALOG): int(statistics.median(longest_run(stem) for stem in stems)),
    }


def quotes(path: Path, value: int) -> bool:
    """Whether a file states this number, grouped with commas or not."""
    grouped = f"{value:,}"
    text = path.read_text(encoding="utf-8")
    return any(
        re.search(rf"(?<![0-9,]){re.escape(written)}(?![0-9,])", text)
        for written in {str(value), grouped}
    )


def figure_quotes(figures: Sequence[Figure]) -> list[str]:
    """Check that each file citing a figure quotes the recorded value."""
    return [
        f"{figure.key}: {name} no longer states {figure.quoted}"
        for figure in figures
        for name in figure.quoted_in
        if not quotes(ROOT / name, figure.quoted)
    ]


def check(figures: Sequence[Figure], measured: dict[tuple[str, str], int]) -> list[str]:
    problems = []
    still_derived = []
    for figure in figures:
        derived = measured[figure.key, figure.population]
        if derived != figure.quoted:
            problems.append(
                f"{figure.key}: prose says {figure.quoted}, the catalog gives {derived}"
                f" ({figure.what}, over {figure.population})"
            )
            continue
        still_derived.append(figure)
    return problems + figure_quotes(still_derived)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--aozora-root", default=os.environ.get("CORPUS_CHECKOUT", ""))
    parser.add_argument(
        "--stems",
        action="store_true",
        help="also check the figures that need a pass over every work archive",
    )
    parser.add_argument(
        "--quotes-only",
        action="store_true",
        help="check only that the prose still states each recorded figure, without a corpus",
    )
    arguments = parser.parse_args()
    if arguments.quotes_only:
        figures = list(FIGURES) + list(STEM_FIGURES) + list(HISTORY_FIGURES)
        problems = digest_quotes() + figure_quotes(figures)
        for problem in problems:
            print(f"catalog-figures: {problem}", file=sys.stderr)
        if problems:
            return 1
        cited = sum(len(figure.quoted_in) for figure in figures)
        cited += sum(len(digest.quoted_in) for digest in DIGESTS)
        print(f"checked {cited} citations of {len(figures)} figures; nothing re-derived")
        return 0
    if not arguments.aozora_root:
        print("catalog-figures: --aozora-root or CORPUS_CHECKOUT is required", file=sys.stderr)
        return 2
    aozora_root = Path(arguments.aozora_root)
    if not (aozora_root / CATALOG_ARCHIVE).exists():
        print(f"catalog-figures: no catalog at {aozora_root / CATALOG_ARCHIVE}", file=sys.stderr)
        return 2

    problems = check_digests(aozora_root)
    if problems:
        for problem in problems:
            print(f"catalog-figures: {problem}", file=sys.stderr)
        return 1

    works = read_catalog(aozora_root)
    figures = list(FIGURES) + list(HISTORY_FIGURES)
    measured = measure(works) | measure_history(aozora_root)
    if arguments.stems:
        figures += STEM_FIGURES
        measured |= measure_stems(aozora_root, works)

    problems = check(figures, measured)
    for problem in problems:
        print(f"catalog-figures: {problem}", file=sys.stderr)
    if problems:
        return 1
    skipped = "" if arguments.stems else "; archive stems not checked, pass --stems"
    print(f"checked {len(DIGESTS)} catalog digests and {len(figures)} corpus figures{skipped}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
