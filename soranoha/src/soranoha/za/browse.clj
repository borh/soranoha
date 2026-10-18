(ns soranoha.za.browse
  "The browse layer: static pages generated from one release's own verified
  bytes.

  Presentation, not publication. Nothing here is referenced by a manifest, hashed
  into the chain, or checked by a verifier: a reader who wants the published
  record follows the links these pages carry to `/catalog.json`, the
  manifests and the blobs. The pages provide an entry point: without them the
  only documented entry point is a 64-character hex string.

  Every page is a pure function of the head manifest, its catalog and the
  governance events the chain already contains, so re-exporting the same
  commit produces the same bytes and the exporter's reuse check still holds.
  The pages carry no state and no runtime; serving stays a static tree.

  Site text is bilingual, Japanese first: the corpus is Japanese literature
  read by scholars in Japan, and the encoding and protocol work is aimed at
  an international digital-humanities audience. Titles, personal names and
  source editions are never translated."
  (:require [clojure.string :as string]
            [soranoha.core.json :as json]
            [soranoha.core.rights :as rights]
            [soranoha.za.maturity :as maturity]
            [soranoha.snh.verify :as verify]
            [soranoha.za.citation :as citation]
            [soranoha.za.docs :as docs]
            [soranoha.za.html :as html]
            [soranoha.za.markdown :as markdown]
            [soranoha.za.naming :as naming]
            [soranoha.za.reading :as reading]))

(def ^:private site-name "Soranoha")

(def ^:private orcid-url
  "Where a rights claim is sent. An ORCID record rather than a mail address:
  the served rights statement is what every manifest and every detached TEI
  file points at, so its contact route must remain valid beyond any one mailbox, and
  ORCID is the identifier the project already publishes in `CITATION.cff`."
  "https://orcid.org/0000-0003-2246-8774")

(def ^:private example-identifier
  "A work identifier shown as a shape rather than as a link, so a reader
  recognises the form to quote when identifying a work in a claim."
  "000092_000879")

(def ^:private stylesheet
  (string/join
   "\n"
   ;; Pastel light blue and green, after the name: 空 for the
   ;; blue surfaces and 葉 for the green ones. Green carries the page and its
   ;; rules, blue carries what a reader acts on (links) and the table headings
   ;; that separate a grid from the prose around it. Every text colour in
   ;; either scheme meets WCAG AA against every surface it is used on; the
   ;; dark scheme keeps the same hues on dark surfaces and follows the
   ;; reader's system preference.
   [(str ":root { color-scheme: light dark; --ink: #152a28; --muted: #4a6a66; --rule: #bfdcd5;"
         " --bg: #f5fbf9; --panel: #e2f0ed; --sky: #dcebf4; --field: #fff; --link: #15607f; }")
    (str "@media (prefers-color-scheme: dark) { :root { --ink: #e4efec; --muted: #9fbab5;"
         " --rule: #33504b; --bg: #0f1c1a; --panel: #1a2b28; --sky: #182a36; --field: #142321;"
         " --link: #8ac4e0; } }")
    "* { box-sizing: border-box; }"
    "body { margin: 0; background: var(--bg); color: var(--ink);"
    "  font-family: \"Hiragino Mincho ProN\", \"Yu Mincho\", \"Noto Serif JP\", Georgia, serif;"
    "  line-height: 1.7; }"
    "main, header, footer { max-inline-size: 48rem; margin-block: 0; margin-inline: auto;"
    "  padding-inline: 1.25rem; }"
    "header { border-block-end: 1px solid var(--rule);"
    "  padding-block: 1.5rem 1rem; }"
    "header a.site { font-size: 1.15rem; font-weight: bold; text-decoration: none; color: var(--ink); }"
    "nav { margin-block-start: .5rem; font-size: .9rem; display: flex; flex-wrap: wrap; gap: .35rem 1rem; }"
    "a { color: var(--link); }"
    "a:focus-visible, button:focus-visible, input:focus-visible, summary:focus-visible {"
    "  outline: 2px solid var(--link); outline-offset: 2px; }"
    "main { padding-block: 1.5rem 3rem; }"
    "h1 { font-size: 1.6rem; line-height: 1.35; margin-block: 0 .25rem; }"
    "h2 { font-size: 1.15rem; margin-block: 2rem .5rem;"
    "  border-block-end: 1px solid var(--rule); padding-block-end: .25rem; }"
    ".en { color: var(--muted); font-weight: normal; }"
    ".reading { color: var(--muted); margin-block: 0 1rem; }"
    ;; set off by a rule rather than by a colour: this states the corpus's
    ;; stage, and a warning colour would suggest the bytes are provisional
    ".maturity { border-inline-start: 3px solid var(--rule); padding-inline-start: .75rem;"
    "  font-size: .95rem; }"
    ".stats { list-style: none; padding: 0; display: flex; flex-wrap: wrap; gap: 1.5rem; }"
    ".stats li { margin: 0; }"
    ".stats .n { font-size: 1.5rem; display: block; }"
    "dl.facts { display: grid; grid-template-columns: max-content 1fr; gap: .35rem 1.25rem; margin: 0; }"
    ;; a term can carry several values. Pinning each element to its column
    ;; keeps a second value under the first; auto-placement would put it in
    ;; the term column
    "dl.facts dt { grid-column: 1; color: var(--muted); font-size: .9rem; }"
    "dl.facts dd { grid-column: 2; margin: 0; }"
    "ul.works, ul.plain { list-style: none; padding: 0; }"
    "ul.works li { padding-block: .35rem; border-block-end: 1px solid var(--rule); }"
    "ul.works li.empty { color: var(--muted); }"
    "ul.works .by { color: var(--muted); font-size: .9rem; }"
    ;; smaller than the byline, since it is only needed on the rows that
    ;; would otherwise look identical
    "ul.works .variant { color: var(--muted); font-size: .85rem; }"
    "h3.group { font-size: 1rem; margin-block: 1.5rem .25rem; }"
    "h3.group .by { font-weight: normal; }"
    "ul.cols { list-style: none; padding: 0; columns: 2; }"
    "ul.plain li.evidence { margin-block-start: .5rem; padding-block-start: .5rem;"
    "  border-block-start: 1px solid var(--rule); }"
    "a.read-link { display: inline-block; padding: .4rem .85rem; background: var(--panel);"
    "  border: 1px solid var(--rule); border-radius: 3px; text-decoration: none;"
    "  font-weight: bold; color: var(--ink); }"
    "a.read-link:hover, a.read-link:focus-visible { background: var(--sky); border-color: var(--link); }"
    "code.citation { display: block; white-space: pre-wrap; overflow-wrap: anywhere;"
    "  padding: .6em .8em; background: var(--panel); border-radius: 3px; }"
    "code { font-family: ui-monospace, Menlo, Consolas, monospace; font-size: .85em;"
    "  overflow-wrap: anywhere; }"
    "footer { border-block-start: 1px solid var(--rule); padding-block: 1rem 3rem;"
    "  font-size: .85rem; color: var(--muted); }"
    "#q { inline-size: 100%; padding: .6rem .8rem; font-size: 1rem; font-family: inherit;"
    "  border: 1px solid var(--rule); border-radius: 3px; background: var(--field); color: var(--ink); }"
    "#q:disabled { background: var(--panel); color: var(--muted); }"
    "#results:empty { display: none; }"
    ;; the copy button sits beside the value it copies, in the muted colour
    ;; until hovered or focused, so it is subordinate to the text it copies
    "button.copy { margin-inline-start: .5em; padding: .1em .5em; font: inherit;"
    "  font-size: .75em; line-height: 1.6; color: var(--muted); cursor: pointer;"
    "  background: var(--bg); border: 1px solid var(--rule); border-radius: 3px;"
    "  vertical-align: middle; }"
    "button.copy:hover, button.copy:focus-visible { color: var(--ink);"
    "  background: var(--panel); }"
    "button.copy.done { color: var(--link); border-color: var(--link); }"
    ;; beside a short value the button sits inline, but these two are blocks
    ;; the full width of the column, so the button goes under the block it
    ;; copies and against its trailing edge, so it is visually attached to
    ;; that block
    "pre + button.copy, code.citation + button.copy { display: block;"
    "  inline-size: fit-content; margin-block: .4rem 0; margin-inline: auto 0; }"
    "@media (max-width: 32rem) { dl.facts { grid-template-columns: 1fr; }"
    "  dl.facts dd { grid-column: 1; margin-inline-start: 1rem; } ul.cols { columns: 1; } }"

    ;; the documentation pages. Prose the project wrote, so unlike the reading
    ;; view these are ordinary elements and take ordinary rules.
    "main.doc h3 { font-size: 1.05rem; margin-block: 1.5rem .5rem; }"
    "main.doc table { border-collapse: collapse; inline-size: 100%; font-size: .9rem;"
    "  margin-block: 1rem; }"
    "@media (max-width: 40rem) { main.doc table { display: block; overflow-x: auto; } }"
    "main.doc th, main.doc td { border: 1px solid var(--rule); padding: .3rem .5rem;"
    "  text-align: start; vertical-align: top; }"
    "main.doc th { background: var(--sky); }"
    "main.doc pre { background: var(--panel); padding: .6rem .8rem; border-radius: 3px;"
    "  overflow-x: auto; font-size: .85rem; line-height: 1.5; }"
    "main.doc pre code { font-size: 1em; overflow-wrap: normal; }"
    "main.doc blockquote { margin-block: 1rem; margin-inline: 0; padding-inline-start: 1rem;"
    "  border-inline-start: 3px solid var(--rule); color: var(--muted); }"
    "nav.siblings { margin-block-start: 3rem; border-block-start: 1px solid var(--rule);"
    "  padding-block-start: 1rem; font-size: .9rem; }"

    ;; the reading view. Everything below styles TEI that has been projected
    ;; into HTML by soranoha.za.reading; the class names are that projection's
    ;; vocabulary, and a class with no rule here renders as ordinary text
    ;; rather than disappearing.
    "p.controls { margin-block: 1rem 2rem; font-size: .9rem; }"
    "p.controls label { cursor: pointer; }"
    "details.front { margin-block: 0 2rem; font-size: .9rem; color: var(--muted); }"
    "details.front summary { cursor: pointer; }"
    "details.bibtex summary { cursor: pointer; color: var(--muted); font-size: .9rem; }"
    ;; the entry wraps rather than scrolls: a reader is here to select it, and
    ;; a line running off the edge makes selection harder
    "details.bibtex pre { margin-block: .5rem 0; margin-inline: 0; padding: .6em .8em; background: var(--panel);"
    "  border-radius: 3px; font-family: ui-monospace, Menlo, Consolas, monospace;"
    "  font-size: .85em; white-space: pre-wrap; overflow-wrap: anywhere; }"
    ".colophon, .provenance { font-size: .9rem; }"
    ".tei { line-height: 2; }"
    ".tei [title] { cursor: help; }"
    ".tei section { margin-block: 0 1.5rem; }"
    ".tei p { margin-block: 0 1em; }"
    ;; a source heading is part of the work, not part of the site's chrome, so
    ;; it takes none of the section-heading rule above
    ".tei h2, .tei h3, .tei h4 { border: none; padding: 0; margin-block: 2rem 1rem; }"
    ".tei h2 { font-size: 1.3rem; }"
    ".tei h3 { font-size: 1.15rem; }"
    ".tei h4 { font-size: 1.05rem; }"
    ".tei ruby { ruby-align: center; }"
    ".tei rt { font-size: .5em; font-weight: normal; }"
    ".tei .gaiji { border-block-end: 1px dotted var(--muted); }"
    ".tei .gaiji-unmapped { color: var(--muted); }"
    ".tei .app, .tei .choice { border-block-end: 1px dotted var(--rule); }"
    ".tei .note { font-size: .85em; color: var(--muted); }"
    ".tei .gap { color: var(--muted); }"
    ".tei .pb { display: block; block-size: 1px; background: var(--rule); margin-block: 1.5rem; }"
    ".tei figure { margin-block: 1.5rem; margin-inline: 0; padding: .75rem;"
    "  border: 1px dashed var(--rule);"
    "  color: var(--muted); font-size: .9em; }"
    ".tei .graphic-url { font-size: .8em; }"
    ".tei .ref-target { font-size: .8em; color: var(--muted); }"
    ".tei .rend-bold { font-weight: bold; }"
    ".tei .rend-italic { font-style: italic; }"
    ".tei .rend-gothic, .tei .rend-textbook {"
    "  font-family: \"Hiragino Kaku Gothic ProN\", \"Yu Gothic\", \"Noto Sans JP\", sans-serif; }"
    ".tei .rend-dots { text-emphasis: filled sesame; }"
    ".tei .rend-dots-open { text-emphasis: open sesame; }"
    ".tei .rend-dots-circle { text-emphasis: filled circle; }"
    ".tei .rend-dots-circle-open { text-emphasis: open circle; }"
    ".tei .rend-dots-double-circle { text-emphasis: open double-circle; }"
    ".tei .rend-dots-cross { text-emphasis: filled \"\\00d7\"; }"
    ".tei .rend-dots-triangle { text-emphasis: filled triangle; }"
    ".tei .rend-dots-triangle-open { text-emphasis: open triangle; }"
    ".tei .rend-dots-square-open { text-emphasis: \"\\25a1\"; }"
    ".tei .rend-line { text-decoration: underline; text-underline-position: under right; }"
    ".tei .rend-line-double { text-decoration: underline double; text-underline-position: under right; }"
    ".tei .rend-line-wavy { text-decoration: underline wavy; text-underline-position: under right; }"
    ".tei .rend-line-dashed { text-decoration: underline dashed; text-underline-position: under right; }"
    ".tei .rend-left { text-emphasis-position: under left; text-underline-position: left; }"
    ".tei .layout-chitsuki { display: block; text-align: end; }"
    ".tei .layout-jizume { display: block; }"
    ".tei .layout-exponent { vertical-align: super; font-size: .75em; }"
    ".tei .layout-baseline-position { vertical-align: sub; font-size: .85em; }"
    ".tei .layout-small-script { font-size: .75em; }"
    ".tei .layout-tcy { text-combine-upright: all; }"
    ".tei .layout-keigakomi { border: 1px solid var(--muted); padding: 0 .2em; }"
    ".tei .layout-yokogumi { writing-mode: horizontal-tb; display: inline-block; }"
    ".tei .layout-fraction { font-size: .85em; }"
    ".tei .type-warichu { font-size: .7em; }"
    ".tei .type-upper, .tei .type-lower { display: block; line-height: 1.2; }"
    ".tei .type-heading { font-weight: bold; }"

    ;; Vertical is a checkbox rather than a script. Horizontal is the default
    ;; because it is what a browser lays out correctly without help and what a
    ;; narrow screen can show; vertical is what the source was set in, so it
    ;; is one click away rather than unavailable. Logical properties carry the
    ;; indents across: padding-inline-start is a left margin horizontally and
    ;; a top margin vertically, which is what 字下げ means in each direction.
    ;;
    ;; The mode is set on body, which propagates it to the viewport, so the
    ;; page's own scroller is the one that runs right to left. A nested
    ;; scrolling box would open every work at its last line: a nested scroller
    ;; starts at its left edge, and in vertical-rl the left edge is the end.
    ;; Measured on 小説総論 at 1280px, a nested box places the first paragraph
    ;; at x=2821; on the document scroller it is at x=859, on screen at load. A
    ;; nested box also needs a fixed height, which clips the last line of every
    ;; work.
    "body:has(#tategaki:checked) { writing-mode: vertical-rl; text-orientation: mixed; }"
    "body:has(#tategaki:checked) main, body:has(#tategaki:checked) header, body:has(#tategaki:checked) footer {"
    "  max-inline-size: min(42rem, calc(100dvh - 3rem)); padding-block: 1.5rem; }"]))

