nix_eval := "nix --option eval-cache false"
# The repository gates written in Clojure run through the kernel, which the
# wrapper starts from its store copy of soranoha/, so each takes the
# repository root explicitly.
kernel := nix_eval + " run .#soranoha-kernel --"
# Built tools are linked under the state root so a Nix garbage collection
# during a pass cannot remove them.
nix_root := ". scripts/soranoha-runtime-env.sh; roots=\"$SORANOHA_STATE_ROOT/nix-roots\"; mkdir -p \"$roots\"; root() { " + nix_eval + " build \"$1\" --print-out-paths --out-link \"$roots/$2\"; }"

fidelity-report OUT *ARGS:
    @set -eu; {{nix_root}}; parser="$(root ./ab-validator#ab-aozora ab-aozora)"; \
    checker="$(root ./ab-validator#ab-check ab-check)"; \
    inventory="$(root ./ab-validator#ab-source-inventory ab-source-inventory)"; \
    converter="$(root ./ab-validator#ab-aat-to-parser-ir ab-aat-to-parser-ir)"; \
    tei_probe="$(root .#soranoha-fidelity-probe soranoha-fidelity-probe)"; \
    "$inventory/bin/ab-fidelity-report" --parser "$parser/bin/ab-aozora" \
      --checker "$checker/bin/ab-check" --inventory "$inventory/bin/ab-source-inventory" \
      --converter "$converter/bin/ab-aat-to-parser-ir" --tei-probe "$tei_probe" \
      --out "{{OUT}}" {{ARGS}}

# ARGS supplies --row (repeatable), --index, --corpus, --cache-dir and optional report inputs.
fidelity-alignment OUT *ARGS:
    @set -eu; {{nix_root}}; parser="$(root ./ab-validator#ab-aozora ab-aozora)"; \
    inventory="$(root ./ab-validator#ab-source-inventory ab-source-inventory)"; \
    "$inventory/bin/ab-occurrence-alignment" --matrix ab-validator/data/aozora-syntax-coverage.toml \
      --parser "$parser/bin/ab-aozora" --output-json "{{OUT}}" {{ARGS}}

# ARGS supplies --index, --corpus, --cache-dir, --artifacts-dir and the
# --target-row or --target-form changes the pass intends.
# Diff every edition between a committed BASELINE revision and the working tree.
fidelity-diff BASELINE OUT *ARGS:
    @set -eu; {{nix_root}}; rev="$(git rev-parse '{{BASELINE}}')"; \
    baseline="$(root "git+file://$PWD?dir=ab-validator&rev=$rev#ab-aozora" "ab-aozora-$rev")"; \
    baseline_converter="$(root "git+file://$PWD?dir=ab-validator&rev=$rev#ab-aat-to-parser-ir" "ab-aat-to-parser-ir-$rev")"; \
    baseline_mapping="$(mktemp)"; trap 'rm -f "$baseline_mapping"' EXIT; \
    git show "$rev:ab-validator/data/aat-to-parser-ir-mapping-v2.json" > "$baseline_mapping"; \
    parser="$(root ./ab-validator#ab-aozora ab-aozora)"; \
    inventory="$(root ./ab-validator#ab-source-inventory ab-source-inventory)"; \
    converter="$(root ./ab-validator#ab-aat-to-parser-ir ab-aat-to-parser-ir)"; \
    tei_probe="$(root .#soranoha-fidelity-probe soranoha-fidelity-probe)"; \
    "$inventory/bin/ab-edition-diff" --matrix ab-validator/data/aozora-syntax-coverage.toml \
      --baseline-parser "$baseline/bin/ab-aozora" --candidate-parser "$parser/bin/ab-aozora" \
      --converter "$converter/bin/ab-aat-to-parser-ir" \
      --mapping ab-validator/data/aat-to-parser-ir-mapping-v2.json \
      --baseline-converter "$baseline_converter/bin/ab-aat-to-parser-ir" \
      --baseline-mapping "$baseline_mapping" --tei-probe "$tei_probe" --output-json "{{OUT}}" {{ARGS}}

