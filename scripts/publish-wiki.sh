#!/usr/bin/env bash
# Publish an already rendered Wiki tree; never render or mutate the main repository.
set +x
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: bash scripts/publish-wiki.sh --source-dir DIR [--repository OWNER/REPO]
       [--source-ref COMMIT] [--wiki-url LOCAL_PATH_OR_FILE_URL]

DIR must contain the generator's .wiki-managed-pages manifest and rendered pages.
The default remote is https://github.com/OWNER/REPO.wiki.git; repository defaults
 to GITHUB_REPOSITORY, or hanbong5938/herdr-desktop-pet outside Actions.
--wiki-url overrides the remote ONLY for an isolated local/file:// Git repository.
HTTPS publishing requires WIKI_PUBLISH_TOKEN; the token is never stored in Git.
USAGE
}
fail() { printf 'error: %s\n' "$*" >&2; exit 1; }
source_dir=""
repository="${GITHUB_REPOSITORY:-hanbong5938/herdr-desktop-pet}"
source_ref=""
wiki_url=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --source-dir|--repository|--source-ref|--wiki-url)
      [[ $# -ge 2 && -n "$2" ]] || fail "$1 requires a value"
      case "$1" in
        --source-dir) source_dir="$2" ;;
        --repository) repository="$2" ;;
        --source-ref) source_ref="$2" ;;
        --wiki-url) wiki_url="$2" ;;
      esac
      shift 2 ;;
    --help|-h) usage; exit 0 ;;
    *) fail "unknown option: $1" ;;
  esac
done
[[ -n "$source_dir" && -d "$source_dir" && ! -L "$source_dir" ]] || fail "--source-dir must be an existing, non-symlink rendered directory"
[[ "$repository" =~ ^[A-Za-z0-9][A-Za-z0-9_.-]*/[A-Za-z0-9][A-Za-z0-9_.-]*$ ]] || fail "invalid repository (expected OWNER/REPO)"
[[ -z "$source_ref" || "$source_ref" =~ ^[0-9a-f]{40}$ ]] || fail "--source-ref must be a full immutable Git commit SHA"
command -v bun >/dev/null || fail "Bun is required to validate generated page ownership"
command -v git >/dev/null || fail "Git is required"
source_dir="$(cd "$source_dir" && pwd -P)"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
generator_module="$script_dir/wiki-docs.ts"
[[ -f "$generator_module" ]] || fail "wiki-docs.ts must be installed beside this publisher"

