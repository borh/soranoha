(ns soranoha.za.citation
  "Citations for individual works in formats suitable for scholarship:
  a rendered line, CSL-JSON for Zotero and Pandoc, BibLaTeX for TeX,
  and COinS for one-click capture via browser extensions.

  All four formats are projections. The TEI header serves as the authoritative
  bibliographic record; the same facts are provided in the signed catalog so that
  the serving layer and bulk consumers can construct bibliographies without
  parsing every TEI document. Citations are unversioned projections rather than
  primary metadata records in the chain.

  Each citation includes two essential elements: the work identifier and the
  release. The identifier is required because descriptions alone do not uniquely
  distinguish works: in the upstream catalog this release builds from, 2,357
  works remain ambiguous by author and title alone, and 16 remain
  indistinguishable even when combining author, title, subtitle (副題),
  orthography (文字遣い種別), source edition (底本名), and first publication (初出).
  The release identifier anchors the citation to the exact corpus state read.

  Romanized titles are not generated. Because kana readings lack explicit word
  boundaries, mechanical transliteration cannot ensure standard Hepburn orthography;
  the original Japanese title and reading are provided, leaving romanization style
  to citing venues.

  Deterministic: identical catalog entry, release head, and DOI yield identical
  citation text."
  (:require [clojure.string :as string]
            [soranoha.za.naming :as naming])
  (:import (java.net URLEncoder)
           (java.nio.charset StandardCharsets)))

(def corpus-name "Soranoha Aozora Bunko TEI Corpus")

(def ^:private site-origin
  "Origin URL for work landing pages in citations.

  The persistent identifier `w3id.org/soranoha/works/<id>` resolves by redirecting
  to this origin. Because citations already include both the permanent work
  identifier and the release DOI, landing page links point directly to the
  serving site without intermediate redirects."
  "https://soranoha.org")

(defn- blank->nil [s] (when-not (string/blank? s) s))

(defn- statements
  "Formats multiline bibliographic statements as a single line of running text.
  In fields such as 初出 (first publication), where collections record the
  original venue of each constituent piece, lines are joined with semicolons."
  [value]
  (string/join "; " (string/split-lines value)))

(defn- short-release
  "The release's short name: the first twelve characters of its head hash,
  which `/releases/short/<prefix>.json` resolves. The forms a reader sees
  carry this; the machine-readable fields carry all sixty-four characters."
  [head-hex]
  (subs head-hex 0 12))

