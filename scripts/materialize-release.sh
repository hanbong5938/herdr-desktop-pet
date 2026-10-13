#!/usr/bin/env bash
set -euo pipefail
umask 077
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
source "$ROOT_DIR/scripts/consumer.sh"
[[ "$#" == 4 ]] || fail 'Usage: bash scripts/materialize-release.sh ANONYMOUS_INSTALLER ANONYMOUS_INSTALLER_CHECKSUM CORE_RELEASE_API_RECORD NEW_ABSOLUTE_OUTPUT_DIRECTORY'
installer="$1" checksum="$2" reviewed_record="$3" output="$4"
absolute output "$output"
[[ ! -e "$output" && ! -L "$output" ]] || fail 'output already exists; candidate and published trees are never overwritten'
for input in "$installer" "$checksum" "$reviewed_record"; do
  [[ -f "$input" && ! -L "$input" && "$(/usr/bin/stat -f '%l' "$input")" == 1 ]] || fail "missing actual regular release input: $input"
done
[[ "${installer##*/}" == desktop-pet-install-v0.4.0.sh && "${checksum##*/}" == desktop-pet-install-v0.4.0.sh.sha256 ]] || fail 'inputs must be the actual v0.4.0 core installer/checksum assets'
parent="${output%/*}"; [[ -n "$parent" ]] || parent=/
[[ -d "$parent" && ! -L "$parent" && "$(cd -- "$parent" && pwd -P)" == "$parent" ]] || fail 'output parent must be a physical existing directory'
stage="$(/usr/bin/mktemp -d "$parent/.herdr-consumer-materialize.XXXXXX")" || fail 'cannot reserve materialization directory'
trap '[[ -z "$stage" ]] || rm -rf -- "$stage"' EXIT
mkdir "$stage/downloads"
api='https://api.github.com/repos/hanbong5938/desktop-pet'
base='https://github.com/hanbong5938/desktop-pet/releases/download/v0.4.0'
fetch() {
  # Ignore curl config/netrc/proxy credentials and explicitly omit authorization.
  /usr/bin/curl -q --fail --silent --show-error --location --proto '=https' --proto-redir '=https' --connect-timeout 15 --max-time 120 --header 'Authorization:' --header 'Accept: application/vnd.github+json' "$1" -o "$2" || fail "anonymous release download failed: $1"
}
fresh="$stage/downloads/release.json"
fetch "$api/releases/tags/v0.4.0" "$fresh"
for record in "$reviewed_record" "$fresh"; do
  require_value "$record" tag_name v0.4.0
  require_value "$record" html_url https://github.com/hanbong5938/desktop-pet/releases/tag/v0.4.0
  require_value "$record" url "$api/releases/$(json_value "$record" id)"
  require_value "$record" draft false
  require_value "$record" prerelease false
  [[ "$(json_value "$record" id)" =~ ^[1-9][0-9]*$ ]] || fail 'missing actual published release ID'
done
release_id="$(json_value "$fresh" id)"
require_value "$reviewed_record" id "$release_id"
# Match every release asset to the reviewed fixed-authority record. Exact complete
# six-asset neutral publication is required; integration versions never select it.
asset_index() {
  local record="$1" wanted="$2" index=0 name found=''
  while name="$(/usr/bin/plutil -extract "assets.$index.name" raw -o - "$record" 2>/dev/null)"; do
    if [[ "$name" == "$wanted" ]]; then
      [[ -z "$found" ]] || fail "duplicate release asset: $wanted"
      found="$index"
    fi
    ((index += 1)); ((index <= 100)) || fail 'unbounded release asset record'
  done
  [[ -n "$found" ]] || fail "missing real release asset: $wanted"
  printf '%s' "$found"
}
for name in DesktopPet-v0.4.0-macos-arm64.tar.gz DesktopPet-v0.4.0-macos-arm64.tar.gz.sha256 SHA256SUMS DesktopPet-v0.4.0-rig-native.json desktop-pet-install-v0.4.0.sh desktop-pet-install-v0.4.0.sh.sha256; do
  current="$(asset_index "$fresh" "$name")"
  reviewed="$(asset_index "$reviewed_record" "$name")"
  for record_index in "$fresh:$current" "$reviewed_record:$reviewed"; do
    record="${record_index%:*}" index="${record_index##*:}"
    require_value "$record" "assets.$index.browser_download_url" "$base/$name"
    require_value "$record" "assets.$index.state" uploaded
    [[ "$(json_value "$record" "assets.$index.size")" =~ ^[1-9][0-9]*$ ]] || fail "empty asset: $name"
    [[ "$(json_value "$record" "assets.$index.id")" =~ ^[1-9][0-9]*$ ]] || fail "invalid asset ID: $name"
  done
  for field in id size; do
    require_value "$reviewed_record" "assets.$reviewed.$field" "$(json_value "$fresh" "assets.$current.$field")"
  done
done
for record in "$fresh" "$reviewed_record"; do
  if /usr/bin/plutil -extract assets.6 json -o - "$record" >/dev/null 2>&1; then
    fail 'neutral core release must have exactly the approved six-asset set'
  fi