(def ^:private search-js
  ;; Progressive enhancement only. Every work is reachable through the
  ;; generated author, title and NDC indexes with scripting off; this makes
  ;; 17308 works findable by typing instead of by navigating.
  ;;
  ;; `fold` lowercases, strips diacritics and folds katakana to hiragana on
  ;; both the index and the query, so natsume finds Natsume and ソウセキ
  ;; finds そうせき.
  (string/join
   "\n"
   ["(function () {"
    "  var box = document.getElementById('q'), out = document.getElementById('results'), rows = null, loaded = false;"
    "  if (!box || !out) { return; }"
    "  box.disabled = false;"
    "  box.placeholder = box.getAttribute('data-ready');"
    "  function fold(s) {"
    "    s = (s || '').toLowerCase();"
    "    if (s.normalize) { s = s.normalize('NFKD').replace(/[\\u0300-\\u036f]/g, ''); }"
    "    return s.replace(/[\\u30a1-\\u30f6]/g, function (c) {"
    "      return String.fromCharCode(c.charCodeAt(0) - 0x60);"
    "    });"
    "  }"
    "  function draw() {"
    "    var q = fold(box.value.trim());"
    "    if (!loaded || q === '') { out.textContent = ''; return; }"
    "    var terms = q.split(/[\\s\\u3000]+/).filter(Boolean);"
    "    var hits = [], i, j, ok;"
    "    for (i = 0; i < rows.length && hits.length < 50; i++) {"
    "      ok = true;"
    "      for (j = 0; j < terms.length; j++) {"
    "        if (rows[i][4].indexOf(terms[j]) === -1) { ok = false; break; }"
    "      }"
    "      if (ok) { hits.push(rows[i]); }"
    "    }"
    "    out.innerHTML = '';"
    "    if (hits.length === 0) {"
    "      var empty = document.createElement('li');"
    "      empty.className = 'empty';"
    "      empty.textContent = '該当する作品はありません ';"
    "      var en = document.createElement('span');"
    "      en.className = 'en';"
    "      en.lang = 'en';"
    "      en.textContent = '/ No matching works';"
    "      empty.appendChild(en);"
    "      out.appendChild(empty);"
    "      return;"
    "    }"
    "    for (i = 0; i < hits.length; i++) {"
    "      var li = document.createElement('li');"
    "      var a = document.createElement('a');"
    "      a.href = '/works/' + hits[i][0] + '/';"
    "      a.textContent = hits[i][1];"
    "      li.appendChild(a);"
    "      if (hits[i][3]) {"
    "        var by = document.createElement('span');"
    "        by.className = 'by';"
    "        by.textContent = ' — ' + hits[i][3];"
    "        li.appendChild(by);"
    "      }"
    "      out.appendChild(li);"
    "    }"
    "  }"
    "  function load() {"
    "    if (rows) { return; }"
    "    rows = [];"
    "    fetch('/search-index.json').then(function (r) { return r.json(); }).then(function (d) {"
    "      rows = d.works.map(function (w) {"
    "        var compact = (w[3] || '').replace(/\\s+/g, '');"
    "        return [w[0], w[1], w[2], w[3], fold(w[1] + ' ' + w[2] + ' ' + w[3] + ' ' + compact + ' ' + w[0] + ' ' + w[4])];"
    "      });"
    "      loaded = true;"
    "      draw();"
    "    });"
    "  }"
    "  box.addEventListener('focus', load);"
    "  box.addEventListener('input', function () { load(); draw(); });"
    "})();"]))

