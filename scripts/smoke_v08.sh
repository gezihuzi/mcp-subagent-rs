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
provider = "mock"
instructions = "run mock smoke task"
TOML

cat >"$AGENTS_DIR/codex_runner.agent.toml" <<'TOML'
[core]
name = "codex_runner"
description = "local smoke codex agent"
provider = "codex"
instructions = "run codex smoke task"
TOML

cat >"$AGENTS_DIR/async_only_runner.agent.toml" <<'TOML'
[core]
name = "async_only_runner"
description = "runner that must execute via spawn"
provider = "mock"
instructions = "run async-only smoke task"

[runtime]
spawn_policy = "async"
TOML

cat >"$AGENTS_DIR/review_runner.agent.toml" <<'TOML'
[core]
name = "review_runner"
description = "review runner for evidence artifact smoke"
provider = "mock"
instructions = "review code and output concise findings"
tags = ["review", "correctness", "style"]

[workflow]
enabled = true
stages = ["review"]

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

FAKE_CODEX_BIN="$TMP_DIR/codex"
cat >"$FAKE_CODEX_BIN" <<'SH'
#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "codex-fake 0.8.0"
  exit 0
fi

OUTPUT_LAST_MESSAGE=""
while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --output-last-message)
      shift
      OUTPUT_LAST_MESSAGE="${1:-}"
      ;;
  esac
  shift || true
done

if [[ -z "$OUTPUT_LAST_MESSAGE" ]]; then
  echo "fake codex: missing --output-last-message" >&2
  exit 2
fi

cat >"$OUTPUT_LAST_MESSAGE" <<'JSON'
<<<MCP_SUBAGENT_SUMMARY_JSON_START>>>
{
  "summary": "fake codex smoke run succeeded",
  "key_findings": ["codex fake runner wiring ok"],
  "artifacts": [],
  "open_questions": [],
  "next_steps": ["none"],
  "exit_code": 0,
  "verification_status": "Passed",
  "touched_files": ["src/lib.rs"],
  "plan_refs": []
}
<<<MCP_SUBAGENT_SUMMARY_JSON_END>>>
JSON

echo "fake codex stdout"
exit 0
SH
chmod +x "$FAKE_CODEX_BIN"

FAKE_CLAUDE_BIN="$TMP_DIR/claude"
cat >"$FAKE_CLAUDE_BIN" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "--version" ]]; then
  echo "claude-fake 0.9.0"
  exit 0
fi
echo "claude-fake unsupported command" >&2
exit 2
SH
chmod +x "$FAKE_CLAUDE_BIN"

FAKE_GEMINI_BIN="$TMP_DIR/gemini"
cat >"$FAKE_GEMINI_BIN" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "--version" ]]; then
  echo "gemini-fake 0.9.0"
  exit 0
fi
echo "gemini-fake unsupported command" >&2
exit 2
SH
chmod +x "$FAKE_GEMINI_BIN"

# Ensure provider probes resolve deterministic fake binaries in CI/local smoke.
export PATH="$TMP_DIR:$PATH"
export MCP_SUBAGENT_CODEX_BIN="$FAKE_CODEX_BIN"
export MCP_SUBAGENT_CLAUDE_BIN="$FAKE_CLAUDE_BIN"
export MCP_SUBAGENT_GEMINI_BIN="$FAKE_GEMINI_BIN"

