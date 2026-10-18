(ns soranoha.ori.tei
  (:require [soranoha.ori.publication-whitespace :as whitespace]
            [soranoha.annotations.view :as view]
            [clojure.string :as string]))

(defn- present-text? [text]
  (seq text))

(defn- source-reference [node]
  (when-let [span (get node "source_span")]
    (when-not (and (= "decoded_utf8" (get span "coordinate_system"))
                   (integer? (get span "start")) (integer? (get span "end"))
                   (<= 0 (get span "start") (get span "end")))
      (throw (ex-info "TEI source correspondence requires decoded UTF-8 coordinates" {:span span})))
    (str "source-" (get span "start") "-" (get span "end"))))

(defn- sourced [node element]
  (let [reference (::source-reference node)
        markers (::interpretation-marker-references node)
        attrs (cond-> (if (map? (second element)) (second element) {})
                reference (assoc :source (str "#" reference))
                (seq markers) (assoc :corresp (string/join " " (map #(str "#" %) markers))))]
    (if (or reference (seq markers))
      (if (map? (second element))
        (assoc element 1 attrs)
        (into [(first element) attrs] (rest element)))
      element)))

(defn- register-source-span [acc span]
  (let [reference (source-reference {"source_span" span})
        existing (get-in acc [:source-spans reference])]
    (when (and existing (not= existing span))
      (throw (ex-info "Conflicting metadata for one decoded source extent"
                      {:reference reference :existing existing :span span})))
    [(cond-> acc reference (assoc-in [:source-spans reference] span)) reference]))

(defn- register-source [acc node]
  (let [[acc reference] (register-source-span acc (get node "source_span"))
        [acc markers] (reduce (fn [[acc references] span]
                                (let [[acc marker] (register-source-span acc span)]
                                  [acc (conj references marker)]))
                              [acc []] (get node "interpretation_marker_spans"))]
    [acc (assoc node ::source-reference reference ::interpretation-marker-references markers)]))

(defn- present-ruby-text? [text]
  (and (string? text)
       (not (string/blank? text))))

(defn- mark-omitted [acc node-type]
  (update acc :omitted conj {:type node-type :policy "omitted"}))

(defn- append-inline [acc node]
  (update acc :current-paragraph conj node))

(defn- append-structural-child [acc node]
  (if (seq (:current-division acc))
    (update acc :current-division conj node)
    (update acc :body-children conj node)))

(defn- layout-rend [layout]
  (case (get layout "kind")
    "jisage" (when-some [indent (get layout "indent")]
               (str "jisage indent(" indent ")"))
    "burasage" (let [first-line (get layout "first_line_indent")
                     continuation (get layout "continuation_indent")]
                 (when (and (some? first-line) (some? continuation))
                   (str "burasage first(" first-line ") rest(" continuation ")")))
    "chitsuki" (let [align (get layout "align")
                     offset (get layout "offset_from_end")]
                 (when (and align (some? offset))
                   (str "chitsuki align(" align ") offset-from-end(" offset ")")))
    "jizume" (when-some [width (get layout "width")]
               (str "jizume width(" width ")"))
    "line-jisage" (when-some [indent (get layout "indent")]
                    (str "line-jisage indent(" indent ")"))
    nil))

(defn- layout-params [layout]
  (case (get layout "kind")
    "jisage" (when-some [indent (get layout "indent")]
               (str "indent=" indent))
    "burasage" (let [first-line (get layout "first_line_indent")
                     continuation (get layout "continuation_indent")]
                 (when (and (some? first-line) (some? continuation))
                   (str "first-line-indent=" first-line
                        ";continuation-indent=" continuation)))
    "chitsuki" (let [align (get layout "align")
                     offset (get layout "offset_from_end")]
                 (when (and align (some? offset))
                   (str "align=" align ";offset-from-end=" offset)))
    "jizume" (when-some [width (get layout "width")]
               (str "width=" width))
    "line-jisage" (when-some [indent (get layout "indent")]
                    (str "indent=" indent))
    "font-size" (case (get layout "size_type")
                  "absolute" (str "size-type=absolute;size=" (get layout "size"))
                  "qualitative" (str "size-type=qualitative;direction=" (get layout "direction")
                                     (when-let [qualifier (get layout "qualifier")]
                                       (str ";qualifier=" qualifier)))
                  (let [size-type (get layout "size_type")
                        level (get layout "level")]
                    (when (and size-type (some? level))
                      (str "size-type=" size-type ";level=" level))))
    "baseline-position" (str "position=" (get layout "position")
                             (when-let [degrees (get layout "angle_degrees")]
                               (str ";angle-degrees=" degrees)))
    "small-script" (str "position=" (get layout "position"))
    "tcy" (when-let [marker (get layout "marker")]
            (str "marker=" marker))
    "keigakomi" (when-let [border (get layout "border")]
                  (str "border=" border))
    "yokogumi" "direction=horizontal"
    "tategumi" "direction=vertical"
    nil))

(defn- layout-attributes [layout]
  (if (vector? layout) layout [layout]))

(defn- emphasis-rend
  "The rend value for an emphasis: its style and the decoration's kind and
  position, whichever are present."
  [m]
  (string/join " " (remove nil? [(get m "style")
                                 (get-in m ["decoration" "kind"])
                                 (get-in m ["decoration" "position"])])))

(defn- inline-layout-rend [layout]
  (case (get layout "kind")
    ("chitsuki" "line-jisage") (layout-rend layout)
    "emphasis" (emphasis-rend layout)
    "font-size" (case (get layout "size_type")
                  "absolute" (str "font-size absolute(" (get layout "size") ")")
                  "qualitative" (str "font-size qualitative(" (get layout "direction") ")"
                                     (when-let [qualifier (get layout "qualifier")]
                                       (str " qualifier(" qualifier ")")))
                  (let [size-type (get layout "size_type")
                        level (get layout "level")]
                    (when (and size-type (some? level))
                      (str "font-size " size-type "(" level ")"))))
    "baseline-position" (str "baseline-" (get layout "position")
                             (when-let [degrees (get layout "angle_degrees")]
                               (str " angle(" degrees "deg)")))
    "exponent" "exponent"
    "small-script" (str "small-script " (get layout "position"))
    "tcy" "text-combine-upright"
    "keigakomi" (if-let [border (get layout "border")]
                  (str "keigakomi border(" border ")")
                  "keigakomi")
    "yokogumi" "yokogumi horizontal"
    "tategumi" "tategumi vertical"
    "fraction" "fraction"
    nil))

(defn- paragraph-attrs [paragraph]
  (when-let [layout (get paragraph "layout")]
    (let [rend (layout-rend layout)
          params (layout-params layout)]
      (cond-> {}
        rend
        (assoc :rend rend)

        (get layout "kind")
        (assoc :snh/layout-kind (get layout "kind"))

        params
        (assoc :snh/layout-params params)))))

(defn- leading-indent [text]
  (count (re-find #"^　+" (or text ""))))

(defn- source-note-hiccup [node]
  (into (sourced node [:note {:type (get node "note_type")}])
        (interpose [:lb]
                   (map (fn [line]
                          (let [indent (leading-indent line)]
                            [:seg (cond-> {:type "source-line"}
                                    (pos? indent)
                                    (assoc :style (str "padding-inline-start: " indent "em")))
                             (subs line indent)]))
                        (string/split-lines (get node "text"))))))

(defn- split-source-lines [content]
  (loop [remaining content line [] lines []]
    (if-let [item (first remaining)]
      (if (= [:lb] item)
        (recur (next remaining) [] (conj lines line))
        (recur (next remaining) (conj line item) lines))
      (conj lines line))))

(defn- shift-leading-indent [line indent]
  (let [prefix (apply str (repeat indent "　"))]
    (loop [before [] remaining line]
      (if-let [item (first remaining)]
        (cond
          (and (string? item) (empty? item))
          (recur (conj before item) (next remaining))

          (and (string? item) (string/starts-with? item prefix))
          [(into before (cons (subs ^String item indent) (next remaining))) true]

          :else [line false])
        [line false]))))

(defn- rich-source-note-hiccup [node content]
  (let [source-lines (string/split-lines (get node "text"))
        rendered-lines (take (count source-lines) (split-source-lines content))]
    (into (sourced node [:note {:type (get node "note_type")}])
          (interpose
           [:lb]
           (map (fn [source-line rendered-line]
                  (let [indent (leading-indent source-line)
                        [rendered-line shift-indent?] (if (pos? indent)
                                                        (shift-leading-indent rendered-line indent)
                                                        [rendered-line false])]
                    (into [:seg (cond-> {:type "source-line"}
                                  shift-indent?
                                  (assoc :style (str "padding-inline-start: " indent "em")))]
                          rendered-line)))
                source-lines rendered-lines)))))

(defn- flush-paragraph [acc]
  (if (seq (:current-paragraph acc))
    (let [paragraph-node (into (cond-> [:p]
                                 (:current-paragraph-attrs acc)
                                 (conj (:current-paragraph-attrs acc)))
                               (:current-paragraph acc))]
      (assoc (append-structural-child acc paragraph-node)
             :current-paragraph []
             :current-paragraph-attrs nil))
    (assoc acc :current-paragraph-attrs nil)))

(defn- normalize-layout-siblings [children]
  ;; TEI permits prose before divisions, but later prose must belong to a
  ;; division. Neutral wrappers keep those later paragraphs outside the
  ;; source's indented division without inventing an embedded text.
  (let [[before remaining] (split-with #(not= :div (first %)) children)]
    (into (vec before)
          (mapcat (fn [run]
                    (if (= :div (ffirst run)) run [(into [:div] run)])))
          (partition-by #(= :div (first %)) remaining))))

(defn- flush-division [acc]
  (if (seq (:current-division acc))
    (-> acc
        (update :body-children conj (into [:div] (normalize-layout-siblings (:current-division acc))))
        (assoc :current-division []))
    acc))

(defn- count-node [acc node-type]
  (update acc :node_counts update node-type (fnil inc 0)))

(defn- fallback-gaiji-id [span]
  (str "gaiji-" (get span "start") "-" (get span "end")))

(defn- sanitize-gaiji-reference-id
  "A character declaration's `xml:id`, derived from the gaiji's reference.

  A document mints ids in two independent schemes: this one, and
  `source-<start>-<end>` for the note carrying a source span. Both write into
  the one `xml:id` space, so they cannot collide only if one of them owns a
  prefix. `gaiji-` is that prefix, and every id from this scheme carries it,
  including the ones whose reference was already a usable name. Without the
  prefix, a reference of `source-0-6` would duplicate the id of the span note
  for extent 0 to 6, so `g/@ref` would point at two elements and the
  assertions that resolve through `id()` would have no single target."
  [reference]
  (when-let [reference (some-> reference
                               (string/replace #"^#" "")
                               string/trim
                               (string/replace #"[^A-Za-z0-9_.-]+" "-")
                               (string/replace #"-+" "-")
                               (string/replace #"^-|-$" ""))]
    (when (seq reference)
      (str "gaiji-" reference))))

(defn- normalize-gaiji-id [reference span]
  (or (sanitize-gaiji-reference-id reference)
      (fallback-gaiji-id span)))

(defn- register-char-declaration [acc declaration]
  (if (contains? (:char-declaration-ids acc) (:xml-id declaration))
    acc
    (-> acc
        (update :char_declarations conj declaration)
        (update :char-declaration-ids conj (:xml-id declaration)))))

(defn- register-witness [acc witness]
  (let [label (get witness "label")
        existing (get-in acc [:witness-ids label])]
    (if existing
      [acc existing]
      (let [[acc source] (register-source-span acc (get witness "span"))
            id (str "witness-" (inc (count (:witness-declarations acc))))]
        [(-> acc
             (assoc-in [:witness-ids label] id)
             (update :witness-declarations conj
                     (cond-> {:xml-id id :label label}
                       source (assoc :source (str "#" source)))))
         id]))))

(defn- gaiji-declaration [node]
  (let [gaiji (get node "gaiji")
        span (or (get node "source_span") (get node "span"))]
    (cond-> {:xml-id (normalize-gaiji-id (get gaiji "reference") span)}
      (contains? gaiji "unicode")
      (assoc :unicode (get gaiji "unicode"))

      (contains? gaiji "raw_marker")
      (assoc :raw-marker (get gaiji "raw_marker"))

      (seq (get gaiji "reference"))
      (assoc :source-reference (get gaiji "reference"))

      (seq (get gaiji "reference_parts"))
      (assoc :reference-parts (get gaiji "reference_parts"))

      (seq (get gaiji "unresolved_reason"))
      (assoc :unresolved-reason (get gaiji "unresolved_reason"))

      (seq (get gaiji "ids"))
      (assoc :ids (get gaiji "ids"))

      (seq (get gaiji "glyphwiki_glyph"))
      (assoc :glyphwiki (get gaiji "glyphwiki_glyph")))))

(declare render-node)

(def ^:private max-inline-depth 64)

(defn- render-inline-children [acc children depth]
  (if (>= depth max-inline-depth)
    (mark-omitted acc "emphasis-inline-depth")
    ;; Every inline position in the document reaches its children through
    ;; here, so this is where a child is marked as inline. Two renderers
    ;; otherwise close the paragraph in progress and pass their result to the
    ;; document structure; from inside a phrase that would publish the phrase
    ;; ahead of the text it followed.
    (let [outer (:inline-context? acc)
          rendered (reduce (fn [state child]
                             (-> state
                                 (count-node (get child "type"))
                                 (render-node child (inc depth))))
                           (assoc acc :inline-context? true)
                           children)]
      (assoc rendered :inline-context? outer))))

(defn- emit-ruby-anchor [acc node position]
  (let [key [(get-in node ["span" "coordinate_system"]) position]]
    (if-let [id (get (:ruby-anchors acc) key)]
      (-> acc (update :ruby-anchors dissoc key) (append-inline [:anchor {:xml/id id}]))
      acc)))

(defn- anchored-text [acc node]
  (let [^String text (get node "text")
        start (get-in node ["span" "start"])
        axis (get-in node ["span" "coordinate_system"])
        marks (into (sorted-map)
                    (for [[[coordinate position] id] (:ruby-anchors acc)
                          :when (and (= axis coordinate) (< start position (get-in node ["span" "end"])))]
                      [(- position start) id]))
        offsets (view/utf8-offsets text (keys marks))
        pending (map (fn [[byte id]] [(get offsets byte) [:anchor {:xml/id id}]]) marks)
        matcher (re-matcher #"[^\r\n]+" text)
        content (loop [remaining pending output [] seen? false]
                  (if (.find matcher)
                    (let [from (.start matcher) to (.end matcher)
                          [before remaining] (split-with #(<= (first %) from) remaining)
                          [inside remaining] (split-with #(<= (first %) to) remaining)
                          output (into (cond-> output seen? (conj [:lb])) (map second before))
                          [output cursor] (reduce (fn [[out cursor] [position anchor]]
                                                    [(cond-> out (< cursor position) (conj (.substring text cursor position))
                                                             true (conj anchor)) position])
                                                  [output from] inside)]
                      (recur remaining (cond-> output (< cursor to) (conj (.substring text cursor to))) true))
                    (into output (map second remaining))))]
    [(update acc :ruby-anchors #(apply dissoc % (map (fn [offset] [axis (+ start offset)]) (keys marks)))) content]))

(defn- render-text-node [acc node _depth]
  (let [[acc content] (cond
                        (seq (:ruby-anchors acc)) (anchored-text acc node)
                        (:source-note-context? acc)
                        [acc (interpose [:lb] (string/split (get node "text") #"\r\n|\r|\n" -1))]
                        :else [acc (whitespace/source-text->tei-inline (get node "text"))])]
    (if (and (seq content) (::source-reference node) (not (:source-note-context? acc)))
      (append-inline acc (into (sourced node [:seg]) content))
      (update acc :current-paragraph into content))))

(defn- render-ruby-node [acc node depth]
   ;; Both sides of a ruby carry their content in either of two places: the
   ;; plain string the converter derived, or the child nodes it kept when that
   ;; string could not hold them. Whether a side is present is therefore
   ;; decided by rendering it, not by reading one of the two places: a base
   ;; made only of kinds that contribute no visible text (`raw`, `kunten`,
   ;; `editorial_note`, `layout_break`, `figure`) has an empty string beside
   ;; the children that hold it.
   ;;
   ;; A side that renders to nothing cannot become an `rb` or `rt`, because the
   ;; profile has no empty one. Only a ruby that is then dropped incurs a
   ;; rendering whose result is discarded.
  (let [ruby (get node "ruby")
        attrs (cond-> {:type "furigana"}
                (get ruby "direction")
                (assoc :rend (get ruby "direction")))
        side (fn [state children text]
               (if (seq children)
                 (render-inline-children (assoc state :current-paragraph []) children depth)
                 (assoc state :current-paragraph
                        (if (present-ruby-text? text) [text] []))))
        before (:current-paragraph acc)
        base (side acc (get node "inline_children") (get ruby "base"))
        reading (side base (get node "reading_children") (get ruby "reading"))]
    (if (and (seq (:current-paragraph base)) (seq (:current-paragraph reading)))
      (append-inline (assoc reading :current-paragraph before)
                     (sourced node [:ruby attrs
                                    (into [:rb] (:current-paragraph base))
                                    (into [:rt] (:current-paragraph reading))]))
      (mark-omitted acc "ruby"))))

(defn- render-ruby-group-node [acc node depth]
  (let [ordinal (inc (get acc :ruby-group-count 0))
        prefix (str "ruby-base-" ordinal "-")
        anchor-id #(str prefix %)
        readings (get node "readings")
        boundaries (into #{} (mapcat #(vals (get % "base_range"))) readings)
        start (get-in node ["span" "start"])
        axis (get-in node ["span" "coordinate_system"])
        anchors (into {} (map (fn [position] [[axis (+ start position)] (anchor-id position)])) boundaries)
        before (:current-paragraph acc)
        outer-anchors (:ruby-anchors acc)
        base (render-inline-children (assoc acc :current-paragraph [] :ruby-anchors anchors :ruby-group-count ordinal)
                                     (get node "inline_children") depth)
        _ (when (seq (:ruby-anchors base))
            (throw (ex-info "Ruby endpoints do not locate base text" {:endpoints (:ruby-anchors base)})))
        [state rendered] (reduce
                          (fn [[state rendered] reading]
                            (let [[state reading] (register-source state reading)
                                  state (assoc state :current-paragraph [])
                                  state (if (seq (get reading "reading_children"))
                                          (render-inline-children state (get reading "reading_children") depth)
                                          (append-inline state (get reading "reading")))
                                  attrs (cond-> {:from (str "#" (anchor-id (get-in reading ["base_range" "start"])))
                                                 :to (str "#" (anchor-id (get-in reading ["base_range" "end"])))}
                                          (get reading "direction") (assoc :place (get reading "direction")))]
                              [state (conj rendered (sourced reading (into [:rt attrs] (:current-paragraph state))))]))
                          [(dissoc base :ruby-anchors) []] readings)]
    (append-inline (assoc state :current-paragraph before :ruby-anchors outer-anchors)
                   (sourced node (into [:ruby {:type "furigana"} (into [:rb] (:current-paragraph base))] rendered)))))

(defn- render-gaiji-node [acc node _depth]
  (let [identity (gaiji-declaration node)
        base-id (:xml-id identity)
        id (or (get (:gaiji-identities acc) identity)
               (if-not (contains? (:char-declaration-ids acc) base-id)
                 base-id
                 (loop [ordinal 0]
                   (let [candidate (str base-id "-at-" (or (get-in node ["source_span" "start"])
                                                           (get-in node ["span" "start"]) 0)
                                        (when (pos? ordinal) (str "-" ordinal)))]
                     (if (contains? (:char-declaration-ids acc) candidate)
                       (recur (inc ordinal)) candidate)))))
        declaration (assoc identity :xml-id id)]
    (-> acc
        (assoc-in [:gaiji-identities identity] id)
        (register-char-declaration declaration)
        (append-inline (sourced node (cond-> [:g {:ref (str "#" id)}]
                                       (seq (:unicode declaration)) (conj (:unicode declaration))
                                       (and (not (seq (:unicode declaration)))
                                            (seq (get-in node ["gaiji" "image_or_glyph_fallback"])))
                                       (conj (get-in node ["gaiji" "image_or_glyph_fallback"]))))))))

(defn- instruction-operands [acc element operands & [role]]
  (reduce (fn [[acc element] operand]
            (let [[acc operand] (register-source acc operand)]
              [acc (conj element (sourced operand [:snh/formatting-operand
                                                   (cond-> {}
                                                     role (assoc :role role))
                                                   (get operand "text")]))]))
          [acc element] operands))

(defn- instruction-selector [acc selector]
  (let [predecessor (get selector "predecessor")
        attrs (cond-> {:kind (get selector "kind")}
                (get selector "ordinal") (assoc :ordinal (get selector "ordinal"))
                (get selector "lines") (assoc :lines (get selector "lines"))
                predecessor (assoc :predecessor (get predecessor "kind")))]
    (if-let [enclosing (get selector "enclosing")]
      (let [[acc element] (instruction-operands acc [:snh/formatting-selector attrs]
                                                [enclosing] "enclosing")]
        (instruction-operands acc element (get selector "operands") "selected"))
      (instruction-operands acc [:snh/formatting-selector attrs]
                            (or (get selector "operands") (get predecessor "operands"))))))

(defn- instruction-layout-attributes [layout]
  (let [attrs (reduce-kv (fn [attrs key value]
                           (if (or (nil? value) (= key "decoration"))
                             attrs
                             (assoc attrs (case key
                                            "kind" :layout-kind
                                            "source" :source-kind
                                            (keyword (string/replace key "_" "-"))) value)))
                         {} layout)]
    (if-let [decoration (get layout "decoration")]
      (assoc attrs :decoration-kind (get decoration "kind")
             :decoration-position (get decoration "position"))
      attrs)))

(defn- instruction-effect [acc effect]
  (case (get effect "kind")
    "attribute" [acc [:snh/formatting-effect (assoc (instruction-layout-attributes (get effect "layout"))
                                                    :kind "attribute")]]
    "upper-relative" (let [[acc element] (instruction-operands acc [:snh/formatting-effect {:kind "upper-relative"}]
                                                               [(get effect "base")])]
                       (if-let [span (get effect "base_target")]
                         (let [[acc base] (register-source acc {"source_span" span})]
                           [acc (conj element (sourced base [:snh/formatting-base]))])
                         [acc element]))
    "reduced-point-lines"
    (let [placement (get effect "placement")
          attrs (cond-> {:kind "reduced-point-lines"
                         :placement (get placement "kind")
                         :lines (get effect "lines")}
                  (get effect "parentheses_lines")
                  (assoc :parentheses-lines (get effect "parentheses_lines")))]
      (if (= "below" (get placement "kind"))
        (instruction-operands acc [:snh/formatting-effect attrs]
                              [(get placement "anchor")] "anchor")
        (let [[acc element] (instruction-operands acc [:snh/formatting-effect attrs]
                                                  [(get placement "preceding")] "preceding")]
          (instruction-operands acc element [(get placement "following")] "following"))))
    "horizontal-center" [acc [:snh/formatting-effect
                              {:kind "horizontal-center"
                               :lines (get effect "lines")}]]
    "line-setting" [acc [:snh/formatting-effect
                         (cond-> {:kind "line-setting"
                                  :lines (get effect "lines")}
                           (get effect "parentheses_lines")
                           (assoc :parentheses-lines (get effect "parentheses_lines")))]]
    "relative-placement"
    (let [anchor (get effect "anchor")
          add-endpoint (fn [[acc element] role endpoint]
                         (let [[acc element] (instruction-operands acc element
                                                                   [(get endpoint "operand")] role)]
                           (if-let [target (get endpoint "target")]
                             (let [[acc source] (register-source acc {"source_span" target})]
                               [acc (conj element
                                          (sourced source [:snh/formatting-anchor
                                                           {:role role
                                                            :day (get endpoint "day")}]))])
                             [acc element])))
          initial [:snh/formatting-effect {:kind "relative-placement"
                                           :relation (get effect "relation")
                                           :anchor-kind (get anchor "kind")}]
          start (get anchor "start")
          end (get anchor "end")]
      (-> [acc initial]
          (add-endpoint "range-start" start)
          (add-endpoint "range-end" end)))
    "rotation" [acc [:snh/formatting-effect
                     {:kind "rotation"
                      :degrees (get effect "degrees")}]]
    "layout-policy" [acc [:snh/formatting-effect
                          {:kind "layout-policy"
                           :indent (get effect "indent")
                           :horizontal (get effect "horizontal")
                           :align-end (get effect "align_end")}]]
    "letter-spacing" [acc [:snh/formatting-effect
                           {:kind "letter-spacing"
                            :amount (get effect "amount")}]]
    "line-start-alignment"
    (let [relation (get effect "relation")
          relation-kind (get relation "kind")
          initial [:snh/formatting-effect
                   (cond-> {:kind "line-start-alignment"
                            :relation-kind relation-kind}
                     (= "relative-line" relation-kind)
                     (assoc :lines-before (get relation "lines_before")))]]
      (case relation-kind
        "relative-line"
        (if-let [target (get relation "target")]
          (let [[acc reference] (register-source-span acc target)]
            [acc (assoc-in initial [1 :reference-source] (str "#" reference))])
          [acc initial])
        "collective" (instruction-operands acc initial [(get relation "members")] "members")))
    "heading-typography" [acc [:snh/formatting-effect
                               {:kind "heading-typography"
                                :variant (get effect "variant")}]]
    "rule-style" [acc [:snh/rule-presentation
                       {:variant (get effect "style")}]]
    "ligature" (instruction-operands acc [:snh/formatting-effect {:kind "ligature"}]
                                     (get effect "components") "component")
    "kunten-policy"
    (let [[acc element] (instruction-operands
                         acc
                         [:snh/formatting-effect
                          {:kind "kunten-policy"
                           :scope (get effect "scope")
                           :repeat-following (get effect "repeat_following")
                           :vertical-rule-right (get effect "vertical_rule_right")}]
                         [(get effect "marks")] "marks")]
      (reduce (fn [[acc element] exclusion]
                (let [[acc excluded] (instruction-operands
                                      acc [:snh/kunten-exclusion]
                                      [(get exclusion "enclosing")] "enclosing")
                      [acc excluded] (instruction-operands
                                      acc excluded
                                      [(get exclusion "mark")] "mark")]
                  [acc (conj element excluded)]))
              [acc element]
              (get effect "exclusions")))))

(defn- instruction-selection [acc selection]
  (let [[acc scope] (register-source acc {"source_span" (get selection "scope")})]
    (reduce (fn [[acc element] span]
              (let [[acc target] (register-source acc {"source_span" span})]
                [acc (conj element (sourced target [:snh/formatting-target]))]))
            [acc (sourced scope [:snh/formatting-selection])]
            (get selection "targets"))))

(defn- formatting-instruction-element [acc node]
  (let [[acc instruction]
        (reduce (fn [[acc instruction] operation]
                  (let [[acc operation] (register-source acc operation)
                        [acc selector] (instruction-selector acc (get operation "selector"))
                        [acc effect] (instruction-effect acc (get operation "effect"))
                        [acc selection] (if-let [selection (get operation "selection")]
                                          (instruction-selection acc selection)
                                          [acc nil])]
                    [acc (conj instruction (sourced operation (cond-> [:snh/formatting-operation selector effect]
                                                                selection (conj selection))))]))
                [acc [:snh/formatting-instruction {:original (get node "raw")}]]
                (get node "operations"))]
    [acc (sourced node [:note {:type "formatting-instruction"} instruction])]))

(defn- figure-note-content [node]
  (let [text (get node "text")
        description (get node "figure_description")]
    (if-not description
      [text]
      (let [start (string/index-of text description)]
        (when-not start
          (throw (ex-info "figure description is absent from its source statement"
                          {:text text :description description})))
        [(subs text 0 start)
         [:seg {:type "figure-description"} description]
         (subs text (+ start (count description)))]))))

(defn- base-correction-content [node base-source]
  (let [text (get node "text")
        base-text (get node "base_text")
        start (string/index-of text base-text)]
    (when-not start
      (throw (ex-info "base correction reading is absent from its source statement"
                      {:text text :base-text base-text})))
    [(subs text 0 start)
     [:sic {:source (str "#" (::source-reference base-source))} base-text]
     (subs text (+ start (count base-text)))]))

(defn- render-formatting-instruction-node [acc node _depth]
  (let [[acc element] (formatting-instruction-element acc node)]
    (append-inline acc element)))

(defn- render-editor-note-node [acc node depth]
  (let [before (:current-paragraph acc)
        annotation (when-let [children (get node "annotation_children")]
                     (render-inline-children (assoc acc :current-paragraph []) children depth))
        acc (if annotation (assoc annotation :current-paragraph before) acc)
        proposed (when-let [children (get node "proposed_children")]
                   (render-inline-children (assoc acc :current-paragraph []) children depth))
        acc (if proposed (assoc proposed :current-paragraph before) acc)
        [acc base-source] (if-let [span (get node "base_source_span")]
                            (register-source acc {"source_span" span})
                            [acc nil])
        element (sourced node
                         (if-let [kind (get node "note_kind")]
                           (into [:note (cond-> {:type kind}
                                          (= kind "indentation-question") (assoc :subtype "possible-omission")
                                          (= kind "uncertain-correction") (assoc :type "correction"
                                                                                 :subtype "uncertain")
                                          (= kind "inputter-note-reference")
                                          (assoc :type "inputter-note-reference"
                                                 :n (str (get node "note_number")))
                                          (= kind "figure-insertion")
                                          (cond-> (get node "figure_kind")
                                            (assoc :subtype (get node "figure_kind"))
                                            (get node "source_locator")
                                            (assoc :n (get node "source_locator"))
                                            (get node "page_position")
                                            (assoc :place (get node "page_position")))
                                          (= "uncertain" (get node "certainty"))
                                          (assoc :cert "low")
                                          (get node "base_text")
                                          (assoc :subtype "correction")
                                          (and (contains? #{"base-edition"
                                                            "original-text"
                                                            "uncertain-correction"
                                                            "source-legibility"} kind)
                                               (get node "source_locator"))
                                          (assoc :n (get node "source_locator")))]
                                 (cond
                                   (get node "base_text")
                                   (base-correction-content node base-source)
                                   (= kind "figure-insertion")
                                   (figure-note-content node)
                                   (= kind "external-table-reference")
                                   [[:ref {:target (get node "source_filename")} (get node "text")]]
                                   (= kind "indentation-question")
                                   [[:measure {:type "indentation" :quantity (get node "indentation_characters")
                                               :unit "character"} "一字下げ"]
                                    "忘れか？" [:ref {:type "source-locator"} (get node "source_locator")]]
                                   (= kind "uncertain-correction")
                                   (cond-> [(into [:corr {:cert "low"}]
                                                  (or (:current-paragraph proposed)
                                                      [(get node "proposed_text")]))]
                                     (get node "source_locator")
                                     (conj [:ref {:type "source-locator"}
                                            (get node "source_locator")]))
                                   annotation (:current-paragraph annotation)
                                   :else [(get node "text")]))
                           (let [note (get node "note")]
                             [:note (cond-> {:type (get note "category")}
                                      (get note "resolution") (assoc :subtype (get note "resolution")))
                              (get note "raw")])))
        [acc element] (if-let [span (get node "subject_source_span")]
                        (let [[acc subject] (register-source acc {"source_span" span})]
                          [acc (assoc-in element [1 :corresp]
                                         (str "#" (::source-reference subject)))])
                        [acc element])]
    (if-let [targets (get node "target_source_spans")]
      (let [[acc references] (reduce (fn [[acc references] span]
                                       (let [[acc target] (register-source acc {"source_span" span})]
                                         [acc (conj references (str "#" (::source-reference target)))]))
                                     [acc []] targets)
            element (assoc-in element [1 :target] (string/join " " references))]
        (if-let [closing-span (get node "closing_source_span")]
          (let [[acc closing] (register-source acc {"source_span" closing-span})]
            (append-inline acc (update-in element [1 :source] str " #" (::source-reference closing))))
          (append-inline acc element)))
      (append-inline acc element))))

(defn- render-kunten-node [acc node depth]
  (let [kind (get node "kunten_kind")
        before (:current-paragraph acc)
        annotation (when-let [children (get node "annotation_children")]
                     (render-inline-children (assoc acc :current-paragraph []) children depth))
        acc (if annotation (assoc annotation :current-paragraph before) acc)]
    (append-inline acc (sourced node (into [:note {:type "kunten" :subtype kind
                                                   :rend (case kind "return-mark" "subscript" "okurigana" "superscript")}]
                                           (if annotation
                                             (:current-paragraph annotation)
                                             [(get node "text")]))))))

(defn- render-gap-node [acc node _depth]
  (append-inline acc
                 [:gap (cond-> {:reason (get node "reason")}
                         (get node "quantity") (assoc :quantity (get node "quantity")
                                                      :unit (get node "unit"))
                         (get node "extent") (assoc :extent (get node "extent")))]))

(defn- render-annotated-text-node [acc node depth]
  (let [before (:current-paragraph acc)
        principal (render-inline-children (assoc acc :current-paragraph [])
                                          (get node "inline_children") depth)
        annotation (render-inline-children (assoc principal :current-paragraph [])
                                           (get node "annotation_children") depth)
        kind (get node "note_kind")
        attributes (if (contains? #{"annotation-number" "author-note"} kind)
                     {:type "source-role" :subtype kind}
                     (cond-> {:type kind}
                       (get node "position") (assoc :place (get node "position"))
                       ;; The side and the anchor are separate axes the source
                       ;; states independently, so they occupy separate
                       ;; attributes rather than one combined value.
                       (get node "anchor") (assoc :subtype (get node "anchor"))))
        note (into [:note attributes]
                   (:current-paragraph annotation))]
    (append-inline (assoc annotation :current-paragraph before)
                   (sourced node (conj (into [:seg {:type "annotated-text"}]
                                             (:current-paragraph principal)) note)))))

(defn- render-source-realization-node [acc node _depth]
  (let [[acc annotation] (register-source acc {"source_span" (get node "annotation_span")})
        reference (::source-reference annotation)]
    (append-inline acc (sourced node [:choice (cond-> {} reference (assoc :corresp (str "#" reference)))
                                      [:orig (get node "source")]
                                      [:reg (get node "text")]]))))

(defn- render-base-text-variant-node [acc node depth]
  (let [before (:current-paragraph acc)
        witness-kind (or (get-in node ["variant" "witness_kind"]) "base-edition")
        reading-type (case witness-kind
                       "base-edition" "base-text"
                       "first-publication" "first-publication"
                       "original-text" "original-text")
        rendered (if (seq (get node "inline_children"))
                   (render-inline-children (assoc acc :current-paragraph [])
                                           (get node "inline_children") depth)
                   (assoc acc :current-paragraph [(get node "text")]))
        witness (render-inline-children (assoc rendered :current-paragraph [])
                                        (get-in node ["variant" "base_children"]) depth)]
    (append-inline (assoc witness :current-paragraph before)
                   (sourced node [:app {:type "base-text-variant"}
                                  (into [:lem] (:current-paragraph rendered))
                                  (into [:rdg (cond-> {:type reading-type}
                                                (= "" (get-in node ["variant" "base_text"]))
                                                (assoc :subtype "omission"))]
                                        (:current-paragraph witness))]))))

(defn- render-text-apparatus-node [acc node depth]
  (let [before (:current-paragraph acc)
        principal (render-inline-children (assoc acc :current-paragraph [])
                                          (get node "inline_children") depth)
        [rendered readings]
        (reduce
         (fn [[acc elements] reading]
           (let [[acc witness-ids]
                 (reduce (fn [[acc ids] witness]
                           (let [[acc id] (register-witness acc witness)]
                             [acc (conj ids id)]))
                         [acc []] (get reading "witnesses"))
                 reading-result (render-inline-children (assoc acc :current-paragraph [])
                                                        (get reading "reading_children") depth)]
             [reading-result
              (conj elements
                    (into [:rdg {:wit (string/join " " (map #(str "#" %) witness-ids))}]
                          (:current-paragraph reading-result)))]))
         [principal []] (get node "readings"))]
    (append-inline (assoc rendered :current-paragraph before)
                   (sourced node
                            (into [:app {:type "text-apparatus"}
                                   (into [:lem] (:current-paragraph principal))]
                                  readings)))))

(defn- render-inline-wrapper [acc children text depth wrapper]
  (if (seq children)
    ;; The wrapped fragment is what the children put in a paragraph of their
    ;; own, which is how every other renderer here composes one. A slice of
    ;; the paragraph in progress by index would assume that a child only
    ;; appends to that paragraph; a child that closes the paragraph would put
    ;; the slice past its end.
    (let [before (:current-paragraph acc)
          rendered (render-inline-children (assoc acc :current-paragraph [])
                                           children depth)]
      (append-inline (assoc rendered :current-paragraph before)
                     (into wrapper (:current-paragraph rendered))))
    (append-inline acc (conj wrapper text))))

(defn- render-warichu-node [acc node depth]
  (let [wrapper (sourced node [:seg {:type "warichu" :rend "two-line"}])]
    (if (contains? node "inline_children")
      (render-inline-wrapper acc (get node "inline_children") (get node "text") depth wrapper)
      (let [before (:current-paragraph acc)
            upper (render-inline-wrapper (assoc acc :current-paragraph [])
                                         (get node "upper_children") "" depth [:seg {:type "upper"}])
            lower (render-inline-wrapper upper (get node "lower_children") "" depth [:seg {:type "lower"}])]
        (append-inline (assoc lower :current-paragraph before)
                       (into wrapper (:current-paragraph lower)))))))

(defn- render-emphasis-node [acc node depth]
  (let [locator (get-in node ["decoration" "source_locator"])]
    (render-inline-wrapper acc
                           (seq (get node "inline_children"))
                           (get node "text")
                           depth
                           (sourced node [:hi (cond-> {:rend (emphasis-rend node)}
                                                locator (assoc :snh/decoration-source-locator locator))]))))

(defn- render-accent-node [acc node _depth]
  (append-inline acc
                 (sourced node
                          [:hi (cond-> {:rend "accent" :snh/accent-code (get node "code")}
                                 (get node "name") (assoc :snh/accent-name (get node "name"))
                                 (get node "source_spelling")
                                 (assoc :snh/source-marker (get node "source_spelling")))
                           (get node "text")])))

(defn- render-layout-span-node [acc node depth]
  (let [layout (get node "layout")
        layouts (layout-attributes layout)
        rends (mapv inline-layout-rend layouts)
        geometry? (some #(#{"chitsuki" "line-jisage"} (get % "kind")) layouts)]
    (if (every? some? rends)
      (let [rend (string/join " " rends)
            params (if (vector? layout)
                     (not-empty (string/join ";" (mapcat (fn [attribute]
                                                           (when-let [params (layout-params attribute)]
                                                             (map #(str (get attribute "kind") "." %)
                                                                  (string/split params #";"))))
                                                         layouts)))
                     (layout-params layout))]
        (render-inline-wrapper acc
                               (seq (get node "inline_children"))
                               (get node "text")
                               depth
                               (sourced node [(if geometry? :seg :hi) (cond-> {:rend rend}
                                                                        (seq layouts)
                                                                        (assoc :snh/layout-kind (string/join " " (map #(get % "kind") layouts)))

                                                                        params
                                                                        (assoc :snh/layout-params params))])))
      (mark-omitted acc "layout-span"))))

(defn- heading-attrs [node]
  (cond-> {:n (str (get node "level"))}
    (some? (get node "indent"))
    (assoc :style (str "padding-inline-start: " (get node "indent") "em"))))

(defn- render-heading-node [acc node depth]
   ;; `dogyo` and `mado` state in the source that a heading is in the run of
   ;; text rather than above it. A heading nested inside inline content has
   ;; the same position, so it is read the same way. The converter produces
   ;; this case: its inline child projection maps the `heading`
   ;; kind, and the style it copies over is any of `normal`, `dogyo`, `mado`.
  (if (or (:inline-context? acc) (#{"dogyo" "mado"} (get node "style")))
    (render-inline-wrapper acc
                           (seq (get node "inline_children"))
                           (get node "text")
                           depth
                           (sourced node [:seg (assoc (heading-attrs node)
                                                      :type "heading"
                                                      :rend (get node "style"))]))
    (let [children (seq (get node "inline_children"))
          base (-> acc flush-paragraph flush-division)]
      (if children
        (let [scratch (assoc base :current-paragraph [])
              rendered (render-inline-children scratch children depth)
              head-fragment (:current-paragraph rendered)]
          (-> rendered
              (assoc :current-paragraph [])
              (update :current-division conj
                      (into (sourced node [:head (heading-attrs node)])
                            head-fragment))))
        (update base :current-division conj
                (sourced node [:head (heading-attrs node)
                               (get node "text")]))))))

(defn- render-indentation-node [acc node _depth]
  (if (present-text? (get node "text"))
    (append-inline acc
                   (sourced node [:seg {:type "indentation"
                                        :n (str (get node "depth"))}
                                  (get node "text")]))
    (mark-omitted acc "indentation")))

(defn- break-layout-params [node]
  (not-empty (string/join ";"
                          (cond-> []
                            (get node "destination_side")
                            (conj (str "destination-side=" (get node "destination_side")))
                            (get node "following_placement")
                            (conj (str "following-placement=" (get node "following_placement")))
                            (some? (get node "following_indent"))
                            (conj (str "following-indent=" (get node "following_indent")))))))

(defn- render-page-break-node [acc node _depth]
  (let [params (break-layout-params node)]
    (append-inline acc
                   (sourced node (cond-> [(if (= "column-break" (get node "type")) :cb :pb) {}]
                                   (#{"kaicho" "kaimihiraki"} (get node "marker"))
                                   (assoc-in [1 :rend] (get node "marker"))
                                   (some? (get node "page_number"))
                                   (assoc-in [1 :n] (get node "page_number"))
                                   (some? (get node "column_number"))
                                   (assoc-in [1 :n] (get node "column_number"))
                                   params
                                   (assoc-in [1 :snh/layout-params] params))))))

(defn- render-line-break-node [acc node _depth]
  (append-inline acc (sourced node [:lb])))

(defn- render-image-node [acc node depth]
  (let [before (:current-paragraph acc)
        rendered (render-inline-children (assoc acc :current-paragraph [])
                                         (get node "caption_reference_children") depth)
        description (render-inline-children (assoc rendered :current-paragraph [])
                                            (get node "description_children") depth)
        annotations (render-inline-children (assoc description :current-paragraph [])
                                            (get node "annotation_children") depth)
        ;; A source that writes a bare stem where the asset name belongs
        ;; identifies no file. The parser passes that text as `src_source` with a null
        ;; `src`, and a `graphic` built from it would assert an asset the
        ;; source never supplied, so the figure carries the text as a note and
        ;; makes no url claim at all.
        graphic (when-let [url (get node "src")]
                  [:graphic (cond-> {:url url}
                              (some? (get node "width")) (assoc :width (str (get node "width") "px"))
                              (some? (get node "height")) (assoc :height (str (get node "height") "px")))])
        figure (cond-> [:figure (cond-> {}
                                  (get node "number") (assoc :n (get node "number"))
                                  (get node "figure_class")
                                  (assoc :snh/figure-class (get node "figure_class")))]
                 graphic (conj graphic)
                 (and (present-text? (get node "alt")) (empty? (:current-paragraph description))) (conj [:figDesc (get node "alt")])
                 (seq (:current-paragraph description)) (conj (into [:note {:type "image-description"}] (:current-paragraph description)))
                 (seq (:current-paragraph rendered)) (conj (into [:note {:type "caption-reference"}] (:current-paragraph rendered)))
                 (get node "src_source") (conj [:note {:type "uninterpreted-image-source"} (get node "src_source")])
                 (get node "description_source") (conj [:note {:type "uninterpreted-image-description"} (get node "description_source")])
                 (get node "caption_source") (conj [:note {:type "uninterpreted-caption-reference"} (get node "caption_source")])
                 (and (get node "dimensions_source") (nil? (get node "width")))
                 (conj [:note {:type "image-dimensions"} (get node "dimensions_source")]))]
    (append-inline (assoc annotations :current-paragraph before)
                   (sourced node (into figure (:current-paragraph annotations))))))

(defn- render-caption-node [acc node depth]
  (render-inline-wrapper acc (get node "inline_children") (get node "text") depth
                         (sourced node [:seg {:type "caption"}])))

(defn- render-quote-node [acc node depth]
   ;; The parser IR's `quote` node is one quotation delimiter, carrying
   ;; `marker_type` open or close and the single character as its text. It is
   ;; a marker, not a container: the quoted passage is a sibling. Rendering it
   ;; as TEI `<quote>` would state that the delimiter is the quotation and the
   ;; speech is not, the opposite of the source's meaning. The character is
   ;; source text and carries the information itself, so it is emitted the
   ;; same way any other text run is.
  (if (present-text? (get node "text"))
    (render-text-node acc node depth)
    (mark-omitted acc "quote")))

(defn- render-source-note-node [acc node depth]
  (cond
    (not (or (present-text? (get node "text")) (seq (get node "inline_children"))))
    (mark-omitted acc "source-note")

     ;; ［＃本文終わり］. TEI already states where the body ends: the colophon is
     ;; a `div type="source"` and everything before it is the body. Emitting
     ;; the marker as well would state that boundary twice, the second time as
     ;; a note about the source edition, which it is not. The line stays in the
     ;; parser IR and the source inventory counts it as a region boundary, so
     ;; it remains recorded.
    (= "body-end-boundary" (get node "note_type"))
    (mark-omitted acc "body-end-boundary")

    :else
     ;; `front` and `back` put a note in the document's front or back matter,
     ;; which a note inside a phrase cannot mean: placing it there would end the
     ;; paragraph that phrase belongs to. Inline, the note stays at its
     ;; position. The current converter emits source notes only as blocks, so
     ;; this shape arrives from parser IR written by hand or by another
     ;; producer rather than from the corpus.
    (let [before (:current-paragraph acc)
          outer (:source-note-context? acc)
          rendered (when (seq (get node "inline_children"))
                     (render-inline-children (assoc acc :current-paragraph [] :source-note-context? true)
                                             (get node "inline_children") depth))
          note (if rendered
                 (rich-source-note-hiccup node (:current-paragraph rendered))
                 (source-note-hiccup node))
          acc (if rendered (assoc rendered :current-paragraph before :source-note-context? outer) acc)]
      (case (if (and (:inline-context? acc)
                     (#{"front" "back"} (get node "placement")))
              "body"
              (get node "placement"))
        "front" (-> acc
                    flush-paragraph
                    flush-division
                    (update :front-notes conj note))
        "body" (append-inline acc note)
        "back" (-> acc
                   flush-paragraph
                   flush-division
                   (update :back-notes conj note))
        (mark-omitted acc "source-note")))))

(def ^:private node-renderers
  {"text" render-text-node
   "ruby" render-ruby-node
   "ruby-group" render-ruby-group-node
   "gaiji" render-gaiji-node
   "editor-note" render-editor-note-node
   "formatting-instruction" render-formatting-instruction-node
   "base-text-variant" render-base-text-variant-node
   "text-apparatus" render-text-apparatus-node
   "annotated-text" render-annotated-text-node
   "gap" render-gap-node
   "emphasis" render-emphasis-node
   "accent" render-accent-node
   "layout-span" render-layout-span-node
   "heading" render-heading-node
   "indentation" render-indentation-node
   "page-break" render-page-break-node
   "column-break" render-page-break-node
   "line-break" render-line-break-node
   "image" render-image-node
   "caption" render-caption-node
   "quote" render-quote-node
   "source-note" render-source-note-node
   "kunten" render-kunten-node
   "iteration-mark" render-source-realization-node
   "supplied-diacritic" render-source-realization-node
   "warichu" render-warichu-node})

(defn- render-node
  ([acc node] (render-node acc node 0))
  ([acc node depth]
   (let [acc (emit-ruby-anchor acc node (get-in node ["span" "start"]))
         node-type (get node "type")
         [acc node] (register-source acc node)]
     (if-let [render-node-fn (get node-renderers node-type)]
       (emit-ruby-anchor (render-node-fn acc node depth) node (get-in node ["span" "end"]))
       (throw (ex-info "Unsupported TEI parser-IR node type"
                       {:node-type node-type}))))))

(defn- emendation-span
  "An emendation's extent as a node span. `end_line` is dropped when it equals
  `line`, since the `source-span` note carries only the line an extent begins
  on; an emendation that spans lines throws."
  [emendation]
  (let [{:strs [line end_line] :as span} (get emendation "source_span")]
    (when-not (or (nil? end_line) (= line end_line))
      (throw (ex-info "An emendation must lie within one source line"
                      {:emendation emendation})))
    (dissoc span "end_line")))

(defn- initial-acc [emendations]
  (reduce
   (fn [acc emendation]
     (first (register-source-span acc (emendation-span emendation))))
   {:body-children []
    :current-division []
    :current-paragraph []
    :current-paragraph-attrs nil
    :inline-context? false
    :front-notes []
    :back-notes []
    :source-spans (sorted-map)
    :char_declarations []
    :char-declaration-ids #{}
    :witness-declarations []
    :witness-ids {}
    :gaiji-identities {}
    :node_counts {}
    :omitted []}
   emendations))

(defn- render-node-seq [acc nodes]
  (reduce (fn [acc node]
            (-> acc
                (count-node (get node "type"))
                (render-node node)))
          acc
          nodes))

(def ^:private recoverable-span-keys
  "Everything a `source-span` note's reference and its declaration account for.
  `start` and `end` are in the id, `coordinate_system` is fixed for every span
  in the corpus, and `line` becomes `@n`. Any other key would be dropped by the
  reduction, so its presence is an error rather than a loss."
  ["start" "end" "line" "coordinate_system"])

(defn- source-span-note
  "One extent of the authenticated source text, as an empty note.

  Everything the extent is made of is already elsewhere in the document.
  `start` and `end` are in the id a consumer must parse anyway to resolve the
  reference; `coordinate_system` is `decoded_utf8` for every span, because
  `source-reference` rejects any other; and the primary text hash is declared
  once in the header's `sourceDesc`. Only the line number is new, so only the
  line number is carried, in `@n`. The `refsDecl` in `encodingDesc` states how
  to read the id.

  A span carrying anything the reference does not account for throws rather
  than being reduced. This is the one place the reduction could lose
  information, and a lossy reduction would publish an extent that does not
  state the span it was derived from."
  [id span]
  (let [{:strs [start end line coordinate_system]} span]
    (when-not (and (= "decoded_utf8" coordinate_system)
                   (= id (str "source-" start "-" end))
                   (or (nil? line) (integer? line))
                   (empty? (apply dissoc span recoverable-span-keys)))
      (throw (ex-info "source span is not recoverable from its reference"
                      {:reason :irreducible-source-span :id id :span span})))
    ;; A span whose line is unknown carries no `n`, rather than a placeholder
    ;; a consumer would have to recognise as one.
    [:note (cond-> {:type "source-span" :xml/id id}
             line (assoc :n (str line)))]))

(defn- tei-source-div [notes]
  (into [:div {:type "source"}] notes))

(defn- tei-text [result]
  (let [body-children (if (seq (:body-children result))
                        (:body-children result)
                        [[:p [:gap {:reason "missing"}]]])]
    (into [:text]
          (concat
           (when (seq (:front-notes result))
             [[:front (tei-source-div (:front-notes result))]])
           [(into [:body] body-children)]
           (when (seq (:back-notes result))
             [[:back (tei-source-div (:back-notes result))]])))))

(defn- finalize-result [result]
  (let [result (-> result
                   flush-paragraph
                   flush-division)
        result (update result :back-notes into
                       (map (fn [[id span]] (source-span-note id span)))
                       (:source-spans result))]
    {:body (tei-text result)
     :char_declarations (:char_declarations result)
     :witness_declarations (:witness-declarations result)
     :node_counts (:node_counts result)
     :omitted (:omitted result)}))

(defn- paragraph-range [paragraph]
  (get paragraph "node_range"))

(defn- validate-paragraph-ranges! [nodes paragraphs]
  (loop [remaining paragraphs
         prior-end (Long/valueOf 0)]
    (when-let [paragraph (first remaining)]
      (let [{start "start" end "end"} (paragraph-range paragraph)]
        (when-not (and (integer? start)
                       (integer? end)
                       (<= 0 start end (count nodes))
                       (<= prior-end start))
          (throw (ex-info "Invalid parser-IR paragraph node_range"
                          {:paragraph paragraph
                           :nodes-count (count nodes)
                           :prior-end prior-end})))
        (recur (rest remaining) end)))))

(defn- paragraph-render-inputs [nodes paragraph hanging-offset]
  (let [{start "start" end "end"} (paragraph-range paragraph)
        first-index (first (drop-while
                            (fn [index]
                              (let [node (nth nodes index)]
                                (or (contains? #{"editor-note" "formatting-instruction"} (get node "type"))
                                    (and (= "text" (get node "type"))
                                         (empty? (get node "text"))))))
                            (range start end)))
        first-node (when first-index (nth nodes first-index))
        indent (if (and (= "body" (get paragraph "role"))
                        (= "text" (get first-node "type")))
                 (leading-indent (get first-node "text"))
                 0)]
    [(if (pos? indent)
       (update-in nodes [first-index "text"] subs indent)
       nodes)
     (cond-> (paragraph-attrs paragraph)
       (pos? indent) (assoc :style (str "text-indent: " (+ hanging-offset indent) "em"))
       (pos? indent) (update :rend #(string/join " " (remove string/blank? [% (str "first-line-indent(" indent ")")]))))]))

(defn- paragraph-source-attrs
  "`attrs` with the paragraph's own extent as `@source`, registered like a
  node's. A paragraph with no inline markup holds one `seg` of the same
  extent; the paragraph still states it, so a block is addressable as a
  block and not only through its first child."
  [acc paragraph attrs]
  (let [[acc reference] (register-source-span acc (get paragraph "source_span"))]
    [acc (cond-> attrs reference (assoc :source (str "#" reference)))]))

(defn- layout-block-attrs [block]
  (let [indent (get block "indent")
        placement (get block "relative_placement")
        anchor (get placement "anchor_span")
        attributes (layout-attributes (get block "typography"))
        frame-style (some (fn [attribute]
                            (when (= "keigakomi" (get attribute "kind"))
                              (case (get attribute "border")
                                ("rule" "box" "circle") "border-style: solid"
                                "dashed-rule" "border-style: dashed"
                                "dotted-circle" "border-style: dotted"
                                "double-rule" "border-style: double"
                                nil))) attributes)
        continuation (get block "continuation_indent")
        start-padding (or continuation indent)
        styles (cond-> []
                 (some? start-padding) (conj (str "padding-inline-start: " start-padding "em"))
                 (and (some? indent) (some? continuation)) (conj (str "text-indent: " (- indent continuation) "em"))
                 (contains? block "offset_from_end") (conj (str "padding-inline-end: " (get block "offset_from_end") "em"))
                 (contains? block "width") (conj (str "inline-size: " (get block "width") "em"))
                 (get block "column_count") (conj (str "column-count: " (get block "column_count")))
                 (get block "direction") (conj (str "writing-mode: " (case (get block "direction")
                                                                       "horizontal" "horizontal-tb"
                                                                       "vertical" "vertical-rl")))
                 (get block "align") (conj (str "text-align: " (get block "align")))
                 frame-style (conj frame-style))
        rend (cond-> []
               (get block "placement") (conj (get block "placement"))
               placement (conj "placement-below" (str "anchor-kind(" (get placement "anchor_kind") ")"))
               (contains? placement "offset_chars") (conj (str "anchor-offset-chars(" (get placement "offset_chars") ")"))
               (get block "column_rule") (conj "column-rule")
               (get block "column_number") (conj (str "column-part(" (get block "column_number") ")"))
               ;; An explicit `罫無し`: the source states the table has no rules.
               ;; Absent when the source states nothing about rules.
               (false? (get block "table_rules")) (conj "table-rules(none)")
               (get block "typography") (into (keep inline-layout-rend) attributes)
               (get block "page_placement") (conj (str "page-horizontal-" (get-in block ["page_spacing" "position"] "center")))
               (get block "page_vertical_placement") (conj (str "page-vertical-" (get block "page_vertical_placement")))
               (contains? (get block "page_spacing") "before_blank_lines") (conj (str "source-blank-lines-before(" (get-in block ["page_spacing" "before_blank_lines"]) ")")
                                                                                 (str "source-blank-lines-after(" (get-in block ["page_spacing" "after_blank_lines"]) ")"))
               (get block "line_count") (conj (str "line-count(" (get block "line_count") ")"))
               (contains? block "first_line_indent") (conj (str "block-first-line-indent(" (get block "first_line_indent") ")")))]
    (cond-> {:type (get block "role" "layout")}
      (get block "source_kind") (assoc :subtype (get block "source_kind"))
      anchor (assoc :corresp (str "#" (source-reference {"source_span" anchor})))
      (source-reference block) (assoc :source (str "#" (source-reference block)))
      (seq styles) (assoc :style (string/join "; " styles))
      (seq rend) (assoc :rend (string/join " " rend))
      (= "warichu" (get block "role")) (assoc :rend "two-line"))))

(defn- wrap-scope-intersections [content scopes]
  (reduce (fn [children scope]
            (let [alignment (when (and (= "right" (get scope "align"))
                                       (contains? scope "offset_from_end"))
                              (assoc (select-keys scope ["align" "offset_from_end"]) "kind" "chitsuki"))
                  attrs (layout-block-attrs (if alignment (dissoc scope "align" "offset_from_end") scope))]
              [(into [(if (get scope "typography") :hi :seg)
                      (cond-> attrs
                        (get scope "typography") (dissoc :type)
                        alignment (assoc :rend (string/join " " (remove nil? [(:rend attrs) (layout-rend alignment)]))
                                         :snh/layout-kind "chitsuki"
                                         :snh/layout-params (layout-params alignment)))]
                     children)]))
          content (reverse scopes)))

(defn- render-paragraph-intersections [acc nodes start end scopes]
  (let [boundaries (into (sorted-set start end)
                         (mapcat (fn [scope]
                                   [(max start (get-in scope ["node_range" "start"]))
                                    (min end (get-in scope ["node_range" "end"]))])) scopes)]
    (reduce (fn [acc [from to]]
              (let [active (filterv #(<= (get-in % ["node_range" "start"]) from
                                         (dec to) (dec (get-in % ["node_range" "end"]))) scopes)
                    prefix (:current-paragraph acc)
                    rendered (render-node-seq (assoc acc :current-paragraph []) (subvec nodes from to))]
                (assoc rendered :current-paragraph
                       (into prefix (wrap-scope-intersections (:current-paragraph rendered) active)))))
            acc (partition 2 1 boundaries))))

(defn- render-paragraph-row [nodes acc paragraph scopes frames]
  (let [hanging-offset (or (some (fn [{:keys [block]}]
                                   (when (and (contains? block "indent") (contains? block "continuation_indent"))
                                     (- (get block "indent") (get block "continuation_indent"))))
                                 (rseq frames)) 0)
        [nodes attrs] (paragraph-render-inputs nodes paragraph hanging-offset)
        [acc attrs] (paragraph-source-attrs acc paragraph attrs)
        {start "start" end "end"} (paragraph-range paragraph)
        node-slice (subvec nodes start end)]
    (case (get paragraph "role")
      "body" (-> acc
                 (assoc :current-paragraph-attrs attrs)
                 (render-paragraph-intersections nodes start end scopes)
                 flush-paragraph)
      "source-note" (-> acc
                        flush-paragraph
                        (render-node-seq node-slice)
                        flush-paragraph)
      (-> acc
          (assoc :current-paragraph-attrs attrs)
          (render-node-seq node-slice)
          flush-paragraph))))

(defn- paragraph-boundary-predicate [paragraphs]
  (let [ends (into (sorted-map)
                   (map (fn [p] [(get-in p ["node_range" "start"]) (get-in p ["node_range" "end"])]))
                   paragraphs)]
    (fn [index]
      (let [[start end] (first (rsubseq ends <= index))]
        (not (and start (< start index end)))))))

(defn- validate-layout-blocks! [nodes paragraphs blocks]
  (let [boundary? (paragraph-boundary-predicate paragraphs)
        external-notes (into (sorted-set)
                             (keep-indexed (fn [index node]
                                             (when (and (= "source-note" (get node "type"))
                                                        (not= "body" (get node "placement"))) index))) nodes)]
    (doseq [block blocks]
      (let [{start "start" end "end"} (get block "node_range")]
        (when-not (and (integer? start) (integer? end) (<= 0 start) (< start end)
                       (<= end (count nodes))
                       (or (and (boundary? start) (boundary? end))
                           (and (or (get block "typography") (get block "role")
                                    (some #(contains? block %) ["indent" "continuation_indent" "offset_from_end" "width" "line_count" "column_count" "align" "direction" "border"]))
                                (source-reference block)))
                       (empty? (subseq external-notes >= start < end)))
          (throw (ex-info "Invalid layout block node range" {:block block}))))))
  (reduce (fn [stack block]
            (let [{start "start" end "end"} (get block "node_range")
                  stack (loop [stack stack]
                          (if (and (seq stack) (<= (get-in (peek stack) ["node_range" "end"]) start))
                            (recur (pop stack)) stack))]
              (when (and (seq stack) (> end (get-in (peek stack) ["node_range" "end"])))
                (throw (ex-info "Crossing layout block ranges" {:blocks [(peek stack) block]})))
              (conj stack block)))
          [] (sort-by (juxt #(get-in % ["node_range" "start"])
                            #(- (get-in % ["node_range" "end"]))) blocks)))

(defn- scope-intersections [nodes paragraphs scopes]
  (let [runs (into (sorted-map)
                   (concat (map (fn [p] [(get-in p ["node_range" "start"]) (get-in p ["node_range" "end"])]) paragraphs)
                           (keep-indexed (fn [index node] (when (and (= "heading" (get node "type")) (= "normal" (get node "style"))) [index (inc index)])) nodes)))]
    (reduce (fn [result scope]
              (let [{start "start" end "end"} (get scope "node_range")
                    first-start (or (ffirst (rsubseq runs <= start)) start)]
                (reduce (fn [result [from to]]
                          (if (> to start) (update result from (fnil conj []) scope) result))
                        result (take-while (fn [[from _]] (< from end)) (subseq runs >= first-start)))))
            {} (sort-by (juxt #(get-in % ["node_range" "start"]) #(- (get-in % ["node_range" "end"]))) scopes))))

(defn- close-layout-blocks [acc frames node-end]
  (loop [acc acc frames frames]
    (if-let [{:keys [block parent-content]} (peek frames)]
      (if (= node-end (get-in block ["node_range" "end"]))
        (let [acc (-> acc flush-paragraph flush-division)
              [acc apparatus] (reduce (fn [[acc elements] instruction]
                                        (let [[acc instruction] (register-source acc instruction)
                                              [acc element] (formatting-instruction-element acc instruction)]
                                          [acc (conj elements element)]))
                                      [acc []] (get block "source_instructions"))
              content (normalize-layout-siblings (:body-children acc))
              wrapped (into [:div (layout-block-attrs block)] (concat apparatus content))]
          (recur (append-structural-child (merge acc parent-content) wrapped) (pop frames)))
        [acc frames])
      [acc frames])))

(defn- render-with-paragraphs [nodes paragraphs layout-blocks emendations]
  (validate-paragraph-ranges! nodes paragraphs)
  (validate-layout-blocks! nodes paragraphs layout-blocks)
  (let [boundary? (paragraph-boundary-predicate paragraphs)
        grouped (group-by #(and (boundary? (get-in % ["node_range" "start"]))
                                (boundary? (get-in % ["node_range" "end"]))) layout-blocks)
        intersections (scope-intersections nodes paragraphs (get grouped false))
        starts (group-by #(get-in % ["node_range" "start"]) (reverse (get grouped true)))
        paragraph-starts (into {} (keep (fn [paragraph]
                                          (let [{start "start" end "end"} (paragraph-range paragraph)]
                                            (when (< start end) [start paragraph])))) paragraphs)]
    (loop [acc (reduce (fn [acc block]
                         (reduce (fn [acc span]
                                   (if-let [reference (source-reference {"source_span" span})]
                                     (assoc-in acc [:source-spans reference] span) acc))
                                 acc [(get block "source_span") (get-in block ["relative_placement" "anchor_span"])]))
                       (initial-acc emendations) layout-blocks)
           index (Long/valueOf 0) frames []]
      (let [[acc frames] (close-layout-blocks acc frames index)]
        (if (= index (count nodes))
          (finalize-result (cond-> (-> acc flush-paragraph flush-division)
                             (seq layout-blocks) (update :body-children normalize-layout-siblings)))
          (let [[acc frames] (reduce
                              (fn [[acc frames] block]
                                (let [acc (flush-paragraph acc)]
                                  [(assoc acc :body-children [] :current-division [])
                                   (conj frames {:block block :parent-content
                                                 (select-keys acc [:body-children :current-division])})]))
                              [acc frames]
                              (sort-by #(get-in % ["node_range" "end"]) > (get starts index)))
                paragraph (get paragraph-starts index)]
            (if paragraph
              (recur (render-paragraph-row nodes (flush-paragraph acc) paragraph (get intersections index) frames)
                     (get-in paragraph ["node_range" "end"]) frames)
              (let [rendered (render-node-seq acc [(nth nodes index)])
                    scopes (get intersections index)
                    rendered (if (and (seq scopes) (= "heading" (get (nth nodes index) "type")))
                               (update-in rendered [:current-division 0]
                                          (fn [head] (into (subvec head 0 2) (wrap-scope-intersections (subvec head 2) scopes))))
                               rendered)]
                (recur rendered (inc index) frames)))))))))

(defn- render-flat [nodes emendations]
  (finalize-result
   (render-node-seq (initial-acc emendations) nodes)))

(defn render
  "Render the parser-IR body. The extents of the parser-IR's `emendations`
  join the `source-span` notes, so the header's correction declarations
  resolve."
  [parser-ir]
  (let [nodes (vec (get parser-ir "nodes"))
        paragraphs (seq (get parser-ir "paragraphs"))
        emendations (get parser-ir "emendations")]
    (if (or paragraphs (seq (get parser-ir "layout_blocks")))
      (render-with-paragraphs nodes (vec paragraphs) (get parser-ir "layout_blocks" []) emendations)
      (render-flat nodes emendations))))
