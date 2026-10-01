import { spawnSync } from "node:child_process"
import { mkdir, rm, stat, writeFile } from "node:fs/promises"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const rigRoot = join(root, "native", "rig")
const profile = process.env.HERDR_RIG_BUILD_PROFILE?.trim() || "release"
if (!/^[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(profile)) throw new Error(`invalid HERDR_RIG_BUILD_PROFILE: ${profile}`)
const outputRoot = join(root, "native", "target", "rig-native", profile)
const runtimeRoot = join(outputRoot, "Resources", "rig")
const swiftHeader = join(rigRoot, "RigBridge.h")

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

const jsRuntime = process.env.HERDR_RIG_JS_RUNTIME?.trim() || "bun"
await rm(outputRoot, { recursive: true, force: true })
await mkdir(runtimeRoot, { recursive: true })
await run(jsRuntime, [join(rigRoot, "build-decoder.mjs"), runtimeRoot])
const rigLimits = join(runtimeRoot, "RigLimits.swift")
await requireFile(join(runtimeRoot, "decoder.js"), "generated rig decoder")
await requireFile(rigLimits, "generated RigLimits.swift")

const swiftc = await output("xcrun", ["--find", "swiftc"])
const sdk = await output("xcrun", ["--sdk", "macosx", "--show-sdk-path"])

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
  join(rigRoot, "RigModel.swift"),
  join(rigRoot, "RigDecoder.swift"),
  join(rigRoot, "RigDeformation.swift"),
  join(rigRoot, "RigHitTesting.swift"),
  join(rigRoot, "RigMetalShaders.swift"),
  join(rigRoot, "RigMetalRenderer.swift"),
  join(rigRoot, "RigMotion.swift"),
  join(rigRoot, "RigNativeHost.swift"),
  join(rigRoot, "RigBridge.swift"),
  rigLimits,
  ...frameworkArgs,
])

const workerOutput = join(outputRoot, "rig-decode-worker")
await run(swiftc, [
  ...common,
  "-parse-as-library",
  "-module-name", "HerdrRigDecodeWorker",
  "-o", workerOutput,
  join(rigRoot, "RigModel.swift"),
  join(rigRoot, "RigDecodeWorker.swift"),
  rigLimits,
  ...frameworkArgs,
])

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
}
await writeFile(join(outputRoot, "rig-native.json"), JSON.stringify(manifest, null, 2) + "\n", "utf8")
await requireFile(hostOutput, "native rig dylib")
await requireFile(workerOutput, "native rig decode worker")
console.log(`Native rig outputs:`)
console.log(`  dylib: ${hostOutput}`)
console.log(`  worker: ${workerOutput}`)
console.log(`  decoder: ${join(runtimeRoot, "decoder.js")}`)
console.log(`  manifest: ${join(outputRoot, "rig-native.json")}`)