(def ^:private copy-js
  ;; Progressive enhancement, like the search box: the button is created here
  ;; rather than emitted into every page, so with scripting off a reader gets
  ;; selectable text and no inoperative control. The button is a sibling of the code
  ;; rather than a child, so selecting the code by hand never picks it up, and
  ;; a code inside a link gets its button after the link so the anchor stays
  ;; whole.
  ;;
  ;; A button belongs beside a value a reader would rather copy than retype:
  ;; every `pre`, the citation blocks, and the codes the pages mark
  ;; `value`, which are identifiers, hashes, filenames and URLs. A code in a
  ;; sentence is a name, and a field name in the documentation takes no
  ;; button.
  (string/join
   "\n"
   ["(function () {"
    "  var clip = navigator.clipboard;"
    "  if (!clip || !document.querySelectorAll) { return; }"
    "  var ja = document.documentElement.lang === 'ja';"
    "  var idle = ja ? 'コピー' : 'Copy', done = ja ? 'コピーしました' : 'Copied';"
    "  function attach(node, text) {"
    "    var anchor = node.parentNode;"
    "    if (!anchor) { return; }"
    "    if (anchor.tagName === 'A') { node = anchor; anchor = anchor.parentNode; }"
    "    var button = document.createElement('button'), timer = null;"
    "    button.type = 'button';"
    "    button.className = 'copy';"
    "    button.textContent = idle;"
    "    button.setAttribute('aria-label', idle);"
    "    button.addEventListener('click', function () {"
    "      clip.writeText(text).then(function () {"
    "        if (timer) { clearTimeout(timer); }"
    "        button.textContent = done;"
    "        button.setAttribute('aria-label', done);"
    "        button.classList.add('done');"
    "        timer = setTimeout(function () {"
    "          button.textContent = idle;"
    "          button.setAttribute('aria-label', idle);"
    "          button.classList.remove('done');"
    "          timer = null;"
    "        }, 1500);"
    "      });"
    "    });"
    "    anchor.insertBefore(button, node.nextSibling);"
    "  }"
    "  var nodes = document.querySelectorAll('pre, code.citation, code.value'), i;"
    "  for (i = 0; i < nodes.length; i++) {"
    "    attach(nodes[i], nodes[i].textContent);"
    "  }"
    "})();"]))

(defn- bilingual
  "Japanese label with its English counterpart marked as English, so a screen
  reader and a translation tool both get the language right."
  [ja en]
  (list ja " " [:span {:class "en" :lang "en"} (str "/ " en)]))

(defn- head-nodes [title]
  [[:meta {:charset "utf-8"}]
   [:meta {:name "viewport" :content "width=device-width, initial-scale=1"}]
   [:title (str title " — " site-name)]
   [:link {:rel "stylesheet" :href "/style.css"}]])

(defn- chrome
  ([title body] (chrome title {} body))
  ([title {:keys [main-class]} body]
   (html/document
    {:lang "ja"}
    (head-nodes title)
    [[:header
      [:a {:class "site" :href "/"} site-name]
      [:nav
       [:a {:href "/authors/"} (bilingual "著者" "Authors")]
       [:a {:href "/titles/"} (bilingual "作品名" "Titles")]
       [:a {:href "/ndc/"} (bilingual "分類" "NDC")]
       [:a {:href "/rights"} (bilingual "権利" "Rights")]
       [:a {:href "/citation"} (bilingual "引用" "Citation")]
       [:a {:href "/start-here"} (bilingual "解説" "Docs")]]]
     (into [:main (cond-> {} main-class (assoc :class main-class))] body)
     [:footer
      [:p (bilingual
           "本文は青空文庫の収録作品で、符号化データはCC0 1.0で提供しています。作品ごとの権利の状態は各作品のページに記載しています。公開マニフェストは電子署名されており、各ファイルのハッシュ値を記録しています。本サイトはその署名済みの内容を表示しています。"
           "Texts are works from Aozora Bunko; the encoding is CC0 1.0. Each work's page states the rights standing of that work. The release manifest is signed and records the hash of every file; this site displays the signed content.")]
      [:p [:a {:href "/catalog.json"} "/catalog.json"] " · "
       [:a {:href "/releases/HEAD"} "/releases/HEAD"] " · "
       [:a {:href "https://www.aozora.gr.jp/"} "青空文庫"]]]
     [:script {:src "/copy.js"}]])))

;; Personal names, bylines and download filenames are rendered by
;; soranoha.za.naming, which the bulk archives share: a name on a page and
;; the name inside a ZIP must agree.

(def ^:private kana-rows
  [["a" "あ行" "あいうえおぁぃぅぇぉゔ"]
   ["ka" "か行" "かきくけこがぎぐげごゕゖ"]
   ["sa" "さ行" "さしすせそざじずぜぞ"]
   ["ta" "た行" "たちつてとだぢづでどっ"]
   ["na" "な行" "なにぬねの"]
   ["ha" "は行" "はひふへほばびぶべぼぱぴぷぺぽ"]
   ["ma" "ま行" "まみむめも"]
   ["ya" "や行" "やゆよゃゅょ"]
   ["ra" "ら行" "らりるれろ"]
   ["wa" "わ行" "わをんゎ"]
   ["other" "その他" ""]])

(defn- hiragana
  "Katakana folded to hiragana so one reading form keys the index."
  [^String s]
  (apply str (map (fn [c]
                    (let [n (int c)]
                      (if (<= 0x30A1 n 0x30F6) (char (- n 0x60)) c)))
                  s)))

(defn- kana-row-key [work]
  (let [reading (get work "title_reading")
        head (when-not (string/blank? reading) (first (hiragana reading)))]
    (or (some (fn [[key _ members]]
                (when (and head (string/includes? members (str head))) key))
              kana-rows)
        "other")))

(def ndc-classes
  [["0" "総記" "General works"]
   ["1" "哲学" "Philosophy"]
   ["2" "歴史" "History"]
   ["3" "社会科学" "Social sciences"]
   ["4" "自然科学" "Natural sciences"]
   ["5" "技術・工学" "Technology"]
   ["6" "産業" "Industry"]
   ["7" "芸術・美術" "The arts"]
   ["8" "言語" "Language"]
   ["9" "文学" "Literature"]
   ["other" "その他・分類なし" "Other or unclassified"]])

