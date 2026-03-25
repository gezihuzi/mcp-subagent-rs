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
mcp-subagent init [--preset claude-opus-supervisor-minimal|claude-opus-supervisor|codex-primary-builder|gemini-frontend-team|local-ollama-fallback|minimal-single-provider] [--root-dir ... | --in-place] [--force] [--json]
mcp-subagent connect --host claude|codex|gemini [--run-host]
mcp-subagent connect-snippet --host claude|codex|gemini
mcp-subagent clean [--all] [--dry-run] [--json]
mcp-subagent list-agents [--json]
mcp-subagent ps [--limit ...] [--json]
mcp-subagent show <handle-id> [--json]
mcp-subagent result <handle-id> [--raw | --normalized | --summary] [--json]
mcp-subagent logs <handle-id> [--stdout | --stderr] [--json]
mcp-subagent timeline <handle-id> [--event ...] [--json]
mcp-subagent watch <handle-id> [--interval-ms ...] [--timeout-secs ...] [--json]
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

## MCP Transport

- Current implementation is `stdio` only (`mcp-subagent mcp`).
- HTTP transport is not implemented in current build.

MCP tools:

- `list_agents`, `run_agent`, `spawn_agent`, `get_agent_status`, `cancel_agent`, `read_agent_artifact`
- `list_runs`, `get_run_result`, `read_run_logs`, `watch_run`

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
Use this fixed order for first-time setup:

```bash
mcp-subagent init --preset claude-opus-supervisor-minimal
mcp-subagent validate
mcp-subagent doctor
mcp-subagent connect --host claude
```

If you explicitly want old in-place behavior, run `init --in-place`.

If you only want to print and inspect the host command without executing it, use:

```bash
mcp-subagent connect-snippet --host claude
```

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
