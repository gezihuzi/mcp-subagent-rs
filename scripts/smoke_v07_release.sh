#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

AGENTS_DIR="$TMP_DIR/agents"
STATE_DIR="$TMP_DIR/state"
WORK_DIR="$TMP_DIR/work"
mkdir -p "$AGENTS_DIR" "$STATE_DIR" "$WORK_DIR/src"
echo "fn smoke() {}" >"$WORK_DIR/src/lib.rs"

cat >"$AGENTS_DIR/mock_runner.agent.toml" <<'TOML'
[core]
name = "mock_runner"
description = "local smoke mock agent"
provider = "Mock"
instructions = "run mock smoke task"
TOML

cat >"$AGENTS_DIR/codex_runner.agent.toml" <<'TOML'
[core]
name = "codex_runner"
description = "local smoke codex agent"
provider = "Codex"
instructions = "run codex smoke task"
TOML

cat >"$AGENTS_DIR/async_only_runner.agent.toml" <<'TOML'
[core]
name = "async_only_runner"
description = "runner that must execute via spawn"
provider = "Mock"
instructions = "run async-only smoke task"

[runtime]
spawn_policy = "Async"
TOML

cat >"$AGENTS_DIR/review_runner.agent.toml" <<'TOML'
[core]
name = "review_runner"
description = "review runner for evidence artifact smoke"
provider = "Mock"
instructions = "review code and output concise findings"
tags = ["review", "correctness", "style"]

[workflow]
enabled = true
stages = ["Review"]

[workflow.require_plan_when]
require_plan_if_touched_files_ge = 999
require_plan_if_cross_module = false
require_plan_if_parallel_agents = false
require_plan_if_new_interface = false
require_plan_if_migration = false
require_plan_if_human_approval_point = false
require_plan_if_estimated_runtime_minutes_ge = 999

[workflow.review_policy]
require_correctness_review = true
require_style_review = true
allow_same_provider_dual_review = true
prefer_cross_provider_review = false
TOML

if [[ -n "${MCP_SUBAGENT_SMOKE_OLLAMA_MODEL:-}" ]]; then
cat >"$AGENTS_DIR/ollama_runner.agent.toml" <<TOML
[core]
name = "ollama_runner"
description = "local smoke ollama agent"
provider = "Ollama"
model = "${MCP_SUBAGENT_SMOKE_OLLAMA_MODEL}"
instructions = "run ollama smoke task"
TOML
fi

run_cmd() {
  cargo run --quiet -- \
    --agents-dir "$AGENTS_DIR" \
    --state-dir "$STATE_DIR" \
    "$@"
}

extract_handle_id() {
  awk -F'"' '/"handle_id"[[:space:]]*:/ {print $4; exit}'
}

echo "[smoke-v07] doctor"
run_cmd doctor >"$TMP_DIR/doctor.txt"

echo "[smoke-v07] doctor --json"
run_cmd doctor --json >"$TMP_DIR/doctor.json"
grep -Eq '"status"[[:space:]]*:' "$TMP_DIR/doctor.json"
grep -Eq '"version_pins"[[:space:]]*:' "$TMP_DIR/doctor.json"

echo "[smoke-v07] validate"
run_cmd validate >"$TMP_DIR/validate.txt"

echo "[smoke-v07] list-agents"
run_cmd list-agents --json >"$TMP_DIR/list_agents.json"

echo "[smoke-v07] run mock"
run_cmd run mock_runner --task "smoke mock run" --working-dir "$WORK_DIR" --json >"$TMP_DIR/run_mock.json"
grep -Eq '"status"[[:space:]]*:[[:space:]]*"Succeeded"' "$TMP_DIR/run_mock.json"

echo "[smoke-v07] run async-only (must fail)"
set +e
run_cmd run async_only_runner --task "must fail sync" --working-dir "$WORK_DIR" --json >"$TMP_DIR/run_async_invalid.out" 2>"$TMP_DIR/run_async_invalid.err"
ASYNC_INVALID_RC=$?
set -e
if [[ "$ASYNC_INVALID_RC" -eq 0 ]]; then
  echo "[smoke-v07] expected run async_only_runner to fail"
  exit 1
