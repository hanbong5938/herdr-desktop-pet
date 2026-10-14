#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
source "$ROOT_DIR/scripts/validate.sh"
# Intentionally execute the real legacy daemon image. The compiled bridge entry
# owns old HERDR_PLUGIN_*/HERDR_PET_* profile precedence, --socket, and both.
# Preserve all inherited host/profile/socket environment and native arguments.
binary="$ROOT_DIR/dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet"
[[ -x "$binary" && -f "$binary" && ! -L "$binary" ]] || fail "missing genuine legacy daemon: $binary"
[[ "$("$binary" --version 2>/dev/null)" == 'herdr-desktop-pet 0.3.7' ]] || fail "this bridge wrapper requires the installed 0.3.7 image"
exec "$binary" "$@"
