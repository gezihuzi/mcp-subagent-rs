#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

AGENTS_DIR="$TMP_DIR/agents"
STATE_DIR="$TMP_DIR/state"
WORK_DIR="$TMP_DIR/work"
mkdir -p "$AGENTS_DIR" "$STATE_DIR" "$WORK_DIR"

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

echo "[smoke] doctor"
run_cmd doctor >"$TMP_DIR/doctor.txt"

echo "[smoke] validate"
run_cmd validate >"$TMP_DIR/validate.txt"

echo "[smoke] list-agents"
run_cmd list-agents --json >"$TMP_DIR/list_agents.json"

echo "[smoke] run mock"
run_cmd run mock_runner --task "smoke mock run" --working-dir "$WORK_DIR" --json >"$TMP_DIR/run_mock.json"
grep -Eq '"status"[[:space:]]*:[[:space:]]*"Succeeded"' "$TMP_DIR/run_mock.json"

echo "[smoke] run codex (optional)"
if run_cmd run codex_runner --task "smoke codex run" --working-dir "$WORK_DIR" --json >"$TMP_DIR/run_codex.json" 2>"$TMP_DIR/run_codex.err"; then
  echo "[smoke] codex run succeeded"
else
  echo "[smoke] codex unavailable in current environment (allowed for local smoke)"
fi

if [[ -n "${MCP_SUBAGENT_SMOKE_OLLAMA_MODEL:-}" ]]; then
  echo "[smoke] run ollama (optional)"
  if run_cmd run ollama_runner --task "smoke ollama run" --working-dir "$WORK_DIR" --json >"$TMP_DIR/run_ollama.json" 2>"$TMP_DIR/run_ollama.err"; then
    echo "[smoke] ollama run succeeded"
  else
    echo "[smoke] ollama unavailable or model not pulled (allowed for local smoke)"
  fi
else
  echo "[smoke] run ollama skipped (set MCP_SUBAGENT_SMOKE_OLLAMA_MODEL to enable)"
fi

echo "[smoke] mcp boot check"
set +e
timeout 3s cargo run --quiet -- \
  --agents-dir "$AGENTS_DIR" \
  --state-dir "$STATE_DIR" \
  mcp >"$TMP_DIR/mcp.stdout" 2>"$TMP_DIR/mcp.stderr"
MCP_RC=$?
set -e

if [[ "$MCP_RC" -ne 0 && "$MCP_RC" -ne 124 ]]; then
  if [[ "$MCP_RC" -eq 1 ]] && grep -q "initialize request" "$TMP_DIR/mcp.stderr"; then
    echo "[smoke] mcp stdio boot verified (terminated after no initialize request)"
  else
    echo "[smoke] mcp command failed unexpectedly (rc=$MCP_RC)"
    cat "$TMP_DIR/mcp.stderr"
    exit 1
  fi
fi

echo "[smoke] ok"
