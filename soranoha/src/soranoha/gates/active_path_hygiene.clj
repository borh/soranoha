(ns soranoha.gates.active-path-hygiene
  "Lexical verification that file paths in active code remain within this
  monorepo, rejecting references to external checkouts or untracked
  reference directories without source evaluation."
  (:require [babashka.fs :as fs]
            [clojure.string :as string]))

(def ^:private roots
  ["flake.nix" "justfile" "config" "scripts" "tests" "nix" "soranoha/src"
   "ab-validator/flake.nix" "ab-validator/justfile" "ab-validator/adapters"
   "ab-validator/crates" "ab-validator/scripts"])

(def ^:private active-names #{"Cargo.toml" "justfile"})
(def ^:private active-extensions #{"clj" "nix" "rs" "sh" "toml"})
(def ^:private excluded-directories #{"target" "__pycache__"})

(def ^:private targets #{"soranoha" "ab-validator"})
(def ^:private approved-roots
  #{"$repo_root" "$src" "$work_dir" "$AB_WORKSPACE_ROOT" "$SORANOHA_WORKSPACE_ROOT"})
(def ^:private token #"[^\s\"'`]+")
(def ^:private trim-characters "()[],:;{}")

;; Assembled rather than written out, since every file under soranoha/src is
;; itself subject to this check.
(def ^:private forbidden-references
  [(str "references/" "parsers/") (str "soranoha/references/" "TEI/P5")])

(defn- trim-edges [text]
  (let [trim? (set trim-characters)]
    (->> text
         (drop-while trim?)
         reverse
         (drop-while trim?)
         reverse
         (apply str))))

(defn- components
  "Non-empty path components excluding `.`, returning elements without the root marker."
  [path]
  (vec (remove #{"" "."} (string/split path #"/"))))

(defn normpath
  "Path normalization: collapses redundant separators and `.`, resolves `..`
  against preceding components, and preserves leading `..` in relative paths."
  [path]
  (let [absolute? (string/starts-with? path "/")
        resolved (reduce (fn [acc part]
                           (cond
                             (not= ".." part) (conj acc part)
                             (and (seq acc) (not= ".." (peek acc))) (pop acc)
                             absolute? acc
                             :else (conj acc part)))
                         []
                         (components path))
        joined (string/join "/" resolved)]
    (cond
      absolute? (str "/" joined)
      (empty? joined) "."
      :else joined)))

(defn token-suffixes
  "Returns a token alongside substrings following each `=` or `:`, permitting
  inspection of assignments and flags without evaluating surrounding expressions."
  [raw]
  (let [cleaned (trim-edges raw)]
    (if (string/includes? cleaned "://")
      [cleaned]
      (into [cleaned]
            (for [[index character] (map-indexed vector cleaned)
                  :when (contains? #{\= \:} character)
                  :let [suffix (-> (subs cleaned (inc index))
                                   trim-edges
                                   (string/replace #"^-+" ""))]
                  :when (seq suffix)]
              suffix)))))

(defn candidate-tokens
  "Extracts path-shaped tokens from `text` that reference a target checkout and
  either traverse upward via `..` or begin at the filesystem root."
  [text]
  (vec (for [raw (re-seq token text)
             suffix (token-suffixes raw)
             :when (and (string/includes? suffix "/")
                        (some #(string/includes? suffix (str % "/")) targets)
                        (or (some #{".."} (components suffix))
                            (string/starts-with? suffix "/")))]
         suffix)))

(defn- lexical-base
  "Base directory for relative paths in `source`: the parent directory for
  Rust, Nix, and Cargo manifests, and the repository root for other sources."
  [repo source]
  (if (or (contains? #{"rs" "nix"} (fs/extension source))
          (= "Cargo.toml" (fs/file-name source)))
    (let [parent (fs/parent source)]
      (if parent (str repo "/" parent) repo))
    repo))

(defn- relative-components
  "Path components relative to `repo`, or nil when `path` lies outside the repository."
  [repo path]
  (cond
    (= path repo) []
    (string/starts-with? path (str repo "/")) (components (subs path (inc (count repo))))
    :else nil))

(defn- outside-targets? [repo normalized]
  (let [relative (relative-components repo normalized)]
    (or (nil? relative)
        (empty? relative)
        (not (contains? targets (first relative))))))

(defn forbidden-candidate?
  "Checks whether `raw`, located in `source` relative to absolute `repo`,
  resolves outside repository component trees. Because shell and environment
  expansions are not performed, unresolved variable references fail verification."
  [repo source raw]
  (let [candidate (trim-edges raw)
        parts (components candidate)]
    (cond
      (not (some targets parts)) false
      (or (string/includes? candidate "://") (string/starts-with? candidate "{https:")) false
      (some #(string/starts-with? candidate (str % "/")) approved-roots) (boolean (some #{".."} parts))
      (re-find #"[${}]" candidate) true
      (and (string/starts-with? candidate "/../")
           (= "rs" (fs/extension source))
           (some #{"crates"} (components source)))
      (let [source-parts (components source)
            crate-index (inc (.indexOf ^java.util.List source-parts "crates"))
            crate-root (string/join "/" (cons repo (take (inc crate-index) source-parts)))]
        (outside-targets? repo (normpath (str crate-root candidate))))
      (string/starts-with? candidate "/") true
      :else (outside-targets? repo (normpath (str (lexical-base repo source) "/" candidate))))))

(defn- active-file? [relative]
  (let [parts (components relative)
        name (peek parts)]
    (and (or (contains? active-names name)
             (contains? active-extensions (fs/extension name)))
         (not-any? excluded-directories (pop parts)))))

(defn- active-files
  "Sorted list of repository-relative paths inspected during the gate check."
  [repo-root]
  (sort (for [root roots
              :let [path (fs/path repo-root root)]
              :when (fs/exists? path)
              file (if (fs/directory? path)
                     (filter fs/regular-file? (fs/glob path "**" {:hidden true}))
                     [path])
              :let [relative (str (fs/relativize repo-root file))]
              :when (active-file? relative)]
          relative)))

(defn- escapes-in [repo relative]
  (for [[index line] (map-indexed vector (string/split-lines (slurp (fs/file (fs/path repo relative)))))
        candidate (candidate-tokens line)
        :when (forbidden-candidate? repo relative candidate)]
    (str relative ":" (inc index) ":" candidate)))

(defn- references-in [repo relative]
  (for [[index line] (map-indexed vector (string/split-lines (slurp (fs/file (fs/path repo relative)))))
        :when (some #(string/includes? line %) forbidden-references)]
    (str relative ":" (inc index) ":" line)))

(defn check
  "Evaluates active files under `repo-root`, returning a map of file counts,
  detected paths escaping the monorepo, and references to untracked sources."
  [repo-root]
  (let [repo (str (fs/normalize (fs/absolutize repo-root)))
        files (active-files repo)]
    {:files (count files)
     :escapes (vec (mapcat #(escapes-in repo %) files))
     :references (vec (mapcat #(references-in repo %) files))}))

(defn gate!
  "Reports path hygiene findings and returns the exit code for the kernel."
  [repo-root]
  (let [{:keys [files escapes references]} (check repo-root)]
    (binding [*out* *err*]
      (when (zero? files)
        (println "no active files found for path hygiene check"))
      (doseq [finding escapes] (println finding))
      (when (seq escapes)
        (println "active code must not escape into a sibling soranoha/ab-validator checkout"))
      (doseq [finding references] (println finding))
      (when (seq references)
        (println "active code must not depend on sibling checkout or untracked references/ paths")))
    (if (or (zero? files) (seq escapes) (seq references))
      1
      (do (println (str "active path hygiene passed: " files " files")) 0))))
