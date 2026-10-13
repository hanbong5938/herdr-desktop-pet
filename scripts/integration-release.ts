import { createHash } from "node:crypto";
import { existsSync, lstatSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { isDeepStrictEqual } from "node:util";

const repository = "hanbong5938/herdr-desktop-pet";
const root = path.resolve(import.meta.dir, "..");
const actions = ["settings", "start", "ensure", "stop", "status", "toggle", "show", "hide", "passthrough", "alpha_passthrough", "show_bubble", "hide_bubble", "bubble_above", "bubble_below", "bubble_left", "bubble_right", "bubble_auto", "reset", "bigger", "smaller", "restart"];
function requireCondition(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}
function object(value: unknown, label: string): Record<string, unknown> {
  requireCondition(typeof value === "object" && value !== null && !Array.isArray(value), `${label} must be an object`);
  // The boundary check proves only an object; every consumed field stays unknown.
  return value as Record<string, unknown>;
}
function records(value: unknown, label: string): Record<string, unknown>[] {
  requireCondition(Array.isArray(value), `${label} must be an array`);
  return value.map((entry: unknown) => object(entry, label));
}
function file(relative: string): string {
  const absolute = path.join(root, relative);
  requireCondition(existsSync(absolute) && lstatSync(absolute).isFile() && !lstatSync(absolute).isSymbolicLink(), `Missing regular integration file: ${relative}`);
  return readFileSync(absolute, "utf8");
}
function run(command: string, args: string[], cwd = root): string {
  const result = spawnSync(command, args, { cwd, encoding: "utf8", env: process.env });
  requireCondition(!result.error && result.status === 0, `${command} ${args.join(" ")} failed: ${result.error?.message ?? result.stderr.trim()}`);
  return result.stdout.trim();
}
function source(): string {
  const manifest = object(Bun.TOML.parse(file("herdr-plugin.toml")), "Herdr manifest");
  requireCondition(manifest.id === "desktop-pet" && manifest.min_herdr_version === "0.9.3", "Herdr plugin identity/host prerequisite changed");
  requireCondition(typeof manifest.version === "string" && /^0\.(?:[4-9]|[1-9][0-9]+)\.[0-9]+$/.test(manifest.version), "Integration version must be a stable post-cutover 0.4+ version, never a bridge/beta tag");
  requireCondition(isDeepStrictEqual(records(manifest.build, "build hooks").map((entry) => entry.command), [["bash", "scripts/install.sh"]]), "Build hook must be the core consumer bootstrap");
  requireCondition(isDeepStrictEqual(records(manifest.startup, "startup hooks").map((entry) => entry.command), [["bash", "scripts/run-native.sh", "ensure"]]), "Startup hook must preserve ensure");
  const events = records(manifest.events, "event hooks");
  requireCondition(events.length === 1 && events[0]?.on === "client.attached" && isDeepStrictEqual(events[0]?.command, ["bash", "scripts/run-native.sh", "ensure"]), "Herdr attach hook changed");
  const configuredActions = records(manifest.actions, "actions");
  requireCondition(isDeepStrictEqual(configuredActions.map((entry) => entry.id), actions), "Retained Herdr action IDs/order changed");
  for (const action of configuredActions) requireCondition(isDeepStrictEqual(action.command, ["bash", "scripts/run-native.sh", action.id]), `Action ${action.id} is not a consumer call`);
  for (const script of ["consumer.sh", "install.sh", "run-native.sh", "materialize-release.sh", "verify-integration.sh"]) file(`scripts/${script}`);
  const adapter = object(Bun.TOML.parse(file("native-adapter/Cargo.toml")), "adapter Cargo manifest");
  const dependencies = object(adapter.dependencies, "adapter dependencies");
  const contracts = object(dependencies["desktop-pet-contracts"], "contracts dependency");
  requireCondition(object(adapter.package, "adapter package").name === "desktop-pet-herdr" && contracts.git === "https://github.com/hanbong5938/desktop-pet" && typeof contracts.rev === "string" && /^[0-9a-f]{40}$/.test(contracts.rev) && !contracts.path, "Adapter must depend on the published immutable contracts revision, not common app or local source");
  const lock = object(Bun.TOML.parse(file("native-adapter/Cargo.lock")), "adapter Cargo lock");
  const locked = records(lock.package, "locked packages").filter((entry) => entry.name === "desktop-pet-contracts");
  requireCondition(locked.length === 1 && locked[0]?.source === `git+${contracts.git}?rev=${contracts.rev}#${contracts.rev}`, "Adapter lock does not resolve the reviewed contracts revision");
  requireCondition(!dependencies["desktop-pet-core"] && !dependencies["desktop-pet-app"], "Herdr adapter must not own/build the common app");
  file("integrations/herdr/client-attached.patch");
  file("integrations/omp/herdr-pet-outcomes.ts");
  const pkg = object(JSON.parse(file("package.json")), "integration package");
  const scripts = object(pkg.scripts, "integration package scripts");
  requireCondition(pkg.private === true && pkg.version === undefined && scripts.check === "tsc --noEmit", "Root package must be integration tooling, not a native release version authority");
  for (const obsolete of ["native", "web", "assets", "tools", ".agents/skills/character-creator", ".agents/skills/create-pet-character", "scripts/package-native.ts", "scripts/build-rig-native.mjs", "scripts/release-docs.ts", "scripts/release-docs.test.ts", "scripts/wiki-docs.ts", "scripts/wiki-docs.test.ts", "scripts/publish-wiki.sh", ".github/workflows/wiki.yml"]) requireCondition(!existsSync(path.join(root, obsolete)), `Obsolete common product owner remains: ${obsolete}`);
  for (const command of ["build:native", "test:native", "package:native", "install:local"]) requireCondition(!(command in scripts), `Obsolete root command remains: ${command}`);
  return manifest.version;
}

async function release(mode: string, tag: string | undefined, ref: string | undefined, output: string | undefined): Promise<void> {
  // Fail clearly even before tag/ref checks when real core publication is absent.
  requireCondition(existsSync(path.join(root, "core-release.json")), "Missing reviewed core-release.json; publish the real core release, materialize its actual metadata/evidence and review it before integration publication");
  const metadataText = file("core-release.json");
  const metadata = object(JSON.parse(metadataText), "reviewed core release metadata");
  const version = source();
  requireCondition(tag === `v${version}`, "Tag must match only herdr-plugin.toml integration version");
  requireCondition(ref && /^[0-9a-f]{40}$/.test(ref), "Supply the reviewed full immutable Herdr commit SHA");
  requireCondition(run("git", ["rev-parse", "HEAD"]) === ref, "Checked-out Herdr HEAD differs from reviewed integration ref");
  requireCondition(run("git", ["status", "--porcelain", "--untracked-files=normal"]) === "", "Integration publication requires a clean, materialized tracked tree");
  const refs = run("git", ["ls-remote", `https://github.com/${repository}.git`, `refs/tags/${tag}`, `refs/tags/${tag}^{}`]).split("\n").filter(Boolean).map((line) => line.split(/\s+/));
  const publicCommit = refs.find((entry) => entry[1] === `refs/tags/${tag}^{}`) ?? refs.find((entry) => entry[1] === `refs/tags/${tag}`);
  requireCondition(publicCommit?.[0] === ref, "Public integration tag is absent or resolves to a different reviewed commit; this publisher never creates/moves tags");
  if (process.env.GITHUB_REPOSITORY) requireCondition(process.env.GITHUB_REPOSITORY === repository, "Wrong publication repository");
  const notes = file(`docs/releases/${tag}.md`);
  requireCondition(notes.split(/\r?\n/)[0] === `# Desktop Pet — Herdr integration ${tag} release notes` && notes.trim().length > 200, "Missing reviewed integration-only release note/header");
  requireCondition(notes.includes("integration_api") && notes.includes("https://github.com/hanbong5938/desktop-pet/releases/tag/v0.4.0"), "Notes must describe the independent integration_api contract and actual core release authority");
  for (const match of notes.matchAll(/!?\[[^\]]*\]\(([^\s)]+)(?:\s+[^)]*)?\)/g)) requireCondition(/^(https:\/\/|#)/.test(match[1]!), "Reviewed release notes must use absolute HTTPS or in-page links, not repository-relative Release-body links");
  const evidence = "release-evidence";
  const record = file(`${evidence}/release.json`);
  requireCondition(typeof metadata === "object" && metadata !== null && "release_record_sha256" in metadata && metadata.release_record_sha256 === createHash("sha256").update(record).digest("hex"), "Reviewed core metadata does not match retained real release evidence");
  file(`${evidence}/desktop-pet-install-v0.4.0.sh`);
  file(`${evidence}/desktop-pet-install-v0.4.0.sh.sha256`);
  const scratch = mkdtempSync(path.join(tmpdir(), "herdr-integration-release-"));
  try {
    // Reuse the existing anonymous fixed-authority verifier, including all six
    // actual assets, installer/checksum bytes, release ID and resolved core ref.
    run("bash", ["scripts/materialize-release.sh", path.join(root, evidence, "desktop-pet-install-v0.4.0.sh"), path.join(root, evidence, "desktop-pet-install-v0.4.0.sh.sha256"), path.join(root, evidence, "release.json"), path.join(scratch, "materialized")]);
    const currentMetadata = object(JSON.parse(readFileSync(path.join(scratch, "materialized/core-release.json"), "utf8")), "authenticated core release metadata");
    // The frozen evidence hash is checked above. Fresh API bytes can differ in
    // download counts/timestamps; compare every materialized authority field,
    // not the hash of those mutable counters. Never rewrite the reviewed record.
    const { release_record_sha256: reviewedRecordDigest, ...reviewedAuthority } = metadata;
    const { release_record_sha256: currentRecordDigest, ...currentAuthority } = currentMetadata;
    requireCondition(typeof reviewedRecordDigest === "string" && typeof currentRecordDigest === "string" && isDeepStrictEqual(reviewedAuthority, currentAuthority), "Committed reviewed core-release.json differs from the actual public release; re-review metadata, never infer a digest/ref");
    if (output) writeFileSync(path.resolve(output), notes, { flag: "wx" });
    if (mode === "publish") {
      requireCondition(process.env.HERDR_INTEGRATION_RELEASE_AUTHORIZATION === `${repository}@${ref}`, "Publication requires explicit parent/manual authorization for this exact repository and reviewed commit");
      const body = path.join(scratch, "notes.md");
      writeFileSync(body, notes);
      // No file arguments: this release has no app, archive, checksum or rig assets.
      // gh refuses an existing release. Never edit existing releases or old tags.
      run("gh", ["release", "create", tag, "--repo", repository, "--verify-tag", "--target", ref, "--title", `Desktop Pet — Herdr integration ${tag}`, "--notes-file", body, "--latest=false"]);
      const published = object(JSON.parse(run("gh", ["api", `repos/${repository}/releases/tags/${tag}`])), "published integration release");
      requireCondition(published.tag_name === tag && published.draft === false && published.prerelease === false && records(published.assets, "published assets").length === 0 && published.body === notes, "Published integration release does not match reviewed assetless notes; inspect manually, no automatic rewrite");
      console.log(published.html_url);
    } else console.log(`Reviewed integration ${tag} at ${ref}; actual core release metadata authenticated. Nothing published.`);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

try {
  const [mode, tag, ref, output, ...extra] = process.argv.slice(2);
  requireCondition(extra.length === 0, "Unexpected release arguments");
  if (mode === "source" && !tag && !ref && !output) {
    console.log(`Integration source contract ${source()} validated without release metadata or publication.`);
  } else {
    requireCondition(mode === "check" || mode === "publish", "Usage: bun scripts/integration-release.ts source | {check|publish} TAG REVIEWED_HERDR_SHA [NEW_NOTES_OUTPUT]");
    await release(mode, tag, ref, output);
  }
} catch (error) {
  console.error(`Herdr integration release: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
}
