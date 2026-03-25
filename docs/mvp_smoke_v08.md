# v0.8 MVP Smoke Checklist

Single-command local smoke:

```bash
./scripts/smoke_v08.sh
```

Checklist:

1. `validate` executes and validates all discovered specs + summary contract template.
2. `doctor` executes and prints provider matrix + health sections.
3. `doctor --json` emits stable machine-readable report with `status` and `version_pins`.
4. `list-agents --json` returns structured provider availability.
5. `run mock_runner --json` succeeds.
6. `run async_only_runner --json` fails with async-mode enforcement message.
7. `spawn async_only_runner --json` succeeds and `status <handle> --json` can read run status.
8. `run review_runner --stage Review --parent-summary ...` produces `review/evidence.json`.
9. `artifact <handle> --path review/evidence.json` returns dual-review evidence.
10. `run codex_runner --json` succeeds via fake codex binary (`MCP_SUBAGENT_CODEX_BIN`).
11. `connect-snippet --host claude|codex|gemini` each emits executable commands with absolute paths and no placeholders.
12. `mcp` boot path is verified with short timeout.

Provider status declarations for current build:

- `Codex`: Primary
- `Claude`: Beta
- `Gemini`: Experimental
- `Mock`: Stable local debug
- `Ollama`: Local (requires local model)
