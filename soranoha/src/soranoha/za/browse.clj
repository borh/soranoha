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
            [soranoha.core.ndc :as ndc]
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
    "dl.facts dd { grid-column: 2; margin: 0; overflow-wrap: anywhere; }"
    "ul.works, ul.plain { list-style: none; padding: 0; }"
    "ul.works li { padding-block: .35rem; border-block-end: 1px solid var(--rule); }"
    "ul.works li.empty { color: var(--muted); }"
    "ul.works .by { color: var(--muted); font-size: .9rem; }"
    ;; smaller than the byline, since it is only needed on the rows that
    ;; would otherwise look identical
    "ul.works .variant, ul.works .k { color: var(--muted); font-size: .85rem; }"
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
    "button.copy { margin-inline-start: .4em; padding: .2em .3em; font: inherit;"
    "  color: var(--muted); cursor: pointer;"
    "  background: var(--bg); border: 1px solid var(--rule); border-radius: 3px;"
    "  vertical-align: middle; display: inline-flex; align-items: center; justify-content: center; }"
    "button.copy svg { display: block; inline-size: 14px; block-size: 14px; }"
    "button.copy:hover, button.copy:focus-visible { color: var(--ink);"
    "  background: var(--panel); }"
    "button.copy.done { color: var(--link); border-color: var(--link); }"
    ;; a code box embeds the copy button in its top-right corner
    ".code-box { position: relative; margin-block: 1rem; }"
    ".code-box pre, .code-box code.citation { margin: 0; padding-inline-end: 2.8rem;"
    "  white-space: pre-wrap; overflow-wrap: anywhere; }"
    ".code-box button.copy { position: absolute; inset-block-start: .4rem; inset-inline-end: .4rem;"
    "  margin: 0; padding: .35em; text-align: center; }"
    "details.bibtex .code-box { margin-block: .5rem 0; }"
    "@media (max-width: 32rem) { dl.facts { grid-template-columns: 1fr; }"
    "  dl.facts dd { grid-column: 1; margin-inline-start: 1rem; } ul.cols { columns: 1; } }"

    ;; the documentation pages. Prose the project wrote, so unlike the reading
    ;; view these are ordinary elements and take ordinary rules.
    "main.doc h3 { font-size: 1.05rem; margin-block: 1.5rem .5rem; }"
    "main.doc table { border-collapse: collapse; inline-size: 100%; max-inline-size: 100%;"
    "  font-size: .9rem; margin-block: 1rem; }"
    "@media (max-width: 40rem) { main.doc table { display: block; overflow-x: auto; } }"
    "main.doc th, main.doc td { border: 1px solid var(--rule); padding: .3rem .5rem;"
    "  text-align: start; vertical-align: top; }"
    "main.doc th { background: var(--sky); }"
    "main.doc pre { background: var(--panel); padding: .6rem .8rem; border-radius: 3px;"
    "  font-size: .85rem; line-height: 1.5; white-space: pre-wrap; overflow-wrap: anywhere; }"
    "main.doc pre code { font-size: 1em; }"
    ".tok-c { color: var(--muted); font-style: italic; }"
    ".tok-tag { color: var(--link); font-weight: 500; }"
    ".tok-attr { color: var(--muted); }"
    ".tok-str { color: #1c6d59; }"
    ".tok-kw { color: var(--link); font-weight: 500; }"
    ".tok-fn { color: #23586e; }"
    ".tok-num { color: #824729; }"
    ".tok-meta { color: var(--muted); }"
    "@media (prefers-color-scheme: dark) {"
    "  .tok-str { color: #5bc4a0; } .tok-fn { color: #6db2d6; } .tok-num { color: #d68965; } }"
    "main.doc blockquote { margin-block: 1rem; margin-inline: 0; padding-inline-start: 1rem;"
    "  border-inline-start: 3px solid var(--rule); color: var(--muted); }"
    "main.doc img { max-inline-size: 100%; block-size: auto; display: block; margin-block: 1.5rem; }"
    "nav.siblings { display: block; margin-block-start: 3rem; border-block-start: 1px solid var(--rule);"
    "  padding-block-start: 1rem; font-size: .9rem; }"
    "nav.siblings p { margin-block: .5rem; }"

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
    "      if (hits[i][5]) {"
    "        var k = document.createElement('span');"
    "        k.className = 'k';"
    "        k.textContent = ' 児童書 ';"
    "        var ken = document.createElement('span');"
    "        ken.className = 'en';"
    "        ken.lang = 'en';"
    "        ken.textContent = \"/ children's\";"
    "        k.appendChild(ken);"
    "        li.appendChild(k);"
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
    "        return [w[0], w[1], w[2], w[3], fold(w[1] + ' ' + w[2] + ' ' + w[3] + ' ' + compact + ' ' + w[0] + ' ' + w[4]), w[5]];"
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
    "  var idleIcon = '<svg viewBox=\"0 0 16 16\" width=\"14\" height=\"14\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.5\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\"><rect x=\"5.5\" y=\"5.5\" width=\"8\" height=\"8\" rx=\"1.5\"/><path d=\"M10.5 3.5V3a1.5 1.5 0 0 0-1.5-1.5H3A1.5 1.5 0 0 0 1.5 3v6A1.5 1.5 0 0 0 3 10.5h.5\"/></svg>';"
    "  var doneIcon = '<svg viewBox=\"0 0 16 16\" width=\"14\" height=\"14\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\"><polyline points=\"3.5 8.5 6.5 11.5 12.5 4.5\"/></svg>';"
    "  var idleLabel = 'コピー / Copy', doneLabel = 'コピーしました / Copied';"
    "  function attach(node, text) {"
    "    var anchor = node.parentNode;"
    "    if (!anchor) { return; }"
    "    if (anchor.tagName === 'A') { node = anchor; anchor = anchor.parentNode; }"
    "    var button = document.createElement('button'), timer = null;"
    "    button.type = 'button';"
    "    button.className = 'copy';"
    "    button.innerHTML = idleIcon;"
    "    button.setAttribute('aria-label', idleLabel);"
    "    button.title = idleLabel;"
    "    button.addEventListener('click', function () {"
    "      clip.writeText(text).then(function () {"
    "        if (timer) { clearTimeout(timer); }"
    "        button.innerHTML = doneIcon;"
    "        button.setAttribute('aria-label', doneLabel);"
    "        button.title = doneLabel;"
    "        button.classList.add('done');"
    "        timer = setTimeout(function () {"
    "          button.innerHTML = idleIcon;"
    "          button.setAttribute('aria-label', idleLabel);"
    "          button.title = idleLabel;"
    "          button.classList.remove('done');"
    "          timer = null;"
    "        }, 1500);"
    "      });"
    "    });"
    "    if (node.tagName === 'PRE' || (node.classList && node.classList.contains('citation'))) {"
    "      var wrap = document.createElement('div');"
    "      wrap.className = 'code-box';"
    "      anchor.insertBefore(wrap, node);"
    "      wrap.appendChild(node);"
    "      wrap.appendChild(button);"
    "    } else {"
    "      anchor.insertBefore(button, node.nextSibling);"
    "    }"
    "  }"
    "  var nodes = document.querySelectorAll('pre, code.citation, code.value'), i;"
    "  for (i = 0; i < nodes.length; i++) {"
    "    attach(nodes[i], nodes[i].textContent);"
    "  }"
    "})();"]))

