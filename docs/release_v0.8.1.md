# Release v0.8.1

## Scope

This patch release finalizes bootstrap onboarding usability:

- default `init` bootstrap root remains isolated, with project-root autodiscovery now working out of the box
- automatic bridge config generation at project root (`./.mcp-subagent/config.toml`)
- automatic and idempotent target-project `.gitignore` patching for runtime artifacts
- docs and tests synced for first-time setup behavior

## Cut Checklist

1. Confirm local verification:
   - `cargo fmt && cargo test -q`
   - `./scripts/smoke_v08.sh`
2. Ensure docs are current:
   - `README.md`
   - `CHANGELOG.md`
   - `PLAN.md`
   - `TODO.md`
3. Confirm version sync:
   - `Cargo.toml` -> `0.8.1`
   - `src/init.rs` `PRESET_CATALOG_VERSION` -> `v0.8.1`
4. Commit and tag:
   - `git tag -a v0.8.1 -m "release: v0.8.1"`
   - `git push origin v0.8.1`

## Notes

- MCP transport remains stdio-only in current build.
- `connect-snippet` continues to emit absolute path commands for Claude/Codex/Gemini hosts.
- CI smoke still validates codex path via fake runner to reduce host-environment variance.
