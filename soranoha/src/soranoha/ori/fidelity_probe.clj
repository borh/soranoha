(ns soranoha.ori.fidelity-probe
  (:require [babashka.fs :as fs]
            [charred.api :as json]
            [clojure.string :as str]
            [clojure.tools.cli :refer [parse-opts]]
            [soranoha.annotations.view :as view]
            [soranoha.core.json :as record-json]
            [soranoha.core.parallel :as parallel]
            [soranoha.core.rights :as rights]
            [soranoha.ori.projection :as projection]
            [soranoha.ori.relaxng :as rng]
            [soranoha.ori.render :as render]
            [soranoha.ori.schematron :as schematron])
  (:import (java.io File)
           (javax.xml XMLConstants)
           (javax.xml.namespace NamespaceContext)
           (javax.xml.parsers DocumentBuilderFactory)
           (javax.xml.xpath XPathConstants XPathFactory)
           (org.w3c.dom Element NodeList)))

(def ^:private options
  [[nil "--manifest PATH"]
   [nil "--diff-manifest PATH"]
   [nil "--assets-root PATH"]
   [nil "--clj-toolchain-id ID"]])

(defn- render-tei [grant parser-ir-path]
  (:tei (render/render-work
         {:rights grant
          :parser-ir (json/read-json (slurp parser-ir-path))
          :metadata-record {"work" {"title" "Fidelity fixture"
                                    "aozora_modified" "2026-09-20"}
                            "contributors" []}
          :persons-by-id {}})))

(defn- validation [assets xml-path]
  (let [violations (:violations
                    (rng/validate! {:schema-path (str (fs/path assets "schemas/tei-profile.rng"))
                                    :xml-path xml-path :label "fidelity fixture"}))
        findings (:findings
                  (schematron/validate!
                   {:schema-path (str (fs/path assets "schemas/tei-profile.sch"))
                    :xml-path xml-path :label "fidelity fixture"}))]
    {"valid" (and (empty? violations) (empty? findings))
     "relaxng_violations" violations
     "schematron_findings" findings}))

(def ^:private namespaces
  {"tei" "http://www.tei-c.org/ns/1.0"
   "snh" "https://w3id.org/soranoha/ns/tei"
   "xml" "http://www.w3.org/XML/1998/namespace"})

(def ^:private namespace-context
  (reify NamespaceContext
    (getNamespaceURI [_ prefix] (get namespaces prefix XMLConstants/NULL_NS_URI))
    (getPrefix [_ _] nil)
    (getPrefixes [_ _] nil)))

(defn- document-element ^Element [xml-path]
  (let [factory (doto (DocumentBuilderFactory/newInstance) (.setNamespaceAware true))]
    (.getDocumentElement (.parse (.newDocumentBuilder factory) (File. ^String xml-path)))))

