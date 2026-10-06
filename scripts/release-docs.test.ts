import { expect, spyOn, test } from "bun:test";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
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

test("rewrites decoded repository paths and images while preserving titles, fragments, and queries", () => {
  const seen: [string, string, boolean][] = [];
  const input = '[guide](../../readme.md#install "Read it")\n![art](../../assets/my%20art.png)\n[root](/docs/releases/policy.md)\n[query](../migrations/upgrade.md?plain=1#safe)';
  const output = rewriteRelativeLinks(input, "docs/releases/v0.1.11.md", (path, fragment, image) => {
    seen.push([path, fragment, image]);
    return `https://example.test/${path}${fragment}`;
  });
  expect(seen).toEqual([
    ["readme.md", "#install", false], ["assets/my art.png", "", true],
    ["docs/releases/policy.md", "", false], ["docs/migrations/upgrade.md", "#safe", false],
  ]);
  expect(output).toContain('[guide](https://example.test/readme.md#install "Read it")');
  expect(output).toContain('[query](https://example.test/docs/migrations/upgrade.md?plain=1#safe)');
});

test("preserves external and same-page links, fenced and inline code, and escaped link syntax", () => {
  const input = [
    '[web](https://example.test/x) [mail](mailto:author@example.test) [local](#heading) [cdn](//example.test/a)',
    '`[literal](../../readme.md)` and ``x `[literal](../../readme.md)` x``',
    '```md', '[literal](../../readme.md)', '```',
    '~~~', '[literal](../../readme.md)', '~~~',
    '\\[not a link](../../readme.md)',
  ].join("\n");
  expect(rewriteRelativeLinks(input, "docs/releases/v0.1.11.md", () => { throw new Error("Must not resolve literal code or external links"); })).toBe(input);
});

test("handles nested parentheses, angle destinations, nested image links, and reference images", () => {
  const input = [
    '[nested](../../guides/install(mac).md)',
    '[spaced](<../../guides/install notes.md> "title")',
    '[![art](../../assets/a.png)](../../readme.md)',
    '![Portrait][Art] [Guide][] [shortcut]',
    '[Art]: ../../assets/portrait.png "Portrait title"',
    '[Guide]: ../../readme.md#install',
    '[shortcut]: ../migrations/upgrade.md',
  ].join("\n");
  const seen: [string, boolean][] = [];
  const output = rewriteRelativeLinks(input, "docs/releases/v0.1.11.md", (path, fragment, image) => {
    seen.push([path, image]);
    return `https://example.test/${encodeURI(path)}${fragment}`;
  });
  expect(output).toContain('[nested](https://example.test/guides/install(mac).md)');
  expect(output).toContain('[spaced](<https://example.test/guides/install%20notes.md> "title")');
  expect(output).toContain('[![art](https://example.test/assets/a.png)](https://example.test/readme.md)');
  expect(output).toContain('[Art]: https://example.test/assets/portrait.png "Portrait title"');
  expect(seen).toContainEqual(["assets/portrait.png", true]);
  expect(seen).toContainEqual(["readme.md", false]);
  expect(seen).toContainEqual(["docs/migrations/upgrade.md", false]);
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
    expect(await generateReleaseBody(root, tag, repository)).toContain(`https://github.com/${repository}/blob/${tag}/readme.md#install`);
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

test("release images serve raw bytes at the exact tag while source links remain tagged blob URLs", async () => {
  const root = await fixture([
    heading, "",
    'Reviewed artwork. [![Portrait](../../assets/my%20portrait.png "Portrait")](../../readme.md#art)',
    '![Reference portrait][portrait]',
    '[Image source](../../assets/my%20portrait.png)',
    '[portrait]: ../../assets/my%20portrait.png "Reference portrait"',
  ].join("\n"));
  try {
    await mkdir(join(root, "assets"));
    await writeFile(join(root, "assets/my portrait.png"), new Uint8Array([137, 80, 78, 71]));
    await writeFile(join(root, "readme.md"), "# Art\n");
    const body = await generateReleaseBody(root, tag, repository);
    const imageUrl = `https://raw.githubusercontent.com/${repository}/${tag}/assets/my%20portrait.png`;
    expect(body).toContain(`[![Portrait](${imageUrl} "Portrait")](https://github.com/${repository}/blob/${tag}/readme.md#art)`);
    expect(body).toContain(`[portrait]: ${imageUrl} "Reference portrait"`);
    expect(body).toContain(`[Image source](https://github.com/${repository}/blob/${tag}/assets/my%20portrait.png)`);
    await rm(join(root, "assets/my portrait.png"));
    await expect(generateReleaseBody(root, tag, repository)).rejects.toThrow("missing repository path");
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
  const root = await fixture();
  const output = join(root, "generated", "release.md");
  const command = [process.execPath, join(import.meta.dir, "release-docs.ts"), "check", tag, "--root", root, "--repository", repository, "--output", output];
  try {
    const success = Bun.spawn(command, { stdout: "pipe", stderr: "pipe" });
    expect(await success.exited).toBe(0);
    expect(await readFile(output, "utf8")).toContain("Reviewed native-menu improvement.");
    await writeFile(output, "previous successful body");
    await writeFile(join(root, `docs/releases/${tag}.md`), `${heading}\n\nTBD`);
    const failure = Bun.spawn(command, { stdout: "pipe", stderr: "pipe" });
    expect(await failure.exited).toBe(1);
    expect(await readFile(output, "utf8")).toBe("previous successful body");
  } finally { await rm(root, { recursive: true, force: true }); }
});
