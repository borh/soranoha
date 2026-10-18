#!/usr/bin/env bash
# Build and locally serve a browsable preview of the public serving tree.
# Uses fixture signing keys, an asserted assessment snapshot, and a temporary chain.
set -euo pipefail

works="${1:-8}"
port="${2:-8787}"
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
corpus="${CORPUS_CHECKOUT:-}"
work_dir="${SORANOHA_PREVIEW_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/soranoha/preview}"

if [[ -z "$corpus" ]]; then
	echo "set CORPUS_CHECKOUT to an aozorabunko checkout" >&2
	exit 2
fi

# Clean artifacts from prior preview runs.
rm -rf "$work_dir/run" "$work_dir/chain" "$work_dir/tree"
mkdir -p "$work_dir"

nix run "$repo#soranoha-kernel" -- build \
	--root "$work_dir/root" \
	--aozora-root "$corpus" \
	--assets-root "$repo/soranoha" \
	--out "$work_dir/run" \
	--limit "$works"

(cd "$repo/soranoha" && nix develop "$repo#default" --command \
	clojure -M:site-preview \
	--run "$work_dir/run" \
	--root "$work_dir/root" \
	--out "$work_dir" \
	--assets-root "$repo/soranoha")

echo
echo "serving http://127.0.0.1:$port/  (ctrl-c to stop)"
exec nix develop "$repo#default" --command env \
	SORANOHA_SERVE_LISTEN="127.0.0.1:$port" \
	SORANOHA_SERVE_ROOT="$work_dir/tree" \
	caddy run --config "$repo/soranoha/config/caddy/Caddyfile" --adapter caddyfile
