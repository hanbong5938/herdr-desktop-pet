import { chmod, cp, mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const root = join(import.meta.dir, "..");
const cargoTarget = process.env.HERDR_PET_CARGO_TARGET?.trim() ?? "";
const releaseDirectory = cargoTarget
  ? join(root, "native", "target", cargoTarget, "release")
  : join(root, "native", "target", "release");
const nativeBinary = join(releaseDirectory, "herdr-desktop-pet");
const assetsSource = join(root, "assets", "rubelia-default");
const thumbnailSource = join(root, "assets", "rubelia-thumbnail.png");
const rigBuild = join(root, "native", "target", "rig-native", "release");
const appName = process.env.HERDR_PET_APP_NAME?.trim() || "HerdrDesktopPet.app";
const appRoot = join(root, "dist", appName);
const appContents = join(appRoot, "Contents");
const appBinary = join(appContents, "MacOS", "herdr-desktop-pet");
const appResources = join(appContents, "Resources");
const appAssets = join(appResources, "default");
const appFrameworks = join(appContents, "Frameworks");
const appRigLibrary = join(appFrameworks, "libherdr_rig.dylib");
const appRigWorker = join(appContents, "MacOS", "rig-decode-worker");
const workerEntitlements = join(root, "native", "rig", "RigDecodeWorker.entitlements.plist");
const appCreator = join(appResources, "creator");
const licenseSource = join(root, "LICENSE.txt");
const licenseDestination = join(appResources, "LICENSE.txt");
const infoPlist = join(appContents, "Info.plist");

function fail(message: string): never {
  console.error(`package:native: ${message}`);
  process.exit(1);
}

async function requireFile(path: string, label: string): Promise<void> {
  try {
    const details = await stat(path);
    if (!details.isFile()) fail(`${label} is not a regular file: ${path}`);
  } catch {
    fail(`missing ${label}: ${path}; run the prerequisite command first`);
  }
}

async function requireDirectory(path: string, label: string): Promise<void> {
  try {
    const details = await stat(path);
    if (!details.isDirectory()) fail(`${label} is not a directory: ${path}`);
  } catch {
    fail(`missing ${label}: ${path}`);
  }
}


function escapeXml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&apos;");
}

async function runTool(command: string[], label: string, input?: string): Promise<string> {
  const child = Bun.spawn(command, { stdin: input === undefined ? undefined : "pipe", stdout: "pipe", stderr: "pipe" });
  if (input !== undefined) {
    child.stdin.write(input);
    child.stdin.end();
  }
  const [exit, stdout, stderr] = await Promise.all([
    child.exited, new Response(child.stdout).text(), new Response(child.stderr).text(),
  ]);
  if (exit !== 0) throw new Error(`${label} failed (exit ${exit}): ${stderr.trim()}`);
  return stdout;
}

async function stripDevelopmentRpaths(binary: string): Promise<void> {
  const commands = await runTool(["otool", "-l", binary], "reading Mach-O load commands");
  for (const match of commands.matchAll(/cmd LC_RPATH\s+cmdsize \d+\s+path (.+?) \(offset \d+\)/g)) {
    const path = match[1]!;
    if (!path.startsWith("@") && path !== "/usr/lib/swift") {
      await runTool(["install_name_tool", "-delete_rpath", path, binary], "removing development loader path");
    }
  }
}

async function validateNativePack(binary: string, path: string): Promise<void> {
  const temporary = await mkdtemp(join(tmpdir(), "herdr-package-validation-"));
  try {
    await runTool([
      binary, "pack", "validate", "--path", path,
      "--config-dir", join(temporary, "config"), "--state-dir", join(temporary, "state"),
    ], `full native validation of ${path}`);
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

async function readEntitlements(target: string): Promise<Record<string, unknown>> {
  const plist = await runTool(["codesign", "-d", "--xml", "--entitlements", "-", target], `reading entitlements of ${target}`);
  if (!plist.trim()) return {};
  const json = await runTool(["plutil", "-convert", "json", "-o", "-", "-"], `parsing entitlements of ${target}`, plist);
  const value: unknown = JSON.parse(json);
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`invalid entitlements of ${target}: expected a dictionary`);
  }
  return value as Record<string, unknown>;
}

async function runCodesign(identity: string): Promise<void> {
  for (const target of [appRigLibrary, appRigWorker, appBinary, appRoot]) {
    const command = ["codesign", "--force", "--timestamp=none", "--sign", identity];
    if (target === appRigWorker) command.push("--entitlements", workerEntitlements);
    await runTool([...command, target], `signing ${target}`);
  }
  await runTool(["codesign", "--verify", "--strict", appRigWorker], "verifying packaged decode worker signature");
  await runTool(["codesign", "--verify", "--deep", "--strict", appRoot], "verifying packaged signature");
  const worker = await readEntitlements(appRigWorker);
  const keys = Object.keys(worker);
  if (keys.length !== 1 || keys[0] !== "com.apple.security.cs.allow-jit" || worker[keys[0]] !== true) {
    throw new Error("packaged decode worker must have only com.apple.security.cs.allow-jit=true");
  }
  for (const target of [appRigLibrary, appBinary, appRoot]) {
    if (Object.keys(await readEntitlements(target)).length !== 0) {
      throw new Error(`unexpected entitlements on ${target}`);
    }
  }
}

