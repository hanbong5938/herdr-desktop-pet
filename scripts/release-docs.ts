import { definition } from "micromark-core-commonmark";
import { fromMarkdown } from "mdast-util-from-markdown";
import { normalizeIdentifier } from "micromark-util-normalize-identifier";
import type { Nodes, Root } from "mdast";
import { mkdir, readFile, rename, rm, stat, writeFile } from "node:fs/promises";
import { dirname, join, posix, resolve } from "node:path";

export type ReleaseRecord = {
  tag_name: string;
  html_url: string;
  published_at: string | null;
  draft: boolean;
  prerelease: boolean;
  assets: { name: string; size: number; state?: string }[];
};

export function parseStableTag(tag: string): { version: string; major: number; minor: number; patch: number } {
  const match = /^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/.exec(tag);
  if (!match || match[0] !== tag) throw new Error(`Expected a stable vMAJOR.MINOR.PATCH tag without leading zeros: ${tag}`);
  const [major, minor, patch] = match.slice(1).map(Number);
  if (![major, minor, patch].every(Number.isSafeInteger)) throw new Error(`Tag version exceeds safe integer range: ${tag}`);
  return { version: tag.slice(1), major: major!, minor: minor!, patch: patch! };
}

export function requiredAssetNames(tag: string): string[] {
  const { version } = parseStableTag(tag);
  const archive = `HerdrDesktopPet-v${version}-macos-arm64.tar.gz`;
  return [archive, `${archive}.sha256`, "SHA256SUMS"];
}

export function isPublishedRelease(record: ReleaseRecord): boolean {
  let required: string[];
  try { required = requiredAssetNames(record.tag_name); } catch { return false; }
  if (record.draft || record.prerelease || !record.published_at || !Number.isFinite(Date.parse(record.published_at))) return false;
  return required.every((name) => record.assets.some((asset) =>
    asset.name === name && Number.isFinite(asset.size) && asset.size > 0 &&
    asset.state === "uploaded"));
}

function requireRepository(repository: string): void {
  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]*\/[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(repository) || /[\r\n]/.test(repository)) {
    throw new Error(`Invalid GitHub repository: ${repository}`);
  }
}

function isReleaseRecord(value: unknown): value is ReleaseRecord {
  if (!value || typeof value !== "object") return false;
  const record = value as ReleaseRecord;
  return typeof record.tag_name === "string" && typeof record.html_url === "string" &&
    (record.published_at === null || typeof record.published_at === "string") &&
    typeof record.draft === "boolean" && typeof record.prerelease === "boolean" &&
    Array.isArray(record.assets) && record.assets.every((asset) => asset &&
      typeof asset.name === "string" && typeof asset.size === "number" &&
      (asset.state === undefined || typeof asset.state === "string"));
}

export async function fetchReleases(repository: string, token?: string): Promise<ReleaseRecord[]> {
  requireRepository(repository);
  const headers: Record<string, string> = {
    Accept: "application/vnd.github+json",
    "X-GitHub-Api-Version": "2022-11-28",
  };
  if (token) headers.Authorization = `Bearer ${token}`;
  const releases: ReleaseRecord[] = [];
  for (let page = 1; ; page++) {
    const response = await fetch(`https://api.github.com/repos/${repository}/releases?per_page=100&page=${page}`, { headers });
    if (!response.ok) throw new Error(`GitHub release catalog request failed (HTTP ${response.status}, page ${page})`);
    const data: unknown = await response.json();
    if (!Array.isArray(data) || !data.every(isReleaseRecord)) throw new Error(`Invalid GitHub release catalog response on page ${page}`);
    releases.push(...data);
    if (data.length < 100) return releases;
  }
}

function escaped(text: string, index: number): boolean {
  let count = 0;
  for (let i = index - 1; i >= 0 && text[i] === "\\"; i--) count++;
  return count % 2 !== 0;
}