(defn ndc-class-key
  "First character of the first NDC code. Aozora Bunko records children's material
  as K-prefixed codes and leaves some works unclassified, so anything that is
  not a main-class digit collects under one key rather than inventing a
  class for it."
  [work]
  (let [code (some-> (get work "ndc") (string/split #"\s+") second)
        head (when-not (string/blank? code) (str (first code)))]
    (if (some #(= head (first %)) (butlast ndc-classes)) head "other")))

(defn- source-edition-title [work]
  (get (first (get work "source_editions")) "title"))

(defn- variant-facts
  "The facts that distinguish works a list would otherwise print identically.

  Aozora Bunko carries several transcriptions of one text: 樋口一葉's わかれ道
  is keyed three times. They differ in the edition they were keyed from and in
  whether that edition modernized the orthography, so those are the two facts
  that separate them.

  Orthography alone separates most groups; the source edition is added only
  where it does not."
  [works]
  (let [styles (mapv #(get % "orthographic_style") works)]
    (if (or (apply distinct? styles) (= 1 (count styles)))
      [:orthographic_style]
      [:orthographic_style :source-edition])))

(defn- variant-label [work facts]
  (let [parts (keep (fn [fact]
                      (case fact
                        :orthographic_style (get work "orthographic_style")
                        :source-edition (source-edition-title work)))
                    facts)]
    (when (seq parts)
      (str "（" (string/join "・" parts) "）"))))

(defn- work-link [work facts stated-byline]
  (let [slug (get work "slug")
        by (naming/byline work)]
    [:li
     [:a {:href (str "/works/" slug "/")} (get work "title")]
     (when (and by (not= by stated-byline))
       [:span {:class "by"} (str " — " by)])
     (when-let [label (and facts (variant-label work facts))]
       [:span {:class "variant"} (str " " label)])]))

(defn- work-list
  "One row per work. A title and byline do not identify a work uniquely.
  Where the same pair identifies more than one work in a list, each row also shows
  what separates it, so that three rows reading わかれ道 — 樋口 一葉 are not
  read as one row repeated three times.

  `stated-byline` is the byline the heading above the list already states;
  a row whose byline is exactly that one leaves it out, and a row with a
  different byline, such as a co-authored or translated work on an author's
  page, keeps it."
  ([works] (work-list works nil))
  ([works stated-byline]
   (if (seq works)
     (let [ambiguous (into {}
                           (keep (fn [[key group]]
                                   (when (< 1 (count group))
                                     [key (variant-facts group)])))
                           (group-by (juxt #(get % "title") naming/byline) works))]
       (into [:ul {:class "works"}]
             (map #(work-link % (get ambiguous [(get % "title") (naming/byline %)]) stated-byline)
                  works)))
     [:p (bilingual "該当する作品はありません。" "No works here.")])))

(defn- author-groups
  "`works` grouped under their authors, as a library shelf is: one heading
  per author set, in the order of the author index, each with the works in
  the order given. Works with no recorded author come last under one heading."
  [works]
  (let [groups (group-by #(mapv (fn [person] (get person "person_id")) (naming/authors-of %))
                         works)
        order (fn [[ids group]]
                (if (empty? ids)
                  [1]
                  [0 (mapv (fn [person]
                             [(or (naming/person-name-romaji person) "￿")
                              (or (naming/person-name-ja person) "")
                              (get person "person_id")])
                           (naming/authors-of (first group)))]))]
    (for [[_ group] (sort-by order groups)
          :let [authors (naming/authors-of (first group))]]
      (list
       [:h3 {:class "group"}
        (if (seq authors)
          (interpose "、" (map (fn [person]
                                [:a {:href (str "/authors/" (get person "person_id"))}
                                 (naming/person-label person)])
                              authors))
          (bilingual "著者の記録なし" "No recorded author"))
        [:span {:class "by"} (str " (" (count group) ")")]]
       (work-list group (naming/byline (first group)))))))

(defn- archive-label [artifact-type]
  (case artifact-type
    "tei" "TEI XML"
    "plaintext" (bilingual "プレーンテキスト" "Plain text")
    artifact-type))

(defn- bulk-section
  "The pre-built archives for one selection. Serving is a static tree with no
  runtime, so a selection cannot be assembled on request: these are the
  selections, and they mirror the axis of the page they appear on."
  [scope-ja scope-en paths]
  [:section
   [:h2 (bilingual "まとめてダウンロード" "Download in bulk")]
   [:p (bilingual scope-ja scope-en)]
   (into [:ul {:class "plain"}]
         (map (fn [[path artifact-type]]
                [:li [:a {:href (str "/" path)} (archive-label artifact-type)]
                 [:span {:class "by"} " ZIP"]])
              paths))
   [:p (bilingual
        (str "各ZIPの直下には、識別子・作品名・著者・底本・初出・分類・リリースなどを収めたcatalog.csvが入っています。"
             "表計算ソフトでそのまま開いて文献表を作成できます。"
             "引用には識別子・底本ハッシュ・リリースの列を使ってください（ファイル名は便宜上のものです）。")
        (str "Every archive includes catalog.csv at its root with columns for identifier, "
             "title, author, source edition, first publication, class and release. "
             "Cite the identifier, source hash and release columns; the filename is for convenience."))]])

(defn- corpus-archives []
  (map (juxt naming/corpus-bundle-path identity) naming/bulk-artifact-types))

(defn- release-note [head-hex work-count]
  [:section
   [:h2 (bilingual "このリリース" "This release")]
   [:dl {:class "facts"}
    [:dt (bilingual "リリース" "Release")]
    [:dd [:a {:href (str "/releases/" head-hex ".json")} [:code head-hex]]]
    (when work-count
      (list [:dt (bilingual "作品数" "Works")] [:dd (str work-count)]))
    [:dt (bilingual "目録" "Catalog")]
    [:dd [:a {:href "/catalog.json"} "/catalog.json"]]
    [:dt (bilingual "履歴" "History")]
    [:dd [:a {:href "/history"} (bilingual "リリース履歴一覧" "All releases up to this one")]]]])

(defn- document-entries
  "Every served document with its own text and its own English title.

  Read once per activation and passed around, because the landing page, the
  page itself and every page's sibling list all list the same set, and two
  reads of one document could differ."
  []
  (mapv (fn [document]
          (let [text (docs/read-text (:path document))]
            (assoc document :text text :en (markdown/title text))))
        docs/documents))

(defn- sibling-nav
  "The other served documents, so a reader who arrived at one by link can
  reach the rest without going back to the landing page."
  [entries current]
  [:nav {:class "siblings"}
   [:p (bilingual "ほかの解説：" "Other documents:")]
   [:p (interpose
        " · "
        (for [{:keys [route ja en]} entries
              :when (not= route current)]
          [:a {:href (str "/" route)} (bilingual ja en)]))]])

(defn- document-page [entries {:keys [route path text ja en]}]
  (chrome
   ja
   {:main-class "doc"}
   (concat [[:h1 (bilingual ja en)]]
           (markdown/render {:text text
                             :source path
                             :link (partial docs/resolve-link path)})
           [(sibling-nav entries route)])))

(defn- maturity-notice
  "The corpus's maturity, stated once where a reader arrives, rather than in the
  chrome of every page. It is a statement about the corpus's stage, not about
  the release being viewed, and repeating it on every page would read as a
  warning about the bytes on that page instead."
  [release-name {:keys [ja en note-ja note-en]}]
  [:p {:class "maturity"}
   [:strong (bilingual (str (when release-name (str release-name " · ")) ja)
                       (str (when release-name (str release-name " · ")) en))]
   " "
   (bilingual note-ja note-en)])

(defn- landing [head-hex works withdrawn-count release-name maturity entries]
  (chrome
   "青空文庫TEIコーパス"
   [[:h1 (bilingual "青空文庫TEIコーパス" "Aozora Bunko TEI corpus")]
    [:p (bilingual
         (str "青空文庫の作品をTEI P5に変換し、ハッシュ値で識別できる署名付きリリースとして公開しています。"
              "各作品につき、TEI、プレーンテキスト、Markdown、検証レポートの4形式を提供しています。")
         (str "Works from Aozora Bunko converted to TEI P5 and published as "
              "signed, content-addressed releases. Each work is published in four formats: "
              "TEI, plain text, Markdown and a validation report."))]
    (when maturity (maturity-notice release-name maturity))
    [:ul {:class "stats"}
     [:li [:span {:class "n"} (str (count works))] (bilingual "作品" "works")]
     [:li [:span {:class "n"} "4"] (bilingual "形式" "formats")]
     [:li [:span {:class "n"} "CC0"] (bilingual "符号化のライセンス" "encoding licence")]
     (when (pos? withdrawn-count)
       [:li [:span {:class "n"} (str withdrawn-count)] (bilingual "取り下げ" "withdrawn")])]

    [:section
     [:h2 (bilingual "作品を探す" "Find a work")]
     [:p [:input {:id "q" :type "search" :disabled true
                  :aria-label "作品名・著者名で検索 / Search by title or author"
                  :placeholder "検索にはJavaScriptが必要です / Search requires JavaScript"
                  :data-ready "作品名・読み・著者名（ローマ字も可） / Title, reading or author, romaji accepted"
                  :autocomplete "off"}]]
     [:ul {:class "works" :id "results" :aria-live "polite"}]
     [:p (bilingual "一覧から探せます：" "Or browse the indexes:")
      " "
      [:a {:href "/authors/"} (bilingual "著者" "authors")] " · "
      [:a {:href "/titles/"} (bilingual "作品名の読み" "title readings")] " · "
      [:a {:href "/ndc/"} (bilingual "NDC分類" "NDC classes")]]]

    [:section
     [:h2 (bilingual "そのまま使う" "Use it directly")]
     [:dl {:class "facts"}
      [:dt [:a {:href "/catalog.json"} "/catalog.json"]]
      [:dd (bilingual "全作品の書誌目録（署名済みリリースの一部）"
                      "Bibliography of every work in this release; part of the signed record.")]
      [:dt [:a {:href "/releases/HEAD"} "/releases/HEAD"]]
      [:dd (bilingual "現在のリリースのハッシュ" "The current release head.")]
      [:dt [:a {:href "/releases/latest"} "/releases/latest"]]
      [:dd (bilingual "現在のリリースのマニフェスト" "The current release manifest.")]
      [:dt [:code "/works/<識別子>/tei"]]
      [:dd (bilingual "作品ごとのファイル（plaintext・markdown・tei-validationも同様）"
                      "Per-work artifacts; plaintext, markdown and tei-validation take the same form.")]]]

    (bulk-section
     "このリリースの全作品を一つのZIPにまとめています。著者別・NDC分類別のZIPは各ページにあります。"
     "Every work in this release, in one archive. Per-author and per-class archives are on the author and NDC pages."
     (corpus-archives))

    [:section
     [:h2 (bilingual "解説" "Documentation")]
     [:p (bilingual
          "符号化の方針、識別子の体系、収録判定の基準をまとめています。概要は「はじめに」を参照してください。"
          "See Start here for an overview of encoding policy, identifiers, and assessment criteria.")]
     [:ul {:class "cols plain"}
      (for [{:keys [route ja en]} entries]
        [:li [:a {:href (str "/" route)} (bilingual ja en)]])]]

    [:section
     [:h2 (bilingual "権利と引用" "Rights and citation")]
     [:p (bilingual
          "底本とした作品は、著作権の保護期間が満了しているか、または権利者がクリエイティブ・コモンズ表示ライセンス（CC BY）で公開しているものです。Soranohaの符号化データはCC0 1.0で提供しています。作品ごとの権利状態は各作品のページとcatalog.jsonに記載しています。"
          "Each underlying work is either out of copyright or published by its rightsholder under a Creative Commons Attribution licence; Soranoha's encoding is CC0 1.0. Each work's page and catalog.json record each work's standing.")]
     [:p [:a {:href "/rights"} (bilingual "権利について" "Rights statement")] " · "
      [:a {:href "/citation"} (bilingual "引用のしかた" "How to cite")]]]

    (release-note head-hex (count works))
    [:script {:src "/search.js"}]]))

(defn- author-index [people]
  (chrome
   "著者一覧"
   [[:h1 (bilingual "著者一覧" "Authors")]
    [:p (bilingual
         (str "このリリースの収録作品に関わる人物は" (count people) "名です（ローマ字表記の頭文字順）。")
         (str (count people) " people in this release, ordered by the initial of the romanized name."))]
    (into [:ul {:class "cols"}]
          (map (fn [{:keys [person-id label count*]}]
                 [:li [:a {:href (str "/authors/" person-id)} label]
                  [:span {:class "by"} (str " (" count* ")")]])
               people))]))

(defn- author-page [{:keys [label romaji person-id person]} entries]
  (chrome
   label
   [[:h1 label]
    (when romaji [:p {:class "reading" :lang "ja-Latn"} romaji])
    [:dl {:class "facts"}
     [:dt (bilingual "人物ID" "Person ID")]
     [:dd [:code {:class "value"} person-id]]]
    (mapcat (fn [[relation works]]
              [[:h2 (str relation " (" (count works) ")")]
               (work-list works label)])
            entries)
    (bulk-section
     "この人物が関わる作品を、役割を問わず一括取得できます。"
     "Every work this person contributed to, in any role, in one archive."
     (map (juxt #(naming/author-bundle-path person person-id %) identity)
          naming/bulk-artifact-types))]))

(defn- title-index [rows]
  (chrome
   "作品名から探す"
   [[:h1 (bilingual "作品名から探す" "Titles by reading")]
    [:p (bilingual
         "作品名の読みの頭文字で分類しています。読みの記録がない作品は「その他」に入ります。"
         "Grouped by the first kana of the title reading; works with no recorded reading fall under その他.")]
    (into [:ul {:class "cols"}]
          (map (fn [{:keys [key label count*]}]
                 [:li [:a {:href (str "/titles/" key)} label]
                  [:span {:class "by"} (str " (" count* ")")]])
               rows))]))

(defn- title-row-page [label works]
  (chrome
   (str "作品名 " label)
   [[:h1 label]
    [:p (bilingual (str "全" (count works) "作品（読みの順）。")
                   (str (count works) " works, ordered by reading."))]
    (work-list works)]))

(defn- ndc-index [classes]
  (chrome
   "NDC分類から探す"
   [[:h1 (bilingual "NDC分類から探す" "By NDC class")]
    [:p (bilingual
         "日本十進分類法（NDC）の第一次区分です。分類のない作品と児童書のK記号は「その他」にまとめています。"
         "Nippon Decimal Classification main classes. Unclassified works and Aozora Bunko's K-prefixed children's codes are grouped under その他.")]
    (into [:ul {:class "plain"}]
          (map (fn [{:keys [key ja en count*]}]
                 [:li [:a {:href (str "/ndc/" key)} (bilingual (str key " " ja) en)]
                  [:span {:class "by"} (str " (" count* ")")]])
               classes))]))

(defn- ndc-class-page [{:keys [key ja en]} works]
  (chrome
   (str "NDC " key " " ja)
   [[:h1 (bilingual (str key " " ja) en)]
    [:p (bilingual (str "全" (count works) "作品（著者別）。") (str (count works) " works, by author."))]
    (if (seq works) (author-groups works) (work-list works))
    (bulk-section
     (str "この分類の" (count works) "作品を一括取得できます。")
     (str "All " (count works) " works in this class, in one archive.")
     (map (juxt #(naming/ndc-bundle-path key %) identity)
          naming/bulk-artifact-types))]))

(defn- source-edition-line
  "One 底本, shown the way Aozora Bunko recorded it. 初版発行年 is a free-form
  publication history rather than a year, as `1981（昭和56）年3月20日` is, and
  914 values carry a printing history after that, so it is shown verbatim and
  nothing is appended to it. Every recorded value already ends in its own
  年, 月 or 日."
  [{:strs [title publisher first_edition_year]}]
  (string/join "、" (remove string/blank?
                           [(when-not (string/blank? title) (str "『" title "』"))
                            publisher
                            first_edition_year])))

(defn- work-page
  "One work's bibliography page. `release` carries the head hex and the
  release DOI, which every citation form on the page must state."
  [release work]
  (let [{:keys [head-hex]} release
        {:strs [slug title title_reading subtitle original_title first_published
                orthographic_style ndc card_url source_content_hash
                contributors source_editions]} work
        works-standing (get work "rights")
        trailing-bytes (get work "trailing_bytes_after_archive")]
    (chrome
     title
     [[:h1 title]
      (when-not (string/blank? title_reading)
        [:p {:class "reading"} title_reading])
      (when-not (string/blank? subtitle) [:p subtitle])

      [:dl {:class "facts"}
       (mapcat (fn [contributor]
                 [[:dt (get contributor "relation_to_work")]
                  [:dd [:a {:href (str "/authors/" (get contributor "person_id"))}
                        (naming/person-label contributor)]
                   (when-let [romaji (naming/person-name-romaji contributor)]
                     [:span {:class "by" :lang "ja-Latn"} (str " " romaji)])]])
               contributors)
       (when-not (string/blank? original_title)
         (list [:dt (bilingual "原題" "Original title")] [:dd original_title]))
       ;; A collection carries one statement per constituent piece, up to
       ;; 414 of them. One `dd` each, so a reader can see where each ends
       ;; rather than reading them as one sentence.
       (when-not (string/blank? first_published)
         (cons [:dt (bilingual "初出" "First published")]
               (map (fn [statement] [:dd statement])
                    (string/split-lines first_published))))
       (when-not (string/blank? orthographic_style)
         (list [:dt (bilingual "文字遣い" "Orthography")] [:dd orthographic_style]))
       (when-not (string/blank? ndc)
         (list [:dt (bilingual "分類" "NDC")] [:dd ndc]))
       (when (seq source_editions)
         (list [:dt (bilingual "底本" "Source edition")]
               (into [:dd] (interpose [:br] (map source-edition-line source_editions)))))
       ;; the site-wide statement covers almost every work, and a reader who
       ;; saw only it would take attribution for a request on the works where
       ;; it is a licence condition. So the condition is stated here, on the
       ;; page of the work it binds.
       (when-not (string/blank? works-standing)
         (list [:dt (bilingual "底本の権利" "Rights in the source text")]
               [:dd [:a {:href (rights/works-uri works-standing)} works-standing] " "
                (if (rights/attribution-condition? works-standing)
                  (bilingual "クレジット表示はこのライセンスの条件です。"
                             "Attribution is a condition of this licence.")
                  (bilingual "著作権の存続期間が満了しています。"
                             "The copyright term has expired."))]))
       (when-not (string/blank? card_url)
         (list [:dt (bilingual "青空文庫" "Aozora Bunko card")]
               [:dd [:a {:href card_url} card_url]]))
       [:dt (bilingual "識別子" "Identifier")]
       [:dd [:code {:class "value"} slug]]
       [:dt (bilingual "底本ハッシュ" "Source hash")]
       [:dd [:code {:class "value"} (str "sha256:" source_content_hash)]]
       ;; Standard zip readers reject an archive that carries bytes past its
       ;; end, so a reader who checks this transcription against its source
       ;; meets the rejection first. The count is beside the hash of the bytes
       ;; read because together they locate the fault: in the file Aozora
       ;; Bunko ships, not in what was published from it.
       (when trailing-bytes
         (list [:dt (bilingual "底本アーカイブ" "Source archive")]
               [:dd (bilingual
                     (str "本作品の底本ZIPアーカイブは、末尾に"
                          trailing-bytes "バイトの余剰データを含みます。"
                          "このためunzipなどの一般的な展開ツールはこのファイルを読み込めませんが、"
                          "収録された本文データは破損しておらず、本版はそれをそのまま変換しています。")
                     (str "Aozora Bunko's archive for this work has "
                          trailing-bytes " bytes of trailing data after the end of the archive. "
                          "Standard tools such as unzip reject the file on account of those bytes; "
                          "the archived text is intact, and this edition is converted from it directly."))]))]

      [:section
       [:h2 (bilingual "本文を読む" "Read the text")]
       [:p [:a {:class "read-link" :href (str "/works/" slug "/read")}
            (bilingual "この作品を読む" "Read this work")]]
       [:p (bilingual
            "TEI版から変換した閲覧用表示です。ルビ・外字・傍点・字下げを再現し、縦書き表示にも切り替えられます。"
            "Rendered from the TEI edition with ruby, gaiji, emphasis marks, indentation, and vertical layout support.")]]

      [:section
       [:h2 (bilingual "ダウンロード" "Downloads")]
       (into [:ul {:class "plain"}]
             (map (fn [[artifact-type label class]]
                    (let [name (naming/filename work artifact-type)]
                      [:li (cond-> {} class (assoc :class class))
                       [:a {:href (str "/works/" slug "/" name) :download name} label]
                       [:span {:class "by"} " " [:code {:class "value"} name]]]))
                  ;; the first three are the work itself in three forms; the
                  ;; report is evidence about them, so it sits below a rule
                  ;; rather than reading as a fourth copy of the text
                  [["tei" "TEI XML"]
                   ["plaintext" (bilingual "プレーンテキスト" "Plain text")]
                   ["markdown" "Markdown"]
                   ["tei-validation" (bilingual "検証レポート" "Validation report") "evidence"]]))
       [:p (bilingual
            (str "引用には識別子を使ってください（ファイル名は便宜上のものです）。"
                 "同じデータは、識別子と種別名からなる固定URLでも取得できます：")
            (str "Cite the identifier; the filename is for convenience. The same bytes are also "
                 "served at stable URLs formed from the identifier and artifact type:"))]
       [:p (->> naming/artifact-kinds
                (map (fn [artifact-type]
                       [:code {:class "value"} (str "/works/" slug "/" artifact-type)]))
                (interpose " · "))]]

      [:section
       [:h2 (bilingual "引用" "Citation")]
       [:p (bilingual
            "引用には識別子とリリースを含めてください。作品名と著者名だけでは作品を特定できません。"
            "Include the identifier and release when citing; title and author alone do not uniquely identify a work.")]
       [:p {:class "by"} (bilingual "日本語文献用" "For a Japanese bibliography")]
       [:p [:code {:class "citation" :lang "ja"} (citation/rendered release work)]]
       [:p {:class "by"} (bilingual "英語文献用" "For an English bibliography")]
       [:p [:code {:class "citation"} (citation/rendered-en release work)]]
       [:p {:class "by"}
        (bilingual
         "作品名は日本語表記のまま記載しています。ローマ字表記が必要な場合は、投稿先の規定に従って翻字してください。"
         "Titles are given in Japanese; romanize them according to your venue's style.")]
       [:p (bilingual "文献管理ソフト向け：" "For reference managers:")
        " "
        [:a {:href (str "/works/" slug "/citation.json") :download (str slug ".json")}
         "CSL-JSON"]
        " · "
        [:a {:href (str "/works/" slug "/citation.bib") :download (str slug ".bib")}
         "BibLaTeX"]]
       ;; a download is the wrong shape for the common case, which is pasting
       ;; one entry into a .bib file already open in an editor. Collapsed by
       ;; default, because the link above serves the file.
       [:details {:class "bibtex"}
        [:summary (bilingual "BibLaTeXをそのままコピーする" "Copy the BibLaTeX entry")]
        [:pre (citation/biblatex release work)]]
       ;; COinS: an empty span whose title is an OpenURL context object. This
       ;; is the one metadata form a browser connector reads without
       ;; site-specific configuration, and `genre=bookitem` makes it save the
       ;; work as a section of its 底本 rather than as a book of its own.
       [:span {:class "Z3988" :title (citation/coins release work)}]
       [:p [:a {:href "/citation"} (bilingual "引用のしかた" "How to cite")]]]

      (release-note head-hex nil)])))

(defn- reading-page
  "One work rendered for reading. The page is a projection: it states this,
  identifies the release it was rendered from, and links the TEI file it was rendered
  out of, because a reader who thinks the rendering is wrong needs the bytes
  to check it against."
  [release work {:keys [front body back]}]
  (let [{:keys [head-hex]} release
        {:strs [slug title title_reading]} work]
    (chrome
     title
     {:main-class "read"}
     [[:h1 title]
      (when-not (string/blank? title_reading)
        [:p {:class "reading"} title_reading])
      (when-let [by (naming/byline work)]
        [:p {:class "reading"} by])

      ;; a checkbox, not a script: the toggle has to keep working with
      ;; scripting off, like every other way into this site
      [:p {:class "controls"}
       [:input {:type "checkbox" :id "tategaki"}]
       " "
       [:label {:for "tategaki"} (bilingual "縦書きで読む" "Read vertically")]
       " · "
       [:a {:href (str "/works/" slug "/")}
        (bilingual "書誌・ダウンロード" "Bibliography and downloads")]]

      (when front
        [:details {:class "front"}
         [:summary (bilingual "テキストについての注記" "Notes on the text")]
         (into [:div {:class "tei"}] front)])

      (into [:article {:id "reading" :class "tei" :lang "ja"}] body)

      (when back
        [:section {:class "colophon"}
         [:h2 (bilingual "底本" "Source edition")]
         (into [:div {:class "tei"}] back)])

      [:section {:class "provenance"}
       [:h2 (bilingual "この表示について" "About this rendering")]
       [:p (bilingual
            "本ページは署名済みTEI文書の閲覧用表示です。原本データは「TEI」欄のリンクから取得できます。複数の読みがある箇所は、本文に一方を表示し、もう一方を要素のtitle属性に記載しています。"
            "This page is a reading view of the signed TEI file, which is the authoritative record and is linked under TEI. Where a word carries two readings, one is shown inline and the other in the element's title attribute.")]
       [:dl {:class "facts"}
        [:dt (bilingual "リリース" "Release")]
        [:dd [:a {:href (str "/releases/" head-hex ".json")} [:code head-hex]]]
        [:dt "TEI"]
        [:dd [:a {:href (str "/works/" slug "/tei")} (str "/works/" slug "/tei")]]
        [:dt (bilingual "プレーンテキスト" "Plain text")]
        [:dd [:a {:href (str "/works/" slug "/plaintext")} (str "/works/" slug "/plaintext")]]
        [:dt (bilingual "書誌" "Bibliography")]
        [:dd [:a {:href (str "/works/" slug "/")} (str "/works/" slug "/")]]]
       [:p [:code {:class "citation"} (citation/rendered release work)]]
       ;; the same OpenURL record the bibliography page carries, because this
       ;; is the page a reader is on when they reach for their reference
       ;; manager
       [:span {:class "Z3988" :title (citation/coins release work)}]
       [:p [:a {:href "/citation"} (bilingual "引用のしかた" "How to cite")]]]])))

(defn- withdrawn-page [slug {:strs [reason_code statement]} last-release]
  (chrome
   (str slug " — 取り下げ")
   [[:h1 (bilingual "取り下げられた作品" "Withdrawn work")]
    [:p [:code slug]]
    [:dl {:class "facts"}
     [:dt (bilingual "理由区分" "Reason code")]
     [:dd [:code (or reason_code "—")]]
     [:dt (bilingual "説明" "Statement")]
     [:dd (or statement "—")]
     ;; the two rows above are read out of this file, so the label identifies the
     ;; document a reader would be opening rather than its role in the protocol
     [:dt (bilingual "取り下げの記録" "Withdrawal record")]
     [:dd [:a {:href (str "/withdrawn/" slug ".json")} (str "/withdrawn/" slug ".json")]]]
    [:p (bilingual
         "この作品は現在のリリースでは配布していません。取り下げ以後のリリースには収録されず、それ以前のリリースは署名済みの履歴にそのまま残ります。"
         "This work is not distributed in the current release. Withdrawal stops distribution in subsequent releases; earlier releases in the signed history are unchanged.")]
    (when last-release
      [:p (bilingual "最後に収録されたリリース：" "Last release that contained it:")
       " " [:a {:href (str "/releases/" last-release ".json")} [:code last-release]]])
    [:p [:a {:href "/rights"} (bilingual "権利について" "Rights statement")]]]))

(defn- covered-range
  "The upstream revisions a release covers. A release is minted when the
  corpus changed rather than on every upstream commit, so one release spans
  `(covers_from, upstream_rev]`, which can be several revisions. Genesis
  has no predecessor revision to open its range and shows only its own."
  [{:strs [covers_from upstream_rev]}]
  (if covers_from
    (str (subs covers_from 0 12) " .. " (subs upstream_rev 0 12))
    (subs upstream_rev 0 12)))

(defn- published-change
  "What a release did to the published set against its predecessor: works
  added, works removed, and works whose source bytes changed.

  Counts rather than lists. A corpus release changes a median of two works
  out of 17,602, but a toolchain change changes every one of them, and a page
  listing them all would be larger than the manifests it summarises.

  `source_content_hash` decides `changed`, because it is the fact
  about the work rather than about the toolchain: a release that reruns an
  unchanged source through a new parser publishes different artifacts, and
  saying that every work changed would obscure the two that did."
  [manifest predecessor]
  (let [source-of (fn [m]
                    (into {} (map (juxt #(get % "slug") #(get % "source_content_hash")))
                          (get m "works")))
        before (source-of predecessor)
        now (source-of manifest)]
    {:added (count (remove before (keys now)))
     :removed (count (remove now (keys before)))
     :changed (count (filter (fn [[slug hash]]
                               (when-let [was (get before slug)] (not= was hash)))
                             now))}))

(defn- history-row [[hex manifest] predecessor events]
  (let [event-id (get manifest "governance_event")
        event (some->> event-id verify/id->hex (get events))
        {:keys [added removed changed]} (when predecessor
                                          (published-change manifest predecessor))]
    [:tr
     [:td [:a {:href (str "/releases/" hex ".json")} [:code (subs hex 0 12)]]
      (when event
        (list " "
              [:a {:href (str "/governance/" (verify/id->hex event-id) ".json")}
               (get event "kind")]))]
     [:td [:code (covered-range (get manifest "corpus"))]]
     [:td (str (count (get manifest "works")))]
     [:td (if predecessor (str added) "")]
     [:td (if predecessor (str removed) "")]
     [:td (if predecessor (str changed) "")]]))

(defn- history-page
  "Every release in the chain, newest first, with what it changed.

  Assembled here rather than published as an artifact for the reason
  `adr/0001` gave the citation forms and the catalog CSV: it is a projection
  over signed bytes, and a projection stays correctable. Every number on it
  is recomputed from the manifests the export has already written, so a
  reader who distrusts the page can derive it from the same files.

  The oldest release has no predecessor and so has no change to state; its
  cells are left empty rather than filled with the count of the whole
  corpus, which would read as a release that added everything at once."
  [manifest-seq events]
  (chrome
   "リリースの履歴"
   {:main-class "doc"}
   [[:h1 (bilingual "リリースの履歴" "Release history")]
    [:p (bilingual
         (str "署名付きリリース履歴の一覧です（新しい順）。"
              "各行の件数は、直前のリリースからの差分を公開マニフェストから集計した値です。")
         (str "All releases in the signed history, newest first. Each row shows the "
              "changes relative to the preceding release, computed from the published manifests."))]
    [:table
     [:thead
      [:tr
       [:th (bilingual "リリース" "Release")]
       [:th (bilingual "底本の範囲" "Upstream range")]
       [:th (bilingual "作品数" "Works")]
       [:th (bilingual "追加" "Added")]
       [:th (bilingual "削除" "Removed")]
       [:th (bilingual "変更" "Changed")]]]
     (into [:tbody]
           (map (fn [[entry predecessor]]
                  (history-row entry (second predecessor) events))
                (partition-all 2 1 (manifest-seq))))]
    [:p (bilingual
         "「底本の範囲」は、そのリリースが対象とする青空文庫のリビジョン範囲です。作品アーカイブが追加・更新された時点でリリースを作成するため、1つのリリースが複数のリビジョンを含む場合があります。最初のリリースには比較対象がないため、差分の列は空欄です。"
         (str "The upstream range is the span of Aozora Bunko revisions a release covers. "
              "Releases are created when a work archive is added or updated, so a single release can span "
              "multiple revisions. The initial release has no predecessor, so its change columns are blank."))]]))

(defn- last-release-of
  "The newest release still listing each of `slugs`, in one newest-first pass.

  A withdrawn work's page must state the release it was last published in,
  and the chain is ordered newest first, so the answer for every slug is
  found by walking until none are outstanding. Done once for all of them
  rather than once per slug: a per-slug pass would read the chain again for
  every withdrawal, and reading the chain is the expensive part."
  [manifest-seq slugs]
  (loop [remaining (manifest-seq)
         wanted (set slugs)
         found {}]
    (let [[hex manifest] (first remaining)]
      (if (or (nil? hex) (empty? wanted))
        found
        (let [listed (into #{} (comp (map #(get % "slug")) (filter wanted))
                           (get manifest "works"))]
          (recur (rest remaining)
                 (reduce disj wanted listed)
                 (into found (map (fn [slug] [slug hex])) listed)))))))

(defn- generated-page
  "One of the two pages that state facts read from the release.

  The facts come from the head manifest rather than from a document that could
  disagree with it, which is the reason these two pages are assembled here
  instead of being served from Markdown.

  `:source` identifies the repository document that treats the subject at length.
  The page stands on its own without it: a reader who followed
  `rights.statement_url` out of a detached TEI file has to find the answer
  here, not a forwarding address."
  [{:keys [ja source]} title lead]
  (chrome
   ja
   {:main-class "doc"}
   (concat [[:h1 (bilingual ja title)]] lead
           [[:p {:class "by"}
             (bilingual (str "背景と詳細はリポジトリの" source "にあります。")
                        (str "Background and detail are in " source
                             " in the repository."))]])))

(defn- works-standings
  "The standings the release publishes under, in the order the rights
  page lists them: the public domain first, because it is almost every work,
  then the licences, so a reader meets the common case before the exception."
  [manifest]
  (let [present (into #{} (map #(get % "rights")) (get manifest "works"))]
    (into (filterv present ["public-domain"])
          (sort (disj present "public-domain")))))

(defn- rights-page [document standings {:strs [encoding statement_url]}]
  (generated-page
   document
   "Rights and licensing"
   [[:p (bilingual
         "複製、再頒布、翻案、翻訳、情報解析、公衆送信はいずれも、営利・非営利を問わず自由に行えます。許諾や対価は不要です。ほとんどの作品では、クレジット表示はお願いであって利用の条件ではありません。権利者がクリエイティブ・コモンズ表示ライセンス（CC BY）で公開している作品に限り、クレジット表示はそのライセンスの条件です。各作品の権利状態は作品ページに記載しています。"
         "You may copy, redistribute, adapt, translate, mine and republish everything here, commercially or not, without permission or fee. For almost every work, attribution is requested rather than required. For works published by their rightsholder under a Creative Commons Attribution licence, attribution is a condition of that licence. Each work page states which applies.")]
    ;; the grant as this release states it, rather than as this document
    ;; describes it: a served rights statement must not disagree with the
    ;; signed manifest
    [:dl {:class "facts"}
     [:dt (bilingual "底本の権利状態" "Underlying works")]
     ;; one <dd> per standing: <dl> allows several for one <dt>, and a corpus
     ;; with two rights regimes has to show both under the one term
     (map (fn [standing]
            [:dd [:a {:href (rights/works-uri standing)} standing]])
          standings)
     [:dt (bilingual "符号化のライセンス" "Encoding licence")]
     [:dd [:a {:href (rights/licence-uri encoding)} encoding]]
     [:dt (bilingual "この文書" "This statement")]
     [:dd [:a {:href statement_url} statement_url]]]
    [:p (bilingual
         "これらの項目の値は、このリリースのマニフェストから読み取っています。底本の権利状態は作品ごとに記録されており、このリリースに含まれるすべての区分を列挙しています。"
         "These values are read from this release's manifest. Underlying rights are recorded per work, and every standing present in this release is listed.")]

    [:h2 (bilingual "権利の二つの層" "Two rights layers")]
    [:p (bilingual
         "底本の大部分は、著作権保護期間が満了した青空文庫の作品です。Soranohaはこれらについて著作権その他の権利を有しておらず、主張もしません。青空文庫の「収録ファイルの取り扱い規準」に基づき、著作権が満了した作品のファイルは有償・無償を問わず自由に複製・再頒布・翻案できます。"
         "Most underlying texts are Aozora Bunko works whose copyright term has expired. Soranoha neither holds nor claims any rights in them. Under Aozora Bunko's handling guidelines, files for out-of-copyright works may be freely copied, redistributed and adapted, whether commercially or not.")]
    ;; the second regime. Aozora Bunko has no licence column, so these works
    ;; are identified from the notice their rightsholder wrote into the
    ;; colophon, and that notice is what the standing beside each work records.
    [:p (bilingual
         "残りは著作権が存続しており、権利者がクリエイティブ・コモンズ表示ライセンス（CC BY）で青空文庫に公開している作品です。これらも自由に利用できますが、クレジット表示がライセンス上の条件となります。ライセンスのバージョンは作品ページとTEIヘッダーの双方に記載しています。Soranohaは表示のみを条件とする作品を収録し、非営利・改変禁止・継承の条件が付いた作品は収録していません。"
         "The remaining works are in copyright and published on Aozora Bunko by their rightsholders under a Creative Commons Attribution (CC BY) licence. They may be used just as freely, provided attribution is given as required by the licence. Each work's page and TEI header record the specific licence version. Soranoha includes only attribution-only works; works carrying NonCommercial, NoDerivatives or ShareAlike conditions are excluded.")]
    ;; CC0 can only cover what Soranoha added. In a file whose text is under
    ;; CC BY, the text is not Soranoha's to dedicate, so the paragraph states
    ;; what the waiver covers and what it does not.
    [:p (bilingual
         "Soranohaによる符号化データ（TEIマークアップ、プレーンテキストおよびMarkdownへの変換結果、検証レポート、リリースマニフェスト）は、CC0-1.0によりパブリックドメインに提供します。著作権および関連する権利（データベース権を含む）が生じる範囲において、これをすべて放棄します。この権利放棄はSoranohaが付加した部分にのみ及びます。底本がCC BYの作品では、TEI・プレーンテキスト・Markdown内の本文に元のCC BYライセンスが適用されるため、利用時にはクレジット表示が必要です。"
         "Soranoha's own encoding—the TEI markup, plaintext and Markdown projections, validation reports and release manifests—is dedicated to the public domain under CC0-1.0, waiving all copyright and related database rights to the extent they arise. This waiver applies only to what Soranoha added: for works under CC BY, the text inside the TEI, plaintext and Markdown files remains under CC BY and requires attribution.")]
    ;; Catalog bibliographic metadata derives from Aozora Bunko under CC BY 4.0;
    ;; catalog structure and Soranoha-authored fields remain CC0.
    [:p (bilingual
         "目録（catalog.jsonとcatalog.csv）の書誌情報は、青空文庫がCC BY 4.0で公開している「公開作品情報一覧」に由来します。目録やその派生一覧を再頒布する際は、出典として青空文庫を表示してください（これは本文ではなく書誌データに対する条件です）。目録のスキーマ、Soranohaが付与した識別子、および底本・検証に関する追加項目はCC0-1.0で提供します。"
         "The bibliographic metadata in catalog.json and catalog.csv derives from Aozora Bunko's published catalog (CC BY 4.0). Credit Aozora Bunko when redistributing the catalog or listings derived from it (this obligation applies to the bibliographic metadata, not the texts). The catalog schema, Soranoha identifiers, and validation fields are under CC0-1.0.")]
    ;; Community attribution and textual provenance requests from Aozora Bunko (non-binding).
    [:p (bilingual
         "また青空文庫は、作品名・著者・底本・入力者・校正者を記したクレジット表示を残すこと、および底本や表記を変更した際にその記録を添えることを求めています。Soranohaは各TEI文書のback要素と作品ページにクレジットを保持し、検証レポートと底本との差異を公開しています。再頒布の際にも同様の配慮をお願いします。"
         "Aozora Bunko also asks—without making it a legal condition—that the credit block (work, author, source edition, inputter and proofreader) be retained and that changes to the source edition or notation be documented. Soranoha retains the credit block in each TEI back element and work page, and documents differences in each validation report. We ask redistributors to do the same.")]

    ;; Published corpus terms (CC0/CC-BY) are distinct from toolchain licensing (Apache-2.0/MIT).
    [:p (bilingual
         "公開されたコーパスを再頒布しても、ツールチェーンのライセンス上の義務は生じません。ツールチェーン自体を再頒布する場合にのみ、ソースコードのライセンス（自作部分はApache-2.0、フォークしたパーサークレートは上流から継承したMIT OR Apache-2.0）が適用されます。"
         "Redistributing the published corpus does not trigger the toolchain's licence obligations. Redistributing the toolchain itself does: locally authored code is Apache-2.0, and the forked parser crates remain under their upstream MIT OR Apache-2.0 terms.")]

    [:h2 (bilingual "公開作品に権利をお持ちの方へ" "If you hold rights in a published work")]
    [:p (bilingual
         "Soranohaは、著作権が満了したと判定された作品、および権利者が表示のみを条件とするライセンスで公開している作品のみを収録しています。万一、権利判定に誤りがある場合はご連絡ください。"
         "Soranoha includes only works assessed as out of copyright or published by their rightsholders under an attribution-only licence. If you hold rights in a published work or find an error in our rights assessment, please contact us.")]
    [:p (bilingual
         (str "作品識別子（URLに含まれる" example-identifier "の形式）または青空文庫の図書カードと、権利主張の根拠を添えて、"
              orcid-url "に記載の連絡先までご連絡ください。書式は問いません。")
         (str "Write to the contact address on " orcid-url " with the work identifier (such as "
              example-identifier " in the URL) or the Aozora Bunko card, along with the "
              "basis of the claim. Informal notifications are accepted."))]
    ;; the event's own release is the one that removes the work, so a
    ;; rights holder does not wait for a corpus release that may be weeks
    ;; away; `transact/successor-for-event` builds that manifest
    [:p (bilingual
         "公開を取り下げる場合は、理由を記したガバナンスイベントを含む新しいリリースを即座に公開し、該当作品をworks[]と目録から除外してwithdrawn[]に記録します。追記専用の署名履歴であるため、過去のリリース自体は変更されません。"
         "To withdraw a work, Soranoha immediately publishes a new release containing a governance event that states the reason, removes the work from works[] and the catalog, and records it in withdrawn[]. Because the signed history is append-only, earlier releases remain unchanged.")]

    [:h2 (bilingual "無保証" "No warranty")]
    [:p (bilingual
         "Soranohaは検証レポートと底本との差異を添えて翻刻データを公開していますが、内容の完全性や正確性を保証するものではありません。コーパスは現状有姿で提供します。法律・医療・安全に関わる用途では、各作品のsourceDescに記載された底本と必ず照合してください。"
         "Soranoha publishes transcriptions together with validation reports and recorded source divergences, without any warranty of accuracy. The corpus is provided as-is; do not rely on it for legal, medical or safety purposes without verifying against the source edition named in sourceDesc.")]]))

(def ^:private example-work
  "The work the how-to-cite page is worked through. A literal rather than a
  lookup into the release: the page has to render the same example on every
  activation, and a real catalog entry would change when that work's record
  is corrected. The facts are 蜘蛛の糸's own, so the example matches the page
  a reader reaches by following it."
  {"slug" "000092_000879"
   "title" "蜘蛛の糸"
   "title_reading" "くものいと"
   "orthographic_style" "新字新仮名"
   "source_editions" [{"title" "芥川龍之介全集2"
                       "publisher" "ちくま文庫、筑摩書房"
                       "first_edition_year" "1986（昭和61）年10月28日"}]
   "contributors" [{"person_id" "000879"
                    "family_name" "芥川" "given_name" "竜之介"
                    "family_name_romaji" "Akutagawa" "given_name_romaji" "Ryunosuke"
                    "relation_to_work" "著者"}]})

(defn- citation-page [document {:keys [head-hex doi] :as release} release-name maturity]
  (generated-page
   document
   "Citing Soranoha"
   ;; Citing Soranoha and attributing a work are different obligations, and
   ;; only the second can be a licence condition: CC0 sets none, CC BY does.
   [[:p (bilingual
         "Soranohaの符号化データはCC0のため、コーパスの引用はライセンス上の義務ではなく学術上の慣行です。一方、権利者がCC BYで公開している作品については、作品のクレジット表示がライセンスの条件となります（対象作品のページに明記しています）。引用時はリリースを明記してください。"
         "Soranoha's encoding is CC0, so citing the corpus is a scholarly convention rather than a licence requirement. For works published under CC BY, attributing the work is a licence condition stated on the work's page. Always cite the specific release.")]
    ;; Release names designate a project stage; citations require the immutable release head.
    (when release-name
      [:p (bilingual
           (str "コーパス全体の呼称は" release-name
                (when maturity (str "（" (:ja maturity) "）"))
                "です。呼称は単一のリリースではなく現在の開発段階を指すため、引用にはリリースハッシュを使用してください。")
           (str "The corpus stage is named " release-name
                (when maturity (str " (" (:en maturity) ")"))
                ". The name designates a stage rather than a release; citations use the release hash, which identifies the exact snapshot."))])
    ;; Abbreviated 12-char release prefix without trailing ellipsis so the hash resolves directly.
    [:p {:class "by"} (bilingual "コーパス全体" "The whole corpus")]
    [:p [:code {:class "citation"}
         (str site-name " Aozora Bunko TEI Corpus. Release " (subs head-hex 0 12)
              (when doi (str ". https://doi.org/" doi)))]]
    [:p (bilingual
         "個々の作品の引用形式を、現在のリリースと「蜘蛛の糸」を例に示します。各作品のページには、その作品の引用形式を掲載しています。"
         "Citation forms for one work, shown for this release and 蜘蛛の糸. Every work page provides the same forms filled in for that work.")]
    [:p {:class "by"} (bilingual "日本語文献用" "For a Japanese bibliography")]
    [:p [:code {:class "citation" :lang "ja"}
         (citation/rendered release example-work)]]
    [:p {:class "by"} (bilingual "英語文献用" "For an English bibliography")]
    [:p [:code {:class "citation"}
         (citation/rendered-en release example-work)]]
    [:p [:a {:href "/catalog.json"} "/catalog.json"] " · "
     [:a {:href (str "/releases/" head-hex ".json")} (str "/releases/" head-hex ".json")]]]))

(defn- utf8 ^bytes [^String s] (.getBytes s "UTF-8"))

(defn- json-escape [^String s]
  (str "\""
       (apply str (map (fn [c]
                         (case c
                           \" "\\\""
                           \\ "\\\\"
                           \newline "\\n"
                           \return "\\r"
                           \tab "\\t"
                           (if (< (int c) 0x20)
                             (format "\\u%04x" (int c))
                             c)))
                       s))
       "\""))

(defn- search-index
  "A compact array-of-arrays index: slug, title, reading, byline, and the
  authors' romanized names. Written directly rather than through the
  protocol canonicalizer, which exists to make signed bytes comparable and
  is not applied to a presentation file."
  [head-hex works]
  (str "{\"release\":" (json-escape head-hex) ",\"works\":["
       (string/join
        ","
        (map (fn [work]
               (str "[" (json-escape (get work "slug"))
                    "," (json-escape (or (get work "title") ""))
                    "," (json-escape (or (get work "title_reading") ""))
                    "," (json-escape (or (naming/byline work) ""))
                    "," (json-escape (string/join " " (keep naming/person-name-romaji
                                                            (naming/authors-of work))))
                    "]"))
             works))
       "]}"))

(defn people
  "Every person in the release, keyed by person_id.

  `:works` is every work they contributed to and `:by-relation` is the same
  works split by the relation the catalog records; both hold a work once and
  keep catalog order. A catalog can list one person twice for one work,
  holding two relations or repeating one, and neither is a reason to list the
  work twice: the works of a person are works, not contributor rows.

  One definition because the author pages and the bulk selections describe the
  same release from the same catalog, and a person's works have to mean the
  same thing on the page and in the archive it links to. Contributors of a
  work are reached consecutively here, so the last entry in a list is enough
  to recognise a repeat."
  [works]
  (let [add (fn [ws work] (if (identical? (peek ws) work) ws (conj (or ws []) work)))]
    (reduce
     (fn [acc work]
       (reduce (fn [acc contributor]
                 (let [id (get contributor "person_id")]
                   (-> acc
                       (assoc-in [id :person] (get-in acc [id :person] contributor))
                       (update-in [id :works] add work)
                       (update-in [id :by-relation (get contributor "relation_to_work")]
                                  add work))))
               acc
               (get work "contributors")))
     (sorted-map)
     works)))

(defn pages
  "Every browse file for one release, as a sequence of `[path bytes]` pairs.

  Deterministic: the same manifest, catalog, events and TEI bytes in, the
  same pairs out in the same order, so the exporter's reuse check treats
  these files exactly like chain content.

  A sequence rather than a map because of the reading pages. There is one per
  work, each holding a whole rendered text, and the corpus is large enough
  that materialising them together would cost more memory than the whole
  serving tree costs on disk. The caller consumes them one at a time and
  writes each out; nothing here holds a page after it has been handed over.

  `tei` is a function from slug to that work's published TEI bytes. It
  allows a reading page without a second copy of the text: the bytes it
  returns are the artifact the manifest references and the export has
  already written.

  `manifest-seq` is a function of no arguments returning the chain newest
  first as `[hex manifest]`, and it is a function for the same reason `tei`
  is. A published manifest is about nine megabytes at the current corpus, and
  three places here read the chain, so a realized sequence would be held for
  the whole export and would cost the chain's length times that. Each caller
  requests its own sequence, walks it once and releases it, which bounds the
  chain's contribution to what one manifest costs."
  [{:keys [head-hex manifest-seq catalog events tei doi release-name maturity]}]
  (let [release {:head-hex head-hex :doi doi}
        maturity (maturity/label! maturity)
        head (second (first (manifest-seq)))
        works (get catalog "works")
        by-slug (into {} (map (juxt #(get % "slug") identity)) works)
        withdrawn (get head "withdrawn")
        people-index (people works)
        reading-key (fn [work] [(or (some-> (get work "title_reading") hiragana) "")
                                (get work "slug")])
        page (fn [path ^String content] [path (utf8 content)])
        by-kana (group-by kana-row-key works)
        by-ndc (group-by ndc-class-key works)
        entries (document-entries)
        by-route (into {} (map (juxt :route identity)) docs/generated)]
    (concat
     [(page "style.css" stylesheet)
      (page "search.js" search-js)
      (page "copy.js" copy-js)
      (page "search-index.json" (search-index head-hex works))
      (page "index.html" (landing head-hex works (count withdrawn) release-name maturity entries))
      (page "rights.html" (rights-page (by-route "rights")
                                       (works-standings head)
                                       (get head "rights")))
      (page "citation.html" (citation-page (by-route "citation") release release-name maturity))
      (page "history.html" (history-page manifest-seq events))

      (page "authors/index.html"
            (author-index (mapv (fn [[id {:keys [person works]}]]
                                  {:person-id id
                                   :label (naming/person-label person)
                                   :count* (count works)})
                                (sort-by (fn [[id {:keys [person]}]]
                                           [(or (naming/person-name-romaji person) "￿")
                                            (or (naming/person-name-ja person) "")
                                            id])
                                         people-index))))

      (page "titles/index.html"
            (title-index (mapv (fn [[key label _]]
                                 {:key key :label label :count* (count (get by-kana key))})
                               kana-rows)))

      (page "ndc/index.html"
            (ndc-index (mapv (fn [[key ja en]]
                               {:key key :ja ja :en en :count* (count (get by-ndc key))})
                             ndc-classes)))]

     ;; the project's own documentation, rendered from the repository files it
     ;; is reviewed in, and the files those documents link to
     (map (fn [entry] (page (str (:route entry) ".html") (document-page entries entry)))
          entries)

     (map (fn [{:keys [route path]}] [route (docs/read-bytes path)]) (docs/verbatim-files))

     (map (fn [[id {:keys [person by-relation]}]]
            (page (str "authors/" id ".html")
                  (author-page {:label (naming/person-label person)
                                :romaji (naming/person-name-romaji person)
                                :person-id id
                                :person person}
                               (sort-by key
                                        (update-vals by-relation
                                                     #(vec (sort-by reading-key %)))))))
          people-index)

     (map (fn [[key label _]]
            (page (str "titles/" key ".html")
                  (title-row-page label (vec (sort-by reading-key (get by-kana key))))))
          kana-rows)

     (map (fn [[key ja en]]
            (page (str "ndc/" key ".html")
                  (ndc-class-page {:key key :ja ja :en en}
                                  (vec (sort-by reading-key (get by-ndc key))))))
          ndc-classes)

     ;; the works, each with its bibliography page and its reading page. This
     ;; is the part that must stay lazy.
     (mapcat (fn [work]
               (let [slug (get work "slug")]
                 [(page (str "works/" slug "/index.html") (work-page release work))
                  ;; the two records a reference manager imports. Files rather
                  ;; than a service because there is no runtime here, and they
                  ;; cost about a kilobyte each against a corpus whose texts
                  ;; are three orders of magnitude larger.
                  (page (str "works/" slug "/citation.json")
                        (json/write-deterministic-json-str
                         [(citation/csl-json-value release work)]))
                  (page (str "works/" slug "/citation.bib")
                        (citation/biblatex release work))
                  (page (str "works/" slug "/read.html")
                        (reading-page
                         release work
                         (reading/render (String. ^bytes (tei slug) "UTF-8"))))]))
             works)

     ;; then the withdrawn works, which get a withdrawal page and no reading page
     (let [gone (remove #(contains? by-slug (get % "slug")) withdrawn)
           last-release (delay (last-release-of manifest-seq (map #(get % "slug") gone)))]
       (map (fn [{:strs [slug event]}]
              (let [entry (->> (get (get events (verify/id->hex event)) "entries")
                               (filter #(= slug (get % "slug")))
                               first)]
                (page (str "works/" slug "/index.html")
                      (withdrawn-page slug entry (get @last-release slug)))))
            gone)))))
