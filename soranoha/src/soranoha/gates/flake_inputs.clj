(ns soranoha.gates.flake-inputs
  "Release-critical flake inputs are pinned to a revision or tag in flake.nix,
  and the component flake's lock agrees with the root lock."
  (:require [babashka.fs :as fs]
            [soranoha.core.json :as json]))

(def release-critical
  #{"aozorabunko-src" "mecab-dic-converter-src" "ndc9-ld" "tei-eaj-aozora-tei" "tei-p5"})

(def infrastructure
  #{"clj-nix" "crane" "devshell" "flake-parts" "flake-utils" "local-pkgs" "nix-fetcher-data"
    "nixpkgs-lib" "nixpkgs" "root" "rust-overlay" "systems"})

(defn- explicitly-pinned?
  "A path or file input has no revision; the lock's narHash is its pin, and a
  changed file fails the fetch rather than drifting."
  [original]
  (or (contains? #{"path" "file"} (get original "type"))
      (boolean (some #(and (string? %) (seq %)) [(get original "rev") (get original "ref")]))))

(defn lock-messages
  "Errors, then warnings prefixed `warning: `, for one parsed lock file."
  [lock-path lock]
  (let [nodes (sort-by key (get lock "nodes"))
        originals (for [[name node] nodes
                        :let [original (get node "original")]
                        :when (map? original)]
                    [name original])
        errors (for [[name original] originals
                     :when (and (contains? release-critical name)
                                (not (explicitly-pinned? original)))]
                 (str lock-path ": release-critical input '" name
                      "' lacks an explicit rev/ref in flake.nix"))
        warnings (for [[name original] originals
                       :when (and (not (contains? release-critical name))
                                  (not (contains? infrastructure name))
                                  (not= "path" (get original "type")))]
                   (str "warning: " lock-path ": input '" name
                        "' is not classified as release-critical or infrastructure"))]
    (vec (concat errors warnings))))

(defn- comparable-node [node]
  (let [original (get node "original")
        locked (get node "locked")]
    (when (and (map? original) (not= "path" (get original "type")))
      {"original" original "locked" (if (map? locked) locked {})})))

(defn component-coherence
  "Inputs the component lock shares with the root lock resolve to the same
  original and locked values."
  [component-lock-path root-lock component-lock]
  (let [root-nodes (get root-lock "nodes" {})]
    (vec (for [[name node] (sort-by key (get component-lock "nodes" {}))
               :when (and (not= "root" name) (contains? root-nodes name))
               :let [here (comparable-node node)
                     there (comparable-node (get root-nodes name))]
               :when (and here there (not= here there))]
           (str component-lock-path ": input '" name "' differs from root flake.lock")))))

(defn check
  "Every message for the repository's lock files, errors first."
  [repo-root]
  (let [root-lock-path (fs/path repo-root "flake.lock")
        component-lock-path (fs/path repo-root "ab-validator" "flake.lock")
        read (fn [path] (json/read-json-file (fs/file path)))
        messages (concat (when (fs/exists? root-lock-path)
                           (lock-messages (str root-lock-path) (read root-lock-path)))
                         (when (fs/exists? component-lock-path)
                           (lock-messages (str component-lock-path) (read component-lock-path)))
                         (when (fs/exists? component-lock-path)
                           (component-coherence (str component-lock-path)
                                                (read root-lock-path) (read component-lock-path))))]
    {:errors (vec (remove #(.startsWith ^String % "warning:") messages))
     :warnings (vec (filter #(.startsWith ^String % "warning:") messages))}))

(defn gate!
  "Print errors to stderr and warnings to stdout; the exit code for the kernel."
  [repo-root]
  (let [{:keys [errors warnings]} (check repo-root)]
    (binding [*out* *err*]
      (doseq [message errors] (println message)))
    (doseq [message warnings] (println message))
    (if (seq errors)
      1
      (do (println "flake input policy passed") 0))))
