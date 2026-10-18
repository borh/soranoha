(ns soranoha.core.ndc-test
  (:require [clojure.test :refer [deftest is]]
            [soranoha.core.ndc :as ndc]))

(def ^:private turtle
  (str "ndc9: a skos:ConceptScheme ;\n"
       "\tskos:prefLabel \"日本十進分類法新訂9版\"@ja\n"
       ".\n"
       "ndc9:913 a ndcv:Section, skos:Concept ;\n"
       "\trdfs:label \"日本文学--小説．物語\" ;\n"
       "\tskos:prefLabel \"小説．物語\"@ja, \"Fiction.Romance.Novel\"@en ;\n"
       "\tskos:notation \"913\"\n"
       ".\n"
       "ndc9:913.2 a skos:Concept ;\n"
       "\tskos:prefLabel \"古代前期\"@ja\n"
       ".\n"
       "ndc9:914 a ndcv:Section ;\n"
       "\tskos:prefLabel \"評論．エッセイ．随筆\"@ja\n"
       ".\n"
       "ndc9:915 a ndcv:Section ;\n"
       "\tskos:prefLabel \"Diaries\"@en\n"
       ".\n"))

(deftest three-digit-classes-are-read-with-their-labels
  (is (= {"913" {"ja" "小説．物語" "en" "Fiction.Romance.Novel"}
          "914" {"ja" "評論．エッセイ．随筆"}}
         (ndc/sections turtle))
      "the scheme, finer subdivisions and a class without a Japanese label are left out"))

(deftest the-committed-table-resolves-the-classes-the-header-declares
  (is (= {"ja" "小説．物語" "en" "Fiction.Romance.Novel"} (ndc/labels "913")))
  (is (nil? (ndc/labels "998"))))
