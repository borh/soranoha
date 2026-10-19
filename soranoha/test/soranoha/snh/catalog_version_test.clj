(ns soranoha.snh.catalog-version-test
  "The catalog schema boundary. A published chain carries the catalog version
  it was signed under, so reading those bytes must select the historical
  snh-catalog/1 as well as the current snh-catalog/2; the writer stays on
  snh-catalog/2. The chain verifier decodes the head manifest's catalog
  through the same boundary, so one case here builds a real chain that
  publishes v1 and verifies it."
  (:require [charred.api :as json]
            [clojure.java.io :as io]
            [clojure.test :refer [deftest is testing]]
            [soranoha.core.canonical :as canonical]
            [soranoha.core.hash :as hash]
            [soranoha.snh.decode :as decode]
            [soranoha.snh.fixture :as fx]
            [soranoha.snh.verify :as verify]
            [soranoha.snh.view :as view]))

(defn- vector-json [name]
  (json/read-json (slurp (io/resource (str "snh/vectors/" name)))))

(defn- canonical-bytes ^bytes [value]
  (canonical/rfc8785-safe-integer-json-bytes-v1 value))

(defn- reason-of [thunk]
  (try (thunk) :accepted
       (catch clojure.lang.ExceptionInfo e (:reason (ex-data e)))))

(defn- without-body-count [catalog]
  (update catalog "works"
          (fn [works] (mapv #(dissoc % "body_character_count") works))))

(defn- catalog-v1-value []
  (-> (vector-json "catalog-valid.json")
      (assoc "schema" "snh-catalog/1")
      without-body-count))

(deftest a-historically-signed-catalog-v1-decodes-unchanged
  (let [value (catalog-v1-value)
        bytes (canonical-bytes value)
        decoded (decode/decode "catalog" bytes)]
    (testing "the historical version is read, not rewritten"
      (is (= "snh-catalog/1" (get-in decoded [:value "schema"])))
      (is (= (hash/sha256-bytes bytes) (:hex decoded)))
      (is (= (str "snh:1:catalog:" (hash/sha256-bytes bytes)) (:id decoded)))
      (is (java.util.Arrays/equals bytes (canonical-bytes (:value decoded)))
          "the decoded value re-encodes to the same historical bytes"))))

(deftest a-malformed-catalog-v1-is-refused
  (let [missing (update (catalog-v1-value) "works"
                        (fn [works] (mapv #(dissoc % "rights") (take 1 works))))]
    (is (= :schema-invalid
           (reason-of #(decode/decode "catalog" (canonical-bytes missing)))))))

(deftest an-unknown-catalog-version-is-refused
  (is (= :schema-invalid
         (reason-of #(decode/decode
                      "catalog"
                      (canonical-bytes
                       (assoc (catalog-v1-value) "schema" "snh-catalog/3")))))))

(deftest the-current-catalog-v2-still-requires-the-body-count
  (is (= :schema-invalid
         (reason-of #(decode/decode
                      "catalog"
                      (canonical-bytes
                       (without-body-count (vector-json "catalog-valid.json"))))))))

(deftest the-writer-stays-on-the-current-catalog-version
  (is (= :schema-invalid
         (reason-of #(decode/encode "catalog" (catalog-v1-value)))))
  (is (= "snh-catalog/2"
         (get-in (decode/encode "catalog" (vector-json "catalog-valid.json"))
                 [:value "schema"]))))

;; The same boundary decode runs inside the chain verifier's catalog check.

(def slug-a "hashire_merosu_000035_1567")
(def slug-b "kumo_no_ito_000879_92")

(defn- assembled [opts head-value]
  ((fx/make-assemble (merge {:admitted [slug-a slug-b]} opts)) head-value))

(defn- chain-ctx []
  (let [{:keys [clone] :as repos} (fx/make-repos!)]
    (fx/publish! clone {:admitted [slug-a slug-b]})
    (fx/publish! clone {:admitted [slug-a slug-b]
                        :selection-params {"config" "fixture" "round" 2}})
    (let [head-commit (fx/head-of clone)]
      (merge repos {:head-commit head-commit
                    :head (fx/manifest-at clone head-commit)}))))

(defn- blob-files [blobs]
  (into {} (map (fn [[hex bytes]] [(verify/blob-path hex) bytes])) blobs))

(defn- verify-reason-at [clone commit]
  (try
    (verify/verify-repository-at (view/git-view clone) commit (fx/pinned-keys))
    :valid
    (catch clojure.lang.ExceptionInfo e (:reason (ex-data e)))))

(deftest a-chain-publishing-a-catalog-v1-verifies
  (let [{:keys [clone head-commit head]} (chain-ctx)
        {:keys [core blobs]} (assembled {:selection-params {"config" "fixture" "round" 3}}
                                        (:value head))
        v2-bytes (get blobs (verify/id->hex (get core "catalog")))
        v1-value (-> (json/read-json (String. ^bytes v2-bytes "UTF-8"))
                     (assoc "schema" "snh-catalog/1")
                     without-body-count)
        v1-bytes (canonical-bytes v1-value)
        v1-hex (hash/sha256-bytes v1-bytes)
        manifest (assoc core
                        "prev_manifest" (:hex head)
                        "withdrawn" [] "governance_event" nil
                        "catalog" (str "snh:1:catalog:" v1-hex))
        commit (:commit (fx/craft-release!
                         clone {:parents [head-commit]
                                :base-tree-of head-commit
                                :manifest-value manifest
                                :extra-files (blob-files (assoc blobs v1-hex v1-bytes))
                                :keep-catalog true}))]
    (is (= :valid (verify-reason-at clone commit)))))
