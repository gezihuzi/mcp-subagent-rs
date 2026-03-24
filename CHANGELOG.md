# Changelog

## 0.7.0 - 2026-03-24

### Highlights

- Completed v0.7 closure aligned to `docs/mcp-subagent_tech_design_v0.7.md`.
- Hardened workflow runtime with execution policy capture, retry/max-turn controls, and stage-aware enforcement.
- Added review-stage evidence and archive knowledge-capture hooks as first-class runtime artifacts.
- Finished MCP server decomposition pass and refined conflict lock granularity to path scopes.
- Added CI/IDE-friendly `doctor --json` plus provider version pin compatibility reporting.

### Provider Status (Current Build)

- `Codex`: Primary
- `Claude`: Beta
- `Gemini`: Experimental
- `Mock`: Stable local debug
- `Ollama`: Local

## 0.6.0 - 2026-03-24

### Highlights

- Completed v0.6 runtime closure for local runnable delivery.
- Added full local CLI command surface (`doctor/validate/list-agents/run/spawn/status/cancel/artifact/mcp`).
- Upgraded summary contract to `SummaryEnvelope` with schema-first runner flags.
- Introduced `WorkflowSpec` and runtime stage/plan gate behavior.
- Added `WorkingDirPolicy::Auto`, provider tier closure (`Mock` stable, `Ollama` local runner path).
- Upgraded run state layout with additional persisted snapshots and `events.ndjson`.
- Added local smoke script and docs for reproducible acceptance.

### Provider Status (Current Build)

- `Codex`: Primary
- `Claude`: Beta
- `Gemini`: Experimental
- `Mock`: Stable local debug
- `Ollama`: Local
