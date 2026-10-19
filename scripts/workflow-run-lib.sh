#!/usr/bin/env bash

workflow_utc_now() {
  date -u +%Y-%m-%dT%H:%M:%SZ
}

workflow_json_escape() {
  jq -cn --arg value "$1" '$value'
}

workflow_init() {
  WORKFLOW_RUN_FILE="$1"
  WORKFLOW_ID="$2"
  WORKFLOW_RUN_ID="$3"
  WORKFLOW_STARTED_AT="$(workflow_utc_now)"
  WORKFLOW_STEPS_FILE="$WORKFLOW_RUN_FILE.steps.jsonl"
  mkdir -p "$(dirname "$WORKFLOW_RUN_FILE")"
  : > "$WORKFLOW_STEPS_FILE"
}

workflow_step_pass() {
  local step_id="$1"
  local output_path="$2"
  local now
  now="$(workflow_utc_now)"
  printf '{"id":%s,"status":"passed","started_at":%s,"ended_at":%s,"duration_ms":0,"requires":[],"produces":[],"inputs":[],"outputs":[{"role":"output","path":%s}],"messages":[]}\n' \
    "$(workflow_json_escape "$step_id")" \
    "$(workflow_json_escape "$now")" \
    "$(workflow_json_escape "$now")" \
    "$(workflow_json_escape "$output_path")" \
    >> "$WORKFLOW_STEPS_FILE"
}

workflow_step_fail() {
  local step_id="$1"
  local message="$2"
  local now
  now="$(workflow_utc_now)"
  printf '{"id":%s,"status":"failed","started_at":%s,"ended_at":%s,"duration_ms":0,"requires":[],"produces":[],"inputs":[],"outputs":[],"messages":[],"error":{"error_class":"shell","message":%s,"data":{}}}\n' \
    "$(workflow_json_escape "$step_id")" \
    "$(workflow_json_escape "$now")" \
    "$(workflow_json_escape "$now")" \
    "$(workflow_json_escape "$message")" \
    >> "$WORKFLOW_STEPS_FILE"
}

workflow_finish() {
  local status="$1"
  local ended_at
  ended_at="$(workflow_utc_now)"
  jq -S -s \
    --arg workflow_id "$WORKFLOW_ID" \
    --arg run_id "$WORKFLOW_RUN_ID" \
    --arg status "$status" \
    --arg started_at "$WORKFLOW_STARTED_AT" \
    --arg ended_at "$ended_at" \
    '. as $steps
     | {
         schema_id: "https://w3id.org/soranoha/schemas/workflow-run.schema.json",
         schema_version: "soranoha-workflow-run-v1",
         workflow_id: $workflow_id,
         run_id: $run_id,
         status: $status,
         started_at: $started_at,
         ended_at: $ended_at,
         duration_ms: 0,
         step_count: ($steps | length),
         steps_passed: ([$steps[] | select(.status == "passed")] | length),
         steps_failed: ([$steps[] | select(.status == "failed")] | length),
         steps: $steps
       }' "$WORKFLOW_STEPS_FILE" > "$WORKFLOW_RUN_FILE"
}