type Span = { start: number; end: number };
type DefinitionSpan = Span & {
  label: Span;
  destination: Span;
  prefixes: Span[];
  title: string | null | undefined;
  url: string;
};

// The optional title is an attempted micromark construct. Reject invalid
// parenthesized titles inside that attempt, so micromark can keep a preceding
// destination-only definition when the invalid title starts on the next line.
const guardedDefinition: typeof definition = {
  name: "herdrCommonmarkDefinition",
  tokenize(effects, ok, nok) {
    const context = this;
    return definition.tokenize.call(context, {
      ...effects,
      attempt(construct, success, failure) {
        // The definition grammar has one attempt: its optional title.
        const titleConstruct = construct as typeof definition;
        return effects.attempt({
          ...titleConstruct,
          tokenize(titleEffects, titleOk, titleNok) {
            const from = context.events.length;
            return titleConstruct.tokenize.call(context, titleEffects, (code) => {
              for (let index = from; index < context.events.length; index++) {
                const [event, token] = context.events[index]!;
                if (event !== "exit" || token.type !== "definitionTitle") continue;
                const title = context.sliceSerialize(token);
                if (title[0] !== "(") continue;
                for (let at = 1; at < title.length - 1; at++) {
                  if (title[at] === "(" && !escaped(title, at)) return titleNok(code);
                }
              }
              return titleOk(code);
            }, titleNok);
          },
        }, success, failure);
      },
    }, ok, nok);
  },
};

function markdownStructure(markdown: string): { tree: Root; definitions: DefinitionSpan[]; mask: Uint8Array } {
  const definitions: DefinitionSpan[] = [];
  const mask = new Uint8Array(markdown.length);
  const prefixTokens: { type: string; span: Span }[] = [];
  let active: { span: Span; label?: Span; destination?: Span; literal?: Span } | undefined;
  const span = (token: { type: string; start: { offset?: number }; end: { offset?: number } }): Span => {
    const start = token.start.offset;
    const end = token.end.offset;
    if (start === undefined || end === undefined || start < 0 || end < start || end > markdown.length) {
      throw new Error(`Invalid Markdown token span: ${token.type}`);
    }
    return { start, end };
  };
  const tree = fromMarkdown(markdown, {
    extensions: [{ disable: { null: ["definition"] }, contentInitial: { 91: guardedDefinition } }],
    mdastExtensions: [{
      beforeEnter(token) {
        if (token.type === "definition") {
          if (active) throw new Error("Nested Markdown definition tokens");
          active = { span: span(token) };
        }
      },
      afterExit(token) {
        const part = span(token);
        if (["blockQuotePrefix", "listItemIndent", "linePrefix"].includes(token.type)) prefixTokens.push({ type: token.type, span: part });
        if (active) {
          if (token.type === "definitionLabelString") active.label = part;
          if (token.type === "definitionDestinationString") active.destination = part;
          if (token.type === "definitionDestinationLiteral") active.literal = part;
          if (token.type === "definition") {
            const { span: whole, label, destination, literal } = active;
            if (!label || (!destination && !literal) || part.start !== whole.start || part.end !== whole.end) {
              throw new Error("Incomplete Markdown definition token spans");
            }
            const url = destination ?? { start: literal!.start + 1, end: literal!.start + 1 };
            if (label.start < whole.start || label.end > whole.end || url.start < whole.start || url.end > whole.end ||
              (literal && markdown.slice(literal.start, literal.end) !== "<>" && !destination)) {
              throw new Error("Ambiguous Markdown definition token spans");
            }
            definitions.push({ ...whole, label, destination: url, prefixes: [], title: undefined, url: "" });
            mask.fill(1, whole.start, whole.end);
            active = undefined;
          }
        }
      },
    }],
  });
  definitions.sort((left, right) => left.start - right.start);
  const definitionsByStart = new Map(definitions.map((definition) => [definition.start, definition]));
  const visit = (node: Nodes | Root): void => {
    if (node.type === "code" || node.type === "inlineCode") {
      if (!node.position) throw new Error("Missing Markdown code position");
      const { start, end } = span({ type: node.type, start: node.position.start, end: node.position.end });
      mask.fill(1, start, end);
    }
    if (node.type === "definition") {
      const match = definitionsByStart.get(node.position?.start.offset ?? -1);
      if (!match) throw new Error("Markdown definition tree/token mismatch");
      match.title = node.title;
      match.url = node.url;
    }
    if ("children" in node) for (const child of node.children) visit(child);
  };
  visit(tree);
  if (definitions.some((definition) => definition.title === undefined)) throw new Error("Unmatched Markdown definition tokens");
  for (const definition of definitions) {
    definition.prefixes = prefixTokens.filter(({ type, span: part }) => {
      if (part.start <= definition.start || part.end > definition.end) return false;
      if (type !== "linePrefix") return true;
      const lineStart = markdown.lastIndexOf("\n", part.start - 1) + 1;
      return prefixTokens.some(({ type: other, span }) => other === "blockQuotePrefix" && span.start === part.end && part.start >= lineStart);
    }).map(({ span }) => span);
  }
  return { tree, definitions, mask };
}

