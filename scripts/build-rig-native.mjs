import { spawnSync } from "node:child_process"
import { createHash } from "node:crypto"
import { createReadStream } from "node:fs"
import { mkdir, readdir, rm, stat, writeFile } from "node:fs/promises"
import { dirname, join, relative, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const rigRoot = join(root, "native", "rig")
const profile = process.env.HERDR_RIG_BUILD_PROFILE?.trim() || "release"
if (!/^[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(profile)) throw new Error(`invalid HERDR_RIG_BUILD_PROFILE: ${profile}`)
const outputRoot = join(root, "native", "target", "rig-native", profile)
const runtimeRoot = join(outputRoot, "Resources", "rig")
const swiftHeader = join(rigRoot, "RigBridge.h")
const workerEntitlements = join(rigRoot, "RigDecodeWorker.entitlements.plist")

function run(program, args, cwd = root) {
  const child = spawnSync(program, args, { cwd, stdio: "inherit" })
  if (child.error) throw child.error
  if (child.status !== 0) throw new Error(`${program} ${args.join(" ")} failed with exit ${child.status}`)
}

function output(program, args) {
  const child = spawnSync(program, args, { cwd: root, encoding: "utf8" })
  if (child.error) throw child.error
  if (child.status !== 0) throw new Error(`${program} ${args.join(" ")} failed: ${child.stderr.trim()}`)
  return child.stdout.trim()
}

async function requireFile(path, label) {
  const details = await stat(path).catch(() => null)
  if (!details?.isFile()) throw new Error(`missing ${label}: ${path}`)
}

async function sha256(path) {
  const hash = createHash("sha256")
  for await (const chunk of createReadStream(path)) hash.update(chunk)
  return hash.digest("hex")
}

async function filesUnder(directory) {
  const files = []
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name)
    if (entry.isDirectory()) files.push(...await filesUnder(path))
    else if (entry.isFile()) files.push(path)
    else throw new Error(`unexpected rig resource: ${path}`)
  }
  return files.sort()
}

async function identities(paths, base = root) {
  const entries = await Promise.all(paths.map(async (path) => [relative(base, path), await sha256(path)]))
  return Object.fromEntries(entries.sort(([a], [b]) => a.localeCompare(b)))
}

const swiftSources = [
  "RigModel.swift", "RigDecoder.swift", "RigDeformation.swift", "RigHitTesting.swift",
  "RigMetalShaders.swift", "RigMetalRenderer.swift", "RigMotion.swift",
  "RigNativeHost.swift", "RigBridge.swift",
].map((name) => join(rigRoot, name))
const workerSources = [join(rigRoot, "RigModel.swift"), join(rigRoot, "RigDecodeWorker.swift")]
// Bound source closure to the decoder's pinned inputs, not the entire checkout.
const decoderSources = [
  "native/rig/build-decoder.mjs", "native/rig/build-limits.mjs",
  "native/rig/decoder.ts", "native/rig/override-validation.ts", "native/rig/limits.json",
  "web/rig/package.json", "web/rig/package-lock.json",
  "web/rig/vendor/anime25drig/PsdRigLoader.ts",
  "web/rig/vendor/anime25drig/HairPhysicsConfig.ts",
  "web/rig/vendor/anime25drig/RigAssetInspector.ts",
  "web/rig/vendor/anime25drig/RigOverrides.ts",
  "web/rig/vendor/anime25drig/EyeBlink.ts",
  "web/rig/vendor/anime25drig/MouthMorph.ts",
  "web/rig/vendor/anime25drig/HeadFollow.ts",
  "web/rig/vendor/anime25drig/types.ts",
  "web/rig/vendor/anime25drig/upstream/rigger.js",
  "web/rig/vendor/anime25drig/upstream/genericparts.js",
  "web/rig/node_modules/ag-psd/dist/bundle.js",
  "web/rig/node_modules/pako/dist/pako.esm.mjs",
  "native/rig/NOTICE.txt",
  "web/rig/vendor/anime25drig/LICENSE",
  "web/rig/vendor/anime25drig/UPSTREAM.txt",
  "web/rig/vendor/DAEMONLET-LICENSE.txt",
  "web/rig/vendor/AG-PSD-NOTICE.txt",
  "web/rig/vendor/PROVENANCE.txt",
  "web/rig/node_modules/base64-js/LICENSE",
  "web/rig/node_modules/pako/LICENSE",
].map((path) => join(root, path))