# One fidelity pass for the space-separated ROWS over the corpus at INDEX
# and CORPUS. OUT keeps the caches and the last passing alignment, whose
# residue ledger this pass must not exceed. A BASELINE revision also diffs
# every edition against it and compares every corpus check with its parser;
# the pass fails when an obligation in the space-separated FLIPS changes
# outcome at no occurrence.
fidelity-pass ROWS INDEX CORPUS OUT BASELINE="" FLIPS="":
    @set -u; [ -z '{{FLIPS}}' ] || [ -n '{{BASELINE}}' ] || { echo "FLIPS requires BASELINE"; exit 1; }; \
    {{nix_root}}; out="$(realpath -m '{{OUT}}')"; mkdir -p "$out"; status=0; \
    rm -f "$out/alignment.candidate.json" "$out/diff.json" "$out/timings.json"; \
    variant_rows=""; rows=""; target_rows=""; flips=""; \
    for row in {{ROWS}}; do variant_rows="$variant_rows --variant-row $row"; rows="$rows --row $row"; target_rows="$target_rows --target-row $row"; done; \
    for obligation in {{FLIPS}}; do flips="$flips --expect-flip $obligation"; done; \
    baseline_parser=""; [ -n '{{BASELINE}}' ] && { rev="$(git rev-parse '{{BASELINE}}')"; baseline="$(root "git+file://$PWD?dir=ab-validator&rev=$rev#ab-aozora" "ab-aozora-$rev")" \
      && baseline_parser="--baseline-parser $baseline/bin/ab-aozora $flips" || { echo "baseline parser build failed"; status=1; }; }; \
    now() { date +%s.%N; }; stages=""; \
    stage() { name="$1"; shift; start=$(now); "$@" > "$out/$name.log" 2>&1 || { echo "$name failed; see $out/$name.log"; status=1; }; \
      stages="$stages\"$name\": $(awk "BEGIN { print $(now) - $start }"),"; }; \
    stage fixture-report just fidelity-report "$out/report" --index '{{INDEX}}' --corpus '{{CORPUS}}' \
      --inventory-cache "$out/inventory-cache" $variant_rows; \
    ratchet=""; [ -f "$out/alignment.json" ] && ratchet="--ratchet $out/alignment.json"; \
    [ "$status" = 0 ] && stage alignment just fidelity-alignment "$out/alignment.candidate.json" $rows \
      --index '{{INDEX}}' --corpus '{{CORPUS}}' --cache-dir "$out/alignment-cache" \
      --fidelity-report "$out/report/report.json" --inventory-json "$out/report/corpus-inventory.json" \
      --requirements ab-validator/data/fidelity-requirements.toml \
      --decisions ab-validator/data/residue-decisions.toml $ratchet $baseline_parser; \
    [ "$status" = 0 ] && mv "$out/alignment.candidate.json" "$out/alignment.json"; \
    [ -n '{{BASELINE}}' ] && stage diff just fidelity-diff '{{BASELINE}}' "$out/diff.json" \
      --index '{{INDEX}}' --corpus '{{CORPUS}}' --cache-dir "$out/diff-cache" \
      --artifacts-dir "$out/diff-artifacts" $target_rows; \
    echo "{${stages%,}}" | jq . > "$out/timings.json"; \
    report="$out/alignment.json"; [ -f "$out/alignment.candidate.json" ] && report="$out/alignment.candidate.json"; \
    [ -f "$report" ] && jq -r '.residue_ledger | to_entries[] | .key as $row | .value as $l | $row, \
        "  residue \($l.residue_occurrences) of \($l.occurrences)", \
        ($l.classes | to_entries[] | "  \(.key): \(.value.occurrences) in \(.value.forms) forms")' "$report"; \
    [ -f "$report" ] && jq -r '(.qualification // {}) | to_entries[] | "  \(.key): " + (if .value.qualified then "qualified" else "not qualified: " + ((.value.report_gaps + [.value.residue | to_entries[] | "\(.key) \(.value)"]) | join(", ")) end)' "$report"; \
    [ -f "$report" ] && jq -r '(.ratchet // {}) | to_entries[] | "ratchet \(.key): \(.value.current_residue) (previous \(.value.previous_residue))" + (if .value.regressed then " REGRESSED" else "" end)' "$report"; \
    [ -f "$report" ] && jq -r '.obligation_checks | to_entries[] | select(.value.failed > 0) | "check \(.key): \(.value.failed) failed"' "$report"; \
    [ -f "$report" ] && jq -r '.obligation_checks | to_entries[] | select(.value.discrimination) | .value.discrimination as $d | "flip \(.key): gained \($d.gained), lost \($d.lost), both satisfied \($d.both_satisfied), both failed \($d.both_failed)"' "$report"; \
    [ -f "$out/diff.json" ] && jq -r '"diff: \(.editions.changed) changed editions, \(.editions.unexplained) unexplained, passed \(.passed)"' "$out/diff.json"; \
    jq -r 'to_entries[] | "  \(.key): \(.value | floor) s"' "$out/timings.json"; \
    . scripts/soranoha-runtime-env.sh; register="$SORANOHA_STATE_ROOT/fidelity-runs.jsonl"; mkdir -p "$SORANOHA_STATE_ROOT"; \
    jq -c -n --arg at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --arg rev "$(git rev-parse HEAD)" --arg dirty "$(git status --porcelain | head -c1)" \
      --arg rows '{{ROWS}}' --arg out "$out" --arg baseline '{{BASELINE}}' --argjson status $status \
      --arg decisions "$(sha256sum ab-validator/data/residue-decisions.toml | cut -c1-64)" \
      --slurpfile report "$out/report/report.json" --slurpfile timings "$out/timings.json" \
      --slurpfile alignment "$report" --slurpfile diff "$([ -f "$out/diff.json" ] && echo "$out/diff.json" || echo /dev/null)" \
      '{recorded_at: $at, status: (if $status == 0 then "passed" else "failed" end), rows: ($rows | split(" ")), \
        revision: $rev, dirty: ($dirty != ""), baseline: $baseline, out: $out, \
        inputs: (($report[0].inputs // {}) + {decisions_sha256: $decisions, index_sha256: $alignment[0].inputs.index_sha256}), \
        ledger: ($alignment[0].residue_ledger | map_values({occurrences, residue_occurrences, classes: (.classes | map_values(.occurrences))})), \
        ratchet: $alignment[0].ratchet, obligation_checks_failed: [$alignment[0].obligation_checks | to_entries[] | select(.value.failed > 0) | .key], \
        diff: (if ($diff | length) > 0 then {changed: $diff[0].editions.changed, unexplained: $diff[0].editions.unexplained, passed: $diff[0].passed} else null end), \
        timings: $timings[0]}' >> "$register" && echo "recorded in $register"; \
    exit $status

