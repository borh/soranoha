(ns soranoha.snh.release-delta
  "What changed between two published releases, read from the two manifests
  and nothing else.

  Signed bytes provide all data required for the comparison.
  `corpus.upstream_rev` identifies the upstream revision the release was built
  from, `toolchain` carries each stage's closure hash and code version, and
  every `works[]` entry carries the work's `source_content_hash` beside its
  artifact ids. A reader holding two manifests therefore derives the same
  answer the serving layer does, with no corpus checkout, no rebuild and no
  trust in the publisher.

  The classification keeps two causes apart. A work whose
  `source_content_hash` changed is publishing a different source state. A work
  whose source hash is unchanged while its artifact ids changed is the same
  edition converted again: that produces new documents and never a new
  edition, which is the distinction the published glossary draws between a
  work, an edition and a document. Reporting both as `changed` would lose it.

  `:unexplained` narrows `:documents-changed` to the works for which no
  declared input changed either. A document is a function of more than its
  source and the stages that convert it: the metadata record comes from the
  catalog rows, and the rights block reaches the header. Both are manifest
  fields, `catalog` as the published catalog's content id and `rights`
  verbatim, so both are reported here beside the toolchain and all three must
  be unchanged before a work counts as unexplained. Without that the field
  would report every catalog-only revision that edited a published work's
  metadata, about one in ten of them according to `release-needed!`, and
  would carry no determinism signal.

  The catalog id is release-wide. A changed catalog therefore withdraws the
  claim for every work rather than attributing any one work's change to its
  own metadata, which would need a per-work metadata identity that
  `snh-manifest/3` does not carry.

  The remainder is a determinism finding: bytes that changed while every
  declared input to their production is unchanged. Between two adjacent
  releases the transaction should already have refused to publish it, since
  a matching projection and differing derived content is its determinism
  halt, so finding one here means the halt failed. Between releases that are
  not adjacent, or in a chain the reader did not observe being built, this is
  the only place the question is asked.

  Any two manifests may be compared. Adjacency is not required and is not
  checked, so the corpus range reported is the pair of revisions rather than
  a claim about the chain between them."
  (:require [clojure.set :as set]))

