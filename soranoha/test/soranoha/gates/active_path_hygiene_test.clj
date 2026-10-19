(ns soranoha.gates.active-path-hygiene-test
  (:require [babashka.fs :as fs]
            [clojure.string :as string]
            [clojure.test :refer [deftest is testing]]
            [soranoha.gates.active-path-hygiene :as hygiene]))

(deftest normpath-follows-posixpath
  (is (= "/repo/soranoha/x" (hygiene/normpath "/repo/./ab-validator/../soranoha//x")))
  (is (= "/" (hygiene/normpath "/../..")))
  (is (= "../../a" (hygiene/normpath "../b/../../a")))
  (is (= "." (hygiene/normpath "a/.."))))

(deftest a-candidate-is-classified-against-its-source-file
  (let [repo "/repo"
        cases [["tests/check.sh" "../soranoha/x" true]
               ["tests/check.sh" "./../soranoha/x" true]
               ["tests/check.sh" "${ROOT}/../soranoha/x" true]
               ["tests/check.sh" "foo/../../soranoha/x" true]
               ["tests/check.sh" "foo//.././../soranoha/x" true]
               ["scripts/check.sh" "/home/user/soranoha/x" true]
               ["scripts/check.sh" "'/Users/me/ab-validator/x'" true]
               ["scripts/check.sh" "/opt/build/soranoha/x" true]
               ["tests/check.sh" "$repo_root/soranoha/x" false]
               ["tests/check.sh" "$AB_WORKSPACE_ROOT/soranoha/x" false]
               ["tests/check.sh" "$AB_WORKSPACE_ROOT/../soranoha/x" true]
               ["ab-validator/crates/c/src/lib.rs" "../../../../soranoha/data/x" false]
               ["ab-validator/crates/c/src/lib.rs" "../../../../soranoha//data/./x" false]
               ["soranoha/src/a.clj" "soranoha/data/x" false]
               ["flake.nix" "./soranoha/flake.nix" false]
               ["ab-validator/Cargo.toml" "crates/abc-helper" false]
               ["ab-validator/crates/c/tests/x.rs" "/../../../soranoha/data/x" false]]]
    (doseq [[source candidate expected] cases
            :let [stripped (string/replace candidate "'" "")]]
      (when expected
        (is (some #{stripped} (hygiene/candidate-tokens stripped))
            (str "extractor keeps forbidden candidate " candidate)))
      (is (= expected (hygiene/forbidden-candidate? repo source stripped))
          (str source ": " candidate)))))

(deftest source-text-is-inspected-through-assignment-and-option-values
  (let [repo "/repo"
        source "scripts/check.sh"
        cases [["ROOT=../soranoha/x" true]
               ["--root=../soranoha/x" true]
               ["ROOT=/home/user/soranoha/x" true]
               ["ROOT=\"../soranoha/x\"" true]
               ["A=B=../soranoha/x" true]
               ["ROOT:../soranoha/x" true]
               ["ROOT=${BASE:-../soranoha/x}" true]
               ["ROOT=${BASE:-${OTHER:-../soranoha/x}}" true]
               ["https://w3id.org/soranoha/schemas/x" false]
               ["schema=sha256:abc/012345" false]
               ["label=soranoha/source" false]
               ["ROOT=$repo_root/soranoha/x" false]]]
    (doseq [[text expected] cases]
      (is (= expected
             (boolean (some #(hygiene/forbidden-candidate? repo source %)
                            (hygiene/candidate-tokens text))))
          (str text " tokens=" (pr-str (hygiene/candidate-tokens text)))))))

(deftest active-files-are-scanned-for-escapes-and-reference-checkouts
  (let [repo (fs/create-temp-dir {:prefix "active-path-hygiene"})
        write (fn [relative text]
                (let [path (fs/path repo relative)]
                  (fs/create-dirs (fs/parent path))
                  (spit (fs/file path) text)))]
    (try
      (write "scripts/bad.sh" "#!/usr/bin/env bash\nROOT=../soranoha/x\n")
      (write "scripts/ref.sh" "cat references/parsers/x\n")
      (write "tests/good.sh" "echo \"$repo_root/soranoha/x\"\n")
      (write "tests/notes.md" "../soranoha/x\n")
      (write "ab-validator/crates/c/target/debug/build.rs" "include!(\"../soranoha/x\");\n")
      (let [{:keys [files escapes references]} (hygiene/check repo)]
        (is (= 3 files) "Markdown and build outputs are outside the active set")
        (is (= ["scripts/bad.sh:2:../soranoha/x"] escapes))
        (is (= ["scripts/ref.sh:1:cat references/parsers/x"] references)))
      (testing "a repository with no active files is a finding of its own"
        (is (= {:files 0 :escapes [] :references []}
               (hygiene/check (fs/create-dirs (fs/path repo "empty"))))))
      (finally (fs/delete-tree repo)))))
