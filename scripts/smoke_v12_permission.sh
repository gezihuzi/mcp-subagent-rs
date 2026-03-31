#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

AGENTS_DIR="$TMP_DIR/agents"
STATE_DIR="$TMP_DIR/state"
PROJECT_DIR="$TMP_DIR/project"
ALLOWLIST_DIR="$TMP_DIR/allowlist"
mkdir -p "$AGENTS_DIR" "$STATE_DIR" "$PROJECT_DIR" "$ALLOWLIST_DIR"

cat >"$AGENTS_DIR/codex_direct.agent.toml" <<'TOML'
[core]
name = "codex_direct"
description = "codex direct workspace permission smoke"
provider = "codex"
instructions = "write concise output"

[runtime]
working_dir_policy = "direct"
sandbox = "workspace_write"
TOML

FAKE_CODEX_BIN="$TMP_DIR/codex"
cat >"$FAKE_CODEX_BIN" <<'SH'
#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "codex-fake 0.12.0"
  exit 0
fi

OUTPUT_LAST_MESSAGE=""
WORK_DIR=""

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --output-last-message)
      shift
      OUTPUT_LAST_MESSAGE="${1:-}"
      ;;
    --cd)
      shift
      WORK_DIR="${1:-}"
      ;;
  esac
  shift || true
done

if [[ -z "$OUTPUT_LAST_MESSAGE" || -z "$WORK_DIR" ]]; then
  echo "fake codex: missing required flags" >&2
  exit 2
fi

mkdir -p "$WORK_DIR"
echo "approved direct write" >"$WORK_DIR/permission-approved.txt"

cat >"$OUTPUT_LAST_MESSAGE" <<'JSON'
<<<MCP_SUBAGENT_SUMMARY_JSON_START>>>
{
  "summary": "permission approve flow succeeded",
  "key_findings": ["direct workspace write applied"],
  "artifacts": [],
  "open_questions": [],
  "next_steps": [],
  "exit_code": 0,
  "verification_status": "Passed",
  "touched_files": ["permission-approved.txt"],
  "plan_refs": []
}
<<<MCP_SUBAGENT_SUMMARY_JSON_END>>>
JSON

echo "fake codex stdout"
exit 0
SH
chmod +x "$FAKE_CODEX_BIN"

export PATH="$TMP_DIR:$PATH"
export MCP_SUBAGENT_CODEX_BIN="$FAKE_CODEX_BIN"
export MCP_SUBAGENT_ALLOWED_PATHS="$ALLOWLIST_DIR"

run_cmd() {
  cargo run --quiet -- \
    --agents-dir "$AGENTS_DIR" \
    --state-dir "$STATE_DIR" \
    "$@"
}

extract_handle_id() {
  awk -F'"' '/"handle_id"[[:space:]]*:/ {print $4; exit}'
}

echo "[smoke-v12-permission] spawn direct workspace run (expect permission_required)"
run_cmd spawn codex_direct --task "permission smoke" --working-dir "$PROJECT_DIR" --json >"$TMP_DIR/spawn.json"
HANDLE_ID="$(extract_handle_id <"$TMP_DIR/spawn.json")"
if [[ -z "$HANDLE_ID" ]]; then
  echo "[smoke-v12-permission] failed to parse handle_id"
  cat "$TMP_DIR/spawn.json"
  exit 1
fi

BLOCKED=0
for _ in $(seq 1 40); do
  run_cmd status "$HANDLE_ID" --json >"$TMP_DIR/status.json"
  if grep -Eq '"block_reason"[[:space:]]*:[[:space:]]*"permission_required"' "$TMP_DIR/status.json"; then
    BLOCKED=1
    break
  fi
  sleep 0.1
done
if [[ "$BLOCKED" -ne 1 ]]; then
  echo "[smoke-v12-permission] run never entered permission_required"
  cat "$TMP_DIR/status.json"
  exit 1
fi

echo "[smoke-v12-permission] approve pending request"
run_cmd approve "$HANDLE_ID" --json >"$TMP_DIR/approve.json"
grep -Eq '"status"[[:space:]]*:[[:space:]]*"running"' "$TMP_DIR/approve.json"

echo "[smoke-v12-permission] wait for resumed run to succeed"
run_cmd wait "$HANDLE_ID" --timeout-secs 20 --json >"$TMP_DIR/wait.json"
grep -Eq '"status"[[:space:]]*:[[:space:]]*"succeeded"' "$TMP_DIR/wait.json"

if [[ ! -f "$PROJECT_DIR/permission-approved.txt" ]]; then
  echo "[smoke-v12-permission] expected direct workspace file write is missing"
  ls -la "$PROJECT_DIR"
  exit 1
fi

echo "[smoke-v12-permission] verify permission event schema and lifecycle"
run_cmd events "$HANDLE_ID" --json >"$TMP_DIR/events.json"
grep -Fq '"permission.requested"' "$TMP_DIR/events.json"
grep -Fq '"permission.approved"' "$TMP_DIR/events.json"
grep -Eq '"kind"[[:space:]]*:[[:space:]]*"direct_workspace"' "$TMP_DIR/events.json"
grep -Eq '"status"[[:space:]]*:[[:space:]]*"requested"' "$TMP_DIR/events.json"
grep -Eq '"working_dir_policy"[[:space:]]*:[[:space:]]*"direct"' "$TMP_DIR/events.json"
grep -Eq '"operation"[[:space:]]*:[[:space:]]*"write"' "$TMP_DIR/events.json"
grep -Eq '"resume_mode"[[:space:]]*:[[:space:]]*"same_handle"' "$TMP_DIR/events.json"
grep -Fq '"approve_permission"' "$TMP_DIR/events.json"
grep -Fq '"deny_permission"' "$TMP_DIR/events.json"

echo "[smoke-v12-permission] ok"
