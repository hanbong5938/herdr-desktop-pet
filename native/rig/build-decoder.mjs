import { createRequire } from "node:module"
import { copyFile, mkdir, readdir } from "node:fs/promises"
import { dirname, join, relative, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { writeRigLimits } from "./build-limits.mjs"

const root = dirname(fileURLToPath(import.meta.url))
const projectRoot = resolve(root, "../..")
const webRigRoot = join(projectRoot, "web", "rig")
const require = createRequire(import.meta.url)
const { build } = require(join(webRigRoot, "node_modules", "esbuild"))
const ts = require(join(webRigRoot, "node_modules", "typescript"))
if (!process.argv[2]) throw new Error("usage: build-decoder.mjs <output-directory>")
const outputRoot = resolve(process.argv[2])
const outputFile = join(outputRoot, "decoder.js")

async function filesUnder(path) {
  const entries = await readdir(path, { withFileTypes: true })
  const files = []
  for (const entry of entries) {
    const full = join(path, entry.name)
    if (entry.isDirectory()) files.push(...await filesUnder(full))
    else files.push(full)
  }
  return files.sort()
}

async function copyTree(sourceRoot, destinationRoot) {
  for (const source of await filesUnder(sourceRoot)) {
    const destination = join(destinationRoot, relative(sourceRoot, source))
    await mkdir(dirname(destination), { recursive: true })
    await copyFile(source, destination)
  }
}

const program = ts.createProgram([join(root, "decoder.ts")], {
  noEmit: true,
  target: ts.ScriptTarget.ES2022,
  module: ts.ModuleKind.ESNext,
  moduleResolution: ts.ModuleResolutionKind.Bundler,
  resolveJsonModule: true,
  strict: true,
  skipLibCheck: true,
  baseUrl: projectRoot,
  paths: {
    "ag-psd": [join(webRigRoot, "node_modules", "ag-psd")],
    "pako": [join(webRigRoot, "node_modules", "@types", "pako")],
  },
})
const diagnostics = ts.getPreEmitDiagnostics(program)
if (diagnostics.length) {
  const host = { getCanonicalFileName: name => name, getCurrentDirectory: () => projectRoot, getNewLine: () => "\n" }
  throw new Error(ts.formatDiagnostics(diagnostics, host))
}

await mkdir(outputRoot, { recursive: true })
await writeRigLimits(join(outputRoot, "RigLimits.swift"))
await build({
  entryPoints: [join(root, "decoder.ts")],
  outfile: outputFile,
  bundle: true,
  format: "iife",
  globalName: "HerdrNativeRigDecoderBundle",
  platform: "browser",
  target: ["es2020"],
  alias: {
    "ag-psd": join(webRigRoot, "node_modules", "ag-psd", "dist", "bundle.js"),
    "pako": join(webRigRoot, "node_modules", "pako", "dist", "pako.esm.mjs"),
  },
  sourcemap: false,
  minify: false,
  legalComments: "eof",
  logLevel: "info",
})

const notices = [
  [join(root, "NOTICE.txt"), join(outputRoot, "NOTICE.txt")],
  [join(webRigRoot, "vendor", "anime25drig", "LICENSE"), join(outputRoot, "vendor", "anime25drig", "LICENSE")],
  [join(webRigRoot, "vendor", "anime25drig", "UPSTREAM.txt"), join(outputRoot, "vendor", "anime25drig", "UPSTREAM.txt")],
  [join(webRigRoot, "vendor", "DAEMONLET-LICENSE.txt"), join(outputRoot, "vendor", "DAEMONLET-LICENSE.txt")],
  [join(webRigRoot, "node_modules", "base64-js", "LICENSE"), join(outputRoot, "vendor", "BASE64-JS-LICENSE.txt")],
  [join(webRigRoot, "node_modules", "pako", "LICENSE"), join(outputRoot, "vendor", "PAKO-LICENSE.txt")],
  [join(webRigRoot, "vendor", "AG-PSD-NOTICE.txt"), join(outputRoot, "vendor", "AG-PSD-NOTICE.txt")],
  [join(webRigRoot, "vendor", "PROVENANCE.txt"), join(outputRoot, "vendor", "PROVENANCE.txt")],
]
for (const [source, destination] of notices) {
  await mkdir(dirname(destination), { recursive: true })
  await copyFile(source, destination)
}
await copyTree(join(webRigRoot, "vendor", "licenses"), join(outputRoot, "vendor", "licenses"))
