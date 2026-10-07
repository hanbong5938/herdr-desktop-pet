import { expect, spyOn, test } from "bun:test";
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  fetchReleases, generateReleaseBody, isPublishedRelease, parseStableTag,
  requiredAssetNames, rewriteRelativeLinks,
} from "./release-docs.ts";
import type { ReleaseRecord } from "./release-docs.ts";

const tag = "v0.1.11";
const heading = `# Herdr Desktop Pet ${tag} release notes`;
const repository = "hanbong5938/herdr-desktop-pet";

function published(overrides: Partial<ReleaseRecord> = {}): ReleaseRecord {
  return {
    tag_name: tag,
    html_url: `https://github.com/${repository}/releases/tag/${tag}`,
    published_at: "2026-01-02T03:04:05Z",
    draft: false,
    prerelease: false,
    assets: requiredAssetNames(tag).map((name) => ({ name, size: 10, state: "uploaded" })),
    ...overrides,
  };
}

async function fixture(note = `${heading}\n\nReviewed native-menu improvement.\n`): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "herdr-release-docs-"));
  await mkdir(join(root, "native"));
  await mkdir(join(root, "docs/releases"), { recursive: true });
  await writeFile(join(root, "package.json"), JSON.stringify({ version: "0.1.11" }));
  await writeFile(join(root, "native/Cargo.toml"), '[package]\nname = "herdr-desktop-pet"\nversion = "0.1.11"\n');
  await writeFile(join(root, "herdr-plugin.toml"), 'version = "0.1.11"\n');
  await writeFile(join(root, "native/Cargo.lock"), 'version = 4\n[[package]]\nname = "herdr-desktop-pet"\nversion = "0.1.11"\n');
  await writeFile(join(root, `docs/releases/${tag}.md`), note);
  return root;
}

async function rendered(markdown: string): Promise<{
  links: { href: string | null; title: string | null }[];
  images: { src: string | null; title: string | null; alt: string | null; parent: string | null }[];
}> {
  const links: { href: string | null; title: string | null }[] = [];
  const images: { src: string | null; title: string | null; alt: string | null; parent: string | null }[] = [];
  const html = Bun.markdown.html(markdown);
  await new HTMLRewriter()
    .on("a", { element(element) { links.push({ href: element.getAttribute("href"), title: element.getAttribute("title") }); } })
    .on("img", { element(element) { images.push({
      src: element.getAttribute("src"), title: element.getAttribute("title"), alt: element.getAttribute("alt"), parent: null,
    }); } })
    .on("a img", { element() { images.at(-1)!.parent = links.at(-1)!.href; } })
    .transform(new Response(html)).text();
  return { links, images };
}

test("stable tags preserve zero versions and reject suffixes, leading zeros, and unsafe numbers", () => {
  expect(parseStableTag("v0.0.0")).toEqual({ version: "0.0.0", major: 0, minor: 0, patch: 0 });
  expect(parseStableTag("v12.30.456")).toEqual({ version: "12.30.456", major: 12, minor: 30, patch: 456 });
  for (const invalid of ["0.1.11", "v01.1.11", "v0.01.11", "v0.1.011", "v0.1.11-rc.1", "v0.1.11+build", "v0.1.11\n", "v9007199254740992.0.0"]) {
    expect(() => parseStableTag(invalid)).toThrow();
  }
  expect(requiredAssetNames(tag)).toEqual([
    "HerdrDesktopPet-v0.1.11-macos-arm64.tar.gz",
    "HerdrDesktopPet-v0.1.11-macos-arm64.tar.gz.sha256",
    "SHA256SUMS",
  ]);
});