(defn- select-elements [^Element root ^String selector]
  (let [xpath (doto (.newXPath (XPathFactory/newInstance)) (.setNamespaceContext namespace-context))
        ^NodeList nodes (.evaluate xpath selector root XPathConstants/NODESET)]
    (mapv #(.item nodes (int %)) (range (.getLength nodes)))))

(defn- attribute-value [^Element element ^String qualified-name]
  (let [[prefix local] (if (str/includes? qualified-name ":")
                         (str/split qualified-name #":" 2)
                         [nil qualified-name])
        uri (when prefix (get namespaces prefix))]
    (when (.hasAttributeNS element uri local)
      (.getAttributeNS element uri local))))

(defn- element-text [^Element element] (.getTextContent element))

(defn evaluate-projection
  "Evaluate a TEI projection's element and element-multiset assertions
  against the TEI at `xml-path`."
  [projection xml-path]
  (let [root (document-element xml-path)
        assertions (get projection "assertions")
        elements (for [expected (get assertions "elements")]
                   (let [selected (select-elements root (get expected "selector"))
                         matching (filter (fn [element]
                                            (and (or (not (contains? expected "text"))
                                                     (= (element-text element) (get expected "text")))
                                                 (every? (fn [[attribute value]]
                                                           (= (attribute-value element attribute) value))
                                                         (get expected "attributes"))))
                                          selected)]
                     {"expected" expected
                      "matching_count" (count matching)
                      "selected_count" (count selected)
                      "passed" (= (count matching) (get expected "count" 1))}))
        multisets (for [expected (get assertions "element_multiset")]
                    (let [selected (select-elements root (get expected "selector"))
                          field (fn [element field-name]
                                  (cond
                                    (= field-name "text") (element-text element)
                                    (str/starts-with? field-name "@") (attribute-value element (subs field-name 1))
                                    :else (throw (ex-info (str "unsupported XML multiset field: " field-name) {}))))
                          actual (sort (for [element selected]
                                         (mapv #(field element %) (get expected "fields"))))
                          wanted (sort (map vec (get expected "values")))]
                      {"expected" expected
                       "actual" (vec actual)
                       "selected_count" (count selected)
                       "passed" (= (vec actual) (vec wanted))}))
        results (vec (concat elements multisets))
        matched (boolean (and (seq results) (every? #(get % "passed") results)))]
    {"assertions_match" matched
     "oracle_status" (if matched "passed" "failed")
     "matching_aspects" (vec (sort (into #{} (comp (filter #(get % "passed"))
                                                   (mapcat #(get-in % ["expected" "qualifies"])))
                                         results)))
     "assertions" results}))

(defn- render-entry
  "Render and validate one fixture, and evaluate its TEI projection when the
  entry carries one and the TEI is valid."
  [assets grant entry]
  (let [xml-path (get entry "xml_out")
        _ (spit xml-path (render-tei grant (get entry "parser_ir")))
        result (assoc (validation assets xml-path) "id" (get entry "id"))]
    (cond-> result
      (and (get result "valid") (get entry "tei_projection"))
      (assoc "tei_evaluation" (evaluate-projection (get entry "tei_projection") xml-path)))))

(defn- render-side [grant out-dir entry side]
  (try
    (let [tei (render-tei grant (get entry (str side "_parser_ir")))
          plaintext (projection/plaintext (view/from-tei tei))
          xml-path (str (fs/path out-dir (str side ".tei.xml")))]
      (spit xml-path tei)
      (spit (str (fs/path out-dir (str side ".txt"))) plaintext)
      {:tei tei :plaintext plaintext :xml-path xml-path})
    (catch Exception e
      {:error (ex-message e)})))

(defn- diff-entry
  "Render both sides' parser-IR to TEI and plaintext, write them beside the
  inputs, and validate the candidate TEI, which is valid when it has no
  Relax NG violations and no Schematron errors, as in publication. A side
  that fails to render reports its error instead of a comparison."
  [assets grant entry]
  (let [out-dir (get entry "out_dir")
        baseline (render-side grant out-dir entry "baseline")
        candidate (render-side grant out-dir entry "candidate")
        compared? (not (or (:error baseline) (:error candidate)))
        candidate-validation (when-not (:error candidate)
                               (validation assets (:xml-path candidate)))]
    (cond-> {"id" (get entry "id")
             "candidate_valid" (boolean
                                (and candidate-validation
                                     (empty? (get candidate-validation "relaxng_violations"))
                                     (not-any? #(= :error (:severity %))
                                               (get candidate-validation "schematron_findings"))))}
      compared? (assoc "tei_changed" (not= (:tei baseline) (:tei candidate))
                       "plaintext_changed" (not= (:plaintext baseline) (:plaintext candidate)))
      candidate-validation (assoc "relaxng_violations" (get candidate-validation "relaxng_violations")
                                  "schematron_findings" (get candidate-validation "schematron_findings"))
      (:error baseline) (assoc "baseline_error" (:error baseline))
      (:error candidate) (assoc "candidate_error" (:error candidate)))))

(defn -main [& args]
  (let [{:keys [options errors]} (parse-opts args options)
        manifest-path (or (:manifest options) (:diff-manifest options))]
    (when (or (seq errors) (nil? (:assets-root options)) (nil? manifest-path)
              (and (:manifest options) (:diff-manifest options)))
      (throw (ex-info "fidelity probe requires assets and exactly one manifest path"
                      {:errors errors})))
    (let [assets (:assets-root options)
          manifest (json/read-json (slurp manifest-path))
          grant (rights/work-terms
                 (rights/grant-from-bytes
                  (fs/read-all-bytes (fs/path assets "data/publication-policy.edn")))
                 "public-domain")
          entry-fn (if (:diff-manifest options) diff-entry render-entry)
          results (parallel/ordered-pmap (.availableProcessors (Runtime/getRuntime))
                                         #(entry-fn assets grant %) manifest)]
      (println (record-json/write-deterministic-json-str {"results" results}))
      (when-not (:diff-manifest options)
        (when-not (every? #(get % "valid") results)
          (System/exit 1))))))