type Destination = { start: number; end: number; after: number };
function destinationAt(text: string, start: number): Destination | undefined {
  while (/\s/.test(text[start] ?? "") && start < text.length) start++;
  if (text[start] === "<") {
    const end = text.indexOf(">", start + 1);
    if (end < 0 || /[\r\n]/.test(text.slice(start, end))) return undefined;
    return { start: start + 1, end, after: end + 1 };
  }
  let depth = 0;
  let end = start;
  for (; end < text.length; end++) {
    const character = text[end]!;
    if (escaped(text, end)) continue;
    if (character === "(") depth++;
    else if (character === ")") {
      if (depth === 0) break;
      depth--;
    } else if (/\s/.test(character)) break;
  }
  if (end === start || depth !== 0) return undefined;
  return { start, end, after: end };
}

function inlineWhitespaceEnd(text: string, start: number): number {
  let at = start;
  let seenNewline = false;
  while (at < text.length && /\s/.test(text[at]!)) {
    if (text[at] === "\n") {
      if (seenNewline) return -1;
      seenNewline = true;
    }
    at++;
  }
  return at;
}

// Recognize an entire inline suffix before masking it or resolving its target.
// Try a bare destination first, including quote-prefixed names. Only if that
// cannot complete the suffix may a quoted title follow an empty destination.
function inlineTailAt(text: string, open: number): { destination: Destination; end: number } | undefined {
  if (text[open] !== "(") return undefined;
  const destinationStart = inlineWhitespaceEnd(text, open + 1);
  if (destinationStart < 0) return undefined;
  const suffix = (destination: Destination): { destination: Destination; end: number } | undefined => {
    let close = inlineWhitespaceEnd(text, destination.after);
    if (close < 0) return undefined;
    if (["\"", "'", "("].includes(text[close] ?? "")) {
      // A bare destination requires separating whitespace; <angle> destinations
      // may be followed immediately by a title.
      if (close === destination.after && text[destination.start - 1] !== "<" && destination.start !== destination.end) return undefined;
      const delimiter = text[close]!;
      const closing = delimiter === "(" ? ")" : delimiter;
      let titleEnd = close + 1;
      for (; titleEnd < text.length; titleEnd++) {
        if (text[titleEnd] === "\n") {
          let previous = titleEnd - 1;
          while (previous > close && /[ \t\r]/.test(text[previous]!)) previous--;
          if (text[previous] === "\n") return undefined;
        }
        if (escaped(text, titleEnd)) continue;
        if (delimiter === "(" && text[titleEnd] === "(") return undefined;
        if (text[titleEnd] === closing) break;
      }
      if (titleEnd === text.length) return undefined;
      close = inlineWhitespaceEnd(text, titleEnd + 1);
    }
    return close >= 0 && text[close] === ")" ? { destination, end: close + 1 } : undefined;
  };
  if (text[destinationStart] === ")") {
    return suffix({ start: destinationStart, end: destinationStart, after: destinationStart });
  }
  const bare = destinationAt(text, destinationStart);
  const completed = bare && suffix(bare);
  if (completed) return completed;
  if (destinationStart > open + 1 && ["\"", "'"].includes(text[destinationStart] ?? "")) {
    return suffix({ start: destinationStart, end: destinationStart, after: destinationStart });
  }
  return undefined;
}

