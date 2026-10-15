#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
source "$ROOT_DIR/scripts/validate.sh"
# Intentionally execute the real legacy daemon image. The compiled bridge entry
# owns old HERDR_PLUGIN_*/HERDR_PET_* profile precedence, --socket, and both.
# Preserve all inherited host/profile/socket environment and native arguments.
[[ -d "$ROOT_DIR/dist/HerdrDesktopPet.app" && ! -L "$ROOT_DIR/dist" && ! -L "$ROOT_DIR/dist/HerdrDesktopPet.app" ]] || fail "installed bridge must be a physical app bundle"
binary="$ROOT_DIR/dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet"
[[ -x "$binary" && -f "$binary" && ! -L "$binary" ]] || fail "missing genuine legacy daemon: $binary"
[[ "$("$binary" --version 2>/dev/null)" == 'herdr-desktop-pet 0.3.11' ]] || fail "this bridge wrapper requires the installed 0.3.11 image"
metadata="$ROOT_DIR/release.json"
[[ -f "$metadata" && ! -L "$metadata" ]] || fail "missing materialized bridge release metadata"
require_value "$metadata" distribution legacy-0.3.11
require_value "$metadata" app_version 0.3.11
require_value "$metadata" protocol 3
require_value "$metadata" observation_mode 1
require_value "$metadata" integration_api 1
revision="$(json_value "$metadata" source_core_revision)"
migration_revision="$(json_value "$metadata" herdr_migration_revision)"
helper="$ROOT_DIR/dist/HerdrDesktopPet.app/Contents/MacOS/herdr-update-coordinator"
[[ -x "$helper" && -f "$helper" && ! -L "$helper" ]] || fail "missing matching genuine legacy helper: $helper"
codesign --verify --deep --strict "$ROOT_DIR/dist/HerdrDesktopPet.app" >/dev/null 2>&1 || fail "invalid installed bridge signature"
validate_compiled_bridge "$binary"
validate_compiled_bridge "$helper"
exec "$binary" "$@"
