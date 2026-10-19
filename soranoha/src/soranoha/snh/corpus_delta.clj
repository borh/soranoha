(ns soranoha.snh.corpus-delta
  "How the corpus at one upstream revision differs from what a release
  published, computed on the reader's own machine.

  `soranoha.snh.release-delta` computes the delta between two
  releases, from published bytes alone. This namespace computes it for a revision no
  release was built at, which no manifest describes: the bytes at that
  revision are not in the chain. The archives at that revision must be read,
  and the reader reads them from their own aozorabunko checkout, rather than
  a server rebuilding history on demand.

  No parser runs. A work's `source_content_hash` is the hash of its archive's
  identity object, produced by the `extract` stage before any parsing, so a
  release's per-work source identity can be recomputed from the archive alone.
  The cost is proportional to reading the corpus rather than to
  converting it.

  The comparison is therefore about sources, not about documents. Under one
  toolchain a source difference is the only thing that can change a
  document, so source comparisons resolve the full difference; across a toolchain
  change it does not, and a full rebuild of both revisions is the only thing
  that does. `docs/performance.md` has that recipe.

  Comparability is checked rather than assumed. A `source_content_hash` means
  what the `extract` stage version that produced it means, so a release built by
  a different version of that stage is rejected rather than compared."
  (:require [clojure.set :as set]
            [soranoha.aozora.csv :as csv]
            [soranoha.aozora.source-bundle :as source-bundle]
            [soranoha.core.hash :as hash]
            [soranoha.core.parallel :as parallel]
            [soranoha.ori.stages :as stages]
            [soranoha.yomi.catalog :as catalog]
            [soranoha.yomi.select :as select]))

(def source-hash-stage-version
  "The `extract` stage version whose source identity this namespace
  recomputes. Read off the stage rather than restated, so the scan cannot
  report a version other than the stage's own. The constructor argument is the
  toolchain identity, which keys derivations and does not affect the version."
  (:stage-version (stages/extract-stage nil)))

(defn- bare-hex
  "The manifest's form of a source content hash. `inspect-zip` returns the
  prefixed `sha256:<hex>`, the schema pins bare hex, and the release assembler
  strips the prefix on the way in, so the scan has to strip it on the way out
  or every work would compare as changed."
  [source-content-hash]
  (or (hash/bare-sha256-hex source-content-hash)
      (throw (ex-info "malformed source content hash"
                      {:reason :malformed-source-content-hash
                       :value source-content-hash}))))

(defn scan
  "Every selected work's `source_content_hash` in an aozorabunko checkout.

  Returns `{:works {slug hex} :unreadable [{:slug :relpath :reason}]}`. An
  archive the admission rules refuse is collected rather than thrown: an
  arbitrary historical revision is where one is likely, and one refused
  archive should not withhold the results for the other seventeen thousand."
  [aozora-root {:keys [concurrency limit]}]
  (let [{:keys [csv-text]} (catalog/read-catalog-zip aozora-root)
        rows (csv/read-rows-from-string csv-text)
        {:keys [candidates]} (select/select-candidates aozora-root rows)
        candidates (if (and limit (pos? limit)) (vec (take limit candidates)) candidates)
        n (if (and concurrency (pos? concurrency))
            concurrency
            (.availableProcessors (Runtime/getRuntime)))
        outcomes (parallel/ordered-pmap
                  n
                  (fn [{:keys [slug file relpath]}]
                    (try
                      {:ok [slug (bare-hex (:bundle-hash (source-bundle/inspect-zip file)))]}
                      ;; only an admission refusal is data about the archive;
                      ;; anything else is a defect and must reach the caller
                      (catch clojure.lang.ExceptionInfo e
                        (let [data (ex-data e)]
                          (if (:soranoha.aozora.source-bundle/admission-error data)
                            {:unreadable {:slug slug :relpath relpath
                                          :reason (name (:reason data))}}
                            (throw e))))))
                  candidates)]
    {:works (into (sorted-map) (keep :ok) outcomes)
     :unreadable (vec (keep :unreadable outcomes))}))

(defn delta
  "Compare a decoded release manifest against a `scan` of a local checkout.

  Returns

    {:corpus {:release <upstream_rev> :checkout <rev or nil>}
     :extract {:release <version> :checkout <version>}
     :works {:only-in-checkout [slug] :only-in-release [slug]
             :withdrawn [slug] :source-changed [slug] :unchanged <count>}
     :unreadable [{:slug :relpath :reason}]}

  `:withdrawn` is separated out of `:only-in-checkout` because it is the one
  case where the release omits a work by decision: the slug is listed in the
  manifest's `withdrawn` map, so a governance event removed it and the reader's
  checkout still holding the source is expected rather than a difference.

  Throws when the manifest's `extract` stage version is not the one `scan`
  reproduces. Two versions may hash the same archive differently, so
  differencing across them would report source changes that are the stage's."
  [manifest {:keys [works unreadable]} checkout-rev]
  (let [release-version (get-in manifest ["toolchain" "extract" "stage_code_version"])]
    (when-not (= release-version source-hash-stage-version)
      (throw (ex-info "release source hashes were produced by another extract version"
                      {:reason :extract-version-mismatch
                       :release release-version
                       :checkout source-hash-stage-version})))
    (let [published (into {} (map (juxt #(get % "slug") #(get % "source_content_hash")))
                          (get manifest "works"))
          withdrawn (into #{} (map #(get % "slug")) (get manifest "withdrawn"))
          local (set (keys works))
          absent (set/difference local (set (keys published)))
          shared (set/intersection local (set (keys published)))
          changed (filterv #(not= (get works %) (get published %)) (sort shared))]
      {:corpus {:release (get-in manifest ["corpus" "upstream_rev"])
                :checkout checkout-rev}
       :extract {:release release-version :checkout source-hash-stage-version}
       :works {:only-in-checkout (vec (sort (set/difference absent withdrawn)))
               :only-in-release (vec (sort (set/difference (set (keys published)) local)))
               :withdrawn (vec (sort (set/intersection absent withdrawn)))
               :source-changed changed
               :unchanged (- (count shared) (count changed))}
       :unreadable (vec unreadable)})))

(defn report
  "`delta` as the deterministic JSON value the CLI prints: string keys, sorted
  maps, slug vectors already in order."
  [manifest scanned checkout-rev]
  (let [d (delta manifest scanned checkout-rev)
        strings (fn [m] (into (sorted-map) (map (fn [[k v]] [(name k) v])) m))]
    (into (sorted-map)
          {"corpus" (strings (:corpus d))
           "extract" (strings (:extract d))
           "works" (strings (:works d))
           "unreadable" (mapv strings (:unreadable d))})))
