(ns soranoha.tei-fidelity-test
  (:require [babashka.fs :as fs]
            [clojure.string :as string]
            [soranoha.ori.fixture :as fixture]
            [soranoha.ori.render :as render]
            [soranoha.ori.tei-header :as header]
            [soranoha.ori.relaxng :as rng]
            [soranoha.ori.schematron :as schematron]
            [clojure.test :refer [deftest is testing]]
            [soranoha.ori.tei :as tei]))

(defn- elements [tree tag]
  (filter #(and (vector? %) (= tag (first %)))
          (tree-seq coll? seq tree)))

(deftest paragraph-ranges-do-not-discard-intervening-structure
  (let [rendered (tei/render
                  {"nodes" [{"type" "heading" "level" 2 "text" "一"}
                            {"type" "text" "text" "本文"}
                            {"type" "heading" "level" 2 "text" "二"}
                            {"type" "text" "text" "続き"}
                            {"type" "heading" "level" 2 "text" "三"}]
                   "paragraphs" [{"id" "p1" "role" "body" "node_range" {"start" 1 "end" 2}}
                                 {"id" "p2" "role" "body" "node_range" {"start" 3 "end" 4}}]})]
    (is (= ["一" "二" "三"] (map last (elements (:body rendered) :head))))
    (is (= 3 (get-in rendered [:node_counts "heading"])))))

(deftest layout-indentation-is-not-lexical-whitespace
  (let [rendered (tei/render
                  {"nodes" [{"type" "text" "text" "　最初。"}
                            {"type" "text" "text" "　続く　本文。"}
                            {"type" "source-note" "placement" "back"
                             "note_type" "source-attribution"
                             "text" "底本：書名　巻数\n　　　刊行日\n"}]
                   "paragraphs" [{"id" "p1" "role" "body" "node_range" {"start" 0 "end" 2}}]})
        body (:body rendered)
        paragraph (first (elements body :p))
        note (first (elements body :note))]
    (is (= "text-indent: 1em" (:style (second paragraph))))
    (is (= ["最初。" "　続く　本文。"] (drop 2 paragraph)))
    (is (= [[:seg {:type "source-line"} "底本：書名　巻数"]
            [:seg {:type "source-line" :style "padding-inline-start: 3em"} "刊行日"]]
           (vec (elements note :seg))))
    (is (= 1 (count (elements note :lb))))))

