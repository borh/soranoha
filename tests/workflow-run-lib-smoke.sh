#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# shellcheck source=/dev/null
source "$repo_root/scripts/workflow-run-lib.sh"

run_file="$tmp/workflow-run.json"
workflow_init "$run_file" "shell.fixture.v1" "fixture-run"
workflow_step_pass "first" "out/first.txt"
workflow_step_fail "second" "broken"
workflow_finish "failed"

jq -e '
  .workflow_id == "shell.fixture.v1"
  and .status == "failed"
  and .step_count == 2
  and .steps_passed == 1
  and .steps_failed == 1
  and .steps[0].id == "first"
  and .steps[1].id == "second"
  and .steps[1].status == "failed"
  and .steps[1].error.message == "broken"' "$run_file" > /dev/null

echo "workflow run lib smoke ok"
