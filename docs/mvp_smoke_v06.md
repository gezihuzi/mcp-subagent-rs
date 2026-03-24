# v0.6 MVP Smoke Checklist

Single-command local smoke:

```bash
./scripts/smoke_v06.sh
```

Checklist:

1. `doctor` executes and prints provider matrix + health sections.
2. `validate` executes and validates all discovered specs + summary contract template.
3. `list-agents --json` returns structured provider availability.
4. `run mock_runner --json` succeeds.
5. `run codex_runner --json` is attempted (optional pass; may be unavailable by environment).
6. `mcp` boot path is verified with short timeout.

Provider status declarations for current build:

- `Codex`: Primary
- `Claude`: Beta
- `Gemini`: Experimental
- `Mock`: Stable local debug
- `Ollama`: Reserved
