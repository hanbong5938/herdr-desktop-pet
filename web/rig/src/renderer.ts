import { Anime25DRenderer } from "../vendor/anime25drig/Anime25DRenderer"
import { clampParameter, DEFAULT_PARAMETERS, PARAMETER_RANGES } from "../vendor/anime25drig/Anime25DParameters"
import { DEFAULT_HAIR_PHYSICS } from "../vendor/anime25drig/HairPhysicsConfig"
import { selectIndependentPoseLayers } from "../vendor/anime25drig/pose/PoseLayerSelector"
import type { PoseTransform } from "../vendor/anime25drig/pose/types"
import type { Anime25DParameter, Anime25DParameterState, RigImage } from "../vendor/anime25drig/types"
import type { LoadedRigBundle } from "./loader"

export type ParameterRecord = Partial<Record<Anime25DParameter, number>>
export type RigMetrics = {
  canvas: { width: number; height: number; dpr: number }
  webgl: { version: "WebGL1"; stencil: boolean; framebufferCount: number; renderbufferCount: number }
  layerCount: number
  meshCount: number
  textureCount: number
  timings: { renderMs: number; readbackMs: number; loadMs: number }
  alphaPixels: number
  alphaCursor: number
  cursor: { x: number; y: number }
}
export type CaptureResult = { dataUrl: string; metrics: RigMetrics }
export type ScenarioResult = {
  name: string
  parameters: Anime25DParameterState
  metrics: RigMetrics
  changedPixels: number
  changedAlphaPixels: number
  available: boolean
}

type FrameSample = { image: RigImage; metrics: RigMetrics }

const IDENTITY_POSE: PoseTransform = {
  scale: 1,
  rotationRad: 0,
  translationX: 0,
  translationY: 0,
  sourceCenterX: 0,
  sourceCenterY: 0,
}

function smoothPulse(seconds: number, period: number, phase: number): number {
  const cycle = ((seconds / period + phase) % 1 + 1) % 1
  if (cycle > 0.18) return 0
  const normalized = cycle / 0.18
  return normalized < 0.5 ? normalized * 2 : (1 - normalized) * 2
}

function countChanged(left: Uint8ClampedArray, right: Uint8ClampedArray): { pixels: number; alphaPixels: number } {
  let pixels = 0
  let alphaPixels = 0
  for (let index = 0; index < left.length; index += 4) {
    if (left[index] !== right[index] || left[index + 1] !== right[index + 1] || left[index + 2] !== right[index + 2] || left[index + 3] !== right[index + 3]) pixels++
    if (left[index + 3] !== right[index + 3]) alphaPixels++
  }
  return { pixels, alphaPixels }
}

function parameterSnapshot(values: ParameterRecord): Anime25DParameterState {
  return { ...DEFAULT_PARAMETERS, ...values }
}

