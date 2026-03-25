# Release v0.9.0

## Scope

This release packages the v0.9 runtime usability and observability closure:

- delegation-minimal + native-first result behavior
- run observability command surface (`submit/ps/show/result/logs/timeline/watch`)
- MCP parity tools (`list_runs/get_run_result/read_run_logs/watch_run`)
- ambient isolation diagnostics in `doctor --json`
- retry classification output fields for result surfaces

## Cut Checklist

1. Confirm local verification:
   - `cargo fmt && cargo test -q`
   - `./scripts/smoke_v08.sh`
2. Ensure docs are current:
   - `README.md`
   - `CHANGELOG.md`
   - `PLAN.md`
   - `TODO.md`
   - `docs/result_contract_v1.md`
3. Confirm version sync:
   - `Cargo.toml` -> `0.9.0`
   - `src/init.rs` `PRESET_CATALOG_VERSION` -> `v0.9.0`
4. Commit and tag:
   - `git tag -a v0.9.0 -m "release: v0.9.0"`
   - `git push origin v0.9.0`

## Notes

- MCP transport remains stdio-only.
- Existing host integration commands remain compatible (`connect`, `connect-snippet`).
- Retry classification is output-only in this release and does not change retry execution behavior.
