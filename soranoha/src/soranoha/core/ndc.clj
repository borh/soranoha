(ns soranoha.core.ndc
  "Three-digit classes of 日本十進分類法 新訂9版 with Japanese and English labels.

  Labels derive from the Japan Library Association's NDC9 Linked Data
  release (https://www.jla.or.jp/committees/bunrui/ndc-data/, CC BY), pinned
  as the `ndc9-ld` flake input. The committed resource at
  `resources/ndc/ndc9-sections.json` is generated from `ndc9.ttl` via `render`,
  verified through the `ndc9-sections` kernel command, and referenced in
  TEI header generation."
  (:require [clojure.java.io :as io]
            [clojure.string :as string]
            [soranoha.core.hash :as hash]
            [soranoha.core.json :as json]))

(def scheme-iri "http://jla.or.jp/data/ndc9")

(def ^:private source-url
  "https://www.jla.or.jp/wp/wp-content/uploads/2025/06/ndc9.zip")

(def ^:private resource-path "ndc/ndc9-sections.json")

(defn sections
  "Extracts three-digit classes from a Turtle document, returning digits mapped
  to `{\"ja\" label, \"en\" label}`. Subject blocks are delimited by a period on
  its own line, with three-digit classes designated by `ndc9:NNN`. Classes
  lacking a Japanese preferred label are omitted."
  [ttl]
  (into (sorted-map)
        (keep (fn [block]
                (when-let [[_ digits] (re-find #"^\s*ndc9:(\d{3}) a " block)]
                  (when-let [[_ labels] (re-find #"(?s)skos:prefLabel\s+(.*?)\s*(?:;|$)" block)]
                    (let [entry (into {}
                                      (map (fn [[_ text lang]] [lang text]))
                                      (re-seq #"\"((?:[^\"\\]|\\.)*)\"@(ja|en)" labels))]
                      (when (contains? entry "ja")
                        [digits entry]))))))
        (string/split ttl #"\n\.\n")))

(defn render
  "Generates deterministic JSON for an `ndc9.ttl` file to produce the committed
  resource text."
  [ttl-file]
  (let [file (io/file ttl-file)
        bytes (java.nio.file.Files/readAllBytes (.toPath file))]
    (str (json/write-deterministic-json-str
          {"source" {"title" "日本十進分類法新訂9版 Linked Data (ndc9), 日本図書館協会"
                     "url" source-url
                     "file" (.getName file)
                     "sha256" (hash/sha256-bytes bytes)
                     "licence" "CC BY"
                     "scheme_iri" scheme-iri}
           "sections" (sections (String. bytes "UTF-8"))})
         "\n")))

(def ^:private table
  (delay (with-open [in (io/input-stream (io/resource resource-path))]
           (get (json/read-json-bytes (.readAllBytes in)) "sections"))))

(defn labels
  "Returns `{\"ja\" label, \"en\" label}` for a three-digit code, or nil when
  unlisted in the table."
  [digits]
  (get @table digits))