fi
grep -Eq 'execution mode resolved to `async`' "$TMP_DIR/run_async_invalid.err"

echo "[smoke-v07] spawn async-only (must pass)"
run_cmd spawn async_only_runner --task "spawn async ok" --working-dir "$WORK_DIR" --json >"$TMP_DIR/spawn_async.json"
SPAWN_HANDLE="$(extract_handle_id <"$TMP_DIR/spawn_async.json")"
if [[ -z "$SPAWN_HANDLE" ]]; then
  echo "[smoke-v07] failed to parse spawn handle_id"
  cat "$TMP_DIR/spawn_async.json"
  exit 1
fi
for _ in {1..30}; do
  run_cmd status "$SPAWN_HANDLE" --json >"$TMP_DIR/status_async.json"
  if grep -Eq '"status"[[:space:]]*:[[:space:]]*"(Succeeded|Failed|Cancelled|TimedOut)"' "$TMP_DIR/status_async.json"; then
    break
  fi
  sleep 0.1
done
grep -Eq '"status"[[:space:]]*:[[:space:]]*"Succeeded"' "$TMP_DIR/status_async.json"

echo "[smoke-v07] run review runner + read review evidence artifact"
run_cmd run review_runner \
  --task "review parser behavior" \
  --stage Review \
  --working-dir "$WORK_DIR" \
  --parent-summary "previous style review confirmed maintainability and style quality" \
  --json >"$TMP_DIR/run_review.json"
grep -Eq '"status"[[:space:]]*:[[:space:]]*"Succeeded"' "$TMP_DIR/run_review.json"
grep -Eq '"path"[[:space:]]*:[[:space:]]*"review/evidence.json"' "$TMP_DIR/run_review.json"
REVIEW_HANDLE="$(extract_handle_id <"$TMP_DIR/run_review.json")"
run_cmd artifact "$REVIEW_HANDLE" --path review/evidence.json --json >"$TMP_DIR/review_evidence.json"
grep -Eq '"dual_review_satisfied"[[:space:]]*:[[:space:]]*true' "$TMP_DIR/review_evidence.json"

echo "[smoke-v07] run codex (optional)"
if run_cmd run codex_runner --task "smoke codex run" --working-dir "$WORK_DIR" --json >"$TMP_DIR/run_codex.json" 2>"$TMP_DIR/run_codex.err"; then
  echo "[smoke-v07] codex run succeeded"
else
  echo "[smoke-v07] codex unavailable in current environment (allowed for local smoke)"
fi

if [[ -n "${MCP_SUBAGENT_SMOKE_OLLAMA_MODEL:-}" ]]; then
  echo "[smoke-v07] run ollama (optional)"
  if run_cmd run ollama_runner --task "smoke ollama run" --working-dir "$WORK_DIR" --json >"$TMP_DIR/run_ollama.json" 2>"$TMP_DIR/run_ollama.err"; then
    echo "[smoke-v07] ollama run succeeded"
  else
    echo "[smoke-v07] ollama unavailable or model not pulled (allowed for local smoke)"
  fi
else
  echo "[smoke-v07] run ollama skipped (set MCP_SUBAGENT_SMOKE_OLLAMA_MODEL to enable)"
fi

echo "[smoke-v07] mcp boot check"
set +e
timeout 3s cargo run --quiet -- \
  --agents-dir "$AGENTS_DIR" \
  --state-dir "$STATE_DIR" \
  mcp >"$TMP_DIR/mcp.stdout" 2>"$TMP_DIR/mcp.stderr"
MCP_RC=$?
set -e

if [[ "$MCP_RC" -ne 0 && "$MCP_RC" -ne 124 ]]; then
  if [[ "$MCP_RC" -eq 1 ]] && grep -q "initialize request" "$TMP_DIR/mcp.stderr"; then
    echo "[smoke-v07] mcp stdio boot verified (terminated after no initialize request)"
  else
    echo "[smoke-v07] mcp command failed unexpectedly (rc=$MCP_RC)"
    cat "$TMP_DIR/mcp.stderr"
    exit 1
  fi
fi

echo "[smoke-v07] ok"
