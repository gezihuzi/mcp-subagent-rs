# mcp-subagent-rs (v0.6.0)

Rust implementation of an MCP subagent runtime aligned to `docs/mcp-subagent_tech_design_v0.6.md`.

## Provider Tiers (v0.6)

- `Mock`: stable local debug path (built-in, no external binary required)
- `Codex`: primary implementation path
- `Claude`: beta path
- `Gemini`: experimental path
- `Ollama`: reserved (runner intentionally disabled in current build)

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

## Local Smoke (v0.6)

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

## Example Workflow Specs

Repository examples used by CI and regression tests:

- `examples/agents/workflow_builder.agent.toml`
- `examples/workspaces/workflow_demo/`

Quick validation against example specs:

```bash
cargo run -- --agents-dir examples/agents validate
```
