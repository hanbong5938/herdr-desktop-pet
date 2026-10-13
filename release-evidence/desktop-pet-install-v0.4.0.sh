#!/usr/bin/env bash
set -euo pipefail
umask 077

# This file is also the self-contained, checksum-pinned release installer asset.
# Prebuilt mode does not read a checkout, host manifest, HOME, or host environment.
RELEASE_VERSION="0.4.0"
REPOSITORY="hanbong5938/desktop-pet"
APP_NAME="DesktopPet.app"
BUNDLE_ID="io.github.hanbong5938.desktop-pet"
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
TEMP_DIR=""
SOURCE_STAGE=""
stage_only=""

fail() { printf 'desktop pet install: %s\n' "$*" >&2; exit 1; }
usage() {
  cat >&2 <<'EOF'
Usage: bash scripts/install.sh [--prebuilt | --source [--stage-only ABS]]

  (default), --prebuilt  Install the pinned Desktop Pet 0.4.0 release only.
                         A failed download never falls back to source.
  --source               Build this verified core checkout, then admit the
                         staged app at this checkout's dist/DesktopPet.app.
                         Another healthy installation owner is not taken over.
  --stage-only ABS       With --source only: leave the validated complete stage
                         in an existing private empty directory. The updater's
                         reserved transaction owns commit and stage cleanup.

The native coordinator serializes installation with the private OS-home install
lock, both updater namespaces, installation roots, and live-profile admission.
Compatible same/higher stable installations are reused. Beta/incompatible/live
or reserved installations require an explicit native update/migration instead.

DESKTOP_PET_INSTALL_MODE accepts prebuilt (default) or source.
DESKTOP_PET_SHA256 may pin the archive digest in addition to the fixed authority.
DESKTOP_PET_CARGO_TARGET selects the Rust target for an explicit source build.
Development: bash scripts/run-native.sh --config-dir ABS --state-dir ABS start
EOF
}
cleanup() {
  [[ -z "$SOURCE_STAGE" || -n "$stage_only" ]] || rm -rf -- "$SOURCE_STAGE"
  [[ -z "$TEMP_DIR" ]] || rm -rf -- "$TEMP_DIR"
}
trap cleanup EXIT

mode="${DESKTOP_PET_INSTALL_MODE:-prebuilt}"
while (($# > 0)); do
  case "$1" in
    --prebuilt) mode=prebuilt ;;
    --source) mode=source ;;
    --stage-only)
      (($# >= 2)) || fail '--stage-only requires an absolute directory'
      stage_only="$2"
      shift
      ;;
    --help|-h) usage; exit 0 ;;
    *) usage; fail "unknown option '$1'" ;;
  esac
  shift
