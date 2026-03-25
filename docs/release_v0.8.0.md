# Release v0.8.0

## Scope

This release marks closure of the v0.8 beta onboarding and release baseline:

- connect snippet command surface for Claude/Codex/Gemini hosts
- init-generated onboarding with executable integration commands
- v0.8 smoke baseline (`smoke_v08.sh`) with codex fake-runner stabilization
- CI smoke alignment to v0.8
- changelog/version/catalog synchronization

## Cut Checklist

1. Confirm local verification:
   - `cargo fmt && cargo test -q`
   - `cargo run -- --agents-dir examples/agents validate`
   - `./scripts/smoke_v08.sh`
2. Ensure docs are current:
   - `README.md`
   - `docs/mvp_smoke_v08.md`
   - `CHANGELOG.md`
3. Confirm version sync:
   - `Cargo.toml` -> `0.8.0`
   - `src/init.rs` `PRESET_CATALOG_VERSION` -> `v0.8.0`
4. Commit and tag:
   - `git tag -a v0.8.0 -m "release: v0.8.0"`
   - `git push origin v0.8.0`

## Notes

- MCP transport remains stdio-only in current build.
- CI smoke validates codex path via fake binary to avoid host-environment variance.
- `Ollama` remains optional/local and is not required for v0.8 smoke pass.
