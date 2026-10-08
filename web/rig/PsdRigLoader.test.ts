import { expect, test } from "bun:test"
import type { Layer, Psd } from "ag-psd"
import { validateNativeRigReferences } from "../../native/rig/override-validation"
import { PsdRigLoader } from "./vendor/anime25drig/PsdRigLoader"

function plate(name: string, left: number, top: number, width: number, height: number, color: number[]): Layer {
  const data = new Uint8ClampedArray(width * height * 4)
  for (let offset = 0; offset < data.length; offset += 4) data.set(color, offset)
  return { name, left, top, right: left + width, bottom: top + height, imageData: { width, height, data } }
}

test("explicit eye sides remain independently addressable when both lie right of the face center", () => {
  const children: Layer[] = [
    plate("face", 16, 16, 80, 48, [235, 195, 170, 255]),
    plate("topwear", 16, 66, 80, 42, [230, 230, 240, 255]),
  ]
  for (const [side, x] of [["l", 64], ["r", 82]] as const) {
    const suffix = side === "l" ? "-l" : "_r"
    children.push(
      plate(`eyewhite${suffix}`, x, 28, 12, 10, [250, 250, 250, 255]),
      plate(`irides${suffix}`, x + 3, 30, 6, 6, [30, side === "l" ? 120 : 180, 50, 255]),
      plate(`eyelash${suffix}`, x, 26, 12, 2, [60, 25, 20, 255]),
      plate(`eyebrow${suffix}`, x, 21, 12, 2, [80, 30, 20, 255]),
      plate(`eye_close${suffix}`, x, 34, 12, 2, [60, 25, 20, 255]),
    )
  }
  const psd: Psd = { width: 128, height: 128, children }
  const result = new PsdRigLoader().build(psd, "side-facing.psd", {
    cleanupThresholds: { eyewhite: 1, irides: 1, eyelash: 1, eyebrow: 1, eye_close: 1 },
  })
  const rig = result.model.rig
  const eyeNames = ["eyewhite", "irides", "eyelash", "eyebrow", "eye_close"].flatMap(name => [`${name}_l`, `${name}_r`])
  validateNativeRigReferences({ layerOrder: eyeNames }, {
    raw: children.map(layer => layer.name!),
    loader: rig.layers.map(layer => layer.name),
  })
  for (const name of eyeNames) {
    const layer = rig.layers.find(candidate => candidate.name === name)!
    expect(layer.side).toBe(name.endsWith("_l") ? "L" : "R")
  }
  expect(rig.anchors.eyeL!.icx).toBe(69.5)
  expect(rig.anchors.eyeR!.icx).toBe(87.5)
})

test("reference-only meshes leave complete independent geometry and retained cutout links", () => {
  const psd: Psd = {
    width: 128,
    height: 128,
    children: [
      plate("face", 16, 16, 80, 40, [235, 195, 170, 255]),
      plate("topwear", 16, 66, 80, 42, [230, 230, 240, 255]),
      plate("handwear_3", 28, 55, 24, 40, [100, 110, 120, 255]),
      plate("handwear_1", 32, 60, 12, 16, [210, 170, 150, 255]),
      plate("contact_hand_1", 36, 64, 4, 4, [220, 180, 160, 255]),
    ],
  }
  const { model: { rig } } = new PsdRigLoader().build(psd, "contact-hand.psd", {
    cleanupThresholds: { handwear: 1, contact_hand: 1 },
    meshSources: { handwear_1: "handwear_3", contact_hand_1: "handwear_1" },
    excludeAfterMeshResolution: [" HANDWEAR_3 "],
  })
  const hand = rig.layers.find(layer => layer.name === "handwear_1")!
  const cutout = rig.layers.find(layer => layer.name === "contact_hand_1")!
  expect(rig.layers.some(layer => layer.name === "handwear_3")).toBe(false)
  expect(hand.meshSource).toBeUndefined()
  expect(cutout.meshSource).toBe(hand.name)
  expect([cutout.x, cutout.y, cutout.w, cutout.h]).toEqual([hand.x, hand.y, hand.w, hand.h])
  const pixel = (layer: typeof hand, x: number, y: number) =>
    [...layer.img.data.slice(((y - layer.y) * layer.img.width + x - layer.x) * 4,
      ((y - layer.y) * layer.img.width + x - layer.x) * 4 + 4)]
  expect(pixel(hand, 32, 60)).toEqual([210, 170, 150, 255])
  expect(pixel(hand, 28, 55)[3]).toBe(0)
  expect(pixel(cutout, 36, 64)).toEqual([220, 180, 160, 255])
  for (const layer of rig.layers) {
    if (layer.meshSource) expect(rig.layers.some(source => source.name === layer.meshSource)).toBe(true)
  }
})
