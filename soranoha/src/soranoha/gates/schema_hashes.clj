(ns soranoha.gates.schema-hashes
  "Verifies that schema hashes recorded in repository documents match the
  canonical digests of their referenced schemas."
  (:require [babashka.fs :as fs]
            [clojure.string :as string]
            [soranoha.core.hash :as hash]
            [soranoha.core.json :as json]))

(def ^:private skipped
  "Directories containing historical records pinned to earlier schema versions."
  [".beads" ".git" "archive" "ab-validator/docs"])

(def ^:private id-suffix "_schema_id")
(def ^:private hash-suffix "_schema_hash")

(defn schema-hashes
  "Maps each published schema's `$id` to its canonical SHA-256 digest."
  [repo-root]
  (into (sorted-map)
        (map (fn [path]
               (let [schema (json/read-json-file (fs/file path))
                     id (get schema "$id")]
                 (when-not (string? id)
                   (throw (ex-info (str path " has no string $id") {:path (str path)})))
                 [id (str "sha256:" (hash/sha256-canonical-json schema))])))
        (sort (fs/glob (fs/path repo-root "soranoha" "schemas") "*.schema.json"))))

(defn record-pins
  "Extracts `[schema-id schema-hash]` pairs from sibling members
  `<name>_schema_id` and `<name>_schema_hash` at any depth."
  [node]
  (cond
    (map? node)
    (into (vec (for [[k v] node
                     :when (and (string? k) (string/ends-with? k id-suffix) (string? v))
                     :let [paired (get node (str (subs k 0 (- (count k) (count id-suffix))) hash-suffix))]
                     :when (string? paired)]
                 [v paired]))
          (mapcat record-pins (vals node)))

    (sequential? node) (vec (mapcat record-pins node))
    :else []))

(defn- json-files [repo-root]
  (let [skipped-dirs (set (map #(str (fs/path repo-root %)) skipped))]
    (->> (fs/glob repo-root "**.json" {:hidden true})
         (remove (fn [path] (some #(contains? skipped-dirs (str %)) (take-while some? (iterate fs/parent (fs/parent path))))))
         sort)))

(defn- parsed-or-nil [path]
  (try (json/read-json-file (fs/file path))
       (catch Exception _ nil)))

(defn check
  "`{:schemas n :errors [message ...]}`: each stated pin whose schema is
  published and whose digest differs."
  [repo-root]
  (let [repo-root (fs/absolutize repo-root)
        hashes (schema-hashes repo-root)]
    {:schemas (count hashes)
     :errors (vec (for [path (json-files repo-root)
                        :let [document (parsed-or-nil path)]
                        :when document
                        [schema-id recorded] (record-pins document)
                        :let [expected (get hashes schema-id)]
                        :when (and expected (not= recorded expected))]
                    (str (fs/relativize repo-root path) ": " schema-id
                         " recorded " recorded ", schema hashes " expected)))}))

(defn gate!
  "Print the errors and the summary line; the exit code for the kernel."
  [repo-root]
  (let [{:keys [schemas errors]} (check repo-root)]
    (binding [*out* *err*]
      (doseq [error errors] (println error)))
    (if (seq errors)
      1
      (do (println (str "schema hash coherence passed: " schemas " published schemas")) 0))))
