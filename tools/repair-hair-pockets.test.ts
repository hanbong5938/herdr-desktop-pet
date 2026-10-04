import { expect, test } from "bun:test"
import { createHash } from "node:crypto"
import { createRequire } from "node:module"
import { neutralComposite, repairHairPockets } from "./repair-hair-pockets.mjs"

const require = createRequire(new URL("../web/rig/package.json", import.meta.url))
const { readPsd, writePsd } = require("ag-psd")

function fixture() {
  const hair = new Uint8ClampedArray(4 * 4 * 4)
  for (let i = 0; i < hair.length; i += 4) hair.set([130, 55, 40, 255], i)
  hair.set([255, 255, 255, 255], 0)
  hair.set([255, 255, 255, 128], 4)
  hair.set([255, 255, 255, 255], (1 * 4 + 2) * 4) // Diagonal, separate pocket.
  const layer = (name: string, data: Uint8ClampedArray, width: number, height: number) => ({
    name, left: 10, top: 20, right: 10 + width, bottom: 20 + height,
    hidden: false, clipping: false, opacity: 1, blendMode: "normal",
    imageData: { width, height, data },
  })
  const psd = { width: 32, height: 32, children: [
    layer("back hair", hair, 4, 4),
    layer("topwear", new Uint8ClampedArray([255, 255, 255, 255]), 1, 1),
    layer("face", new Uint8ClampedArray([245, 220, 200, 255]), 1, 1),
  ] }
  const input = Buffer.from(writePsd({ ...psd, imageData: neutralComposite(psd) }, { noBackground: true }))
  const plan = { version: 1, inputSha256: createHash("sha256").update(input).digest("hex"), minimumChannel: 195, maximumChroma: 40,
    pockets: [{ layer: "back hair", seed: [10, 20] }] }
  return { input, plan }
}

test("clears only the reviewed four-connected hair pocket, preserving white clothes and the independent face", () => {
  const { input, plan } = fixture()
  const before = readPsd(input, { useImageData: true })
  const { output } = repairHairPockets(input, plan)
  const after = readPsd(output, { useImageData: true })
  const expectedHair = new Uint8ClampedArray(before.children[0].imageData.data)
  expectedHair.fill(0, 0, 8)
  expect(after.children[0].imageData.data).toEqual(expectedHair)
  expect(after.children[0].left).toBe(10)
  expect(after.children[0].top).toBe(20)
  expect(after.children[0].right).toBe(14)
  expect(after.children[0].bottom).toBe(24)
  expect(after.children[1].imageData.data).toEqual(before.children[1].imageData.data)
  expect(after.children[2].imageData.data).toEqual(before.children[2].imageData.data)
})

test("refuses a stale source hash or a garment-layer seed rather than deleting pale artwork", () => {
  const { input, plan } = fixture()
  expect(() => repairHairPockets(input, { ...plan, inputSha256: "0".repeat(64) })).toThrow("immutable input")
  expect(() => repairHairPockets(input, { ...plan, pockets: [{ layer: "topwear", seed: [10, 20] }] })).toThrow("hair layers")
})

test("refuses a matte from a different hair raster or one that adds background opacity", () => {
  const { input, plan } = fixture()
  const decoded = readPsd(input, { useImageData: true })
  const sourceRgbaSha256 = createHash("sha256").update(decoded.children[0].imageData.data).digest("hex")
  const matteRepairs = [{ layer: "back hair", pixels: [[11, 20, 130, 55, 40, 255]] }]
  expect(() => repairHairPockets(input, { ...plan, matteRepairs, matteRecords: [] })).toThrow("source raster")
  expect(() => repairHairPockets(input, {
    ...plan, matteRepairs, matteRecords: [{ layer: "back hair", sourceRgbaSha256 }],
  })).toThrow("cannot add alpha")
})

test("applies source-bound matte coverage and colour without changing clothes or face pixels", () => {
  const { input, plan } = fixture()
  const before = readPsd(input, { useImageData: true })
  const sourceRgbaSha256 = createHash("sha256").update(before.children[0].imageData.data).digest("hex")
  const { output } = repairHairPockets(input, {
    ...plan,
    matteRecords: [{ layer: "back hair", sourceRgbaSha256 }],
    matteRepairs: [{ layer: "back hair", pixels: [[10, 20, 0, 0, 0, 0], [11, 20, 150, 50, 25, 64]] }],
  })
  const after = readPsd(output, { useImageData: true })
  const expected = new Uint8ClampedArray(before.children[0].imageData.data)
  expected.set([0, 0, 0, 0, 150, 50, 25, 64], 0)
  expect(after.children[0].imageData.data).toEqual(expected)
  expect(after.children[1].imageData.data).toEqual(before.children[1].imageData.data)
  expect(after.children[2].imageData.data).toEqual(before.children[2].imageData.data)
})

test("does not authorize white garment removal through the hand-boundary opt-in", () => {
  const { input, plan } = fixture()
  const before = readPsd(input, { useImageData: true })
  const sourceRgbaSha256 = createHash("sha256").update(before.children[1].imageData.data).digest("hex")
  expect(() => repairHairPockets(input, {
    ...plan, pockets: [{ layer: "topwear", seed: [10, 20] }],
    handBoundaryLayers: ["topwear"],
    matteRecords: [{ layer: "topwear", sourceRgbaSha256 }],
    matteRepairs: [{ layer: "topwear", pixels: [[10, 20, 0, 0, 0, 0]] }],
  })).toThrow()
})
