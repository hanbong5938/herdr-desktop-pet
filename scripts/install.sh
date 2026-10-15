#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
source "$ROOT_DIR/scripts/validate.sh"
version=0.3.11
manual_stopped=0
while (($#)); do
  case "$1" in
    --prebuilt) ;;
    --pinned-0.3.6) version=0.3.6 ;;
    --legacy-stopped) manual_stopped=1 ;;
    --help|-h)
      printf '%s\n' 'Usage: bash scripts/install.sh [--prebuilt]' 'Manual protocol-2 staging: bash scripts/install-0.3.6.sh --legacy-stopped' 'Prebuilt only; bridge requires materialized release.json. Never downloads latest or builds sources.'
      exit 0 ;;
    *) fail "unsupported option $1; source builds are not part of the legacy ingress" ;;
  esac
  shift
done
[[ "$(uname -s)" == Darwin && "$(uname -m)" == arm64 ]] || fail "macOS arm64 required"
[[ "${HERDR_PET_INSTALL_MODE:-prebuilt}" == prebuilt || "${HERDR_PET_INSTALL_MODE:-auto}" == auto ]] || fail "only prebuilt installation is permitted"
[[ "${HERDR_PET_LOCAL_BUILD:-0}" == 0 ]] || fail "source fallback is prohibited"
require_clean_checkout
repository=hanbong5938/herdr-desktop-pet
app_name=HerdrDesktopPet.app
asset="HerdrDesktopPet-v$version-macos-arm64.tar.gz"
url="https://github.com/$repository/releases/download/v$version/$asset"
checksum_url="$url.sha256"
for check in REPOSITORY VERSION RELEASE_TAG APP_NAME ASSET_NAME RELEASE_URL RELEASE_BASE_URL CHECKSUM_URL; do
  case "$check" in
    REPOSITORY) expected_value="$repository" ;;
    VERSION) expected_value="$version" ;;
    RELEASE_TAG) expected_value="v$version" ;;
    APP_NAME) expected_value="$app_name" ;;
    ASSET_NAME) expected_value="$asset" ;;
    RELEASE_URL) expected_value="$url" ;;
    RELEASE_BASE_URL) expected_value="${url%/*}" ;;
    CHECKSUM_URL) expected_value="$checksum_url" ;;
  esac
  variable="HERDR_PET_$check"
  [[ "${!variable-$expected_value}" == "$expected_value" ]] || fail "$variable must select the pinned official $version release"
done
if [[ "$version" == 0.3.6 ]]; then
  [[ "$manual_stopped" == 1 ]] || fail "first settle the beta operation with its matching retained helper and stop the old daemon; acknowledge using --legacy-stopped"
  # This is a manual manager transition, never a protocol-2 direct updater candidate.
  [[ -z "${HERDR_PET_SHA256+x}${HERDR_PET_RELEASE_URL+x}${HERDR_PET_VERSION+x}${HERDR_PET_INSTALL_MODE+x}" ]] || fail "frozen 0.3.6 route must not run inside an old-helper update environment"
  if pgrep -f '(^|/)(herdr-desktop-pet|herdr-update-coordinator)([[:space:]]|$)' >/dev/null; then
    fail "legacy app/helper still running; settle the operation and stop it before manual staging"
  fi
  expected=5d40f6ada23f7d91d967a98331b28ea767202f5d01b7cd7732b2d9e543fdaafa
else
  metadata="$ROOT_DIR/release.json"
  [[ -f "$metadata" && ! -L "$metadata" ]] || fail "release.json must be materialized from the actual packaged bridge before installing or tagging"
  require_value "$metadata" version 1
  require_value "$metadata" repository "$repository"
  require_value "$metadata" source_core_repository hanbong5938/desktop-pet
  require_value "$metadata" distribution legacy-0.3.11
  require_value "$metadata" app_version 0.3.11
  require_value "$metadata" protocol 3
  require_value "$metadata" observation_mode 1
  require_value "$metadata" integration_api 1
  require_value "$metadata" archive_url "$url"
  require_value "$metadata" checksum_url "$checksum_url"
  revision="$(json_value "$metadata" source_core_revision)"
  [[ "$revision" =~ ^[0-9a-f]{40}$ ]] || fail "invalid actual source core revision"
  migration_revision="$(json_value "$metadata" herdr_migration_revision)"
  [[ "$migration_revision" == "$retirement_revision" ]] || fail "frozen Herdr v0.4.1 retirement revision required"
  expected="$(json_value "$metadata" archive_sha256)"
  [[ "$expected" =~ ^[0-9a-f]{64}$ ]] || fail "invalid materialized archive digest"
  provenance="$ROOT_DIR/rig-native-provenance.json"
  record="$ROOT_DIR/packaging-release.json"
  for input in "$provenance" "$record"; do
    [[ -f "$input" && ! -L "$input" ]] || fail "missing authentic materialized packaging input: $input"
  done
  require_value "$metadata" provenance_sha256 "$(sha256_file "$provenance")"
  require_value "$provenance" version 1
  require_value "$provenance" core_revision "$revision"
  require_value "$provenance" distribution_identity.core_revision "$revision"
  require_value "$provenance" distribution_identity.distribution legacy-0.3.11
  require_value "$provenance" distribution_identity.app_version 0.3.11
  require_value "$provenance" distribution_identity.herdr_migration_revision "$retirement_revision"
  require_value "$record" version 1
  require_value "$record" source_core_revision "$revision"
  require_value "$record" herdr_migration_revision "$retirement_revision"
  require_value "$record" distribution legacy-0.3.11
  require_value "$record" app_version 0.3.11
  require_value "$record" protocol 3
  require_value "$record" observation_mode 1
  require_value "$record" integration_api 1
  require_value "$record" archive "$asset"
  require_value "$record" archive_sha256 "$expected"
  require_value "$record" provenance HerdrDesktopPet-v0.3.11-rig-native.json
  if [[ "${HERDR_PET_SHA256+x}" == x ]]; then
    [[ "$HERDR_PET_SHA256" =~ ^[[:xdigit:]]{64}$ ]] || fail "invalid helper-approved SHA256"
    approved="$(printf '%s' "$HERDR_PET_SHA256" | tr '[:upper:]' '[:lower:]')"
    [[ "$approved" == "$expected" ]] || fail "helper-approved bytes disagree with the immutable bridge release metadata"
    expected="$approved"
  fi