done
# A supplied record is not self-authenticating: anonymously retrieve actual bytes
# again, and compare both local artifact inputs with the fixed release authority.
fetch "$base/desktop-pet-install-v0.4.0.sh" "$stage/downloads/desktop-pet-install-v0.4.0.sh"
fetch "$base/desktop-pet-install-v0.4.0.sh.sha256" "$stage/downloads/desktop-pet-install-v0.4.0.sh.sha256"
fetch "$base/SHA256SUMS" "$stage/downloads/SHA256SUMS"
digest="$(sha256_file "$installer")"
checksum_digest="$(sha256_file "$checksum")"
[[ "$digest" == "$(sha256_file "$stage/downloads/desktop-pet-install-v0.4.0.sh")" && "$checksum_digest" == "$(sha256_file "$stage/downloads/desktop-pet-install-v0.4.0.sh.sha256")" ]] || fail 'supplied bytes are not the actual anonymously downloaded fixed-authority release'
lines=0
while IFS= read -r line || [[ -n "$line" ]]; do
  ((lines += 1))
  [[ "$line" == "$digest  desktop-pet-install-v0.4.0.sh" ]] || fail 'installer checksum content mismatch'
done < "$checksum"
[[ "$lines" == 1 ]] || fail 'installer checksum must contain exactly one authentic asset entry'
found=0
while IFS= read -r line || [[ -n "$line" ]]; do
  if [[ "$line" == *' desktop-pet-install-v0.4.0.sh' ]]; then
    ((found += 1))
    [[ "$line" == "$digest  desktop-pet-install-v0.4.0.sh" ]] || fail 'release SHA256SUMS disagrees with installer'
  fi
done < "$stage/downloads/SHA256SUMS"
[[ "$found" == 1 ]] || fail 'release SHA256SUMS must authenticate exactly one installer'
for name in desktop-pet-install-v0.4.0.sh desktop-pet-install-v0.4.0.sh.sha256; do
  current="$(asset_index "$fresh" "$name")"
  actual_size="$(/usr/bin/stat -f '%z' "$stage/downloads/$name")"
  require_value "$fresh" "assets.$current.size" "$actual_size"
  if asset_digest="$(/usr/bin/plutil -extract "assets.$current.digest" raw -o - "$fresh" 2>/dev/null)" && [[ "$asset_digest" != null ]]; then
    [[ "$asset_digest" == "sha256:$(sha256_file "$stage/downloads/$name")" ]] || fail 'GitHub release asset digest mismatch'
  fi
done
# Resolve the real published tag, including annotated tags, never plugin version,
# Herdr origin, local HEAD, or a mutable branch as the packaged core revision.
fetch "$api/git/ref/tags/v0.4.0" "$stage/downloads/tag.json"
require_value "$stage/downloads/tag.json" ref refs/tags/v0.4.0
object="$stage/downloads/tag.json" hops=0
while [[ "$(json_value "$object" object.type)" == tag ]]; do
  revision="$(json_value "$object" object.sha)"
  [[ "$revision" =~ ^[0-9a-f]{40}$ ]] || fail 'invalid annotated tag identity'
  ((hops += 1)); ((hops <= 4)) || fail 'annotated tag chain too deep'
  next="$stage/downloads/tag-$hops.json"
  fetch "$api/git/tags/$revision" "$next"
  require_value "$next" sha "$revision"
  object="$next"
done
require_value "$object" object.type commit
revision="$(json_value "$object" object.sha)"
[[ "$revision" =~ ^[0-9a-f]{40}$ && "$revision" != "$(printf '%040d' 0)" ]] || fail 'actual core commit required'
fetch "$api/git/commits/$revision" "$stage/downloads/commit.json"
require_value "$stage/downloads/commit.json" sha "$revision"
mkdir "$stage/scripts"
cp "$ROOT_DIR/herdr-plugin.toml" "$stage/herdr-plugin.toml"
for script in consumer.sh install.sh run-native.sh materialize-release.sh; do
  cp "$ROOT_DIR/scripts/$script" "$stage/scripts/$script"
  chmod 755 "$stage/scripts/$script"
done
record_digest="$(sha256_file "$fresh")"
printf '{\n  "version": 1,\n  "repository": "hanbong5938/desktop-pet",\n  "tag": "v0.4.0",\n  "app_version": "0.4.0",\n  "core_revision": "%s",\n  "release_id": %s,\n  "protocol": 4,\n  "observation_mode": 2,\n  "integration_api": 1,\n  "installer_url": "%s/desktop-pet-install-v0.4.0.sh",\n  "checksum_url": "%s/desktop-pet-install-v0.4.0.sh.sha256",\n  "installer_sha256": "%s",\n  "checksum_sha256": "%s",\n  "release_record_sha256": "%s"\n}\n' "$revision" "$release_id" "$base" "$base" "$digest" "$checksum_digest" "$record_digest" > "$stage/core-release.json"
chmod 600 "$stage/core-release.json"
# Retain anonymous release evidence for review, but never install it as app bytes.
mv "$stage/downloads" "$stage/release-evidence"
ROOT_DIR="$stage" validate_release_metadata
mv "$stage" "$output"
stage=''
printf 'Materialized integration-only candidate at %s from actual core release %s. Review metadata before cutover; nothing was published.\n' "$output" "$release_id"
