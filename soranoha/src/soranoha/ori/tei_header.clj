(ns soranoha.ori.tei-header
  "Pure metadata-record → TEI <teiHeader> hiccup builder + XML emitter.

  build returns nested-vector data; the XML emitters serialize that tree.
  The data shape is the API: any consumer can serialise the same
  hiccup tree without re-deriving the TEI mapping. No xml/alias-uri
  global side effect; the adapter wraps all unprefixed tag keywords
  in the TEI namespace and translates :xml/lang / :xml/id to the
  XML-namespaced attribute qnames clojure.data.xml expects.

  Hiccup contract (the protocol the data shape conveys):

    [:teiHeader
      [:fileDesc
        [:titleStmt
          [:title {:type \"main\" :xml/lang \"ja\"} \"...\"]
          [:title {:type \"reading\" :xml/lang \"ja-Hira\"} \"...\"]?
          [:title {:type \"sub\" :xml/lang \"ja\"} \"...\"]?
          [:title {:type \"sub-reading\" :xml/lang \"ja-Hira\"} \"...\"]?
          [:title {:type \"original\"} \"...\"]?
          [:author <persName>+ <idno>]
          [:respStmt [:resp \"...\"] <persName>+]?]
        [:publicationStmt [:publisher \"...\"] [:idno {:type \"...\"} \"...\"]+ [:date {:when \"...\"} \"...\"] <availability>?]
        [:sourceDesc [:bibl ...]+]]
      [:encodingDesc
        [:styleDefDecl {:scheme \"css\"}]
        [:classDecl [:taxonomy {:xml/id \"...\"} [:bibl ...] [:category {:xml/id \"...\" :n \"...\"} [:catDesc ...]+]*]+]
        [:charDecl [:char {:xml/id \"...\"} ...]?]
        [:editorialDecl
          [:correction {:method \"markup\"}
            [:p {:xml/id \"emendation-...\" :source \"#source-START-END\"}
              [:choice [:sic \"...\"] [:corr \"...\"]]]+]]?]
      [:profileDesc
        [:langUsage [:language {:ident \"ja\"} \"...\"]]
        [:textClass [:classCode {:scheme \"#...\"} \"...\"]+ [:catRef {:scheme \"#...\" :target \"#...\"}]*]]]

  Tag keywords with no namespace are TEI elements. :xml/lang and
  :xml/id route to the XML namespace; :snh/* names use the Soranoha
  namespace. Strings are text content."
  (:require [clojure.data.xml :as xml]
            [clojure.string :as string]
            [soranoha.core.ndc :as ndc]
            [soranoha.core.rights :as rights]))

(def ^:private tei-ns "http://www.tei-c.org/ns/1.0")
(def ^:private xml-ns "http://www.w3.org/XML/1998/namespace")

(defn- person-idno-type
  "Who issued this person identifier.

  A six-digit id is Aozora Bunko's 人物ID, taken from the catalog's 人物ID column.
  Anything else is minted locally, and the record schemas permit one such
  form. Publishing a locally minted id as `aozora-person-id` would claim a
  provenance Aozora Bunko did not grant, and provenance is what an idno type
  states."
  [person-id]
  (if (re-matches #"[0-9]{6}" (str person-id))
    "aozora-person-id"
    "soranoha-person-id"))

(defn- person-name-block
  "Kanji name with the person ID; reading and romaji names when supplied."
  [person]
  (let [pid (get person "person_id")
        pers-name (fn [lang surname forename & extras]
                    (cond-> [:persName {:xml/lang lang} [:surname surname]]
                      forename (conj [:forename forename])
                      true (into extras)))]
    (cond-> [(pers-name "ja"
                        (get person "family_name")
                        (get person "given_name")
                        [:idno {:type (person-idno-type pid)} pid])]
      (get person "family_name_reading")
      (conj (pers-name "ja-Hira"
                       (get person "family_name_reading")
                       (get person "given_name_reading")))
      (get person "family_name_romaji")
      (conj (pers-name "ja-Latn"
                       (get person "family_name_romaji")
                       (get person "given_name_romaji"))))))

(defn- author-block [person]
  (into [:author] (person-name-block person)))

(defn- resp-stmt
  "TEI <respStmt> with the role string and the person's name block."
  [role person]
  (into [:respStmt
         [:resp role]]
        (person-name-block person)))

(defn- title-stmt
  "Build <titleStmt>. `contributors` is a vector of
  {:relation-to-work <role-string> :person <person-body-map>}.

  Four title forms where the catalog carries them. The subtitle is needed for
  identification: author and title alone leave 2357 works in
  the Aozora Bunko catalog ambiguous, and adding 副題 cuts that to 882. The original
  title of a translated work carries no language attribute because the catalog
  records no source language, and guessing one from the string would be an
  assertion this project cannot support."
  [work contributors]
  (let [authors (filter #(= "著者" (:relation-to-work %)) contributors)
        others (remove #(= "著者" (:relation-to-work %)) contributors)
        title (get work "title")
        title-r (get work "title_reading")
        subtitle (get work "subtitle")
        subtitle-r (get work "subtitle_reading")
        original (get work "original_title")]
    (-> [:titleStmt
         [:title {:type "main" :xml/lang "ja"} title]]
        (cond-> title-r
          (conj [:title {:type "reading" :xml/lang "ja-Hira"} title-r]))
        (cond-> subtitle
          (conj [:title {:type "sub" :xml/lang "ja"} subtitle]))
        (cond-> subtitle-r
          (conj [:title {:type "sub-reading" :xml/lang "ja-Hira"} subtitle-r]))
        (cond-> original
          (conj [:title {:type "original"} original]))
        (into (mapv #(author-block (:person %)) authors))
        (into (mapv #(resp-stmt (:relation-to-work %) (:person %)) others)))))

(defn- availability
  "TEI <availability> for the release's rights grant. Two layers, stated
  separately because they differ: the underlying work's standing, which
  Soranoha inherits and does not create, and the licence over Soranoha's own
  encoding. A TEI file is normally read detached from the site it came from,
  so the terms are carried inside it."
  [{:strs [works encoding statement_url]}]
  ;; Two <licence> elements rather than <licence> plus <p>: a <p> here would
  ;; be a paragraph of the document, and the reading view and its projections
  ;; select paragraphs, so header prose would surface as body text.
  [:availability {:status "free"}
   [:licence {:target (rights/works-uri works)}
    (rights/works-statement works)]
   [:licence {:target (rights/licence-uri encoding)}
    (rights/licence-statement encoding works)
    " Full rights statement: "
    [:ptr {:target statement_url}]]])

(defn- publication-stmt
  "Who published these bytes, under what identifier, and on what terms.

  Three identifiers, and the type values state who issued each. Soranoha issues
  the publication identifier, the `<work-id>_<card-directory>` form the site
  serves works under, so it is not labelled `aozora-*`: Aozora Bunko issues the
  work id and the card, not the pair. Without it a downloaded file cannot state
  what it is cited as, and the citation projections depend on that statement.

  The card URL is here rather than in the source description. It identifies Aozora Bunko's
  record for the work, which is what the other two identifiers beside it do;
  the source description is about the printed edition transcribed, which is a
  different thing and is often not the one the card links.

  The publisher is `Soranoha`, the entity, not `Soranoha Aozora Bunko TEI Corpus`,
  which is the corpus title and is already carried by `titleStmt`. `abc`
  remains in the w3id schema namespaces and the source-bundle construction
  id, where it identifies a schema rather than a publisher.

  `@ref` rather than a `pubPlace`: a downloaded file should resolve to its
  publisher without a lookup, and a corpus published only on the web has no
  place of publication that would not be invented.

  The `@ref` is the w3id prefix, not the serving host: these bytes are signed
  and the chain is append-only, so a hostname written here could never be
  corrected. `docs/w3id/htaccess` redirects it, as it does the vocabulary IRI
  and the rights statement beside it."
  [work slug rights-grant]
  (cond-> [:publicationStmt
           [:publisher {:ref "https://w3id.org/soranoha/"} "Soranoha"]]
    slug (conj [:idno {:type "soranoha-work-identifier"} slug])
    true (conj [:idno {:type "aozora-work-id"} (get work "work_id")])
    (get work "card_url") (conj [:idno {:type "aozora-card-url"} (get work "card_url")])
    true (conj [:date {:when (get work "aozora_modified")} (get work "aozora_modified")])
    rights-grant (conj (availability rights-grant))))

(defn- bibl-edition [edition]
  (cond-> [:bibl
           [:title (get edition "title")]
           [:publisher (get edition "publisher")]]
    (get edition "first_edition_year")
    (conj [:date (get edition "first_edition_year")])
    (get edition "input_edition")
    (conj [:note {:type "input-edition"} (get edition "input_edition")])
    (get edition "proof_edition")
    (conj [:note {:type "proof-edition"} (get edition "proof_edition")])))

(defn- fallback-source-bibl
  "What can be stated about the source when the catalog records no edition.
  The Aozora Bunko card URL is not part of this: it is published
  unconditionally in the publication statement, so every work links back to
  Aozora Bunko whether or not it has edition metadata."
  [work]
  (if-let [title (get work "title")]
    [:bibl [:title title]]
    [:bibl [:note "Aozora Bunko source edition metadata is not available."]]))

(defn- witness-list [declarations]
  (when (seq declarations)
    (into [:listWit]
          (map (fn [{:keys [xml-id label source]}]
                 [:witness (cond-> {:xml/id xml-id}
                             source (assoc :source source))
                  label]))
          declarations)))

(defn- source-desc
  "The editions this text was transcribed from, plus where it first appeared.

  初出 is a `bibl` rather than a `note` because `sourceDesc` does not admit
  notes, its content being bibliographic, and because a first-publication
  statement is a reference to another appearance of the work, which is what a
  `bibl` is for. It is distinguished from the transcribed editions by its
  type: it describes where the text was first printed, not what was keyed.
  A work that carries several such statements gets several `bibl`s."
  [work source-content-hash primary-text-hash witness-declarations]
  (let [editions (get work "source_editions")]
    (cond-> (if (seq editions)
              (into [:sourceDesc] (mapv bibl-edition editions))
              [:sourceDesc (fallback-source-bibl work)])
      ;; One `bibl` per line. A collection's 初出 states where each
      ;; constituent piece first appeared, and holding all of those in a
      ;; single element would state that they are one reference to one
      ;; appearance.
      (get work "first_published")
      (into (map (fn [statement] [:bibl {:type "first-publication"} statement]))
            (string/split-lines (get work "first_published")))
      source-content-hash (conj [:bibl [:idno {:type "source-content-hash"} source-content-hash]])
      primary-text-hash (conj [:bibl [:idno {:type "primary-text-hash"} primary-text-hash]])
      (seq witness-declarations) (conj (witness-list witness-declarations)))))

(defn- file-desc [work contributors slug source-content-hash primary-text-hash rights-grant
                  witness-declarations]
  [:fileDesc
   (title-stmt work contributors)
   (publication-stmt work slug rights-grant)
   (source-desc work source-content-hash primary-text-hash witness-declarations)])

(defn- mapping-type
  "`unicode` for one code point, `composed` for a sequence of them, the
  distinction TEI P5 chapter 5 draws between the two."
  [^String value]
  (if (> (.codePointCount value 0 (.length value)) 1) "composed" "unicode"))

(defn- declaration->char [declaration]
  (cond-> [:char {:xml/id (:xml-id declaration)}]
    (:unicode declaration)
    (conj [:mapping {:type (mapping-type (:unicode declaration))} (:unicode declaration)])

    (:ids declaration)
    (conj [:mapping {:type "IDS"} (:ids declaration)])

    (:raw-marker declaration)
    (conj [:localProp {:name "rawMarker"
                       :value (:raw-marker declaration)}])

    (:source-reference declaration)
    (conj [:localProp {:name "sourceReference"
                       :value (:source-reference declaration)}])

    (:unresolved-reason declaration)
    (conj [:localProp {:name "unresolvedReason"
                       :value (:unresolved-reason declaration)}])

    (seq (:reference-parts declaration))
    (into (map (fn [part]
                 [:snh/gaiji-reference (-> part
                                           (dissoc "source")
                                           (assoc "original" (get part "source")))])
               (:reference-parts declaration)))

    (:name declaration)
    (conj [:localProp {:name "charName"
                       :value (:name declaration)}])

    (:desc declaration)
    (conj [:desc (:desc declaration)])

    (:glyphwiki declaration)
    (conj [:localProp {:name "glyphwiki" :value (:glyphwiki declaration)}]
          [:figure
           [:graphic {:url (str "https://glyphwiki.org/glyph/" (:glyphwiki declaration) ".svg")}]
           [:figDesc (or (:ids declaration) (:glyphwiki declaration))]])))

(defn- char-decl [declarations]
  (when (seq declarations)
    (into [:charDecl]
          (map declaration->char declarations))))

(defn- ndc-codes
  "The work's NDC codes in catalog order, each as `[k? digits]`: `K913` is
  `[true \"913\"]`, the K being the mark for children's material that Aozora
  Bunko writes before the number."
  [work]
  (for [code (rest (string/split (or (get work "ndc") "") #"\s+"))]
    (let [k? (string/starts-with? code "K")]
      [k? (if k? (subs code 1) code)])))

(defn- ndc-category
  "The declared category for one three-digit code, or nil for a code not
  listed in the table, which then remains a bare `classCode`."
  [digits]
  (when-let [{:strs [ja en]} (ndc/labels digits)]
    (cond-> [:category {:xml/id (str "ndc" digits) :n digits}
             [:catDesc {:xml/lang "ja"} ja]]
      en (conj [:catDesc {:xml/lang "en"} en]))))

(defn- class-decl
  "The taxonomies the text classification points at, declared rather than
  given as a bare string, so a reader can tell who classified the work and
  what the code means.

  Aozora Bunko assigns both the NDC code and the orthographic style. Declaring the
  taxonomies is what carries that provenance to a reader, and it matters most
  for 文字遣い種別, which is populated for every work and partitions the corpus
  into 新字新仮名, 新字旧仮名, 旧字旧仮名, 旧字新仮名 and その他. A
  historical-kana study that mixes those without distinction is invalid, and the value
  varies within one series: 銭形平次捕物控 001 to 004 are 旧字旧仮名 while 005
  is 新字新仮名.

  The NDC taxonomy declares only the classes this work's codes refer to, and
  the children's-material taxonomy appears only for a K-prefixed code."
  [work]
  (let [codes (ndc-codes work)]
    (cond-> [:classDecl
             (into [:taxonomy {:xml/id "ndc"}
                    [:bibl "分類番号 as recorded by " [:title "青空文庫"]
                     ", read as a class of 日本十進分類法 新訂9版. Category labels are from "
                     [:title "NDC9 Linked Data"] " (日本図書館協会, CC BY); a category's IRI is "
                     [:ref {:target ndc/scheme-iri} ndc/scheme-iri]
                     " followed by # and its n."]]
                   (keep ndc-category (distinct (map second codes))))]
      (some first codes)
      (conj [:taxonomy {:xml/id "aozora-shelf"}
             [:bibl "分類番号 as recorded by " [:title "青空文庫"]
              ": the prefix K (児童図書記号) is placed before the NDC code for children's books."]
             [:category {:xml/id "shelf-k" :n "K"}
              [:catDesc {:xml/lang "ja"} "児童書"]
              [:catDesc {:xml/lang "en"} "Children's material"]]])
      true
      (conj [:taxonomy {:xml/id "aozora-orthography"}
             [:bibl [:title "青空文庫"]
              " 文字遣い種別: the orthographic style Aozora Bunko records for the "
              "transcription."]]))))

(def ^:private source-refs-decl
  "Where a `source-span` reference's two numbers are measured from.

  That is the one fact a reader cannot recover from the document itself: the
  extent is in the note's own id, the line is in its `n`, and the text is the
  one `sourceDesc` already identifies. What remains is the unit, and a pointer to
  the vocabulary for anything further. The convention is documented at the
  namespace IRI rather than restated in each of tens of thousands of files;
  a published file is the text and its evidence, not a manual."
  [:refsDecl {:xml/id "source-spans"}
   [:p "A note of type source-span has an xml:id of the form source-START-END, "
    "giving the extent in UTF-8 bytes of the decoded primary text identified in "
    "sourceDesc, after the emendations declared in editorialDecl/correction are "
    "applied; each emendation replaces its sic text by its corr text at its own "
    "extent. "
    [:ptr {:target "https://w3id.org/soranoha/ns/tei"}]]])

(defn- editorial-decl
  "One `p` per parser-IR emendation, in parser-IR order. `sic` quotes the
  transcribed source text and `corr` the text the parser read; `@source`
  refers to the `source-span` note of the emended extent."
  [emendations]
  (when (seq emendations)
    [:editorialDecl
     (into [:correction {:method "markup"}]
           (map (fn [{:strs [id transcribed emended source_span]}]
                  [:p {:xml/id (str "emendation-" id)
                       :source (str "#source-" (get source_span "start") "-" (get source_span "end"))}
                   [:choice [:sic transcribed] [:corr emended]]]))
           emendations)]))

(defn- encoding-desc [work declarations emendations]
  (cond-> [:encodingDesc [:styleDefDecl {:scheme "css"}] (class-decl work) source-refs-decl]
    (seq declarations) (conj (char-decl declarations))
    (seq emendations) (conj (editorial-decl emendations))))

(defn- text-class
  "NDC code and orthographic style, each referencing its declared taxonomy.
  The code is retained as recorded in the catalog, with `catRef` elements
  referencing declared classes."
  [work]
  ;; `ndc` is already nil unless it matched the catalog's documented form.
  ;; `orthographic_style` is not run through the same nullable guard upstream,
  ;; so a row that carries the column empty would otherwise publish an empty
  ;; classification, which asserts a classification and supplies none.
  (let [ndc (get work "ndc")
        codes (ndc-codes work)
        classes (->> (distinct (map second codes))
                     (filter ndc/labels)
                     (map #(str "#ndc" %)))
        orthography (not-empty (some-> (get work "orthographic_style") string/trim))]
    (when (or ndc orthography)
      (cond-> [:textClass]
        ndc (conj [:classCode {:scheme "#ndc"} (string/replace ndc #"^NDC " "")])
        (seq classes) (conj [:catRef {:scheme "#ndc" :target (string/join " " classes)}])
        (some first codes) (conj [:catRef {:scheme "#aozora-shelf" :target "#shelf-k"}])
        orthography (conj [:classCode {:scheme "#aozora-orthography"} orthography])))))

(defn- profile-desc [work]
  (let [classification (text-class work)]
    (cond-> [:profileDesc
             [:langUsage [:language {:ident "ja"} "日本語"]]]
      classification
      (conj classification))))

(defn build
  "Return a TEI <teiHeader> as hiccup-style nested vectors. Pure;
  no clojure.data.xml coupling at this boundary.

  Input shape:
    {:work         <work map, string keys>
     :contributors [{:relation-to-work \"...\" :person <person map, string keys>} ...]
     :slug          <the publication identifier, when the caller has one>
     :rights        <the release rights grant, manifest key spelling>
     :emendations   <the parser-IR `emendations` array, when present>}

  Role and person are kept separate at every level inside this builder;
  the relation_to_work value never enters the person body.

  :rights is optional here so a header can be built for inspection without a
  policy in hand; the render stage supplies it for every published work and
  the profile's snh-publication-licence rule rejects a published file without
  it."
  [{:keys [work contributors slug char-declarations witness-declarations source-content-hash
           primary-text-hash rights emendations]}]
  [:teiHeader
   (file-desc work contributors slug source-content-hash primary-text-hash rights
              witness-declarations)
   (encoding-desc work char-declarations emendations)
   (profile-desc work)])

(defn- tei-qname [tag]
  (xml/qname (if (= "snh" (namespace tag))
               "https://w3id.org/soranoha/ns/tei"
               tei-ns)
             (name tag)))

(defn- attr-key [k]
  (cond
    (and (keyword? k) (= "xml" (namespace k)))
    (xml/qname xml-ns (name k))

    (and (keyword? k) (= "snh" (namespace k)))
    (xml/qname "https://w3id.org/soranoha/ns/tei" (name k))

    (and (keyword? k) (= "xmlns" (namespace k)))
    (xml/qname "http://www.w3.org/2000/xmlns/" (name k))

    (keyword? k) (keyword (name k))

    :else k))

(defn- xml-text
  "Drop the characters XML 1.0 section 2.2 excludes from Char: the C0
  controls other than tab, line feed and carriage return, and U+FFFE and
  U+FFFF."
  [^String text]
  (string/replace text #"[\u0000-\u0008\u000B\u000C\u000E-\u001F\uFFFE\uFFFF]" ""))

(defn- attrs->xml [attrs]
  (into {}
        (for [[k v] attrs]
          [(attr-key k) (xml-text (str v))])))

(defn- ->xml-element
  "Convert hiccup vector to a clojure.data.xml element. Tags are TEI
  by default; :xml/lang and :xml/id route to the XML namespace."
  [v]
  (cond
    (vector? v)
    (let [[tag attrs-or-child & rest-children] v
          [attrs children] (if (and (map? attrs-or-child)
                                    (not (record? attrs-or-child)))
                             [attrs-or-child rest-children]
                             [{} (cons attrs-or-child rest-children)])]
      (apply xml/element
             (tei-qname tag)
             (attrs->xml attrs)
             (keep ->xml-element children)))

    (nil? v) nil
    :else (xml-text (str v))))

(defn- with-default-tei-ns
  "Declare the TEI namespace as the document default on the root element.

  Without a declaration clojure.data.xml generates a prefix for the namespace
  and emits `<a:TEI xmlns:a=\"...\">`. That is namespace-correct and every
  namespace-aware processor reads it, but it is not the shape TEI corpora are
  published in, and researcher-written XPath (copied from teaching material,
  or run through tooling that ignores namespaces) is written against
  unprefixed element names. A prefixed serialisation returns empty node sets
  there rather than failing visibly."
  [element]
  (update element :attrs assoc :xmlns tei-ns))

(defn- emit-xml-string [element]
  (-> (xml/emit-str element)
      (string/replace "\r" "&#13;")
      (string/replace #"<(?:[^<>\"]++|\"[^\"]*+\")*+>"
                      (fn [tag]
                        (string/replace tag #"\"[^\"]*\""
                                        #(string/escape % {\newline "&#10;" \tab "&#9;"}))))))

(defn hiccup->xml-string
  "Serialise TEI hiccup to an XML string. The TEI namespace is the
  default; xml: prefix is bound to the XML namespace."
  [hiccup]
  (emit-xml-string (with-default-tei-ns (->xml-element hiccup))))

(def ^:private element-only-containers
  #{"TEI" "teiHeader" "fileDesc" "titleStmt" "publicationStmt" "availability" "sourceDesc"
    "encodingDesc" "profileDesc" "langUsage" "textClass" "keywords" "revisionDesc"
    "charDecl" "char" "editorialDecl" "correction" "biblStruct" "analytic" "monogr" "imprint"
    "text" "front" "body" "back" "div"})

(defn- indent-structural-elements [element depth]
  (let [children (:content element)]
    (if (and (contains? element-only-containers (name (:tag element)))
             (not= "preserve" (get (:attrs element) (xml/qname xml-ns "space")))
             (seq children)
             (every? #(and (map? %) (:tag %)) children))
      (let [padding #(str "\n" (apply str (repeat (* 2 %) " ")))]
        (assoc element :content
               (concat
                (mapcat (fn [child]
                          [(padding (inc depth))
                           (indent-structural-elements child (inc depth))])
                        children)
                [(padding depth)])))
      element)))

(defn hiccup->pretty-xml-string
  "Indent structural TEI containers while leaving mixed content and preserved
  whitespace unchanged. Unknown content models remain unformatted."
  [hiccup]
  (emit-xml-string (indent-structural-elements (with-default-tei-ns (->xml-element hiccup)) 0)))
