#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
APP_ROOT="$ROOT_DIR/dist/HerdrDesktopPet.app"
MANIFEST_PATH="$ROOT_DIR/herdr-plugin.toml"

fail() {
  printf 'herdr desktop pet install: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat >&2 <<'EOF'
Usage: scripts/install.sh [--prebuilt | --source]

With no option, a GitHub checkout uses its origin and manifest version to fetch
that exact release asset. A checkout without a GitHub origin is built locally.
Set HERDR_PET_REPOSITORY, HERDR_PET_VERSION, and/or HERDR_PET_RELEASE_URL to
select a release explicitly.
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

read_manifest_version() {
  [[ -f "$MANIFEST_PATH" ]] || fail "missing plugin manifest: $MANIFEST_PATH"
  awk -F'"' '/^[[:space:]]*version[[:space:]]*=/ { print $2; exit }' "$MANIFEST_PATH"
}

validate_version() {
  local version="$1"
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]] ||
    fail "release version must be a semantic version, got '$version'"
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

has_release_configuration() {
  [[ -n "${HERDR_PET_RELEASE_URL:-}" ||
    -n "${HERDR_PET_RELEASE_BASE_URL:-}" ||
    -n "${HERDR_PET_CHECKSUM_URL:-}" ||
    -n "${HERDR_PET_SHA256:-}" ||
    -n "${HERDR_PET_REPOSITORY:-}" ||
    -n "${HERDR_PET_VERSION:-}" ||
    -n "${HERDR_PET_RELEASE_TAG:-}" ]]
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
  local app="$1" archs binary
  [[ -d "$app" ]] || fail "packaged app is missing: $app"
  binary="$app/Contents/MacOS/herdr-desktop-pet"
  [[ -x "$binary" ]] ||
    fail "packaged app has no executable herdr-desktop-pet binary"
  archs="$(lipo -archs "$binary" 2>/dev/null)" ||
    fail "unable to inspect packaged app architecture"
  [[ "$archs" == "arm64" ]] || fail "packaged app is not an arm64-only binary (got $archs)"
  [[ -f "$app/Contents/Info.plist" ]] || fail "packaged app is missing Contents/Info.plist"
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

  command -v codesign >/dev/null 2>&1 || fail "packaged app validation requires codesign"
  codesign --verify --deep --strict "$app" >/dev/null 2>&1 ||
    fail "packaged app has an invalid or missing code signature"

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

  cargo_args=(build --release --manifest-path native/Cargo.toml)
  if [[ -n "${HERDR_PET_CARGO_TARGET:-}" ]]; then
    cargo_args+=(--target "$HERDR_PET_CARGO_TARGET")
  fi
  printf 'Building Herdr Desktop Pet from local Rust sources...\n'
  (cd "$ROOT_DIR" && cargo "${cargo_args[@]}")
  (cd "$ROOT_DIR" && HERDR_PET_APP_NAME=HerdrDesktopPet.app bun "$ROOT_DIR/scripts/package-native.ts")
  validate_app "$APP_ROOT"
}

