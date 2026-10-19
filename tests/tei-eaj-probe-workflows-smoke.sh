#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin"

cat > "$tmp/bin/nix" <<SH
#!$(command -v bash)
set -euo pipefail
: "\${TRACE:?}"
printf '%s\n' "\$*" >> "\$TRACE"
test "\$1" = run
SH
chmod +x "$tmp/bin/nix"

TRACE="$tmp/trace" PATH="$tmp/bin:$PATH" \
  bash "$repo_root/scripts/run-tei-eaj-probe-workflow.sh" alignment-probe --max-rows 4
TRACE="$tmp/trace" PATH="$tmp/bin:$PATH" \
  bash "$repo_root/scripts/run-tei-eaj-probe-workflow.sh" reports-with-probes

grep -Fxq "run ./ab-validator#tei-eaj-aozora-alignment-probe -- --max-rows 4" "$tmp/trace"
grep -Fxq "run ./ab-validator#tei-eaj-aozora-reports-with-probes --" "$tmp/trace"

if TRACE="$tmp/trace" PATH="$tmp/bin:$PATH" \
  bash "$repo_root/scripts/run-tei-eaj-probe-workflow.sh" unknown 2>/dev/null; then
  echo "unknown workflow unexpectedly succeeded" >&2
  exit 1
fi

echo "TEI-EAJ probe workflow smoke ok"
