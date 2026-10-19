(ns soranoha.snh.schema
  "Protocol JSON Schemas, loaded from resources and keyed by artifact type."
  (:require [charred.api :as json]
            [clojure.java.io :as io]
            [soranoha.core.schema :as schema-validator]))

(def schema-resources
  "Artifact type -> classpath resource of the schema the writer emits. A type
  whose bytes can carry more than one published version selects its read
  schema by the object's own version discriminator; see `decode-errors`."
  {"release-manifest"    "snh/schemas/snh-manifest-3.schema.json"
   "assessment-snapshot" "snh/schemas/snh-assessment-snapshot-2.schema.json"
   "admission-report"    "snh/schemas/snh-admission-report-1.schema.json"
   "governance-event"    "snh/schemas/snh-governance-event-1.schema.json"
   "catalog"             "snh/schemas/snh-catalog-2.schema.json"})

(def catalog-version-resources
  "Catalog version discriminator -> classpath resource, one entry per version
  a published chain can carry. The verifier reads the version the stored bytes
  were signed under; the writer always emits the current version named in
  `schema-resources`, because a derived value must not be re-emitted under an
  older schema."
  {"snh-catalog/1" "snh/schemas/snh-catalog-1.schema.json"
   "snh-catalog/2" "snh/schemas/snh-catalog-2.schema.json"})

(def ^:private loaded
  (delay
    (into {}
          (map (fn [resource]
                 (let [url (or (io/resource resource)
                               (throw (ex-info "Protocol schema resource missing"
                                               {:resource resource})))]
                   [resource (json/read-json (slurp url))])))
          (distinct (concat (vals schema-resources)
                            (vals catalog-version-resources))))))

(defn schema-for
  "Parsed current protocol schema for an artifact type; throws on any other type."
  [type]
  (or (get @loaded (get schema-resources type))
      (throw (ex-info "No protocol schema for artifact type"
                      {:type type :known (keys schema-resources)}))))

(defn- catalog-schema
  "The catalog schema named by the value's own version discriminator, or the
  current schema when it names no published version, so an unknown version is
  refused by that schema instead of being accepted."
  [value]
  (get @loaded (or (get catalog-version-resources (get value "schema"))
                   (get schema-resources "catalog"))))

(defn validation-errors
  "Validation errors for `value` against the current schema of `type`; nil
  when valid. This is the writer's check."
  [type value]
  (schema-validator/validation-errors (schema-for type) value))

(defn decode-errors
  "Validation errors for `value` as published. For a versioned artifact the
  schema is selected by the object's own version discriminator, so a
  historical catalog is read under the schema it was signed under; every
  other type has one published version."
  [type value]
  (schema-validator/validation-errors
   (if (= "catalog" type) (catalog-schema value) (schema-for type))
   value))
