import { initializeCanvas } from "ag-psd"
import { PsdRigLoader } from "../vendor/anime25drig/PsdRigLoader"
import type { RigOverrides } from "../vendor/anime25drig/RigOverrides"
import type { RigLoadResult } from "../vendor/anime25drig/types"
import type { RigRuntimeLoadResult } from "./loader"

initializeCanvas(
  () => { throw new Error("Raster canvas is unavailable in the PSD worker") },
  (width, height) => ({ width, height, data: new Uint8ClampedArray(width * height * 4), colorSpace: "srgb" }),
)

const loader = new PsdRigLoader()
type WorkerRequest = {
  baseBuffer: ArrayBuffer
  baseName: string
  poseBuffer: ArrayBuffer | null
  poseName: string | null
  overrides: RigOverrides
}
type WorkerReply = {
  base?: RigRuntimeLoadResult
  pose?: RigRuntimeLoadResult | null
  timings?: { decodeMs: number }
  error?: string
}

function transferableImages(result: RigRuntimeLoadResult | null): ArrayBuffer[] {
  if (!result) return []
  const buffers: ArrayBuffer[] = []
  for (const layer of result.model.rig.layers) {
    const buffer = layer.img.data.buffer
    if (buffer instanceof ArrayBuffer) buffers.push(buffer)
  }
  return buffers
}

function compact(result: RigLoadResult): RigRuntimeLoadResult {
  const model = result.model
  return {
    preprocessing: result.preprocessing,
    model: {
      sourceName: model.sourceName,
      rig: model.rig,
      psdLayers: model.psdLayers,
      missingRequiredLayers: model.missingRequiredLayers,
      assetDiagnostics: model.assetDiagnostics,
    },
  }
}

self.onmessage = (event: MessageEvent<WorkerRequest>) => {
  const started = performance.now()
  try {
    const request = event.data
    const base = compact(loader.loadArrayBuffer(request.baseBuffer, request.baseName, request.overrides))
    const pose = request.poseBuffer && request.poseName
      ? compact(loader.loadArrayBuffer(request.poseBuffer, request.poseName, request.overrides))
      : null
    const transfer = [...new Set([...transferableImages(base), ...transferableImages(pose)])]
    const reply: WorkerReply = { base, pose, timings: { decodeMs: performance.now() - started } }
    self.postMessage(reply, { transfer })
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    self.postMessage({ error: message } satisfies WorkerReply)
  }
}
