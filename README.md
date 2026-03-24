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
mcp-subagent doctor [AGENTS_DIR]
mcp-subagent validate [AGENTS_DIR]
mcp-subagent list-agents [--json]
mcp-subagent run <agent> --task <task> [--task-brief ...] [--parent-summary ...] [--stage ...] [--plan ...] [--selected-file ...] [--working-dir ...] [--json]
mcp-subagent spawn <agent> --task <task> [--task-brief ...] [--parent-summary ...] [--stage ...] [--plan ...] [--selected-file ...] [--working-dir ...] [--json]
mcp-subagent status <handle-id> [--json]
mcp-subagent cancel <handle-id> [--json]
mcp-subagent artifact <handle-id> [--path ... | --kind summary|log|patch|json] [--json]
```

Global flags:

- `--config <path>`
- `--agents-dir <path>` (repeatable)
- `--state-dir <path>`
- `--log-level <level>`

## Config Precedence

`CLI > ENV > config.toml > defaults`

- `MCP_SUBAGENT_CONFIG`
- `MCP_SUBAGENT_AGENTS_DIRS`
- `MCP_SUBAGENT_STATE_DIR`
- `MCP_SUBAGENT_LOG_LEVEL`

Default paths:

- `agents_dirs = ["./agents"]`
- `state_dir = ".mcp-subagent/state"`

## Local Smoke

Run one command for minimal local acceptance:

```bash
./scripts/smoke_v06.sh
```

This script validates:

1. `doctor`
2. `validate`
3. `list-agents`
4. `run` on `Mock` (and `Codex` if available)
5. `mcp` boot via short-lived `timeout`

Optional local run with Ollama:

- set `provider = "Ollama"` and `core.model = "<local-model>"` in agent spec
- or set `MCP_SUBAGENT_OLLAMA_MODEL=<local-model>`

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
