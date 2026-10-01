import { RigLoader } from "./loader"
import { RigRenderer } from "./renderer"
import type { CaptureResult, ParameterRecord, RigMetrics } from "./renderer"

export type RigProbe = {
  load(baseUrl: string): Promise<Record<string, unknown>>
  setParameters(values: Record<string, unknown>): Record<string, unknown>
  capture(): CaptureResult
  setVisible(visible: boolean): { visible: boolean }
  sampleHit(x: number, y: number): Record<string, unknown>
  dispose(): { disposed: true }
  runChecks(): Promise<Record<string, unknown>>
}

type WebKitBridge = { postMessage(message: unknown): void }

type RigWindow = Window & { rigProbe?: RigProbe; webkit?: { messageHandlers?: { rigProbe?: WebKitBridge } } }

declare global {
  interface Window { rigProbe?: RigProbe }
}

function post(type: string, payload: Record<string, unknown> = {}) {
  const bridge = (window as RigWindow).webkit?.messageHandlers?.rigProbe
  bridge?.postMessage({ type, ...payload })
}

function errorDetails(error: unknown): Record<string, unknown> {
  if (error instanceof Error) return { message: error.message, name: error.name }
  return { message: String(error), name: "Error" }
}

export function installRigProbe(canvas: HTMLCanvasElement): RigProbe {
  const loader = new RigLoader()
  const renderer = new RigRenderer(canvas)
  let loadGeneration = 0
  const api: RigProbe = {
    async load(baseUrl: string) {
      const generation = ++loadGeneration
      try {
        const bundle = await loader.load(baseUrl)
        if (generation !== loadGeneration) throw new Error("RIG_STALE_LOAD")
        const metrics = await renderer.applyBundle(bundle)
        const diagnostics = renderer.diagnostics()
        const result = { loaded: true, diagnostics, metrics, sourceUrls: bundle.sourceUrls, timings: bundle.timings, worker: bundle.worker }
        post("loaded", result)
        post("metrics", { metrics, diagnostics })
        return result
      } catch (error) {
        const details = errorDetails(error)
        post("error", details)
        throw error
      }
    },
    setParameters(values: Record<string, unknown>) {
      try {
        const result = renderer.setParameters(values)
        post("metrics", { metrics: renderer.metrics(), parameters: result.parameters })
        return result as unknown as Record<string, unknown>
      } catch (error) {
        const details = errorDetails(error)
        post("error", details)
        throw error
      }
    },
    capture() {
      try {
        const result = renderer.capture()
        post("metrics", { metrics: result.metrics })
        return result
      } catch (error) {
        const details = errorDetails(error)
        post("error", details)
        throw error
      }
    },
    setVisible(visible: boolean) {
      const result = renderer.setVisible(visible)
      post("metrics", { metrics: renderer.metrics(), visible: result.visible })
      return result
    },
    sampleHit(x: number, y: number) {
      const result = renderer.sampleHit(x, y)
      post("metrics", { hit: result, metrics: renderer.metrics() })
      return result
    },
    dispose() {
      ++loadGeneration
      loader.cancel()
      const result = renderer.dispose()
      post("metrics", { disposed: true })
      return result
    },
    async runChecks() {
      try {
        const result = await renderer.runChecks((name, capture) => {
          post("metrics", { kind: "scenario-capture", name, ...capture })
        })
        const payload = {
          ...result,
          diagnostics: { ...result.diagnostics, worker: { module: true, abortable: true, timeoutMs: 30_000 } },
          metrics: renderer.metrics(),
        }
        post("metrics", payload)
        return payload
      } catch (error) {
        const details = errorDetails(error)
        post("error", details)
        throw error
      }
    },
  }
  ;(window as RigWindow).rigProbe = api
  post("ready", { api: "rigProbe", renderer: "Anime2.5DRig WebGL1" })
  return api
}

export type { ParameterRecord, RigMetrics }
