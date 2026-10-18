(ns soranoha.gates.schema-hashes-test
  (:require [babashka.fs :as fs]
            [charred.api :as json]
            [clojure.test :refer [deftest is testing]]
            [soranoha.core.hash :as hash]
            [soranoha.gates.schema-hashes :as schema-hashes]))

(deftest the-canonical-digest-is-the-one-the-records-were-written-against
  ;; sha256 over sorted-key, compact, UTF-8 JSON, which is what produced every
  ;; recorded pin; the value here was digested independently of this code.
  (is (= "eceba50a7cc0584b283170878c05eacbb0c4b3535e58437c1490a5620e0d06eb"
         (hash/sha256-canonical-json {"b" [1 2 {"z" true "y" nil}] "a" "éé\"\\\n" "é" "x"}))))

(deftest pins-are-sibling-id-and-hash-members-at-any-depth
  (is (= [["urn:a" "sha256:1"] ["urn:b" "sha256:2"]]
         (schema-hashes/record-pins
          {"source_schema_id" "urn:a" "source_schema_hash" "sha256:1"
           "unpaired_schema_id" "urn:c"
           "nested" [{"x" {"target_schema_id" "urn:b" "target_schema_hash" "sha256:2"}}]}))))

(deftest a-record-pinning-a-stale-digest-is-reported-with-both-values
  (let [repo (fs/create-temp-dir {:prefix "schema-hashes"})
        schema {"$id" "https://example.org/thing.schema.json" "type" "object"}
        digest (str "sha256:" (hash/sha256-canonical-json schema))
        write (fn [path value] (fs/create-dirs (fs/parent (fs/path repo path)))
                (spit (fs/file (fs/path repo path)) (json/write-json-str value)))]
    (try
      (write "soranoha/schemas/thing.schema.json" schema)
      (write "records/fresh.json" {"thing_schema_id" (get schema "$id") "thing_schema_hash" digest})
      (write "records/stale.json" {"thing_schema_id" (get schema "$id") "thing_schema_hash" "sha256:0"})
      (write "records/other.json" {"thing_schema_id" "urn:unpublished" "thing_schema_hash" "sha256:0"})
      (write "archive/old.json" {"thing_schema_id" (get schema "$id") "thing_schema_hash" "sha256:0"})
      (spit (fs/file (fs/path repo "records" "broken.json")) "{not json")
      (let [{:keys [schemas errors]} (schema-hashes/check repo)]
        (is (= 1 schemas))
        (is (= [(str "records/stale.json: " (get schema "$id") " recorded sha256:0, schema hashes " digest)]
               errors)
            "an unpublished schema, an archived record and an unparsable file are passed over"))
      (testing "the same tree passes once the stale pin is corrected"
        (write "records/stale.json" {"thing_schema_id" (get schema "$id") "thing_schema_hash" digest})
        (is (= [] (:errors (schema-hashes/check repo)))))
      (finally (fs/delete-tree repo)))))
