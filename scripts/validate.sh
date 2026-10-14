#!/usr/bin/env bash
# Shared validation only; no app implementation or source-build fallback.
fail() { printf 'legacy pet ingress: %s\n' "$*" >&2; exit 1; }
json_value() { plutil -extract "$2" raw -o - "$1" 2>/dev/null || fail "missing metadata field $2 in $1"; }
require_value() { [[ "$(json_value "$1" "$2")" == "$3" ]] || fail "unexpected $2 in $1"; }
sha256_file() { local digest rest; read -r digest rest <<< "$(shasum -a 256 "$1")"; [[ "$digest" =~ ^[0-9a-f]{64}$ ]] || fail "cannot hash $1"; printf '%s' "$digest"; }
require_arm64() { local archs; archs="$(lipo -archs "$1" 2>/dev/null)" || fail "cannot inspect architecture: $1"; [[ "$archs" == arm64 ]] || fail "not arm64-only: $1"; }
validate_app() {
  local app="$1" version="$2" binary helper caps scratch output dependencies commands line library
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
    if [[ "$version" == 0.3.7 ]]; then
      [[ "$(plutil -extract integration_api raw -expect integer -o - - <<< "$caps" 2>/dev/null)" == 1 ]] || fail "bridge integration API 1 required"
      caps="$("$executable" distribution-identity 2>/dev/null)" || fail "missing compiled bridge identity"
      [[ "$(plutil -extract distribution raw -o - - <<< "$caps" 2>/dev/null)" == legacy-0.3.7 ]] || fail "not a genuine compiled bridge envelope"
      [[ "$(plutil -extract app_version raw -o - - <<< "$caps" 2>/dev/null)" == 0.3.7 ]] || fail "wrong bridge build identity"
      [[ "$(plutil -extract herdr_migration_revision raw -o - - <<< "$caps" 2>/dev/null)" == "$migration_revision" ]] || fail "compiled bridge retirement revision disagrees with actual release metadata"
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
  scratch="$(mktemp -d "${TMPDIR:-/tmp}/legacy-pack-validation.XXXXXX")" || fail "cannot create pack validation profile"
  if ! output="$("$binary" pack validate --path "$app/Contents/Resources/default" --config-dir "$scratch/config" --state-dir "$scratch/state" 2>&1)"; then
    rm -rf "$scratch"
    printf '%s\n' "$output" >&2
    fail "native Rubelia pack validation failed"
  fi
  rm -rf "$scratch"
  [[ "$output" == *'"backend":"rig"'* ]] || fail "Rubelia pack did not select native rig renderer"
}