function labelEnd(text: string, start: number, mask: Uint8Array): number {
  let depth = 1;
  for (let i = start + 1; i < text.length; i++) {
    if (mask[i] || escaped(text, i)) continue;
    if (text[i] === "[") depth++;
    if (text[i] === "]" && --depth === 0) return i;
  }
  return -1;
}

function referenceId(label: string): string {
  return normalizeIdentifier(label);
}

/** Resolve relative Markdown destinations; fragment includes its leading #. */
export function rewriteRelativeLinks(
  markdown: string,
  sourcePath: string,
  resolveLink: (repoPath: string, fragment: string, isImage: boolean) => string,
): string {
  const normalizedSource = posix.normalize(sourcePath);
  if (posix.isAbsolute(sourcePath) || normalizedSource === ".." || normalizedSource.startsWith("../") || sourcePath.includes("\\")) {
    throw new Error(`Source path must be repository-relative: ${sourcePath}`);
  }
  const { tree, definitions, mask } = markdownStructure(markdown);
  const changes: { start: number; end: number; value: string }[] = [];
  type Reference = DefinitionSpan & { images: Span[]; link: boolean };
  const references = new Map<string, Reference>();
  const reserved = new Set<string>();
  for (const definition of definitions) {
    const id = referenceId(markdown.slice(definition.label.start, definition.label.end));
    reserved.add(id);
    if (!references.has(id)) references.set(id, { ...definition, images: [], link: false });
  }
  const resolveDestination = (destination: Destination, isImage: boolean): string | undefined => {
    const raw = markdown.slice(destination.start, destination.end);
    const target = raw.replace(/\\([^\w\s])/g, "$1").replace(/&amp;/g, "&");
    if (!target || target.startsWith("#") || target.startsWith("//") || /^[A-Za-z][A-Za-z0-9+.-]*:/.test(target)) return undefined;
    const hash = target.indexOf("#");
    const fragment = hash < 0 ? "" : target.slice(hash);
    const beforeHash = hash < 0 ? target : target.slice(0, hash);
    const question = beforeHash.indexOf("?");
    const query = question < 0 ? "" : beforeHash.slice(question);
    const encodedPath = question < 0 ? beforeHash : beforeHash.slice(0, question);
    let path: string;
    try { path = decodeURIComponent(encodedPath); } catch { throw new Error(`Invalid percent encoding in Markdown link: ${raw}`); }
    if (path.includes("\\") || /[\u0000-\u001f\u007f]/.test(path)) throw new Error(`Invalid repository link path: ${raw}`);
    const repoPath = path.startsWith("/") ? posix.normalize(path.slice(1)) : posix.normalize(posix.join(posix.dirname(normalizedSource), path));
    if (repoPath === ".." || repoPath.startsWith("../") || posix.isAbsolute(repoPath)) throw new Error(`Markdown link escapes repository: ${raw}`);
    const resolved = resolveLink(repoPath, fragment, isImage);
    const resolvedHash = resolved.indexOf("#");
    const value = query ? (resolvedHash < 0 ? resolved + query : resolved.slice(0, resolvedHash) + query + resolved.slice(resolvedHash)) : resolved;
    return value.replace(/\(/g, "%28").replace(/\)/g, "%29");
  };
  const imageReferenceEnd = (open: number, close: number, limit: number): number | undefined => {
    const inline = inlineTailAt(markdown, close + 1);
    if (inline && inline.end <= limit) return inline.end;
    if (markdown[close + 1] === "[") {
      const selectorEnd = labelEnd(markdown, close + 1, mask);
      if (selectorEnd < 0 || selectorEnd >= limit) return undefined;
      const selector = markdown.slice(close + 2, selectorEnd) || markdown.slice(open + 1, close);
      return references.has(referenceId(selector)) ? selectorEnd + 1 : undefined;
    }
    return references.has(referenceId(markdown.slice(open + 1, close))) ? close + 1 : undefined;
  };
  // Check only rendered anchors in a prospective link label. Images can sit
  // inside links, but links written in image alt text are not nested anchors.
  const containsInnerLink = (start: number, end: number): boolean => {
    for (let at = start; at < end; at++) {
      if (mask[at] || markdown[at] !== "[" || escaped(markdown, at)) continue;
      const labelClose = labelEnd(markdown, at, mask);
      if (labelClose < 0 || labelClose >= end) continue;
      const image = at > 0 && markdown[at - 1] === "!" && !escaped(markdown, at - 1);
      const tail = inlineTailAt(markdown, labelClose + 1);
      if (image) {
        const imageEnd = imageReferenceEnd(at, labelClose, end);
        if (imageEnd !== undefined) {
          at = imageEnd - 1;
          continue;
        }
      }
      if (tail && tail.end <= end) return true;
      if (markdown[labelClose + 1] === "(") continue;
      if (markdown[labelClose + 1] === "[") {
        const selectorEnd = labelEnd(markdown, labelClose + 1, mask);
        if (selectorEnd >= 0 && selectorEnd < end) {
          const selector = markdown.slice(labelClose + 2, selectorEnd);
          if (references.has(referenceId(selector || markdown.slice(at + 1, labelClose)))) return true;
          at = selectorEnd;
          continue;
        }
      } else if (references.has(referenceId(markdown.slice(at + 1, labelClose)))) {
        return true;
      }
    }
    return false;
  };
  const consumedSelectors = new Set<number>();
  for (let i = 0; i < markdown.length; i++) {
    if (mask[i] || markdown[i] !== "[" || escaped(markdown, i) || consumedSelectors.has(i)) continue;
    const end = labelEnd(markdown, i, mask);
    if (end < 0) continue;
    const isImage = i > 0 && markdown[i - 1] === "!" && !escaped(markdown, i - 1);
    const text = markdown.slice(i + 1, end);
    reserved.add(referenceId(text));
    if (markdown[end + 1] === "(") {
      const tail = inlineTailAt(markdown, end + 1);
      if (tail && (isImage || !containsInnerLink(i + 1, end))) {
        // Only a committed suffix is masked. The label remains open for
        // nested images, including image-in-link reference selectors.
        mask.fill(1, end + 1, tail.end);
        const value = resolveDestination(tail.destination, isImage);
        if (value !== undefined) changes.push({ start: tail.destination.start, end: tail.destination.end, value });
      }
    } else {
      let label = text;
      let selector = { start: end + 1, end: end + 1 };
      if (markdown[end + 1] === "[") {
        const referenceEnd = labelEnd(markdown, end + 1, mask);
        if (referenceEnd >= 0) {
          consumedSelectors.add(end + 1);
          label = markdown.slice(end + 2, referenceEnd) || text;
          selector = { start: end + 1, end: referenceEnd + 1 };
          reserved.add(referenceId(markdown.slice(end + 2, referenceEnd)));
        }
      }
      const reference = references.get(referenceId(label));
      if (reference) {
        if (isImage) reference.images.push(selector);
        else reference.link = true;
      }
    }
    // Scan inside link labels, but a genuine image's ALT is not rendered Markdown.
    if (isImage && imageReferenceEnd(i, end, markdown.length) !== undefined) i = end;
  }
  const newline = markdown.includes("\r\n") ? "\r\n" : "\n";
  const firstBlock = tree.children[0];
  const insertion = firstBlock?.type === "heading" && firstBlock.position?.start.offset === (markdown.charCodeAt(0) === 0xfeff ? 1 : 0)
    ? (() => {
      const lineEnd = markdown.indexOf("\n", firstBlock.position!.end.offset);
      return lineEnd < 0 ? markdown.length : lineEnd + 1;
    })()
    : (markdown.charCodeAt(0) === 0xfeff ? 1 : 0);
  const clones: string[] = [];
  let nextImageId = 1;
  for (const reference of references.values()) {
    const destination: Destination = { ...reference.destination, after: reference.destination.end };
    const linkValue = reference.link || !reference.images.length ? resolveDestination(destination, false) : undefined;
    const imageValue = reference.images.length ? resolveDestination(destination, true) : undefined;
    if (reference.images.length && reference.link && linkValue !== undefined) {
      if (imageValue !== undefined && imageValue !== linkValue) {
        let id: string;
        do { id = `herdr-image-reference-${nextImageId++}`; } while (reserved.has(referenceId(id)));
        reserved.add(referenceId(id));
        for (const selector of reference.images) {
          changes.push({ start: selector.start, end: selector.end, value: `[${id}]` });
        }
        const removals = reference.prefixes.sort((left, right) => left.start - right.start);
        let clone = "";
        let cursor = reference.start;
        const replacements = [
          { ...reference.label, value: id },
          { ...destination, value: imageValue },
          ...removals.map((part) => ({ ...part, value: "" })),
        ].sort((left, right) => left.start - right.start || left.end - right.end);
        for (const part of replacements) {
          if (part.start < cursor || part.end > reference.end) throw new Error("Overlapping Markdown definition clone spans");
          clone += markdown.slice(cursor, part.start) + part.value;
          cursor = part.end;
        }
        clone += markdown.slice(cursor, reference.end);
        const candidate = markdownStructure(clone);
        if (candidate.tree.children.length !== 1 || candidate.tree.children[0]?.type !== "definition" ||
          candidate.definitions.length !== 1 || candidate.definitions[0]?.start !== 0 ||
          candidate.tree.children[0].title !== reference.title ||
          candidate.tree.children[0].url !== imageValue ||
          referenceId(candidate.tree.children[0].label ?? "") !== referenceId(id)) {
          throw new Error("Markdown image definition clone failed root round-trip");
        }
        clones.push(clone);
      }
    }
    const value = reference.images.length && !reference.link ? imageValue : linkValue;
    if (value !== undefined) changes.push({ start: destination.start, end: destination.end, value });
  }
  if (clones.length) {
    const separator = (value: string): string => value.endsWith("\n") ? newline : newline + newline;
    const before = insertion === 0 || insertion === 1 ? "" : separator(markdown.slice(0, insertion));
    changes.push({
      start: insertion, end: insertion,
      value: before + clones.map((clone) => clone + separator(clone)).join(newline),
    });
  }
  changes.sort((left, right) => left.start - right.start || left.end - right.end);
  let result = "";
  let cursor = 0;
  let occupiedEnd = 0;
  for (const change of changes) {
    if (change.start < occupiedEnd || change.end > markdown.length) throw new Error("Overlapping Markdown rewrites");
    result += markdown.slice(cursor, change.start) + change.value;
    cursor = change.end;
    if (change.end > change.start) occupiedEnd = change.end;
  }
  return result + markdown.slice(cursor);
}

