{
  description = "Soranoha monorepo integration flake";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    ab-validator = {
      url = "path:./ab-validator";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        aozorabunko-src.follows = "aozorabunko-src";
      };
    };

    clj-nix = {
      url = "github:jlesquembre/clj-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    tei-p5 = {
      url = "github:TEIC/TEI/P5_Release_4.11.0";
      flake = false;
    };

    # The upstream repository has not been on GitHub since 2026. This input is
    # the project's mirror of its history, on the Forgejo host the runner can
    # reach; the revision is upstream's commit and the tree hash is identical.
    aozorabunko-src = {
      url = "git+ssh://forgejo@code.hyakutake-barbel.ts.net:63333/bor/aozorabunko.git?rev=795f89df7bf0a995727f58ad998101458d5d91c2&shallow=1";
      flake = false;
    };

    # The Japan Library Association's NDC9 Linked Data (CC BY), the source of
    # the class labels the TEI header declares. A zip rather than a tarball,
    # so it is fetched as a file and unpacked by the check that reads it.
    ndc9-ld = {
      url = "file+https://www.jla.or.jp/wp/wp-content/uploads/2025/06/ndc9.zip";
      flake = false;
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      ab-validator,
      clj-nix,
      tei-p5,
      ndc9-ld,
      ...
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      # Soranoha targets Linux only (x86_64 and aarch64) via genAttrs,
      # while ab-validator uses flake-utils.eachDefaultSystem for Rust
      # builds (including Darwin). The root wraps only ab-validator's Linux outputs.
      forAllSystems = nixpkgs.lib.genAttrs systems;

      inherit (nixpkgs) lib;

      pkgsFor =
        system:
        import nixpkgs {
          inherit system;
          overlays = [ (_final: prev: { jdk = prev.jdk25_headless; }) ];
        };

      # The dependency cache is also HOME, so nothing under the user's HOME
      # enters the classpath; the scratch directory holds the classpath
      # cache and XDG config so both stay writable.
      cljCacheExports = cache: scratch: ''
        export HOME="${cache}"
        export JAVA_TOOL_OPTIONS="-Duser.home=${cache}"
        export CLJ_CONFIG="${cache}/.clojure"
        export GITLIBS="${cache}/.gitlibs"
        export CLJ_CACHE="${scratch}/cp-cache"
        export XDG_CONFIG_HOME="${scratch}/xdg-config"
      '';

      exportsOf =
        env: lib.concatLines (lib.mapAttrsToList (name: value: ''export ${name}="${value}"'') env);

      # Everything an output needs for one system, computed once per system
      # rather than once per output.
      perSystem =
        system:
        let
          pkgs = pkgsFor system;
          # clj-nix's dependency-cache builder, without the jdk override: the
          # cache is a fetch, and the toolchain identity below hashes it.
          cljNix = import nixpkgs {
            inherit system;
            overlays = [ clj-nix.overlays.default ];
          };
          abValidatorPackages = ab-validator.packages.${system};

          # Resolve the runtime against the offline dependency cache with user
          # configuration disabled. The same closure and deps.edn form the stage
          # toolchain identity, preventing stale traces after dependency changes.
          # Tests share tool derivations but do not use the wrapper's full environment.
          depsCache = cljNix.mk-deps-cache { lockfile = ./soranoha/deps-lock.json; };
          toolchainId =
            "clj-nix-"
            + builtins.hashString "sha256" "${pkgs.clojure}\n${depsCache}\n${builtins.hashFile "sha256" ./soranoha/deps.edn}";

          # The documents the browse layer serves, laid out as they are in the
          # repository, so one repository-relative path identifies a file for
          # both the served site and the link check. A developer running from
          # the `soranoha` directory reaches the same layout through the default
          # root of `..`; the wrapper points SORANOHA_SITE_DOCS at this instead,
          # because a Nix store path has no enclosing repository.
          siteDocs = pkgs.runCommand "soranoha-site-docs" { } ''
            mkdir -p "$out/soranoha"
            cp -R ${./docs} "$out/docs"
            cp -R ${./soranoha/docs} "$out/soranoha/docs"
            cp -R ${./soranoha/schemas} "$out/soranoha/schemas"
            mkdir -p "$out/soranoha/resources/assessment"
            cp ${./soranoha/resources/assessment/source-1.schema.json} \
              "$out/soranoha/resources/assessment/source-1.schema.json"
            cp ${./LICENSE} "$out/LICENSE"
            cp ${./LICENSE-CC0} "$out/LICENSE-CC0"
            # the toolchain's identifiers: every schema under its schema
            # directory, and the mapping and policy documents that carry
            # their own IRIs
            mkdir -p "$out/ab-validator/schemas" "$out/ab-validator/data"
            cp ${ab-validator}/schemas/*.schema.json "$out/ab-validator/schemas/"
            cp ${ab-validator}/data/aat-to-parser-ir-mapping-v1.json \
              ${ab-validator}/data/aat-to-parser-ir-mapping-v2.json \
              "$out/ab-validator/data/"
            mkdir -p "$out/soranoha/data"
            cp ${./soranoha/data/source-region-publication-policy-v0.json} \
              "$out/soranoha/data/source-region-publication-policy-v0.json"
          '';
        in
        {
          inherit
            system
            pkgs
            cljNix
            abValidatorPackages
            depsCache
            toolchainId
            ;
          tei = import ./nix/tei.nix { inherit pkgs tei-p5; };
          profile = import ./nix/tei-profile-artifacts.nix {
            inherit pkgs;
            odd = ./soranoha/schemas/tei-profile.odd;
          };
          # The adapter binaries and data the kernel's stages call. The mapping
          # is copied out of the ab-validator tree so its store path follows
          # its own content, not every edit to that tree.
          abToolEnv = {
            AB_AAT_TO_PARSER_IR_BIN = "${abValidatorPackages.ab-aat-to-parser-ir}/bin/ab-aat-to-parser-ir";
            AB_AOZORA_BIN = "${abValidatorPackages.ab-aozora}/bin/ab-aozora";
            AB_SOURCE_INVENTORY_BIN = "${abValidatorPackages.ab-source-inventory}/bin/ab-source-inventory";
            AB_AOZORA_SYNTAX_MATRIX = "${ab-validator}/data/aozora-syntax-coverage.toml";
            AB_AAT_TO_PARSER_IR_MAPPING_V2 = builtins.path {
              path = "${ab-validator}/data/aat-to-parser-ir-mapping-v2.json";
              name = "aat-to-parser-ir-mapping-v2.json";
            };
            SORANOHA_SITE_DOCS = "${siteDocs}";
          };
        };

      forEachSystem = f: forAllSystems (system: f (perSystem system));

      # Maven model validation shares a mutable ID cache. Resolve dependencies
      # on one worker; publication stage concurrency is independent.
      mkSoranohaApp =
        {
          pkgs,
          depsCache,
          toolchainId,
          abToolEnv,
          ...
        }:
        { name, invocation }:
        pkgs.writeShellScript name ''
          set -euo pipefail
          export PATH="${
            lib.makeBinPath [
              pkgs.bash
              pkgs.coreutils
              pkgs.git
              pkgs.clojure
            ]
          }"
          ${exportsOf abToolEnv}
          # classpath scratch must stay writable; cleaned via trap, so the
          # final clojure call must not exec-replace this shell
          scratch="$(mktemp -d)"
          trap 'rm -rf "$scratch"' EXIT
          ${cljCacheExports depsCache "$scratch"}
          # inherited launcher variables would alter the JVM or classpath
          # without changing the reported identity
          unset JAVA_CMD CLJ_JVM_OPTS JAVA_OPTS JDK_JAVA_OPTIONS _JAVA_OPTIONS
          cd "${./soranoha}"
          ${invocation} "$@" --clj-toolchain-id "${toolchainId}"
        '';

      # The bench entry points run from the source tree with the test path
      # on the classpath.
      benchInvocation =
        heap: main: args:
        "clojure -Sthreads 1 -J-Xmx${heap} -J--enable-native-access=ALL-UNNAMED -Sdeps '{:paths [\"src\" \"resources\" \"test\"]}' -M -m ${main} ${args}";

      monorepoScripts =
        pkgs:
        let
          kernel = self.apps.${pkgs.stdenv.hostPlatform.system}.soranoha-kernel.program;
          runtimePath = lib.makeBinPath [
            pkgs.bash
            pkgs.coreutils
            pkgs.git
            pkgs.jq
            pkgs.nix
          ];

          mkWrappedScript =
            name: body:
            pkgs.writeShellScript name ''
              set -euo pipefail
              export PATH="${runtimePath}:$PATH"
              ${body}
            '';
        in
        {
          tei-version-coherence = mkWrappedScript "soranoha-tei-version-coherence" ''exec bash scripts/monorepo-tei-version-coherence.sh "$@"'';
          flake-input-policy = mkWrappedScript "soranoha-flake-input-policy" ''exec ${kernel} flake-input-policy --repo "$PWD" "$@"'';
          validate = mkWrappedScript "soranoha-validate" ''
            workspace_root="$PWD"
            ${kernel} active-path-hygiene --repo "$workspace_root"
            bash tests/root-flake-output-contract-smoke.sh
            bash scripts/monorepo-tei-version-coherence.sh
            ${kernel} flake-input-policy --repo "$workspace_root"
            ${kernel} schema-hash-coherence --repo "$workspace_root"
            ${kernel} catalog-figures --repo "$workspace_root" --quotes-only
            nix flake check --no-build "$@"
            (
              cd "$workspace_root/ab-validator"
              AB_WORKSPACE_ROOT="$workspace_root" nix flake check --no-build "$@"
            )
          '';
        };
    in
    {
      formatter = forEachSystem ({ pkgs, ... }: pkgs.nixfmt);

      apps = forEachSystem (
        { pkgs, profile, ... }@ctx:
        let
          scripts = monorepoScripts pkgs;
          mkScriptApp = program: description: {
            type = "app";
            program = "${program}";
            meta.description = description;
          };
          time = "${pkgs.time}/bin/time";
        in
        {
          soranoha-kernel = mkScriptApp (mkSoranohaApp ctx {
            name = "soranoha-kernel";
            invocation = "clojure -Sthreads 1 -M:soranoha/build";
          }) "Soranoha kernel CLI (build/delta/verify) with content-derived Clojure toolchain identity";
          soranoha-replay = mkScriptApp (mkSoranohaApp ctx {
            name = "soranoha-replay";
            invocation = benchInvocation "4g" "soranoha.bench.replay" "--assets-root ${./soranoha}";
          }) "Replay source revisions through the production build and delta oracle";
          soranoha-publication-replay = mkScriptApp (mkSoranohaApp ctx {
            name = "soranoha-publication-replay";
            invocation =
              "${time} --format 'replay_elapsed_seconds=%e replay_peak_rss_kib=%M' "
              +
                benchInvocation "4g" "soranoha.bench.publication"
                  "--time-bin ${time} --assets-root ${./soranoha}";
          }) "Simulate complete publication with recorded observations and isolated fixture keys";
          soranoha-compare-serving = mkScriptApp (mkSoranohaApp ctx {
            name = "soranoha-compare-serving";
            invocation = benchInvocation "512m" "soranoha.bench.serving" "--time-bin ${time}";
          }) "Compare serving activation time and peak memory in balanced order";
          regenerate-tei-profile = mkScriptApp (pkgs.writeShellScript "regenerate-tei-profile" ''
            set -euo pipefail
            target="''${1:-$PWD/soranoha}"
            test -d "$target/schemas"
            ${pkgs.coreutils}/bin/install -m 0644 ${profile.artifacts}/tei-profile.{rng,sch} \
              ${profile.artifacts}/tei-profile-generation.json "$target/schemas/"
          '') "Regenerate the Soranoha TEI profile from its ODD";
          tei-version-coherence = mkScriptApp scripts.tei-version-coherence "Check TEI P5 source/profile version coherence";
          flake-input-policy = mkScriptApp scripts.flake-input-policy "Check release-critical flake inputs are explicitly pinned";
          validate = mkScriptApp scripts.validate "Run Soranoha validation gates";
        }
      );

      checks = forEachSystem (
        {
          pkgs,
          cljNix,
          abValidatorPackages,
          depsCache,
          tei,
          profile,
          abToolEnv,
          ...
        }:
        let
          # The repository gates written in Clojure run through the kernel app,
          # which starts from its store copy of soranoha/ and takes the
          # repository root as `--repo`.
          kernel = self.apps.${pkgs.stdenv.hostPlatform.system}.soranoha-kernel.program;
          # A check that runs a script from the repository root. bash and
          # coreutils are on every check's path; `extraTools` adds what one
          # script needs beyond them.
          mkMonorepoCheck =
            name: extraTools: script:
            pkgs.runCommand name
              {
                nativeBuildInputs = [
                  pkgs.bash
                  pkgs.coreutils
                ]
                ++ extraTools;
                src = self;
              }
              ''
                cd "$src"
                ${script}
                touch "$out"
              '';
          teiEaj = ab-validator.inputs.tei-eaj-aozora-tei;
          teiEajReportsWithProbes =
            ab-validator.apps.${pkgs.stdenv.hostPlatform.system}.tei-eaj-aozora-reports-with-probes.program;
        in
        {
          # Evaluates the ceremony image without building it. nix flake check
          # evaluates a NixOS configuration's toplevel, which does not include the
          # image derivation, so without this check an option conflict there fails
          # only when the image is built. The context is discarded so this
          # depends on the evaluation alone, not on the 1 GiB build.
          monorepo-ceremony-image =
            pkgs.runCommand "monorepo-ceremony-image"
              {
                image = builtins.unsafeDiscardStringContext self.nixosConfigurations.snh-ceremony.config.system.build.isoImage.drvPath;
              }
              ''
                echo "$image" > "$out"
              '';
          ndc9-sections-drift =
            pkgs.runCommand "soranoha-ndc9-sections-drift" { nativeBuildInputs = [ pkgs.unzip ]; }
              ''
                unzip -q ${ndc9-ld} -d "$PWD/source"
                ${self.apps.${pkgs.stdenv.hostPlatform.system}.soranoha-kernel.program} ndc9-sections \
                  --ttl "$PWD/source/ndc9.ttl" --table ${./soranoha/resources/ndc/ndc9-sections.json} --check
                touch "$out"
              '';
          tei-profile-drift = pkgs.runCommand "soranoha-tei-profile-drift" { } ''
            diff -u ${./soranoha/schemas/tei-profile.rng} ${profile.artifacts}/tei-profile.rng
            diff -u ${./soranoha/schemas/tei-profile.sch} ${profile.artifacts}/tei-profile.sch
            diff -u \
              <(${pkgs.jq}/bin/jq -S '{odd_hash, rng_hash, schematron_hash}' ${./soranoha/schemas/tei-profile-generation.json}) \
              <(${pkgs.jq}/bin/jq -S '{odd_hash, rng_hash, schematron_hash}' ${profile.artifacts}/tei-profile-generation.json)
            touch "$out"
          '';
          soranoha-typecheck =
            let
              cache = cljNix.mk-deps-cache {
                lockfile = ./soranoha/dev/typecheck/deps-lock.json;
              };
            in
            pkgs.runCommand "soranoha-typecheck" { nativeBuildInputs = [ pkgs.clojure ]; } ''
              cp -R ${./soranoha/dev/typecheck} work
              chmod -R u+w work
              mkdir -p work/src/soranoha/ori
              cp ${./soranoha/src/soranoha/ori/publication_whitespace.clj} work/src/soranoha/ori/publication_whitespace.clj
              cd work
              ${cljCacheExports cache "$TMPDIR"}
              clojure -Sthreads 1 -M:check
              touch "$out"
            '';
          # The soranoha kernel + snh conformance suite plus its lint and
          # format gates, hermetic against the wrapper's Clojure, Git, and
          # dependency-cache derivations (the same store paths the wrapper
          # binds; not the wrapper's complete environment). git backs the
          # repository-view and publication-transaction test fixtures. The
          # maintained Clojure source is covered by the lint/format gate.
          soranoha-tests =
            pkgs.runCommand "soranoha-tests"
              {
                nativeBuildInputs = [
                  pkgs.clojure
                  pkgs.git
                  pkgs.clj-kondo
                  pkgs.cljfmt
                  pkgs.cmark
                  # the static-serving acceptance runs the checked-in
                  # Caddyfile against an exported tree
                  pkgs.caddy
                ];
              }
              ''
                cp -R ${./soranoha} source
                chmod -R u+w source
                cd source

                find src test -name '*.clj' -print0 \
                  | xargs -0 clj-kondo --fail-level warning --lint
                find src test -name '*.clj' -print0 \
                  | xargs -0 cljfmt check

                ${exportsOf abToolEnv}
                ${cljCacheExports depsCache "$TMPDIR"}

                set -o pipefail
                clojure -Sthreads 1 -M:test 2>&1 | tee test-output.log
                if grep -q "^Reflection warning, soranoha/" test-output.log; then
                  exit 1
                fi

                mkdir -p "$out"
                echo "soranoha suite, lint, and format checks passed" > "$out/result.txt"
              '';

          monorepo-tei-p5-reference = tei.reference;
          monorepo-tei-version-coherence =
            mkMonorepoCheck "soranoha-monorepo-tei-version-coherence" [ pkgs.gnugrep ]
              ''
                AB_TEI_P5_ROOT="${tei.reference}" bash scripts/monorepo-tei-version-coherence.sh "$src"
              '';
          monorepo-flake-input-policy = mkMonorepoCheck "soranoha-monorepo-flake-input-policy" [ ] ''
            ${kernel} flake-input-policy --repo "$src"
          '';
          monorepo-schema-hash-coherence = mkMonorepoCheck "soranoha-monorepo-schema-hash-coherence" [ ] ''
            ${kernel} schema-hash-coherence --repo "$src"
          '';
          monorepo-figure-quotes = mkMonorepoCheck "soranoha-monorepo-figure-quotes" [ ] ''
            ${kernel} catalog-figures --repo "$src" --quotes-only
          '';
          monorepo-runtime-config = mkMonorepoCheck "soranoha-monorepo-runtime-config" [ ] ''
            bash tests/runtime-config-smoke.sh
          '';
          monorepo-active-path-hygiene = mkMonorepoCheck "soranoha-monorepo-active-path-hygiene" [ ] ''
            ${kernel} active-path-hygiene --repo "$src"
          '';
          monorepo-workflow-run-lib = mkMonorepoCheck "soranoha-monorepo-workflow-run-lib" [ pkgs.jq ] ''
            bash tests/workflow-run-lib-smoke.sh
          '';
          monorepo-tei-eaj-probe-workflow = mkMonorepoCheck "soranoha-monorepo-tei-eaj-probe-workflow" [ ] ''
            bash tests/tei-eaj-probe-workflows-smoke.sh
          '';
          monorepo-nix-format =
            mkMonorepoCheck "soranoha-monorepo-nix-format"
              [
                pkgs.findutils
                pkgs.nixfmt
              ]
              ''
                find . \
                  -path './.git' -prune -o \
                  -path './.direnv' -prune -o \
                  -path './result*' -prune -o \
                  -name '*.nix' -print0 \
                  | xargs -0 nixfmt --check
              '';
          tei-eaj-aozora-alignment-probe-generation =
            mkMonorepoCheck "soranoha-tei-eaj-aozora-alignment-probe-generation"
              [
                pkgs.gnugrep
                pkgs.gnused
                pkgs.jq
              ]
              ''
                work="$TMPDIR/tei-eaj-alignment-probe"
                mkdir -p "$work/abc"
                cd "$work"

                # Insert a paragraph after the TEI-EAJ body to produce a tail addition during probe alignment.
                cp "${teiEaj}/data/complete/tei_lib_lv4/1567_tei.xml" abc/melos.xml
                chmod u+w abc/melos.xml
                sed -i 's#</body>#<p>（古伝説と、シルレルの詩から。）</p></body>#' abc/melos.xml

                ${teiEajReportsWithProbes} \
                  --abc-melos abc/melos.xml \
                  --abc-tei 1567=abc/melos.xml \
                  --max-probe-rows 4 \
                  melos.md all-work.md workset.json alignment-probe.json alignment-probe.md

                grep -q "tail_addition" alignment-probe.md
                jq -e '.schema_version == "tei-eaj-alignment-probe-report-v1" and (.rows | length) > 0' alignment-probe.json
                jq -e '[.files[] | select(.alignment_probe)]
                  | length > 0 and any(.[]; .alignment_probe.diagnosis_counts.tail_addition == 1)' workset.json
              '';
        }
      );

      packages = forEachSystem (
        { tei, profile, ... }@ctx:
        {
          tei-p5-reference = tei.reference;
          # The compiled profile, exposed as a package so the checked-in
          # artifacts that ori/validate.clj compares with the ODD's hashes can be
          # regenerated. `apps` and `checks` import the same derivation only to
          # consume it. `just regenerate-tei-profile` copies this output into
          # place.
          tei-profile-artifacts = profile.artifacts;
          soranoha-fidelity-probe = mkSoranohaApp ctx {
            name = "soranoha-fidelity-probe";
            invocation = "clojure -Sthreads 1 -J-Xmx4g -M -m soranoha.ori.fidelity-probe --assets-root ${./soranoha}";
          };
        }
      );

      devShells = forEachSystem (
        { system, pkgs, ... }:
        {
          default = pkgs.mkShell {
            inputsFrom = [ ab-validator.devShells.${system}.default ];
            packages = [
              pkgs.caddy
              pkgs.clojure
              pkgs.cljfmt
              pkgs.cmark
              pkgs.clj-kondo
              pkgs.git
              pkgs.just
              pkgs.jq
              pkgs.nixfmt
            ];
            shellHook = ''
              if [ -f scripts/soranoha-runtime-env.sh ]; then
                source scripts/soranoha-runtime-env.sh
              fi
            '';
          };
        }
      );

      # The offline key ceremony's boot image, described in
      # docs/key-ceremony.md. The module is what an estate configuration
      # imports; the configuration builds the image from this repository
      # alone, so the image the inventory records can be rebuilt and compared.
      nixosModules.snh-ceremony-iso = ./nix/snh-ceremony-iso.nix;

      nixosConfigurations.snh-ceremony = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [ self.nixosModules.snh-ceremony-iso ];
      };
    };
}
