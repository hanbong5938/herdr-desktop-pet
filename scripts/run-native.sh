#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
MANIFEST_PATH="$ROOT_DIR/herdr-plugin.toml"

fail() {
  printf 'herdr desktop pet: %s\n' "$*" >&2
  exit 1
}

[[ -f "$MANIFEST_PATH" ]] || fail "missing plugin manifest: $MANIFEST_PATH"
version="$(awk -F'"' '/^[[:space:]]*version[[:space:]]*=/ { print $2; exit }' "$MANIFEST_PATH")"
if [[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  app_name="HerdrDesktopPet.app"
elif [[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-beta\.(0|[1-9][0-9]*)$ ]]; then
  app_name="HerdrDesktopPetBeta.app"
else
  fail "manifest version must be a stable or beta.N semantic version, got '$version'"
fi

binary="$ROOT_DIR/dist/$app_name/Contents/MacOS/herdr-desktop-pet"
[[ -x "$binary" ]] || fail "installed native binary is missing or not executable: $binary"
exec "$binary" "$@"
