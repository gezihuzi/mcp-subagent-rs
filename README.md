# mcp-subagent-rs

[![GitHub Tag](https://img.shields.io/github/v/tag/gezihuzi/mcp-subagent-rs?sort=semver)](https://github.com/gezihuzi/mcp-subagent-rs/tags)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/gezihuzi/mcp-subagent-rs#license)

Rust implementation of an MCP subagent runtime aligned to the technical design baseline in `docs/`.

## Provider Tiers

- `Mock`: stable local debug path (built-in, no external binary required)
- `Codex`: primary implementation path
- `Claude`: beta path
- `Gemini`: experimental path
- `Ollama`: local community runner path

## Command Surface

```bash
mcp-subagent mcp [AGENTS_DIR]
mcp-subagent doctor [AGENTS_DIR] [--json]
mcp-subagent validate [AGENTS_DIR]
mcp-subagent init [--preset claude-opus-supervisor-minimal|claude-opus-supervisor|codex-primary-builder|gemini-frontend-team|local-ollama-fallback|minimal-single-provider] [--root-dir ... | --in-place] [--force] [--refresh-bootstrap] [--sync-project-config] [--sync-project-config-only] [--json]
mcp-subagent connect --host claude|codex|gemini [--run-host]
mcp-subagent connect-snippet --host claude|codex|gemini
mcp-subagent clean [--all] [--dry-run] [--json]
mcp-subagent list-agents [--json]
mcp-subagent ps [--limit ...] [--json]
mcp-subagent show <handle-id> [--json]
mcp-subagent result <handle-id> [--raw | --normalized | --summary] [--json]
mcp-subagent logs <handle-id> [--stdout | --stderr] [--phase ...] [--follow] [--interval-ms ...] [--timeout-secs ...] [--phase-timeout-secs ...] [--json]
mcp-subagent timeline <handle-id> [--event ...] [--json]
mcp-subagent events [<handle-id>] [--all] [--event ...] [--phase ...] [--follow] [--interval-ms ...] [--timeout-secs ...] [--phase-timeout-secs ...] [--json]
mcp-subagent watch <handle-id> [--phase ...] [--interval-ms ...] [--timeout-secs ...] [--phase-timeout-secs ...] [--json]
mcp-subagent wait <handle-id> [--interval-ms ...] [--timeout-secs ...] [--json]
mcp-subagent stats <handle-id> [--json]
mcp-subagent run <agent> --task <task> [--task-brief ...] [--parent-summary ...] [--stage ...] [--plan ...] [--selected-file ...] [--selected-file-inline ...] [--working-dir ...] [--json]
mcp-subagent spawn <agent> --task <task> [--task-brief ...] [--parent-summary ...] [--stage ...] [--plan ...] [--selected-file ...] [--selected-file-inline ...] [--working-dir ...] [--json]
mcp-subagent submit <agent> --task <task> [--task-brief ...] [--parent-summary ...] [--stage ...] [--plan ...] [--selected-file ...] [--selected-file-inline ...] [--working-dir ...] [--json]
mcp-subagent status <handle-id> [--json]
mcp-subagent cancel <handle-id> [--json]
mcp-subagent artifact <handle-id> [--path ... | --kind summary|log|patch|json] [--json]
```

`result --json` uses stable schema contract `mcp-subagent.result.v1` (MCP `get_run_result` returns the same `contract_version` for parser alignment).
Contract reference: [`docs/result_contract_v1.md`](./docs/result_contract_v1.md).
Result output also exposes retry observability fields: `retry_classification` and `classification_reason`.

`show` renders a compact colorized view in interactive terminals; set `NO_COLOR=1` or use `--json` for plain machine-readable output.

`doctor --json` includes `ambient_isolation` diagnostics (per-provider `native_discovery` profile and workspace-visible skill conflict detection).
It now also includes `project_bridge` diagnostics: whether `./.mcp-subagent/config.toml` exists, which `agents_dir/state_dir` it points to, whether that root is internal or external to the current project, and the exact bridge-only repair command when the bridge is missing or drifted.

## MCP Transport

- Current implementation is `stdio` only (`mcp-subagent mcp`).
- HTTP transport is not implemented in current build.

MCP tools:

- `list_agents`, `run_agent`, `spawn_agent`, `get_agent_status`, `cancel_agent`, `read_agent_artifact`
- `list_runs`, `get_run_result`, `read_run_logs`, `watch_run`
- `watch_agent_events`, `get_agent_stats`

`watch_agent_events` supports optional `phase` and `phase_timeout_secs`, and returns `current_phase`, `current_phase_age_ms`, `phase_timeout_hit`, `block_reason`, `advice`.
`watch_run` also supports optional `phase` and `phase_timeout_secs`, and returns `current_phase`, `current_phase_age_ms`, `phase_timeout_hit`, `block_reason`, `advice`.
`get_agent_status` and `get_agent_stats` also return `block_reason` + `advice` so polling-only hosts can surface actionable guidance without watch streams.

Global flags:

- `--config <path>`
- `--agents-dir <path>` (repeatable)
- `--state-dir <path>`
- `--log-level <level>`

Selected file flags for `run`/`spawn`/`submit`:

- `--selected-file <path>`: pass path only
- `--selected-file-inline <path>`: read local file content and inline into selected context

Runtime policy note:

- when `delegation_context = "plan_section"`, set `plan_section_selector = "<PLAN heading>"` (required)
- reviewer-like runs automatically append checklist items from resolved `plan_section` into acceptance criteria when available

## Config Precedence

`CLI > ENV > config.toml > defaults`

- `MCP_SUBAGENT_CONFIG`
- `MCP_SUBAGENT_AGENTS_DIRS`
- `MCP_SUBAGENT_STATE_DIR`
- `MCP_SUBAGENT_LOG_LEVEL`
- `MCP_SUBAGENT_GEMINI_RESEARCH_SCRATCH_DIR` (optional: overrides stable scratch path for Gemini research-only auto routing)

Default paths:

- `agents_dirs = ["./agents"]`
- `state_dir = ".mcp-subagent/state"`

Optional provider version pins in `.mcp-subagent/config.toml`:

```toml
[provider_version_pins]
enabled = true
codex = "0.9"
claude = "1.0"
gemini = "0.7"
ollama = "0.5"
```

## Local Smoke

Run one command for minimal local acceptance:

```bash
./scripts/smoke_v08.sh
```

## Quick Onboarding (Happy Path)

Default `init` writes to an isolated bootstrap root (`./.mcp-subagent/bootstrap`) to avoid clobbering existing repo files.
It also writes a project bridge config at `./.mcp-subagent/config.toml`, so running from project root auto-resolves bootstrap `agents_dir/state_dir`.
For bootstrap mode, `init` also patches project `.gitignore` idempotently to ignore runtime artifacts.
Generated presets use the current runtime terms `context_mode`, `delegation_context`, `memory_sources`, and `working_dir_policy`.
Built-in templates keep `memory_sources = ["auto_project_memory"]` and do not inject `active_plan` by default.
If `doctor` reports bootstrap template drift, review those local files first; if the drift is accidental, run the exact `refresh_command` emitted by `doctor` (or `mcp-subagent init --refresh-bootstrap --root-dir <generated-root>`) to resync built-in templates while preserving custom agents. Default `init` still will not overwrite files silently.
Use this fixed order for first-time setup:

```bash
mcp-subagent init --preset claude-opus-supervisor-minimal
mcp-subagent validate
mcp-subagent doctor
mcp-subagent connect --host claude
```

If you explicitly want old in-place behavior, run `init --in-place`.
If you intentionally place bootstrap files somewhere else, add `--sync-project-config` once so the current project root points at that custom root without repeating `--agents-dir/--state-dir`. If the custom root already exists and you only need to repair the project bridge later, use `--sync-project-config-only`.

If you only want to print and inspect the host command without executing it, use:

```bash
mcp-subagent connect-snippet --host claude
```

## Recommended Command Flows

Project bootstrap (recommended default):

```bash
mcp-subagent init --preset claude-opus-supervisor-minimal
mcp-subagent validate
mcp-subagent doctor --json
mcp-subagent connect --host claude
mcp-subagent list-agents
```

Custom bootstrap root with project bridge sync:

```bash
mcp-subagent init --root-dir ../shared/mcp-subagent-bootstrap --sync-project-config
mcp-subagent validate
mcp-subagent doctor --json
```

If you only want to repair drifted built-in bootstrap templates without clobbering custom agents, run:

```bash
mcp-subagent init --refresh-bootstrap
```

If you use a different host:

```bash
mcp-subagent connect --host codex
mcp-subagent connect --host gemini
```

Synchronous one-shot task (blocks until completion):

```bash
mcp-subagent run fast-researcher \
  --task "Search the official site of Octoclip and return JSON: {name,url,description}" \
  --json
```

For a live terminal view on the same task, use `--stream`; this reuses the existing event/stdout/stderr follow path and ends with a final status snapshot:

```bash
mcp-subagent run fast-researcher \
  --task "Search the official site of Octoclip and return JSON: {name,url,description}" \
  --stream
```

For Gemini read-only + minimal-delegation research profiles (no selected files / no `plan_ref`), `working_dir_policy=auto` now routes execution to a stable scratch workspace by default:
`~/.mcp-subagent/provider-workspaces/gemini/research`.
When this stable scratch route is active, runtime will auto-downgrade Gemini `native_discovery="isolated"` to `minimal` to avoid auth/trust startup fallback loops.
Use `MCP_SUBAGENT_GEMINI_RESEARCH_SCRATCH_DIR` to override this path.

Asynchronous task (recommended for coding/review jobs):

```bash
mcp-subagent submit backend-coder --task "Implement feature X from PLAN.md" --json
mcp-subagent submit backend-coder --task "Implement feature X from PLAN.md" --stream
mcp-subagent ps --limit 20
mcp-subagent watch <handle-id>
```

`spawn/submit --json` now returns accepted envelope fields (`status=accepted`, `state=accepted`, `phase=accepted`, `queued_at`) for easier host-side lifecycle wiring.
CLI `spawn/submit` now defaults to "accepted + keepalive": it prints accepted envelope immediately, then keeps the process alive until the async run settles (to avoid one-shot CLI exiting before background task persistence).  
If you explicitly want immediate return in one-shot mode, set `MCP_SUBAGENT_CLI_SPAWN_ACCEPT_ONLY=1`.

`ps` now includes observability fields for running jobs: `phase`, `elapsed`, `last_event`, `stalled`, `block_reason`.
`status` now surfaces the same stall/block diagnostics for a single run, while `--stream` on `run/spawn/submit` is the direct live view shortcut over `logs/events --follow`.
`stats` now includes stage timing splits (`workspace_prepare_ms`, `provider_boot_ms`), first-output watchdog markers, and aggregated `wait_reasons`.
`watch`, `events --follow`, and `logs --follow` now emit a rolling `phase_progress` line (phase durations + current phase marker) in text mode.
`events --follow` now tails run events incrementally via cursor offsets instead of re-reading full `events.jsonl` every poll.
`watch` now uses the same incremental cursor model as `events --follow` (no full event-file re-scan per loop).
Provider stdout/stderr delta events are now emitted on the runtime path during execution (Codex/Gemini/Claude streaming paths), not only at run completion.
For long-running phases, use `--phase-timeout-secs` to fail fast when a phase does not progress.

Inspect one run end-to-end:

```bash
mcp-subagent show <handle-id>
mcp-subagent stats <handle-id>
mcp-subagent result <handle-id> --json
mcp-subagent logs <handle-id> --stderr
mcp-subagent logs <handle-id> --follow
mcp-subagent events <handle-id> --json
mcp-subagent events <handle-id> --event provider.heartbeat --follow
mcp-subagent events <handle-id> --event provider.first_output.warning --follow
mcp-subagent events --all --follow
```

`events --all --follow` is a continuous stream mode: it keeps listening for new runs/events until you stop it (Ctrl-C) or set `--timeout-secs`.

`timeline` is kept as a compatibility alias; prefer `events`.

`result --json` and MCP `get_run_result` now include retry observability fields:

- `retry_classification`: `retryable|non_retryable|unknown`
- `classification_reason`: textual reason for the final classification

Other preset examples:

```bash
mcp-subagent init --preset codex-primary-builder
mcp-subagent init --preset gemini-frontend-team
mcp-subagent init --preset local-ollama-fallback
mcp-subagent init --preset minimal-single-provider
```

This script validates:

1. `validate`
2. `doctor`
3. `doctor --json`
4. `list-agents`
5. `run` on `Mock`
6. async policy gate (`run` fail + `spawn/status` path)
7. review evidence artifact generation and readback
8. `run` on `Codex` via fake runner (required)
9. `connect-snippet --host claude|codex|gemini`
10. `mcp` boot via short-lived `timeout`

Optional local run with Ollama:

- set `provider = "Ollama"` and `core.model = "<local-model>"` in agent spec
- or set `MCP_SUBAGENT_OLLAMA_MODEL=<local-model>`

## Cleanup

Clear historical run logs/cache under resolved `state_dir`:

```bash
# default: remove state_dir/runs + state_dir/server.log + state_dir/logs
mcp-subagent clean

# preview only
mcp-subagent clean --dry-run

# remove the whole state_dir
mcp-subagent clean --all
```

## Verification Model

This runtime cannot guarantee zero hallucination. It improves verifiability by enforcing structured summary output and explicit artifacts (`verification_status`, `touched_files`, `plan_refs`, `artifact_index`), while keeping agent context isolated by policy.

## License

Licensed under either of:

- [MIT License](./LICENSE-MIT)
- [Apache License 2.0](./LICENSE-APACHE)

## Example Workflow Specs

Repository examples used by CI and regression tests:

- `examples/agents/workflow_builder.agent.toml`
- `examples/workspaces/workflow_demo/`
- `examples/workspaces/rust_service_refactor/`
- `examples/workspaces/frontend_landing_page/`

Quick validation against example specs:

```bash
cargo run -- --agents-dir examples/agents validate
```