test("publication requires an actual stable published release and all three uploaded nonempty assets", () => {
  const complete = published();
  expect(isPublishedRelease(complete)).toBe(true);
  for (const override of [
    { draft: true }, { prerelease: true }, { tag_name: "v0.1.11-rc.1" },
    { published_at: null }, { published_at: "not-a-date" }, { assets: [] },
    { assets: complete.assets.slice(0, 2) },
    { assets: complete.assets.map((asset, index) => ({ ...asset, size: index === 1 ? 0 : asset.size })) },
    { assets: complete.assets.map((asset, index) => ({ ...asset, state: index === 2 ? "starter" : "uploaded" })) },
  ]) expect(isPublishedRelease(published(override))).toBe(false);
  expect(isPublishedRelease(published({ assets: [{ name: "unrelated.zip", size: 100 }, ...complete.assets] }))).toBe(true);
});

test("catalog consumers exclude releases with unknown upload state for any required asset", () => {
  const complete = published();
  const unknownUploads = complete.assets.map((_, missingIndex) => published({
    assets: complete.assets.map((asset, index) => {
      const { name, size } = asset;
      return index === missingIndex ? { name, size } : asset;
    }),
  }));
  const allUnknown = published({ assets: complete.assets.map(({ name, size }) => ({ name, size })) });
  expect([allUnknown, ...unknownUploads, complete].filter(isPublishedRelease)).toEqual([complete]);
});

