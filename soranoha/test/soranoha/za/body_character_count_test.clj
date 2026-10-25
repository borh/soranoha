(ns soranoha.za.body-character-count-test
  (:require [babashka.fs :as fs]
            [clojure.string :as string]
            [clojure.test :refer [deftest is]]
            [soranoha.core.json :as json]
            [soranoha.kura.cas :as cas]
            [soranoha.ori.stages :as stages]
            [soranoha.snh.decode :as decode]
            [soranoha.za.bundle :as bundle]
            [soranoha.za.catalog :as catalog]
            [soranoha.za.citation :as citation]))

(defn- utf8 [^String text] (.getBytes text "UTF-8"))

(defn- published-count [body]
  (let [dir (fs/create-temp-dir {:prefix "body-count"})
        slug "000001_000001"
        source-hash (apply str (repeat 64 "a"))
        metadata {"work" {"title" "試験" "orthographic_style" "新字新仮名"
                          "ndc" "NDC 913"
                          "card_url" "https://www.aozora.gr.jp/cards/000001/card1.html"
                          "source_editions" []}
                  "contributors" [{"person_id" "000001" "relation_to_work" "著者"}]}
        persons {"000001" {"family_name" "試験"}}
        put-json #(cas/put-bytes! (str dir) (utf8 (json/write-deterministic-json-str %)))]
    (try
      (let [tei (utf8 (str "<TEI xmlns='http://www.tei-c.org/ns/1.0'><teiHeader/>"
                           "<text><body>" body "</body></text></TEI>"))
            outputs ((:f (stages/plaintext-stage "fixture"))
                     {:blob (fn [_] tei)} {"tei" "fixture"})
            value (catalog/catalog-value
                   (str dir) [slug]
                   {slug {:metadata-record (put-json metadata)
                          :persons (put-json persons)
                          :primary-text-member "fixture.txt"
                          :plaintext-projection (cas/put-bytes! (str dir) (get outputs "plaintext-projection"))
                          :source-content-hash (str "sha256:" source-hash) :rights "public-domain"}})
            encoded (decode/encode "catalog" value)
            decoded (:value (decode/decode "catalog" (:bytes encoded)))
            entry (first (get decoded "works"))
            text (String. ^bytes (get outputs "plaintext") "UTF-8")
            body-count (get entry "body_character_count")]
        (is (= source-hash (get entry "source_content_hash")))
        (is (= "public-domain" (get entry "rights")))
        (is (= "試験" (get entry "title")))
        (doseq [{:keys [works artifact-type path]} (bundle/selections decoded)]
          (let [rows (string/split (bundle/catalog-csv {} works artifact-type) #"\r\n")
                column (.indexOf ^java.util.List citation/csv-columns "body_character_count")]
            (is (= (inc (count works)) (count rows)) path)
            (doseq [row (rest rows)]
              (is (= (str "\"" body-count "\"") (nth (string/split row #",") column)) path))))
        {:text text :count body-count})
      (finally (fs/delete-tree dir)))))

(deftest body-count-matches-current-plaintext-in-json-and-every-csv-selection
  (let [content (str "前𠮷か\u3099<ruby><rb>犍</rb><rt>かん</rt></ruby>"
                     "<choice><sic>誤字</sic><corr>正</corr></choice>"
                     "<note>注記</note><fw>柱</fw><g ref='#unknown'/>後")
        plain (published-count (str "<p>" content "</p>"))
        layout (published-count (str "<p rend='first-line-indent(2)'>\u00a0" content
                                     "<lb/><pb/><cb/>\u3000\t</p>"))
        changed (published-count (str "<p>" content "末</p>"))
        empty-body (published-count "<p>\u3000 <note>注記</note></p>")]
    (is (= "前𠮷か\u3099犍正\ufffc後" (:text plain)))
    (is (not= (:text plain) (:text layout)))
    (is (= 8 (:count plain) (:count layout)))
    (is (= 9 (:count changed)))
    (is (= 0 (:count empty-body)))))
