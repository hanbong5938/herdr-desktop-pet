#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
SCRIPT_PATH="$ROOT_DIR/scripts/$(basename -- "${BASH_SOURCE[0]}")"
APP_ROOT=""
MANIFEST_PATH="$ROOT_DIR/herdr-plugin.toml"
DEFAULT_REPOSITORY="hanbong5938/herdr-desktop-pet"

fail() {
  printf 'herdr desktop pet install: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat >&2 <<'EOF'
Usage: scripts/install.sh [--prebuilt | --source]

Modes:
  (default)   Download the release pinned to the manifest version over HTTPS,
              verify it, and install it to dist/HerdrDesktopPet.app (or the
              separate HerdrDesktopPetBeta.app for a beta manifest). An
              unpinned stable download may fall back to a local source build.
  --prebuilt  Install the pinned prebuilt release only; any failure is fatal.
  --source    Build from local sources only (cargo, bun, npm); never download.
              --local-build is an alias.

Environment:
  HERDR_PET_INSTALL_MODE      auto (default), prebuilt, or source.
  HERDR_PET_LOCAL_BUILD=1     Same as --source.
  HERDR_PET_REPOSITORY        GitHub OWNER/REPO to download from (default: the
                              checkout's GitHub origin, else
                              hanbong5938/herdr-desktop-pet).
  HERDR_PET_VERSION           Release version; must match the manifest version.
  HERDR_PET_RELEASE_TAG       v<version> for stable or beta/<version> for beta.
  HERDR_PET_APP_NAME          HerdrDesktopPet.app for stable or
                              HerdrDesktopPetBeta.app for beta.
  HERDR_PET_RELEASE_URL       Explicit HTTPS URL of the release archive.
  HERDR_PET_RELEASE_BASE_URL  HTTPS base URL; the pinned asset name is appended.
  HERDR_PET_CHECKSUM_URL      HTTPS URL of the SHA-256 checksum file
                              (default: <release URL>.sha256).
  HERDR_PET_SHA256            Expected archive SHA-256; skips the checksum download.
  HERDR_PET_ASSET_NAME        Exact release archive filename
                              (HerdrDesktopPet-v<version>-macos-arm64.tar.gz).
EOF
}

mode="${HERDR_PET_INSTALL_MODE:-auto}"
case "$mode" in
  auto|prebuilt|source) ;;
  *) fail "HERDR_PET_INSTALL_MODE must be auto, prebuilt, or source (got '$mode')" ;;
esac

while (($# > 0)); do
  case "$1" in
    --prebuilt) mode=prebuilt ;;
    --source|--local-build) mode=source ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      usage
      fail "unknown option '$1'"
      ;;
  esac
  shift
done

if [[ "${HERDR_PET_LOCAL_BUILD:-}" == "1" || "${HERDR_PET_LOCAL_BUILD:-}" == "true" ]]; then
  mode=source
fi

[[ "$(uname -s)" == "Darwin" ]] || fail "native installation requires macOS"
[[ "$(uname -m)" == "arm64" ]] || fail "native installation requires macOS arm64 (found $(uname -m))"

INFERRED_REPOSITORY=""
CLEANUP_DIR=""
STAGING_DIR=""

cleanup() {
  if [[ -n "$CLEANUP_DIR" ]]; then
    rm -rf "$CLEANUP_DIR"
  fi
  if [[ -n "$STAGING_DIR" ]]; then
    rm -rf "$STAGING_DIR"
  fi
}
trap cleanup EXIT

infer_github_repository() {
  local origin path
  INFERRED_REPOSITORY=""
  command -v git >/dev/null 2>&1 || return 0
  origin="$(git -C "$ROOT_DIR" remote get-url origin 2>/dev/null || true)"
  [[ -n "$origin" ]] || return 0

  case "$origin" in
    https://github.com/*|http://github.com/*)
      path="${origin#*github.com/}"
      ;;
    git@github.com:*)
      path="${origin#git@github.com:}"
      ;;
    ssh://git@github.com/*)
      path="${origin#ssh://git@github.com/}"
      ;;
    *)
      return 0
      ;;
  esac

  path="${path%/}"
  path="${path%.git}"
  if [[ "$path" =~ ^([A-Za-z0-9][A-Za-z0-9_.-]*)/([A-Za-z0-9][A-Za-z0-9_.-]*)$ ]]; then
    INFERRED_REPOSITORY="${BASH_REMATCH[1]}/${BASH_REMATCH[2]}"
  fi
}

