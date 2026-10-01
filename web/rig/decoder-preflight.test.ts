import { beforeAll, expect, test } from "bun:test"
import { build } from "esbuild"
import { readFileSync } from "node:fs"
import { join } from "node:path"
import { createContext, runInContext } from "node:vm"

// decoder.ts replaces console, atob, and Uint8ClampedArray on its global, as
// the bare JavaScriptCore worker expects. Bundle it like build-decoder.mjs and
// evaluate it in a separate realm so those globals never leak into bun test.
const projectRoot = join(import.meta.dir, "..", "..")
type Preflight = (input: Uint8Array, label: string) => { rgba: number; channels: number }
type Decode = (base: Uint8Array, baseName: string, pose: null, poseName: null, overrides: string) => unknown
let preflight: Preflight
let decode: Decode

beforeAll(async () => {
  const result = await build({
    stdin: {
      contents: `import { preflightPSD } from "./native/rig/decoder"\nglobalThis.__preflight = preflightPSD\n`,
      resolveDir: projectRoot,
      loader: "ts",
    },
    bundle: true,
    write: false,
    format: "iife",
    platform: "browser",
    target: ["es2020"],
    alias: {
      "ag-psd": join(import.meta.dir, "node_modules", "ag-psd", "dist", "bundle.js"),
      "pako": join(import.meta.dir, "node_modules", "pako", "dist", "pako.esm.mjs"),
    },
    logLevel: "silent",
  })
  const context = createContext({})
  runInContext(result.outputFiles[0].text, context)
  preflight = context.__preflight
  decode = context.__herdrRigDecode
})

const u16 = (value: number) => [value >>> 8 & 0xff, value & 0xff]
const u32 = (value: number) => [value >>> 24 & 0xff, value >>> 16 & 0xff, value >>> 8 & 0xff, value & 0xff]
const ascii = (text: string) => [...text].map(character => character.charCodeAt(0))

type Block = { key: string; data: number[]; signature?: "8BIM" | "8B64"; length?: number; high?: number }

function tagged({ key, data, signature = "8BIM", length = data.length, high = 0 }: Block): number[] {
  const size = signature === "8B64" ? [...u32(high), ...u32(length)] : u32(length)
  return [...ascii(signature), ...ascii(key), ...size, ...data, ...(data.length % 2 ? [0] : [])]
}

// A nested layer-info payload: count 257 with no records, which ag-psd would
// read past the block as further uncapped layer records.
const nestedLayers = [...u16(257), 0, 0]

/** One 1x1 8-bit RGB layer with raw channels; blocks go in the record and trailer. */
function psd(layerBlocks: Block[] = [], trailerBlocks: Block[] = []): Uint8Array {
  const extra = [...u32(0), ...u32(0), 1, ...ascii("a"), 0, 0, ...layerBlocks.flatMap(tagged)]
  const record = [
    ...u32(0), ...u32(0), ...u32(1), ...u32(1),
    ...u16(4), ...[-1, 0, 1, 2].flatMap(id => [...u16(id & 0xffff), ...u32(3)]),
    ...ascii("8BIMnorm"), 255, 0, 0, 0,
    ...u32(extra.length), ...extra,
  ]
  const layerInfo = [...u16(1), ...record, ...[0, 1, 2, 3].flatMap(() => [0, 0, 0xff])]
  const layerMask = [...u32(layerInfo.length), ...layerInfo, ...u32(0), ...trailerBlocks.flatMap(tagged)]
  return new Uint8Array([
    ...ascii("8BPS"), ...u16(1), 0, 0, 0, 0, 0, 0, ...u16(3), ...u32(1), ...u32(1), ...u16(8), ...u16(3),
    ...u32(0), ...u32(0), ...u32(layerMask.length), ...layerMask,
    ...u16(0), 0xff, 0xff, 0xff,
  ])
}

function failure(run: () => unknown): string {
  try {
    run()
  } catch (error) {
    return String((error as Error).message)
  }
  throw new Error("expected the decoder to reject the PSD")
}

const unknownTag: Block = { key: "zzzz", data: [1, 2, 3] }

test("preflight accepts tagged blocks it does not need to inspect", () => {
  expect(preflight(psd(), "plain").rgba).toBeGreaterThan(0)
  expect(preflight(psd([unknownTag, { ...unknownTag, signature: "8B64" }], [unknownTag]), "unknown").rgba).toBeGreaterThan(0)
})

test("bundled Rubelia PSD passes preflight", () => {
  const bytes = new Uint8Array(readFileSync(join(projectRoot, "assets", "rubelia-default", "waiting.psd")))
  expect(preflight(bytes, "waiting.psd").rgba).toBeGreaterThan(0)
})

test("nested layer-info blocks are rejected before ag-psd parses them", () => {
  for (const key of ["Lr16", "Lr32", "Layr"]) {
    for (const signature of ["8BIM", "8B64"] as const) {
      const block: Block = { key, data: nestedLayers, signature }
      for (const input of [psd([block]), psd([unknownTag, block]), psd([], [block]), psd([], [unknownTag, block])]) {
        expect(failure(() => decode(input, "crafted.psd", null, null, "{}")))
          .toMatch(new RegExp(`^RIG_DECODE_INVALID: base PSD .*additional info ${key} nested layer data`))
      }
    }
  }
})

test("tagged blocks with truncated or oversized lengths fail closed", () => {
  for (const block of [
    { ...unknownTag, length: 1000 },
    { ...unknownTag, signature: "8B64", length: 1000 },
    { ...unknownTag, signature: "8B64", high: 1 },
  ] satisfies Block[]) {
    expect(failure(() => preflight(psd([block]), "layer"))).toStartWith("RIG_DECODE_INVALID: layer layer 0 additional info")
    expect(failure(() => preflight(psd([], [block]), "trailer"))).toStartWith("RIG_DECODE_INVALID: trailer layer/mask trailer additional info")
  }
})