# Every recorded fidelity pass, newest last.
fidelity-runs:
    @. scripts/soranoha-runtime-env.sh; register="$SORANOHA_STATE_ROOT/fidelity-runs.jsonl"; [ -f "$register" ] || { echo "no runs recorded in $register"; exit 0; }; \
    { echo "recorded_at|status|rows|revision|residue|timings"; \
      jq -r '[.recorded_at, .status, (.rows | join(" ")), (.revision[:8] + (if .dirty then "+" else "" end)), \
        ([.ledger | to_entries[] | "\(.key)=\(.value.residue_occurrences)/\(.value.occurrences)"] | join(" ")), \
        ([.timings | to_entries[] | "\(.key)=\(.value | floor)s"] | join(" "))] | join("|")' "$register"; } | column -t -s'|'

tei-version-coherence:
	@bash scripts/monorepo-tei-version-coherence.sh

flake-input-policy:
	@{{kernel}} flake-input-policy --repo "$PWD"

# Recorded schema hashes are assertions about which schema a record or an
# importer was written against. Recompute them, so a schema edit fails here
# rather than at the first import of real data.
schema-hash-coherence:
	@{{kernel}} schema-hash-coherence --repo "$PWD"

runtime-config:
	@bash -lc 'source scripts/soranoha-runtime-env.sh; soranoha_runtime_summary'

