# Release v0.10.0

## Scope

This release packages the v0.10 runtime transparency and bridge-management closure:

- accepted-only async spawn semantics with follow-up observability
- direct `run/spawn/submit --stream` exposure plus richer `status` diagnostics
- parser bridge hardening for bare JSON / degraded-native success paths
- generated-root drift detection and safe `refresh-bootstrap` repair
- project bridge diagnostics, bridge-only repair, and init JSON file accounting
- lexical cwd stability plus external contract freeze for generated-root/project-bridge terminology

## Cut Checklist

1. Confirm local verification:
   - `bash scripts/release_check.sh 0.10.0`
   - Or run the underlying checks manually:
     - `cargo fmt --all`
     - `cargo test --workspace`
     - `cargo clippy --workspace --all-targets -- -D warnings`
     - `bash scripts/smoke_v08.sh`
2. Ensure docs are current:
   - `README.md`
   - `CHANGELOG.md`
   - `PLAN.md`
   - `TODO.md`
   - `docs/result_contract_v1.md`
3. Confirm version sync:
   - `Cargo.toml` -> `0.10.0`
   - `src/init.rs` `PRESET_CATALOG_VERSION` -> `v0.10.0`
4. Commit and tag:
   - `git tag -a v0.10.0 -m "release: v0.10.0"`
   - `git push origin v0.10.0`

## Notes

- MCP transport remains stdio-only in this release.
- Existing host integration commands remain compatible (`connect`, `connect-snippet`).
- Generated-root template drift and project bridge repair are now separate user-facing repair paths:
  - template drift -> `refresh-bootstrap`
  - project bridge drift/missing -> `sync-project-config-only`