BOOTSTRAP_ROOT="$TMP_DIR/bootstrap"
BOOTSTRAP_BACKEND="$BOOTSTRAP_ROOT/agents/backend-coder.agent.toml"
BOOTSTRAP_CUSTOM="$BOOTSTRAP_ROOT/agents/custom.agent.toml"
BOOTSTRAP_PROJECT="$TMP_DIR/project-bootstrap-default"
LEXICAL_PROJECT_TARGET="$TMP_DIR/project-lexical-target"
LEXICAL_PROJECT_LINK="$TMP_DIR/project-lexical-link"
RELEASE_PROJECT_TARGET="$TMP_DIR/project-release-target"
RELEASE_PROJECT_LINK="$TMP_DIR/project-release-link"
RELEASE_ROOT="$TMP_DIR/release-generated-root"
SYNC_PROJECT="$TMP_DIR/project-sync"
SYNC_ROOT="$TMP_DIR/custom-root-sync"
SYNC_PROJECT_INIT="$TMP_DIR/project-sync-during-init"
SYNC_INIT_ROOT="$TMP_DIR/custom-root-sync-during-init"
mkdir -p "$BOOTSTRAP_PROJECT" "$LEXICAL_PROJECT_TARGET" "$RELEASE_PROJECT_TARGET" "$SYNC_PROJECT" "$SYNC_PROJECT_INIT"
ln -s "$LEXICAL_PROJECT_TARGET" "$LEXICAL_PROJECT_LINK"
ln -s "$RELEASE_PROJECT_TARGET" "$RELEASE_PROJECT_LINK"

run_cmd() {
  cargo run --quiet -- \
    --agents-dir "$AGENTS_DIR" \
    --state-dir "$STATE_DIR" \
    "$@"
}

extract_handle_id() {
  awk -F'"' '/"handle_id"[[:space:]]*:/ {print $4; exit}'
}

echo "[smoke-v08] init bootstrap root"
cargo run --quiet -- init --preset codex-primary-builder --root-dir "$BOOTSTRAP_ROOT" --json >"$TMP_DIR/init_bootstrap.json"
perl -0pi -e 's/memory_sources = \["auto_project_memory"\]/memory_sources = ["auto_project_memory", "active_plan"]/g' "$BOOTSTRAP_BACKEND"
cat >"$BOOTSTRAP_CUSTOM" <<'TOML'
[core]
name = "custom-agent"
description = "custom agent preserved during refresh"
provider = "mock"
instructions = "custom"
TOML

echo "[smoke-v08] default bootstrap init reports bridge config and gitignore"
(
  cd "$BOOTSTRAP_PROJECT"
  cargo run --quiet --manifest-path "$ROOT_DIR/Cargo.toml" -- init --json >"$TMP_DIR/init_default_bootstrap.json"
)
grep -Eq '"[^"]*/project-bootstrap-default/\.mcp-subagent/config\.toml"' "$TMP_DIR/init_default_bootstrap.json"
grep -Eq '"[^"]*/project-bootstrap-default/\.gitignore"' "$TMP_DIR/init_default_bootstrap.json"

echo "[smoke-v08] symlink cwd preserves lexical paths in init, doctor, and README"
(
  cd "$LEXICAL_PROJECT_LINK"
  cargo run --quiet --manifest-path "$ROOT_DIR/Cargo.toml" -- init --json >"$TMP_DIR/init_lexical_cwd.json"
  cargo run --quiet --manifest-path "$ROOT_DIR/Cargo.toml" -- doctor --json >"$TMP_DIR/doctor_lexical_cwd.json"
)
grep -Fq "\"root\": \"$LEXICAL_PROJECT_LINK/.mcp-subagent/bootstrap\"" "$TMP_DIR/init_lexical_cwd.json"
grep -Fq "\"$LEXICAL_PROJECT_LINK/.mcp-subagent/config.toml\"" "$TMP_DIR/init_lexical_cwd.json"
grep -Fq "\"cwd\": \"$LEXICAL_PROJECT_LINK\"" "$TMP_DIR/doctor_lexical_cwd.json"
grep -Fq "\"config_path\": \"$LEXICAL_PROJECT_LINK/.mcp-subagent/config.toml\"" "$TMP_DIR/doctor_lexical_cwd.json"
grep -Fq "$LEXICAL_PROJECT_LINK/.mcp-subagent/bootstrap/agents" "$LEXICAL_PROJECT_LINK/.mcp-subagent/bootstrap/README.mcp-subagent.md"

