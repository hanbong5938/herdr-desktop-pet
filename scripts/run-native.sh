#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
source "$ROOT_DIR/scripts/consumer.sh"
[[ "$(/usr/bin/uname -s)" == Darwin && "$(/usr/bin/uname -m)" == arm64 ]] || fail 'Desktop Pet requires macOS arm64'
explicit="${DESKTOP_PET_EXECUTABLE-}"
[[ "${DESKTOP_PET_EXECUTABLE+x}" != x || -n "$explicit" ]] || fail 'DESKTOP_PET_EXECUTABLE is empty'
config='' state='' socket='' args=()
while (($#)); do
  case "$1" in
    --executable|--config-dir|--state-dir|--herdr-socket)
      (($# >= 2)) || fail "$1 requires an absolute path"
      option="$1" value="$2"; shift
      absolute "$option" "$value"
      case "$option" in
        --executable) [[ -z "$explicit" ]] || fail 'duplicate executable selection'; explicit="$value" ;;
        --config-dir) [[ -z "$config" ]] || fail 'duplicate --config-dir'; config="$value" ;;
        --state-dir) [[ -z "$state" ]] || fail 'duplicate --state-dir'; state="$value" ;;
        --herdr-socket) [[ -z "$socket" ]] || fail 'duplicate --herdr-socket'; socket="$value" ;;
      esac
      ;;
    --executable=*|--config-dir=*|--state-dir=*|--herdr-socket=*)
      option="${1%%=*}" value="${1#*=}"
      absolute "$option" "$value"
      case "$option" in
        --executable) [[ -z "$explicit" ]] || fail 'duplicate executable selection'; explicit="$value" ;;
        --config-dir) [[ -z "$config" ]] || fail 'duplicate --config-dir'; config="$value" ;;
        --state-dir) [[ -z "$state" ]] || fail 'duplicate --state-dir'; state="$value" ;;
        --herdr-socket) [[ -z "$socket" ]] || fail 'duplicate --herdr-socket'; socket="$value" ;;
      esac
      ;;
    --socket|--socket=*) fail 'use adapter-owned --herdr-socket ABS; legacy --socket is not a neutral alias' ;;
    *) args+=("$1") ;;
  esac
  shift
done
# Preserve original Herdr precedence: injected profile > explicit CLI > Herdr XDG/HOME defaults.
# Never rewrite the original injection: the adapter retains host provenance.
if [[ "${HERDR_PLUGIN_CONFIG_DIR+x}" == x ]]; then
  [[ -n "$HERDR_PLUGIN_CONFIG_DIR" ]] || fail 'HERDR_PLUGIN_CONFIG_DIR is empty'
  config="$HERDR_PLUGIN_CONFIG_DIR"
fi
if [[ "${HERDR_PLUGIN_STATE_DIR+x}" == x ]]; then
  [[ -n "$HERDR_PLUGIN_STATE_DIR" ]] || fail 'HERDR_PLUGIN_STATE_DIR is empty'
  state="$HERDR_PLUGIN_STATE_DIR"
fi
if [[ -z "$config" ]]; then
  if [[ -n "${XDG_CONFIG_HOME:-}" ]]; then config="$XDG_CONFIG_HOME/herdr/plugins/config/desktop-pet"
  elif [[ -n "${HOME:-}" ]]; then config="$HOME/.config/herdr/plugins/config/desktop-pet"
  else fail 'select --config-dir ABS when no original Herdr home is available'; fi
fi
if [[ -z "$state" ]]; then
  if [[ -n "${XDG_STATE_HOME:-}" ]]; then state="$XDG_STATE_HOME/herdr/plugins/desktop-pet"
  elif [[ -n "${HOME:-}" ]]; then state="$HOME/.local/state/herdr/plugins/desktop-pet"
  else fail 'select --state-dir ABS when no original Herdr home is available'; fi
fi
absolute 'Herdr config directory' "$config"
absolute 'Herdr state directory' "$state"
[[ -n "${args[*]-}" ]] || fail 'Usage: bash scripts/run-native.sh COMMAND [--config-dir ABS --state-dir ABS] [--herdr-socket ABS] [--executable ABS]'
discover_native "$explicit"
[[ -n "$SELECTED_EXECUTABLE" ]] || fail 'Desktop Pet is absent; install the core before linking this integration'
if [[ -n "$socket" ]]; then args+=(--herdr-socket "$socket"); fi
# Native ensure attaches the same selected image/profile and preserves hidden state;
# its image/profile mismatch is surfaced unchanged. Never use open -a, start aliases,
# source builds, or a second discovery after a daemon error.
exec "$SELECTED_EXECUTABLE" --config-dir "$config" --state-dir "$state" "${args[@]}"