fi
APP_ROOT="$ROOT_DIR/dist/$app_name"
[[ ! -L "$ROOT_DIR/dist" && ! -L "$APP_ROOT" ]] || fail "symlink installation destination is not allowed"
if [[ "${HERDR_PET_SHA256+x}" != x ]] && pgrep -f '(^|/)(herdr-desktop-pet|herdr-update-coordinator)([[:space:]]|$)' >/dev/null; then
  fail "manual installation requires the old daemon/helper stopped; use its approved native update operation instead"
fi
mkdir -p "$ROOT_DIR/dist"
lock="$ROOT_DIR/dist/.legacy-ingress-install.lock"
mkdir "$lock" 2>/dev/null || fail "another ingress installation holds the lock; do not retry an uncertain helper operation"
temp=""
staging=""
backup=""
cleanup() {
  [[ -z "$temp" ]] || rm -rf "$temp"
  if [[ -n "$backup" && -e "$backup" && ! -e "$APP_ROOT" ]]; then
    if ! mv "$backup" "$APP_ROOT"; then
      printf 'Original bundle preserved at %s; replacement outcome uncertain.\n' "$backup" >&2
      staging=""
    fi
  fi
  [[ -z "$staging" ]] || rm -rf "$staging"
  rmdir "$lock" 2>/dev/null || true
}
trap cleanup EXIT
if [[ "$version" == 0.3.11 && -e "$APP_ROOT" ]]; then
  installed_binary="$APP_ROOT/Contents/MacOS/herdr-desktop-pet"
  [[ -f "$installed_binary" && -x "$installed_binary" && ! -L "$installed_binary" ]] || fail "invalid existing legacy image"
  codesign --verify --deep --strict "$APP_ROOT" >/dev/null 2>&1 || fail "existing legacy image signature is invalid; no replacement authorized"
  installed_capabilities="$("$installed_binary" update-capabilities 2>/dev/null)" || fail "existing legacy image cannot prove its protocol"
  [[ "$(plutil -extract protocol raw -expect integer -o - - <<< "$installed_capabilities" 2>/dev/null)" == 3 ]] || fail "bridge is not a protocol-2 direct update; stop/settle beta and manually install the frozen 0.3.6 matched pair first"
fi
temp="$(/usr/bin/mktemp -d /private/tmp/legacy-pet-ingress.XXXXXX)" || fail "cannot create private download directory"
archive="$temp/$asset"
curl --fail --location --proto '=https' --proto-redir '=https' --connect-timeout 15 --max-time 300 --silent --show-error "$url" --output "$archive" || fail "cannot download pinned legacy archive"
[[ "$(sha256_file "$archive")" == "$expected" ]] || fail "downloaded bytes do not match the approved archive SHA256"
# Respect the helper checksum selection as well as its approved SHA proof.
if [[ "${HERDR_PET_SHA256+x}" != x && "$version" == 0.3.11 ]]; then
  curl --fail --location --proto '=https' --proto-redir '=https' --connect-timeout 15 --max-time 300 --silent --show-error "$checksum_url" --output "$temp/checksum" || fail "cannot download pinned checksum"
  found=0
  while IFS= read -r line || [[ -n "$line" ]]; do
    read -r digest name extra <<< "$line"
    name="${name#\*}"
    [[ "$digest" != "$expected" || "$name" != "$asset" || -n "$extra" ]] || found=1
  done < "$temp/checksum"
  [[ "$found" == 1 ]] || fail "release checksum disagrees with verified immutable metadata"
fi
extract_archive "$archive" "$temp"
validate_app "$temp/extract/$app_name" "$version"
staging="$(mktemp -d "$ROOT_DIR/dist/.legacy-ingress-stage.XXXXXX")" || fail "cannot reserve same-filesystem staging directory"
cp -R "$temp/extract/$app_name" "$staging/$app_name" || fail "cannot stage verified app"
validate_app "$staging/$app_name" "$version"
if [[ -e "$APP_ROOT" ]]; then
  backup="$staging/previous.app"
  mv "$APP_ROOT" "$backup" || fail "cannot preserve previous installed app"
fi
if ! mv "$staging/$app_name" "$APP_ROOT"; then
  if [[ -n "$backup" ]]; then
    if ! mv "$backup" "$APP_ROOT"; then
      preserved="$staging"
      staging=""
      fail "replacement and rollback failed; original preserved at $preserved/previous.app"
    fi
  fi
  fail "replacement failed; original restored"
fi
printf 'Installed pinned legacy %s at %s\n' "$version" "$APP_ROOT"
