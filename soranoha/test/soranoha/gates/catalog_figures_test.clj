(ns soranoha.gates.catalog-figures-test
  (:require [babashka.fs :as fs]
            [clojure.test :refer [deftest is testing]]
            [soranoha.gates.catalog-figures :as figures]))

(defn- row [work-id title & {:keys [person role romaji text style subtitle year1 year2]
                             :or {role "著者" romaji ["Akutagawa" "Ryunosuke"] text "https://www.aozora.gr.jp/cards/000879/files/1_ruby.zip"
                                  style "新字新仮名" subtitle "" year1 "1990（平成2）年3月5日" year2 ""}}]
  {"作品ID" work-id "作品名" title "副題" subtitle "文字遣い種別" style "底本名1" "b" "初出" "f"
   "人物ID" (or person "000879") "役割フラグ" role "姓ローマ字" (first romaji) "名ローマ字" (second romaji)
   "テキストファイルURL" text "底本初版発行年1" year1 "底本初版発行年2" year2})

(deftest the-catalog-figures-follow-their-definitions
  (let [works [[(row "1" "蜘蛛の糸")]
               [(row "2" "蜘蛛の糸" :style "旧字旧仮名")]
               [(row "3" "鼻" :person "000035" :romaji ["Dazai" ""] :text "" :year1 "1967（昭和42）年")]
               [(row "4" "同題" :person "000001" :romaji ["Mori" "Ogai"] :text "https://www.aozora.gr.jp/cards/000002/files/4.zip" :year1 "")
                (row "4" "同題" :person "000002" :romaji ["Natsume" "Soseki"] :role "翻訳者" :year1 "")]]
        measured (figures/measure works)]
    (is (= 5 (get measured ["catalog-rows" "rows"])))
    (is (= 4 (get measured ["catalog-works" "catalog"])))
    (is (= 4 (get measured ["catalog-persons" "catalog"])))
    (is (= 3 (get measured ["works-with-text" "catalog"])))
    (is (= 3 (get measured ["contributors" "with-text"])))
    (testing "two works by one author under one title are ambiguous, once"
      (is (= 2 (get measured ["author-title-works" "catalog"])))
      (is (= 1 (get measured ["author-title-pairs" "catalog"]))))
    (testing "a title split across orthographies counts its works"
      (is (= 1 (get measured ["orthography-split-titles" "with-text"])))
      (is (= 2 (get measured ["orthography-split-works" "with-text"])))
      (is (= 1 (get measured ["orthography-kyuji-kyukana" "with-text"]))))
    (testing "a work filed under another person's card"
      (is (= 1 (get measured ["card-not-first-author" "catalog"]))))
    (testing "a given name without romaji, and the longest rendered author component"
      (is (= 1 (get measured ["persons-without-given-romaji" "catalog"])))
      (is (= (count "Akutagawa_Ryunosuke") (get measured ["longest-author-component" "catalog"]))))
    (testing "edition years: plain dates against values with more in them"
      (is (= 3 (get measured ["edition-year-values" "rows"])))
      (is (= 2 (get measured ["edition-year-plain" "rows"])))
      (is (= 1 (get measured ["edition-year-irregular" "rows"])))
      (is (= 0 (get measured ["edition-year-appended" "rows"]))))))

(deftest a-quoted-figure-is-found-grouped-or-not-and-never-inside-a-longer-number
  (let [repo (fs/create-temp-dir {:prefix "figures"})
        write (fn [name text] (spit (fs/file (fs/path repo name)) text))
        figure (fn [n files] {:key "k" :population "catalog" :what "w" :quoted n :quoted-in files})]
    (try
      (write "grouped.md" "17,655 works")
      (write "plain.md" "17655 works")
      (write "longer.md" "117655 and 17,6550")
      (is (= [] (figures/figure-quotes repo [(figure 17655 ["grouped.md" "plain.md"])])))
      (is (= ["k: longer.md no longer states 17655"]
             (figures/figure-quotes repo [(figure 17655 ["longer.md"])])))
      (finally (fs/delete-tree repo)))))

(deftest a-derivation-that-disagrees-is-reported-before-its-quotations-are-checked
  (let [repo (fs/create-temp-dir {:prefix "figures-check"})]
    (try
      (spit (fs/file (fs/path repo "doc.md")) "there are 10 things and 7 others")
      (is (= ["b: prose says 7, the catalog gives 8 (what b, over catalog)"]
             (figures/check repo
                            [{:key "a" :population "rows" :what "what a" :quoted 10 :quoted-in ["doc.md"]}
                             {:key "b" :population "catalog" :what "what b" :quoted 7 :quoted-in ["doc.md"]}]
                            {["a" "rows"] 10 ["b" "catalog"] 8})))
      (finally (fs/delete-tree repo)))))

(deftest rounding-ties-go-to-even
  (is (= 2 (#'figures/round-half-even 2.5)))
  (is (= 4 (#'figures/round-half-even 3.5)))
  (is (= 14 (#'figures/round-half-even 14.4)))
  (is (= 2.0 (#'figures/median [1 2 3])))
  (is (= 2.5 (#'figures/median [1 2 3 4]))))