(def ^:private favicon-svg
  "A leaf (葉) against sky (空), in the site palette for both colour schemes."
  (string/join
   "\n"
   ["<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 32 32\">"
    "  <defs>"
    "    <linearGradient id=\"leafGrad\" x1=\"0%\" y1=\"0%\" x2=\"100%\" y2=\"100%\">"
    "      <stop offset=\"0%\" stop-color=\"#2ca17d\"/>"
    "      <stop offset=\"100%\" stop-color=\"#145241\"/>"
    "    </linearGradient>"
    "    <linearGradient id=\"skyGrad\" x1=\"0%\" y1=\"0%\" x2=\"100%\" y2=\"100%\">"
    "      <stop offset=\"0%\" stop-color=\"#2a8eb6\"/>"
    "      <stop offset=\"100%\" stop-color=\"#135b79\"/>"
    "    </linearGradient>"
    "  </defs>"
    "  <style>"
    "    .bg { fill: #f5fbf9; }"
    "    .leaf { fill: url(#leafGrad); }"
    "    .sky { fill: url(#skyGrad); }"
    "    .vein { stroke: #f5fbf9; stroke-width: 1.5; stroke-linecap: round; fill: none; }"
    "    .vein-sub { stroke: #f5fbf9; stroke-width: 1.2; stroke-linecap: round; opacity: 0.8; }"
    "    @media (prefers-color-scheme: dark) {"
    "      .bg { fill: #0f1c1a; }"
    "      .vein, .vein-sub { stroke: #0f1c1a; }"
    "    }"
    "  </style>"
    "  <rect class=\"bg\" width=\"32\" height=\"32\" rx=\"7\"/>"
    "  <path class=\"leaf\" d=\"M 5 27 C 5 15, 14 5, 27 5 C 23 15, 16 22, 5 27 Z\"/>"
    "  <path class=\"sky\" d=\"M 5 27 C 16 22, 23 15, 27 5 C 28 17, 19 26, 5 27 Z\"/>"
    "  <path class=\"vein\" d=\"M 4 28 C 12 21, 19 14, 27 5\" fill=\"none\"/>"
    "  <line class=\"vein-sub\" x1=\"12\" y1=\"20\" x2=\"8\" y2=\"17\"/>"
    "  <line class=\"vein-sub\" x1=\"16\" y1=\"16\" x2=\"11\" y2=\"12\"/>"
    "  <line class=\"vein-sub\" x1=\"20\" y1=\"12\" x2=\"16\" y2=\"8\"/>"
    "</svg>"]))

(defn- bilingual
  "Japanese label with its English counterpart marked as English, so a screen
  reader and a translation tool both get the language right."
  [ja en]
  (list ja " " [:span {:class "en" :lang "en"} (str "/ " en)]))

(defn- bilingual-title
  "The `title` element's text: Japanese, then the English when it differs."
  [ja en]
  (if (and (seq ja) (seq en) (not= ja en))
    (str ja " / " en)
    (or ja en "")))

(defn- head-nodes [title]
  [[:meta {:charset "utf-8"}]
   [:meta {:name "viewport" :content "width=device-width, initial-scale=1"}]
   [:title (str title " — " site-name)]
   [:link {:rel "icon" :type "image/svg+xml" :href "/favicon.svg"}]
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
       [:a {:href "/docs"} (bilingual "解説" "Docs")]]]
     (into [:main (cond-> {} main-class (assoc :class main-class))] body)
     [:footer
      [:p (bilingual
           "本文は青空文庫の収録作品で、符号化データはCC0 1.0で提供しています。作品ごとの権利の状態は各作品のページに記載しています。公開マニフェストは電子署名されており、各ファイルのハッシュ値を記録しています。本サイトはその署名済みの内容を表示しています。"
           "Texts are works from Aozora Bunko; the encoding is CC0 1.0. Each work's page states the rights standing of that work. The release manifest is signed and records the hash of every file; this site displays the signed content.")]
      [:p {:class "editor"}
       (bilingual "編纂：Bor Hodošček（大阪大学）"
                  "Corpus Editor: Bor Hodošček (The University of Osaka)")
       " · "
       [:a {:href orcid-url :rel "external"} "ORCID"]]
      [:p [:a {:href "/catalog.json"} "/catalog.json"] " · "
       [:a {:href "/releases/HEAD"} "/releases/HEAD"] " · "
       [:a {:href "https://github.com/borh/soranoha" :rel "external"} "GitHub"] " · "
       [:a {:href "https://www.aozora.gr.jp/" :rel "external"} "青空文庫"]]]
     [:script {:src "/copy.js"}]])))