echo "[smoke-v08] doctor detects generated-root drift"
cargo run --quiet -- \
  --agents-dir "$BOOTSTRAP_ROOT/agents" \
  --state-dir "$BOOTSTRAP_ROOT/.mcp-subagent/state" \
  doctor --json >"$TMP_DIR/doctor_bootstrap_drift.json"
grep -Eq '"refresh_commands"[[:space:]]*:' "$TMP_DIR/doctor_bootstrap_drift.json"
grep -Fq "\"bootstrap_root\": \"$BOOTSTRAP_ROOT\"" "$TMP_DIR/doctor_bootstrap_drift.json"
grep -Fq "mcp-subagent init --refresh-bootstrap --root-dir '$BOOTSTRAP_ROOT'" "$TMP_DIR/doctor_bootstrap_drift.json"

echo "[smoke-v08] refresh bootstrap root"
cargo run --quiet -- init --root-dir "$BOOTSTRAP_ROOT" --refresh-bootstrap --json >"$TMP_DIR/refresh_bootstrap.json"
grep -Eq '"preset"[[:space:]]*:[[:space:]]*"refresh-bootstrap"' "$TMP_DIR/refresh_bootstrap.json"
grep -Eq 'backend-coder\.agent\.toml' "$TMP_DIR/refresh_bootstrap.json"
grep -Fq 'memory_sources = ["auto_project_memory"]' "$BOOTSTRAP_BACKEND"
if grep -Fq 'active_plan' "$BOOTSTRAP_BACKEND"; then
  echo "[smoke-v08] refresh bootstrap left legacy active_plan behind"
  exit 1
fi
grep -Fq 'name = "custom-agent"' "$BOOTSTRAP_CUSTOM"

echo "[smoke-v08] custom root without sync keeps project config untouched"
(
  cd "$SYNC_PROJECT"
  cargo run --quiet --manifest-path "$ROOT_DIR/Cargo.toml" -- \
    init --preset codex-primary-builder --root-dir "$SYNC_ROOT" --json >"$TMP_DIR/init_custom_root_no_sync.json"
  cargo run --quiet --manifest-path "$ROOT_DIR/Cargo.toml" -- \
    --agents-dir "$SYNC_ROOT/agents" \
    --state-dir "$SYNC_ROOT/.mcp-subagent/state" \
    doctor --json >"$TMP_DIR/custom_root_doctor_missing.json"
)
if [[ -e "$SYNC_PROJECT/.mcp-subagent/config.toml" ]]; then
  echo "[smoke-v08] custom-root init wrote project config without --sync-project-config"
  exit 1
fi
grep -Eq '"project_bridge"[[:space:]]*:' "$TMP_DIR/custom_root_doctor_missing.json"
grep -Eq '"status"[[:space:]]*:[[:space:]]*"missing"' "$TMP_DIR/custom_root_doctor_missing.json"
grep -Eq '"repair_command"[[:space:]]*:[[:space:]]*"mcp-subagent init --root-dir .* --sync-project-config-only"' "$TMP_DIR/custom_root_doctor_missing.json"

echo "[smoke-v08] custom root init with --sync-project-config reports bridge config"
(
  cd "$SYNC_PROJECT_INIT"
  cargo run --quiet --manifest-path "$ROOT_DIR/Cargo.toml" -- \
    init --preset codex-primary-builder --root-dir "$SYNC_INIT_ROOT" --sync-project-config --json >"$TMP_DIR/init_custom_root_sync_during_init.json"
)
grep -Eq '"[^"]*/project-sync-during-init/\.mcp-subagent/config\.toml"' "$TMP_DIR/init_custom_root_sync_during_init.json"

