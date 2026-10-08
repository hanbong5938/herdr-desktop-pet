import { afterEach, describe, expect, test } from "bun:test";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, readdir, rm, stat, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { requiredAssetNames, type ReleaseRecord } from "./release-docs";
import { generateWiki, publicationState, requirePublishedRelease, type WikiOptions } from "./wiki-docs";

const repository = "example/herdr";
const temporary: string[] = [];
afterEach(async () => {
  for (const path of temporary.splice(0)) await rm(path, { recursive: true, force: true });
});
function git(root: string, ...args: string[]): string {
  const result = spawnSync("git", ["-C", root, ...args], { encoding: "utf8" });
  if (result.status !== 0) throw new Error(`Fixture Git failed: ${result.stderr}`);
  return result.stdout.trim();
}
async function put(root: string, path: string, content: string | Uint8Array): Promise<void> {
  await mkdir(dirname(join(root, path)), { recursive: true });
  await writeFile(join(root, path), content);
}
function commitFixture(root: string): string {
  git(root, "add", ".");
  git(root, "-c", "user.name=Wiki test", "-c", "user.email=wiki-test@example.invalid", "commit", "--quiet", "-m", "Fixture documentation");
  return git(root, "rev-parse", "HEAD");
}
async function fixture(): Promise<WikiOptions> {
  const parent = await mkdtemp(join(tmpdir(), "herdr-wiki-test-"));
  temporary.push(parent);
  const root = join(parent, "source");
  await mkdir(root);
  git(root, "init", "--quiet", "--initial-branch=main");
  const sources: Record<string, string> = {
    "README.md": "# Source installation guide\n",
    "docs/releases/README.md": "# Versions\n\n[Policy](policy.md#compatibility) and [installation](../../README.md).\n\n![Icon](../../assets/icon.png)\n\n[Image source](../../assets/icon.png) and [Assets](../../assets).\n",
    "docs/releases/README.ko.md": "# 버전\n\n[정책](policy.md)\n",
    "docs/releases/policy.md": "# Version policy\n\n## Compatibility\nReviewed compatibility guidance.\n",
    "docs/releases/unreleased.md": "# Unreleased\n\nPending work, not shipped. [Upgrade](../migrations/unreleased.md).\n",
    "docs/releases/publishing.md": "# Publishing\n\n[Release status](README.md)\n",
    "docs/releases/0.1.md": "# Release line 0.1\n",
    "docs/migrations/v0.1.4-to-v0.1.6.md": "# Migration\n\nBack up character packs before upgrading.\n",
    "docs/migrations/unreleased.md": "# Future upgrades\n\nDo not migrate yet.\n",
    "docs/migrations/unreleased.ko.md": "# 미출시 업그레이드\n\n아직 마이그레이션하지 마세요.\n",
  };
  for (const [path, contents] of Object.entries(sources)) await put(root, path, contents);
  await put(root, "assets/icon.png", Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aD1sAAAAASUVORK5CYII=", "base64"));
  commitFixture(root);
  return { root, output: join(parent, "wiki"), repository, sourceRef: "main", releases: [] };
}
function release(tag: string, overrides: Partial<ReleaseRecord> = {}): ReleaseRecord {
  return {
    tag_name: tag, html_url: `https://github.com/${repository}/releases/tag/${tag}`,
    published_at: "2026-01-02T03:04:05Z", draft: false, prerelease: false,
    assets: requiredAssetNames(tag).map((name) => ({ name, size: 123, state: "uploaded" })),
    ...overrides,
  };
}
async function note(root: string, tag: string, content = "Reviewed release evidence."): Promise<void> {
  await put(root, `docs/releases/${tag}.md`, `# Herdr Desktop Pet ${tag} release notes\n\n${content}\n`);
}

async function rendered(markdown: string): Promise<{
  links: { href: string | null; title: string | null }[];
  images: { src: string | null; title: string | null; alt: string | null; parent: string | null }[];
}> {
  const links: { href: string | null; title: string | null }[] = [];
  const images: { src: string | null; title: string | null; alt: string | null; parent: string | null }[] = [];
  await new HTMLRewriter()
    .on("a", { element(element) { links.push({ href: element.getAttribute("href"), title: element.getAttribute("title") }); } })
    .on("img", { element(element) { images.push({
      src: element.getAttribute("src"), title: element.getAttribute("title"), alt: element.getAttribute("alt"), parent: null,
    }); } })
    .on("a img", { element() { images.at(-1)!.parent = links.at(-1)!.href; } })
    .transform(new Response(Bun.markdown.html(markdown))).text();
  return { links, images };
}

describe("publication states", () => {
  test("only complete actually published stable records receive published labels", () => {
    const complete = release("v0.1.11");
    expect(publicationState(complete.tag_name, complete)).toBe("published");
    expect(publicationState("v0.1.5")).toBe("tagged-only");
    expect(publicationState("v0.2.0")).toBe("no-release");
    expect(publicationState("v0.1.11", release("v0.1.11", { draft: true }))).toBe("draft");
    expect(publicationState("v0.1.11", release("v0.1.11", { prerelease: true }))).toBe("prerelease");
    for (const published_at of [null, "", "not-a-date"]) {
      expect(publicationState("v0.1.11", release("v0.1.11", { published_at }))).toBe("unpublished");
    }
    for (const assets of [complete.assets.slice(1), complete.assets.map((asset) => ({ ...asset, size: 0 })), complete.assets.map((asset) => ({ ...asset, state: "new" }))]) {
      expect(publicationState("v0.1.11", release("v0.1.11", { assets }))).toBe("partial");
    }
  });
  test("required-release refuses every non-shipped case", () => {
    for (const catalog of [[], [release("v0.1.11", { draft: true })], [release("v0.1.11", { prerelease: true })], [release("v0.1.11", { assets: [] })], [release("v0.1.11", { published_at: null })]]) {
      expect(() => requirePublishedRelease("v0.1.11", catalog)).toThrow("not a complete public");
    }
    expect(() => requirePublishedRelease("v0.1.11-rc.1", [])).toThrow();
  });
});

describe("Wiki projection", () => {
  test("links portable canonical pages, source guides and images; writes only managed pages", async () => {
    const options = await fixture();
    const head = git(options.root, "rev-parse", "HEAD");
    const names = await generateWiki(options);
    expect(names).toContain("Release-Line-0.1.md");
    const { links, images } = await rendered(await readFile(join(options.output, "Home.md"), "utf8"));
    expect(links).toContainEqual({ href: `https://github.com/${repository}/wiki/Versioning-and-Compatibility#compatibility`, title: null });
    expect(links).toContainEqual({ href: `https://github.com/${repository}/blob/${head}/README.md`, title: null });
    expect(links).toContainEqual({ href: `https://github.com/${repository}/blob/${head}/assets/icon.png`, title: null });
    expect(links).toContainEqual({ href: `https://github.com/${repository}/tree/${head}/assets`, title: null });
    expect(images).toContainEqual({ src: `https://raw.githubusercontent.com/${repository}/${head}/assets/icon.png`, title: null, alt: "Icon", parent: null });
    expect((await readFile(join(options.output, ".wiki-managed-pages"), "utf8")).trim().split("\n")).toEqual(names);
    expect((await readdir(options.output)).filter((name) => name.endsWith(".md")).sort()).toEqual(names);
  });

  test("current Wiki renders mapped and configured-ref source, image and shared-reference destinations", async () => {
    const options = await fixture();
    await put(options.root, "assets/a).png", "image");
    await put(options.root, "guides/a).md", "# Guide\n");
    await put(options.root, "guides/%29.md", "# Encoded name\n");
    await put(options.root, "docs/releases/README.md", [
      "# Versions", "", "Reviewed current guidance.",
      "[Policy](policy.md#compatibility) [Guide](../../guides/a%29.md?plain=(1)#part(2))",
      "[Percent](../../guides/%2529.md) [Assets](../../assets)",
      "[![Nested][Art]][Art] ![Art][] [Art] [External](https://example.test/a)",
      "",
      '[Art]: ../../assets/a%29.png "Current portrait"',
      '[Art]: ../../assets/duplicate.png',
      "`![Code][Art]` \\[Escaped][Art] [Missing][undefined]",
    ].join("\n"));
    commitFixture(options.root);
    const head = git(options.root, "rev-parse", "HEAD");
    await generateWiki(options);
    const { links, images } = await rendered(await readFile(join(options.output, "Home.md"), "utf8"));
    const blob = `https://github.com/${repository}/blob/${head}/assets/a%29.png`;
    const raw = `https://raw.githubusercontent.com/${repository}/${head}/assets/a%29.png`;
    expect(links).toContainEqual({ href: `https://github.com/${repository}/wiki/Versioning-and-Compatibility#compatibility`, title: null });
    expect(links).toContainEqual({ href: `https://github.com/${repository}/blob/${head}/guides/a%29.md?plain=%281%29#part%282%29`, title: null });
    expect(links).toContainEqual({ href: `https://github.com/${repository}/blob/${head}/guides/%2529.md`, title: null });
    expect(links).toContainEqual({ href: `https://github.com/${repository}/tree/${head}/assets`, title: null });
    expect(links).toContainEqual({ href: blob, title: "Current portrait" });
    expect(links).toContainEqual({ href: "https://example.test/a", title: null });
    expect(images).toContainEqual({ src: raw, title: "Current portrait", alt: "Nested", parent: blob });
    expect(images).toContainEqual({ src: raw, title: "Current portrait", alt: "Art", parent: null });
    expect(images.every((image) => image.src !== null && !image.src.includes("duplicate"))).toBe(true);
  });

  test("current Wiki keeps rejected definition prose links and literal external inline text", async () => {
    const options = await fixture();
    await put(options.root, "assets/art.png", "art");
    for (const name of ["nested-title", "empty-label", "bracket-label", "tight-title", "bad-title", "inner-link"]) {
      await put(options.root, `guides/${name}.md`, "# Guide\n");
    }
    await put(options.root, "docs/releases/README.md", [
      "# Versions", "", "Reviewed guidance.",
      '[unused]: https://example.test "Title" trailing [Guide](../../README.md)',
      '[unused]: https://example.test (bad ( [Guide](../../guides/nested-title.md)',
      '[ ]: https://example.test "[Guide](../../guides/empty-label.md)"',
      '[a[b]: https://example.test "[Guide](../../guides/bracket-label.md)"',
      '[label]: <https://example.test>"[Guide](../../guides/tight-title.md)"',
      '[Website](https://example.test/![Art]) [Named](https://example.test "literal ![Art]") [Compact](<https://example.test/no-gap>"literal ![Art]")',
      "[bad](https://example.test (bad ( [Guide](../../guides/bad-title.md)))",
      '[outer [Guide](../../guides/inner-link.md)](https://example.test "![Art]")',
      '[outer [Guide][g]](https://example.test/defined "literal reference")',
      '[outer [Guide][undefined]](https://example.test/undefined "literal ![Art]")',
      '[outer [Guide]](https://example.test/ordinary "literal ![Art]")',
      "[![Portrait][Art]][Art] ![Art][Art] [Art]", "",
      "[Art]:",
      "  ../../assets/art.png",
      '"Current portrait"',
      "[g]: policy.md#compatibility",
    ].join("\n"));
    commitFixture(options.root);
    const head = git(options.root, "rev-parse", "HEAD");
    await generateWiki(options);
    const { links, images } = await rendered(await readFile(join(options.output, "Home.md"), "utf8"));
    const blob = `https://github.com/${repository}/blob/${head}/assets/art.png`;
    const raw = `https://raw.githubusercontent.com/${repository}/${head}/assets/art.png`;
    expect(links).toContainEqual({ href: `https://github.com/${repository}/blob/${head}/README.md`, title: null });
    for (const name of ["nested-title", "empty-label", "bracket-label", "tight-title", "bad-title", "inner-link"]) {
      expect(links).toContainEqual({ href: `https://github.com/${repository}/blob/${head}/guides/${name}.md`, title: null });
    }
    expect(links).toContainEqual({ href: `https://github.com/${repository}/wiki/Versioning-and-Compatibility#compatibility`, title: null });
    expect(links).toContainEqual({ href: "https://example.test/!%5BArt%5D", title: null });
    expect(links).toContainEqual({ href: "https://example.test", title: "literal ![Art]" });
    expect(links).toContainEqual({ href: "https://example.test/no-gap", title: "literal ![Art]" });
    expect(links).toContainEqual({ href: "https://example.test/undefined", title: "literal ![Art]" });
    expect(links).toContainEqual({ href: "https://example.test/ordinary", title: "literal ![Art]" });
    expect(links).toContainEqual({ href: blob, title: "Current portrait" });
    expect(images).toContainEqual({ src: raw, title: "Current portrait", alt: "Portrait", parent: blob });
    expect(images).toContainEqual({ src: raw, title: "Current portrait", alt: "Art", parent: null });
    expect(images.filter((image) => image.src === raw && image.alt === "Art" && image.parent === null)).toHaveLength(2);
  });

  test("current and frozen Wiki resolve review regressions at HEAD and tag respectively", async () => {
    const options = await fixture();
    const tag = "v0.2.0";
    const examples = [
      'Reviewed guidance.\n[unused]: https://example.test "[Guide](../../README.md)"',
      '[outer ![alt [Guide](../../README.md)]](https://example.test "[Missing](../../README.md)")',
      '[outer [inner]()](https://example.test "[Missing](../../README.md)")',
    ];
    await put(options.root, "docs/releases/README.md", `# Versions\n\n${examples.join("\n\n")}\n`);
    await note(options.root, tag, examples.join("\n\n"));
    commitFixture(options.root);
    git(options.root, "tag", tag);
    const head = git(options.root, "rev-parse", "HEAD");
    options.releases = [release(tag)];
    await generateWiki(options);
    const current = await readFile(join(options.output, "Home.md"), "utf8");
    const frozen = await readFile(join(options.output, `Release-${tag}.md`), "utf8");
    expect(current).toContain(`https://github.com/${repository}/blob/${head}/README.md`);
    expect(frozen).toContain(`https://github.com/${repository}/blob/${tag}/README.md`);
    expect(current).not.toContain(`https://github.com/${repository}/blob/${tag}/README.md`);
    expect(frozen).not.toContain(`https://github.com/${repository}/blob/${head}/README.md`);
  });

  test("numeric semver determines newest; late old-tag trigger does not roll back current catalog", async () => {
    const options = await fixture();
    git(options.root, "tag", "v0.1.9");
    for (const tag of ["v0.1.9", "v0.1.10", "v0.1.11", "v0.1.5", "v0.2.0"]) await note(options.root, tag);
    commitFixture(options.root);
    options.sourceRef = "v0.1.9";
    options.releases = [
      release("v0.1.9", { published_at: "2026-10-01T00:00:00Z" }),
      release("v0.1.10"), release("v0.1.11", { published_at: "2026-01-01T00:00:00Z" }),
    ];
    await generateWiki(options);
    const index = (await rendered(await readFile(join(options.output, "Release-Status.md"), "utf8"))).links.map((link) => link.href);
    const ordered = ["v0.1.11", "v0.1.10", "v0.1.9"].map((tag) => `https://github.com/${repository}/wiki/Release-${tag}`);
    const positions = ordered.map((href) => index.indexOf(href));
    expect(positions.every((position) => position >= 0)).toBe(true);
    expect(positions).toEqual([...positions].sort((a, b) => a - b));
    for (const tag of ["v0.1.5", "v0.2.0"]) expect(index).not.toContain(`https://github.com/${repository}/releases/tag/${tag}`);
    const older = (await rendered(await readFile(join(options.output, "Older-Releases.md"), "utf8"))).links.map((link) => link.href);
    expect(older).toContain(`https://github.com/${repository}/releases/tag/v0.1.11`);
    expect(older).toContain(`https://github.com/${repository}/wiki/Release-v0.1.10`);
  });

  test("draft, partial and prerelease records never become download recommendations", async () => {
    const options = await fixture();
    for (const tag of ["v0.1.9", "v0.1.10", "v0.1.11"]) await note(options.root, tag);
    options.releases = [release("v0.1.9", { draft: true }), release("v0.1.10", { prerelease: true }), release("v0.1.11", { assets: [] })];
    await generateWiki(options);
    for (const page of ["Release-Status.md", "Older-Releases.md"]) {
      const hrefs = (await rendered(await readFile(join(options.output, page), "utf8"))).links.map((link) => link.href);
      for (const tag of ["v0.1.9", "v0.1.10", "v0.1.11"]) {
        expect(hrefs).toContain(`https://github.com/${repository}/wiki/Release-${tag}`);
      }
      expect(hrefs).not.toContain(`https://github.com/${repository}/releases/tag/v0.1.9`);
      for (const tag of ["v0.1.10", "v0.1.11"]) {
        expect(hrefs).toContain(`https://github.com/${repository}/releases/tag/${tag}`);
      }
      expect(hrefs.some((href) => href?.startsWith(`https://github.com/${repository}/releases/download/`))).toBe(false);
    }
  });

  test("frozen Wiki uses tagged raw, blob, tree and mapped URLs after HEAD deletes assets", async () => {
    const options = await fixture();
    await put(options.root, "upgrade).txt", "Tagged upgrade source");
    await put(options.root, "assets/tagged).png", await readFile(join(options.root, "assets/icon.png")));
    for (const name of ["nested-title", "empty-label", "bracket-label", "tight-title", "bad-title", "inner-link"]) {
      await put(options.root, `guides/${name}.md`, "# Tagged guide\n");
    }
    await note(options.root, "v0.2.0", [
      "Shipped behavior. [Tagged guide](../../upgrade\\).txt?plain=(1)#part(2))",
      "[Policy](policy.md) [Assets](../../assets) [![Tagged icon][Art]][Art] ![Art][]",
      '[unused]: https://example.test "Title" trailing [Guide](../../README.md)',
      '[unused]: https://example.test (bad ( [Guide](../../guides/nested-title.md)',
      '[ ]: https://example.test "[Guide](../../guides/empty-label.md)"',
      '[a[b]: https://example.test "[Guide](../../guides/bracket-label.md)"',
      '[label]: <https://example.test>"[Guide](../../guides/tight-title.md)"',
      '[Website](https://example.test/![Art]) [Named](https://example.test "literal ![Art]") [Compact](<https://example.test/no-gap>"literal ![Art]")',
      "[bad](https://example.test (bad ( [Guide](../../guides/bad-title.md)))",
      '[outer [Guide](../../guides/inner-link.md)](https://example.test "![Art]")',
      '[outer [Guide][g]](https://example.test/defined "literal reference")',
      '[outer [Guide][undefined]](https://example.test/undefined "literal ![Art]")',
      '[outer [Guide]](https://example.test/ordinary "literal ![Art]")',
      "",
      "[Art]: ../../assets/tagged%29.png",
      "[g]: policy.md#compatibility",
    ].join("\n"));
    commitFixture(options.root);
    git(options.root, "tag", "v0.2.0");
    await note(options.root, "v0.2.0", "New main-only features that never shipped.");
    await rm(join(options.root, "upgrade).txt"));
    await rm(join(options.root, "assets/tagged).png"));
    await rm(join(options.root, "guides"), { recursive: true });
    commitFixture(options.root);
    options.releases = [release("v0.2.0")];
    await generateWiki(options);
    const page = await readFile(join(options.output, "Release-v0.2.0.md"), "utf8");
    const { links, images } = await rendered(page);
    const blob = `https://github.com/${repository}/blob/v0.2.0/assets/tagged%29.png`;
    const raw = `https://raw.githubusercontent.com/${repository}/v0.2.0/assets/tagged%29.png`;
    expect(links).toContainEqual({ href: `https://github.com/${repository}/blob/v0.2.0/upgrade%29.txt?plain=%281%29#part%282%29`, title: null });
    expect(links).toContainEqual({ href: `https://github.com/${repository}/wiki/Versioning-and-Compatibility`, title: null });
    expect(links).toContainEqual({ href: `https://github.com/${repository}/tree/v0.2.0/assets`, title: null });
    expect(links).toContainEqual({ href: `https://github.com/${repository}/blob/v0.2.0/README.md`, title: null });
    for (const name of ["nested-title", "empty-label", "bracket-label", "tight-title", "bad-title", "inner-link"]) {
      expect(links).toContainEqual({ href: `https://github.com/${repository}/blob/v0.2.0/guides/${name}.md`, title: null });
    }
    expect(links).toContainEqual({ href: `https://github.com/${repository}/wiki/Versioning-and-Compatibility#compatibility`, title: null });
    expect(links).toContainEqual({ href: "https://example.test/!%5BArt%5D", title: null });
    expect(links).toContainEqual({ href: "https://example.test", title: "literal ![Art]" });
    expect(links).toContainEqual({ href: "https://example.test/no-gap", title: "literal ![Art]" });
    expect(links).toContainEqual({ href: "https://example.test/undefined", title: "literal ![Art]" });
    expect(links).toContainEqual({ href: "https://example.test/ordinary", title: "literal ![Art]" });
    expect(links).toContainEqual({ href: blob, title: null });
    expect(images).toContainEqual({ src: raw, title: null, alt: "Tagged icon", parent: blob });
    expect(images).toContainEqual({ src: raw, title: null, alt: "Art", parent: null });
    expect(images.filter((image) => image.src === raw && image.alt === "Art" && image.parent === null)).toHaveLength(2);
    // Catalog publishes the same frozen surface even without its note at HEAD.
    await rm(join(options.root, "docs/releases/v0.2.0.md"));
    await generateWiki(options);
    expect(await rendered(await readFile(join(options.output, "Release-v0.2.0.md"), "utf8"))).toEqual({ links, images });
  });

  test("future published missing frozen note fails without changing existing managed output", async () => {
    const options = await fixture();
    git(options.root, "tag", "v0.2.0");
    await generateWiki(options);
    const before = await readFile(join(options.output, "Home.md"), "utf8");
    await note(options.root, "v0.2.0", "Not actually frozen at tag.");
    options.releases = [release("v0.2.0")];
    await expect(generateWiki(options)).rejects.toThrow("missing its frozen note");
    expect(await readFile(join(options.output, "Home.md"), "utf8")).toBe(before);
    expect(existsSync(join(options.output, "Release-v0.2.0.md"))).toBe(false);
  });

  test("broken ordinary and frozen relative links fail before changing managed pages or manifest", async () => {
    async function snapshot(output: string) {
      const manifest = await readFile(join(output, ".wiki-managed-pages"), "utf8");
      const names = [...manifest.trimEnd().split("\n"), ".wiki-managed-pages"];
      return {
        names: await readdir(output),
        files: await Promise.all(names.map(async (name) => ({
          name,
          bytes: await readFile(join(output, name)),
          mtimeMs: (await stat(join(output, name))).mtimeMs,
        }))),
      };
    }
    const options = await fixture();
    await generateWiki(options);
    const before = await snapshot(options.output);
    await put(options.root, "docs/releases/README.md", "# Versions\n[Missing](../../missing.md)\n");
    await expect(generateWiki(options)).rejects.toThrow("Broken local link");
    expect(await snapshot(options.output)).toEqual(before);
    for (const prose of [
      '[unused]: https://example.test "Title" trailing [Guide](TARGET)',
      '[unused]: https://example.test (bad ( [Guide](TARGET)',
      '[ ]: https://example.test "[Guide](TARGET)"',
      '[a[b]: https://example.test "[Guide](TARGET)"',
      '[label]: <https://example.test>"[Guide](TARGET)"',
      "[bad](https://example.test (bad ( [Guide](TARGET)))",
      '[outer [Guide](TARGET)](https://example.test "![Art]")',
      'Reviewed guidance.\n[unused]: https://example.test "[Guide](TARGET)"',
      '[outer ![alt [Guide](TARGET)]](https://example.test "[Missing](../../missing.md)")',
      '[outer [inner]()](https://example.test "[Missing](TARGET)")',
    ]) {
      for (const target of ["../../missing.md", "../../../outside.md"]) {
        await put(options.root, "docs/releases/README.md", `# Versions\n\n${prose.replace("TARGET", target)}\n`);
        await expect(generateWiki(options)).rejects.toThrow();
        expect(await snapshot(options.output)).toEqual(before);
      }
    }
    for (const prose of [
      "[bad](https://example.test (bad ( [Guide](TARGET)))",
      '[outer [Guide](TARGET)](https://example.test "![Art]")',
      'Reviewed guidance.\n[unused]: https://example.test "[Guide](TARGET)"',
      '[outer ![alt [Guide](TARGET)]](https://example.test "[Missing](../../missing.md)")',
      '[outer [inner]()](https://example.test "[Missing](TARGET)")',
    ]) {
      for (const target of ["../../missing.md", "../../../outside.md"]) {
        const frozen = await fixture();
        await generateWiki(frozen);
        const frozenBefore = await snapshot(frozen.output);
        await note(frozen.root, "v0.2.0", prose.replace("TARGET", target));
        commitFixture(frozen.root);
        git(frozen.root, "tag", "v0.2.0");
        if (target === "../../missing.md") {
          await put(frozen.root, "missing.md", "Main cannot repair a missing tagged guide");
        }
        frozen.releases = [release("v0.2.0")];
        await expect(generateWiki(frozen)).rejects.toThrow();
        expect(await snapshot(frozen.output)).toEqual(frozenBefore);
        expect(existsSync(join(frozen.output, "Release-v0.2.0.md"))).toBe(false);
      }
    }
  });

  test("repeat rendering is byte/mtime idempotent and removes only prior manifest-owned stale pages", async () => {
    const options = await fixture();
    await note(options.root, "v0.2.0");
    await generateWiki(options);
    await put(options.output, "User-Notes.md", "User-owned page\n");
    await put(options.output, "Release-v9.9.9.md", "Unmanaged despite matching slug\n");
    const before = await stat(join(options.output, "Home.md"));
    const names = await generateWiki(options);
    expect((await stat(join(options.output, "Home.md"))).mtimeMs).toBe(before.mtimeMs);
    expect(names).toContain("Release-v0.2.0.md");
    await rm(join(options.root, "docs/releases/v0.2.0.md"));
    await generateWiki(options);
    expect(existsSync(join(options.output, "Release-v0.2.0.md"))).toBe(false);
    expect(await readFile(join(options.output, "User-Notes.md"), "utf8")).toBe("User-owned page\n");
    expect(await readFile(join(options.output, "Release-v9.9.9.md"), "utf8")).toBe("Unmanaged despite matching slug\n");
  });

  test("bad root/output/ref/repository, mappings, headings and manifests are rejected", async () => {
    const options = await fixture();
    await expect(generateWiki({ ...options, root: join(options.root, "missing") })).rejects.toThrow();
    await expect(generateWiki({ ...options, output: join(options.root, "docs/output") })).rejects.toThrow("outside");
    await expect(generateWiki({ ...options, output: dirname(options.root) })).rejects.toThrow("outside");
    await expect(generateWiki({ ...options, repository: "../bad" })).rejects.toThrow("OWNER/REPO");
    for (const sourceRef of ["--help", "main:README.md", "main..other", "missing-ref"]) {
      await expect(generateWiki({ ...options, sourceRef })).rejects.toThrow();
    }
    await put(options.root, "docs/releases/unknown.md", "# Unknown\n");
    await expect(generateWiki(options)).rejects.toThrow("Unsupported canonical Markdown");
    await rm(join(options.root, "docs/releases/unknown.md"));
    await put(options.root, "docs/releases/v0.2.0.md", "# Wrong release title\n");
    await expect(generateWiki(options)).rejects.toThrow("Invalid release note heading");
    await rm(join(options.root, "docs/releases/v0.2.0.md"));
    await put(options.root, "docs/releases/v00.2.0.md", "# Invalid version\n");
    await expect(generateWiki(options)).rejects.toThrow("stable vMAJOR.MINOR.PATCH");
    await rm(join(options.root, "docs/releases/v00.2.0.md"));
    await put(options.root, "docs/releases/README.md", "# Versions\n[Escape](../../../outside.md)\n");
    await expect(generateWiki(options)).rejects.toThrow();
    expect(existsSync(options.output)).toBe(false);
    await put(options.root, "docs/releases/README.md", "# Versions\n");
    await put(options.output, ".wiki-managed-pages", "../User.md\n");
    await expect(generateWiki(options)).rejects.toThrow("Invalid managed-page manifest");
    await rm(join(options.output, ".wiki-managed-pages"));
    await symlink(join(options.root, "README.md"), join(options.output, "Home.md"));
    await expect(generateWiki(options)).rejects.toThrow("not a regular file");
  });

  test("required release and malformed catalog are validated before writing", async () => {
    const options = await fixture();
    await expect(generateWiki({ ...options, requireRelease: "v0.1.11" })).rejects.toThrow("not a complete public");
    expect(existsSync(options.output)).toBe(false);
    const catalogPath = join(dirname(options.root), "catalog.json");
    await writeFile(catalogPath, JSON.stringify([{ tag_name: "v0.1.11", assets: "wrong" }]));
    await expect(generateWiki({ ...options, releases: undefined, releasesJson: catalogPath })).rejects.toThrow("Malformed release catalog");
    await expect(generateWiki({ ...options, releases: [release("v0.1.11"), release("v0.1.11")] })).rejects.toThrow("Duplicate release catalog tag");
    expect(existsSync(options.output)).toBe(false);
  });

  test("offline CLI generates full Wiki, enforces required release and does not log credentials", async () => {
    const options = await fixture();
    await note(options.root, "v0.1.11");
    const catalogPath = join(dirname(options.root), "catalog.json");
    await writeFile(catalogPath, JSON.stringify([release("v0.1.11")]));
    const args = [resolve(import.meta.dir, "wiki-docs.ts"), "--root", options.root, "--output", options.output, "--repository", repository, "--source-ref", "main", "--releases-json", catalogPath];
    const result = spawnSync(process.execPath, [...args, "--require-release", "v0.1.11"], { encoding: "utf8", env: { ...process.env, GITHUB_TOKEN: "never-print-this-test-token" } });
    expect(result.status).toBe(0);
    expect(result.stdout + result.stderr).not.toContain("never-print-this-test-token");
    expect(existsSync(join(options.output, "_Sidebar.md"))).toBe(true);
    const denied = spawnSync(process.execPath, [...args, "--require-release", "v0.2.0"], { encoding: "utf8" });
    expect(denied.status).not.toBe(0);
    expect(denied.stderr).toContain("not a complete public");
  });
});
