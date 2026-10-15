#!/usr/bin/env bash
# Shared validation only; no app implementation or source-build fallback.
fail() { printf 'legacy pet ingress: %s\n' "$*" >&2; exit 1; }
json_value() { plutil -extract "$2" raw -o - "$1" 2>/dev/null || fail "missing metadata field $2 in $1"; }
require_value() { [[ "$(json_value "$1" "$2")" == "$3" ]] || fail "unexpected $2 in $1"; }
sha256_file() { local digest rest; read -r digest rest <<< "$(shasum -a 256 "$1")"; [[ "$digest" =~ ^[0-9a-f]{64}$ ]] || fail "cannot hash $1"; printf '%s' "$digest"; }
require_arm64() { local archs; archs="$(lipo -archs "$1" 2>/dev/null)" || fail "cannot inspect architecture: $1"; [[ "$archs" == arm64 ]] || fail "not arm64-only: $1"; }
readonly retirement_revision=6d2ac0446f6ecb84f179e21fac3c5b02e07dc8e8
require_clean_checkout() {
  local checkout dirty tracked
  if checkout="$(git -C "$ROOT_DIR" rev-parse --show-toplevel 2>/dev/null)"; then
    checkout="$(cd -- "$checkout" && pwd -P)" || fail "cannot resolve ingress checkout"
    [[ "$checkout" == "$ROOT_DIR" ]] || fail "installation requires the materialized ingress, not a nested producer checkout"
    dirty="$(git -C "$ROOT_DIR" status --porcelain --untracked-files=all)" || fail "cannot inspect ingress user changes"
    [[ -z "$dirty" ]] || fail "tracked or untracked ingress changes block installation; never reset or hide user changes"
    tracked="$(git -C "$ROOT_DIR" ls-files -- dist)" || fail "cannot inspect tracked installation products"
    [[ -z "$tracked" ]] || fail "tracked dist files are user-owned; installation never replaces them"
    git -C "$ROOT_DIR" check-ignore -q -- dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet || fail "generated app must have the shipped narrow Git exclusion before installation"
  elif [[ -e "$ROOT_DIR/.git" || -L "$ROOT_DIR/.git" ]]; then
    fail "cannot inspect ingress Git checkout"
  fi
}
validate_compiled_bridge() {
  local executable="$1" caps field expected
  [[ "${revision:-}" =~ ^[0-9a-f]{40}$ ]] || fail "actual packaged core revision is required"
  [[ "${migration_revision:-}" == "$retirement_revision" ]] || fail "frozen Herdr v0.4.1 retirement authority required"
  caps="$("$executable" distribution-identity 2>/dev/null)" || fail "missing compiled bridge identity"
  for field in distribution app_version app_name executable helper bundle_id core_revision herdr_migration_revision protocol observation_mode integration_api; do
    case "$field" in
      distribution) expected=legacy-0.3.11 ;;
      app_version) expected=0.3.11 ;;
      app_name) expected=HerdrDesktopPet.app ;;
      executable) expected=herdr-desktop-pet ;;
      helper) expected=herdr-update-coordinator ;;
      bundle_id) expected=dev.herdr.desktop-pet ;;
      core_revision) expected="$revision" ;;
      herdr_migration_revision) expected="$retirement_revision" ;;
      protocol) expected=3 ;;
      observation_mode|integration_api) expected=1 ;;
    esac
    [[ "$(plutil -extract "$field" raw -o - - <<< "$caps" 2>/dev/null)" == "$expected" ]] || fail "compiled bridge identity disagrees on $field"
  done
}
extract_archive() {
  local archive="$1" destination="$2" app_name=HerdrDesktopPet.app entry found=0
  [[ -d "$destination" && ! -L "$destination" ]] || fail "missing private extraction directory"
  tar -tzf "$archive" > "$destination/entries" || fail "invalid gzip tar archive"
  while IFS= read -r entry || [[ -n "$entry" ]]; do
    entry="${entry#./}"
    case "$entry" in
      ''|/*|../*|*/../*|*/..|*\\*) fail "unsafe archive path $entry" ;;
      "$app_name"|"$app_name/"|"$app_name/"*) found=1 ;;
      *) fail "unexpected archive entry $entry" ;;
    esac
  done < "$destination/entries"
  [[ "$found" == 1 ]] || fail "archive contains no legacy app"
  tar -tvzf "$archive" > "$destination/details" || fail "cannot inspect tar entry types"
  while IFS= read -r entry || [[ -n "$entry" ]]; do
    case "${entry:0:1}" in d|-) ;; *) fail "archive contains a link or special file" ;; esac
  done < "$destination/details"
  mkdir "$destination/extract"
  tar -xzf "$archive" -C "$destination/extract" || fail "cannot extract approved archive"
}
validate_app() {
  local app="$1" version="$2" binary helper executable caps scratch output dependencies commands line library
  [[ -d "$app" && ! -L "$app" ]] || fail "missing regular app bundle: $app"
  binary="$app/Contents/MacOS/herdr-desktop-pet"
  helper="$app/Contents/MacOS/herdr-update-coordinator"
  for executable in "$binary" "$helper"; do
    [[ -f "$executable" && -x "$executable" && ! -L "$executable" ]] || fail "missing regular executable: $executable"
    require_arm64 "$executable"
    codesign --verify --strict "$executable" >/dev/null 2>&1 || fail "invalid executable signature: $executable"
  done
  codesign --verify --deep --strict "$app" >/dev/null 2>&1 || fail "invalid app signature"
  [[ "$("$binary" --version 2>/dev/null)" == "herdr-desktop-pet $version" ]] || fail "wrong app version"
  require_value "$app/Contents/Info.plist" CFBundleShortVersionString "$version"
  require_value "$app/Contents/Info.plist" CFBundleIdentifier dev.herdr.desktop-pet
  require_value "$app/Contents/Info.plist" CFBundleExecutable herdr-desktop-pet
  for executable in "$binary" "$helper"; do
    caps="$("$executable" update-capabilities 2>/dev/null)" || fail "cannot read updater capabilities"
    [[ "$(plutil -extract protocol raw -expect integer -o - - <<< "$caps" 2>/dev/null)" == 3 ]] || fail "protocol-3 matched pair required; protocol-2 cannot directly update to this bridge"
    [[ "$(plutil -extract observation_mode raw -expect integer -o - - <<< "$caps" 2>/dev/null)" == 1 ]] || fail "observation capability 1 required"
    if [[ "$version" == 0.3.11 ]]; then
      [[ "$(plutil -extract integration_api raw -expect integer -o - - <<< "$caps" 2>/dev/null)" == 1 ]] || fail "bridge integration API 1 required"
      validate_compiled_bridge "$executable"
    fi
  done
  commands="$(otool -l "$helper")" || fail "cannot inspect helper loader commands"
  [[ "$commands" != *"cmd LC_RPATH"* ]] || fail "helper has loader search paths"
  dependencies="$(otool -L "$helper")" || fail "cannot inspect helper dependencies"
  dependencies="${dependencies#*$'\n'}"
  [[ -n "$dependencies" ]] || fail "helper has no system dependencies"
  while IFS= read -r line; do
    [[ -n "$line" ]] || continue
    read -r library _ <<< "$line"
    case "$library" in /usr/lib/*|/System/Library/*) ;; *) fail "non-system helper dependency: $library" ;; esac
  done <<< "$dependencies"
  require_value "$app/Contents/Resources/default/manifest.json" id rubelia-at12-multi-pose
  [[ -f "$app/Contents/Resources/LICENSE.txt" ]] || fail "missing license notice"
  [[ -f "$app/Contents/Frameworks/libherdr_rig.dylib" && ! -L "$app/Contents/Frameworks/libherdr_rig.dylib" ]] || fail "missing native rig library"
  [[ -f "$app/Contents/MacOS/rig-decode-worker" && -x "$app/Contents/MacOS/rig-decode-worker" && ! -L "$app/Contents/MacOS/rig-decode-worker" ]] || fail "missing native rig worker"
  require_arm64 "$app/Contents/Frameworks/libherdr_rig.dylib"
  require_arm64 "$app/Contents/MacOS/rig-decode-worker"
  codesign --verify --strict "$app/Contents/Frameworks/libherdr_rig.dylib" >/dev/null 2>&1 || fail "invalid native rig signature"
  codesign --verify --strict "$app/Contents/MacOS/rig-decode-worker" >/dev/null 2>&1 || fail "invalid worker signature"
  [[ -f "$app/Contents/Resources/rig/decoder.js" ]] || fail "missing bundled rig decoder"
  scratch="$(/usr/bin/mktemp -d /private/tmp/legacy-pack-validation.XXXXXX)" || fail "cannot create pack validation profile"
  if ! output="$("$binary" pack validate --path "$app/Contents/Resources/default" --config-dir "$scratch/config" --state-dir "$scratch/state" 2>&1)"; then
    rm -rf "$scratch"
    printf '%s\n' "$output" >&2
    fail "native Rubelia pack validation failed"
  fi
  rm -rf "$scratch"
  [[ "$output" == *'"backend":"rig"'* ]] || fail "Rubelia pack did not select native rig renderer"
}
