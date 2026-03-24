# Changelog

## 0.6.0 - 2026-03-24

### Highlights

- Completed v0.6 runtime closure for local runnable delivery.
- Added full local CLI command surface (`doctor/validate/list-agents/run/spawn/status/cancel/artifact/mcp`).
- Upgraded summary contract to `SummaryEnvelope` with schema-first runner flags.
- Introduced `WorkflowSpec` and runtime stage/plan gate behavior.
- Added `WorkingDirPolicy::Auto`, provider tier closure (`Mock` stable, `Ollama` reserved).
- Upgraded run state layout with additional persisted snapshots and `events.ndjson`.
- Added local smoke script and docs for reproducible acceptance.

### Provider Status (Current Build)

- `Codex`: Primary
- `Claude`: Beta
- `Gemini`: Experimental
- `Mock`: Stable local debug
- `Ollama`: Reserved
