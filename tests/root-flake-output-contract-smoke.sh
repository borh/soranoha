#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# eval-cache disabled to match the validate cache-independence
# policy: this check exists to detect an output surface that drifted from the
# contract, which a stale eval cache would hide.
nix --option eval-cache false flake show --json --all-systems "$repo_root" | jq '
  (if has("inventory") then
     .inventory | map_values((.output.children // {}) | map_values(.children // {}))
   else . end) as $outputs
  | ["aarch64-linux", "x86_64-linux"] as $systems
  | ["apps", "checks", "devShells", "formatter", "nixosConfigurations", "nixosModules", "packages"]
    as $expected_output_names
  # Outputs keyed by name rather than by system.
  | {
      nixosConfigurations: ["snh-ceremony"],
      nixosModules: ["snh-ceremony-iso"]
    } as $expected_flat
  | {
      apps: [
        "flake-input-policy",
        "soranoha-kernel",
        "soranoha-replay",
        "soranoha-compare-serving",
        "soranoha-publication-replay",
        "regenerate-tei-profile",
        "tei-version-coherence",
        "validate"
      ],
      packages: [
        "soranoha-fidelity-probe",
        "tei-p5-reference",
        "tei-profile-artifacts"
      ],
      devShells: ["default"],
      checks: [
        "monorepo-active-path-hygiene",
        "monorepo-ceremony-image",
        "monorepo-figure-quotes",
        "monorepo-flake-input-policy",
        "monorepo-nix-format",
        "monorepo-runtime-config",
        "monorepo-schema-hash-coherence",
        "monorepo-tei-eaj-probe-workflow",
        "monorepo-tei-p5-reference",
        "monorepo-tei-version-coherence",
        "monorepo-workflow-run-lib",
        "ndc9-sections-drift",
        "soranoha-tests",
        "soranoha-typecheck",
        "tei-profile-drift",
        "tei-eaj-aozora-alignment-probe-generation"
      ]
    } as $expected
  | [
      (($outputs | keys) as $actual
       | select($actual != $expected_output_names)
       | "top-level outputs: expected \($expected_output_names), got \($actual)"),
      ($expected | to_entries[] | .key as $output | (.value | sort) as $names
       | (($outputs[$output] // {}) | keys) as $actual_systems
       | (select($actual_systems != $systems)
          | "\($output) systems: expected \($systems), got \($actual_systems)"),
         ($systems[] | . as $system
          | ((($outputs[$output] // {})[$system] // {}) | keys) as $actual
          | select($actual != $names)
          | "\($output).\($system): expected \($names), got \($actual)")),
      ($expected_flat | to_entries[] | .key as $output | (.value | sort) as $names
       | (($outputs[$output] // {}) | keys) as $actual
       | select($actual != $names)
       | "\($output): expected \($names), got \($actual)"),
      ((($outputs.formatter // {}) | keys) as $actual
       | select($actual != $systems)
       | "formatter systems: expected \($systems), got \($actual)")
    ] as $errors
  | if ($errors | length) > 0 then ($errors | join("\n") | halt_error(1)) else empty end
'

echo "root flake output contract smoke ok"