done
case "$mode" in prebuilt|source) ;; *) fail 'DESKTOP_PET_INSTALL_MODE must be prebuilt or source' ;; esac
[[ -z "$stage_only" || ( "$mode" == source && "$stage_only" == /* ) ]] || fail '--stage-only requires --source and an absolute output directory'
[[ "$(/usr/bin/uname -s)" == Darwin ]] || fail 'native installation requires macOS'
[[ "$(/usr/bin/uname -m)" == arm64 ]] || fail 'native installation requires macOS arm64'
[[ "${DESKTOP_PET_DISTRIBUTION:-neutral}" == neutral ]] || fail 'the shared installer admits only the neutral distribution; use the authorized bridge envelope installer for legacy migration'
[[ "${DESKTOP_PET_VERSION:-$RELEASE_VERSION}" == "$RELEASE_VERSION" ]] || fail 'requested version disagrees with this pinned release installer'
[[ "${DESKTOP_PET_REPOSITORY:-$REPOSITORY}" == "$REPOSITORY" ]] || fail 'release authority is fixed to hanbong5938/desktop-pet'

TEMP_DIR="$(/usr/bin/mktemp -d /private/tmp/desktop-pet-install.XXXXXX)" || fail 'cannot create private staging directory'

json_integer() { /usr/bin/plutil -extract "$2" raw -expect integer -o - - <<< "$1"; }
json_string() { /usr/bin/plutil -extract "$2" raw -expect string -o - - <<< "$1"; }
require_private_directory() {
  local path="$1" mode owner
  [[ -d "$path" && ! -L "$path" ]] || fail "not a real directory: $path"
  owner="$(/usr/bin/stat -f '%u' "$path")"
  [[ "$owner" == "$(/usr/bin/id -u)" ]] || fail "directory has another owner: $path"
  mode="$(/usr/bin/stat -f '%Lp' "$path")"
  (( (8#$mode & 0022) == 0 )) || fail "directory is writable by another account: $path"
}

validate_capabilities() {
  local binary="$1" capabilities
  capabilities="$("$binary" update-capabilities)" || fail "cannot read capabilities: $binary"
  [[ "$(json_integer "$capabilities" protocol)" == 4 &&
     "$(json_integer "$capabilities" observation_mode)" == 2 &&
     "$(json_integer "$capabilities" integration_api)" == 1 ]] || fail "unsupported neutral updater/integration capabilities: $binary"
}
validate_entitlements() {
  local binary="$1" expected="$2" plist json
  plist="$TEMP_DIR/entitlements.plist"
  /usr/bin/codesign -d --xml --entitlements - "$binary" > "$plist" 2>/dev/null || fail "cannot read entitlements: $binary"
  if [[ ! -s "$plist" ]]; then
    json='{}'
  else
    json="$(/usr/bin/plutil -convert json -o - "$plist" | /usr/bin/tr -d '[:space:]')" || fail "invalid entitlements: $binary"
  fi
  [[ "$json" == "$expected" ]] || fail "unexpected entitlements: $binary"
}
validate_native_pack() {
  local binary="$1" pack="$2" output
  output="$("$binary" pack validate --path "$pack" --config-dir "$TEMP_DIR/validation-config" --state-dir "$TEMP_DIR/validation-state" 2>&1)" || {
    printf '%s\n' "$output" >&2
    fail 'full native Rubelia rig validation failed'
  }
  [[ "$output" == *'"backend":"rig"'* ]] || fail 'default pack selected an unexpected renderer'
}
validate_app() {
  local app="$1" binary helper worker library dependencies line dependency commands
  [[ -d "$app" && ! -L "$app" ]] || fail "missing real staged app: $app"
  binary="$app/Contents/MacOS/desktop-pet"
  helper="$app/Contents/MacOS/desktop-pet-update-coordinator"
  worker="$app/Contents/MacOS/rig-decode-worker"
  library="$app/Contents/Frameworks/libherdr_rig.dylib"
  for executable in "$binary" "$helper" "$worker"; do
    [[ -f "$executable" && ! -L "$executable" && -x "$executable" ]] || fail "missing regular executable: $executable"
    [[ "$(/usr/bin/lipo -archs "$executable")" == arm64 ]] || fail "executable is not arm64-only: $executable"
    /usr/bin/codesign --verify --strict "$executable" || fail "invalid executable signature: $executable"
  done
  [[ -f "$library" && ! -L "$library" && "$(/usr/bin/lipo -archs "$library")" == arm64 ]] || fail 'missing arm64 rig library'
  /usr/bin/codesign --verify --deep --strict "$app" || fail 'invalid staged bundle signature'
  commands="$(/usr/bin/otool -l "$helper")" || fail 'cannot inspect coordinator loader commands'
  [[ "$commands" != *'cmd LC_RPATH'* ]] || fail 'coordinator must not have loader search paths'
  dependencies="$(/usr/bin/otool -L "$helper")" || fail 'cannot inspect coordinator system dependencies'
  dependencies="${dependencies#*$'\n'}"
  [[ -n "$dependencies" ]] || fail 'coordinator has no system dependencies'
  while IFS= read -r line; do
    [[ -n "$line" ]] || continue
    read -r dependency _ <<< "$line"
    case "$dependency" in /usr/lib/*|/System/Library/*) ;; *) fail "coordinator non-system dependency: $dependency" ;; esac
  done <<< "$dependencies"
  validate_entitlements "$worker" '{"com.apple.security.cs.allow-jit":true}'
  for executable in "$binary" "$helper" "$library" "$app"; do validate_entitlements "$executable" '{}'; done
  validate_capabilities "$helper"
  validate_capabilities "$binary"
  [[ "$("$binary" --version)" == "desktop-pet $RELEASE_VERSION" ]] || fail 'binary version disagrees with pinned release'
  [[ "$(/usr/bin/plutil -extract CFBundleIdentifier raw -o - "$app/Contents/Info.plist")" == "$BUNDLE_ID" &&
     "$(/usr/bin/plutil -extract CFBundleExecutable raw -o - "$app/Contents/Info.plist")" == desktop-pet &&
     "$(/usr/bin/plutil -extract CFBundleShortVersionString raw -o - "$app/Contents/Info.plist")" == "$RELEASE_VERSION" &&
     "$(/usr/bin/plutil -extract CFBundleVersion raw -o - "$app/Contents/Info.plist")" == "$RELEASE_VERSION" ]] || fail 'bundle identity/version disagrees with pinned neutral release'
  local manifest="$app/Contents/Resources/default/manifest.json"
  [[ "$(/usr/bin/plutil -extract id raw -o - "$manifest")" == rubelia-at12-multi-pose &&
     "$(/usr/bin/plutil -extract version raw -expect integer -o - "$manifest")" == 5 &&
     "$(/usr/bin/plutil -extract render_mode raw -o - "$manifest")" == rig ]] || fail 'default character is not the approved Rubelia v5 rig'
  for resource in "$app/Contents/Resources/LICENSE.txt" "$app/Contents/Resources/rig/decoder.js" "$app/Contents/Resources/rig/RigLimits.swift" "$app/Contents/Resources/rig/RigLimits.rs" "$app/Contents/Resources/rig/NOTICE.txt" "$app.rig-native.json"; do
    [[ -f "$resource" && ! -L "$resource" ]] || fail "missing regular release resource: $resource"
  done
  # The paired native coordinator independently checks every deployed sidecar
  # hash, source identity, signed rig closure and origin again under admission.
  validate_native_pack "$binary" "$app/Contents/Resources/default"
}

fetch() {
  /usr/bin/curl --fail --location --proto '=https' --proto-redir '=https' \
    --connect-timeout 15 --max-time 300 --retry 3 --retry-delay 1 \
    --silent --show-error "$1" --output "$2" || fail "cannot download pinned release asset: $1"
}
checksum_for() {
  local file="$1" asset="$2" line digest name extra result=""
  [[ -f "$file" && "$(/usr/bin/stat -f '%z' "$file")" -le 65536 ]] || fail 'invalid or oversized checksum asset'
  while IFS= read -r line || [[ -n "$line" ]]; do
    read -r digest name extra <<< "$line"
    name="${name#\*}"
    if [[ "$name" == "$asset" ]]; then
      [[ "$digest" =~ ^[[:xdigit:]]{64}$ && -z "$extra" && -z "$result" ]] || fail "ambiguous or invalid checksum: $asset"
      result="$(printf '%s' "$digest" | /usr/bin/tr '[:upper:]' '[:lower:]')"
    fi
  done < "$file"
  [[ -n "$result" ]] || fail "checksum asset has no exact entry: $asset"
  printf '%s' "$result"
}
verify_digest() {
  local output actual
  output="$(/usr/bin/shasum -a 256 "$1")" || fail "cannot hash $1"
  actual="${output%% *}"
  [[ "$actual" == "$2" ]] || fail "SHA-256 mismatch for $1"
}
prebuilt_stage() {
  local asset="DesktopPet-v${RELEASE_VERSION}-macos-arm64.tar.gz"
  local sidecar="DesktopPet-v${RELEASE_VERSION}-rig-native.json"
  local base="https://github.com/$REPOSITORY/releases/download/v$RELEASE_VERSION"
  local archive="$TEMP_DIR/$asset" expected entry duplicates has_app=0
  [[ "${DESKTOP_PET_RELEASE_TAG:-v$RELEASE_VERSION}" == "v$RELEASE_VERSION" &&
     "${DESKTOP_PET_ASSET_NAME:-$asset}" == "$asset" &&
     "${DESKTOP_PET_RELEASE_URL:-$base/$asset}" == "$base/$asset" &&
     "${DESKTOP_PET_CHECKSUM_URL:-$base/$asset.sha256}" == "$base/$asset.sha256" ]] || fail 'release selection disagrees with the fixed core authority'
  printf 'Downloading Desktop Pet %s from the core release authority...\n' "$RELEASE_VERSION" >&2
  fetch "$base/$asset" "$archive"
  fetch "$base/$asset.sha256" "$TEMP_DIR/archive.sha256"
  expected="$(checksum_for "$TEMP_DIR/archive.sha256" "$asset")"
  if [[ -n "${DESKTOP_PET_SHA256:-}" ]]; then
    [[ "$DESKTOP_PET_SHA256" =~ ^[[:xdigit:]]{64}$ ]] || fail 'DESKTOP_PET_SHA256 must contain 64 hex digits'
    [[ "$(printf '%s' "$DESKTOP_PET_SHA256" | /usr/bin/tr '[:upper:]' '[:lower:]')" == "$expected" ]] || fail 'explicit archive digest disagrees with the release checksum'
  fi
  verify_digest "$archive" "$expected"
  /usr/bin/tar -tzf "$archive" > "$TEMP_DIR/archive.list" || fail 'release asset is not a readable gzip tar archive'
  while IFS= read -r entry || [[ -n "$entry" ]]; do
    [[ "$entry" != ./* ]] || fail 'archive contains a noncanonical ./ path alias'
    [[ -n "$entry" ]] || fail 'archive has an empty entry'
    case "$entry" in /*|../*|*/../*|*/..|*/./*|*//*|*$'\r'*) fail "unsafe archive path: $entry" ;; esac
    case "$entry" in "$APP_NAME"|"$APP_NAME/"|"$APP_NAME"/*) has_app=1 ;; *) fail "unexpected archive entry: $entry" ;; esac
  done < "$TEMP_DIR/archive.list"
  [[ "$has_app" == 1 ]] || fail 'archive does not contain the neutral app'
  duplicates="$(LC_ALL=C /usr/bin/sort "$TEMP_DIR/archive.list" | /usr/bin/uniq -d)"
  [[ -z "$duplicates" ]] || fail 'archive contains duplicate entries'
  /usr/bin/tar -tvzf "$archive" > "$TEMP_DIR/archive.details" || fail 'cannot inspect archive entry types'
  while IFS= read -r entry || [[ -n "$entry" ]]; do
    case "${entry:0:1}" in d|-) ;; *) fail 'archive contains a link or special entry' ;; esac
  done < "$TEMP_DIR/archive.details"
  mkdir "$TEMP_DIR/extract"
  /usr/bin/tar -xzf "$archive" --no-same-owner -C "$TEMP_DIR/extract" || fail 'cannot extract verified app'
  fetch "$base/$sidecar" "$TEMP_DIR/extract/$APP_NAME.rig-native.json"
  fetch "$base/SHA256SUMS" "$TEMP_DIR/SHA256SUMS"
  verify_digest "$TEMP_DIR/extract/$APP_NAME.rig-native.json" "$(checksum_for "$TEMP_DIR/SHA256SUMS" "$sidecar")"
  STAGED_APP="$TEMP_DIR/extract/$APP_NAME"
}
source_stage() {
  local origin tool
  for tool in cargo bun npm git; do command -v "$tool" >/dev/null 2>&1 || fail "source build requires $tool"; done
  require_private_directory "$ROOT_DIR"
  origin="$(git -C "$ROOT_DIR" remote get-url origin)" || fail 'source mode requires a core Git checkout'
  case "$origin" in https://github.com/hanbong5938/desktop-pet|https://github.com/hanbong5938/desktop-pet.git|git@github.com:hanbong5938/desktop-pet.git|ssh://git@github.com/hanbong5938/desktop-pet.git) ;; *) fail 'source mode builds only the independent core checkout' ;; esac
  [[ "$(/usr/bin/plutil -extract name raw -o - "$ROOT_DIR/package.json")" == desktop-pet &&
     "$(/usr/bin/plutil -extract version raw -o - "$ROOT_DIR/package.json")" == "$RELEASE_VERSION" &&
     -f "$ROOT_DIR/native/app/Cargo.toml" && -f "$ROOT_DIR/native/core/Cargo.toml" &&
     -f "$ROOT_DIR/bun.lock" && -f "$ROOT_DIR/web/rig/package-lock.json" ]] || fail 'source mode requires the complete pinned core workspace'
  if [[ ! -e "$ROOT_DIR/dist" ]]; then mkdir -m 700 "$ROOT_DIR/dist"; fi
  require_private_directory "$ROOT_DIR/dist"
  if [[ -n "$stage_only" ]]; then
    require_private_directory "$stage_only"
    local entry
    for entry in "$stage_only"/* "$stage_only"/.[!.]* "$stage_only"/..?*; do
      [[ ! -e "$entry" && ! -L "$entry" ]] || fail '--stage-only output must be empty'
    done
    SOURCE_STAGE="$stage_only"
  else
    SOURCE_STAGE="$(/usr/bin/mktemp -d "$ROOT_DIR/dist/.desktop-pet-stage.XXXXXX")"
  fi
  # Never overwrite an existing source app before native live/reservation and
  # locator-owner admission. Only the helper promotes this complete stage.
  (cd "$ROOT_DIR" && bun install --frozen-lockfile)
  (cd "$ROOT_DIR/web/rig" && npm ci --no-audit --no-fund)
  (cd "$ROOT_DIR" && DESKTOP_PET_DISTRIBUTION=neutral bun run build:native)
  (cd "$ROOT_DIR" && DESKTOP_PET_DISTRIBUTION=neutral DESKTOP_PET_PACKAGE_STAGE_DIR="$SOURCE_STAGE" bun run package:native)
  STAGED_APP="$SOURCE_STAGE/$APP_NAME"
}

case "$mode" in prebuilt) prebuilt_stage ;; source) source_stage ;; esac
validate_app "$STAGED_APP"
if [[ -n "$stage_only" ]]; then
  printf 'Validated Desktop Pet source stage: %s\n' "$STAGED_APP"
  exit 0
fi
request="$TEMP_DIR/install-request.json"
printf '{"version":1}\n' > "$request"
/usr/bin/plutil -insert staged_app -string "$STAGED_APP" "$request"
/usr/bin/plutil -insert kind -string "$([[ "$mode" == source ]] && printf source || printf standalone)" "$request"
if [[ "$mode" == source ]]; then /usr/bin/plutil -insert source_root -string "$ROOT_DIR" "$request"; fi
chmod 600 "$request"
# The native transaction owns OS-account-home destination/locator validation,
# rechecking after lock, no-live/no-reservation admission and atomic replacement.
# A failed stage never removes the previous app or locator.
result="$("$STAGED_APP/Contents/MacOS/desktop-pet-update-coordinator" install --request "$request")" || fail 'native installation admission failed; existing installation was preserved'
executable="$(json_string "$result" executable)" || fail 'coordinator returned an invalid installation record'
printf 'Desktop Pet installation: %s\n' "$executable"