(defn- by-slug [manifest]
  (into {} (map (juxt #(get % "slug") identity)) (get manifest "works")))

(defn- withdrawn-slugs [manifest]
  (into #{} (map #(get % "slug")) (get manifest "withdrawn")))

(defn- sorted-vec [slugs]
  (vec (sort slugs)))

(defn- moved
  "A declared manifest input, reported as its pair of values. Always present,
  changed or not: `:unexplained` requires these to be unchanged, so a reader
  of the report can see why it is empty."
  [earlier later key]
  {:from (get earlier key) :to (get later key)})

(defn- unmoved? [{:keys [from to]}] (= from to))

(defn- toolchain-delta
  "Stage coordinates that changed, keyed by stage. A stage present on one side
  only reports the missing side as nil rather than being dropped: a stage
  entering or leaving the toolchain changes what produced the documents just
  as much as a version bump does."
  [earlier later]
  (let [a (get earlier "toolchain")
        b (get later "toolchain")]
    (into (sorted-map)
          (keep (fn [stage]
                  (when (not= (get a stage) (get b stage))
                    [stage {:from (get a stage) :to (get b stage)}])))
          (set/union (set (keys a)) (set (keys b))))))

(defn- validation-delta
  "Differences in invalid works between two releases, with invalid counts:
  works newly invalid in `later`, and works newly valid (invalid in `earlier`
  but valid in `later`). Derived from the `validation_summary` in each
  manifest, which verifiers reconstruct from published validation records.
  No cause is attributed: a tightened profile and a conversion regression
  both appear here, distinguished by differences in `:toolchain`."
  [earlier later]
  (let [before (set (get-in earlier ["validation_summary" "invalid_slugs"]))
        after (set (get-in later ["validation_summary" "invalid_slugs"]))]
    {:invalid {:from (count before) :to (count after)}
     :newly-invalid (sorted-vec (set/difference after before))
     :newly-valid (sorted-vec (set/difference before after))}))

(defn- classify
  "The one established cause for a work present in both releases. Source is
  tested first because it is the only cause the manifest attributes on its
  own: once the source hash has changed, changed artifacts add no further
  cause.

  A standing that changed is tested next, before the documents it would explain.
  Each TEI file states the terms of the work it carries, so a work whose
  standing changed necessarily has different bytes, and reporting that as an
  unattributed document change would hide the cause."
  [a b]
  (cond
    (not= (get a "source_content_hash") (get b "source_content_hash")) :source-changed
    (not= (get a "rights") (get b "rights")) :rights-changed
    (not= (get a "artifacts") (get b "artifacts")) :documents-changed
    (not= (get a "layers") (get b "layers")) :layers-changed
    :else :unchanged))

(defn delta
  "Compare two decoded release manifest values, earlier first.

  Returns

    {:corpus {:from <upstream_rev> :to <upstream_rev>}
     :catalog {:from <catalog id> :to <catalog id>}
     :rights {:from <rights> :to <rights>}
     :toolchain {<stage> {:from <coordinate> :to <coordinate>}}
     :works {:added [slug] :withdrawn [slug] :dropped [slug]
             :source-changed [slug] :rights-changed [slug]
             :documents-changed [slug] :layers-changed [slug]
             :unchanged <count>}
     :validation {:invalid {:from <count> :to <count>}
                  :newly-invalid [slug] :newly-valid [slug]}
     :unexplained [slug]}

  `:withdrawn` and `:dropped` are both works the later release does not
  publish, split because the reason differs and only one of them is a
  governance act: a withdrawn slug is listed in the later manifest's
  `withdrawn` map, a dropped one is not admitted. `:unchanged`
  is a count rather than a list, because at corpus scale it is every work."
  [earlier later]
  (let [a (by-slug earlier)
        b (by-slug later)
        gone (set/difference (set (keys a)) (set (keys b)))
        withdrawn (withdrawn-slugs later)
        stages (toolchain-delta earlier later)
        buckets (group-by #(classify (get a %) (get b %))
                          (set/intersection (set (keys a)) (set (keys b))))
        documents-changed (sorted-vec (:documents-changed buckets))
        catalog (moved earlier later "catalog")
        rights (moved earlier later "rights")]
    {:corpus {:from (get-in earlier ["corpus" "upstream_rev"])
              :to (get-in later ["corpus" "upstream_rev"])}
     :catalog catalog
     :rights rights
     :toolchain stages
     :works {:added (sorted-vec (set/difference (set (keys b)) (set (keys a))))
             :withdrawn (sorted-vec (filter withdrawn gone))
             :dropped (sorted-vec (remove withdrawn gone))
             :source-changed (sorted-vec (:source-changed buckets))
             :rights-changed (sorted-vec (:rights-changed buckets))
             :documents-changed documents-changed
             :layers-changed (sorted-vec (:layers-changed buckets))
             :unchanged (count (:unchanged buckets))}
     :validation (validation-delta earlier later)
     :unexplained (if (and (empty? stages) (unmoved? catalog) (unmoved? rights))
                    documents-changed
                    [])}))

(defn report
  "`delta` as the deterministic JSON value the CLI and the serving layer both
  print: string keys, sorted maps, and slug vectors already in order."
  [earlier later]
  (let [d (delta earlier later)]
    (into (sorted-map)
          {"corpus" (into (sorted-map)
                          {"from" (get-in d [:corpus :from])
                           "to" (get-in d [:corpus :to])})
           "catalog" (into (sorted-map)
                           {"from" (get-in d [:catalog :from])
                            "to" (get-in d [:catalog :to])})
           "rights" (into (sorted-map)
                          {"from" (get-in d [:rights :from])
                           "to" (get-in d [:rights :to])})
           "toolchain" (into (sorted-map)
                             (map (fn [[stage {:keys [from to]}]]
                                    [stage (into (sorted-map)
                                                 {"from" from "to" to})]))
                             (:toolchain d))
           "works" (into (sorted-map)
                         (map (fn [[k v]] [(name k) v]))
                         (:works d))
           "validation" (into (sorted-map)
                              {"invalid" (into (sorted-map)
                                               {"from" (get-in d [:validation :invalid :from])
                                                "to" (get-in d [:validation :invalid :to])})
                               "newly_invalid" (get-in d [:validation :newly-invalid])
                               "newly_valid" (get-in d [:validation :newly-valid])})
           "unexplained" (:unexplained d)})))