echo "[smoke-v08] custom root bridge-only repair writes project config"
(
  cd "$SYNC_PROJECT"
  cargo run --quiet --manifest-path "$ROOT_DIR/Cargo.toml" -- \
    init --root-dir "$SYNC_ROOT" --sync-project-config-only --json >"$TMP_DIR/init_custom_root_sync.json"
  cargo run --quiet --manifest-path "$ROOT_DIR/Cargo.toml" -- validate >"$TMP_DIR/sync_project_validate.txt"
  cargo run --quiet --manifest-path "$ROOT_DIR/Cargo.toml" -- doctor --json >"$TMP_DIR/sync_project_doctor.json"
)
grep -Fq 'agents_dirs = ["'"$SYNC_ROOT"'/agents"]' "$SYNC_PROJECT/.mcp-subagent/config.toml"
grep -Fq 'state_dir = "'"$SYNC_ROOT"'/.mcp-subagent/state"' "$SYNC_PROJECT/.mcp-subagent/config.toml"
grep -Eq '"[^"]*/project-sync/\.mcp-subagent/config\.toml"' "$TMP_DIR/init_custom_root_sync.json"
grep -Eq '"agents_loaded"[[:space:]]*:[[:space:]]*3' "$TMP_DIR/sync_project_doctor.json"
grep -Eq '"project_bridge"[[:space:]]*:' "$TMP_DIR/sync_project_doctor.json"
grep -Eq '"status"[[:space:]]*:[[:space:]]*"synced"' "$TMP_DIR/sync_project_doctor.json"
grep -Eq '"root_scope"[[:space:]]*:[[:space:]]*"project_external"' "$TMP_DIR/sync_project_doctor.json"
if [[ -e "$SYNC_PROJECT/.gitignore" ]]; then
  echo "[smoke-v08] external custom-root sync should not write project .gitignore"
  exit 1
fi

echo "[smoke-v08] release story: drift -> refresh -> bridge repair under lexical cwd"
cargo run --quiet -- init --preset codex-primary-builder --root-dir "$RELEASE_ROOT" --json >"$TMP_DIR/release_story_init_root.json"
perl -0pi -e 's/memory_sources = \["auto_project_memory"\]/memory_sources = ["auto_project_memory", "active_plan"]/g' "$RELEASE_ROOT/agents/backend-coder.agent.toml"
(
  cd "$RELEASE_PROJECT_LINK"
  cargo run --quiet --manifest-path "$ROOT_DIR/Cargo.toml" -- \
    --agents-dir "$RELEASE_ROOT/agents" \
    --state-dir "$RELEASE_ROOT/.mcp-subagent/state" \
    doctor --json >"$TMP_DIR/release_story_doctor_missing.json"
)
grep -Fq "mcp-subagent init --refresh-bootstrap --root-dir '$RELEASE_ROOT'" "$TMP_DIR/release_story_doctor_missing.json"
grep -Fq "mcp-subagent init --root-dir '$RELEASE_ROOT' --sync-project-config-only" "$TMP_DIR/release_story_doctor_missing.json"
grep -Fq "\"cwd\": \"$RELEASE_PROJECT_LINK\"" "$TMP_DIR/release_story_doctor_missing.json"

cargo run --quiet -- init --root-dir "$RELEASE_ROOT" --refresh-bootstrap --json >"$TMP_DIR/release_story_refresh.json"
if grep -Fq 'active_plan' "$RELEASE_ROOT/agents/backend-coder.agent.toml"; then
  echo "[smoke-v08] release story refresh left legacy active_plan behind"
  exit 1
fi
(
  cd "$RELEASE_PROJECT_LINK"
  cargo run --quiet --manifest-path "$ROOT_DIR/Cargo.toml" -- \
    --agents-dir "$RELEASE_ROOT/agents" \
    --state-dir "$RELEASE_ROOT/.mcp-subagent/state" \
    doctor --json >"$TMP_DIR/release_story_doctor_refreshed.json"
)
grep -Eq '"drifted_templates"[[:space:]]*:[[:space:]]*\[\]' "$TMP_DIR/release_story_doctor_refreshed.json"