(defn- contributors-by-role
  "Aozora Bunko's role strings grouped as the citation formats need them. A person
  can hold more than one role on a work, and each role is cited separately."
  [work]
  (group-by #(get % "relation_to_work") (get work "contributors")))

(defn- primary-edition
  "The first recorded 底本. A work can list two; the first is the one the
  transcription was made from, and a citation cites one book."
  [work]
  (first (get work "source_editions")))

(defn- edition-year
  "The Gregorian year of the 底本's first edition, as a number.

  Aozora Bunko's 底本初版発行年 is not a year. It is a free-form publication
  history: 17747 of the 18780 recorded values read `1981（昭和56）年3月20日`,
  and of the 1033 that do not, 667 append a printing history, as in
  `1948（昭和23）年5月15日、1963（昭和38）年5月16日第20刷改版`. Every one of
  the 18886 values across both edition slots contains a four-digit Gregorian
  year, and in all but one it leads the string.

  Taking the first year is the semantically correct reading: the field is
  初版発行年, so where a printing history lists several years the earliest is
  the first edition's, which is the year a bibliography cites. Month and day
  are dropped: a book is cited by year,
  and the 1033 irregular values have no dependable month to take.

  The full string is not discarded; it appears on the work page and in the TEI
  header. The extracted integer is used only in fields requiring a numeric date."
  [work]
  (when-let [recorded (blank->nil (get (primary-edition work) "first_edition_year"))]
    (when-let [year (some-> (re-find #"\d{4}" recorded) parse-long)]
      ;; Recorded years run 1820-2025. A number outside that is not a year
      ;; this field can plausibly hold, so nothing is claimed about it.
      (when (<= 1600 year 2100) year))))

(defn- work-url [work]
  (str site-origin "/works/" (get work "slug") "/"))

(defn- full-title
  "Title and 副題 as one string, joined the way Japanese typography joins
  them. CSL and COinS have no subtitle field; BibLaTeX does and uses it."
  [work]
  (let [title (get work "title")
        subtitle (blank->nil (get work "subtitle"))]
    (if subtitle (str title "──" subtitle) title)))

(defn rendered
  "The citation to copy, in the decided order: author, title with 副題,
  orthography, 底本 with publisher and year, corpus, identifier, release,
  DOI. Japanese punctuation throughout, because the bibliographic content is
  Japanese and a reader pastes this into a Japanese bibliography."
  [{:keys [head-hex doi]} work]
  (let [{:strs [slug orthographic_style]} work
        edition (primary-edition work)
        edition-part (when edition
                       (str "底本『" (get edition "title") "』"
                            (blank->nil (get edition "publisher"))
                            (when-let [year (edition-year work)] (str "、" year "年"))))]
    (str (naming/byline work)
         "「" (full-title work) "」"
         (when-not (string/blank? orthographic_style)
           (str "（" orthographic_style "）"))
         (when edition-part (str "、" edition-part))
         "。" corpus-name ", " slug ", release " (short-release head-hex)
         (when doi (str ". https://doi.org/" doi)))))

(defn rendered-en
  "The same citation for a bibliography written in English: the author in
  Latin script from the romanized name parts the catalog publishes, Western
  ordering and punctuation, and every title left in Japanese.

  The titles remain Japanese because a romanization would be invented. The
  kana reading carries no word boundaries, so a mechanical transliteration is
  not correct Hepburn, and the 文字遣い種別 values are Aozora Bunko's own
  classification rather than terms with settled English equivalents. A writer
  who needs romanized titles has the reading and their journal's style; a
  writer who does not gets an accurate form.

  When a contributor has no romanized name at all (one person in the whole
  Aozora Bunko catalog), this uses the Japanese name rather than dropping
  the author."
  [{:keys [head-hex doi]} work]
  (let [{:strs [slug orthographic_style]} work
        edition (primary-edition work)
        authors (or (seq (naming/authors-of work)) (get work "contributors"))
        by (->> authors
                (map #(or (naming/person-name-romaji %) (naming/person-name-ja %)))
                (remove string/blank?)
                (string/join ", "))]
    (str (when-not (string/blank? by) (str by ". "))
         "“" (full-title work) "”"
         (when-not (string/blank? orthographic_style)
           (str " (" orthographic_style ")"))
         (when edition
           (str ". In " (get edition "title")
                (when-let [publisher (blank->nil (get edition "publisher"))]
                  (str ". " publisher))
                (when-let [year (edition-year work)] (str ", " year))))
         ". " corpus-name ", " slug ", release " (short-release head-hex)
         (when doi (str ". https://doi.org/" doi)))))

(def ^:private csl-roles
  "Aozora Bunko's roles as CSL contributor variables. 校訂者 has no CSL counterpart,
  since CSL models editors and translators but not collators, so it is
  recorded as a plain contributor rather than promoted to editor, which would
  state something the source does not."
  {"著者" "author" "翻訳者" "translator" "編者" "editor" "校訂者" "contributor"})

(defn- csl-name [person]
  (let [family (blank->nil (get person "family_name"))
        given (blank->nil (get person "given_name"))]
    (cond-> {}
      family (assoc "family" family)
      given (assoc "given" given)
      (and (nil? family) (nil? given))
      (assoc "literal" (naming/person-label person)))))

(defn csl-json-value
  "One CSL-JSON record, as data. `chapter` because these are works inside a
  transcribed 底本, which is also why BibLaTeX gets @incollection.

  The identifier goes in `archive` and `archive_location` rather than into a
  note: those are the CSL variables for `which collection, and where in it`,
  and Zotero displays them as fields a reader can use."
  [{:keys [head-hex doi]} work]
  (let [{:strs [slug title_reading original_title first_published
                orthographic_style ndc]} work
        edition (primary-edition work)
        year (edition-year work)
        by-role (contributors-by-role work)]
    (cond-> {"id" (str "soranoha-" slug)
             "type" "chapter"
             "title" (full-title work)
             "language" "ja"
             "archive" corpus-name
             "archive_location" slug
             "URL" (work-url work)
             "note" (string/join
                     " "
                     (remove nil?
                             [(when-let [reading (blank->nil title_reading)]
                                (str "作品名読み: " reading "."))
                              (when-let [style (blank->nil orthographic_style)]
                                (str "文字遣い種別: " style "."))
                              (when-let [first-pub (blank->nil first_published)]
                                (str "初出: " (statements first-pub) "."))
                              (str "Release " head-hex ".")]))}
      edition (assoc "container-title" (get edition "title"))
      (blank->nil (get edition "publisher"))
      (assoc "publisher" (get edition "publisher"))
      ;; CSL date-parts holds numbers; a string here makes Zotero and Pandoc
      ;; treat the year as an uninterpretable literal.
      year (assoc "issued" {"date-parts" [[year]]})
      (blank->nil original_title) (assoc "original-title" original_title)
      (blank->nil ndc) (assoc "call-number" ndc)
      doi (assoc "DOI" doi)
      true (merge (into {}
                        (keep (fn [[role people]]
                                (when-let [variable (get csl-roles role)]
                                  [variable (mapv csl-name people)])))
                        by-role)))))

(def ^:private latex-escapes
  {\\ "\\textbackslash{}" \{ "\\{" \} "\\}" \& "\\&" \% "\\%"
   \$ "\\$" \# "\\#" \_ "\\_" \~ "\\textasciitilde{}"
   \^ "\\textasciicircum{}"})

(defn- tex [value]
  (string/escape (str value) latex-escapes))

(defn- bib-name [person]
  (let [family (blank->nil (get person "family_name"))
        given (blank->nil (get person "given_name"))]
    (if (and family given)
      (str (tex family) ", " (tex given))
      (tex (naming/person-label person)))))

(defn- bib-names [people]
  (string/join " and " (map bib-name people)))

(defn biblatex
  "One @incollection entry. Works are items inside a 底本, which is what
  @incollection means; @book would state the transcription is the whole volume.

  The identifier uses BibLaTeX's own archive idiom rather than a note:
  `eprinttype` identifies the collection and `eprint` the identifier inside it,
  which the standard drivers print as `Soranoha Aozora Bunko TEI Corpus:
  000092_000879`.

  The release and the original title are each written twice, because the
  entry has two consumers that read different fields. `version` and
  `origtitle` are what biber and Zotero import, and no standard style prints
  either of them for an @incollection. `addendum` and `note` are printed by
  every standard driver, so the release the transcription came from and the
  title it was translated from reach the citation a reader sees, so that
  citation identifies the bytes that were read."
  ^String [{:keys [head-hex doi]} work]
  (let [{:strs [slug title subtitle title_reading original_title first_published
                orthographic_style]} work
        edition (primary-edition work)
        by-role (contributors-by-role work)
        field (fn [k v] (when-not (string/blank? (str v)) (str "  " k " = {" v "},\n")))
        names (fn [k role] (when-let [people (seq (get by-role role))]
                             (field k (bib-names people))))]
    (str "@incollection{soranoha-" slug ",\n"
         (names "author" "著者")
         (names "translator" "翻訳者")
         (names "editor" "編者")
         (when-let [collators (seq (get by-role "校訂者"))]
           ;; BibLaTeX's editorial roles are a closed set with no collator in
           ;; it, and a value outside the set prints as itself: `collator
           ;; Suzuki Gyozo` where a value inside it prints `Rev. by Suzuki
           ;; Gyozo`. 校訂 is revision of a text against its sources, so
           ;; `reviser` is the closest accurate entry in that set. The
           ;; Japanese role itself is on the work page and in the TEI
           ;; header.
           (str (field "editora" (bib-names collators))
                (field "editoratype" "reviser")))
         (field "title" (tex title))
         (field "subtitle" (tex subtitle))
         (field "titleaddon" (tex title_reading))
         (field "origtitle" (tex original_title))
         (field "booktitle" (tex (get edition "title")))
         (field "publisher" (tex (get edition "publisher")))
         (field "date" (tex (edition-year work)))
         (field "language" "japanese")
         (field "langid" "japanese")
         (field "eprinttype" (tex corpus-name))
         (field "eprint" slug)
         (field "version" head-hex)
         (field "addendum" (str "Release " (short-release head-hex)))
         (field "url" (work-url work))
         (when doi (field "doi" doi))
         (field "note" (tex (string/join "; " (remove string/blank?
                                                      [orthographic_style
                                                       (when original_title
                                                         (str "原題: " original_title))
                                                       (when first_published
                                                         (str "初出: "
                                                              (statements first_published)))]))))
         "}\n")))

(defn- urlencode [s]
  (-> (URLEncoder/encode (str s) StandardCharsets/UTF_8)
      ;; OpenURL readers expect percent-encoding, not the form-encoding plus
      (string/replace "+" "%20")))

(defn coins
  "The OpenURL context object a Zotero-style connector reads out of the page.

  `genre=bookitem` against the book metadata format, so a work is saved as a
  section of the 底本 rather than as a standalone book. That subtype is the
  reason this is COinS and not Highwire `citation_*` meta tags: Highwire
  cannot express it, so a connector reading those would record every work in
  the corpus as its own book and lose the source edition."
  ^String [{:keys [head-hex doi]} work]
  (let [edition (primary-edition work)
        authors (get (contributors-by-role work) "著者")
        pair (fn [k v] (when-not (string/blank? (str v))
                         (str "&" k "=" (urlencode v))))]
    (str "ctx_ver=Z39.88-2004"
         "&rft_val_fmt=" (urlencode "info:ofi/fmt:kev:mtx:book")
         "&rft.genre=bookitem"
         (pair "rft.atitle" (full-title work))
         (pair "rft.btitle" (get edition "title"))
         (pair "rft.pub" (get edition "publisher"))
         (pair "rft.date" (edition-year work))
         (apply str (map #(pair "rft.au" (naming/person-label %)) authors))
         (pair "rft.language" "jpn")
         (pair "rft_id" (work-url work))
         (when doi (pair "rft_id" (str "info:doi/" doi)))
         (pair "rft.description"
               (str corpus-name ", " (get work "slug") ", release " head-hex)))))

(def csv-columns
  "The bulk archives' `catalog.csv` header. Enough to build a bibliography
  for a whole selection in a spreadsheet without opening a single TEI file,
  and enough to get back to the record: `identifier` and `release` together
  identify the bytes, and `url` resolves to the page that serves them."
  ["identifier" "title" "subtitle" "title_reading" "author"
   "orthographic_style" "ndc" "first_published" "body_character_count"
   "source_edition_title" "source_edition_publisher" "source_edition_year"
   "filename" "source_content_hash" "url" "release" "doi" "first_published_year"])

(defn csv-values
  "Returns values for one work matching `csv-columns` in order. `filename`
  specifies the archive member name for the work."
  [{:keys [head-hex doi]} work filename]
  (let [{:strs [slug title subtitle title_reading orthographic_style ndc
                first_published body_character_count source_content_hash]} work
        edition (primary-edition work)]
    [slug title subtitle title_reading (naming/byline work)
     ;; A spreadsheet cell holding 414 lines is a row that spans 414 of
     ;; them, so the statements are run together here.
     orthographic_style ndc (some-> first_published statements) body_character_count
     (get edition "title") (get edition "publisher") (edition-year work)
     filename (str "sha256:" source_content_hash) (work-url work)
     head-hex doi
     (some->> first_published (re-find #"(?<![0-9])([12][0-9]{3})(?![0-9])") second parse-long)]))
