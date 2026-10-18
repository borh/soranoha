(ns soranoha.ori.projection-test
  (:require [babashka.fs :as fs]
            [clojure.string :as string]
            [clojure.test :refer [deftest is]]
            [soranoha.annotations.view :as view]
            [soranoha.kura.cas :as cas]
            [soranoha.kura.engine :as engine]
            [soranoha.ori.projection :as projection]
            [soranoha.ori.stages :as stages]))

(defn- tei [body]
  (str "<TEI xmlns=\"http://www.tei-c.org/ns/1.0\"><teiHeader/><text><body>"
       body "</body></text></TEI>"))

(deftest projections-share-the-established-reading
  (let [reading (view/from-tei
                 (tei (str "<p style=\"text-indent: 1em\">前<ruby><rb><g ref=\"#g\">犍</g></rb>"
                           "<rt>かん</rt></ruby><choice><sic>誤</sic><corr>正</corr></choice>"
                           "<note>注</note><hi rend=\"bold\">強</hi><lb/>後</p><p>次</p>")))]
    (is (= "前犍正強\n後\n次" (:view/text reading)))
    (is (= "　前犍正強\n後\n次" (projection/plaintext reading)))
    (is (= "前<ruby><rb>犍</rb><rt>かん</rt></ruby>正<strong>強</strong>  \n後\n\n次"
           (projection/markdown reading)))))

(deftest crossing-ruby-projection-names-each-target
  (let [reading (view/from-tei
                 (tei (str "<p><ruby><rb><anchor xml:id='a'/>甲<anchor xml:id='b'/>"
                           "<hi rend='bold'>乙</hi><anchor xml:id='c'/>丙<anchor xml:id='d'/></rb>"
                           "<rt from='#a' to='#c' place='right'>こうおつ</rt>"
                           "<rt from='#b' to='#d' place='left'>おつへい</rt></ruby></p>")))]
    (is (= "甲乙丙" (projection/plaintext reading)))
    (is (= "甲<strong>乙</strong>丙〔甲乙（右）: こうおつ；乙丙（左）: おつへい〕"
           (projection/markdown reading)))))

(deftest crossing-ruby-rejects-unresolved-or-reversed-ranges
  (doseq [[from to] [["#a" "#missing"] ["#b" "#a"] ["#a" "#a"] ["other.xml#a" "#b"]]]
    (let [reading (view/from-tei (tei (str "<p><ruby><rb><anchor xml:id='a'/>甲<anchor xml:id='b'/></rb>"
                                           "<rt from='" from "' to='" to "'>こう</rt></ruby></p>")))]
      (is (thrown-with-msg? clojure.lang.ExceptionInfo #"ordered range"
                            (projection/markdown reading))))))

(deftest markdown-source-text-cannot-inject-markup
  (let [reading (view/from-tei (tei "<p>*本文* &lt;script&gt;<ruby><rb>字</rb><rt>&lt;b&gt;</rt></ruby></p>"))]
    (is (= "\\*本文\\* &lt;script&gt;<ruby><rb>字</rb><rt>&lt;b&gt;</rt></ruby>"
           (projection/markdown reading)))))

(deftest projection-policy-change-does-not-rerun-its-sibling
  (let [dir (fs/create-temp-dir {:prefix "projection-cache"})
        store (engine/open-store! {:cas-dir (str (fs/path dir "objects"))
                                   :db-path (str (fs/path dir "trace.sqlite"))})
        inputs {"tei" (cas/put-bytes! (:cas-dir store) (.getBytes ^String (tei "<p>本文</p>") "UTF-8"))}
        plain (stages/plaintext-stage "runtime")
        markdown (stages/markdown-stage "runtime")]
    (try
      (engine/run-stage! store plain inputs)
      (engine/run-stage! store markdown inputs)
      (is (false? (:cached? (engine/run-stage! store (assoc markdown :stage-version "next-policy") inputs))))
      (is (:cached? (engine/run-stage! store plain inputs)))
      (finally (engine/close-store! store) (fs/delete-tree dir)))))

(deftest unspecified-or-conflicting-renditions-remain-explicitly-unsupported
  (doseq [attributes ["rend='emphasis'" "rend='bouten unknown right'"
                      "rend='bouten 傍点 both'" "rend='bouten 傍点 left extra'"
                      "rend='keigakomi border(double)'"
                      "rend='bosen 傍線 right' style='color:red'"
                      "rend='bold' style='font-style:italic'"]]
    (let [reading (view/from-tei (tei (str "<p><hi " attributes ">本文</hi></p>")))
          report (projection/report :projection/markdown reading)]
      (is (= "limited" (get report "status")))
      (is (some #(= {"family" "hi" "disposition" "unsupported" "count" 1} %) (get report "counts")))
      (is (= "本文" (projection/plaintext reading)))
      (is (= "markdown/2" (get report "profile"))))))

(deftest both-projections-replace-an-unencodable-glyph-the-same-way
  ;; The source identifies the glyph and the TEI carries its description; no
  ;; artifact may state that the bytes were damaged, and the Markdown and the
  ;; plaintext of one work must replace the glyph with the same character.
  (let [reading (view/from-tei (tei "<p>前<g ref=\"#gaiji-1\"/>後</p>"))]
    (is (= (str "前" view/unresolved-glyph "後") (projection/plaintext reading)))
    (is (= (str "前" view/unresolved-glyph "後") (projection/markdown reading)))
    (is (not (string/includes? (projection/markdown reading) "�"))
        "U+FFFD is what a decoder writes when bytes were malformed")))

(deftest the-body-character-count-is-the-non-whitespace-code-points-of-the-plaintext
  ;; 字下げ (U+3000), line breaks and a no-break space are layout and do not
  ;; count; a character outside the BMP is one character, not two UTF-16
  ;; units; U+FFFC stands for an unresolved glyph and counts as the character
  ;; it stands for.
  (is (= 8 (projection/body-character-count "\u3000蜘蛛の糸\n\u00a0a𠮷\ufffc b\t")))
  (is (= 0 (projection/body-character-count "")))
  (is (= 0 (projection/body-character-count "\n\u3000\n")))
  (is (= 6 (projection/body-character-count
            (projection/plaintext
             (view/from-tei (tei "<p style=\"text-indent: 1em\">前<ruby><rb>犍</rb><rt>かん</rt></ruby>正<lb/>後</p><p>次々</p>")))))
      "the restored indentation and the breaks are not counted, the ruby base is"))