const jsRuntime = process.env.HERDR_RIG_JS_RUNTIME?.trim() || "bun"
await requireFile(workerEntitlements, "rig decode worker entitlements")
const codesign = await output("xcrun", ["--find", "codesign"])
const swiftc = await output("xcrun", ["--find", "swiftc"])
const sdk = await output("xcrun", ["--sdk", "macosx", "--show-sdk-path"])
await rm(outputRoot, { recursive: true, force: true })
await mkdir(runtimeRoot, { recursive: true })
await run(jsRuntime, [join(rigRoot, "build-decoder.mjs"), runtimeRoot])
const rigLimits = join(runtimeRoot, "RigLimits.swift")
await requireFile(join(runtimeRoot, "decoder.js"), "generated rig decoder")
await requireFile(rigLimits, "generated RigLimits.swift")

const frameworks = ["AppKit", "CoreGraphics", "Foundation", "ImageIO", "JavaScriptCore", "Metal", "QuartzCore"]
const common = ["-sdk", sdk, "-target", "arm64-apple-macos13.0", "-O", "-whole-module-optimization"]
const frameworkArgs = frameworks.flatMap((name) => ["-framework", name])
const hostOutput = join(outputRoot, "libherdr_rig.dylib")
await run(swiftc, [
  ...common,
  "-emit-library",
  "-parse-as-library",
  "-module-name", "HerdrRig",
  "-import-objc-header", swiftHeader,
  "-Xlinker", "-install_name",
  "-Xlinker", "@rpath/libherdr_rig.dylib",
  "-o", hostOutput,
  ...swiftSources,
  rigLimits,
  ...frameworkArgs,
])

const workerOutput = join(outputRoot, "rig-decode-worker")
await run(swiftc, [
  ...common,
  "-parse-as-library",
  "-module-name", "HerdrRigDecodeWorker",
  "-o", workerOutput,
  ...workerSources,
  rigLimits,
  ...frameworkArgs,
])
await run(codesign, ["--force", "--timestamp=none", "--sign", "-", "--entitlements", workerEntitlements, workerOutput])

const sourceSha256 = await identities([
  join(root, "scripts", "build-rig-native.mjs"), swiftHeader, workerEntitlements,
  ...new Set([...swiftSources, ...workerSources]), ...decoderSources,
  ...await filesUnder(join(root, "web", "rig", "vendor", "licenses")),
])
const outputSha256 = await identities([
  hostOutput, workerOutput, ...await filesUnder(runtimeRoot),
], outputRoot)

const manifest = {
  version: 1,
  dylib: "libherdr_rig.dylib",
  worker: "rig-decode-worker",
  decoder: "Resources/rig/decoder.js",
  limits: "Resources/rig/RigLimits.swift",
  install_name: "@rpath/libherdr_rig.dylib",
  packaged_rpath: "@executable_path/../Frameworks",
  development_rpath: outputRoot,
  probe_faults: false,
  notices: ["Resources/rig/NOTICE.txt", "Resources/rig/vendor"],
  build: {
    profile, platform: process.platform, architecture: process.arch,
    target: "arm64-apple-macos13.0",
    swift_compiler: output(swiftc, ["--version"]),
    macos_sdk_version: output("xcrun", ["--sdk", "macosx", "--show-sdk-version"]),
    js_runtime: jsRuntime,
    js_runtime_version: output(jsRuntime, ["--version"]),
    source_sha256: sourceSha256,
    output_sha256: outputSha256,
  },
}
await writeFile(join(outputRoot, "rig-native.json"), JSON.stringify(manifest, null, 2) + "\n", "utf8")
await requireFile(hostOutput, "native rig dylib")
await requireFile(workerOutput, "native rig decode worker")
console.log(`Native rig outputs:`)
console.log(`  dylib: ${hostOutput}`)
console.log(`  worker: ${workerOutput}`)
console.log(`  decoder: ${join(runtimeRoot, "decoder.js")}`)
console.log(`  manifest: ${join(outputRoot, "rig-native.json")}`)
