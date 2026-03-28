#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <version>" >&2
  exit 1
fi

version="$1"
catalog_version="v${version}"
release_doc="docs/release_v${version}.md"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

echo "[release-check] verify Cargo.toml version"
if ! grep -q "^version = \"${version}\"$" Cargo.toml; then
  echo "Cargo.toml version mismatch: expected ${version}" >&2
  exit 1
fi

echo "[release-check] verify Cargo.lock root package version"
if ! awk -v want="${version}" '
  $0 == "[[package]]" {
    in_pkg = 1
    saw_name = 0
    saw_version = 0
    next
  }
  in_pkg && $0 == "name = \"mcp-subagent\"" {
    saw_name = 1
    next
  }
  in_pkg && saw_name && $0 == "version = \"" want "\"" {
    saw_version = 1
    print "ok"
    exit 0
  }
  in_pkg && /^version = / && saw_name && !saw_version {
    exit 1
  }
' Cargo.lock >/dev/null; then
    echo "Cargo.lock root package version mismatch: expected ${version}" >&2
    exit 1
fi

echo "[release-check] verify preset catalog version"
if ! grep -q "const PRESET_CATALOG_VERSION: &str = \"${catalog_version}\";" src/init.rs; then
  echo "PRESET_CATALOG_VERSION mismatch: expected ${catalog_version}" >&2
  exit 1
fi

echo "[release-check] verify changelog top entry"
if ! grep -q "^## ${version} - " CHANGELOG.md; then
  echo "CHANGELOG.md missing release heading for ${version}" >&2
  exit 1
fi

echo "[release-check] verify release doc"
if [[ ! -f "${release_doc}" ]]; then
  echo "missing release doc: ${release_doc}" >&2
  exit 1
fi

echo "[release-check] cargo fmt --all"
cargo fmt --all

echo "[release-check] cargo test --workspace"
cargo test --workspace

echo "[release-check] cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

echo "[release-check] bash scripts/smoke_v08.sh"
bash scripts/smoke_v08.sh

echo "[release-check] ok (${version})"
