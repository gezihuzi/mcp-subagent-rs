# v0.7 MVP Smoke Checklist

Single-command local smoke:

```bash
./scripts/smoke_v07_release.sh
```

Checklist:

1. `doctor` executes and prints provider matrix + health sections.
2. `doctor --json` emits stable machine-readable report with `status` and `version_pins`.
3. `validate` executes and validates all discovered specs + summary contract template.
4. `list-agents --json` returns structured provider availability.
5. `run mock_runner --json` succeeds.
6. `run async_only_runner --json` fails with async-mode enforcement message.
7. `spawn async_only_runner --json` succeeds and reaches a terminal successful status.
8. `run review_runner --stage Review --parent-summary ...` produces `review/evidence.json`.
9. `artifact <handle> --path review/evidence.json --json` returns dual-review evidence.
10. `run codex_runner --json` is attempted (optional pass; may be unavailable by environment).
11. `run ollama_runner --json` is optional when `MCP_SUBAGENT_SMOKE_OLLAMA_MODEL` is set.
12. `mcp` boot path is verified with short timeout.

Provider status declarations for current build:

- `Codex`: Primary
- `Claude`: Beta
- `Gemini`: Experimental
- `Mock`: Stable local debug
- `Ollama`: Local (requires local model)
