(ns soranoha.gates.docs-links-test
  (:require [babashka.fs :as fs]
            [clojure.string :as string]
            [clojure.test :refer [deftest is testing]]
            [soranoha.gates.docs-links :as docs-links]))

(deftest a-heading-becomes-the-anchor-a-renderer-derives
  (is (= "get-one-text" (docs-links/slug "Get one text")))
  (is (= "rights-licensing" (docs-links/slug "Rights & licensing"))
      "punctuation is dropped and the whitespace around it collapses")
  (is (= "body-v1" (docs-links/slug "body-v1")))
  (is (= "snh_vocab-version" (docs-links/slug "`snh_vocab-version`")))
  (is (= "本文-読み" (docs-links/slug "本文 ・ 読み"))
      "letters outside ASCII are kept, a middle dot is not"))

(deftest links-resolve-to-files-and-fragments-to-headings
  (let [repo (fs/create-temp-dir {:prefix "docs-links"})
        docs (fs/create-dirs (fs/path repo "docs"))
        write (fn [name text] (spit (fs/file (fs/path docs name)) text))]
    (try
      (write "a.md" (string/join "\n" ["# A"
                                       "[ok](b.md) [frag](b.md#second-part) [self](#a)"
                                       "[gone](missing.md) [nohead](b.md#third-part)"
                                       "[ext](https://example.org/x) [mail](mailto:x@example.org)"
                                       "![image](../assets/pipeline.svg)"]))
      (write "b.md" "# B\n\n## Second part\n")
      (fs/create-dirs (fs/path repo "assets"))
      (spit (fs/file (fs/path repo "assets" "pipeline.svg")) "<svg/>")
      (let [{:keys [documents problems]} (docs-links/check repo)]
        (is (= 2 documents))
        (is (= ["docs/a.md: missing.md does not exist"
                "docs/a.md: b.md#third-part names no heading"]
               problems)))
      (testing "a document outside the documentation roots is not scanned"
        (spit (fs/file (fs/path repo "README.md")) "[x](nowhere.md)")
        (is (= 2 (:documents (docs-links/check repo)))))
      (finally (fs/delete-tree repo)))))