(deftest indentation-follows-empty-text-and-apparatus
  (doseq [prefix [[{"type" "text" "text" ""}]
                  [{"type" "editor-note" "note" {"category" "correction" "raw" "注記"}}
                   {"type" "text" "text" ""}]]]
    (let [nodes (into prefix [{"type" "text" "text" "　本文　中。"}])
          body (:body (tei/render
                       {"nodes" nodes
                        "paragraphs" [{"id" "p1" "role" "body"
                                       "node_range" {"start" 0 "end" (count nodes)}}]}))
          paragraph (first (elements body :p))]
      (is (= "text-indent: 1em" (:style (second paragraph))))
      (is (= "本文　中。" (last paragraph)))
      (is (= (count (filter #(= "editor-note" (get % "type")) prefix))
             (count (elements body :note)))))))

(deftest visible-nodes-stop-paragraph-indentation-search
  (doseq [prefix [{"type" "text" "text" "前"}
                  {"type" "ruby" "ruby" {"base" "前" "reading" "まえ"}}
                  {"type" "emphasis" "style" "sesame-dot"
                   "inline_children" [{"type" "text" "text" "前"}]}]]
    (let [body (:body (tei/render
                       {"nodes" [prefix {"type" "text" "text" "　本文。"}]
                        "paragraphs" [{"id" "p1" "role" "body"
                                       "node_range" {"start" 0 "end" 2}}]}))
          paragraph (first (elements body :p))]
      (is (nil? (:style (second paragraph))))
      (is (= "　本文。" (last paragraph))))))

(deftest heading-indentation-is-preserved
  (let [body (:body (tei/render {"nodes" [{"type" "heading" "level" 2 "indent" 8 "text" "一"}]}))]
    (is (= {:n "2" :style "padding-inline-start: 8em"}
           (second (first (elements body :head)))))))

(deftest encoded-accents-retain-their-source-semantics
  (let [node {"type" "accent" "text" "é" "code" "1-09-63"
              "name" "アキュートアクセント付きE小文字" "source_spelling" "e'"}
        body (:body (tei/render {"nodes" [node]}))
        accent (first (elements body :hi))
        dir (fs/create-temp-dir {:prefix "accent-profile"})
        xml-path (str (fs/path dir "tei.xml"))]
    (is (= [:hi {:rend "accent"
                 :snh/accent-code "1-09-63"
                 :snh/accent-name "アキュートアクセント付きE小文字"
                 :snh/source-marker "e'"}
            "é"]
           accent))
    (try
      (spit xml-path
            (:tei (render/render-work
                   {:rights @fixture/grant
                    :parser-ir {"nodes" [node]}
                    :metadata-record {"work" {"title" "試験" "aozora_modified" "2026-09-20"}
                                      "contributors" []}
                    :persons-by-id {}})))
      (is (empty? (:violations (rng/validate! {:schema-path "schemas/tei-profile.rng"
                                               :xml-path xml-path :label "encoded accent"}))))
      (is (empty? (:findings (schematron/validate! {:schema-path "schemas/tei-profile.sch"
                                                    :xml-path xml-path :label "encoded accent"}))))
      (finally (fs/delete-tree dir)))))

(deftest gaiji-remains-inside-ruby-base
  (let [rendered (tei/render
                  {"nodes" [{"type" "ruby"
                             "ruby" {"base" "犍陀多" "reading" "かんだた"}
                             "inline_children" [{"type" "gaiji"
                                                 "gaiji" {"reference" "1-87-71" "unicode" "犍"}}
                                                {"type" "text" "text" "陀多"}]}]})
        rb (first (elements (:body rendered) :rb))]
    (is (= [:rb [:g {:ref "#gaiji-1-87-71"} "犍"] "陀多"] rb))
    (is (= "犍" (:unicode (first (:char_declarations rendered)))))))

(deftest gaiji-declarations-state-composed-sequences-and-external-identities
  (let [dir (fs/create-temp-dir {:prefix "gaiji-identity"})
        xml-path (str (fs/path dir "tei.xml"))
        voiced {"type" "gaiji"
                "gaiji" {"raw_marker" "濁点付き片仮名エ" "reference" "45-2"
                         "unicode" "エ\u3099" "resolved" true}}
        unencoded {"type" "gaiji"
                   "gaiji" {"raw_marker" "冫＋虫" "reference" "12-3" "unicode" nil "resolved" false
                            "unresolved_reason" "unresolved"
                            "ids" "⿰冫虫" "glyphwiki_glyph" "u2ff0-u51ab-u866b"}}
        community {"type" "gaiji"
                   "gaiji" {"raw_marker" "「纏」の「广」に代えて「厂」" "reference" "224-5"
                            "unicode" "纒" "resolved" true "mapping_source" "cjkvi-data"}}
        rendered (tei/render {"nodes" [voiced unencoded community]})
        chars (map #(drop 2 (#'header/declaration->char %)) (:char_declarations rendered))
        ^String xml (:tei (render/render-work
                           {:rights @fixture/grant
                            :parser-ir {"nodes" [voiced unencoded community]}
                            :metadata-record {"work" {"title" "試験" "aozora_modified" "2026-10-19"}
                                              "contributors" []}
                            :persons-by-id {}}))]
    (try
      (is (= [:mapping {:type "composed"} "エ\u3099"] (first (first chars))))
      (is (= #{[:mapping {:type "IDS"} "⿰冫虫"]
               [:localProp {:name "glyphwiki" :value "u2ff0-u51ab-u866b"}]
               [:figure
                [:graphic {:url "https://glyphwiki.org/glyph/u2ff0-u51ab-u866b.svg"}]
                [:figDesc "⿰冫虫"]]}
             (set (remove #(and (= :localProp (first %)) (not= "glyphwiki" (:name (second %))))
                          (second chars)))))
      (is (= [[:mapping {:type "unicode"} "纒"]
              [:localProp {:name "mappingSource"
                           :value "https://github.com/cjkvi/cjkvi-data/blob/e634e7da860d703e2e1f54dd2ac2d5aa113b7ac1/aozora_gaiji_chuki.txt"}]]
             (take 2 (nth chars 2))))
      (is (not (string/includes? xml "type=\"unicode\">エ")))
      (spit xml-path xml)
      (is (empty? (:violations (rng/validate! {:schema-path "schemas/tei-profile.rng"
                                               :xml-path xml-path :label "gaiji identities"}))))
      (is (empty? (:findings (schematron/validate! {:schema-path "schemas/tei-profile.sch"
                                                    :xml-path xml-path :label "gaiji identities"}))))
      (finally (fs/delete-tree dir)))))

(deftest gaiji-reference-schema-rejects-mixed-domains-and-invalid-values
  (let [dir (fs/create-temp-dir {:prefix "gaiji-reference-profile"})
        xml-path (str (fs/path dir "tei.xml"))
        jis {"kind" "jis-code" "plane" 1 "row" 13 "cell" 21}
        unicode {"kind" "unicode-code" "codepoint" 34894}
        location {"kind" "source-location" "page" "225" "part" "図のキャプション"}
        reading {"kind" "reading" "text" "ちょ"}
        uninterpreted {"kind" "uninterpreted"}
        render-part (fn [part]
                      (:tei (render/render-work
                             {:rights @fixture/grant
                              :parser-ir {"nodes" [{"type" "gaiji"
                                                    "gaiji" {"raw_marker" "※［＃外字］" "unicode" "衎"
                                                             "reference_parts" [(assoc part "source" "原文")]}}]}
                              :metadata-record {"work" {"title" "試験" "aozora_modified" "2026-09-06"}
                                                "contributors" []}
                              :persons-by-id {}})))]
    (try
      (doseq [[valid? parts]
              [[true [jis unicode location reading uninterpreted
                      (assoc jis "plane" 2 "row" 94 "cell" 94)
                      (assoc unicode "codepoint" 0)
                      (assoc unicode "codepoint" 55295)
                      (assoc unicode "codepoint" 57344)
                      (assoc unicode "codepoint" 1114111)
                      {"kind" "source-location" "page" "023" "line" "6" "volume" "1巻" "column" "下"}]]
               [false [(assoc jis "page" "13") (assoc jis "plane" 3)
                       (assoc jis "row" 95) (assoc jis "cell" 0) (dissoc jis "cell")
                       (assoc unicode "codepoint" 55296) (assoc unicode "codepoint" 57343)
                       (assoc unicode "codepoint" 1114112) (assoc unicode "codepoint" -1)
                       (assoc location "codepoint" 34894) (dissoc location "part")
                       (assoc location "page" "巻") (assoc location "part" "")
                       (assoc reading "text" "") (assoc uninterpreted "kind" "unknown")]]]
              part parts]
        (spit xml-path (render-part part))
        (is (= valid? (empty? (:violations (rng/validate! {:schema-path "schemas/tei-profile.rng"
                                                           :xml-path xml-path :label "gaiji reference"}))))
            (pr-str part)))
      (spit xml-path (string/replace (render-part jis) " original=\"原文\"" ""))
      (is (seq (:violations (rng/validate! {:schema-path "schemas/tei-profile.rng"
                                            :xml-path xml-path :label "missing original"}))))
      (doseq [version ["" " snh:vocab-version=\"1\""]]
        (spit xml-path (string/replace (render-part jis) " snh:vocab-version=\"3\"" version))
        (is (some #(= "snh-vocab-version-declared" (:rule-id %))
                  (:findings (schematron/validate! {:schema-path "schemas/tei-profile.sch"
                                                    :xml-path xml-path :label "gaiji vocabulary version"})))))
      (spit xml-path (-> (render-part jis)
                         (string/replace " snh:vocab-version=\"3\"" " snh:vocab-version=\"1\"")
                         (string/replace #"<snh:gaiji-reference[^>]*/>" "")))
      (is (empty? (:violations (rng/validate! {:schema-path "schemas/tei-profile.rng"
                                               :xml-path xml-path :label "retained vocabulary 1"}))))
      (is (empty? (:findings (schematron/validate! {:schema-path "schemas/tei-profile.sch"
                                                    :xml-path xml-path :label "retained vocabulary 1"}))))
      (finally (fs/delete-tree dir)))))

(deftest standalone-xml-emitter-binds-soranoha-element-and-attribute-namespaces
  (doseq [emit [header/hiccup->xml-string header/hiccup->pretty-xml-string]
          original ["字>\n\t\r&\"" (str (apply str (repeat 20000 "字")) "\n\t")]]
    (let [text "=\"first\nsecond\"\r"
          ^String xml (emit [:charDecl {:snh/vocab-version "2"}
                             [:char {:xml/id "x"}
                              [:mapping {:type "unicode"} text]
                              [:snh/gaiji-reference {:kind "uninterpreted" :original original}]]])
          factory (doto (javax.xml.parsers.DocumentBuilderFactory/newInstance) (.setNamespaceAware true))
          doc (.parse (.newDocumentBuilder factory) (java.io.ByteArrayInputStream. (.getBytes xml "UTF-8")))
          parts (.getElementsByTagNameNS doc "https://w3id.org/soranoha/ns/tei" "gaiji-reference")
          mappings (.getElementsByTagNameNS doc "http://www.tei-c.org/ns/1.0" "mapping")]
      (is (= 1 (.getLength parts)))
      (is (= "2" (.getAttributeNS (.getDocumentElement doc) "https://w3id.org/soranoha/ns/tei" "vocab-version")))
      (is (= original (.getAttribute ^org.w3c.dom.Element (.item parts 0) "original")))
      (is (= text (.getTextContent (.item mappings 0)))))))

(deftest xml-emitter-drops-the-characters-xml-cannot-carry
  (doseq [emit [header/hiccup->xml-string header/hiccup->pretty-xml-string]]
    (let [^String xml (emit [:charDecl {:snh/vocab-version "2"}
                             [:char {:xml/id "x"}
                              [:mapping {:type "unicode"} "前\u000c\u0000後\n"]
                              [:snh/gaiji-reference {:kind "uninterpreted" :original "甲\u000c乙"}]]])
          factory (doto (javax.xml.parsers.DocumentBuilderFactory/newInstance) (.setNamespaceAware true))
          doc (.parse (.newDocumentBuilder factory) (java.io.ByteArrayInputStream. (.getBytes xml "UTF-8")))
          parts (.getElementsByTagNameNS doc "https://w3id.org/soranoha/ns/tei" "gaiji-reference")
          mappings (.getElementsByTagNameNS doc "http://www.tei-c.org/ns/1.0" "mapping")]
      (is (= "前後\n" (.getTextContent (.item mappings 0))))
      (is (= "甲乙" (.getAttribute ^org.w3c.dom.Element (.item parts 0) "original"))))))

(deftest source-layout-validates-against-the-publication-profile
  (let [dir (fs/create-temp-dir {:prefix "tei-layout-profile"})
        xml-path (str (fs/path dir "tei.xml"))
        result (render/render-work
                {:rights @fixture/grant
                 :parser-ir {"nodes" [{"type" "heading" "level" 2 "indent" 8 "text" "一"}
                                      {"type" "text" "text" "　本文。"}
                                      {"type" "source-note" "placement" "back"
                                       "note_type" "source-attribution" "text" "底本：書名\n　　　刊行日\n"}]
                             "paragraphs" [{"id" "p1" "role" "body" "node_range" {"start" 1 "end" 2}}]}
                 :metadata-record {"work" {"title" "試験" "aozora_modified" "2026-09-06"} "contributors" []}
                 :persons-by-id {}})]
    (try
      (spit xml-path (:tei result))
      (is (empty? (:violations (rng/validate! {:schema-path "schemas/tei-profile.rng"
                                               :xml-path xml-path :label "source layout"}))))
      (finally (fs/delete-tree dir)))))

(def ^:private fully-described-work
  "A work carrying every catalog field the header can express. Measured over
  the Aozora catalog, these are what make a work identifiable: author plus
  title leaves 2357 works ambiguous, and 副題, 文字遣い種別, 底本名 and 初出
  narrow that to 16."
  {"work_id" "000092"
   "title" "蜘蛛の糸"
   "title_reading" "くものいと"
   "subtitle" "副題"
   "subtitle_reading" "ふくだい"
   "original_title" "The Spider's Thread"
   "first_published" "「赤い鳥」1918(大正7)年7月"
   "ndc" "NDC 913"
   "orthographic_style" "新字新仮名"
   "aozora_modified" "2026-09-06"
   "card_url" "https://www.aozora.gr.jp/cards/000879/card92.html"
   "source_editions" [{"title" "芥川龍之介全集　第三巻"
                       "publisher" "筑摩書房"
                       "first_edition_year" "1971"}]})

(defn- render-fully-described []
  (render/render-work
   {:rights @fixture/grant
    :slug "000092_000879"
    :parser-ir {"nodes" [{"type" "text" "text" "　本文。"}]
                "paragraphs" [{"id" "p1" "role" "body" "node_range" {"start" 0 "end" 1}}]}
    :metadata-record {"work" fully-described-work "contributors" []}
    :persons-by-id {}}))

(defn- render-work-fields
  "The fully described work with `overrides` applied, rendered."
  [overrides]
  (render/render-work
   {:rights @fixture/grant
    :slug "000092_000879"
    :parser-ir {"nodes" [{"type" "text" "text" "　本文。"}]
                "paragraphs" [{"id" "p1" "role" "body" "node_range" {"start" 0 "end" 1}}]}
    :metadata-record {"work" (merge fully-described-work overrides) "contributors" []}
    :persons-by-id {}}))

(defn- render-with-contributors [contributors persons]
  (render/render-work
   {:rights @fixture/grant
    :slug "000092_000879"
    :parser-ir {"nodes" [{"type" "text" "text" "　本文。"}]
                "paragraphs" [{"id" "p1" "role" "body" "node_range" {"start" 0 "end" 1}}]}
    :metadata-record {"work" fully-described-work "contributors" contributors}
    :persons-by-id persons}))

(deftest a-person-idno-identifies-the-issuer-of-the-identifier
  ;; The record schemas admit a locally minted person id beside Aozora's
  ;; six-digit 人物ID. Publishing both as `aozora-person-id` would claim a
  ;; provenance Aozora did not grant.
  (let [tei (:tei (render-with-contributors
                   [{"person_id" "000879" "relation_to_work" "著者"}
                    {"person_id" "abc-000000000001" "relation_to_work" "校訂者"}]
                   {"000879" {"person_id" "000879"
                              "family_name" "芥川" "given_name" "龍之介"}
                    "abc-000000000001" {"person_id" "abc-000000000001"
                                        "family_name" "校" "given_name" "訂"}}))]
    (testing "a six-digit id is Aozora's, and is typed as such"
      (is (string/includes? tei "<idno type=\"aozora-person-id\">000879</idno>")))

    (testing "a locally minted id is not attributed to Aozora"
      (is (string/includes?
           tei "<idno type=\"soranoha-person-id\">abc-000000000001</idno>"))
      (is (not (string/includes? tei "\"aozora-person-id\">abc-"))))))

(deftest a-header-with-both-kinds-of-person-validates
  (let [dir (fs/create-temp-dir {:prefix "tei-person-idno"})
        xml-path (str (fs/path dir "tei.xml"))]
    (try
      (spit xml-path (:tei (render-with-contributors
                            [{"person_id" "abc-000000000001" "relation_to_work" "著者"}]
                            {"abc-000000000001" {"person_id" "abc-000000000001"
                                                 "family_name" "校" "given_name" "訂"}})))
      (is (empty? (:violations (rng/validate! {:schema-path "schemas/tei-profile.rng"
                                               :xml-path xml-path
                                               :label "person idno"}))))
      (finally (fs/delete-tree dir)))))

(deftest the-header-carries-what-distinguishes-one-work-from-another
  (let [tei (:tei (render-fully-described))
        has? (fn [fragment] (string/includes? tei fragment))]
    (testing "the titles that separate same-titled works"
      (is (has? "<title type=\"main\" xml:lang=\"ja\">蜘蛛の糸</title>"))
      (is (has? "<title type=\"sub\" xml:lang=\"ja\">副題</title>"))
      (is (has? "<title type=\"sub-reading\" xml:lang=\"ja-Hira\">ふくだい</title>"))
      (is (has? "<title type=\"original\">The Spider's Thread</title>")
          "no xml:lang: the catalog records no source language to assert"))

    (testing "the identifier a downloaded file has to be citable by"
      (is (has? "<idno type=\"soranoha-work-identifier\">000092_000879</idno>"))
      (is (has? "<idno type=\"aozora-work-id\">000092</idno>"))
      (is (not (string/includes? tei "aozora-work-identifier"))
          "Aozora issues the work id and the card, not the pair"))

    (testing "the Aozora card, on every work rather than only the fallback path"
      (is (has? "<idno type=\"aozora-card-url\">https://www.aozora.gr.jp/cards/000879/card92.html</idno>")))

    (testing "where the text first appeared, kept apart from what was keyed"
      (is (has? "<bibl type=\"first-publication\">「赤い鳥」1918(大正7)年7月</bibl>")))

    (testing "a collection states one appearance per constituent piece, so it
              gets one bibl per statement rather than one holding them all"
      (let [collection (:tei (render-work-fields
                              {"first_published" "甲「新潮」1918年\n乙「改造」1922年"}))]
        (is (string/includes? collection "<bibl type=\"first-publication\">甲「新潮」1918年</bibl>"))
        (is (string/includes? collection "<bibl type=\"first-publication\">乙「改造」1922年</bibl>"))
        (is (= 2 (count (re-seq #"type=\"first-publication\"" collection))))))

    (testing "both classifications point at a declared taxonomy"
      (is (has? "<taxonomy xml:id=\"aozora-orthography\">"))
      (is (has? "<classCode scheme=\"#ndc\">913</classCode>"))
      (is (has? "<classCode scheme=\"#aozora-orthography\">新字新仮名</classCode>")))

    (testing "the NDC code is readable: the class it refers to is declared with its labels"
      (is (has? "<category xml:id=\"ndc913\" n=\"913\"><catDesc xml:lang=\"ja\">小説．物語</catDesc><catDesc xml:lang=\"en\">Fiction.Romance.Novel</catDesc></category>"))
      (is (has? "<catRef scheme=\"#ndc\" target=\"#ndc913\"/>"))
      (is (not (has? "aozora-shelf")) "no K, so no children's-material taxonomy"))))

(deftest the-ndc-code-is-kept-verbatim-and-read-into-declared-classes
  (testing "a K before the number is the children's-material mark, declared apart from the class"
    (let [tei (:tei (render-work-fields {"ndc" "NDC K913"}))]
      (is (string/includes? tei "<classCode scheme=\"#ndc\">K913</classCode>"))
      (is (string/includes? tei "<catRef scheme=\"#ndc\" target=\"#ndc913\"/>"))
      (is (string/includes? tei "<catRef scheme=\"#aozora-shelf\" target=\"#shelf-k\"/>"))
      (is (string/includes? tei "<category xml:id=\"shelf-k\" n=\"K\"><catDesc xml:lang=\"ja\">児童書</catDesc>"))))
  (testing "several codes declare several classes and one catRef names them all"
    (let [tei (:tei (render-work-fields {"ndc" "NDC 913 914"}))]
      (is (string/includes? tei "<category xml:id=\"ndc913\""))
      (is (string/includes? tei "<category xml:id=\"ndc914\""))
      (is (string/includes? tei "<catRef scheme=\"#ndc\" target=\"#ndc913 #ndc914\"/>"))))
  (testing "a code the NDC9 table does not have stays a bare classCode"
    (let [tei (:tei (render-work-fields {"ndc" "NDC 998"}))]
      (is (string/includes? tei "<classCode scheme=\"#ndc\">998</classCode>"))
      (is (not (string/includes? tei "<catRef scheme=\"#ndc\"")))
      (is (not (string/includes? tei "<category"))))))

(deftest a-fully-described-header-validates-against-the-publication-profile
  (let [dir (fs/create-temp-dir {:prefix "tei-header-profile"})
        xml-path (str (fs/path dir "tei.xml"))]
    (try
      (spit xml-path (:tei (render-fully-described)))
      (is (empty? (:violations (rng/validate! {:schema-path "schemas/tei-profile.rng"
                                               :xml-path xml-path :label "full header"}))))
      (is (empty? (remove #(= :warning (:severity %))
                          (:findings (schematron/validate!
                                      {:schema-path "schemas/tei-profile.sch"
                                       :xml-path xml-path :label "full header"})))))
      (finally (fs/delete-tree dir)))))

(deftest a-header-with-several-first-publications-validates
  ;; 156 published works state more than one, the largest 414, so the
  ;; repetition has to be something the profile admits rather than
  ;; something that happens to work on the two-element case.
  (let [dir (fs/create-temp-dir {:prefix "tei-header-first-publications"})
        xml-path (str (fs/path dir "tei.xml"))
        statements (string/join "\n" (map #(str "作品" % "「新潮」1918年") (range 414)))]
    (try
      (spit xml-path (:tei (render-work-fields {"first_published" statements})))
      (is (empty? (:violations (rng/validate! {:schema-path "schemas/tei-profile.rng"
                                               :xml-path xml-path
                                               :label "many first publications"}))))
      (is (empty? (remove #(= :warning (:severity %))
                          (:findings (schematron/validate!
                                      {:schema-path "schemas/tei-profile.sch"
                                       :xml-path xml-path
                                       :label "many first publications"})))))
      (finally (fs/delete-tree dir)))))

(deftest a-header-with-only-subordinate-titles-is-refused
  ;; the main-title rule has to exclude every subordinate form, or a file
  ;; carrying only a reading or only a subtitle satisfies it
  (let [dir (fs/create-temp-dir {:prefix "tei-header-subordinate"})
        xml-path (str (fs/path dir "tei.xml"))
        tei (string/replace (:tei (render-fully-described))
                            #"<title type=\"main\"[^>]*>[^<]*</title>" "")]
    (try
      (spit xml-path tei)
      (is (= ["snh-tei-header-title"]
             (mapv :rule-id
                   (remove #(= :warning (:severity %))
                           (:findings (schematron/validate!
                                       {:schema-path "schemas/tei-profile.sch"
                                        :xml-path xml-path :label "no main title"}))))))
      (finally (fs/delete-tree dir)))))

(def ^:private emendation
  {"id" "pariyori-403-ruby-bar-after-base"
   "transcribed" "一番向｜《むか》うにある"
   "emended" "一番｜向《むか》うにある"
   "source_span" {"start" 12 "end" 48 "line" 403 "end_line" 403
                  "coordinate_system" "decoded_utf8"}})

(defn- render-with-emendations [parser-ir]
  (:tei (render/render-work
         {:rights @fixture/grant
          :slug "000092_000879"
          :parser-ir (merge {"nodes" [{"type" "text" "text" "　本文。"}]
                             "paragraphs" [{"id" "p1" "role" "body" "node_range" {"start" 0 "end" 1}}]}
                            parser-ir)
          :metadata-record {"work" fully-described-work "contributors" []}
          :persons-by-id {}})))

(deftest an-emendation-is-declared-in-the-header-with-a-resolving-source-span
  (let [tei (render-with-emendations {"emendations" [emendation]})
        dir (fs/create-temp-dir {:prefix "tei-header-emendation"})
        xml-path (str (fs/path dir "tei.xml"))]
    (testing "the declaration quotes the transcribed and the emended text"
      (is (string/includes? tei "<correction method=\"markup\">"))
      (is (string/includes?
           tei (str "<p xml:id=\"emendation-pariyori-403-ruby-bar-after-base\" source=\"#source-12-48\">"
                    "<choice><sic>一番向｜《むか》うにある</sic><corr>一番｜向《むか》うにある</corr></choice></p>"))))
    (testing "the source pointer resolves to a source-span note carrying the line"
      (is (string/includes? tei "<note type=\"source-span\" xml:id=\"source-12-48\" n=\"403\"/>")))
    (testing "the refsDecl states that offsets are measured after emendation"
      (is (string/includes? tei "after the emendations declared in editorialDecl/correction")))
    (try
      (spit xml-path tei)
      (is (empty? (:violations (rng/validate! {:schema-path "schemas/tei-profile.rng"
                                               :xml-path xml-path :label "emendation"}))))
      (is (empty? (remove #(= :warning (:severity %))
                          (:findings (schematron/validate!
                                      {:schema-path "schemas/tei-profile.sch"
                                       :xml-path xml-path :label "emendation"})))))
      (finally (fs/delete-tree dir)))))

(deftest a-work-without-emendations-has-no-editorial-declaration
  (is (not (string/includes? (render-with-emendations {}) "<editorialDecl"))))

(deftest an-emendation-across-source-lines-is-refused
  (is (thrown-with-msg? clojure.lang.ExceptionInfo #"within one source line"
                        (render-with-emendations
                         {"emendations" [(assoc-in emendation ["source_span" "end_line"] 404)]}))))

(deftest formatting-instructions-preserve-ordered-operands-and-source-references
  (let [span (fn [start end] {"start" start "end" end "coordinate_system" "decoded_utf8"})
        operand {"text" "x2" "source_span" (span 12 14)}
        node {"type" "formatting-instruction" "raw" "［＃原文］"
              "source_span" (span 2 50)
              "operations" [{"source_span" (span 8 30)
                             "selector" {"kind" "each-quoted" "operands" [operand operand]}
                             "effect" {"kind" "attribute" "layout" {"kind" "tcy" "source" "aat-inline"}}}
                            {"source_span" (span 31 48)
                             "selector" {"kind" "character-position" "ordinal" 2 "operands" [operand]}
                             "effect" {"kind" "upper-relative"
                                       "base_target" (span 0 1)
                                       "base" {"text" "e" "source_span" (span 38 39)}}}]}
        rendered (tei/render {"nodes" [{"type" "text" "text" "本文"} node]})
        body (:body rendered)
        selectors (elements body :snh/formatting-selector)
        operands (elements body :snh/formatting-operand)
        operations (elements body :snh/formatting-operation)
        notes (elements body :note)]
    (is (= ["each-quoted" "character-position"] (map #(get (second %) :kind) selectors)))
    (is (= 2 (:ordinal (second (second selectors)))))
    (is (= ["x2" "x2" "x2" "e"] (map last operands)))
    (is (= ["#source-12-14" "#source-12-14" "#source-12-14" "#source-38-39"]
           (map #(get (second %) :source) operands)))
    (is (= ["#source-8-30" "#source-31-48"] (map #(get (second %) :source) operations)))
    (is (= "#source-0-1" (:source (second (first (elements body :snh/formatting-base))))))
    (is (= "#source-2-50" (:source (second (first notes)))))))

(deftest formatting-instructions-preserve-subtargets-and-spatial-anchors
  (let [span (fn [start end] {"start" start "end" end "coordinate_system" "decoded_utf8"})
        operand (fn [text start end] {"text" text "source_span" (span start end)})
        node {"type" "formatting-instruction" "raw" "［＃原文］" "source_span" (span 2 90)
              "operations" [{"source_span" (span 8 30)
                             "selector" {"kind" "subtarget"
                                         "enclosing" (operand "一ノ戸" 10 19)
                                         "operands" [(operand "ノ" 24 27)]}
                             "effect" {"kind" "attribute"
                                       "layout" {"kind" "font-size" "size_type" "absolute"
                                                 "size" "small" "source" "aat-inline"}}}
                            {"source_span" (span 31 88)
                             "selector" {"kind" "quoted" "operands" [(operand "注記" 33 39)]}
                             "effect" {"kind" "reduced-point-lines" "lines" 2
                                       "parentheses_lines" 2
                                       "placement" {"kind" "between"
                                                    "preceding" (operand "前" 45 48)
                                                    "following" (operand "後" 52 55)}}}]}
        body (:body (tei/render {"nodes" [node]}))
        selectors (elements body :snh/formatting-selector)
        effects (elements body :snh/formatting-effect)
        operands (elements body :snh/formatting-operand)]
    (is (= ["subtarget" "quoted"] (map #(get (second %) :kind) selectors)))
    (is (= ["enclosing" "selected" nil "preceding" "following"]
           (map #(get (second %) :role) operands)))
    (is (= {:kind "reduced-point-lines" :placement "between" :lines 2 :parentheses-lines 2}
           (second (second effects))))))

(deftest formatting-instructions-preserve-day-range-placement-anchors
  (let [span (fn [start end] {"start" start "end" end "coordinate_system" "decoded_utf8"})
        endpoint (fn [day text operand-start operand-end target-start target-end]
                   {"day" day
                    "operand" {"text" text "source_span" (span operand-start operand-end)}
                    "target" (span target-start target-end)})
        node {"type" "formatting-instruction" "raw" "［＃原文］" "source_span" (span 2 90)
              "operations" [{"source_span" (span 31 88)
                             "selector" {"kind" "quoted"
                                         "operands" [{"text" "注記" "source_span" (span 33 39)}]}
                             "effect" {"kind" "relative-placement"
                                       "relation" "below"
                                       "anchor" {"kind" "day-range"
                                                 "start" (endpoint 2 "２" 45 48 5 8)
                                                 "end" (endpoint 10 "10" 52 54 20 22)}}}]}
        body (:body (tei/render {"nodes" [node]}))
        effect (first (elements body :snh/formatting-effect))
        operands (elements effect :snh/formatting-operand)
        anchors (elements effect :snh/formatting-anchor)]
    (is (= {:kind "relative-placement" :relation "below" :anchor-kind "day-range"}
           (second effect)))
    (is (= [["range-start" "２" "#source-45-48"]
            ["range-end" "10" "#source-52-54"]]
           (mapv (fn [operand]
                   [(get (second operand) :role) (last operand) (get (second operand) :source)])
                 operands)))
    (is (= [["range-start" 2 "#source-5-8"]
            ["range-end" 10 "#source-20-22"]]
           (mapv (fn [anchor]
                   [(get (second anchor) :role) (get (second anchor) :day)
                    (get (second anchor) :source)])
                 anchors)))))

(deftest formatting-instructions-preserve-following-line-rule-style
  (let [span (fn [start end] {"start" start "end" end "coordinate_system" "decoded_utf8"})
        selection {"scope" (span 73 246)
                   "targets" [(span 73 130) (span 131 188) (span 189 246)]}
        selector {"kind" "following-lines" "lines" 3}
        node {"type" "formatting-instruction"
              "raw" "［＃次３行は、文字はゴシック体、罫線は全て波線］"
              "source_span" (span 0 72)
              "operations" [{"source_span" (span 6 45)
                             "selector" selector
                             "selection" selection
                             "effect" {"kind" "attribute"
                                       "layout" {"kind" "emphasis" "style" "gothic"
                                                 "source" "aat-inline"}}}
                            {"source_span" (span 48 69)
                             "selector" selector
                             "selection" selection
                             "effect" {"kind" "rule-style" "style" "wavy"}}]}
        body (:body (tei/render {"nodes" [node]}))
        selectors (elements body :snh/formatting-selector)
        rule-presentations (elements body :snh/rule-presentation)
        selections (elements body :snh/formatting-selection)]
    (is (= [{:kind "following-lines" :lines 3}
            {:kind "following-lines" :lines 3}]
           (mapv second selectors)))
    (is (= {:variant "wavy"} (second (first rule-presentations))))
    (is (= ["#source-73-130" "#source-131-188" "#source-189-246"]
           (mapv #(get (second %) :source)
                 (elements (first selections) :snh/formatting-target))))))

(deftest formatting-instruction-xml-enforces-selectors-and-vocabulary
  (let [dir (fs/create-temp-dir {:prefix "formatting-instruction-profile"})
        xml-path (str (fs/path dir "tei.xml"))
        span {"start" 0 "end" 20 "coordinate_system" "decoded_utf8"}
        operand {"text" "2" "source_span" span}
        instruction {"type" "formatting-instruction" "raw" "［＃原文］" "source_span" span
                     "operations" [{"source_span" span
                                    "selector" {"kind" "occurrence" "ordinal" 2 "operands" [operand]}
                                    "selection" {"scope" (assoc span "end" 2) "targets" [(assoc span "end" 1)]}
                                    "effect" {"kind" "attribute"
                                              "layout" {"kind" "emphasis" "style" "superscript" "source" "aat-inline"}}}]}
        xml (:tei (render/render-work
                   {:rights @fixture/grant
                    :parser-ir {"nodes" [{"type" "text" "text" "本文"} instruction]}
                    :metadata-record {"work" {"title" "試験" "aozora_modified" "2026-09-06"}
                                      "contributors" []}
                    :persons-by-id {}}))]
    (try
      (spit xml-path xml)
      (is (empty? (:violations (rng/validate! {:schema-path "schemas/tei-profile.rng"
                                               :xml-path xml-path :label "formatting instruction"}))))
      (is (empty? (:findings (schematron/validate! {:schema-path "schemas/tei-profile.sch"
                                                    :xml-path xml-path :label "formatting instruction"}))))
      (doseq [invalid [(string/replace xml #"<snh:formatting-target[^>]*/>" "")
                       (string/replace xml "ordinal=\"2\"" "ordinal=\"0\"")
                       (string/replace xml "kind=\"occurrence\"" "kind=\"digits\"")
                       (string/replace xml #"<snh:formatting-operand[^>]*>[^<]*</snh:formatting-operand>" "")]]
        (spit xml-path invalid)
        (is (seq (:violations (rng/validate! {:schema-path "schemas/tei-profile.rng"
                                              :xml-path xml-path :label "invalid formatting selector"})))))
      (doseq [version ["1" "2"]]
        (spit xml-path (string/replace xml "snh:vocab-version=\"3\"" (str "snh:vocab-version=\"" version "\"")))
        (is (some #(= "snh-vocab-version-declared" (:rule-id %))
                  (:findings (schematron/validate! {:schema-path "schemas/tei-profile.sch"
                                                    :xml-path xml-path :label "formatting vocabulary"})))))
      (finally (fs/delete-tree dir)))))

(deftest unresolved-formatting-selector-is-valid-source-apparatus
  (let [dir (fs/create-temp-dir {:prefix "unresolved-formatting-selector"})
        xml-path (str (fs/path dir "tei.xml"))
        span {"start" 0 "end" 20 "coordinate_system" "decoded_utf8"}
        xml (:tei (render/render-work
                   {:rights @fixture/grant
                    :parser-ir {"nodes" [{"type" "formatting-instruction"
                                          "raw" "［＃は地より７字上げ］"
                                          "source_span" span
                                          "operations" [{"source_span" span
                                                         "selector" {"kind" "unresolved"}
                                                         "effect" {"kind" "attribute"
                                                                   "layout" {"kind" "chitsuki"
                                                                             "align" "right"
                                                                             "offset_from_end" 7
                                                                             "source" "aat-inline"}}}]}]}
                    :metadata-record {"work" {"title" "試験" "aozora_modified" "2026-09-06"}
                                      "contributors" []}
                    :persons-by-id {}}))]
    (try
      (spit xml-path xml)
      (is (string/includes? xml "kind=\"unresolved\""))
      (is (empty? (:violations (rng/validate! {:schema-path "schemas/tei-profile.rng"
                                               :xml-path xml-path :label "unresolved formatting selector"}))))
      (finally (fs/delete-tree dir)))))

(deftest formatting-effect-schema-preserves-attribute-only-alternatives
  (let [dir (fs/create-temp-dir {:prefix "formatting-effect-profile"})
        xml-path (str (fs/path dir "tei.xml"))
        span {"start" 0 "end" 20 "coordinate_system" "decoded_utf8"}]
    (try
      (doseq [layout [{"kind" "baseline-position" "position" "lowered"}
                      {"kind" "small-script" "position" "left"}
                      {"kind" "font-size" "size_type" "absolute" "size" "large"}
                      {"kind" "font-size" "size_type" "qualitative" "direction" "smaller"}
                      {"kind" "font-size" "size_type" "large" "level" 2}
                      {"kind" "keigakomi" "border" "source-specific"}]]
        (let [xml (:tei (render/render-work
                         {:rights @fixture/grant
                          :parser-ir {"nodes" [{"type" "formatting-instruction" "raw" "［＃原文］"
                                                "source_span" span
                                                "operations" [{"source_span" span "selector" {"kind" "digits"}
                                                               "effect" {"kind" "attribute" "layout" (assoc layout "source" "aat-inline")}}]}]}
                          :metadata-record {"work" {"title" "試験" "aozora_modified" "2026-09-06"}
                                            "contributors" []}
                          :persons-by-id {}}))]
          (spit xml-path xml)
          (is (empty? (:violations (rng/validate! {:schema-path "schemas/tei-profile.rng"
                                                   :xml-path xml-path :label "formatting effect"})))
              (pr-str layout))
          (spit xml-path (string/replace xml #" layout-kind=\"[^\"]*\"" ""))
          (is (seq (:violations (rng/validate! {:schema-path "schemas/tei-profile.rng"
                                                :xml-path xml-path :label "missing layout kind"})))
              (pr-str layout))))
      (finally (fs/delete-tree dir)))))
