# mcp-subagent-rs

[![GitHub Release](https://img.shields.io/github/v/release/gezihuzi/mcp-subagent-rs?display_name=tag)](https://github.com/gezihuzi/mcp-subagent-rs/releases)
[![GitHub License](https://img.shields.io/github/license/gezihuzi/mcp-subagent-rs)](https://github.com/gezihuzi/mcp-subagent-rs#license)

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
mcp-subagent init [--preset claude-opus-supervisor|codex-primary-builder|gemini-frontend-team|local-ollama-fallback|minimal-single-provider] [--root-dir ...] [--force] [--json]
mcp-subagent list-agents [--json]
mcp-subagent run <agent> --task <task> [--task-brief ...] [--parent-summary ...] [--stage ...] [--plan ...] [--selected-file ...] [--selected-file-inline ...] [--working-dir ...] [--json]
mcp-subagent spawn <agent> --task <task> [--task-brief ...] [--parent-summary ...] [--stage ...] [--plan ...] [--selected-file ...] [--selected-file-inline ...] [--working-dir ...] [--json]
mcp-subagent status <handle-id> [--json]
mcp-subagent cancel <handle-id> [--json]
mcp-subagent artifact <handle-id> [--path ... | --kind summary|log|patch|json] [--json]
```

## MCP Transport

- Current implementation is `stdio` only (`mcp-subagent mcp`).
- HTTP transport is not implemented in current build.

Global flags:

- `--config <path>`
- `--agents-dir <path>` (repeatable)
- `--state-dir <path>`
- `--log-level <level>`

Selected file flags for `run`/`spawn`:

- `--selected-file <path>`: pass path only
- `--selected-file-inline <path>`: read local file content and inline into selected context

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
./scripts/smoke_v07_release.sh
```

Initialize a local preset workspace:

```bash
mcp-subagent init --preset claude-opus-supervisor
```

Other preset examples:

```bash
mcp-subagent init --preset codex-primary-builder
mcp-subagent init --preset gemini-frontend-team
mcp-subagent init --preset local-ollama-fallback
mcp-subagent init --preset minimal-single-provider
```

This script validates:

1. `doctor`
2. `doctor --json`
3. `validate`
4. `list-agents`
5. `run` on `Mock`
6. async policy gate (`run` fail + `spawn` pass)
7. review evidence artifact generation and readback
8. `run` on `Codex` (optional)
9. `run` on `Ollama` (optional)
10. `mcp` boot via short-lived `timeout`

Optional local run with Ollama:

- set `provider = "Ollama"` and `core.model = "<local-model>"` in agent spec
- or set `MCP_SUBAGENT_OLLAMA_MODEL=<local-model>`

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

Quick validation against example specs:

```bash
cargo run -- --agents-dir examples/agents validate
```