validate_repository() {
  local repository="$1"
  [[ "$repository" =~ ^[A-Za-z0-9][A-Za-z0-9_.-]*/[A-Za-z0-9][A-Za-z0-9_.-]*$ ]] ||
    fail "GitHub repository must be OWNER/REPO, got '$repository'"
}

validate_https_url() {
  local url="$1"
  [[ "$url" == https://* ]] || fail "release URL must use https://"
  [[ "$url" != *[[:space:]]* ]] || fail "release URL contains whitespace"
}

read_manifest_version() {
  [[ -f "$MANIFEST_PATH" ]] || fail "missing plugin manifest: $MANIFEST_PATH"
  awk -F'"' '/^[[:space:]]*version[[:space:]]*=/ { print $2; exit }' "$MANIFEST_PATH"
}

select_bundle() {
  local manifest_version version
  manifest_version="$(read_manifest_version)"
  [[ -n "$manifest_version" ]] || fail "plugin manifest has no version"
  version="${HERDR_PET_VERSION-$manifest_version}"
  [[ "$version" == "$manifest_version" ]] ||
    fail "HERDR_PET_VERSION '$version' does not match manifest version '$manifest_version'"
  if [[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
    APP_NAME="HerdrDesktopPet.app"
    EXPECTED_TAG="v$version"
    BUNDLE_ID="dev.herdr.desktop-pet"
  elif [[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-beta\.(0|[1-9][0-9]*)$ ]]; then
    APP_NAME="HerdrDesktopPetBeta.app"
    EXPECTED_TAG="beta/$version"
    BUNDLE_ID="dev.herdr.desktop-pet.beta"
  else
    fail "manifest version must be a stable or beta.N semantic version, got '$version'"
  fi
  [[ "${HERDR_PET_APP_NAME-$APP_NAME}" == "$APP_NAME" ]] ||
    fail "HERDR_PET_APP_NAME does not match manifest version '$version'"
  APP_ROOT="$ROOT_DIR/dist/$APP_NAME"
  SELECTED_VERSION="$version"
}

official_release_url() {
  local repository="$1" tag="$2" asset="$3"
  # GitHub encodes the slash inside prerelease tags in asset download URLs.
  printf 'https://github.com/%s/releases/download/%s/%s' "$repository" "${tag//\//%2F}" "$asset"
}

validate_native_pack() {
  local binary="$1" path="$2" label="$3" expected_backend="$4" validation_root validation_output

  validation_root="$(mktemp -d "${TMPDIR:-/tmp}/herdr-pack-validation.XXXXXX")" ||
    fail "unable to create temporary state for native pack validation"
  if ! validation_output="$(
    "$binary" pack validate --path "$path" \
      --config-dir "$validation_root/config" --state-dir "$validation_root/state" 2>&1
  )"; then
    rm -rf "$validation_root"
    [[ -n "$validation_output" ]] && printf '%s\n' "$validation_output" >&2
    fail "native pack validation failed for $label"
  fi
  if [[ -n "$expected_backend" && "$validation_output" != *"\"backend\":\"$expected_backend\""* ]]; then
    rm -rf "$validation_root"
    fail "native pack validation selected an unexpected renderer for $label"
  fi
  rm -rf "$validation_root"
}


validate_app() {
  local app="$1" allow_legacy="${2:-0}" archs binary helper main_capabilities helper_capabilities main_protocol="" helper_protocol main_observation="" helper_observation="" commands dependencies library line bundle_id
  [[ -d "$app" ]] || fail "packaged app is missing: $app"
  binary="$app/Contents/MacOS/herdr-desktop-pet"
  [[ -x "$binary" ]] ||
    fail "packaged app has no executable herdr-desktop-pet binary"
  archs="$(lipo -archs "$binary" 2>/dev/null)" ||
    fail "unable to inspect packaged app architecture"
  [[ "$archs" == "arm64" ]] || fail "packaged app is not an arm64-only binary (got $archs)"
  command -v codesign >/dev/null 2>&1 || fail "packaged app validation requires codesign"
  codesign --verify --deep --strict "$app" >/dev/null 2>&1 ||
    fail "packaged app has an invalid or missing code signature"
  [[ "$("$binary" --version 2>/dev/null)" == "herdr-desktop-pet $SELECTED_VERSION" ]] ||
    fail "packaged app binary does not match expected version $SELECTED_VERSION"
  [[ -f "$app/Contents/Info.plist" ]] || fail "packaged app is missing Contents/Info.plist"
  [[ "$(plutil -extract CFBundleShortVersionString raw -o - "$app/Contents/Info.plist" 2>/dev/null)" == "$SELECTED_VERSION" ]] ||
    fail "packaged app bundle does not match expected version $SELECTED_VERSION"
  bundle_id="$(plutil -extract CFBundleIdentifier raw -o - "$app/Contents/Info.plist" 2>/dev/null)" ||
    fail "packaged app has no bundle identifier"
  if main_capabilities="$("$binary" update-capabilities 2>/dev/null)"; then
    main_protocol="$(plutil -extract protocol raw -expect integer -o - - <<< "$main_capabilities" 2>/dev/null)" ||
      fail "packaged app does not advertise a numeric updater protocol"
    [[ "$main_protocol" == 2 || "$main_protocol" == 3 ]] ||
      fail "packaged app does not advertise supported updater protocol 2 or 3"
    helper="$app/Contents/MacOS/herdr-update-coordinator"
    [[ -x "$helper" && -f "$helper" && ! -L "$helper" ]] ||
      fail "updater-capable app is missing its standalone executable update coordinator"
    archs="$(lipo -archs "$helper" 2>/dev/null)" ||
      fail "unable to inspect packaged update coordinator architecture"
    [[ "$archs" == "arm64" ]] ||
      fail "packaged update coordinator is not arm64-only (got $archs)"
    codesign --verify --strict "$helper" >/dev/null 2>&1 ||
      fail "packaged update coordinator has an invalid or missing code signature"
    helper_capabilities="$("$helper" update-capabilities 2>/dev/null)" ||
      fail "packaged update coordinator cannot advertise updater capabilities"
    helper_protocol="$(plutil -extract protocol raw -expect integer -o - - <<< "$helper_capabilities" 2>/dev/null)" ||
      fail "packaged update coordinator does not advertise a numeric updater protocol"
    [[ "$helper_protocol" == "$main_protocol" ]] ||
      fail "packaged update coordinator protocol does not match packaged app"
    if plutil -type observation_mode - <<< "$main_capabilities" >/dev/null 2>&1; then
      main_observation="$(plutil -extract observation_mode raw -expect integer -o - - <<< "$main_capabilities" 2>/dev/null)" ||
        fail "packaged app does not advertise numeric observation-mode support"
      [[ "$main_observation" == 1 ]] ||
        fail "packaged app advertises unsupported observation-mode support"
    fi
    if plutil -type observation_mode - <<< "$helper_capabilities" >/dev/null 2>&1; then
      helper_observation="$(plutil -extract observation_mode raw -expect integer -o - - <<< "$helper_capabilities" 2>/dev/null)" ||
        fail "packaged update coordinator does not advertise numeric observation-mode support"
      [[ "$helper_observation" == 1 ]] ||
        fail "packaged update coordinator advertises unsupported observation-mode support"
    fi
    [[ "$helper_observation" == "$main_observation" ]] ||
      fail "packaged update coordinator observation-mode support does not match packaged app"
    commands="$(otool -l "$helper")" ||
      fail "unable to inspect packaged update coordinator loader paths"
    [[ "$commands" != *"cmd LC_RPATH"* ]] ||
      fail "packaged update coordinator contains loader search paths"
    dependencies="$(otool -L "$helper")" ||
      fail "unable to inspect packaged update coordinator dependencies"
    dependencies="${dependencies#*$'\n'}"
    [[ -n "$dependencies" ]] || fail "packaged update coordinator has no system dependencies"
    while IFS= read -r line; do
      [[ -n "$line" ]] || continue
      read -r library _ <<< "$line"
      case "$library" in
        /usr/lib/*|/System/Library/*) ;;
        *) fail "packaged update coordinator has a non-system dependency: $library" ;;
      esac
    done <<< "$dependencies"
  elif [[ "$allow_legacy" != 1 || "$SELECTED_VERSION" != "0.2.0" || "$APP_NAME" != "HerdrDesktopPet.app" ]]; then
    fail "packaged app does not support updater protocol 2 or 3"
  fi
  if [[ "$main_protocol" == 2 && "$APP_NAME" == "HerdrDesktopPetBeta.app" ]]; then
    # Published protocol-2 beta bundles predate the separate beta identifier.
    [[ "$bundle_id" == "dev.herdr.desktop-pet" || "$bundle_id" == "$BUNDLE_ID" ]] ||
      fail "packaged beta app has an unexpected bundle identifier"
  else
    [[ "$bundle_id" == "$BUNDLE_ID" ]] ||
      fail "packaged app has an unexpected bundle identifier"
  fi
  [[ -f "$app/Contents/Resources/default/manifest.json" ]] ||
    fail "packaged app is missing the default asset manifest"
  [[ "$(plutil -extract id raw -o - "$app/Contents/Resources/default/manifest.json" 2>/dev/null)" == "rubelia-at12-multi-pose" ]] ||
    fail "packaged default character is not Rubelia"
  [[ -f "$app/Contents/Resources/LICENSE.txt" ]] ||
    fail "packaged app is missing the license notice"
  [[ -f "$app/Contents/Frameworks/libherdr_rig.dylib" ]] ||
    fail "packaged app is missing the bundled native rig library"
  [[ -x "$app/Contents/MacOS/rig-decode-worker" ]] ||
    fail "packaged app is missing the executable native rig decode worker"
  [[ -f "$app/Contents/Resources/rig/decoder.js" ]] ||
    fail "packaged app is missing the bundled native rig decoder"

  validate_native_pack "$binary" "$app/Contents/Resources/default" "the default Rubelia rig pack" "rig"
}

source_build() {
  local -a cargo_args
  command -v cargo >/dev/null 2>&1 ||
    fail "source build requires cargo; use a pinned prebuilt release with --prebuilt"
  command -v bun >/dev/null 2>&1 ||
    fail "source build requires bun to package the app; use a pinned prebuilt release with --prebuilt"
  command -v npm >/dev/null 2>&1 ||
    fail "source build requires npm to install the pinned web/rig dependencies"
  [[ -f "$ROOT_DIR/bun.lock" ]] ||
    fail "source build is missing the pinned root Bun lockfile: $ROOT_DIR/bun.lock"
  [[ -f "$ROOT_DIR/web/rig/package-lock.json" ]] ||
    fail "source build is missing the pinned web/rig npm lockfile: $ROOT_DIR/web/rig/package-lock.json"

  printf 'Installing pinned root JavaScript dependencies...\n'
  (cd "$ROOT_DIR" && bun install --frozen-lockfile)
  printf 'Installing pinned web/rig authoring and build dependencies...\n'
  (cd "$ROOT_DIR/web/rig" && npm ci --no-audit --no-fund)

  cargo_args=(build --release --locked --target-dir native/target)
  if [[ -n "${HERDR_PET_CARGO_TARGET:-}" ]]; then
    cargo_args+=(--target "$HERDR_PET_CARGO_TARGET")
  fi
  printf 'Building Herdr Desktop Pet from local Rust sources...\n'
  (cd "$ROOT_DIR" && cargo "${cargo_args[@]}" --manifest-path native/Cargo.toml)
  printf 'Building standalone update coordinator from local Rust sources...\n'
  (cd "$ROOT_DIR" && cargo "${cargo_args[@]}" --manifest-path native/update-coordinator/Cargo.toml)
  (cd "$ROOT_DIR" && HERDR_PET_APP_NAME="$APP_NAME" bun "$ROOT_DIR/scripts/package-native.ts")
  validate_app "$APP_ROOT"
}

prebuilt_install() {
  local repository version release_tag asset_name release_url checksum_url
  local temp_dir archive checksum_file expected expected_candidate name actual
  local archive_list archive_details extract_dir staging entry has_app allow_legacy

  infer_github_repository
  repository="${HERDR_PET_REPOSITORY:-${INFERRED_REPOSITORY:-$DEFAULT_REPOSITORY}}"
  validate_repository "$repository"

  version="$SELECTED_VERSION"
  release_tag="${HERDR_PET_RELEASE_TAG-$EXPECTED_TAG}"
  [[ "$release_tag" == "$EXPECTED_TAG" ]] ||
    fail "release tag '$release_tag' is not the pinned version tag $EXPECTED_TAG"
  asset_name="HerdrDesktopPet-v${version}-macos-arm64.tar.gz"
  [[ "${HERDR_PET_ASSET_NAME-$asset_name}" == "$asset_name" ]] ||
    fail "release asset name must match pinned version $version"
  if [[ -n "${HERDR_PET_RELEASE_URL:-}" ]]; then
    release_url="$HERDR_PET_RELEASE_URL"
  elif [[ -n "${HERDR_PET_RELEASE_BASE_URL:-}" ]]; then
    release_url="${HERDR_PET_RELEASE_BASE_URL%/}/$asset_name"
  else
    release_url="$(official_release_url "$repository" "$release_tag" "$asset_name")"
  fi
  checksum_url="${HERDR_PET_CHECKSUM_URL:-$release_url.sha256}"
  validate_https_url "$release_url"
  validate_https_url "$checksum_url"
  if [[ "${HERDR_PET_SHA256+x}" == x ]]; then
    [[ "$HERDR_PET_SHA256" =~ ^[[:xdigit:]]{64}$ ]] ||
      fail "HERDR_PET_SHA256 must be a 64-character SHA-256 hex digest"
  fi

  command -v curl >/dev/null 2>&1 || fail "prebuilt installation requires curl for HTTPS release downloads"
  command -v shasum >/dev/null 2>&1 || fail "prebuilt installation requires shasum"
  command -v tar >/dev/null 2>&1 || fail "prebuilt installation requires tar"

  temp_dir="$(mktemp -d "${TMPDIR:-/tmp}/herdr-desktop-pet.XXXXXX")" ||
    fail "unable to create a temporary download directory"
  CLEANUP_DIR="$temp_dir"
  archive="$temp_dir/$asset_name"
  checksum_file="$temp_dir/$asset_name.sha256"

  printf 'Downloading Herdr Desktop Pet %s from %s...\n' "$version" "$release_url"
  curl --fail --location --proto '=https' --proto-redir '=https' \
    --connect-timeout 15 --max-time 300 --retry 3 --retry-delay 1 \
    --silent --show-error "$release_url" --output "$archive" ||
    fail "unable to download pinned release asset from $release_url"

  if [[ -n "${HERDR_PET_SHA256:-}" ]]; then
    printf '%s  %s\n' "$HERDR_PET_SHA256" "$asset_name" > "$checksum_file" ||
      fail "unable to record the expected checksum"
  else
    curl --fail --location --proto '=https' --proto-redir '=https' \
      --connect-timeout 15 --max-time 300 --retry 3 --retry-delay 1 \
      --silent --show-error "$checksum_url" --output "$checksum_file" ||
      fail "unable to download checksum for pinned release asset from $checksum_url"
  fi

  expected=""
  while IFS= read -r entry || [[ -n "$entry" ]]; do
    # Accept both `sha  file` and `sha *file` checksum formats.
    read -r expected_candidate name _ <<<"$entry"
    name="${name#\*}"
    expected_candidate="$(printf '%s' "$expected_candidate" | tr '[:upper:]' '[:lower:]')"
    if [[ "$expected_candidate" =~ ^[[:xdigit:]]{64}$ ]] &&
      [[ -z "$name" || "$name" == "$asset_name" ]]; then
      expected="$expected_candidate"
      break
    fi
  done < "$checksum_file"
  [[ -n "$expected" ]] || fail "checksum file does not contain a SHA-256 for $asset_name"

  actual="$(shasum -a 256 "$archive" | awk '{ print tolower($1) }')" ||
    fail "unable to compute the SHA-256 of $asset_name"
  [[ "$actual" == "$expected" ]] ||
    fail "checksum mismatch for $asset_name (expected $expected, got $actual)"
  # Only the already-published v0.2.0 archive predates the update protocol.
  # A new source build (even at 0.2.0) must include the coordinator.
  allow_legacy=0
  if [[ "$version" == 0.2.0 &&
    "$actual" == 871096fc0993ac68d6afa9cce54198c033cae07db28202156b57f4fd23032cdf ]]; then
    allow_legacy=1
  fi

  archive_list="$temp_dir/archive.list"
  tar -tzf "$archive" > "$archive_list" || fail "release asset is not a readable gzip tar archive"
  has_app=0
  while IFS= read -r entry || [[ -n "$entry" ]]; do
    entry="${entry#./}"
    [[ -n "$entry" ]] || continue
    case "$entry" in
      */../*|../*|/*) fail "release archive contains an unsafe path '$entry'" ;;
      "$APP_NAME"|"$APP_NAME"/*) has_app=1 ;;
      *) fail "release archive contains unexpected path '$entry'" ;;
    esac
  done < "$archive_list"
  [[ "$has_app" == 1 ]] || fail "release archive does not contain $APP_NAME"

  archive_details="$temp_dir/archive.details"
  tar -tvzf "$archive" > "$archive_details" || fail "unable to inspect release archive entries"
  while IFS= read -r entry || [[ -n "$entry" ]]; do
    case "${entry:0:1}" in
      d|-) ;;
      *) fail "release archive contains an unsupported link or special entry" ;;
    esac
  done < "$archive_details"

  extract_dir="$temp_dir/extract"
  mkdir -p "$extract_dir" || fail "unable to create the extraction directory"
  tar -xzf "$archive" -C "$extract_dir" || fail "unable to extract verified release asset"
  validate_app "$extract_dir/$APP_NAME" "$allow_legacy"

  mkdir -p "$ROOT_DIR/dist" || fail "unable to create $ROOT_DIR/dist"
  staging="$ROOT_DIR/dist/.$APP_NAME.install.$$.app"
  STAGING_DIR="$staging"
  rm -rf "$staging" || fail "unable to clear the staging directory $staging"
  cp -R "$extract_dir/$APP_NAME" "$staging" ||
    fail "unable to stage the verified app bundle"
  validate_app "$staging" "$allow_legacy"
  rm -rf "$APP_ROOT" || fail "unable to remove the previous app at $APP_ROOT"
  mv "$staging" "$APP_ROOT" || fail "unable to move the verified app into $APP_ROOT"
  STAGING_DIR=""
  validate_app "$APP_ROOT" "$allow_legacy"
  printf 'Installed Herdr Desktop Pet at %s\n' "$APP_ROOT"
}

select_bundle

case "$mode" in
  auto)
    # An explicit release selection (including a beta manifest) must never
    # turn into an unrelated local source build on a failed download.
    if [[ "$APP_NAME" == "HerdrDesktopPetBeta.app" ||
      "${HERDR_PET_REPOSITORY+x}" == x || "${HERDR_PET_VERSION+x}" == x ||
      "${HERDR_PET_RELEASE_TAG+x}" == x || "${HERDR_PET_APP_NAME+x}" == x ||
      "${HERDR_PET_ASSET_NAME+x}" == x || "${HERDR_PET_RELEASE_URL+x}" == x ||
      "${HERDR_PET_RELEASE_BASE_URL+x}" == x || "${HERDR_PET_CHECKSUM_URL+x}" == x ||
      "${HERDR_PET_SHA256+x}" == x ]]; then
      prebuilt_install
    elif ! bash "$SCRIPT_PATH" --prebuilt; then
      printf 'herdr desktop pet install: prebuilt release unavailable or invalid; building from source instead.\n' >&2
      source_build
    fi
    ;;
  prebuilt) prebuilt_install ;;
  source) source_build ;;
  *) fail "unsupported installation mode '$mode'" ;;
esac