export async function generateReleaseBody(root: string, tag: string, repository: string): Promise<string> {
  const { version } = parseStableTag(tag);
  requireRepository(repository);
  const packageJson = JSON.parse(await readFile(join(root, "package.json"), "utf8")) as { version?: unknown };
  const cargo = Bun.TOML.parse(await readFile(join(root, "native/Cargo.toml"), "utf8")) as { package?: { name?: string; version?: unknown } };
  const plugin = Bun.TOML.parse(await readFile(join(root, "herdr-plugin.toml"), "utf8")) as { version?: unknown };
  const lock = Bun.TOML.parse(await readFile(join(root, "native/Cargo.lock"), "utf8")) as { package?: { name?: string; version?: unknown; source?: unknown }[] };
  const rootCrates = lock.package?.filter((entry) => entry.name === cargo.package?.name && entry.source === undefined) ?? [];
  if (!cargo.package?.name || rootCrates.length !== 1 ||
    [packageJson.version, cargo.package.version, plugin.version, rootCrates[0]?.version].some((value) => value !== version)) {
    throw new Error(`Tag ${tag} must match package.json, native/Cargo.toml, herdr-plugin.toml, and the root crate in native/Cargo.lock`);
  }
  const sourcePath = `docs/releases/${tag}.md`;
  const markdown = await readFile(join(root, sourcePath), "utf8");
  const heading = `# Herdr Desktop Pet ${tag} release notes`;
  const lines = markdown.split(/\r?\n/);
  const first = lines.findIndex((line) => line.trim() !== "");
  if (first < 0 || lines[first] !== heading) throw new Error(`${sourcePath} must start with the exact heading: ${heading}`);
  const body = lines.slice(first + 1).join("\n").replace(/<!--[\s\S]*?-->/g, "").trim();
  const prose = body.split("\n").filter((line) => line.trim() && !/^\s*#/.test(line));
  if (!prose.length || prose.every((line) => /^(?:[-*\s]*)(?:TODO|TBD|PLACEHOLDER|COMING SOON|RELEASE NOTES HERE)[.!\s]*$/i.test(line))) {
    throw new Error(`${sourcePath} must contain nonempty, reviewed release notes, not a placeholder`);
  }
  const paths = new Set<string>();
  const result = rewriteRelativeLinks(markdown, sourcePath, (repoPath, fragment, isImage) => {
    paths.add(repoPath);
    const base = isImage ? `https://raw.githubusercontent.com/${repository}/${tag}` : `https://github.com/${repository}/blob/${tag}`;
    return `${base}/${repoPath.split("/").map(encodeURIComponent).join("/")}${fragment}`;
  });
  for (const path of paths) {
    try { await stat(join(root, path)); } catch { throw new Error(`${sourcePath} links to a missing repository path: ${path}`); }
  }
  return result;
}

async function main(args: string[]): Promise<void> {
  if (args[0] !== "check" || !args[1]) throw new Error("Usage: bun scripts/release-docs.ts check TAG [--root ROOT] [--repository OWNER/REPO] [--output FILE]");
  const tag = args[1];
  const options = new Map<string, string>();
  for (let i = 2; i < args.length; i += 2) {
    const name = args[i]!;
    const value = args[i + 1];
    if (!["--root", "--repository", "--output"].includes(name) || !value || value.startsWith("--") || options.has(name)) throw new Error(`Invalid or repeated option: ${name}`);
    options.set(name, value);
  }
  const root = resolve(options.get("--root") ?? join(import.meta.dir, ".."));
  const repository = options.get("--repository") ?? process.env.GITHUB_REPOSITORY ?? "hanbong5938/herdr-desktop-pet";
  const body = await generateReleaseBody(root, tag, repository);
  const output = resolve(options.get("--output") ?? join(root, "dist", `${tag}-release-notes.md`));
  await mkdir(dirname(output), { recursive: true });
  const temporary = `${output}.${process.pid}.tmp`;
  try {
    await writeFile(temporary, body, { flag: "wx" });
    await rename(temporary, output);
  } finally {
    await rm(temporary, { force: true });
  }
  console.log(`Validated ${tag}; wrote frozen release notes to ${output}`);
}

if (import.meta.main) {
  main(process.argv.slice(2)).catch((error: unknown) => {
    console.error(`release-docs: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  });
}