;; Personal names, bylines and download filenames are rendered by
;; soranoha.za.naming, which the bulk archives share: a name on a page and
;; the name inside a ZIP must agree.

(def ^:private kana-rows
  [["a" "あ行" "A-row" "あいうえおぁぃぅぇぉゔ"]
   ["ka" "か行" "Ka-row" "かきくけこがぎぐげごゕゖ"]
   ["sa" "さ行" "Sa-row" "さしすせそざじずぜぞ"]
   ["ta" "た行" "Ta-row" "たちつてとだぢづでどっ"]
   ["na" "な行" "Na-row" "なにぬねの"]
   ["ha" "は行" "Ha-row" "はひふへほばびぶべぼぱぴぷぺぽ"]
   ["ma" "ま行" "Ma-row" "まみむめも"]
   ["ya" "や行" "Ya-row" "やゆよゃゅょ"]
   ["ra" "ら行" "Ra-row" "らりるれろ"]
   ["wa" "わ行" "Wa-row" "わをんゎ"]
   ["other" "その他" "Other" ""]])

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
    (or (some (fn [[key _ _ members]]
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
   ["other" "分類なし" "Unclassified"]])

(defn- ndc-codes
  "NDC classification codes in catalog order, omitting the `K` prefix used in
  Aozora Bunko for children's literature (e.g. `K913`). The numeric digits
  determine classification placement, with children's literature tracked as
  a separate facet."
  [work]
  (map #(string/replace % #"^K" "")
       (rest (string/split (or (get work "ndc") "") #"\s+"))))

(defn- first-ndc-code [work]
  (first (ndc-codes work)))

(defn- ndc-label
  "Bilingual label node for a three-digit NDC9 code, or nil when unlisted
  in the classification table."
  [digits]
  (when-let [{:strs [ja en]} (ndc/labels digits)]
    (bilingual ja (or en ja))))

(defn childrens?
  "Returns true when the work's primary NDC classification includes the `K`
  prefix, designating children's literature."
  [work]
  (boolean (re-find #"^NDC K" (or (get work "ndc") ""))))

(defn ndc-class-key
  "First digit of the first NDC code, or `other` for a work with no code."
  [work]
  (let [head (some-> (first-ndc-code work) first str)]
    (if (some #(= head (first %)) (butlast ndc-classes)) head "other")))

(defn ndc-subclass-key
  "The three-digit code the work's first NDC code begins with, or nil for a
  work with no code."
  [work]
  (some->> (first-ndc-code work) (re-find #"^\d{3}")))

(defn ndc-subclasses
  "Groups works by three-digit NDC subclass, returned as `[subclass-code works]`
  pairs sorted by code. Only subclasses with at least one work are included."
  [works]
  (sort-by first (group-by ndc-subclass-key (filter ndc-subclass-key works))))

(defn- source-edition-title [work]
  (get (first (get work "source_editions")) "title"))

(defn- variant-facts
  "Distinguishing bibliographic facts for works sharing identical author and title.

  Multiple transcriptions of a single text frequently appear in Aozora Bunko
  (for instance, three separate transcriptions of Higuchi Ichiyō's わかれ道).
  These transcriptions are differentiated primarily by orthographic style
  (historical versus modernized kana usage) and source edition (底本).

  Orthographic style distinguishes most groups; the source edition is added
  when orthography alone does not differentiate the works."
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
     (when (childrens? work)
       [:span {:class "k"} " " (bilingual "児童書" "children's")])
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

(defn- person-order
  "Sort key for a person: romanised name, then Japanese name, then id. A
  person with no romanised name sorts after every person who has one."
  [person]
  [(or (naming/person-name-romaji person) "\uFFFF")
   (or (naming/person-name-ja person) "")
   (get person "person_id")])

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
                  [0 (mapv person-order (naming/authors-of (first group)))]))]
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
        (str "各ZIPアーカイブの最上階層に、各作品の識別子・作品名・著者・底本・初出・分類・文字数・リリースなどを収めたcatalog.csvを同梱しています。"
             "引用には識別子・底本ハッシュ・リリースの列を使用してください（ファイル名は利便性のためのものです）。")
        (str "Included at the root of each ZIP archive is catalog.csv, with bibliographic details for each work: "
             "identifier, title, author, source edition, first publication, classification, character count and release. "
             "For citations, use the identifier, source hash and release; filenames are provided for convenience."))]])

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
   [:p (bilingual "ほかの解説：" "Other documents:")
    " "
    (interpose
     " · "
     (for [{:keys [route ja en]} entries
           :when (not= route current)]
       [:a {:href (str "/" route)} (bilingual ja en)]))]
   [:p [:a {:href "/docs"} (bilingual "解説一覧" "All documents")]]])

(def ^:private document-groups
  [[:guide "入門" "Guides"]
   [:encoding "符号化の仕様" "Encoding"]
   [:evidence "検証と収録基準" "Validation and assessment"]])

(defn- docs-index-page
  "Every served document under its `:group` heading, then the rights and
  citation pages."
  [entries]
  (chrome
   (bilingual-title "解説一覧" "Documentation")
   {:main-class "doc"}
   (concat
    [[:h1 (bilingual "解説一覧" "Documentation")]]
    (for [[group group-ja group-en] document-groups
          :let [members (filter #(= group (:group %)) entries)]
          :when (seq members)]
      [:section
       [:h2 (bilingual group-ja group-en)]
       [:ul {:class "plain"}
        (for [{:keys [route ja en]} members]
          [:li [:a {:href (str "/" route)} (bilingual ja en)]])]])
    [[:section
      [:h2 (bilingual "権利と引用" "Rights and citation")]
      [:ul {:class "plain"}
       (for [{:keys [route ja en]} docs/generated]
         [:li [:a {:href (str "/" route)} (bilingual ja en)]])]]])))

(defn- document-page [entries {:keys [route path text ja en]}]
  (chrome
   (bilingual-title ja en)
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
   (bilingual-title "青空文庫TEIコーパス" "Aozora Bunko TEI corpus")
   [[:h1 (bilingual "青空文庫TEIコーパス" "Aozora Bunko TEI corpus")]
    [:p (bilingual
         (str "青空文庫の作品をTEI P5に変換し、ハッシュ値で一意に特定できる署名付きリリースとして公開しています。"
              "各作品につき、TEI XML、プレーンテキスト、Markdown、検証レポートの4形式を提供しています。")
         (str "Works from Aozora Bunko converted to TEI P5 and published in signed "
              "releases identified by hash. Each work is provided in four formats: "
              "TEI XML, plain text, Markdown and a validation report."))]
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
     [:h2 (bilingual "直接アクセス" "Direct access")]
     [:dl {:class "facts"}
      [:dt [:a {:href "/catalog.json"} "/catalog.json"]]
      [:dd (bilingual "全作品の書誌目録（署名済みリリースの一部）"
                      "Catalog of every work in this release; part of the signed record.")]
      [:dt [:a {:href "/releases/HEAD"} "/releases/HEAD"]]
      [:dd (bilingual "現在のリリースのハッシュ" "Hash of the current release.")]
      [:dt [:a {:href "/releases/latest"} "/releases/latest"]]
      [:dd (bilingual "現在のリリースのマニフェスト" "Manifest of the current release.")]
      [:dt [:code "/works/<識別子>/tei"]]
      [:dd (bilingual "個別作品のファイル（plaintext・markdown・tei-validationも同一のURL形式で取得できます）"
                      "Files for an individual work; plaintext, markdown, and tei-validation follow the same URL pattern.")]]]

    (bulk-section
     "本リリースの全作品を一つのZIPアーカイブにまとめています。著者別・NDC分類別のアーカイブは各一覧ページからダウンロードできます。"
     "All works in this release collected in a single archive. Bulk archives by author and NDC class are available on their respective index pages."
     (corpus-archives))

    [:section
     [:h2 (bilingual "解説" "Documentation")]
     [:p (bilingual
          "符号化の方針、識別子の体系、収録判定の基準をまとめています。概要は「はじめに」を参照してください。"
          "How the texts are encoded, how works are identified, and how they are admitted. For an overview, see Start here.")]
     [:ul {:class "cols plain"}
      (for [{:keys [route ja en]} entries]
        [:li [:a {:href (str "/" route)} (bilingual ja en)]])]
     [:p [:a {:href "/docs"} (bilingual "解説一覧" "All documents")]]]

    [:section
     [:h2 (bilingual "権利と引用" "Rights and citation")]
     [:p (bilingual
          "底本とした作品は、著作権の保護期間が満了しているか、または権利者がクリエイティブ・コモンズ表示ライセンス（CC BY）で公開しているものです。Soranohaの符号化データはCC0 1.0で提供しています。作品ごとの権利状態は各作品のページとcatalog.jsonに記載しています。"
          "The source works are either out of copyright or published by their rightsholder under a Creative Commons Attribution (CC BY) licence. Soranoha's encoding is CC0 1.0. The rights status of each work is recorded on its page and in catalog.json.")]
     [:p [:a {:href "/rights"} (bilingual "権利について" "Rights and licensing")] " · "
      [:a {:href "/citation"} (bilingual "引用のしかた" "Citing Soranoha")]]]

    (release-note head-hex (count works))
    [:script {:src "/search.js"}]]))

(defn- author-index [people]
  (chrome
   (bilingual-title "著者一覧" "Authors")
   [[:h1 (bilingual "著者一覧" "Authors")]
    [:p (bilingual
         (str "本リリースの収録作品に関わる人物" (count people) "名（ローマ字表記のアルファベット順）。")
         (str (count people) " contributors in this release, ordered alphabetically by romanized name."))]
    (into [:ul {:class "cols"}]
          (map (fn [{:keys [person-id label count*]}]
                 [:li [:a {:href (str "/authors/" person-id)} label]
                  [:span {:class "by"} (str " (" count* ")")]])
               people))]))

(defn- author-page [{:keys [label romaji person-id person]} entries]
  (chrome
   (bilingual-title label romaji)
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
   (bilingual-title "作品名から探す" "Titles by reading")
   [[:h1 (bilingual "作品名から探す" "Titles by reading")]
    [:p (bilingual
         "作品名の読みの頭文字（五十音順）で分類しています。読みの記録がない作品は「その他」にまとめています。"
         "Grouped by the initial kana of the title reading; works without a recorded reading are placed under その他 (Other).")]
    (into [:ul {:class "cols"}]
          (map (fn [{:keys [key label en count*]}]
                 [:li [:a {:href (str "/titles/" key)} (bilingual label en)]
                  [:span {:class "by"} (str " (" count* ")")]])
               rows))]))

(defn- title-row-page [{:keys [label en]} works]
  (chrome
   (bilingual-title (str "作品名 " label) (str "Titles: " en))
   [[:h1 (bilingual label en)]
    [:p (bilingual (str "全" (count works) "作品（五十音順）。")
                   (str (count works) " works, ordered by title reading."))]
    (work-list works)]))

(defn- work-count-line
  "The number of works on a class or subclass page and, where any are
  children's material, how many."
  [works]
  (let [k (count (filter childrens? works))]
    [:p (bilingual (str "全" (count works) "作品（著者別）" (when (pos? k) (str "、うち児童書" k "作品")) "。")
                   (str (count works) " works, arranged by author" (when (pos? k) (str ", including " k " works of children's literature")) "."))]))

(defn- ndc-index [classes k-count]
  (chrome
   (bilingual-title "NDC分類から探す" "By NDC class")
   [[:h1 (bilingual "NDC分類から探す" "By NDC class")]
    [:p (bilingual
         "日本十進分類法（NDC）の第一次区分です。児童書（青空文庫のK記号が付された作品）は該当する数字の分類に含めて「児童書」と明記しているほか、末尾の「K 児童書」にも全作品をまとめています。分類記号のない作品は「分類なし」にまとめています。"
         "The main classes of the Nippon Decimal Classification (NDC). Works of children's literature (carrying Aozora Bunko's K prefix) are included under their numeric classification with a 児童書 (children's) label, and are also collected together in the separate K index. Works without a classification code are gathered under 分類なし (Unclassified).")]
    (conj (into [:ul {:class "plain"}]
                (map (fn [{:keys [key ja en count*]}]
                       [:li [:a {:href (str "/ndc/" key)} (bilingual (str key " " ja) en)]
                        [:span {:class "by"} (str " (" count* ")")]])
                     classes))
          [:li [:a {:href "/ndc/K"} (bilingual "K 児童書" "Children's material")]
           [:span {:class "by"} (str " (" k-count ")")]])]))

(defn- ndc-k-page
  "Every work Aozora Bunko marks as children's material, across all classes."
  [works]
  (chrome
   (bilingual-title "児童書" "Children's material")
   [[:h1 (bilingual "児童書" "Children's material")]
    [:p (bilingual "青空文庫の分類記号にK（児童図書記号）が付された作品です。各作品は該当する数字のNDC分類ページにも掲載されています。"
                   "Works marked with Aozora Bunko's children's literature prefix (K). Each title is also indexed under its respective numeric NDC classification.")]
    [:p (bilingual (str "全" (count works) "作品（著者別）。")
                   (str (count works) " works, arranged by author."))]
    (author-groups works)
    (bulk-section
     (str "児童書全" (count works) "作品を一つのZIPアーカイブで取得できます。")
     (str "All " (count works) " works of children's literature in a single archive.")
     (map (juxt #(naming/ndc-bundle-path "K" %) identity)
          naming/bulk-artifact-types))]))

(defn- ndc-class-page [{:keys [key ja en]} works]
  (let [subclasses (ndc-subclasses works)
        page-title (if (= key "other")
                     (bilingual-title ja en)
                     (bilingual-title (str "NDC " key " " ja) (str "NDC " key " " en)))]
    (chrome
     page-title
     [[:h1 (if (= key "other") (bilingual ja en) (bilingual (str key " " ja) en))]
      (work-count-line works)
      (when (seq subclasses)
        [:section
         [:h2 (bilingual "細目" "Subclasses")]
         [:p (bilingual "三桁の分類ごとに個別ページと一括ダウンロードを用意しています。"
                        "Pages and bulk downloads are available for each three-digit subclass.")]
         (into [:ul {:class "cols plain"}]
               (map (fn [[subclass members]]
                      [:li [:a {:href (str "/ndc/" subclass)} (str "NDC " subclass)]
                       (when-let [label (ndc-label subclass)] (list " " label))
                       [:span {:class "by"} (str " (" (count members) ")")]])
                    subclasses))])
      (if (seq works) (author-groups works) (work-list works))
      (bulk-section
       (str "この分類の全" (count works) "作品を一つのZIPアーカイブで取得できます。")
       (str "All " (count works) " works in this class in a single archive.")
       (map (juxt #(naming/ndc-bundle-path key %) identity)
            naming/bulk-artifact-types))])))

(defn- ndc-subclass-page [{:keys [key class-key ja en]} works]
  (let [{sub-ja "ja" sub-en "en"} (ndc/labels key)
        page-title (if sub-ja
                     (bilingual-title (str "NDC " key " " sub-ja) (str "NDC " key " " (or sub-en sub-ja)))
                     (str "NDC " key))]
    (chrome
     page-title
     [[:h1 (str "NDC " key) (when-let [label (ndc-label key)] (list " " label))]
      [:p {:class "reading"}
       [:a {:href (str "/ndc/" class-key)} (bilingual (str class-key " " ja) en)]]
      (work-count-line works)
      (author-groups works)
      (bulk-section
       (str "この細目の全" (count works) "作品を一つのZIPアーカイブで取得できます。")
       (str "All " (count works) " works in this subclass in a single archive.")
       (map (juxt #(naming/ndc-bundle-path key %) identity)
            naming/bulk-artifact-types))])))

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
         (list [:dt (bilingual "分類" "NDC")]
               (into [:dd ndc]
                     (comp (keep ndc-label) (map (fn [label] (list " " label))))
                     (distinct (ndc-codes work)))))
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
         (list [:dt (bilingual "青空文庫の図書カード" "Aozora Bunko card")]
               [:dd [:a {:href card_url :rel "external"} card_url]]))
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
                          "このためunzip等の一般的な展開ツールではエラーとなる場合がありますが、"
                          "収録本文自体は破損しておらず、本版はその内容を直接変換しています。")
                     (str "Aozora Bunko's archive for this work contains "
                          trailing-bytes " bytes of trailing data after the end of the archive. "
                          "Extracting the archive with standard tools such as unzip may fail due to these trailing bytes; "
                          "the text content itself is intact and has been directly converted for this edition."))]))]

      [:section
       [:h2 (bilingual "本文を読む" "Read the text")]
       [:p [:a {:class "read-link" :href (str "/works/" slug "/read")}
            (bilingual "この作品を読む" "Read this work")]]
       [:p (bilingual
            "TEI版から変換した閲覧用表示です。ルビ・外字・傍点・字下げを再現し、縦書き表示にも切り替えられます。"
            "Reading view rendered from the TEI, with ruby, gaiji, emphasis marks and indentation, switchable to vertical writing.")]]

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
            (str "Cite the identifier; the filename is only for convenience. The same files are also "
                 "available at persistent URLs formed from the identifier and artifact type:"))]
       [:p (->> naming/artifact-kinds
                (map (fn [artifact-type]
                       (let [href (str "/works/" slug "/" artifact-type)]
                         [:a {:href href} [:code {:class "value"} href]])))
                (interpose " · "))]]

      [:section
       [:h2 (bilingual "引用" "Citation")]
       [:p (bilingual
            "引用には識別子とリリースを含めてください。作品名と著者名だけでは作品を特定できません。"
            "Give the identifier and the release. Title and author alone do not uniquely identify a work.")]
       [:p {:class "by"} (bilingual "日本語文献用" "For a Japanese bibliography")]
       [:p [:code {:class "citation" :lang "ja"} (citation/rendered release work)]]
       [:p {:class "by"} (bilingual "英語文献用" "For an English bibliography")]
       [:p [:code {:class "citation"} (citation/rendered-en release work)]]
       [:p {:class "by"}
        (bilingual
         "作品名は日本語表記のまま記載しています。ローマ字表記が必要な場合は、投稿先の規定に従って翻字してください。"
         "Titles are presented in Japanese; romanize them according to your publication's requirements.")]
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
        [:summary (bilingual "BibLaTeX項目を表示" "Show BibLaTeX entry")]
        [:pre (citation/biblatex release work)]]
       ;; COinS: an empty span whose title is an OpenURL context object. This
       ;; is the one metadata form a browser connector reads without
       ;; site-specific configuration, and `genre=bookitem` makes it save the
       ;; work as a section of its 底本 rather than as a book of its own.
       [:span {:class "Z3988" :title (citation/coins release work)}]
       [:p [:a {:href "/citation"} (bilingual "引用のしかた" "Citing Soranoha")]]]

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
     (bilingual-title (str title "（本文）") (str title " (Text)"))
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
            "本ページは署名済みTEI文書の閲覧用表示です。原本データは「TEI」欄のリンクから取得できます。複数の読みや校異がある箇所は、本文に一方を表示し、もう一方をカーソルを合わせた際の注記（title属性）に記載しています。"
            "This page is a reading view of the signed TEI file, which is linked under TEI as the authoritative record. Where alternative readings appear in the source, one is shown in the text and the other is viewable on hover (in the element's title attribute).")]
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
       [:p [:a {:href "/citation"} (bilingual "引用のしかた" "Citing Soranoha")]]]])))