local_remote=false
if [[ -n "$wiki_url" ]]; then
  case "$wiki_url" in
    file:///*) local_remote=true ;;
    /*) local_remote=true ;;
    *://*|*@*|*:*) fail "--wiki-url is restricted to a local path or file:/// URL (never a credential-bearing URL)" ;;
    *) wiki_url="$(pwd -P)/$wiki_url"; local_remote=true ;;
  esac
else
  wiki_url="https://github.com/$repository.wiki.git"
  [[ -n "${WIKI_PUBLISH_TOKEN:-}" ]] || fail "WIKI_PUBLISH_TOKEN is missing; use a dedicated credential with Wiki write access"
fi

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
# The manifest is the sole ownership boundary; reject unsafe names, symlinks,
# unknown pages and incomplete render output before any network or Git mutation.
validate_manifest() {
  bun - "$generator_module" "$1" "$2" <<'BUN'
import { lstatSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
const [modulePath, directory, mode] = process.argv.slice(2);
const { managedPageName } = await import(pathToFileURL(modulePath).href);
const manifest = join(directory, ".wiki-managed-pages");
const stat = lstatSync(manifest);
if (!stat.isFile() || stat.isSymbolicLink()) throw new Error("Wiki ownership manifest must be a regular file");
const text = readFileSync(manifest, "utf8");
if (!text.endsWith("\n")) throw new Error("Wiki ownership manifest must end with a newline");
const pages = text.slice(0, -1).split("\n");
if (!pages.length || new Set(pages).size !== pages.length) throw new Error("Wiki ownership manifest is empty or contains duplicates");
for (const page of pages) {
  if (page !== page.trim() || !managedPageName(page) || page.includes("/") || page.includes("\\")) throw new Error(`Invalid managed Wiki page: ${page}`);
  const path = join(directory, page);
  let info;
  try { info = lstatSync(path); } catch (error) { if (mode === "previous" && error.code === "ENOENT") continue; throw error; }
  if (!info.isFile() || info.isSymbolicLink() || (mode === "rendered" && info.size === 0)) throw new Error(`Managed Wiki page is not a nonempty regular file: ${page}`);
}
if (mode === "rendered") {
  const expected = new Set([...pages, ".wiki-managed-pages"]);
  for (const name of readdirSync(directory)) if (!expected.has(name)) throw new Error(`Unexpected rendered file: ${name}`);
  if (!pages.includes("Home.md")) throw new Error("Rendered Wiki must include Home.md");
}
BUN
}
validate_manifest "$source_dir" rendered

# Disable tracing and cached credential helpers. Askpass contains no credential;
# it reads the publish-step-only environment and is removed even after failure.
unset GIT_TRACE GIT_TRACE_PACKET GIT_TRACE_CURL GIT_CURL_VERBOSE GIT_TRACE2 GIT_TRACE2_EVENT GIT_TRACE2_PERF
export GIT_TERMINAL_PROMPT=0
if [[ "$local_remote" == false ]]; then
  cat > "$scratch/askpass" <<'ASKPASS'
#!/usr/bin/env bash
set +x
case "$1" in
  *Username*) printf '%s\n' 'x-access-token' ;;
  *Password*) printf '%s\n' "$WIKI_PUBLISH_TOKEN" ;;
  *) exit 1 ;;
esac
ASKPASS
  chmod 700 "$scratch/askpass"
  export GIT_ASKPASS="$scratch/askpass"
fi
wiki_dir="$scratch/wiki"
if ! git -c credential.helper= clone --no-tags -- "$wiki_url" "$wiki_dir"; then
  fail "cannot clone the Wiki: enable Wiki, initialize Home, and check the dedicated credential's repository access"
fi
branch="$(git -C "$wiki_dir" symbolic-ref --quiet --short HEAD)" || fail "Wiki remote HEAD is not a branch; initialize the Wiki Home page first"
git -C "$wiki_dir" rev-parse --verify HEAD >/dev/null 2>&1 || fail "Wiki is uninitialized; create and save an initial Home page in GitHub first"
git check-ref-format "refs/heads/$branch" >/dev/null || fail "Wiki remote HEAD has an invalid branch name"

previous_manifest="$scratch/previous-pages"
: > "$previous_manifest"
if [[ -e "$wiki_dir/.wiki-managed-pages" || -L "$wiki_dir/.wiki-managed-pages" ]]; then
  validate_manifest "$wiki_dir" previous
  cp -- "$wiki_dir/.wiki-managed-pages" "$previous_manifest"
fi
while IFS= read -r page; do
  if ! [[ -f "$source_dir/$page" ]]; then rm -f -- "$wiki_dir/$page"; fi
done < "$previous_manifest"
while IFS= read -r page; do
  [[ ! -L "$wiki_dir/$page" && ! -d "$wiki_dir/$page" ]] || fail "managed Wiki destination is a symlink or directory: $page"
  cp -- "$source_dir/$page" "$wiki_dir/$page"
done < "$source_dir/.wiki-managed-pages"
cp -- "$source_dir/.wiki-managed-pages" "$wiki_dir/.wiki-managed-pages"
git -C "$wiki_dir" add -- .wiki-managed-pages
while IFS= read -r page; do
  if [[ -e "$wiki_dir/$page" ]] || git -C "$wiki_dir" ls-files --error-unmatch -- "$page" >/dev/null 2>&1; then
    git -C "$wiki_dir" add -A -- "$page"
  fi
done < <(cat "$previous_manifest" "$source_dir/.wiki-managed-pages")
if git -C "$wiki_dir" diff --cached --quiet; then
  printf 'Wiki is already up to date; no commit or push.\n'
  exit 0
fi
git -C "$wiki_dir" -c user.name='herdr Wiki publisher' -c user.email='wiki-publisher@users.noreply.github.com' \
  -c commit.gpgsign=false commit -m "Sync reviewed Wiki documentation${source_ref:+ from $source_ref}"
# A concurrent/manual remote change fails normally; never force or overwrite it.
if ! git -C "$wiki_dir" -c credential.helper= push origin "HEAD:refs/heads/$branch"; then
  fail "Wiki push failed (including possible concurrent edits); inspect the remote and rerun the wiki-only workflow, without force-pushing"
fi
printf 'Wiki updated on its discovered %s branch.\n' "$branch"