(
  cd "$RELEASE_PROJECT_LINK"
  cargo run --quiet --manifest-path "$ROOT_DIR/Cargo.toml" -- \
    init --root-dir "$RELEASE_ROOT" --sync-project-config-only --json >"$TMP_DIR/release_story_sync.json"
  cargo run --quiet --manifest-path "$ROOT_DIR/Cargo.toml" -- doctor --json >"$TMP_DIR/release_story_doctor_synced.json"
)
grep -Fq "\"cwd\": \"$RELEASE_PROJECT_LINK\"" "$TMP_DIR/release_story_doctor_synced.json"
grep -Eq '"status"[[:space:]]*:[[:space:]]*"synced"' "$TMP_DIR/release_story_doctor_synced.json"
grep -Fq "\"config_path\": \"$RELEASE_PROJECT_LINK/.mcp-subagent/config.toml\"" "$TMP_DIR/release_story_doctor_synced.json"
grep -Eq '"repair_command"[[:space:]]*:[[:space:]]*null' "$TMP_DIR/release_story_doctor_synced.json"

echo "[smoke-v08] validate"
run_cmd validate >"$TMP_DIR/validate.txt"

echo "[smoke-v08] doctor"
run_cmd doctor >"$TMP_DIR/doctor.txt"

echo "[smoke-v08] doctor --json"
run_cmd doctor --json >"$TMP_DIR/doctor.json"
grep -Eq '"status"[[:space:]]*:' "$TMP_DIR/doctor.json"
grep -Eq '"version_pins"[[:space:]]*:' "$TMP_DIR/doctor.json"

echo "[smoke-v08] list-agents"
run_cmd list-agents --json >"$TMP_DIR/list_agents.json"

echo "[smoke-v08] run mock"
run_cmd run mock_runner --task "smoke mock run" --working-dir "$WORK_DIR" --json >"$TMP_DIR/run_mock.json"
grep -Eq '"status"[[:space:]]*:[[:space:]]*"succeeded"' "$TMP_DIR/run_mock.json"

echo "[smoke-v08] run async-only (must fail)"
set +e
run_cmd run async_only_runner --task "must fail sync" --working-dir "$WORK_DIR" --json >"$TMP_DIR/run_async_invalid.out" 2>"$TMP_DIR/run_async_invalid.err"
ASYNC_INVALID_RC=$?
set -e
if [[ "$ASYNC_INVALID_RC" -eq 0 ]]; then
  echo "[smoke-v08] expected run async_only_runner to fail"
  exit 1
fi
grep -Eq 'execution mode resolved to `async`' "$TMP_DIR/run_async_invalid.err"

echo "[smoke-v08] spawn async-only (must pass)"
run_cmd spawn async_only_runner --task "spawn async ok" --working-dir "$WORK_DIR" --json >"$TMP_DIR/spawn_async.json"
SPAWN_HANDLE="$(extract_handle_id <"$TMP_DIR/spawn_async.json")"
if [[ -z "$SPAWN_HANDLE" ]]; then
  echo "[smoke-v08] failed to parse spawn handle_id"
  cat "$TMP_DIR/spawn_async.json"
  exit 1
fi
run_cmd status "$SPAWN_HANDLE" --json >"$TMP_DIR/status_async.json"
grep -Eq '"status"[[:space:]]*:[[:space:]]*"(running|succeeded|failed|cancelled|timed_out)"' "$TMP_DIR/status_async.json"
grep -Eq '"stalled"[[:space:]]*:' "$TMP_DIR/status_async.json"
grep -Eq '"block_reason"[[:space:]]*:' "$TMP_DIR/status_async.json"
grep -Eq '"wait_reasons"[[:space:]]*:' "$TMP_DIR/status_async.json"
grep -Eq '"advice"[[:space:]]*:' "$TMP_DIR/status_async.json"