(defn- withdrawn-page [slug {:strs [reason_code statement]} last-release]
  (chrome
   (bilingual-title (str slug " — 取り下げ") (str slug " — Withdrawn"))
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
         "This work is not included in the current release and will not appear in subsequent releases. Earlier releases in the signed history remain unchanged.")]
    (when last-release
      [:p (bilingual "最後に収録されたリリース：" "Last release that contained it:")
       " " [:a {:href (str "/releases/" last-release ".json")} [:code last-release]]])
    [:p [:a {:href "/rights"} (bilingual "権利について" "Rights and licensing")]]]))

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

  `source_content_hash` determines `changed`, because it is the fact
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
   (bilingual-title "リリースの履歴" "Release history")
   {:main-class "doc"}
   [[:h1 (bilingual "リリースの履歴" "Release history")]
    [:p (bilingual
         (str "署名付きリリース履歴の一覧です（新しい順）。"
              "各行の件数は、直前のリリースからの差分を公開マニフェストから集計した値です。")
         (str "All releases in the signed history, newest first. The counts in each row are "
              "the differences from the previous release, taken from the published manifests."))]
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
         "「底本の範囲」には、そのリリースが対象とする青空文庫のリビジョン範囲を記載しています。作品アーカイブが追加・更新された時点でリリースを作成するため、1つのリリースが複数のリビジョンにまたがる場合があります。最初のリリースには比較対象がないため、差分の列は空欄としています。"
         (str "Under Upstream range, the Aozora Bunko revisions covered by each release are recorded. "
              "Releases are minted when work archives are added or updated, so a single release can span "
              "multiple upstream revisions. Because the initial release has no predecessor, its difference columns are blank."))]]))

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
  [{:keys [ja en source]} lead]
  (chrome
   (bilingual-title ja en)
   {:main-class "doc"}
   (concat [[:h1 (bilingual ja en)]] lead
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
   [[:p (bilingual
         "複製、再頒布、翻案、翻訳、情報解析、公衆送信はいずれも、営利・非営利を問わず自由に行えます。許諾や対価は不要です。ほとんどの作品では、クレジット表示はお願いであって利用の条件ではありません。権利者がクリエイティブ・コモンズ表示ライセンス（CC BY）で公開している作品に限り、クレジット表示はそのライセンスの条件です。各作品の権利状態は作品ページに記載しています。"
         "You may copy, redistribute, adapt, translate, mine and republish everything here, commercially or not, without permission or fee. For most works, attribution is requested but not required. For works their rightsholder publishes under a Creative Commons Attribution (CC BY) licence, attribution is a condition of that licence. The applicable status is recorded on each work's page.")]
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
         "These values come from this release's manifest. The rights of the source works are recorded per work; every status that occurs in this release is listed.")]

    [:h2 (bilingual "権利の二つの層" "Two rights layers")]
    [:p (bilingual
         "底本の大部分は、著作権保護期間が満了した青空文庫の作品です。Soranohaはこれらについて著作権その他の権利を有しておらず、主張もしません。青空文庫の「収録ファイルの取り扱い規準」に基づき、著作権が満了した作品のファイルは有償・無償を問わず自由に複製・再頒布・翻案できます。"
         "Most source texts are Aozora Bunko works whose copyright has expired. Soranoha holds no rights in them and claims none. Under Aozora Bunko's handling rules, files of such works may be copied, redistributed and adapted freely, commercially or not.")]
    ;; the second regime. Aozora Bunko has no licence column, so these works
    ;; are identified from the notice their rightsholder wrote into the
    ;; colophon, and that notice is what the standing beside each work records.
    [:p (bilingual
         "残りは著作権が存続しており、権利者がクリエイティブ・コモンズ表示ライセンス（CC BY）で青空文庫に公開している作品です。これらも自由に利用できますが、クレジット表示がライセンス上の条件となります。ライセンスのバージョンは作品ページとTEIヘッダーの双方に記載しています。Soranohaは表示のみを条件とする作品を収録し、非営利・改変禁止・継承の条件が付いた作品は収録していません。"
         "The rest remain in copyright; their rightsholders publish them on Aozora Bunko under a Creative Commons Attribution (CC BY) licence. They may be used freely, provided that attribution is given as required by the licence. The licence version is recorded on each work's page and in its TEI header. Soranoha includes only works whose sole condition is attribution; works with NonCommercial, NoDerivatives or ShareAlike conditions are excluded.")]
    ;; CC0 can only cover what Soranoha added. In a file whose text is under
    ;; CC BY, the text is not Soranoha's to dedicate, so the paragraph states
    ;; what the waiver covers and what it does not.
    [:p (bilingual
         "Soranohaによる符号化データ（TEIマークアップ、プレーンテキストおよびMarkdownへの変換結果、検証レポート、リリースマニフェスト）は、CC0-1.0によりパブリックドメインに提供します。著作権および関連する権利（データベース権を含む）が生じる範囲において、これをすべて放棄します。この権利放棄はSoranohaが付加した部分にのみ及びます。底本がCC BYの作品では、TEI・プレーンテキスト・Markdown内の本文に元のCC BYライセンスが適用されるため、利用時にはクレジット表示が必要です。"
         "Soranoha's own encoding (the TEI markup, plain text and Markdown versions, validation reports, and release manifests) is dedicated to the public domain under CC0-1.0. Any copyright or database right that arises in it is waived. The waiver covers only what Soranoha added. In a CC BY work, the text inside the TEI, plain text, and Markdown files remains under CC BY and requires attribution.")]
    ;; Catalog bibliographic metadata derives from Aozora Bunko under CC BY 4.0;
    ;; catalog structure and Soranoha-authored fields remain CC0.
    [:p (bilingual
         "目録（catalog.jsonとcatalog.csv）の書誌情報は、青空文庫がCC BY 4.0で公開している「公開作品情報一覧」に由来します。目録やその派生一覧を再頒布する際は、出典として青空文庫を表示してください（これは本文ではなく書誌データに対する条件です）。目録のスキーマ、Soranohaが付与した識別子、および底本・検証に関する追加項目はCC0-1.0で提供します。"
         "The bibliographic data in catalog.json and catalog.csv comes from Aozora Bunko's published catalog (CC BY 4.0). Attribute Aozora Bunko when redistributing the catalog or lists derived from it; this condition applies to bibliographic metadata, not to the texts. The catalog schema, Soranoha identifiers, and source-edition and validation fields are CC0-1.0.")]
    ;; Community attribution and textual provenance requests from Aozora Bunko (non-binding).
    [:p (bilingual
         "また青空文庫は、作品名・著者・底本・入力者・校正者を記したクレジット表示を残すこと、および底本や表記を変更した際にその記録を添えることを求めています。Soranohaは各TEI文書のback要素と作品ページにクレジットを保持し、検証レポートと底本との差異を公開しています。再頒布の際にも同様の配慮をお願いします。"
         "Aozora Bunko also requests, though not as a legal condition, that the credit block (work, author, source edition, inputter, and proofreader) be retained and that changes to the source edition or notation be documented. Soranoha preserves the credit block in each TEI back element and on each work page, and records differences in each validation report. Please maintain the same practice when redistributing.")]

    ;; Published corpus terms (CC0/CC-BY) are distinct from toolchain licensing (Apache-2.0/MIT).
    [:p (bilingual
         "公開されたコーパスを再頒布しても、ツールチェーンのライセンス上の義務は生じません。ツールチェーン自体を再頒布する場合にのみ、ソースコードのライセンス（自作部分はApache-2.0、フォークしたパーサークレートは上流から継承したMIT OR Apache-2.0）が適用されます。"
         "Redistributing the published corpus incurs no obligations under the toolchain's licenses. Redistributing the toolchain itself does: Soranoha's own code is Apache-2.0, and the forked parser crates retain their upstream MIT OR Apache-2.0 terms.")]

    [:h2 (bilingual "公開作品に権利をお持ちの方へ" "If you hold rights in a published work")]
    [:p (bilingual
         "Soranohaは、著作権が満了したと判定された作品、および権利者が表示のみを条件とするライセンスで公開している作品のみを収録しています。万一、権利判定に誤りがある場合はご連絡ください。"
         "Soranoha publishes only works assessed as out of copyright, or whose rightsholders have published them under an attribution-only licence. If you hold rights in a published work, or find an error in an assessment, please contact us.")]
    [:p (bilingual
         (str "作品識別子（URLに含まれる" example-identifier "の形式）または青空文庫の図書カードと、権利主張の根拠を添えて、管理責任者のBor Hodošček（"
              orcid-url "）までご連絡ください。書式は問いません。")
         (str "Write to the maintainer, Bor Hodošček (" orcid-url "), specifying the work identifier (such as "
              example-identifier " in the URL) or the Aozora Bunko card, along with the "
              "basis of the claim. Informal notifications are accepted."))]
    ;; the event's own release is the one that removes the work, so a
    ;; rights holder does not wait for a corpus release that may be weeks
    ;; away; `transact/successor-for-event` builds that manifest
    [:p (bilingual
         "公開を取り下げる場合は、理由を記したガバナンスイベントを含む新しいリリースを即座に公開し、該当作品をworks[]と目録から除外してwithdrawn[]に記録します。追記専用の署名履歴であるため、過去のリリース自体は変更されません。"
         "When a work is withdrawn, Soranoha publishes an immediate successor release containing a governance event stating the reason. The work is removed from works[] and catalog.json and recorded in withdrawn[]. Because the signed history is append-only, earlier releases remain unchanged.")]

    [:h2 (bilingual "無保証" "No warranty")]
    [:p (bilingual
         "Soranohaは検証レポートと底本との差異を添えて翻刻データを公開していますが、内容の完全性や正確性を保証するものではありません。コーパスは現状有姿で提供します。法律・医療・安全に関わる用途では、各作品のsourceDescに記載された底本と必ず照合してください。"
         "Soranoha publishes transcriptions alongside validation reports and recorded source divergences, without warranty of completeness or accuracy. The corpus is provided as-is; do not rely on it for legal, medical, or safety purposes without verifying against the source edition named in each work's sourceDesc.")]]))

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
   ;; Citing Soranoha and attributing a work are different obligations, and
   ;; only the second can be a licence condition: CC0 sets none, CC BY does.
   [[:p (bilingual
         "Soranohaの符号化データはCC0のため、コーパスの引用はライセンス上の義務ではなく学術上の慣行です。一方、権利者がCC BYで公開している作品については、作品のクレジット表示がライセンスの条件となります（対象作品のページに明記しています）。引用時はリリースを明記してください。"
         "Because Soranoha's encoding is CC0, citing the corpus is a matter of scholarly practice rather than a licence requirement. For works published under CC BY, attribution is a licence condition, as noted on each work's page. Always cite the specific release.")]
    ;; Release names designate a project stage; citations require the immutable release head.
    (when release-name
      [:p (bilingual
           (str "コーパス全体の呼称は" release-name
                (when maturity (str "（" (:ja maturity) "）"))
                "です。呼称は単一のリリースではなく現在の開発段階を指すため、引用にはリリースハッシュを使用してください。")
           (str "The corpus as a whole is named " release-name
                (when maturity (str " (" (:en maturity) ")"))
                ". The name designates a stage of the project, not an individual release; cite the release hash."))])
    ;; Abbreviated 12-char release prefix without trailing ellipsis so the hash resolves directly.
    [:p {:class "by"} (bilingual "コーパス全体" "The whole corpus")]
    [:p [:code {:class "citation"}
         (str "Hodošček, Bor. " site-name " Aozora Bunko TEI Corpus. Release " (subs head-hex 0 12)
              (when doi (str ". https://doi.org/" doi)))]]
    [:p (bilingual
         "個々の作品の引用形式を、現在のリリースと「蜘蛛の糸」を例に示します。各作品のページには、その作品の引用形式を掲載しています。"
         "Citation forms for individual works are illustrated below using this release and 蜘蛛の糸. These forms are provided on each work's page, pre-filled for that work.")]
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
  "A compact array-of-arrays index: slug, title, reading, byline, the
  authors' romanized names, and 1 for children's material. Written directly rather than through the
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
                    "," (if (childrens? work) "1" "0")
                    "]"))
             works))
       "]}"))

