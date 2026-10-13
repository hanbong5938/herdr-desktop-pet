#!/usr/bin/env bash
# Integration boundary only. The paired native helper owns installation/origin proof.
fail() { printf 'desktop pet Herdr integration: %s\n' "$*" >&2; exit 1; }
json_value() { /usr/bin/plutil -extract "$2" raw -o - "$1" 2>/dev/null || fail "missing metadata field $2"; }
json_text() { /usr/bin/plutil -extract "$2" raw -o - - <<< "$1" 2>/dev/null || fail "invalid native response field $2"; }
require_value() { [[ "$(json_value "$1" "$2")" == "$3" ]] || fail "unexpected metadata $2"; }
sha256_file() { local digest rest; read -r digest rest <<< "$(/usr/bin/shasum -a 256 "$1")"; [[ "$digest" =~ ^[0-9a-f]{64}$ ]] || fail "cannot hash $1"; printf '%s' "$digest"; }
absolute() { [[ "$2" == /* && "$2" != *$'\n'* && "$2" != *$'\r'* ]] || fail "$1 must be an absolute path"; }
require_owned() {
  local path="$1" mask="$2" mode
  [[ ! -L "$path" && "$(/usr/bin/stat -f '%u' "$path")" == "$(/usr/bin/id -u)" ]] || fail "unsafe owner/link: $path"
  mode="$(/usr/bin/stat -f '%Lp' "$path")"
  (( (8#$mode & mask) == 0 )) || fail "unsafe permissions: $path"
}
physical_file() {
  local path="$1" target hops=0 directory
  absolute executable "$path"
  while [[ -L "$path" ]]; do
    ((hops += 1)); ((hops <= 40)) || fail 'executable symlink cycle'
    target="$(/usr/bin/readlink "$path")" || fail 'cannot resolve executable link'
    if [[ "$target" == /* ]]; then path="$target"; else path="${path%/*}/$target"; fi
  done
  directory="$(cd -- "${path%/*}" && pwd -P)" || fail "cannot resolve executable directory: $path"
  printf '%s/%s' "$directory" "${path##*/}"
}
os_home() {
  local line home=''
  while IFS= read -r line; do
    case "$line" in 'dir: '*) home="${line#dir: }" ;; esac
  done <<< "$(/usr/bin/dscacheutil -q user -a uid "$(/usr/bin/id -u)")"
  absolute 'OS account home' "$home"
  [[ -d "$home" && ! -L "$home" && "$(cd -- "$home" && pwd -P)" == "$home" ]] || fail 'cannot resolve physical OS account home'
  printf '%s' "$home"
}
validate_release_metadata() {
  local metadata="$ROOT_DIR/core-release.json" digest revision
  [[ -f "$metadata" && ! -L "$metadata" ]] || fail 'missing reviewed core-release.json; materialize from the actual anonymous core release before cutover'
  require_owned "$metadata" 0022
  [[ "$(/usr/bin/stat -f '%l' "$metadata")" == 1 ]] || fail 'linked core release metadata'
  require_value "$metadata" version 1
  require_value "$metadata" repository hanbong5938/desktop-pet
  require_value "$metadata" tag v0.4.0
  require_value "$metadata" app_version 0.4.0
  require_value "$metadata" protocol 4
  require_value "$metadata" observation_mode 2
  require_value "$metadata" integration_api 1
  require_value "$metadata" installer_url https://github.com/hanbong5938/desktop-pet/releases/download/v0.4.0/desktop-pet-install-v0.4.0.sh
  require_value "$metadata" checksum_url https://github.com/hanbong5938/desktop-pet/releases/download/v0.4.0/desktop-pet-install-v0.4.0.sh.sha256
  for field in installer_sha256 checksum_sha256 release_record_sha256; do
    digest="$(json_value "$metadata" "$field")"
    [[ "$digest" =~ ^[0-9a-f]{64}$ && "$digest" != "$(printf '%064d' 0)" ]] || fail "invalid observed digest $field"
  done
  revision="$(json_value "$metadata" core_revision)"
  [[ "$revision" =~ ^[0-9a-f]{40}$ && "$revision" != "$(printf '%040d' 0)" ]] || fail 'invalid actual core release revision'
  [[ "$(json_value "$metadata" release_id)" =~ ^[1-9][0-9]*$ ]] || fail 'missing observed release ID'
}
preflight_pair() {
  local binary="$1" app helper file capabilities dependencies line library
  [[ "$binary" == */DesktopPet.app/Contents/MacOS/desktop-pet ]] || fail 'select the actual DesktopPet.app executable, not a shell wrapper or legacy app'
  app="${binary%/Contents/MacOS/desktop-pet}"
  helper="$app/Contents/MacOS/desktop-pet-update-coordinator"
  [[ -d "$app" && ! -L "$app" ]] || fail 'native bundle missing; install/upgrade the core explicitly'
  require_owned "$app" 0022
  require_value "$app/Contents/Info.plist" CFBundleIdentifier io.github.hanbong5938.desktop-pet
  require_value "$app/Contents/Info.plist" CFBundleExecutable desktop-pet
  for file in "$binary" "$helper"; do
    [[ -f "$file" && -x "$file" && ! -L "$file" ]] || fail "missing paired native executable: $file"
    require_owned "$file" 0022
    [[ "$(/usr/bin/lipo -archs "$file" 2>/dev/null)" == arm64 ]] || fail "not an arm64 native image: $file"
    /usr/bin/codesign --verify --strict "$file" >/dev/null 2>&1 || fail "invalid native signature: $file"
  done
  /usr/bin/codesign --verify --deep --strict "$app" >/dev/null 2>&1 || fail 'invalid bundle signature; explicit native repair required'
  [[ "$(/usr/bin/otool -l "$helper")" != *'cmd LC_RPATH'* ]] || fail 'helper has loader search paths'
  dependencies="$(/usr/bin/otool -L "$helper")" || fail 'cannot inspect paired helper'
  dependencies="${dependencies#*$'\n'}"
  [[ -n "$dependencies" ]] || fail 'empty helper dependency closure'
  while IFS= read -r line; do
    [[ -n "$line" ]] || continue
    read -r library _ <<< "$line"
    case "$library" in /usr/lib/*|/System/Library/*) ;; *) fail 'paired helper has non-system dependencies' ;; esac
  done <<< "$dependencies"
  for file in "$binary" "$helper"; do
    capabilities="$("$file" update-capabilities)" || fail 'incompatible installed app; explicit native upgrade required'
    [[ "$(json_text "$capabilities" protocol)" == 4 && "$(json_text "$capabilities" observation_mode)" == 2 && "$(json_text "$capabilities" integration_api)" == 1 ]] || fail 'incompatible installed app; explicit native upgrade required'
  done
  SELECTED_HELPER="$helper"
}
accept_discovery() {
  local record="$1" digest
  [[ "$record" != null ]] || fail 'native helper did not find the selected installation'
  [[ "$(json_text "$record" version)" == 1 && "$(json_text "$record" protocol)" == 4 && "$(json_text "$record" observation_mode)" == 2 && "$(json_text "$record" integration_api)" == 1 ]] || fail 'incompatible discovery proof'
  SELECTED_EXECUTABLE="$(json_text "$record" executable)"
  absolute 'discovered executable' "$SELECTED_EXECUTABLE"
  [[ "$SELECTED_EXECUTABLE" == "$(json_text "$record" image.path)" ]] || fail 'discovery image mismatch'
  case "$(json_text "$record" kind)" in homebrew|standalone|source) ;; *) fail 'unsupported native installation owner' ;; esac
  digest="$(json_text "$record" image.sha256)"
  [[ "$digest" =~ ^[0-9a-f]{64}$ && "$(sha256_file "$SELECTED_EXECUTABLE")" == "$digest" ]] || fail 'native image changed after discovery'
}
discover_native() {
  local explicit="$1" home locator directory mode candidate path_entry manager physical prefix record
  SELECTED_EXECUTABLE=''
  if [[ -n "$explicit" ]]; then
    absolute '--executable' "$explicit"
    candidate="$(physical_file "$explicit")"
    preflight_pair "$candidate"
    record="$("$SELECTED_HELPER" discover --executable "$explicit" --json)" || fail 'invalid explicit installation; no fallback (use explicit native upgrade/repair)'
    accept_discovery "$record"
    return
  fi
  home="$(os_home)"
  directory="$home/.local"
  for component in .local share desktop-pet; do
    if [[ "$component" != .local ]]; then directory="$directory/$component"; fi
    if [[ -e "$directory" || -L "$directory" ]]; then
      [[ -d "$directory" && ! -L "$directory" && "$(cd -- "$directory" && pwd -P)" == "$directory" ]] || fail 'unsafe installation locator namespace'
      if [[ "$component" == desktop-pet ]]; then require_owned "$directory" 0077; else require_owned "$directory" 0022; fi
    fi
  done
  locator="$home/.local/share/desktop-pet/installation.json"
  if [[ -e "$locator" || -L "$locator" ]]; then
    [[ -f "$locator" && ! -L "$locator" && "$(/usr/bin/stat -f '%l' "$locator")" == 1 ]] || fail 'invalid installation locator; no fallback'
    require_owned "$locator" 0077
    require_value "$locator" version 1
    candidate="$(json_value "$locator" executable)"
    absolute 'locator executable' "$candidate"
    candidate="$(physical_file "$candidate")"
    preflight_pair "$candidate"
    record="$("$SELECTED_HELPER" discover --json)" || fail 'invalid installation locator/origin; no fallback'
    accept_discovery "$record"
    return
  fi
  local paths=()
  IFS=: read -r -a paths <<< "${PATH:-}"
  for path_entry in ${paths[@]+"${paths[@]}"}; do
    [[ "$path_entry" == /* ]] || continue
    candidate="$path_entry/desktop-pet"
    [[ -e "$candidate" || -L "$candidate" ]] || continue
    candidate="$(physical_file "$candidate")"
    # Never run a PATH shell wrapper as evidence for the application image.
    [[ "$candidate" == */DesktopPet.app/Contents/MacOS/desktop-pet ]] || continue
    preflight_pair "$candidate"
    record="$("$SELECTED_HELPER" discover --json)" || fail 'installed native image/origin requires explicit repair'
    accept_discovery "$record"
    return
  done
  for manager in /opt/homebrew/bin/brew /usr/local/bin/brew; do
    [[ -x "$manager" ]] || continue
    physical="$(physical_file "$manager")"
    prefix="${manager%/bin/brew}"
    [[ "$physical" == "$prefix/Homebrew/bin/brew" || "$physical" == "$manager" ]] || continue
    require_owned "$physical" 0022
    prefix="$("$physical" --prefix desktop-pet 2>/dev/null)" || continue
    absolute 'Brew formula prefix' "$prefix"
    candidate="$prefix/libexec/DesktopPet.app/Contents/MacOS/desktop-pet"
    [[ -e "$candidate" || -L "$candidate" ]] || continue
    candidate="$(physical_file "$candidate")"
    preflight_pair "$candidate"
    record="$("$SELECTED_HELPER" discover --json)" || fail 'Brew installation requires explicit native repair'
    accept_discovery "$record"
    return
  done
  candidate="$home/Applications/DesktopPet.app/Contents/MacOS/desktop-pet"
  if [[ -e "$home/Applications/DesktopPet.app" || -L "$home/Applications/DesktopPet.app" ]]; then
    candidate="$(physical_file "$candidate")"
    preflight_pair "$candidate"
    record="$("$SELECTED_HELPER" discover --json)" || fail 'standalone installation requires explicit native repair'
    accept_discovery "$record"
  fi
}