export class RigRenderer {
  readonly canvas: HTMLCanvasElement
  readonly engine: Anime25DRenderer
  private bundle: LoadedRigBundle | null = null
  private parameters: Anime25DParameterState = { ...DEFAULT_PARAMETERS }
  private manual = new Set<Anime25DParameter>()
  private frameHandle = 0
  private running = false
  private disposed = false
  private visible = true
  private lastRenderMs = 0
  private lastLoadMs = 0
  private cursor = { x: 0, y: 0 }
  private pointerAlpha = 0
  private poseMix = 0

  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas
    this.engine = new Anime25DRenderer(canvas)
  }

  get loaded() { return this.bundle !== null }

  async applyBundle(bundle: LoadedRigBundle): Promise<RigMetrics> {
    if (this.disposed) throw new Error("RIG_RENDERER_DISPOSED")
    const started = performance.now()
    this.stop()
    this.applyModels(bundle)
    this.bundle = bundle
    this.poseMix = 0
    this.parameters = { ...DEFAULT_PARAMETERS }
    this.manual.clear()
    this.lastLoadMs = performance.now() - started + bundle.timings.totalMs
    this.renderAt(performance.now(), true)
    this.start()
    return this.metrics(0)
  }

  private applyModels(bundle: LoadedRigBundle): void {
    this.engine.applyRig(bundle.base.model.rig)
    this.engine.setHairPhysics({
      ...bundle.overrides.physics,
      frontHair: { ...DEFAULT_HAIR_PHYSICS.frontHair, ...bundle.overrides.physics?.frontHair },
      backHair: { ...DEFAULT_HAIR_PHYSICS.backHair, ...bundle.overrides.physics?.backHair },
    })
    if (bundle.pose) {
      const selection = selectIndependentPoseLayers(bundle.base.model.rig, bundle.pose.model.rig)
      if (selection.errors.length) throw new Error(`RIG_POSE_INVALID: ${selection.errors.join("; ")}`)
      this.engine.applyPoseRig(bundle.pose.model.rig, selection, IDENTITY_POSE)
    }
    this.engine.setPoseMix(0)
  }

  setParameters(values: Record<string, unknown>): { parameters: Anime25DParameterState; changed: string[] } {
    if (this.disposed) throw new Error("RIG_RENDERER_DISPOSED")
    const changed: string[] = []
    for (const [rawName, rawValue] of Object.entries(values)) {
      if (!Object.hasOwn(DEFAULT_PARAMETERS, rawName)) throw new Error(`RIG_PARAMETER_UNKNOWN: ${rawName}`)
      if (typeof rawValue !== "number" || !Number.isFinite(rawValue)) throw new Error(`RIG_PARAMETER_NONFINITE: ${rawName}`)
      const name = rawName as Anime25DParameter
      const value = clampParameter(name, rawValue)
      this.parameters[name] = value
      this.manual.add(name)
      changed.push(name)
    }
    if (this.loaded) this.renderAt(performance.now(), false)
    return { parameters: { ...this.parameters }, changed }
  }

  setVisible(visible: boolean): { visible: boolean } {
    if (this.disposed) throw new Error("RIG_RENDERER_DISPOSED")
    this.visible = visible
    this.canvas.style.visibility = visible ? "visible" : "hidden"
    if (visible) this.start()
    else this.stop()
    return { visible }
  }

  sampleHit(x: number, y: number) {
    if (!this.loaded || !Number.isFinite(x) || !Number.isFinite(y)) return { x, y, alpha: 0, interactive: false, model: { x, y } }
    const modelX = Math.max(0, Math.min(this.canvas.width - 1, x))
    const modelY = Math.max(0, Math.min(this.canvas.height - 1, y))
    const alpha = this.engine.sampleAlpha(modelX, this.canvas.height - 1 - modelY)
    this.cursor = { x: modelX, y: modelY }
    this.pointerAlpha = alpha
    return { x: modelX, y: modelY, alpha, interactive: alpha > 0.02, model: { x: modelX, y: modelY } }
  }

  capture(): CaptureResult {
    if (!this.loaded) throw new Error("RIG_NOT_LOADED")
    const started = performance.now()
    this.engine.gl.finish()
    const image = this.engine.readFrame()
    const readbackMs = performance.now() - started
    const dataUrl = this.canvas.toDataURL("image/png")
    return { dataUrl, metrics: this.metrics(readbackMs, image) }
  }

  async runChecks(onFrame?: (name: string, capture: CaptureResult) => void): Promise<{ ok: boolean; scenarios: ScenarioResult[]; diagnostics: Record<string, unknown> }> {
    if (!this.bundle) throw new Error("RIG_NOT_LOADED")
    const wasRunning = this.running
    this.stop()
    const savedParameters = { ...this.parameters }
    const savedManual = new Set(this.manual)
    const savedPoseMix = this.poseMix
    // Rebuild spring state before clock-zero replay, not just parameter values.
    this.applyModels(this.bundle)
    const baseTime = 0
    const scenarios: ScenarioResult[] = []
    let baseline: FrameSample | null = null
    const run = (name: string, values: ParameterRecord, at = baseTime, neutral = true) => {
      this.parameters = parameterSnapshot({ physAmp: 0, fhAmp: 0, ...values })
      this.manual = new Set(Object.keys(this.parameters) as Anime25DParameter[])
      this.renderAt(at, neutral)
      const frame = this.captureFrame()
      onFrame?.(name, { dataUrl: this.canvas.toDataURL("image/png"), metrics: frame.metrics })
      const delta = baseline ? countChanged(baseline.image.data, frame.image.data) : { pixels: 0, alphaPixels: 0 }
      scenarios.push({ name, parameters: { ...this.parameters }, metrics: frame.metrics, changedPixels: delta.pixels, changedAlphaPixels: delta.alphaPixels, available: true })
      baseline ??= frame
    }

    this.poseMix = 0
    this.engine.setPoseMix(0)
    run("neutral", {}, baseTime, true)
    run("blink-left", { eyeOpenL: 0.1 })
    run("blink-right", { eyeOpenR: 0.1 })
    run("gaze-independent", { eyeX: 0.78, eyeY: -0.34 })
    run("mouth-independent", { mouthOpen: 0.88, mouthForm: 0.62, mouthCY: 0.16 })
    run("head-independent", { angleX: 0.72, angleY: -0.48, angleZ: 0.45 })
    run("breath-engine", {}, baseTime + 1200, false)
    if (this.bundle.pose) {
      this.poseMix = 0.5
      this.engine.setPoseMix(0.5)
      run("pose-crossfade-fbo", {})
      this.poseMix = 1
      this.engine.setPoseMix(1)
      run("alternate-pose", {})
    } else {
      scenarios.push({ name: "alternate-pose", parameters: { ...this.parameters }, metrics: this.metrics(0), changedPixels: 0, changedAlphaPixels: 0, available: false })
    }

    this.parameters = savedParameters
    this.manual = savedManual
    this.poseMix = savedPoseMix
    this.engine.setPoseMix(savedPoseMix)
    this.renderAt(performance.now(), false)
    if (wasRunning && this.visible) this.start()
    const active = scenarios.filter(scenario => scenario.available)
    const ok = active.length >= 6 && active.slice(1).every(scenario => scenario.changedPixels > 0)
    return {
      ok,
      scenarios,
      diagnostics: this.diagnostics(),
    }
  }

  dispose(): { disposed: true } {
    if (!this.disposed) {
      this.stop()
      this.engine.unload()
      this.bundle = null
      this.disposed = true
    }
    return { disposed: true }
  }

  start() {
    if (this.running || this.disposed || !this.visible || !this.loaded) return
    this.running = true
    this.frameHandle = requestAnimationFrame(this.frame)
  }

  stop() {
    this.running = false
    if (this.frameHandle) cancelAnimationFrame(this.frameHandle)
    this.frameHandle = 0
  }

  metrics(readbackMs = 0, image?: RigImage): RigMetrics {
    const frame = image ?? null
    let alphaPixels = 0
    if (frame) for (let index = 3; index < frame.data.length; index += 4) if (frame.data[index] > 0) alphaPixels++
    return {
      canvas: { width: this.canvas.width, height: this.canvas.height, dpr: window.devicePixelRatio || 1 },
      webgl: { version: "WebGL1", stencil: this.engine.stencilEnabled, framebufferCount: this.engine.framebufferCount, renderbufferCount: this.engine.renderbufferCount },
      layerCount: this.bundle?.base.model.rig.layers.length ?? 0,
      meshCount: this.engine.meshCount,
      textureCount: this.engine.textureCount,
      timings: { renderMs: this.lastRenderMs, readbackMs, loadMs: this.lastLoadMs },
      alphaPixels,
      alphaCursor: this.pointerAlpha || this.engine.sampleAlpha(Math.floor(this.canvas.width / 2), Math.floor(this.canvas.height / 2)),
      cursor: { ...this.cursor },
    }
  }

  diagnostics(): Record<string, unknown> {
    const model = this.bundle?.base.model
    const rig = model?.rig
    return {
      renderer: "Anime2.5DRig WebGL1",
      rigger: "Rigger.buildRig",
      upstreamCommit: "d48825867acd081de22b0e7b5585bb562288796d",
      webglStencil: this.engine.stencilEnabled,
      psdCanvas: rig ? `${rig.canvas.w}x${rig.canvas.h}` : null,
      psdLayerCount: model?.psdLayers.length ?? 0,
      layerNames: rig?.layers.map(layer => layer.name) ?? [],
      missingRequiredLayers: model?.missingRequiredLayers ?? [],
      rigLayerCount: rig?.layers.length ?? 0,
      meshCount: this.engine.meshCount,
      textureCount: this.engine.textureCount,
      headLayers: rig?.layers.filter(layer => layer.group === "head").length ?? 0,
      bodyLayers: rig?.layers.filter(layer => layer.group === "body").length ?? 0,
      eyeLayers: rig?.layers.filter(layer => /^(eyewhite|irides|eyelash|eye_close)/.test(layer.name)).length ?? 0,
      hairLayers: rig?.layers.filter(layer => layer.phys === "hair").length ?? 0,
      hairStrandCount: rig?.layers.reduce((count, layer) => count + (layer.strands?.length ?? 0), 0) ?? 0,
      syntheticEyeClose: Boolean(rig?.synth.eye),
      syntheticMouthClose: Boolean(rig?.synth.mouth),
      warnings: [...(rig?.warnings ?? []), ...(model?.assetDiagnostics?.warnings ?? [])],
      layerDiagnostics: model?.assetDiagnostics ?? null,
      poseLoaded: Boolean(this.bundle?.pose),
    }
  }

  private readonly frame = (now: number) => {
    if (!this.running || this.disposed) return
    this.renderAt(now, false)
    this.frameHandle = requestAnimationFrame(this.frame)
  }

  private renderAt(now: number, neutral: boolean) {
    if (!this.bundle) return
    const started = performance.now()
    const next = this.animatedParameters(now, neutral)
    this.engine.render(next, now, neutral)
    this.lastRenderMs = performance.now() - started
  }

  private animatedParameters(now: number, neutral: boolean): Anime25DParameterState {
    if (neutral) return { ...this.parameters }
    const seconds = now / 1000
    const automatic: ParameterRecord = {
      eyeOpenL: 1 - smoothPulse(seconds, 4.4, 0.03) * 0.92,
      eyeOpenR: 1 - smoothPulse(seconds, 4.4, 0.11) * 0.92,
      eyeX: Math.sin(seconds * 0.73) * 0.38,
      eyeY: Math.sin(seconds * 0.49 + 0.4) * 0.18,
      mouthOpen: Math.max(0, Math.sin(seconds * 0.91) * 0.12),
      mouthForm: Math.sin(seconds * 0.31) * 0.16,
      angleX: Math.sin(seconds * 0.37) * 0.12,
      angleY: Math.sin(seconds * 0.29 + 0.8) * 0.1,
      angleZ: Math.sin(seconds * 0.23) * 0.08,
      body: Math.sin(seconds * 0.17) * 0.05,
      armY: Math.sin(seconds * 0.41) * 0.08,
      armPos: Math.cos(seconds * 0.34) * 0.08,
      bangL: Math.sin(seconds * 0.61) * 0.12,
      bangC: Math.sin(seconds * 0.53 + 0.3) * 0.1,
      bangR: Math.sin(seconds * 0.47 + 0.6) * 0.12,
      bustY: Math.sin(seconds * 0.52) * 0.18,
    }
    for (const [name, value] of Object.entries(automatic)) if (!this.manual.has(name as Anime25DParameter)) this.parameters[name as Anime25DParameter] = clampParameter(name as Anime25DParameter, value as number)
    return { ...this.parameters }
  }

  private captureFrame(): FrameSample {
    const started = performance.now()
    this.engine.gl.finish()
    const image = this.engine.readFrame()
    const readbackMs = performance.now() - started
    return { image, metrics: this.metrics(readbackMs, image) }
  }
}

export { PARAMETER_RANGES }
