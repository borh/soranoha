(ns soranoha.ori.fidelity-probe-test
  (:require [babashka.fs :as fs]
            [charred.api :as json]
            [clojure.test :refer [deftest is]]
            [soranoha.ori.fidelity-probe :as probe]
            [soranoha.ori.fixture :as fixture]))

(deftest diff-entry-separates-tei-changes-from-plaintext-changes
  (let [dir (fs/create-temp-dir {:prefix "fidelity-probe-diff"})
        write-ir (fn [name nodes]
                   (let [path (str (fs/path dir (str name ".parser-ir.json")))]
                     (spit path (json/write-json-str {"nodes" nodes}))
                     path))
        entry {"id" "edition"
               "baseline_parser_ir" (write-ir "baseline" [{"type" "text" "text" "私は籠をさげ"}])
               "candidate_parser_ir" (write-ir "candidate"
                                               [{"type" "text" "text" "私は"}
                                                {"type" "ruby" "ruby" {"base" "籠" "reading" "ざる"}}
                                                {"type" "text" "text" "をさげ"}])
               "out_dir" (str dir)}]
    (try
      (let [result (#'probe/diff-entry "." @fixture/grant entry)]
        (is (= "edition" (get result "id")))
        (is (true? (get result "tei_changed")))
        (is (false? (get result "plaintext_changed")))
        (is (true? (get result "candidate_valid")) (pr-str result))
        (is (= "私は籠をさげ" (slurp (str (fs/path dir "candidate.txt"))))))
      (finally (fs/delete-tree dir)))))

(deftest diff-entry-counts-schematron-warnings-as-valid
  (let [dir (fs/create-temp-dir {:prefix "fidelity-probe-warning"})
        write-ir (fn [name nodes]
                   (let [path (str (fs/path dir (str name ".parser-ir.json")))]
                     (spit path (json/write-json-str {"nodes" nodes}))
                     path))
        entry {"id" "edition"
               "baseline_parser_ir" (write-ir "baseline" [{"type" "text" "text" "本文"}])
               "candidate_parser_ir" (write-ir "candidate" [{"type" "image" "src" "fig.png"}])
               "out_dir" (str dir)}]
    (try
      (let [result (#'probe/diff-entry "." @fixture/grant entry)]
        (is (= ["snh-figure-accessibility"]
               (mapv :rule-id (get result "schematron_findings"))))
        (is (true? (get result "candidate_valid"))))
      (finally (fs/delete-tree dir)))))

(defn- with-tei [xml f]
  (let [path (fs/create-temp-file {:prefix "fidelity-probe" :suffix ".xml"})]
    (try
      (spit (str path) xml)
      (f (str path))
      (finally (fs/delete path)))))

(deftest tei-projection-checks-namespaced-attributes-text-and-cardinality
  (with-tei
    (str "<TEI xmlns=\"http://www.tei-c.org/ns/1.0\" "
         "xmlns:snh=\"https://w3id.org/soranoha/ns/tei\"><text><body><p>"
         "<hi rend=\"accent\" snh:accent-code=\"1-09-63\">é</hi>"
         "</p></body></text></TEI>")
    (fn [path]
      (let [projection (fn [attributes]
                         {"assertions" {"elements" [{"selector" ".//tei:hi"
                                                     "count" 1
                                                     "text" "é"
                                                     "attributes" attributes
                                                     "qualifies" ["semantic-role" "value"]}]}})
            result (probe/evaluate-projection
                    (projection {"snh:accent-code" "1-09-63" "rend" "accent"}) path)]
        (is (true? (get result "assertions_match")))
        (is (= ["semantic-role" "value"] (get result "matching_aspects")))
        (is (false? (get (probe/evaluate-projection
                          (projection {"snh:accent-code" "1-09-63" "rend" "wrong"}) path)
                         "assertions_match")))
        (is (false? (get (probe/evaluate-projection
                          (projection {"snh:accent-code" "1-09-63" "rend" "accent" "type" "x"}) path)
                         "assertions_match"))
            "an absent attribute matches no expected value")))))

(deftest tei-projection-compares-exact-element-field-multisets
  (with-tei
    (str "<TEI xmlns=\"http://www.tei-c.org/ns/1.0\"><text><body><p>"
         "<hi rend=\"bouten right\">右</hi><hi rend=\"bouten left\">左</hi><hi>无</hi>"
         "</p></body></text></TEI>")
    (fn [path]
      (let [projection (fn [values]
                         {"assertions" {"element_multiset" [{"selector" ".//tei:hi"
                                                             "fields" ["text" "@rend"]
                                                             "values" values
                                                             "qualifies" ["value"]}]}})
            values [["左" "bouten left"] ["右" "bouten right"] ["无" nil]]
            result (probe/evaluate-projection (projection values) path)]
        (is (true? (get result "assertions_match")))
        (is (= ["value"] (get result "matching_aspects")))
        (is (= [["右" "bouten right"] ["左" "bouten left"] ["无" nil]]
               (get-in result ["assertions" 0 "actual"]))
            "rows are ordered by their Python JSON text, in code-point order")
        (is (false? (get (probe/evaluate-projection
                          (projection (conj values ["右" "bouten right"])) path)
                         "assertions_match")))))))
