#!/usr/bin/env bash
set -euo pipefail
umask 077
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
source "$ROOT_DIR/scripts/consumer.sh"
[[ "$(/usr/bin/uname -s)" == Darwin && "$(/usr/bin/uname -m)" == arm64 ]] || fail 'Desktop Pet requires macOS arm64'
explicit="${DESKTOP_PET_EXECUTABLE-}"
[[ "${DESKTOP_PET_EXECUTABLE+x}" != x || -n "$explicit" ]] || fail 'DESKTOP_PET_EXECUTABLE is empty'
while (($#)); do
  case "$1" in
    --executable)
      (($# >= 2)) || fail '--executable requires ABS'
      [[ -z "$explicit" ]] || fail 'duplicate executable selection'
      explicit="$2"; absolute '--executable' "$explicit"; shift ;;
    *) fail 'Usage: bash scripts/install.sh [--executable ABS]; native builds/upgrades belong to the core installer' ;;
  esac
  shift
done
validate_release_metadata
discover_native "$explicit"
if [[ -n "$SELECTED_EXECUTABLE" ]]; then
  printf 'Reusing verified shared Desktop Pet: %s\n' "$SELECTED_EXECUTABLE"
  exit 0
fi
# Verified absence only: invalid/incompatible installations have already failed.
# Download authority and digest come solely from reviewed, materialized core metadata.
scratch="$(/usr/bin/mktemp -d /private/tmp/herdr-pet-bootstrap.XXXXXX)" || fail 'cannot reserve bootstrap download directory'
trap 'rm -rf -- "$scratch"' EXIT
metadata="$ROOT_DIR/core-release.json"
url="$(json_value "$metadata" installer_url)"
digest="$(json_value "$metadata" installer_sha256)"
/usr/bin/curl -q --fail --silent --show-error --location --proto '=https' --proto-redir '=https' --connect-timeout 15 --max-time 120 --header 'Authorization:' "$url" -o "$scratch/desktop-pet-install-v0.4.0.sh" || fail 'core installer download failed; no source/plugin-local fallback'
[[ "$(sha256_file "$scratch/desktop-pet-install-v0.4.0.sh")" == "$digest" ]] || fail 'core installer digest differs from reviewed release metadata'
# Explicit prebuilt mode prevents inherited source-build selection. The pinned core
# installer admits the shared public app atomically; this integration never owns it.
/usr/bin/env DESKTOP_PET_INSTALL_MODE=prebuilt DESKTOP_PET_DISTRIBUTION=neutral DESKTOP_PET_VERSION=0.4.0 DESKTOP_PET_REPOSITORY=hanbong5938/desktop-pet /bin/bash "$scratch/desktop-pet-install-v0.4.0.sh" --prebuilt || fail 'core installer refused admission; use explicit native upgrade/repair'
discover_native ''
[[ -n "$SELECTED_EXECUTABLE" ]] || fail 'core installer returned without a discoverable verified installation'
printf 'Using verified shared Desktop Pet: %s\n' "$SELECTED_EXECUTABLE"