async function main(): Promise<void> {
  if (process.platform !== "darwin") fail("native packaging requires macOS");
  if (process.arch !== "arm64") fail("native packaging requires macOS arm64");
  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]*\.app$/.test(appName)) {
    fail("HERDR_PET_APP_NAME must be a plain .app filename inside dist");
  }
  if (cargoTarget && !/^[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(cargoTarget)) {
    fail(`invalid HERDR_PET_CARGO_TARGET: ${cargoTarget}`);
  }
  if (!Bun.which("codesign")) fail("codesign is required but was not found on PATH");

  await requireFile(licenseSource, "license notice");
  await requireFile(nativeBinary, "release native binary");
  await requireDirectory(assetsSource, "Rubelia rig default");
  await requireFile(thumbnailSource, "Rubelia menu thumbnail");
  await requireFile(join(rigBuild, "libherdr_rig.dylib"), "release native rig library");
  await requireFile(join(rigBuild, "rig-decode-worker"), "release native decode worker");
  await requireFile(workerEntitlements, "decode worker JIT entitlements");
  await requireFile(join(rigBuild, "Resources", "rig", "decoder.js"), "trusted decoder bundle");
  await requireFile(join(rigBuild, "Resources", "rig", "NOTICE.txt"), "renderer license closure");
  await requireFile(join(root, "tools", "character-pack.py"), "creator tool");
  await requireFile(join(root, ".agents", "skills", "character-creator", "SKILL.md"), "creator skill");
  await validateNativePack(nativeBinary, assetsSource);

  let packageJson: { version?: unknown };
  try {
    packageJson = JSON.parse(await readFile(join(root, "package.json"), "utf8")) as {
      version?: unknown;
    };
  } catch {
    fail("package.json is missing or invalid JSON");
  }
  if (
    typeof packageJson.version !== "string" ||
    !/^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$/.test(packageJson.version)
  ) {
    fail("package.json must contain a semantic version");
  }
  const version = escapeXml(packageJson.version);

  const cargoManifest = Bun.TOML.parse(
    await readFile(join(root, "native", "Cargo.toml"), "utf8"),
  ) as { package?: { version?: unknown } };
  const pluginManifest = Bun.TOML.parse(
    await readFile(join(root, "herdr-plugin.toml"), "utf8"),
  ) as { version?: unknown };
  if (cargoManifest.package?.version !== packageJson.version || pluginManifest.version !== packageJson.version) {
    fail("package.json, native/Cargo.toml, and herdr-plugin.toml versions must match");
  }
  const versionProcess = Bun.spawn([nativeBinary, "--version"], {
    stdout: "pipe",
    stderr: "pipe",
  });
  const [versionExit, versionOutput, versionError] = await Promise.all([
    versionProcess.exited,
    new Response(versionProcess.stdout).text(),
    new Response(versionProcess.stderr).text(),
  ]);
  if (versionExit !== 0 || versionOutput.trim() !== `herdr-desktop-pet ${packageJson.version}`) {
    fail(`release binary version does not match ${packageJson.version}; rebuild before packaging: ${versionError.trim()}`);
  }

  await rm(appRoot, { recursive: true, force: true });
  await mkdir(join(appContents, "MacOS"), { recursive: true });
  await mkdir(appResources, { recursive: true });
  await mkdir(appFrameworks, { recursive: true });
  await mkdir(appCreator, { recursive: true });
  await cp(nativeBinary, appBinary);
  await chmod(appBinary, 0o755);
  await cp(licenseSource, licenseDestination);
  await cp(assetsSource, appAssets, { recursive: true });
  await cp(thumbnailSource, join(appResources, "default-thumbnail.png"));
  await cp(join(rigBuild, "libherdr_rig.dylib"), appRigLibrary);
  await cp(join(rigBuild, "rig-decode-worker"), appRigWorker);
  await chmod(appRigWorker, 0o755);
  await mkdir(join(appResources, "rig"), { recursive: true });
  for (const resource of ["decoder.js", "NOTICE.txt", "vendor"]) {
    await cp(join(rigBuild, "Resources", "rig", resource), join(appResources, "rig", resource), { recursive: true });
  }
  await cp(join(root, "native", "rig", "limits.json"), join(appResources, "rig", "limits.json"));
  await cp(join(root, "tools", "character-pack.py"), join(appCreator, "character-pack.py"));
  await cp(join(root, ".agents", "skills", "character-creator", "SKILL.md"), join(appCreator, "SKILL.md"));
  for (const binary of [appBinary, appRigLibrary, appRigWorker]) {
    await stripDevelopmentRpaths(binary);
  }

  const plist = `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleLocalizations</key>
  <array>
    <string>ko</string>
    <string>en</string>
  </array>
  <key>CFBundleDisplayName</key>
  <string>Herdr Desktop Pet</string>
  <key>CFBundleExecutable</key>
  <string>herdr-desktop-pet</string>
  <key>CFBundleIdentifier</key>
  <string>dev.herdr.desktop-pet</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>Herdr Desktop Pet</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${version}</string>
  <key>CFBundleVersion</key>
  <string>${version}</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
  <key>LSUIElement</key>
  <true/>
</dict>
</plist>
`;
  await writeFile(infoPlist, plist, "utf8");

  const identity =
    process.env.HERDR_PET_CODESIGN_IDENTITY?.trim() ||
    process.env.CODESIGN_IDENTITY?.trim() ||
    "-";
  await runCodesign(identity);
  await validateNativePack(appBinary, appAssets);
  if (identity === "-") {
    console.log(`Packaged ${appRoot} with an ad-hoc local signature; no official signing or notarization was performed.`);
  } else {
    console.log(`Packaged ${appRoot} with signing identity ${identity}; notarization was not performed.`);
  }
}

await main();
