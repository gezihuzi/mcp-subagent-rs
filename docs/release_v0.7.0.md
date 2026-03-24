# Release v0.7.0

## Scope

This release marks completion of the v0.7 technical design baseline:

- provider mapping/documentation consistency closure
- workflow policy hardening (async gate, retry/max-turn controls)
- review/archival hooks as runtime artifacts
- MCP server decomposition and finer-grained conflict lock
- `doctor --json` + provider version pin compatibility report

## Cut Checklist

1. Confirm local verification:
   - `cargo fmt && cargo test -q`
   - `cargo run -- --agents-dir examples/agents validate`
   - `./scripts/smoke_v07_release.sh`
2. Ensure docs are current:
   - `README.md`
   - `docs/mvp_smoke_v07.md`
   - `CHANGELOG.md`
3. Bump version to `0.7.0` in `Cargo.toml`.
4. Commit and tag:
   - `git tag -a v0.7.0 -m "release: v0.7.0"`
   - `git push origin v0.7.0`

## Notes

- Current MCP transport implementation remains stdio-only.
- `Ollama` is available as a local runner path and requires a configured local model.
- `Mock` remains the guaranteed fallback for development and CI smoke runs.