test("release destinations render complete URLs, titles, and nested image links", async () => {
  const root = await fixture([
    heading, "", "Reviewed artwork.",
    '[closing](../../guides/a%29.md "Closing") [escaped](../../guides/a\\).md)',
    '[percent](../../guides/%2529.md) [spaced](<../../guides/install notes.md> "Space")',
    '[query](../../guides/a%29.md?view=(large)#frag(ment))',
    '[![Portrait](../../assets/a%29.png "Portrait")](../../guides/a%29.md#install)',
    '[source](../../assets/a%29.png)',
  ].join("\n"));
  try {
    for (const path of ["guides/a).md", "guides/%29.md", "guides/install notes.md", "assets/a).png"]) {
      await mkdir(join(root, path.split("/")[0]!), { recursive: true });
      await writeFile(join(root, path), "fixture");
    }
    const { links, images } = await rendered(await generateReleaseBody(root, tag, repository));
    const blob = `https://github.com/${repository}/blob/${tag}/`;
    const raw = `https://raw.githubusercontent.com/${repository}/${tag}/`;
    expect(links).toContainEqual({ href: `${blob}guides/a%29.md`, title: "Closing" });
    expect(links).toContainEqual({ href: `${blob}guides/a%29.md`, title: null });
    expect(links).toContainEqual({ href: `${blob}guides/%2529.md`, title: null });
    expect(links).toContainEqual({ href: `${blob}guides/install%20notes.md`, title: "Space" });
    expect(links).toContainEqual({ href: `${blob}guides/a%29.md?view=%28large%29#frag%28ment%29`, title: null });
    expect(links).toContainEqual({ href: `${blob}assets/a%29.png`, title: null });
    expect(images).toContainEqual({ src: `${raw}assets/a%29.png`, title: "Portrait", alt: "Portrait", parent: `${blob}guides/a%29.md#install` });
    await rm(join(root, "assets/a).png"));
    await expect(generateReleaseBody(root, tag, repository)).rejects.toThrow("missing repository path");
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("external, fragment, code, escaped and unresolved Markdown do not become repository links", async () => {
  const input = [
    "Reviewed links. [web](https://example.test/x) [mail](mailto:author@example.test) [local](#heading) [cdn](//example.test/a)",
    "`[literal](../../readme.md)` and ``x `[literal](../../readme.md)` x``",
    "```md", "[literal](../../readme.md)", "```",
    "~~~", "[literal](../../readme.md)", "~~~",
    "\\[not a link](../../readme.md) [Missing][undefined]",
    "![External][Remote] [Remote]",
    "",
    "[Remote]: https://example.test/image(1).png",
  ].join("\n");
  const root = await fixture(`${heading}\n\n${input}\n`);
  try {
    const { links, images } = await rendered(await generateReleaseBody(root, tag, repository));
    expect(links.map((link) => link.href)).toEqual([
      "https://example.test/x", "mailto:author@example.test", "#heading", "//example.test/a",
      "https://example.test/image(1).png",
    ]);
    expect(images).toContainEqual({ src: "https://example.test/image(1).png", title: null, alt: "External", parent: null });
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("refuses repository escapes, malformed encoding, and invalid source paths", () => {
  for (const link of ["../../../outside.md", "%2e%2e/%2e%2e/%2e%2e/outside.md", "bad%zz.md", "bad%5cpath.md"]) {
    expect(() => rewriteRelativeLinks(`[bad](${link})`, "docs/releases/v0.1.11.md", () => "unused")).toThrow();
  }
  expect(() => rewriteRelativeLinks("", "../outside.md", () => "unused")).toThrow();
});

test("catalog consumers retain published releases beyond the first 100-record page", async () => {
  const first = Array.from({ length: 100 }, (_, index) => {
    const pageTag = `v0.1.${index}`;
    return published({ tag_name: pageTag, assets: requiredAssetNames(pageTag).map((name) => ({ name, size: 10, state: "uploaded" })) });
  });
  const nextTag = "v0.2.0";
  const laterRelease = published({ tag_name: nextTag, assets: requiredAssetNames(nextTag).map((name) => ({ name, size: 10, state: "uploaded" })) });
  let firstPageRead = false;
  const fetchImplementation: typeof fetch = Object.assign(
    async (input: Parameters<typeof fetch>[0]) => {
      const page = new URL(input instanceof Request ? input.url : String(input)).searchParams.get("page");
      if (page === "1" && !firstPageRead) { firstPageRead = true; return Response.json(first); }
      if (page === "2") return Response.json([laterRelease]);
      throw new Error("Unexpected or repeated catalog page");
    },
    { preconnect: globalThis.fetch.preconnect },
  );
  const mock = spyOn(globalThis, "fetch").mockImplementation(fetchImplementation);
  try {
    const visibleTags = (await fetchReleases(repository)).filter(isPublishedRelease).map((release) => release.tag_name);
    expect(visibleTags).toContain("v0.1.0");
    expect(visibleTags).toContain(nextTag);
  } finally { mock.mockRestore(); }
});

test("catalog errors reject rather than invent publication evidence or expose token text", async () => {
  const mock = spyOn(globalThis, "fetch").mockResolvedValue(new Response("test-token", { status: 403 }));
  try {
    const failure: unknown = await fetchReleases(repository, "test-token").catch((error: unknown) => error);
    expect(failure).toBeInstanceOf(Error);
    if (!(failure instanceof Error)) throw new Error("Expected catalog failure");
    expect(failure.message).toContain("403");
    expect(failure.message).not.toContain("test-token");
    mock.mockResolvedValue(Response.json([{ tag_name: tag }]));
    await expect(fetchReleases(repository)).rejects.toThrow("Invalid GitHub release catalog");
    await expect(fetchReleases("bad/repo/extra")).rejects.toThrow("Invalid GitHub repository");
  } finally { mock.mockRestore(); }
});

test("generates frozen tagged Markdown only after matching all manifest and root-lock versions", async () => {
  const root = await fixture(`${heading}\n\nReviewed change. [Guide](../../readme.md#install)\n`);
  try {
    await writeFile(join(root, "readme.md"), "# Guide\n");
    expect((await rendered(await generateReleaseBody(root, tag, repository))).links).toContainEqual({
      href: `https://github.com/${repository}/blob/${tag}/readme.md#install`, title: null,
    });
    for (const [path, badContent] of [
      ["package.json", '{"version":"0.1.10"}'],
      ["native/Cargo.toml", '[package]\nname = "herdr-desktop-pet"\nversion = "0.1.10"'],
      ["herdr-plugin.toml", 'version = "0.1.10"'],
      ["native/Cargo.lock", 'version = 4\n[[package]]\nname = "herdr-desktop-pet"\nversion = "0.1.10"'],
    ]) {
      const previous = await readFile(join(root, path!), "utf8");
      await writeFile(join(root, path!), badContent!);
      await expect(generateReleaseBody(root, tag, repository)).rejects.toThrow("must match");
      await writeFile(join(root, path!), previous);
    }
    await writeFile(join(root, "native/Cargo.lock"), 'version = 4\n[[package]]\nname = "different-dependency"\nversion = "0.1.11"');
    await expect(generateReleaseBody(root, tag, repository)).rejects.toThrow("root crate");
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("mixed Release references keep original source links and image-only raw URLs across selector forms", async () => {
  const root = await fixture([
    heading, "", "Reviewed artwork.",
    "[![Portrait][ART]][art] ![Other][art] ![Art][] ![Art] [Art][] [Art]",
    "",
    "[Art]:\r\n  <../../assets/a%29.png>\r\n\"Line one\r\nLine two\r\nLine three\r\nLine four\"\r\n",
    "[Art]: ../../assets/duplicate.png\r\n",
    "`![code][Art]` \\[escaped][Art] [Absent][missing]\r\n",
  ].join("\r\n"));
  try {
    await mkdir(join(root, "assets"));
    await writeFile(join(root, "assets/a).png"), "fixture");
    const { links, images } = await rendered(await generateReleaseBody(root, tag, repository));
    const blob = `https://github.com/${repository}/blob/${tag}/assets/a%29.png`;
    const raw = `https://raw.githubusercontent.com/${repository}/${tag}/assets/a%29.png`;
    const title = "Line one\nLine two\nLine three\nLine four";
    expect(links).toContainEqual({ href: blob, title });
    expect(links.filter((link) => link.href === blob).every((link) => link.title === title)).toBe(true);
    expect(images.map((image) => image.src)).toEqual([raw, raw, raw, raw]);
    expect(images.map((image) => image.title)).toEqual([title, title, title, title]);
    expect(images.map((image) => image.alt)).toEqual(["Portrait", "Other", "Art", "Art"]);
    expect(images[0]?.parent).toBe(blob);
    expect(images.slice(1).every((image) => image.parent === null)).toBe(true);
    expect(links.every((link) => link.href !== null && !link.href.includes("duplicate"))).toBe(true);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("rejected definition prose links resolve, and missing or unsafe targets fail preflight", async () => {
  const root = await fixture();
  const path = join(root, `docs/releases/${tag}.md`);
  try {
    await mkdir(join(root, "guides"));
    for (const [index, prose] of [
      '[unused]: https://example.test "Title" trailing [Guide](TARGET)',
      '[unused]: https://example.test (bad ( [Guide](TARGET)',
      '[ ]: https://example.test "[Guide](TARGET)"',
      '[a[b]: https://example.test "[Guide](TARGET)"',
      '[label]: <https://example.test>"[Guide](TARGET)"',
    ].entries()) {
      const guide = `guides/prose-${index}.md`;
      await writeFile(join(root, guide), "# Guide\n");
      await writeFile(path, `${heading}\n\nReviewed guidance.\n\n${prose.replace("TARGET", `../../${guide}`)}\n`);
      const { links } = await rendered(await generateReleaseBody(root, tag, repository));
      expect(links).toContainEqual({ href: `https://github.com/${repository}/blob/${tag}/${guide}`, title: null });
      for (const target of ["../../missing.md", "../../../outside.md"]) {
        await writeFile(path, `${heading}\n\nReviewed guidance.\n\n${prose.replace("TARGET", target)}\n`);
        await expect(generateReleaseBody(root, tag, repository)).rejects.toThrow();
      }
    }
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("invalid inline parents expose real nested Guide links and image references to release preflight", async () => {
  const root = await fixture();
  const path = join(root, `docs/releases/${tag}.md`);
  const guide = `https://github.com/${repository}/blob/${tag}/readme.md`;
  const rawArt = `https://raw.githubusercontent.com/${repository}/${tag}/assets/art.png`;
  try {
    await writeFile(join(root, "readme.md"), "# Guide\n");
    await mkdir(join(root, "assets"));
    await writeFile(join(root, "assets/art.png"), "art");
    for (const prose of [
      "[bad](https://example.test (bad ( [Guide](../../readme.md)))",
      '[outer [Guide](../../readme.md)](https://example.test "![Art]")\n\n[Art]: ../../assets/art.png',
    ]) {
      await writeFile(path, `${heading}\n\nReviewed guidance. ${prose}\n`);
      const { links, images } = await rendered(await generateReleaseBody(root, tag, repository));
      expect(links).toEqual([{ href: guide, title: null }]);
      if (prose.startsWith("[outer")) {
        expect(images).toEqual([{ src: rawArt, title: null, alt: "Art", parent: null }]);
      } else {
        expect(images).toHaveLength(0);
      }
      for (const target of ["../../missing.md", "../../../outside.md"]) {
        await writeFile(path, `${heading}\n\nReviewed guidance. ${prose.replace("../../readme.md", target)}\n`);
        await expect(generateReleaseBody(root, tag, repository)).rejects.toThrow(
          target.includes("missing") ? "missing repository path" : "escapes repository",
        );
      }
    }
    for (const target of ["../../missing.png", "../../../outside.png"]) {
      const prose = '[outer [Guide](../../readme.md)](https://example.test "![Art]")\n\n[Art]: ' + target;
      await writeFile(path, `${heading}\n\nReviewed guidance. ${prose}\n`);
      await expect(generateReleaseBody(root, tag, repository)).rejects.toThrow(
        target.includes("missing") ? "missing repository path" : "escapes repository",
      );
    }
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("defined inner references invalidate an outer link, but unresolved bracket labels do not", async () => {
  const root = await fixture();
  const path = join(root, `docs/releases/${tag}.md`);
  try {
    await writeFile(join(root, "readme.md"), "# Guide\n");
    await mkdir(join(root, "assets"));
    await writeFile(join(root, "assets/art.png"), "art");
    await writeFile(path, `${heading}\n\nReviewed guidance. [outer [Guide][g]](https://example.test "Title")\n\n[g]: ../../readme.md\n`);
    expect((await rendered(await generateReleaseBody(root, tag, repository))).links).toEqual([
      { href: `https://github.com/${repository}/blob/${tag}/readme.md`, title: null },
    ]);
    for (const target of ["../../missing.md", "../../../outside.md"]) {
      await writeFile(path, `${heading}\n\nReviewed guidance. [outer [Guide][g]](https://example.test "Title")\n\n[g]: ${target}\n`);
      await expect(generateReleaseBody(root, tag, repository)).rejects.toThrow(
        target.includes("missing") ? "missing repository path" : "escapes repository",
      );
    }
    for (const label of ["[Guide][g]", "[Guide]"]) {
      await writeFile(path, `${heading}\n\nReviewed guidance. [outer ${label}](https://example.test "![Art]")\n\n[Art]: ../../assets/art.png\n`);
      const { links, images } = await rendered(await generateReleaseBody(root, tag, repository));
      expect(links).toEqual([{ href: "https://example.test", title: "![Art]" }]);
      expect(images).toHaveLength(0);
    }
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("literal image-like text inside external inline URLs and titles never claims mixed references", async () => {
  const root = await fixture([
    heading, "", "Reviewed artwork.",
    '[Website](https://example.test/![Art]) [Named](https://example.test "literal ![Art]") [Compact](<https://example.test/no-gap>"literal ![Art]")',
    "[![Portrait][Art]][Art] ![Art][Art] [Art]",
    "",
    "[Art]: ../../assets/art.png",
  ].join("\n"));
  try {
    await mkdir(join(root, "assets"));
    await writeFile(join(root, "assets/art.png"), "art");
    const { links, images } = await rendered(await generateReleaseBody(root, tag, repository));
    const blob = `https://github.com/${repository}/blob/${tag}/assets/art.png`;
    const raw = `https://raw.githubusercontent.com/${repository}/${tag}/assets/art.png`;
    expect(links).toContainEqual({ href: "https://example.test/!%5BArt%5D", title: null });
    expect(links).toContainEqual({ href: "https://example.test", title: "literal ![Art]" });
    expect(links).toContainEqual({ href: "https://example.test/no-gap", title: "literal ![Art]" });
    expect(links).toContainEqual({ href: blob, title: null });
    expect(images).toContainEqual({ src: raw, title: null, alt: "Portrait", parent: blob });
    expect(images).toContainEqual({ src: raw, title: null, alt: "Art", parent: null });
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("image-only references use raw asset URLs without source links", async () => {
  const root = await fixture(`${heading}\n\nReviewed artwork. ![Portrait][Art]\n\n[Art]: ../../assets/art.png "Portrait"\n`);
  try {
    await mkdir(join(root, "assets"));
    await writeFile(join(root, "assets/art.png"), "art");
    const { links, images } = await rendered(await generateReleaseBody(root, tag, repository));
    expect(links).toHaveLength(0);
    expect(images).toEqual([{
      src: `https://raw.githubusercontent.com/${repository}/${tag}/assets/art.png`,
      title: "Portrait", alt: "Portrait", parent: null,
    }]);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("escaped title delimiters remain attached to both mixed reference modes", async () => {
  const root = await fixture(`${heading}\n\nReviewed image. ![Art][Art] [Art]\n\n[Art]: ../../assets/art.png "Named \\"portrait\\""\n`);
  try {
    await mkdir(join(root, "assets"));
    await writeFile(join(root, "assets/art.png"), "art");
    const source = await rendered(await readFile(join(root, `docs/releases/${tag}.md`), "utf8"));
    const sourceTitle = source.links[0]?.title;
    if (!sourceTitle) throw new Error("Expected rendered source title");
    const { links, images } = await rendered(await generateReleaseBody(root, tag, repository));
    expect(links).toContainEqual({ href: `https://github.com/${repository}/blob/${tag}/assets/art.png`, title: sourceTitle });
    expect(images).toContainEqual({
      src: `https://raw.githubusercontent.com/${repository}/${tag}/assets/art.png`, title: sourceTitle, alt: "Art", parent: null,
    });
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("an unmatched or blank-separated reference title never hides later links", async () => {
  const root = await fixture();
  try {
    await mkdir(join(root, "assets"));
    await writeFile(join(root, "assets/art.png"), "art");
    await writeFile(join(root, "readme.md"), "# Guide\n");
    for (const middle of ['"Unclosed title\n\n', '\n', '\n"Blank-separated title"\n']) {
      const body = `${heading}\n\nReviewed artwork. ![Art][Art] [Art]\n\n[Art]: ../../assets/art.png\n${middle}[Guide](../../readme.md)\n`;
      await writeFile(join(root, `docs/releases/${tag}.md`), body);
      const { links } = await rendered(await generateReleaseBody(root, tag, repository));
      expect(links).toContainEqual({ href: `https://github.com/${repository}/blob/${tag}/readme.md`, title: null });
    }
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("synthetic image references cannot claim defined or unresolved labels", async () => {
  const root = await fixture(`${heading}\n\nReviewed artwork. ![Art][Art] [Art]\n\n[Art]: ../../assets/art.png\n`);
  try {
    await mkdir(join(root, "assets"));
    await writeFile(join(root, "assets/art.png"), "art");
    await writeFile(join(root, "assets/reserved.png"), "reserved");
    const initial = await generateReleaseBody(root, tag, repository);
    const candidate = /^\[([^\]\r\n]+)\]: https:\/\/raw\.githubusercontent\.com\//m.exec(initial)?.[1];
    if (!candidate) throw new Error("Expected mixed image reference in generated body");
    const base = `${heading}\n\nReviewed artwork. ![Art][Art] [Art]\n\n[Art]: ../../assets/art.png\n`;
    for (const suffix of [
      `\n![Reserved][${candidate}]\n\n[${candidate}]: ../../assets/reserved.png\n`,
      `\n[${candidate}]\n`,
    ]) {
      await writeFile(join(root, `docs/releases/${tag}.md`), base + suffix);
      const { links, images } = await rendered(await generateReleaseBody(root, tag, repository));
      expect(links).toContainEqual({ href: `https://github.com/${repository}/blob/${tag}/assets/art.png`, title: null });
      expect(images).toContainEqual({ src: `https://raw.githubusercontent.com/${repository}/${tag}/assets/art.png`, title: null, alt: "Art", parent: null });
      if (suffix.includes("Reserved")) {
        expect(images).toContainEqual({ src: `https://raw.githubusercontent.com/${repository}/${tag}/assets/reserved.png`, title: null, alt: "Reserved", parent: null });
      } else {
        expect(images).toHaveLength(1);
        expect(links).toHaveLength(1);
      }
    }
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("rejects missing, wrong-heading, empty, placeholder, and broken-link notes without a fallback body", async () => {
  const root = await fixture();
  const path = join(root, `docs/releases/${tag}.md`);
  try {
    for (const note of ["# Wrong title\n\nReviewed change.", `${heading}\n`, `${heading}\n\n## Summary\n<!-- pending -->`, `${heading}\n\nTODO`, `${heading}\n\n## Summary\nTODO\n## Fixes\nTBD`]) {
      await writeFile(path, note);
      await expect(generateReleaseBody(root, tag, repository)).rejects.toThrow();
    }
    await writeFile(path, `${heading}\n\nSee [missing](../../missing.md).`);
    await expect(generateReleaseBody(root, tag, repository)).rejects.toThrow("missing repository path");
    await rm(path);
    await expect(generateReleaseBody(root, tag, repository)).rejects.toThrow();
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("CLI writes a usable body and keeps a previous output untouched on failed preflight", async () => {
  const root = await fixture(`${heading}\n\nReviewed guidance. [Guide](../../readme.md)\n`);
  const output = join(root, "generated", "release.md");
  const command = [process.execPath, join(import.meta.dir, "release-docs.ts"), "check", tag, "--root", root, "--repository", repository, "--output", output];
  try {
    await writeFile(join(root, "readme.md"), "# Guide\n");
    const success = Bun.spawn(command, { stdout: "pipe", stderr: "pipe" });
    expect(await success.exited).toBe(0);
    const previous = await readFile(output);
    expect((await rendered(previous.toString())).links).toContainEqual({
      href: `https://github.com/${repository}/blob/${tag}/readme.md`, title: null,
    });
    const previousMtime = (await stat(output, { bigint: true })).mtimeNs;
    for (const prose of [
      "[bad](https://example.test (bad ( [Guide](../../missing.md)))",
      '[outer [Guide](../../missing.md)](https://example.test "![Art]")\n\n[Art]: ../../assets/art.png',
    ]) {
      await writeFile(join(root, `docs/releases/${tag}.md`), `${heading}\n\nReviewed guidance. ${prose}\n`);
      const failure = Bun.spawn(command, { stdout: "pipe", stderr: "pipe" });
      expect(await failure.exited).toBe(1);
      expect(await new Response(failure.stderr).text()).toContain("missing repository path");
      expect(await readFile(output)).toEqual(previous);
      expect((await stat(output, { bigint: true })).mtimeNs).toBe(previousMtime);
    }
  } finally { await rm(root, { recursive: true, force: true }); }
});