prebuilt_install() {
  local repository version manifest_version release_tag asset_name release_url checksum_url use_gh=0
  local temp_dir archive checksum_file expected actual archive_list archive_details extract_dir staging entry has_app


  infer_github_repository
  repository="${HERDR_PET_REPOSITORY:-$INFERRED_REPOSITORY}"
  if [[ -z "${HERDR_PET_RELEASE_URL:-}" && -z "${HERDR_PET_RELEASE_BASE_URL:-}" ]]; then
    [[ -n "$repository" ]] ||
      fail "prebuilt installation needs HERDR_PET_REPOSITORY or a GitHub origin remote"
    validate_repository "$repository"
  elif [[ -n "$repository" ]]; then
    validate_repository "$repository"
  fi

  manifest_version="$(read_manifest_version)"
  version="${HERDR_PET_VERSION:-$manifest_version}"
  [[ -n "$version" ]] || fail "prebuilt installation needs HERDR_PET_VERSION or a manifest version"
  validate_version "$version"
  if [[ -n "$manifest_version" && "$version" != "$manifest_version" ]]; then
    fail "HERDR_PET_VERSION '$version' does not match manifest version '$manifest_version'"
  fi

  release_tag="${HERDR_PET_RELEASE_TAG:-v$version}"
  [[ "$release_tag" == "v$version" ]] ||
    fail "release tag '$release_tag' is not the pinned version tag v$version"
  [[ "$release_tag" != */* && "$release_tag" != *[[:space:]]* ]] ||
    fail "release tag contains an invalid character"

  asset_name="${HERDR_PET_ASSET_NAME:-HerdrDesktopPet-v${version}-macos-arm64.tar.gz}"
  [[ "$asset_name" != */* && "$asset_name" != .* && "$asset_name" != *[[:space:]]* ]] ||
    fail "release asset name must be a simple filename"

  if [[ -n "${HERDR_PET_RELEASE_URL:-}" ]]; then
    release_url="$HERDR_PET_RELEASE_URL"
  elif [[ -n "${HERDR_PET_RELEASE_BASE_URL:-}" ]]; then
    release_url="${HERDR_PET_RELEASE_BASE_URL%/}/$asset_name"
  else
    release_url="https://github.com/$repository/releases/download/$release_tag/$asset_name"
  fi
  checksum_url="${HERDR_PET_CHECKSUM_URL:-$release_url.sha256}"
  validate_https_url "$release_url"
  validate_https_url "$checksum_url"

  if [[ -z "${HERDR_PET_RELEASE_URL:-}" &&
    -z "${HERDR_PET_RELEASE_BASE_URL:-}" ]] &&
    command -v gh >/dev/null 2>&1; then
    use_gh=1
  fi
  if (( use_gh == 0 )) || [[ -n "${HERDR_PET_CHECKSUM_URL:-}" ]]; then
    command -v curl >/dev/null 2>&1 || fail "prebuilt installation requires curl for HTTPS release downloads"
  fi
  command -v shasum >/dev/null 2>&1 || fail "prebuilt installation requires shasum"
  command -v tar >/dev/null 2>&1 || fail "prebuilt installation requires tar"

  temp_dir="$(mktemp -d "${TMPDIR:-/tmp}/herdr-desktop-pet.XXXXXX")"
  CLEANUP_DIR="$temp_dir"
  archive="$temp_dir/$asset_name"
  checksum_file="$temp_dir/$asset_name.sha256"

  if (( use_gh == 1 )); then
    printf 'Downloading Herdr Desktop Pet %s from GitHub release %s via authenticated gh...\n' \
      "$version" "$release_tag"
    if [[ -n "${HERDR_PET_SHA256:-}" || -n "${HERDR_PET_CHECKSUM_URL:-}" ]]; then
      if ! gh release download "$release_tag" --repo "$repository" \
        --pattern "$asset_name" --dir "$temp_dir" \
        >"$temp_dir/gh-release-download.log" 2>&1; then
        fail "unable to download pinned release with GitHub CLI; authenticate gh (for example, gh auth login) or use an explicit HTTPS release URL; no curl fallback was attempted"
      fi
    else
      if ! gh release download "$release_tag" --repo "$repository" \
        --pattern "$asset_name" --pattern "$asset_name.sha256" --dir "$temp_dir" \
        >"$temp_dir/gh-release-download.log" 2>&1; then
        fail "unable to download pinned release with GitHub CLI; authenticate gh (for example, gh auth login) or use an explicit HTTPS release URL; no curl fallback was attempted"
      fi
    fi
    [[ -f "$archive" ]] || fail "GitHub release did not provide the pinned asset $asset_name"
    if [[ -z "${HERDR_PET_SHA256:-}" && -z "${HERDR_PET_CHECKSUM_URL:-}" ]]; then
      [[ -f "$checksum_file" ]] ||
        fail "GitHub release did not provide the checksum asset $asset_name.sha256"
    fi
  else
    printf 'Downloading Herdr Desktop Pet %s from %s...\n' "$version" "$release_url"
    curl --fail --location --proto '=https' --proto-redir '=https' \
      --connect-timeout 15 --max-time 300 --retry 3 --retry-delay 1 \
      --silent --show-error "$release_url" --output "$archive" ||
      if [[ -z "${HERDR_PET_RELEASE_URL:-}" && -z "${HERDR_PET_RELEASE_BASE_URL:-}" ]]; then
        fail "unable to download pinned GitHub release over HTTPS; install and authenticate gh for private repositories"
      else
        fail "unable to download pinned release asset"
      fi
  fi

  if [[ -n "${HERDR_PET_SHA256:-}" ]]; then
    printf '%s  %s\n' "$HERDR_PET_SHA256" "$asset_name" > "$checksum_file"
  elif [[ ! -f "$checksum_file" ]]; then
    curl --fail --location --proto '=https' --proto-redir '=https' \
      --connect-timeout 15 --max-time 300 --retry 3 --retry-delay 1 \
      --silent --show-error "$checksum_url" --output "$checksum_file" ||
      fail "unable to download checksum for pinned release asset"
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

  actual="$(shasum -a 256 "$archive" | awk '{ print tolower($1) }')"
  [[ "$actual" == "$expected" ]] ||
    fail "checksum mismatch for $asset_name (expected $expected, got $actual)"

  archive_list="$temp_dir/archive.list"
  tar -tzf "$archive" > "$archive_list" || fail "release asset is not a readable gzip tar archive"
  has_app=0
  while IFS= read -r entry || [[ -n "$entry" ]]; do
    entry="${entry#./}"
    [[ -n "$entry" ]] || continue
    case "$entry" in
      */../*|../*|/*) fail "release archive contains an unsafe path '$entry'" ;;
      HerdrDesktopPet.app|HerdrDesktopPet.app/*) has_app=1 ;;
      *) fail "release archive contains unexpected path '$entry'" ;;
    esac
  done < "$archive_list"
  [[ "$has_app" == 1 ]] || fail "release archive does not contain HerdrDesktopPet.app"

  archive_details="$temp_dir/archive.details"
  tar -tvzf "$archive" > "$archive_details" || fail "unable to inspect release archive entries"
  while IFS= read -r entry || [[ -n "$entry" ]]; do
    case "${entry:0:1}" in
      d|-) ;;
      *) fail "release archive contains an unsupported link or special entry" ;;
    esac
  done < "$archive_details"

  extract_dir="$temp_dir/extract"
  mkdir -p "$extract_dir"
  tar -xzf "$archive" -C "$extract_dir" || fail "unable to extract verified release asset"
  validate_app "$extract_dir/HerdrDesktopPet.app"

  mkdir -p "$ROOT_DIR/dist"
  staging="$ROOT_DIR/dist/.HerdrDesktopPet.install.$$.app"
  STAGING_DIR="$staging"
  rm -rf "$staging"
  cp -R "$extract_dir/HerdrDesktopPet.app" "$staging"
  validate_app "$staging"
  rm -rf "$APP_ROOT"
  mv "$staging" "$APP_ROOT"
  STAGING_DIR=""
  validate_app "$APP_ROOT"
  printf 'Installed Herdr Desktop Pet at %s\n' "$APP_ROOT"
}

if [[ "$mode" == "auto" ]]; then
  if has_release_configuration; then
    mode=prebuilt
  else
    infer_github_repository
    if [[ -n "$INFERRED_REPOSITORY" ]]; then
      mode=prebuilt
    else
      mode=source
    fi
  fi
fi

case "$mode" in
  prebuilt) prebuilt_install ;;
  source) source_build ;;
  *) fail "unsupported installation mode '$mode'" ;;
esac