runtime-config-smoke:
	@bash tests/runtime-config-smoke.sh

active-path-hygiene:
	@bash tests/monorepo-active-path-hygiene-smoke.sh

python-quality:
	@bash scripts/python-quality.sh

# Scans the files git tracks, so it runs here rather than as a flake check:
# the check source is a store path with no git directory, and a tree walk
# would include build outputs and any local corpus checkout.
comment-hygiene:
	@bash scripts/comment-hygiene-check.sh

docs-links:
	@{{kernel}} docs-links --repo "$PWD"

# The two JIS X 0213 gaiji tables must match the generator's output from the
# licensed source table, so a hand edit cannot separate either from its terms.
gaiji-table-drift:
	@python ab-validator/crates/ab-aozora-encoding/data/generate_jisx0213_tables.py --check

# Re-derive every corpus figure quoted in prose from the pinned Aozora catalog.
# Not part of any bundled gate: it needs a corpus checkout, which the sandboxed
# checks do not have. Set CORPUS_CHECKOUT, or pass --aozora-root. `--stems`
# adds the figures that need a pass over every work archive.
catalog-figures *args:
	@{{kernel}} catalog-figures --repo "$PWD" {{args}}

# The part of that check that needs no corpus, which the bundled gates run:
# every file recorded as quoting a figure still quotes the same number.
figure-quotes:
	@{{kernel}} catalog-figures --repo "$PWD" --quotes-only

# Build a browsable preview of the public serving tree and serve it on
# 127.0.0.1:PORT. Not a release: the chain is local and temporary, signed with
# the checked-in conformance fixture keys, and the assessment snapshot is
# asserted rather than evaluated. Every served page is produced by the
# publication and serving code. Needs a corpus checkout: set CORPUS_CHECKOUT.
# SORANOHA_PREVIEW_DIR sets the work directory, which defaults under
# XDG_CACHE_HOME so a preview writes nothing in the repository tree.
site-preview works="8" port="8787":
	@bash scripts/site-preview.sh {{works}} {{port}}

# Regenerate the compiled TEI profile from soranoha/schemas/tei-profile.odd.
# ori/validate.clj holds the recorded odd, rng and schematron hashes against
# the files on disk, so the ODD cannot change without its compiled artifacts:
# editing it without running this fails every validation test with
# :profile-generation-mismatch.
# The generator is pinned in nix/tei-profile-artifacts.nix, so the output is a
# function of the ODD and that recipe alone.
regenerate-tei-profile:
	@nix run .#regenerate-tei-profile && echo "regenerated soranoha/schemas from the ODD"

nix-format-check:
	@find . \
		-path './.git' -prune -o \
		-path './.claude' -prune -o \
		-path './.worktrees' -prune -o \
		-path './worktrees' -prune -o \
		-path './.direnv' -prune -o \
		-path './result*' -prune -o \
		-name '*.nix' -print0 \
		| xargs -0 nixfmt --check

root-flake-check-no-build:
	@{{nix_eval}} flake check --no-build

root-flake-output-contract:
	@bash tests/root-flake-output-contract-smoke.sh

tei-eaj-alignment-probe *args:
	@scripts/run-tei-eaj-probe-workflow.sh alignment-probe {{args}}

tei-eaj-reports-with-probes *args:
	@scripts/run-tei-eaj-probe-workflow.sh reports-with-probes {{args}}

validate-eval-cache-smoke:
	@bash tests/validate-eval-cache-smoke.sh