echo "[smoke-v08] run review runner + read review evidence artifact"
run_cmd run review_runner \
  --task "review parser behavior" \
  --stage review \
  --working-dir "$WORK_DIR" \
  --parent-summary "previous style review confirmed maintainability and style quality" \
  --json >"$TMP_DIR/run_review.json"
grep -Eq '"status"[[:space:]]*:[[:space:]]*"succeeded"' "$TMP_DIR/run_review.json"
grep -Eq '"path"[[:space:]]*:[[:space:]]*"review/evidence.json"' "$TMP_DIR/run_review.json"
REVIEW_HANDLE="$(extract_handle_id <"$TMP_DIR/run_review.json")"
run_cmd artifact "$REVIEW_HANDLE" --path review/evidence.json >"$TMP_DIR/review_evidence.json"
grep -Eq '"dual_review_satisfied"[[:space:]]*:[[:space:]]*true' "$TMP_DIR/review_evidence.json"

echo "[smoke-v08] run codex via fake runner (must pass)"
run_cmd run codex_runner --task "smoke codex fake run" --working-dir "$WORK_DIR" --json >"$TMP_DIR/run_codex.json"
grep -Eq '"status"[[:space:]]*:[[:space:]]*"succeeded"' "$TMP_DIR/run_codex.json"

echo "[smoke-v08] run codex via fake runner --stream (must pass)"
run_cmd run codex_runner --task "smoke codex fake stream run" --working-dir "$WORK_DIR" --json --stream >"$TMP_DIR/run_codex_stream.jsonl"
grep -Fq '"kind":"accepted"' "$TMP_DIR/run_codex_stream.jsonl"
grep -Fq '"kind":"stream"' "$TMP_DIR/run_codex_stream.jsonl"
grep -Fq '"stream":"stdout"' "$TMP_DIR/run_codex_stream.jsonl"
grep -Fq '"kind":"final_status"' "$TMP_DIR/run_codex_stream.jsonl"
grep -Fq '"status":"succeeded"' "$TMP_DIR/run_codex_stream.jsonl"

echo "[smoke-v08] connect snippets"
for host in claude codex gemini; do
  run_cmd connect-snippet --host "$host" >"$TMP_DIR/connect_${host}.txt"
  case "$host" in
    claude)
      grep -Fq "claude mcp add --transport stdio mcp-subagent --" "$TMP_DIR/connect_${host}.txt"
      ;;
    codex)
      grep -Fq "codex mcp add mcp-subagent --" "$TMP_DIR/connect_${host}.txt"
      ;;
    gemini)
      grep -Fq "gemini mcp add mcp-subagent" "$TMP_DIR/connect_${host}.txt"
      ;;
  esac
  grep -Fq -- "--agents-dir '$AGENTS_DIR'" "$TMP_DIR/connect_${host}.txt"
  grep -Fq -- "--state-dir '$STATE_DIR'" "$TMP_DIR/connect_${host}.txt"
  if grep -Fq "<ABSOLUTE_PATH_TO_" "$TMP_DIR/connect_${host}.txt"; then
    echo "[smoke-v08] host=$host snippet still has placeholder"
    exit 1
  fi
done

echo "[smoke-v08] mcp boot check"
set +e
timeout 3s cargo run --quiet -- \
  --agents-dir "$AGENTS_DIR" \
  --state-dir "$STATE_DIR" \
  mcp >"$TMP_DIR/mcp.stdout" 2>"$TMP_DIR/mcp.stderr"
MCP_RC=$?
set -e

if [[ "$MCP_RC" -ne 0 && "$MCP_RC" -ne 124 ]]; then
  if [[ "$MCP_RC" -eq 1 ]] && grep -q "initialize request" "$TMP_DIR/mcp.stderr"; then
    echo "[smoke-v08] mcp stdio boot verified (terminated after no initialize request)"
  else
    echo "[smoke-v08] mcp command failed unexpectedly (rc=$MCP_RC)"
    cat "$TMP_DIR/mcp.stderr"
    exit 1
  fi
fi

echo "[smoke-v08] ok"