(defn people
  "Every person in the release, keyed by person_id.

  `:works` contains every work to which the person contributed, and
  `:by-relation` groups those works by the contributor relation recorded in
  the catalog; both maintain catalog order without duplicate works. A person
  may be associated with multiple relations for a single work or repeated in
  contributor entries, but works are listed only once per person.

  This single definition ensures consistency across author pages and bulk
  download archives for the release."
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
      (page "favicon.svg" favicon-svg)
      (page "search.js" search-js)
      (page "copy.js" copy-js)
      (page "search-index.json" (search-index head-hex works))
      (page "index.html" (landing head-hex works (count withdrawn) release-name maturity entries))
      (page "rights.html" (rights-page (by-route "rights")
                                       (works-standings head)
                                       (get head "rights")))
      (page "citation.html" (citation-page (by-route "citation") release release-name maturity))
      (page "history.html" (history-page manifest-seq events))
      (page "docs.html" (docs-index-page entries))

      (page "authors/index.html"
            (author-index (mapv (fn [[id {:keys [person works]}]]
                                  {:person-id id
                                   :label (naming/person-label person)
                                   :count* (count works)})
                                (sort-by (fn [[_ {:keys [person]}]] (person-order person))
                                         people-index))))

      (page "titles/index.html"
            (title-index (mapv (fn [[key label en _]]
                                 {:key key :label label :en en :count* (count (get by-kana key))})
                               kana-rows)))

      (page "ndc/index.html"
            (ndc-index (mapv (fn [[key ja en]]
                               {:key key :ja ja :en en :count* (count (get by-ndc key))})
                             ndc-classes)
                       (count (filter childrens? works))))
      (page "ndc/K.html"
            (ndc-k-page (vec (sort-by reading-key (filter childrens? works)))))]

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

     (map (fn [[key label en _]]
            (page (str "titles/" key ".html")
                  (title-row-page {:label label :en en} (vec (sort-by reading-key (get by-kana key))))))
          kana-rows)

     (map (fn [[key ja en]]
            (page (str "ndc/" key ".html")
                  (ndc-class-page {:key key :ja ja :en en}
                                  (vec (sort-by reading-key (get by-ndc key))))))
          ndc-classes)

     (map (fn [[subclass members]]
            (let [[class-key ja en] (some #(when (= (subs subclass 0 1) (first %)) %) ndc-classes)]
              (page (str "ndc/" subclass ".html")
                    (ndc-subclass-page {:key subclass :class-key class-key :ja ja :en en}
                                       (vec (sort-by reading-key members))))))
          (ndc-subclasses works))

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
