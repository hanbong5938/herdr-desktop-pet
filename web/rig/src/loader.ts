import type { RigOverrides } from "../vendor/anime25drig/RigOverrides"
import type { Anime25DModelData } from "../vendor/anime25drig/types"

export type RigRuntimeModelData = Pick<
  Anime25DModelData,
  "sourceName" | "rig" | "psdLayers" | "missingRequiredLayers" | "assetDiagnostics"
>

export type RigRuntimeLoadResult = {
  model: RigRuntimeModelData
  preprocessing: { noisy: number; layers: number }
}

export type PilotManifest = {
  format?: string
  version?: number
  id?: string
  models?: {
    base?: string | { path?: string; file?: string; model?: string }
    pose?: string | { path?: string; file?: string; model?: string }
    alternate?: string | { path?: string; file?: string; model?: string }
  }
  baseModel?: string
  poseModel?: string
  alternate?: string
  alternateModel?: string
  model?: string
  pose?: string
  metadata?: Record<string, unknown>
  bounds?: Record<string, unknown>
  landmarks?: Record<string, unknown>
  [key: string]: unknown
}

export type LoadedRigBundle = {
  pilot: PilotManifest
  overrides: RigOverrides
  base: RigRuntimeLoadResult
  pose: RigRuntimeLoadResult | null
  sourceUrls: { pilot: string; overrides: string; base: string; pose: string | null }
  timings: { fetchMs: number; decodeMs: number; totalMs: number }
  worker: { module: true; abortable: true; timeoutMs: number }
}

type DecodeRequest = {
  baseBuffer: ArrayBuffer
  baseName: string
  poseBuffer: ArrayBuffer | null
  poseName: string | null
  overrides: RigOverrides
}

type DecodeResponse = {
  base: RigRuntimeLoadResult
  pose: RigRuntimeLoadResult | null
  timings?: { decodeMs: number }
  error?: string
}

type ModelSpec = string | { path?: string; file?: string; model?: string } | undefined

function withTrailingSlash(value: string): string {
  return value.endsWith("/") ? value : `${value}/`
}

function resourceUrl(baseUrl: string, path: string): string {
  return new URL(path.replace(/^\/+/, ""), withTrailingSlash(baseUrl)).toString()
}

function modelPath(value: ModelSpec, fallback: string): string {
  if (typeof value === "string") return value
  if (value && typeof value === "object") {
    const candidate = value.path ?? value.file ?? value.model
    if (typeof candidate === "string" && candidate.length > 0) return candidate
  }
  return fallback
}

async function fetchJson<T>(url: string, signal: AbortSignal): Promise<T> {
  const response = await fetch(url, { signal, credentials: "omit", cache: "no-store" })
  if (!response.ok) throw new Error(`RIG_RESOURCE_HTTP_${response.status}: ${url}`)
  return await response.json() as T
}

async function fetchBytes(url: string, signal: AbortSignal): Promise<ArrayBuffer> {
  const response = await fetch(url, { signal, credentials: "omit", cache: "no-store" })
  if (!response.ok) throw new Error(`RIG_RESOURCE_HTTP_${response.status}: ${url}`)
  return await response.arrayBuffer()
}

function decode(workerRequest: DecodeRequest, signal: AbortSignal): Promise<DecodeResponse> {
  if (signal.aborted) return Promise.reject(new DOMException("Rig decode cancelled", "AbortError"))
  if (typeof Worker === "undefined") throw new Error("RIG_WORKER_UNAVAILABLE")
  return new Promise((resolve, reject) => {
    const worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module", name: "herdr-rig-decode" })
    let settled = false
    let timer = 0
    const abort = () => finish(new DOMException("Rig decode cancelled", "AbortError"))
    const finish = (error?: unknown, value?: DecodeResponse) => {
      if (settled) return
      settled = true
      window.clearTimeout(timer)
      signal.removeEventListener("abort", abort)
      worker.terminate()
      if (error) reject(error)
      else if (value) resolve(value)
      else reject(new Error("RIG_WORKER_EMPTY_RESULT"))
    }
    timer = window.setTimeout(() => finish(new Error("RIG_WORKER_TIMEOUT")), 30_000)
    signal.addEventListener("abort", abort, { once: true })
    worker.onerror = event => finish(new Error(`RIG_WORKER_ERROR: ${event.message || "decode failed"}`))
    worker.onmessage = event => {
      const data: DecodeResponse = event.data
      if (data?.error) finish(new Error(data.error))
      else finish(undefined, data)
    }
    worker.postMessage(workerRequest, [workerRequest.baseBuffer, ...(workerRequest.poseBuffer ? [workerRequest.poseBuffer] : [])])
  })
}

export class RigLoader {
  private active: AbortController | null = null

  cancel() {
    this.active?.abort()
    this.active = null
  }

  async load(baseUrl: string, signal?: AbortSignal): Promise<LoadedRigBundle> {
    this.cancel()
    const controller = new AbortController()
    this.active = controller
    const relayAbort = () => controller.abort()
    signal?.addEventListener("abort", relayAbort, { once: true })
    const started = performance.now()
    try {
      const root = withTrailingSlash(baseUrl)
      const pilotUrl = resourceUrl(root, "pilot.json")
      const pilot = await fetchJson<PilotManifest>(pilotUrl, controller.signal)
      const overridePath = typeof pilot.overrides === "string"
        ? pilot.overrides
        : typeof pilot.rigOverrides === "string" ? pilot.rigOverrides : "rig-overrides.json"
      const overridesUrl = resourceUrl(root, overridePath)
      const overrides = await fetchJson<RigOverrides>(overridesUrl, controller.signal)
      const basePath = modelPath(pilot.models?.base, pilot.baseModel ?? pilot.model ?? "model.psd")
      const posePath = modelPath(pilot.models?.pose ?? pilot.models?.alternate, pilot.poseModel ?? pilot.alternateModel ?? pilot.alternate ?? pilot.pose ?? "pose.psd")
      const baseUrlResolved = resourceUrl(root, basePath)
      const poseUrlResolved = resourceUrl(root, posePath)
      const fetchStarted = performance.now()
      const [baseBuffer, poseBuffer] = await Promise.all([
        fetchBytes(baseUrlResolved, controller.signal),
        fetchBytes(poseUrlResolved, controller.signal),
      ])
      const fetchMs = performance.now() - fetchStarted
      const decoded = await decode({
        baseBuffer,
        baseName: basePath,
        poseBuffer,
        poseName: posePath,
        overrides,
      }, controller.signal)
      return {
        pilot,
        overrides,
        base: decoded.base,
        pose: decoded.pose,
        sourceUrls: { pilot: pilotUrl, overrides: overridesUrl, base: baseUrlResolved, pose: poseUrlResolved },
        timings: { fetchMs, decodeMs: decoded.timings?.decodeMs ?? 0, totalMs: performance.now() - started },
        worker: { module: true, abortable: true, timeoutMs: 30_000 },
      }
    } finally {
      signal?.removeEventListener("abort", relayAbort)
      if (this.active?.signal === controller.signal) this.active = null
    }
  }
}

