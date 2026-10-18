(ns soranoha.gates.docs-links
  "Relative Markdown links and fragment anchors across the project documents
  resolve to a file and, for a Markdown target, to one of its headings."
  (:require [babashka.fs :as fs]
            [clojure.string :as string]))

(def ^:private roots ["docs" "soranoha/docs"])
(def ^:private link #"\[(?:[^\]]+)\]\(([^)\s]+)\)")
(def ^:private heading #"^#{1,6} (.*)$")
(def ^:private external #"^(?:https?://|mailto:|ftp://)")

(defn slug
  "The anchor a Markdown renderer derives from a heading: lower-cased,
  letters, digits and marks kept along with space, underscore, hyphen and
  tab, then runs of whitespace joined into one hyphen."
  [^String text]
  (let [out (StringBuilder.)]
    (doseq [cp (.toArray (.codePoints (string/lower-case text)))]
      (let [type (Character/getType (int cp))]
        ;; Character types 1 to 11 are the letter, mark and number categories
        (when (or (<= Character/UPPERCASE_LETTER type Character/OTHER_NUMBER)
                  (contains? #{(int \space) (int \_) (int \-) (int \tab)} cp))
          (.appendCodePoint out (int cp)))))
    (-> (str out) string/trim (string/replace #"\s+" "-"))))

(defn- headings [file]
  (into #{}
        (keep (fn [line] (when-let [[_ text] (re-matches heading line)] (slug text))))
        (string/split-lines (slurp (fs/file file)))))

(defn- problems-in [repo-root document]
  (let [here (str (fs/relativize repo-root document))]
    (for [[_ target] (re-seq link (slurp (fs/file document)))
          :when (not (re-find external target))
          :let [[path fragment] (string/split target #"#" 2)
                resolved (fs/normalize (fs/absolutize (if (string/blank? path)
                                                        document
                                                        (fs/path (fs/parent document) path))))]
          :let [problem (cond
                          (not (fs/exists? resolved)) (str here ": " target " does not exist")
                          (and fragment (= "md" (fs/extension resolved))
                               (not (contains? (headings resolved) fragment)))
                          (str here ": " target " names no heading"))]
          :when problem]
      problem)))

(defn check
  "`{:documents n :problems [message ...]}` over every Markdown document under
  the documentation roots of `repo-root`."
  [repo-root]
  (let [repo-root (fs/absolutize repo-root)
        documents (sort (mapcat #(fs/glob (fs/path repo-root %) "**.md") roots))]
    {:documents (count documents)
     :problems (vec (mapcat #(problems-in repo-root %) documents))}))

(defn gate!
  "Print the problems and the summary line; the exit code for the kernel."
  [repo-root]
  (let [{:keys [documents problems]} (check repo-root)]
    (binding [*out* *err*]
      (doseq [problem problems] (println problem)))
    (println (str "checked " documents " documents, " (count problems) " unresolved links"))
    (if (seq problems) 1 0)))
