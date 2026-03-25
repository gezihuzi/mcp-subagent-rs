# Changelog

## 0.9.0 - 2026-03-25

### Highlights

- Completed v0.9 runtime strategy closure for delegation-minimal and native-first result handling.
- Added observability command surface for run lifecycle: `submit/ps/show/result/logs/timeline/watch`.
- Added MCP run observability tools parity: `list_runs/get_run_result/read_run_logs/watch_run`.
- Stabilized result contract `mcp-subagent.result.v1` and published versioned contract docs.
- Added native-first usage capture with mixed fallback and broader provider usage parsing coverage.
- Added per-provider ambient isolation diagnostics in `doctor --json` (native discovery profile + skill conflict detection).
- Added retry classification observability fields (`retry_classification`, `classification_reason`) to run result outputs.

### Provider Status (Current Build)

- `Codex`: Primary
- `Claude`: Beta
- `Gemini`: Experimental
- `Mock`: Stable local debug
- `Ollama`: Local

## 0.8.1 - 2026-03-25

### Highlights

- Fixed first-time bootstrap onboarding ergonomics: `init` now generates project bridge config and root autodiscovery works from project root.
- Added idempotent target-project `.gitignore` autopatch in bootstrap mode to suppress runtime artifact noise.
- Added test coverage for project config path resolution and `.gitignore` merge behavior (create/append/skip-catch-all).

### Provider Status (Current Build)

- `Codex`: Primary
- `Claude`: Beta
- `Gemini`: Experimental
- `Mock`: Stable local debug
- `Ollama`: Local

## 0.8.0 - 2026-03-25

### Highlights

- Completed v0.8 P0 first-success-path closure aligned to `docs/mcp-subagent_tech_design_v0.8.md`.
- Added `connect-snippet --host claude|codex|gemini` with absolute-path output and shell-safe escaping.
- Upgraded `init` onboarding template to emit executable host integration snippets (no placeholder paths).
- Added `scripts/smoke_v08.sh` with codex fake runner stabilization and connect-snippet validation for all hosts.
- Switched CI smoke baseline to v0.8 and synced release docs/checklists.

### Provider Status (Current Build)

- `Codex`: Primary
- `Claude`: Beta
- `Gemini`: Experimental
- `Mock`: Stable local debug
- `Ollama`: Local

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
