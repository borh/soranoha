(ns soranoha.gates.flake-inputs-test
  (:require [clojure.test :refer [deftest is testing]]
            [soranoha.gates.flake-inputs :as flake-inputs]))

(defn- lock [nodes] {"nodes" nodes})

(deftest release-critical-inputs-need-a-revision-or-tag
  (let [messages (flake-inputs/lock-messages
                  "flake.lock"
                  (lock {"root" {"inputs" {}}
                         "tei-p5" {"original" {"type" "github" "owner" "TEIC" "repo" "TEI" "ref" "P5_Release_4.11.0"}}
                         "aozorabunko-src" {"original" {"type" "git" "url" "ssh://x/y.git"}}
                         "ndc9-ld" {"original" {"type" "file" "url" "https://example.org/ndc9.zip"}}
                         "nixpkgs" {"original" {"type" "github" "owner" "NixOS" "repo" "nixpkgs" "ref" "nixos-unstable"}}
                         "local-pkgs" {"original" {"type" "path" "path" "./nix"}}
                         "stray" {"original" {"type" "github" "owner" "a" "repo" "b"}}}))]
    (is (= ["flake.lock: release-critical input 'aozorabunko-src' lacks an explicit rev/ref in flake.nix"
            "warning: flake.lock: input 'stray' is not classified as release-critical or infrastructure"]
           messages)
        "a file input is pinned by its narHash, a tag counts as a pin, infrastructure is exempt")))

(deftest a-component-lock-must-agree-with-the-root-lock
  (let [node (fn [rev] {"original" {"type" "github" "owner" "o" "repo" "r"}
                        "locked" {"rev" rev}})
        root (lock {"nixpkgs" (node "aaa") "crane" (node "ccc") "local-pkgs" {"original" {"type" "path"}}})]
    (is (= [] (flake-inputs/component-coherence "ab-validator/flake.lock" root
                                                (lock {"root" {} "nixpkgs" (node "aaa") "only-here" (node "x")}))))
    (testing "a differing locked revision is reported by name"
      (is (= ["ab-validator/flake.lock: input 'nixpkgs' differs from root flake.lock"]
             (flake-inputs/component-coherence "ab-validator/flake.lock" root
                                               (lock {"nixpkgs" (node "bbb") "crane" (node "ccc")})))))
    (testing "path inputs are outside the comparison"
      (is (= [] (flake-inputs/component-coherence "c" root
                                                  (lock {"local-pkgs" {"original" {"type" "path"} "locked" {"x" 1}}})))))))
