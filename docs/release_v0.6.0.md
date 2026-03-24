# Release v0.6.0

## Scope

This release marks completion of the v0.6 technical design baseline:

- local runnable command surface
- workflow-aware runtime behavior
- strict summary envelope contract
- provider tier alignment
- upgraded run persistence and events

## Cut Checklist

1. Confirm local verification:
   - `cargo fmt && cargo test -q`
   - `cargo run -- --agents-dir examples/agents validate`
   - `./scripts/smoke_v06.sh`
2. Ensure docs are current:
   - `README.md`
   - `docs/mvp_smoke_v06.md`
   - `CHANGELOG.md`
3. Bump version to `0.6.0` in `Cargo.toml`.
4. Commit and tag:
   - `git tag -a v0.6.0 -m "release: v0.6.0"`
   - `git push origin v0.6.0`

## Notes

- `Ollama` remains intentionally reserved in this release and is not treated as runnable.
- `Mock` remains the guaranteed local fallback for development and CI smoke runs.
