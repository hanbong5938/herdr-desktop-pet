#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
[[ "$#" == 0 ]] || { echo 'Usage: bash scripts/verify-integration.sh' >&2; exit 1; }
for path in native-adapter/Cargo.toml native-adapter/Cargo.lock native-adapter/src/lib.rs integrations/herdr/client-attached.patch integrations/omp/herdr-pet-outcomes.ts; do
  [[ -f "$ROOT_DIR/$path" ]] || { echo "Missing retained integration source: $ROOT_DIR/$path" >&2; exit 1; }
done
# Validate the entire working tree and Git index (including ignored tracked
# artifacts) before selecting any overlay files. A clean stage is not proof that
# the original checkout owns only integration source.
bun "$ROOT_DIR/scripts/integration-release.ts" source
# Overlay only the retained integration closure into disposable storage.
stage="$(mktemp -d "${TMPDIR:-/tmp}/herdr-integration-verify.XXXXXX")"
trap 'rm -rf -- "$stage"' EXIT
mkdir -p "$stage/scripts" "$stage/native-adapter"
cp "$ROOT_DIR/package.json" "$ROOT_DIR/bun.lock" "$ROOT_DIR/tsconfig.json" "$ROOT_DIR/herdr-plugin.toml" "$stage/"
cp "$ROOT_DIR/scripts/"*.sh "$ROOT_DIR/scripts/integration-release.ts" "$stage/scripts/"
cp "$ROOT_DIR/native-adapter/Cargo.toml" "$ROOT_DIR/native-adapter/Cargo.lock" "$stage/native-adapter/"
cp -R "$ROOT_DIR/native-adapter/src" "$stage/native-adapter/src"
cp -R "$ROOT_DIR/integrations" "$stage/integrations"
for script in "$stage/scripts/"*.sh; do bash -n "$script"; done
cd -- "$stage"
bun install --frozen-lockfile
bun run check
cargo test --locked --manifest-path native-adapter/Cargo.toml
printf 'Integration-only shell, manifest, pinned adapter/contracts and OMP TypeScript verification completed. No app was built or release published.\n'
