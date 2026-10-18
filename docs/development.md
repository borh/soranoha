# Development

This is the canonical monorepo for the conversion and validation system,
developed and maintained by Bor Hodošček at [github.com/borh/soranoha](https://github.com/borh/soranoha). Readers
of the corpus are directed to [start here](start-here.md), [a worked example](worked-example.md)
and the [glossary](user-glossary.md) instead; this document is for people who
build or change the system.

## Layout

- `ab-validator/`: the Rust Aozora parser and the AAT contract and parser-IR
  conversion around it. The parser core (`ab-aozora-*` crates and
  `ab-notation-strategies`) is forked from
  [P4suta/aozora](https://github.com/P4suta/aozora)
  (`1a4f864603970983719655aa4af4525958ac2d38`, MIT or Apache-2.0).
- `soranoha/`: publication kernel, TEI/plaintext conversion, record schemas,
  validation, and the snh protocol, including
  [assessment evaluation](../soranoha/docs/assessment-evaluation.md) and the
  [operator runbook](../soranoha/docs/assessment-runbook.md).

## Shell and gates

Enter the root development shell:

```sh
nix develop
```

Inspect local runtime paths:

```sh
just runtime-config
```

Run the standard validation gate:

```sh
just validate
```

This checks runtime configuration, active path hygiene, TEI profile generation and
version coherence, release-critical flake input pins, Python quality, and Nix
formatting. It runs the publication tests, builds every check of the validator
flake, checks parser binary reproducibility, and evaluates the root and validator
flakes directly.

Focused checks:

```sh
just python-quality
just nix-format-check
just soranoha-tests
just typecheck
nix build ./ab-validator#checks.x86_64-linux.cargo-check
nix build ./ab-validator#checks.x86_64-linux.cargo-clippy
nix build ./ab-validator#checks.x86_64-linux.cargo-fmt
```

`just typecheck` checks the five publication-whitespace functions with Typed Clojure.
Its annotations and locked checker dependencies live in `soranoha/dev/typecheck`,
separate from runtime dependencies and publication toolchain identities.

`nix run .#soranoha-replay` measures cold-cache builds and consecutive source
revisions with the production stages and delta oracle. See
[performance measurements](performance.md) for the command and timing scope.

## Building and reviewing exports

To build the full export artifacts for a corpus checkout without publishing them:

```sh
nix run .#soranoha-kernel -- build \
  --root "$BUILD_STORE" \
  --aozora-root "$CORPUS_CHECKOUT" \
  --assets-root "$PWD/soranoha" \
  --out "$EXPORT_DIR"
```

Each work directory in `$EXPORT_DIR` contains `tei.xml`, `plain.txt`, `text.md`,
`tei-validation.json`, `plaintext-projection.json`, `markdown-projection.json`,
`source-accountability.json`, and `interpretation-coverage.json`. A top-level
`build.json` records artifact hashes and build results. Omitting `--out` retains
outputs in the computation store. Building exports does not publish them or accept
an assessment.

## Previewing the site locally

`just site-preview` builds a subset of works, assembles a serving tree around
a temporary chain signed with the checked-in fixture keys, and serves it with
Caddy on 127.0.0.1. Every page comes from the publication and serving code, so
a page that renders incorrectly here renders incorrectly in production. It is
not a release.

### Corpus checkout

The build takes its inputs from an Aozora Bunko checkout at the revision
pinned by the flake as `aozorabunko-src`, a Forgejo mirror of the upstream
repository. Clone it once outside the repository and check out the pinned
revision:

```sh
rev=$(sed -n 's/.*aozorabunko.git?rev=\([0-9a-f]*\).*/\1/p' flake.nix)
git clone https://code.hyakutake-barbel.ts.net/bor/aozorabunko.git "$HOME/Dependencies/aozorabunko"
git -C "$HOME/Dependencies/aozorabunko" checkout "$rev"
```

Repeat the checkout step when the pin moves. The source revision recorded by the
kernel is the checkout's commit, so a checkout at another commit builds against a
different corpus from the one the checks use.

### Machine configuration

The scripts take their paths from the first of `.soranoha/env` in the
repository and `${XDG_CONFIG_HOME:-$HOME/.config}/soranoha/env`. Environment
variables exported in the shell take precedence over both. In environments using
direnv, `.envrc` sources the same configuration file. Copy
`config/soranoha.env.example` to one of those locations and set:

```sh
export CORPUS_CHECKOUT="$HOME/Dependencies/aozorabunko"
```

`SORANOHA_PREVIEW_DIR` in the same file moves the preview's work directory.

### Work directory

The preview writes under `SORANOHA_PREVIEW_DIR`, by default
`${XDG_CACHE_HOME:-$HOME/.cache}/soranoha/preview`, and nothing in the
repository:

| Path | Contents |
|---|---|
| `root/` | The kernel's computation store. Kept between runs, so a second preview of the same works is quick. It grows with every distinct set of works built. |
| `run/` | The build export of the current preview. |
| `chain/` | The temporary chain origin and clone. |
| `tree/` | The serving tree Caddy serves. |

Each run deletes `run/`, `chain/` and `tree/` first. Delete the whole
directory to reclaim the space.

### Serving

```sh
just site-preview works=200 port=8787
```

`works` is the number of works to build, taken from the start of the catalog
selection; `port` is the local port. Stop with Ctrl-C. To look at the last
tree again without rebuilding:

```sh
nix develop --command env SORANOHA_SERVE_LISTEN=127.0.0.1:8787 \
  SORANOHA_SERVE_ROOT="${XDG_CACHE_HOME:-$HOME/.cache}/soranoha/preview/tree" \
  caddy run --config soranoha/config/caddy/Caddyfile --adapter caddyfile
```

## Regenerating the TEI profile

The pinned recipe uses TEI P5 4.11.0 and TEI Stylesheets 7.60.0 with Saxon-HE.
After editing the ODD:

```sh
nix run .#regenerate-tei-profile
just evidence-gate
```

`regenerate-tei-profile` rewrites the RNG, the Schematron and
`soranoha/schemas/tei-profile-generation.json` in place. `evidence-gate` runs
the drift check, which compares the generated artifacts and their provenance
with the checked-in profile, and then the test suite. Which hashes change
depends on the prose edited: a `remarks` block or a `constraintSpec`
description is copied into neither generated file, so it changes `odd_hash`
alone; an `attDef` description is copied into the Relax NG as documentation,
so it changes `rng_hash` as well.

## Regenerating the NDC class table

`soranoha/resources/ndc/ndc9-sections.json` lists the three-digit classes of
NDC9 with their labels, derived from the Japan Library Association's Linked
Data release pinned as the `ndc9-ld` flake input. After updating that input,
unzip it and rewrite the table:

```sh
nix run .#soranoha-kernel -- ndc9-sections --ttl /path/to/ndc9.ttl \
  --table "$PWD/soranoha/resources/ndc/ndc9-sections.json"
just evidence-gate
```

The `ndc9-sections-drift` check in `evidence-gate` re-derives the table
from the pinned input, failing if the committed file differs.

## Source identity

Nix owns source identity and the toolchain; domain behaviour is implemented in
Clojure, Rust and Python. Every evidence-bearing input (the TEI P5 source, the TEI-EAJ
reference, the corpus snapshot, dictionary and converter sources) is pinned by
revision or release tag in its flake URL, not only in the lock, so a routine
lock update cannot move it. Infrastructure inputs such as nixpkgs and clj-nix
are lock-only pins.

The root `flake.lock` is the canonical lock. The `ab-validator/flake.lock`
supports working in that component directly and must agree with the root lock
for shared non-path inputs, so one change updates a shared input in both locks.
`just flake-input-policy` enforces both rules, and `just tei-version-coherence`
checks that every TEI reference in the repository is P5 4.11.0.

## Where the rest is written

- [Publication architecture](design/publication-architecture.md): the cache,
  assessment, publication and serving boundaries.
- [snh specification](design/snh-protocol-v1.md): the wire contract.
- [Key ceremony](key-ceremony.md): the two pinned signing roles and the offline
  governance signing path used to withdraw or amend a published work.
- [Zenodo deposits and ORCID anchoring](zenodo-deposits.md): the trust anchor
  that publishes the role assignment and the quarterly snapshot deposit that
  gives a release a DOI.
- [Archival](archival.md): the Software Heritage observation a release has to
  pass before it is citable, and how a citation resolves through the archive.
- [TEI extension vocabulary](../soranoha/docs/tei-vocabulary.md): the
  `https://w3id.org/soranoha/ns/tei` namespace, bound to `snh:`, with its
  attributes and validation rule identifiers.
- [Work identifiers](../soranoha/docs/work-identifiers.md): the
  `000092_000879` form, what it guarantees across releases, and where exact
  edition identity lives instead.
- [Parser and rendering invariants](parser-invariants.md): the behaviour the
  converter must preserve, for use in code comments and commit messages.
- [Rights](rights.md) and [citation](citation.md): the two rights layers,
  toolchain licensing, and the citation forms; `CITATION.cff` carries the
  machine-readable corpus citation.
