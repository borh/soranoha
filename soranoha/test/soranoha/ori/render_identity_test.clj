(ns soranoha.ori.render-identity-test
  (:require [babashka.fs :as fs]
            [charred.api :as json]
            [clojure.test :refer [deftest is]]
            [next.jdbc :as jdbc]
            [soranoha.core.hash :as hash]
            [soranoha.kura.cas :as cas]
            [soranoha.kura.engine :as engine]
            [soranoha.kura.trace :as trace]
            [soranoha.main :as main]
            [soranoha.ori.accountability :as accountability]
            [soranoha.ori.stages :as stages]))

(defn- json-bytes [value]
  (.getBytes ^String (json/write-json-str value) "UTF-8"))

(defn- fixture-stage [id outputs]
  {:stage-id id :stage-version "1" :toolchain-id "fixture"
   :f (fn [_ _] outputs)})

(deftest facts-only-reuse-keeps-current-full-ir-and-coverage-provenance
  (let [dir (fs/create-temp-dir {:prefix "render-identity"})
        source (str (fs/path dir "source.txt"))
        _ (spit source "本文")
        source-hash (hash/format-sha256 (hash/sha256-file source))
        ir {"nodes" [{"type" "text" "text" "本文"}]
            "source" {"work_content_hash" source-hash "primary_text_hash" source-hash
                      "decode_outcome" "utf-8"}
            "warnings" [] "errors" [] "interpretation_problems" [] "interpretation_facts" []}
        calls (atom 0)
        render (stages/render-stage "fixture" {"encoding" "CC0-1.0"
                                               "statement_url" "https://w3id.org/soranoha/rights"})
        render (update render :f (fn [f] (fn [resolve inputs] (swap! calls inc) (f resolve inputs))))
        store (engine/open-store! {:cas-dir (str (fs/path dir "objects"))
                                   :db-path (str (fs/path dir "trace.sqlite"))})
        stage-set {:extract (fixture-stage "extract" {"primary-text" (.getBytes "本文" "UTF-8")
                                                      "source-facts" (json-bytes {"work_content_hash" source-hash})})
                   :metadata (fixture-stage "metadata" {"metadata-record" (json-bytes {"work" {"title" "試験" "work_id" "1"} "contributors" []})
                                                        "persons" (json-bytes {})})
                   :parse (fixture-stage "parse" {"aat" (json-bytes {})})
                   :render render
                   :plaintext (stages/plaintext-stage "fixture")
                   :markdown (stages/markdown-stage "fixture")
                   :validate (fixture-stage "validate" {"tei-validation" (json-bytes {"status" "passed"})})
                   :accountability (fixture-stage "accountability" {"source-accountability" (json-bytes {"schema" "aozora-source-accountability/2"
                                                                                                         "source_sha256" source-hash "encoding" "utf-8"
                                                                                                         "decode_outcome" "lossless" "occurrences" []})})
                   :coverage (accountability/coverage-stage "fixture")}
        run (fn [version value]
              (main/run-work! store
                              (assoc stage-set :convert (assoc (fixture-stage "convert" {"parser-ir" (json-bytes value)}) :stage-version version))
                              {:slug "000001_000001" :row {"作品ID" "1"} :file source :rights "public-domain"} []))]
    (try
      (let [first-run (run "a" ir)
            changed (assoc ir "interpretation_facts" [{"kind" "ruby" "outcome" "established" "aspects" ["content"]
                                                       "source_span" {"start" 0 "end" 6 "coordinate_system" "decoded_utf8"}}])
            second-run (run "b" changed)
            full-ir (get-in second-run [:outputs :convert "parser-ir"])
            coverage (json/read-json (String. ^bytes (cas/get-bytes (:cas-dir store) (get-in second-run [:outputs :coverage "interpretation-coverage"])) "UTF-8"))
            row (first (jdbc/execute! (get-in store [:trace :conn]) ["SELECT inputs_json FROM trace WHERE trace_key = ?" (get-in second-run [:trace-keys :render])]))
            recorded (json/read-json (:trace/inputs_json row))]
        (is (= 1 @calls))
        (is (get-in second-run [:cached :render]))
        (is (= (get-in first-run [:trace-keys :render]) (get-in second-run [:trace-keys :render])))
        (is (= (get-in first-run [:outputs :render]) (get-in second-run [:outputs :render])))
        (is (not= (get-in first-run [:outputs :convert "parser-ir"]) full-ir))
        (is (= changed (json/read-json (String. ^bytes (cas/get-bytes (:cas-dir store) full-ir) "UTF-8"))))
        (is (= (str "sha256:" full-ir) (get-in coverage ["artifacts" "parser-ir"])))
        (is (not (contains? recorded "parser-ir")))
        (is (= (hash/sha256-canonical-json (dissoc changed "interpretation_facts")) (get recorded "render-content")))
        (is (= (trace/derivation-key render recorded) (get-in second-run [:trace-keys :render])))
        (doseq [[version value] [["body" (assoc-in changed ["nodes" 0 "text"] "別文")]
                                 ["source" (assoc-in changed ["source" "work_content_hash"] (str "sha256:" (apply str (repeat 64 "b"))))]
                                 ["warning" (assoc changed "warnings" [{"severity" "warning" "code" "synthetic" "message" "記録"}])]]]
          (let [result (run version value)]
            (is (false? (get-in result [:cached :render])))
            (is (not= (get-in second-run [:trace-keys :render]) (get-in result [:trace-keys :render])))
            (is (not= (get-in second-run [:outputs :render]) (get-in result [:outputs :render])))))
        (is (= 4 @calls))
        (is (empty? (trace/determinism-violations (:trace store)))))
      (finally (engine/close-store! store) (fs/delete-tree dir)))))
