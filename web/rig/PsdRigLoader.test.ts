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