check-no-build: runtime-config-smoke active-path-hygiene root-flake-output-contract tei-version-coherence flake-input-policy schema-hash-coherence figure-quotes python-quality comment-hygiene docs-links gaiji-table-drift nix-format-check validate-eval-cache-smoke root-flake-check-no-build
	@(cd ab-validator && AB_WORKSPACE_ROOT="$(pwd)/.." {{nix_eval}} flake check --no-build)

# Publication tests, the generated TEI profile and the NDC9 class table must match their checked-in sources.
evidence-gate: soranoha-tests typecheck
	@system="$({{nix_eval}} eval --impure --raw --expr builtins.currentSystem)"; \
	{{nix_eval}} build ".#checks.$system.tei-profile-drift" ".#checks.$system.ndc9-sections-drift" \
		"./ab-validator#checks.$system.tei-eaj-python-tests" --print-build-logs

# Soranoha kernel + snh conformance suite plus clj-kondo/cljfmt, hermetic
# against the root wrapper's shared Clojure/dependency-cache context.
soranoha-tests:
	@system="$({{nix_eval}} eval --impure --raw --expr builtins.currentSystem)"; \
	{{nix_eval}} build ".#checks.$system.soranoha-tests" --print-build-logs

# release-parser-reproducible: the two release binaries must rebuild
# byte-identically. `nix build --rebuild` re-realizes a derivation and fails if
# the freshly built output differs byte-for-byte from the cached path, so it
# establishes reproducibility against the baseline built first. This needs the
# Nix daemon (a sandboxed runCommand builder cannot invoke it), so it is a just
# recipe rather than a checks.<system> derivation.
#
# Each derivation path is resolved once, and the rebuild addresses that path
# rather than re-evaluating the flake attribute. A flake source in a dirty git
# tree carries the working tree into the derivation hash, so an edit between the
# two steps would make the second step address an unbuilt derivation, for which
# nix reports that it cannot check rather than comparing bytes. With the path
# resolved once, both steps address one derivation regardless of tree changes,
# and no clean-tree precondition is needed.
release-parser-reproducible:
	@aozora="$({{nix_eval}} eval --raw ./ab-validator#ab-aozora.drvPath)" \
	&& parser_ir="$({{nix_eval}} eval --raw ./ab-validator#ab-aat-to-parser-ir.drvPath)" \
	&& {{nix_eval}} build "$aozora^*" "$parser_ir^*" --no-link --print-build-logs \
	&& {{nix_eval}} build "$aozora^*" --rebuild --no-link --print-build-logs \
	&& {{nix_eval}} build "$parser_ir^*" --rebuild --no-link --print-build-logs \
	&& echo "release parser binaries rebuild reproducibly"

# Build every flake check. `flake check --no-build` evaluates checks without
# running them, so the ab-validator set is built here by name. The names are
# read from the flake rather than a hand-kept list, so a new check is built
# without an edit to this recipe.
#
# `default` is the workspace test suite; `ab-validator` and `cargo-test` are
# its aliases and are dropped so the derivation is listed once.
flake-checks:
	@{{nix_eval}} flake check --print-build-logs
	@system="$({{nix_eval}} eval --impure --raw --expr builtins.currentSystem)"; \
	names="$({{nix_eval}} eval --raw "./ab-validator#checks.$system" --apply \
		'cs: builtins.concatStringsSep " " (builtins.filter (n: !(builtins.elem n [ "ab-validator" "cargo-test" ])) (builtins.attrNames cs))')"; \
	targets=""; \
	for name in $names; do targets="$targets ./ab-validator#checks.$system.$name"; done; \
	{{nix_eval}} build --no-link --print-build-logs $targets

validate: check-no-build evidence-gate flake-checks release-parser-reproducible

typecheck:
	@system="$({{nix_eval}} eval --impure --raw --expr builtins.currentSystem)"; \
	{{nix_eval}} build ".#checks.$system.soranoha-typecheck" --print-build-logs
