import { build } from "esbuild"
import { createHash } from "node:crypto"
import { mkdir, readdir, readFile, rm, writeFile, copyFile } from "node:fs/promises"
import { dirname, join, relative } from "node:path"
import { fileURLToPath } from "node:url"

const root = dirname(fileURLToPath(import.meta.url))
const dist = join(root, "dist")
await rm(dist, { recursive: true, force: true })
await mkdir(dist, { recursive: true })

const common = {
  bundle: true,
  format: "esm",
  platform: "browser",
  alias: { "ag-psd": "ag-psd/dist/bundle.js" },
  target: ["es2022"],
  sourcemap: false,
  minify: false,
  legalComments: "eof",
  logLevel: "info",
}
await build({ ...common, entryPoints: [join(root, "src/main.ts")], outfile: join(dist, "index.js") })
await build({ ...common, entryPoints: [join(root, "src/worker.ts")], outfile: join(dist, "worker.js") })
await copyFile(join(root, "index.html"), join(dist, "index.html"))

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

const notices = [
  ["vendor/anime25drig/LICENSE", "vendor/anime25drig/LICENSE"],
  ["vendor/anime25drig/UPSTREAM.txt", "vendor/anime25drig/UPSTREAM.txt"],
  ["vendor/DAEMONLET-LICENSE.txt", "vendor/DAEMONLET-LICENSE.txt"],
  ["node_modules/base64-js/LICENSE", "vendor/BASE64-JS-LICENSE.txt"],
  ["node_modules/pako/LICENSE", "vendor/PAKO-LICENSE.txt"],
  ["vendor/AG-PSD-NOTICE.txt", "vendor/AG-PSD-NOTICE.txt"],
  ["vendor/PROVENANCE.txt", "vendor/PROVENANCE.txt"],
]
for (const [source, target] of notices) {
  const destination = join(dist, target)
  await mkdir(dirname(destination), { recursive: true })
  await copyFile(join(root, source), destination)
}
for (const source of await filesUnder(join(root, "vendor", "licenses"))) {
  const target = relative(root, source)
  const destination = join(dist, target)
  await mkdir(dirname(destination), { recursive: true })
  await copyFile(source, destination)
}

const hash = createHash("sha256")
const fingerprints = []
for (const file of await filesUnder(join(root, "vendor"))) {
  const bytes = await readFile(file)
  const digest = createHash("sha256").update(bytes).digest("hex")
  fingerprints.push(`${digest}  ${relative(root, file)}`)
  hash.update(relative(root, file)).update("\0").update(bytes).update("\0")
}
const generated = [
  "Herdr browser rig build fingerprint",
  `sourceTreeSHA256 ${hash.digest("hex")}`,
  ...fingerprints,
  "",
].join("\n")
await writeFile(join(dist, "vendor-fingerprint.txt"), generated)
await writeFile(join(dist, "build-info.json"), JSON.stringify({
  format: "herdr.rig.browser",
  renderer: "Anime2.5DRig WebGL1",
  daemonletCommit: "e0b555d142fc4c6dc46897d94a4b6e9c5fc5b8ed",
  anime25dRigCommit: "d48825867acd081de22b0e7b5585bb562288796d",
  entry: "index.js",
  worker: "worker.js",
  relativeResources: true,
}, null, 2) + "\n")
