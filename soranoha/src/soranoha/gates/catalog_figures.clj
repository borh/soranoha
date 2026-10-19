(ns soranoha.gates.catalog-figures
  "Verifies that corpus figures cited in documentation remain consistent with
  their derivations from the pinned Aozora Bunko catalog, and that citations
  in referenced documents continue to match recorded values.

  The registry below records the definition and derivation basis for each
  cited figure, along with its citing documents. Because re-derivation requires
  a local checkout of the pinned corpus snapshot, default gates verify
  citation consistency; full re-derivation runs upon snapshot updates."
  (:require [babashka.fs :as fs]
            [babashka.process :as process]
            [charred.api :as charred]
            [clojure.string :as string]
            [soranoha.core.hash :as hash]
            [soranoha.yomi.catalog :as catalog]
            [soranoha.za.naming :as naming])
  (:import [java.math RoundingMode]
           [java.text NumberFormat]
           [java.util Locale]
           [java.util.zip ZipFile]))

(def ^:private catalog-archive "index_pages/list_person_all_extended_utf8.zip")
(def ^:private card-directory #"/cards/(\d{6})/")
(def ^:private text-archive #"/cards/(\d{6})/files/(.+\.zip)$")
(def ^:private author "著者")

;; Populations a figure is counted over.
(def ^:private catalog-pop "catalog")
(def ^:private with-text "with-text")
(def ^:private rows-pop "rows")
(def ^:private history "first-parent-history")
(def ^:private skipped "skipped-commits")

(def ^:private work-archives "cards/*/files/*.zip")
(def ^:private catalog-directory "index_pages/*")

;; A standard edition date: Gregorian year, era year, month and day.
(def ^:private plain-edition-date #"(?U)^\d{4}（[^）]+）年\d+月\d+日")

(def ^:private citation "docs/citation.md")
(def ^:private citation-src "soranoha/src/soranoha/za/citation.clj")
(def ^:private evidence "soranoha/docs/evidence/aozora-rights-source-contract.md")
(def ^:private identifiers "soranoha/docs/work-identifiers.md")
(def ^:private naming-src "soranoha/src/soranoha/za/naming.clj")
(def ^:private naming-test "soranoha/test/soranoha/za/naming_test.clj")
(def ^:private main-src "soranoha/src/soranoha/main.clj")
(def ^:private manifest-3 "docs/design/adr/0003-snh-manifest-3.md")
(def ^:private performance "docs/performance.md")
(def ^:private start-here "docs/start-here.md")
(def ^:private worked-example "docs/worked-example.md")
(def ^:private tei-header-src "soranoha/src/soranoha/ori/tei_header.clj")
(def ^:private tei-fidelity-test "soranoha/test/soranoha/tei_fidelity_test.clj")

(def digests
  "The pinned snapshot the figures were derived from: `[what sha256 quoted-in]`."
  [["the catalog archive"
    "5ea13273dd457f89af31de39f559ea9c6f5435d9bda1ae3d46d6a683b8bc3c92" [evidence]]
   ["the catalog CSV"
    "c0ace54c7ac037e5aebd045922c7879b9f7dc01f85b8f7483c8d4ecc29569ef4" [evidence]]])

(defn- figure [key population what quoted quoted-in]
  {:key key :population population :what what :quoted quoted :quoted-in quoted-in})

(def figures
  "One quoted count each: what it claims, over which population, and where."
  [(figure "catalog-rows" rows-pop "rows in the catalog CSV" 19470 [evidence])
   (figure "catalog-works" catalog-pop "works the catalog lists" 17810 [evidence identifiers])
   (figure "catalog-persons" catalog-pop "distinct people in the catalog" 1334 [evidence naming-src])
   (figure "works-with-text" catalog-pop "catalog works that have a downloadable text file" 17655
           [start-here worked-example])
   (figure "contributors" with-text "people credited on a work that has a text file" 1167 [start-here])
   (figure "author-title-works" catalog-pop "works sharing an author-and-title pair with another work" 2357
           [citation start-here identifiers tei-header-src citation-src naming-src naming-test tei-fidelity-test])
   (figure "author-title-pairs" catalog-pop "distinct author-and-title pairs shared by more than one work" 470
           [identifiers naming-src naming-test])
   (figure "author-title-subtitle-works" catalog-pop "works still ambiguous once the subtitle is added" 882
           [tei-header-src])
   (figure "six-field-works" catalog-pop
           "works still ambiguous under author, title, 副題, 文字遣い種別, 底本名 and 初出" 16
           [citation citation-src tei-fidelity-test])
   (figure "multi-author-works" catalog-pop "works recording more than one 著者" 422 [naming-src])
   (figure "card-not-first-author" catalog-pop
           "works filed under a card belonging to someone other than the first 著者" 146 [identifiers])
   (figure "persons-without-given-romaji" catalog-pop "people whose given name carries no romaji" 73
           [naming-src])
   (figure "longest-author-component" catalog-pop
           "characters in the longest rendered author filename component" 38 [naming-src])
   (figure "orthography-shinji-shinkana" with-text "新字新仮名 works" 10791 [worked-example])
   (figure "orthography-shinji-kyukana" with-text "新字旧仮名 works" 4569 [worked-example])
   (figure "orthography-kyuji-kyukana" with-text "旧字旧仮名 works" 2183 [worked-example])
   (figure "orthography-kyuji-shinkana" with-text "旧字新仮名 works" 93 [worked-example])
   (figure "orthography-other" with-text "その他 works" 19 [worked-example])
   (figure "orthography-split-titles" with-text
           "title-and-subtitle pairs recorded under more than one orthography" 568 [worked-example])
   (figure "orthography-split-works" with-text
           "works sharing a title and subtitle with a differently-spelled sibling" 1343 [worked-example])
   (figure "edition-year-values" rows-pop "rows recording a 底本初版発行年 for the first edition slot" 18780
           [citation citation-src])
   (figure "edition-year-values-both-slots" rows-pop
           "recorded 底本初版発行年 values across both edition slots" 18886 [citation-src])
   (figure "edition-year-plain" rows-pop "first-slot values that are a date and nothing else" 17747
           [citation citation-src])
   (figure "edition-year-irregular" rows-pop "first-slot values that are not a date and nothing else" 1033
           [citation citation-src])
   (figure "edition-year-appended" rows-pop
           "irregular values that are a date followed by a printing history" 667 [citation citation-src])
   ;; derived only: citation.clj writes the figure as prose rather than a numeral
   (figure "edition-year-not-leading" rows-pop "values whose four-digit year does not lead the string" 1 [])])

(def stem-figures
  "Figures that need a pass over every work archive."
  [(figure "apostrophe-stems" catalog-pop "archive stems using Hepburn's apostrophe" 204 [naming-src])
   (figure "shared-stems" catalog-pop "archive stems claimed by more than one work" 802 [naming-test])
   (figure "works-sharing-a-stem" catalog-pop "works whose archive stem is not theirs alone" 1908 [naming-test])
   (figure "median-stem-run" catalog-pop "median longest unbroken run of letters in an archive stem" 8
           [naming-src])])

(def history-figures
  "Figures derived from commit history rather than catalog metadata."
  [(figure "first-parent-commits" history "commits on aozorabunko's first-parent history" 5476
           [main-src manifest-3 performance])
   (figure "work-archive-commits" history "commits touching a work archive" 4673 [main-src])
   (figure "catalog-or-archive-commits" history
           "commits touching a work archive or the catalog directory" 5469 [main-src])
   (figure "catalog-only-commits" history
           "commits touching the catalog directory but no work archive" 796 [main-src])
   (figure "longest-skipped-run" skipped "longest unbroken run of commits changing no work archive" 14
           [main-src])
   (figure "longest-skip-wait-days" skipped
           "days a skipped commit waits for its covering release, at most" 14 [main-src])
   ;; derived only: cited in prose as a word or a bare digit
   (figure "median-skip-wait-days" skipped
           "days a skipped commit waits for its covering release, at the median" 2 [])])

(defn- strip
  "Whitespace trimmed by the Unicode White_Space property, so U+3000 counts."
  [^String s]
  (string/replace (or s "") #"^\p{IsWhite_Space}+|\p{IsWhite_Space}+$" ""))

(defn- median [values]
  (let [sorted (vec (sort values))
        n (count sorted)
        middle (quot n 2)]
    (if (odd? n)
      (double (sorted middle))
      (/ (+ (double (sorted (dec middle))) (double (sorted middle))) 2.0))))

(defn- round-half-even
  "Rounding to the nearest integer with ties to even, so the recorded figures
  do not depend on which rounding rule a tool defaults to."
  [x]
  (.longValue (.setScale (bigdec x) 0 RoundingMode/HALF_EVEN)))

(defn- git [aozora-root & arguments]
  (let [{:keys [exit out err]} @(process/process (into ["git" "-C" (str aozora-root)] arguments)
                                                 {:out :string :err :string})]
    (when-not (zero? exit)
      (throw (ex-info (str "git " (string/join " " arguments) " failed in " aozora-root ": " (string/trim err))
                      {:reason :git-failed :arguments arguments})))
    (string/split-lines out)))

(defn measure-history
  "Commit-history and skipped-commit figures from the first-parent log."
  [aozora-root]
  (let [commits (->> (git aozora-root "rev-list" "--first-parent" "--format=%H %ct" "--no-commit-header" "HEAD")
                     (remove string/blank?)
                     (map #(string/split % #"\s+"))
                     reverse)
        moved (set (git aozora-root "rev-list" "--first-parent" "HEAD" "--" work-archives))
        either (remove string/blank? (git aozora-root "rev-list" "--first-parent" "HEAD" "--" work-archives catalog-directory))
        {:keys [longest waits]}
        (reduce (fn [{:keys [current pending] :as state} [commit timestamp]]
                  (let [seconds (parse-long timestamp)]
                    (if (contains? moved commit)
                      (-> state
                          (update :waits into (map #(/ (- seconds %) 86400.0) pending))
                          (assoc :pending [] :current 0))
                      (let [current (inc current)]
                        (-> state
                            (assoc :current current)
                            (update :longest max current)
                            (update :pending conj seconds))))))
                {:longest 0 :current 0 :waits [] :pending []}
                commits)]
    {["first-parent-commits" history] (count commits)
     ["work-archive-commits" history] (count moved)
     ["catalog-or-archive-commits" history] (count either)
     ["catalog-only-commits" history] (- (count either) (count moved))
     ["longest-skipped-run" skipped] longest
     ["longest-skip-wait-days" skipped] (round-half-even (apply max waits))
     ["median-skip-wait-days" skipped] (round-half-even (median waits))}))

(defn- quotes?
  "Whether a file states this number, grouped with commas or not."
  [file value]
  (let [text (slurp (fs/file file))
        grouped (.format (NumberFormat/getIntegerInstance Locale/US) (long value))]
    (boolean (some (fn [written]
                     (re-find (re-pattern (str "(?<![0-9,])" (java.util.regex.Pattern/quote written) "(?![0-9,])")) text))
                   (distinct [(str value) grouped])))))

(defn digest-quotes
  "Each file citing a pinned digest still states it."
  [repo-root]
  (vec (for [[what sha256 quoted-in] digests
             name quoted-in
             :when (not (string/includes? (slurp (fs/file (fs/path repo-root name))) sha256))]
         (str what ": " name " no longer states " sha256))))

(defn figure-quotes
  "Each file citing a figure quotes the recorded value."
  [repo-root figures]
  (vec (for [{:keys [key quoted quoted-in]} figures
             name quoted-in
             :when (not (quotes? (fs/path repo-root name) quoted))]
         (str key ": " name " no longer states " quoted))))

(defn check-digests
  "The checkout matches the pinned catalog digests; when it does, the files
  citing them still state them."
  [repo-root aozora-root]
  (let [archive-path (fs/path aozora-root catalog-archive)
        {:keys [catalog-csv-hash]} (catalog/read-catalog-zip aozora-root)
        found [(hash/sha256-file (fs/file archive-path)) catalog-csv-hash]
        problems (for [[[what pinned _] actual] (map vector digests found)
                       :when (not= pinned actual)]
                   (str what ": pinned " pinned ", checkout has " actual))]
    (if (seq problems) (vec problems) (digest-quotes repo-root))))

(defn read-catalog
  "Catalog rows grouped by 作品ID, in the order the CSV lists them, each row a
  map from header to the cell as written."
  [aozora-root]
  (let [[header & rows] (charred/read-csv (:csv-text (catalog/read-catalog-zip aozora-root)))
        header (update (vec header) 0 #(string/replace % #"^﻿" ""))
        rows (map #(zipmap header (map (fnil identity "") %)) rows)]
    (->> rows
         (reduce (fn [[order groups] row]
                   (let [id (get row "作品ID")]
                     [(if (contains? groups id) order (conj order id))
                      (update groups id (fnil conj []) row)]))
                 [[] {}])
         ((fn [[order groups]] (mapv groups order))))))

(defn- authors [work]
  (filterv #(= author (strip (get % "役割フラグ"))) work))

(defn- author-key [work]
  (vec (sort (map #(get % "人物ID") (authors work)))))

(defn- ambiguous
  "`[works descriptions]`: works sharing a description with another, and how
  many descriptions are shared."
  [works fields]
  (let [shared (->> works
                    (map (fn [work] (into [(author-key work)] (map #(get (first work) %)) fields)))
                    frequencies
                    vals
                    (filter #(> % 1)))]
    [(reduce + 0 shared) (count shared)]))

(defn- author-component [work]
  (let [person (or (first (authors work)) (first work))]
    (string/join "_" (remove string/blank?
                             [(naming/ascii-component (get person "姓ローマ字"))
                              (naming/ascii-component (get person "名ローマ字"))]))))

(defn- archive-stems
  "Text member filename stems of the work archives present in the checkout."
  [aozora-root works]
  (vec (for [work works
             :let [[_ directory archive] (re-find text-archive (strip (get (first work) "テキストファイルURL")))]
             :when directory
             :let [path (fs/path aozora-root "cards" directory "files" archive)]
             :when (fs/exists? path)
             :let [members (try (with-open [zip (ZipFile. (fs/file path))]
                                  (->> (enumeration-seq (.entries zip))
                                       (map #(.getName ^java.util.zip.ZipEntry %))
                                       (filterv #(string/ends-with? (string/lower-case %) ".txt"))))
                                (catch Exception _ nil))]
             :when (= 1 (count members))]
         (let [name (last (string/split (first members) #"/"))
               dot (string/last-index-of name ".")]
           (if (and dot (pos? dot)) (subs name 0 dot) name)))))

(defn- zero-padded
  "A person id as the six-digit card directory writes it."
  [id]
  (str (apply str (repeat (max 0 (- 6 (count id))) "0")) id))

(defn- longest-run [value]
  (reduce max 0 (map count (remove string/blank? (string/split value #"[^A-Za-z0-9]+")))))

(defn measure
  "The catalog figures over the grouped rows."
  [works]
  (let [texted (filterv #(seq (strip (get (first %) "テキストファイルURL"))) works)
        [author-title author-title-pairs] (ambiguous works ["作品名"])
        filed-elsewhere (count (filter (fn [work]
                                         (let [[_ directory] (re-find card-directory (get (first work) "テキストファイルURL"))
                                               written (authors work)]
                                           (and directory (seq written)
                                                (not= (zero-padded (get (first written) "人物ID")) directory))))
                                       works))
        orthography (frequencies (map #(strip (get (first %) "文字遣い種別")) texted))
        by-title (reduce (fn [acc work]
                           (let [style (strip (get (first work) "文字遣い種別"))]
                             (if (seq style)
                               (update acc [(get (first work) "作品名") (get (first work) "副題")] (fnil conj #{}) style)
                               acc)))
                         {} texted)
        split-titles (set (keep (fn [[title styles]] (when (> (count styles) 1) title)) by-title))
        split-works (count (filter #(contains? split-titles [(get (first %) "作品名") (get (first %) "副題")]) texted))
        rows (vec (mapcat identity works))
        first-slot (filterv seq (map #(strip (get % "底本初版発行年1")) rows))
        both-slots (into first-slot (filter seq (map #(strip (get % "底本初版発行年2")) rows)))
        irregular (remove #(re-matches plain-edition-date %) first-slot)
        person-ids (fn [works pred] (count (distinct (for [work works row work :when (pred row)] (get row "人物ID")))))]
    {["catalog-rows" rows-pop] (count rows)
     ["catalog-works" catalog-pop] (count works)
     ["catalog-persons" catalog-pop] (person-ids works (constantly true))
     ["works-with-text" catalog-pop] (count texted)
     ["contributors" with-text] (person-ids texted (constantly true))
     ["author-title-works" catalog-pop] author-title
     ["author-title-pairs" catalog-pop] author-title-pairs
     ["author-title-subtitle-works" catalog-pop] (first (ambiguous works ["作品名" "副題"]))
     ["six-field-works" catalog-pop] (first (ambiguous works ["作品名" "副題" "文字遣い種別" "底本名1" "初出"]))
     ["multi-author-works" catalog-pop] (count (filter #(> (count (authors %)) 1) works))
     ["card-not-first-author" catalog-pop] filed-elsewhere
     ["persons-without-given-romaji" catalog-pop] (person-ids works #(string/blank? (strip (get % "名ローマ字"))))
     ["longest-author-component" catalog-pop] (reduce max 0 (map #(count (author-component %)) works))
     ["orthography-shinji-shinkana" with-text] (get orthography "新字新仮名" 0)
     ["orthography-shinji-kyukana" with-text] (get orthography "新字旧仮名" 0)
     ["orthography-kyuji-kyukana" with-text] (get orthography "旧字旧仮名" 0)
     ["orthography-kyuji-shinkana" with-text] (get orthography "旧字新仮名" 0)
     ["orthography-other" with-text] (get orthography "その他" 0)
     ["orthography-split-titles" with-text] (count split-titles)
     ["orthography-split-works" with-text] split-works
     ["edition-year-values" rows-pop] (count first-slot)
     ["edition-year-values-both-slots" rows-pop] (count both-slots)
     ["edition-year-plain" rows-pop] (- (count first-slot) (count irregular))
     ["edition-year-irregular" rows-pop] (count irregular)
     ["edition-year-appended" rows-pop] (count (filter #(re-find plain-edition-date %) irregular))
     ["edition-year-not-leading" rows-pop] (count (remove #(re-find #"(?U)^\d{4}" %) both-slots))}))

(defn measure-stems [aozora-root works]
  (let [stems (archive-stems aozora-root works)
        shared (filter #(> % 1) (vals (frequencies stems)))]
    {["apostrophe-stems" catalog-pop] (count (filter #(string/includes? % "'") stems))
     ["shared-stems" catalog-pop] (count shared)
     ["works-sharing-a-stem" catalog-pop] (reduce + 0 shared)
     ["median-stem-run" catalog-pop] (long (median (map longest-run stems)))}))

(defn check
  "Finds figures whose derivations differ from their quoted values, followed by
  quotation discrepancies among figures that remain consistent."
  [repo-root figures measured]
  (let [{:keys [problems agreeing]}
        (reduce (fn [acc {:keys [key population what quoted] :as figure}]
                  (let [derived (get measured [key population])]
                    (if (not= derived quoted)
                      (update acc :problems conj
                              (str key ": prose says " quoted ", the catalog gives " derived
                                   " (" what ", over " population ")"))
                      (update acc :agreeing conj figure))))
                {:problems [] :agreeing []}
                figures)]
    (into problems (figure-quotes repo-root agreeing))))

(defn- report! [problems]
  (binding [*out* *err*]
    (doseq [problem problems] (println (str "catalog-figures: " problem)))))

(defn gate!
  "Executes quotation verification when `:quotes-only` is set; otherwise verifies
  derivations against the checkout at `:aozora-root`. Returns the exit code."
  [{:keys [repo aozora-root stems quotes-only]}]
  (if quotes-only
    (let [all (concat figures stem-figures history-figures)
          problems (into (digest-quotes repo) (figure-quotes repo all))]
      (report! problems)
      (if (seq problems)
        1
        (do (println (str "checked " (+ (reduce + (map #(count (:quoted-in %)) all))
                                        (reduce + (map #(count (nth % 2)) digests)))
                          " citations of " (count all) " figures; nothing re-derived"))
            0)))
    (cond
      (string/blank? aozora-root)
      (do (report! ["--aozora-root or CORPUS_CHECKOUT is required"]) 2)

      (not (fs/exists? (fs/path aozora-root catalog-archive)))
      (do (report! [(str "no catalog at " (fs/path aozora-root catalog-archive))]) 2)

      :else
      (let [digest-problems (check-digests repo aozora-root)]
        (if (seq digest-problems)
          (do (report! digest-problems) 1)
          (let [works (read-catalog aozora-root)
                all (cond-> (into (vec figures) history-figures) stems (into stem-figures))
                measured (cond-> (merge (measure works) (measure-history aozora-root))
                           stems (merge (measure-stems aozora-root works)))
                problems (check repo all measured)]
            (report! problems)
            (if (seq problems)
              1
              (do (println (str "checked " (count digests) " catalog digests and " (count all)
                                " corpus figures"
                                (when-not stems "; archive stems not checked, pass --stems")))
                  0))))))))
