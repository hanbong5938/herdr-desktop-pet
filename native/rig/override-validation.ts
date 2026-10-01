import limits from "./limits.json"
import type { RigOverrides } from "../../web/rig/vendor/anime25drig/RigOverrides"

/**
 * This module deliberately does not use JSON.parse.  The native worker runs the
 * bundle in JavaScriptCore, and JSON.parse silently keeps the last occurrence
 * of a duplicate key (including keys which become equal after a unicode
 * escape).  The small parser below also lets us enforce the tree budget before
 * creating the object graph consumed by the pinned loader.
 */

const MAX_JSON_DEPTH = 16
const MAX_JSON_NODES = 100_000
const MAX_ARRAY_LENGTH = 4_096
const MAX_OBJECT_KEYS = 2_048
const MAX_STRING_LENGTH = 8_192
const MAX_NAME_LENGTH = 512
const MAX_NUMBER = 1e9
const MAX_ERROR_LENGTH = 256
const MAX_OVERRIDE_BYTES = limits.overrideBytes
const MAX_REFERENCE_ENTRIES = limits.outputLayers
const MAX_LAYER_NAMES = limits.psdLayers + limits.outputLayers
const MAX_REPAIR_ENTRIES = limits.outputLayers
const MAX_OVERLAY_ENTRIES = limits.outputLayers
const MAX_POLYGON_POINTS = 64
const MAX_PROFILE_SAMPLES = 64

const ROOT_FIELDS = [
  "layerAliases",
  "layerOrder",
  "layerOrderConstraints",
  "maskedLayerOverlays",
  "interpolatedPatchRepairs",
  "depthOverrides",
  "groupOverrides",
  "deformationSources",
  "meshSources",
  "hairAttachments",
  "headFollow",
  "cleanupThresholds",
  "hiddenLayers",
  "excludeAfterMeshResolution",
  "anchorOverrides",
  "interactionAreas",
  "mouthExpressions",
  "blinkRepair",
  "hairSplit",
  "physics",
] as const

export type NativeRigLayerNames = {
  /** Flat PSD pixel-layer names before loader filtering and canonicalization. */
  raw: Iterable<string>
  /** Names visible to loader overrides, including generated rig layers. */
  loader: Iterable<string>
}
type JsonObject = { [key: string]: unknown }
type LayerRecord = { name: string; normalized: string; base: string }
type Graph = Map<string, Set<string>>

const hasOwn = Object.prototype.hasOwnProperty
const DANGEROUS_KEYS = new Set(["__proto__", "prototype", "constructor"])

function fail(message: string): never {
  const safe = message.length > MAX_ERROR_LENGTH ? `${message.slice(0, MAX_ERROR_LENGTH - 1)}…` : message
  throw new Error(`RIG_OVERRIDE_INVALID: ${safe}`)
}

function isRecord(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value)
}

function own(value: JsonObject, key: string): boolean {
  return hasOwn.call(value, key)
}

function validUnicodeString(value: string): boolean {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index)
    if (code >= 0xd800 && code <= 0xdbff) {
      const next = value.charCodeAt(index + 1)
      if (next < 0xdc00 || next > 0xdfff) return false
      index += 1
    } else if (code >= 0xdc00 && code <= 0xdfff) {
      return false
    }
  }
  return true
}

function assertString(value: unknown, label: string, max = MAX_STRING_LENGTH): asserts value is string {
  if (typeof value !== "string" || value.length > max || !validUnicodeString(value) || value.includes("\u0000")) {
    fail(`${label} must be a bounded UTF-8 string`)
  }
}

function assertName(value: unknown, label: string): asserts value is string {
  assertString(value, label, MAX_NAME_LENGTH)
  if (value.length === 0 || value.normalize("NFKC").trim().length === 0) fail(`${label} must not be empty`)
}

function assertNumber(value: unknown, label: string, minimum?: number, maximum?: number): asserts value is number {
  if (typeof value !== "number" || !Number.isFinite(value) || Math.abs(value) > MAX_NUMBER) {
    fail(`${label} must be finite and bounded`)
  }
  if (minimum !== undefined && value < minimum) fail(`${label} is below its minimum`)
  if (maximum !== undefined && value > maximum) fail(`${label} is above its maximum`)
}

function assertBoolean(value: unknown, label: string): asserts value is boolean {
  if (typeof value !== "boolean") fail(`${label} must be boolean`)
}

function assertExactObject(value: unknown, label: string, allowed: readonly string[]): JsonObject {
  if (!isRecord(value)) fail(`${label} must be an object`)
  const allowedSet = new Set(allowed)
  for (const key of Object.keys(value)) {
    if (!allowedSet.has(key)) fail(`${label} contains unknown field`)
  }
  return value
}

function required(object: JsonObject, key: string, label: string): unknown {
  if (!own(object, key)) fail(`${label}.${key} is required`)
  return object[key]
}


function assertArray(value: unknown, label: string, minimum: number, maximum = MAX_ARRAY_LENGTH): unknown[] {
  if (!Array.isArray(value) || value.length < minimum || value.length > maximum) {
    fail(`${label} array length is outside [${minimum}, ${maximum}]`)
  }
  return value
}

function assertPoint(value: unknown, label: string): void {
  const point = assertExactObject(value, label, ["cx", "cy"])
  assertNumber(required(point, "cx", label), `${label}.cx`)
  assertNumber(required(point, "cy", label), `${label}.cy`)
}

function assertBounds(value: unknown, label: string): void {
  const bounds = assertExactObject(value, label, ["x0", "y0", "x1", "y1"])
  const x0 = required(bounds, "x0", label)
  const y0 = required(bounds, "y0", label)
  const x1 = required(bounds, "x1", label)
  const y1 = required(bounds, "y1", label)
  assertNumber(x0, `${label}.x0`)
  assertNumber(y0, `${label}.y0`)
  assertNumber(x1, `${label}.x1`)
  assertNumber(y1, `${label}.y1`)
  if (x1 < x0 || y1 < y0) fail(`${label} has inverted bounds`)
}

function assertFaceBounds(value: unknown, label: string): void {
  const face = assertExactObject(value, label, ["x0", "y0", "x1", "y1", "cx", "cy"])
  const x0 = required(face, "x0", label)
  const y0 = required(face, "y0", label)
  const x1 = required(face, "x1", label)
  const y1 = required(face, "y1", label)
  assertNumber(x0, `${label}.x0`)
  assertNumber(y0, `${label}.y0`)
  assertNumber(x1, `${label}.x1`)
  assertNumber(y1, `${label}.y1`)
  assertNumber(required(face, "cx", label), `${label}.cx`)
  assertNumber(required(face, "cy", label), `${label}.cy`)
  if (x1 < x0 || y1 < y0) fail(`${label} has inverted bounds`)
}

function assertPolygon(value: unknown, label: string): void {
  const polygon = assertArray(value, label, 3, MAX_POLYGON_POINTS)
  for (let index = 0; index < polygon.length; index += 1) {
    const point = assertArray(polygon[index], `${label}[${index}]`, 2, 2)
    assertNumber(point[0], `${label}[${index}][0]`)
    assertNumber(point[1], `${label}[${index}][1]`)
  }
}

function assertNamedMap(value: unknown, label: string, validateValue: (value: unknown, label: string) => void): void {
  if (!isRecord(value)) fail(`${label} must be an object`)
  const keys = Object.keys(value)
  if (keys.length > MAX_REFERENCE_ENTRIES) fail(`${label} contains too many references`)
  for (const key of keys) {
    assertName(key, `${label} key`)
    validateValue(value[key], `${label}.${key}`)
  }
}

function assertStringArray(value: unknown, label: string): void {
  const values = assertArray(value, label, 0, MAX_REFERENCE_ENTRIES)
  for (let index = 0; index < values.length; index += 1) assertName(values[index], `${label}[${index}]`)
}

function assertEyeBlink(value: unknown, label: string): void {
  const blink = assertExactObject(value, label, [
    "center",
    "angleDeg",
    "u0",
    "u1",
    "upper",
    "lower",
    "closed",
    "closedSource",
    "closedTarget",
    "closedRotationDeg",
  ])
  assertPoint(required(blink, "center", label), `${label}.center`)
  const angle = required(blink, "angleDeg", label)
  const u0 = required(blink, "u0", label)
  const u1 = required(blink, "u1", label)
  assertNumber(angle, `${label}.angleDeg`)
  assertNumber(u0, `${label}.u0`)
  assertNumber(u1, `${label}.u1`)
  if (u1 <= u0) fail(`${label}.u1 must be greater than u0`)
  const upper = assertArray(required(blink, "upper", label), `${label}.upper`, 2, MAX_PROFILE_SAMPLES)
  const lower = assertArray(required(blink, "lower", label), `${label}.lower`, 2, MAX_PROFILE_SAMPLES)
  const closed = assertArray(required(blink, "closed", label), `${label}.closed`, 2, MAX_PROFILE_SAMPLES)
  if (upper.length !== lower.length || upper.length !== closed.length) fail(`${label} curve lengths must match`)
  for (let index = 0; index < upper.length; index += 1) {
    const upperValue = upper[index]
    const lowerValue = lower[index]
    assertNumber(upperValue, `${label}.upper[${index}]`)
    assertNumber(lowerValue, `${label}.lower[${index}]`)
    assertNumber(closed[index], `${label}.closed[${index}]`)
    if (lowerValue <= upperValue) fail(`${label}.lower must stay above upper`)
  }
  assertPoint(required(blink, "closedSource", label), `${label}.closedSource`)
  assertPoint(required(blink, "closedTarget", label), `${label}.closedTarget`)
  assertNumber(required(blink, "closedRotationDeg", label), `${label}.closedRotationDeg`)
}

function assertEyeAnchor(value: unknown, label: string): void {
  const eye = assertExactObject(value, label, ["x0", "y0", "x1", "y1", "icx", "icy", "closeY", "blink"])
  const x0 = required(eye, "x0", label)
  const y0 = required(eye, "y0", label)
  const x1 = required(eye, "x1", label)
  const y1 = required(eye, "y1", label)
  assertNumber(x0, `${label}.x0`)
  assertNumber(y0, `${label}.y0`)
  assertNumber(x1, `${label}.x1`)
  assertNumber(y1, `${label}.y1`)
  if (x1 < x0 || y1 < y0) fail(`${label} has inverted bounds`)
  assertNumber(required(eye, "icx", label), `${label}.icx`)
  assertNumber(required(eye, "icy", label), `${label}.icy`)
  assertNumber(required(eye, "closeY", label), `${label}.closeY`)
  if (own(eye, "blink")) assertEyeBlink(eye.blink, `${label}.blink`)
}

function assertMouthShape(value: unknown, label: string, closed: boolean): void {
  const shape = assertExactObject(value, label, ["u0", "u1", "upper", "lower"])
  const u0 = required(shape, "u0", label)
  const u1 = required(shape, "u1", label)
  assertNumber(u0, `${label}.u0`)
  assertNumber(u1, `${label}.u1`)
  if (u1 <= u0) fail(`${label}.u1 must be greater than u0`)
  const upper = assertArray(required(shape, "upper", label), `${label}.upper`, 2, MAX_PROFILE_SAMPLES)
  const lower = assertArray(required(shape, "lower", label), `${label}.lower`, 2, MAX_PROFILE_SAMPLES)
  if (upper.length !== lower.length) fail(`${label} curve lengths must match`)
  for (let index = 0; index < upper.length; index += 1) {
    const upperValue = upper[index]
    const lowerValue = lower[index]
    assertNumber(upperValue, `${label}.upper[${index}]`)
    assertNumber(lowerValue, `${label}.lower[${index}]`)
    if (closed ? lowerValue < upperValue : lowerValue <= upperValue) {
      fail(`${label}.lower must stay ${closed ? "at or above" : "above"} upper`)
    }
  }
}

function assertMouthMorph(value: unknown, label: string): void {
  const morph = assertExactObject(value, label, ["center", "angleDeg", "neutral", "open", "smile"])
  assertPoint(required(morph, "center", label), `${label}.center`)
  assertNumber(required(morph, "angleDeg", label), `${label}.angleDeg`)
  assertMouthShape(required(morph, "neutral", label), `${label}.neutral`, true)
  assertMouthShape(required(morph, "open", label), `${label}.open`, false)
  assertMouthShape(required(morph, "smile", label), `${label}.smile`, true)
}

function assertMouthAnchor(value: unknown, label: string): void {
  const mouth = assertExactObject(value, label, ["x0", "y0", "x1", "y1", "cx", "cy", "morph"])
  const x0 = required(mouth, "x0", label)
  const y0 = required(mouth, "y0", label)
  const x1 = required(mouth, "x1", label)
  const y1 = required(mouth, "y1", label)
  assertNumber(x0, `${label}.x0`)
  assertNumber(y0, `${label}.y0`)
  assertNumber(x1, `${label}.x1`)
  assertNumber(y1, `${label}.y1`)
  if (x1 < x0 || y1 < y0) fail(`${label} has inverted bounds`)
  assertNumber(required(mouth, "cx", label), `${label}.cx`)
  assertNumber(required(mouth, "cy", label), `${label}.cy`)
  if (own(mouth, "morph")) assertMouthMorph(mouth.morph, `${label}.morph`)
}

function assertAnchorOverrides(value: unknown, label: string): void {
  const anchors = assertExactObject(value, label, [
    "face",
    "eyeL",
    "eyeR",
    "mouth",
    "neckPivot",
    "bodyPivot",
    "neckTop",
    "neckBottom",
    "hairRootY",
    "faceScale",
  ])
  if (own(anchors, "face")) assertFaceBounds(anchors.face, `${label}.face`)
  if (own(anchors, "eyeL")) assertEyeAnchor(anchors.eyeL, `${label}.eyeL`)
  if (own(anchors, "eyeR")) assertEyeAnchor(anchors.eyeR, `${label}.eyeR`)
  if (own(anchors, "mouth")) assertMouthAnchor(anchors.mouth, `${label}.mouth`)
  if (own(anchors, "neckPivot")) assertPoint(anchors.neckPivot, `${label}.neckPivot`)
  if (own(anchors, "bodyPivot")) assertPoint(anchors.bodyPivot, `${label}.bodyPivot`)
  for (const key of ["neckTop", "neckBottom", "hairRootY"] as const) {
    if (own(anchors, key)) assertNumber(anchors[key], `${label}.${key}`)
  }
  if (own(anchors, "faceScale")) assertNumber(anchors.faceScale, `${label}.faceScale`, Number.MIN_VALUE)
}

function assertInteractionAreas(value: unknown, label: string): void {
  const areas = assertExactObject(value, label, ["face", "head", "torso"])
  for (const key of ["face", "head", "torso"] as const) if (own(areas, key)) assertBounds(areas[key], `${label}.${key}`)
}

const PHYSICS_FIELDS = ["amplitude", "stiffness", "damping", "wind", "inertia", "rootLock", "maxOffset"] as const

function assertPhysicsTuning(value: unknown, label: string): void {
  const tuning = assertExactObject(value, label, PHYSICS_FIELDS)
  for (const key of PHYSICS_FIELDS) if (own(tuning, key)) assertNumber(tuning[key], `${label}.${key}`)
}

function assertPhysics(value: unknown, label: string): void {
  const physics = assertExactObject(value, label, ["frontHair", "backHair", "layers"])
  if (own(physics, "frontHair")) assertPhysicsTuning(physics.frontHair, `${label}.frontHair`)
  if (own(physics, "backHair")) assertPhysicsTuning(physics.backHair, `${label}.backHair`)
  if (own(physics, "layers")) {
    assertNamedMap(physics.layers, `${label}.layers`, assertPhysicsTuning)
  }
}

function assertOverrideShape(value: unknown, label: string): asserts value is RigOverrides {
  const root = assertExactObject(value, label, ROOT_FIELDS)
  if (own(root, "layerAliases")) assertNamedMap(root.layerAliases, `${label}.layerAliases`, (entry, entryLabel) => assertName(entry, entryLabel))
  if (own(root, "layerOrder")) assertStringArray(root.layerOrder, `${label}.layerOrder`)
  if (own(root, "layerOrderConstraints")) {
    const constraints = assertArray(root.layerOrderConstraints, `${label}.layerOrderConstraints`, 0, MAX_REFERENCE_ENTRIES)
    for (let index = 0; index < constraints.length; index += 1) {
      const constraint = assertExactObject(constraints[index], `${label}.layerOrderConstraints[${index}]`, ["behind", "inFrontOf"])
      assertName(required(constraint, "behind", `${label}.layerOrderConstraints[${index}]`), `${label}.layerOrderConstraints[${index}].behind`)
      assertName(required(constraint, "inFrontOf", `${label}.layerOrderConstraints[${index}]`), `${label}.layerOrderConstraints[${index}].inFrontOf`)
    }
  }
  if (own(root, "maskedLayerOverlays")) {
    const overlays = assertArray(root.maskedLayerOverlays, `${label}.maskedLayerOverlays`, 0, MAX_OVERLAY_ENTRIES)
    for (let index = 0; index < overlays.length; index += 1) {
      const itemLabel = `${label}.maskedLayerOverlays[${index}]`
      const overlay = assertExactObject(overlays[index], itemLabel, [
        "source",
        "name",
        "inFrontOf",
        "polygon",
        "replaceExisting",
        "featherPx",
        "textureSource",
        "excludeConnectedNeutral",
      ])
      assertName(required(overlay, "source", itemLabel), `${itemLabel}.source`)
      assertName(required(overlay, "name", itemLabel), `${itemLabel}.name`)
      assertName(required(overlay, "inFrontOf", itemLabel), `${itemLabel}.inFrontOf`)
      assertPolygon(required(overlay, "polygon", itemLabel), `${itemLabel}.polygon`)
      if (own(overlay, "replaceExisting")) assertBoolean(overlay.replaceExisting, `${itemLabel}.replaceExisting`)
      if (own(overlay, "featherPx")) assertNumber(overlay.featherPx, `${itemLabel}.featherPx`, 0)
      if (own(overlay, "textureSource")) assertName(overlay.textureSource, `${itemLabel}.textureSource`)
      if (own(overlay, "excludeConnectedNeutral")) {
        const neutralLabel = `${itemLabel}.excludeConnectedNeutral`
        const neutral = assertExactObject(overlay.excludeConnectedNeutral, neutralLabel, ["maxChroma", "minLuminance", "maxLuminance", "maxColorStep"])
        const minimum = required(neutral, "minLuminance", neutralLabel)
        const maximum = required(neutral, "maxLuminance", neutralLabel)
        assertNumber(minimum, `${neutralLabel}.minLuminance`, 0)
        assertNumber(maximum, `${neutralLabel}.maxLuminance`, 0)
        assertNumber(required(neutral, "maxChroma", neutralLabel), `${neutralLabel}.maxChroma`, 0)
        assertNumber(required(neutral, "maxColorStep", neutralLabel), `${neutralLabel}.maxColorStep`, 0)
        if (maximum < minimum) fail(`${neutralLabel} luminance bounds are inverted`)
      }
    }
  }
  if (own(root, "interpolatedPatchRepairs")) {
    const repairs = assertArray(root.interpolatedPatchRepairs, `${label}.interpolatedPatchRepairs`, 0, MAX_REPAIR_ENTRIES)
    for (let index = 0; index < repairs.length; index += 1) {
      const itemLabel = `${label}.interpolatedPatchRepairs[${index}]`
      const repair = assertExactObject(repairs[index], itemLabel, ["layer", "polygon", "axis"])
      assertName(required(repair, "layer", itemLabel), `${itemLabel}.layer`)
      assertPolygon(required(repair, "polygon", itemLabel), `${itemLabel}.polygon`)
      const axis = required(repair, "axis", itemLabel)
      if (axis !== "horizontal") fail(`${itemLabel}.axis must be horizontal`)
    }
  }
  if (own(root, "depthOverrides")) assertNamedMap(root.depthOverrides, `${label}.depthOverrides`, (entry, entryLabel) => assertNumber(entry, entryLabel))
  if (own(root, "groupOverrides")) assertNamedMap(root.groupOverrides, `${label}.groupOverrides`, (entry, entryLabel) => assertName(entry, entryLabel))
  if (own(root, "deformationSources")) assertNamedMap(root.deformationSources, `${label}.deformationSources`, (entry, entryLabel) => assertName(entry, entryLabel))
  if (own(root, "meshSources")) assertNamedMap(root.meshSources, `${label}.meshSources`, (entry, entryLabel) => assertName(entry, entryLabel))
  if (own(root, "hairAttachments")) {
    assertNamedMap(root.hairAttachments, `${label}.hairAttachments`, (entry, entryLabel) => {
      const attachment = assertExactObject(entry, entryLabel, ["rootY", "bodyY"])
      const rootY = required(attachment, "rootY", entryLabel)
      const bodyY = required(attachment, "bodyY", entryLabel)
      assertNumber(rootY, `${entryLabel}.rootY`)
      assertNumber(bodyY, `${entryLabel}.bodyY`)
      if (bodyY <= rootY) fail(`${entryLabel}.bodyY must be greater than rootY`)
    })
  }
  if (own(root, "headFollow")) {
    assertNamedMap(root.headFollow, `${label}.headFollow`, (entry, entryLabel) => {
      const follow = assertExactObject(entry, entryLabel, ["center", "radius", "falloffRadius"])
      assertPoint(required(follow, "center", entryLabel), `${entryLabel}.center`)
      const radius = required(follow, "radius", entryLabel)
      const falloffRadius = required(follow, "falloffRadius", entryLabel)
      assertNumber(radius, `${entryLabel}.radius`, Number.MIN_VALUE)
      assertNumber(falloffRadius, `${entryLabel}.falloffRadius`)
      if (falloffRadius <= radius) fail(`${entryLabel}.falloffRadius must be greater than radius`)
    })
  }
  if (own(root, "cleanupThresholds")) assertNamedMap(root.cleanupThresholds, `${label}.cleanupThresholds`, (entry, entryLabel) => assertNumber(entry, entryLabel))
  if (own(root, "hiddenLayers")) assertStringArray(root.hiddenLayers, `${label}.hiddenLayers`)
  if (own(root, "excludeAfterMeshResolution")) assertStringArray(root.excludeAfterMeshResolution, `${label}.excludeAfterMeshResolution`)
  if (own(root, "anchorOverrides")) assertAnchorOverrides(root.anchorOverrides, `${label}.anchorOverrides`)
  if (own(root, "interactionAreas")) assertInteractionAreas(root.interactionAreas, `${label}.interactionAreas`)
  if (own(root, "mouthExpressions")) {
    const expressions = assertExactObject(root.mouthExpressions, `${label}.mouthExpressions`, ["neutral", "open", "smile"])
    for (const key of ["neutral", "open", "smile"] as const) assertName(required(expressions, key, `${label}.mouthExpressions`), `${label}.mouthExpressions.${key}`)
  }
  if (own(root, "blinkRepair")) {
    const repair = assertExactObject(root.blinkRepair, `${label}.blinkRepair`, ["enabled", "paddingX", "paddingY"])
    if (own(repair, "enabled")) assertBoolean(repair.enabled, `${label}.blinkRepair.enabled`)
    if (own(repair, "paddingX")) assertNumber(repair.paddingX, `${label}.blinkRepair.paddingX`, 0)
    if (own(repair, "paddingY")) assertNumber(repair.paddingY, `${label}.blinkRepair.paddingY`, 0)
  }
  if (own(root, "hairSplit")) {
    const split = assertExactObject(root.hairSplit, `${label}.hairSplit`, ["backHairLeftRight", "centerX"])
    if (own(split, "backHairLeftRight")) assertBoolean(split.backHairLeftRight, `${label}.hairSplit.backHairLeftRight`)
    if (own(split, "centerX")) assertNumber(split.centerX, `${label}.hairSplit.centerX`)
  }
  if (own(root, "physics")) assertPhysics(root.physics, `${label}.physics`)
}

function assertTree(value: unknown, label: string, depth = 0, state = { nodes: 0 }, active = new WeakSet<object>()): void {
  if (depth > MAX_JSON_DEPTH) fail(`${label} nesting exceeds cap`)
  state.nodes += 1
  if (state.nodes > MAX_JSON_NODES) fail(`${label} node count exceeds cap`)
  if (value === null || typeof value === "boolean") return
  if (typeof value === "number") {
    assertNumber(value, label)
    return
  }
  if (typeof value === "string") {
    assertString(value, label)
    return
  }
  if (typeof value !== "object") fail(`${label} contains an unsupported value`)
  if (active.has(value)) fail(`${label} contains a cyclic object graph`)
  active.add(value)
  try {
    if (Array.isArray(value)) {
      if (value.length > MAX_ARRAY_LENGTH) fail(`${label} array exceeds cap`)
      for (let index = 0; index < value.length; index += 1) assertTree(value[index], `${label}[${index}]`, depth + 1, state, active)
    } else {
      const keys = Object.keys(value)
      if (keys.length > MAX_OBJECT_KEYS) fail(`${label} object exceeds cap`)
      for (const key of keys) {
        if (key.length > MAX_STRING_LENGTH || DANGEROUS_KEYS.has(key) || key.startsWith("__")) fail(`${label} contains an unsafe key`)
        assertTree((value as Record<string, unknown>)[key], `${label}.${key}`, depth + 1, state, active)
      }
    }
  } finally {
    active.delete(value)
  }
}

class JsonParser {
  private index = 0
  private nodes = 0

  constructor(private readonly source: string) {}

  parse(): unknown {
    this.skipWhitespace()
    if (this.index >= this.source.length) fail("override JSON is empty")
    const result = this.value(0)
    this.skipWhitespace()
    if (this.index !== this.source.length) fail("override JSON has trailing data")
    return result
  }

  private value(depth: number): unknown {
    if (depth > MAX_JSON_DEPTH) fail("override JSON nesting exceeds cap")
    this.nodes += 1
    if (this.nodes > MAX_JSON_NODES) fail("override JSON node count exceeds cap")
    const code = this.source.charCodeAt(this.index)
    if (code === 0x22) return this.string()
    if (code === 0x7b) return this.object(depth)
    if (code === 0x5b) return this.array(depth)
    if (code === 0x74 && this.source.startsWith("true", this.index)) {
      this.index += 4
      return true
    }
    if (code === 0x66 && this.source.startsWith("false", this.index)) {
      this.index += 5
      return false
    }
    if (code === 0x6e && this.source.startsWith("null", this.index)) {
      this.index += 4
      return null
    }
    if (code === 0x2d || (code >= 0x30 && code <= 0x39)) return this.number()
    fail("override JSON contains an invalid value")
  }

  private object(depth: number): JsonObject {
    this.index += 1
    const result: JsonObject = Object.create(null)
    const keys = new Set<string>()
    this.skipWhitespace()
    if (this.source.charCodeAt(this.index) === 0x7d) {
      this.index += 1
      return result
    }
    while (true) {
      this.skipWhitespace()
      if (this.source.charCodeAt(this.index) !== 0x22) fail("override JSON object key must be a string")
      const key = this.string()
      if (DANGEROUS_KEYS.has(key) || key.startsWith("__")) fail("override JSON contains an unsafe key")
      if (keys.has(key)) fail("override JSON contains a duplicate object key")
      if (keys.size >= MAX_OBJECT_KEYS) fail("override JSON object exceeds cap")
      keys.add(key)
      this.skipWhitespace()
      if (this.source.charCodeAt(this.index) !== 0x3a) fail("override JSON object key is missing a colon")
      this.index += 1
      this.skipWhitespace()
      result[key] = this.value(depth + 1)
      this.skipWhitespace()
      const delimiter = this.source.charCodeAt(this.index)
      if (delimiter === 0x7d) {
        this.index += 1
        return result
      }
      if (delimiter !== 0x2c) fail("override JSON object is missing a delimiter")
      this.index += 1
    }
  }

  private array(depth: number): unknown[] {
    this.index += 1
    const result: unknown[] = []
    this.skipWhitespace()
    if (this.source.charCodeAt(this.index) === 0x5d) {
      this.index += 1
      return result
    }
    while (true) {
      if (result.length >= MAX_ARRAY_LENGTH) fail("override JSON array exceeds cap")
      this.skipWhitespace()
      result.push(this.value(depth + 1))
      this.skipWhitespace()
      const delimiter = this.source.charCodeAt(this.index)
      if (delimiter === 0x5d) {
        this.index += 1
        return result
      }
      if (delimiter !== 0x2c) fail("override JSON array is missing a delimiter")
      this.index += 1
    }
  }

  private string(): string {
    if (this.source.charCodeAt(this.index) !== 0x22) fail("override JSON string is malformed")
    this.index += 1
    let result = ""
    while (this.index < this.source.length) {
      const code = this.source.charCodeAt(this.index)
      if (code === 0x22) {
        this.index += 1
        if (!validUnicodeString(result)) fail("override JSON contains an unpaired surrogate")
        if (result.length > MAX_STRING_LENGTH || result.includes("\u0000")) fail("override JSON string exceeds its safety bounds")
        return result
      }
      if (code < 0x20) fail("override JSON contains an unescaped control character")
      if (code !== 0x5c) {
        if (code >= 0xd800 && code <= 0xdbff) {
          const next = this.source.charCodeAt(this.index + 1)
          if (next < 0xdc00 || next > 0xdfff) fail("override JSON contains an unpaired surrogate")
          result += this.source.slice(this.index, this.index + 2)
          this.index += 2
        } else if (code >= 0xdc00 && code <= 0xdfff) {
          fail("override JSON contains an unpaired surrogate")
        } else {
          result += this.source[this.index]
          this.index += 1
        }
      } else {
        result += this.escape()
      }
      if (result.length > MAX_STRING_LENGTH) fail("override JSON string exceeds cap")
    }
    fail("override JSON string is unterminated")
  }

  private escape(): string {
    this.index += 1
    if (this.index >= this.source.length) fail("override JSON escape is truncated")
    const code = this.source.charCodeAt(this.index)
    this.index += 1
    switch (code) {
      case 0x22: return '"'
      case 0x5c: return "\\"
      case 0x2f: return "/"
      case 0x62: return "\b"
      case 0x66: return "\f"
      case 0x6e: return "\n"
      case 0x72: return "\r"
      case 0x74: return "\t"
      case 0x75: {
        const first = this.hex4()
        if (first >= 0xd800 && first <= 0xdbff) {
          if (this.source.charCodeAt(this.index) !== 0x5c || this.source.charCodeAt(this.index + 1) !== 0x75) {
            fail("override JSON contains an unpaired surrogate")
          }
          this.index += 2
          const second = this.hex4()
          if (second < 0xdc00 || second > 0xdfff) fail("override JSON contains an unpaired surrogate")
          return String.fromCharCode(first, second)
        }
        if (first >= 0xdc00 && first <= 0xdfff) fail("override JSON contains an unpaired surrogate")
        return String.fromCharCode(first)
      }
      default: fail("override JSON contains an invalid escape")
    }
  }

  private hex4(): number {
    if (this.index + 4 > this.source.length) fail("override JSON unicode escape is truncated")
    let result = 0
    for (let offset = 0; offset < 4; offset += 1) {
      const code = this.source.charCodeAt(this.index + offset)
      let digit: number
      if (code >= 0x30 && code <= 0x39) digit = code - 0x30
      else if (code >= 0x41 && code <= 0x46) digit = code - 0x41 + 10
      else if (code >= 0x61 && code <= 0x66) digit = code - 0x61 + 10
      else fail("override JSON unicode escape is invalid")
      result = result * 16 + digit
    }
    this.index += 4
    return result
  }

  private number(): number {
    const start = this.index
    if (this.source.charCodeAt(this.index) === 0x2d) this.index += 1
    const first = this.source.charCodeAt(this.index)
    if (first === 0x30) {
      this.index += 1
      const next = this.source.charCodeAt(this.index)
      if (next >= 0x30 && next <= 0x39) fail("override JSON number has a leading zero")
    } else {
      if (first < 0x31 || first > 0x39) fail("override JSON number is malformed")
      while (this.isDigit(this.source.charCodeAt(this.index))) this.index += 1
    }
    if (this.source.charCodeAt(this.index) === 0x2e) {
      this.index += 1
      if (!this.isDigit(this.source.charCodeAt(this.index))) fail("override JSON fraction is malformed")
      while (this.isDigit(this.source.charCodeAt(this.index))) this.index += 1
    }
    const exponent = this.source.charCodeAt(this.index)
    if (exponent === 0x65 || exponent === 0x45) {
      this.index += 1
      const sign = this.source.charCodeAt(this.index)
      if (sign === 0x2b || sign === 0x2d) this.index += 1
      if (!this.isDigit(this.source.charCodeAt(this.index))) fail("override JSON exponent is malformed")
      while (this.isDigit(this.source.charCodeAt(this.index))) this.index += 1
    }
    const result = Number(this.source.slice(start, this.index))
    assertNumber(result, "override JSON number")
    return result
  }

  private isDigit(code: number): boolean {
    return code >= 0x30 && code <= 0x39
  }

  private skipWhitespace(): void {
    while (this.index < this.source.length) {
      const code = this.source.charCodeAt(this.index)
      if (code !== 0x20 && code !== 0x09 && code !== 0x0a && code !== 0x0d) return
      this.index += 1
    }
  }
}

function utf8ByteLength(value: string): number {
  let bytes = 0
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index)
    if (code <= 0x7f) bytes += 1
    else if (code <= 0x7ff) bytes += 2
    else if (code >= 0xd800 && code <= 0xdbff) {
      const next = value.charCodeAt(index + 1)
      if (next < 0xdc00 || next > 0xdfff) fail("override JSON is not valid UTF-8 text")
      bytes += 4
      index += 1
    } else if (code >= 0xdc00 && code <= 0xdfff) {
      fail("override JSON is not valid UTF-8 text")
    } else bytes += 3
    if (bytes > Number.MAX_SAFE_INTEGER - 4) fail("override JSON byte length overflows")
  }
  return bytes
}

function normalized(name: string): string {
  return name.normalize("NFKC").trim().toLowerCase()
}

function baseName(name: string): string {
  return normalized(name).replace(/[-_]([lr])$/i, "").replace(/_\d+$/, "")
}

function graphNode(value: string): string {
  return normalized(value)
}


function addGraphEdge(graph: Graph, from: string, to: string): void {
  let targets = graph.get(from)
  if (!targets) {
    targets = new Set<string>()
    graph.set(from, targets)
  }
  targets.add(to)
  if (!graph.has(to)) graph.set(to, new Set<string>())
}

function assertAcyclic(graph: Graph, label: string): void {
  const state = new Map<string, 0 | 1 | 2>()
  for (const start of graph.keys()) {
    if (state.get(start) === 2) continue
    const stack: Array<{ node: string; exit: boolean }> = [{ node: start, exit: false }]
    while (stack.length) {
      const current = stack.pop()!
      if (current.exit) {
        state.set(current.node, 2)
        continue
      }
      const currentState = state.get(current.node) ?? 0
      if (currentState === 1) fail(`${label} contains a cycle`)
      if (currentState === 2) continue
      state.set(current.node, 1)
      stack.push({ node: current.node, exit: true })
      const targets = graph.get(current.node)
      if (!targets) continue
      for (const target of targets) {
        const targetState = state.get(target) ?? 0
        if (targetState === 1) fail(`${label} contains a cycle`)
        if (targetState === 0) stack.push({ node: target, exit: false })
      }
    }
  }
}

function localGraphForMap(value: Record<string, string> | undefined, label: string): Graph {
  const graph: Graph = new Map()
  if (!value) return graph
  const entries = Object.entries(value)
  for (const [key, target] of entries) {
    const from = graphNode(key)
    graphNode(target)
    if (!graph.has(from)) graph.set(from, new Set<string>())
  }
  for (const [key, target] of entries) {
    const targetNormalized = normalized(target)
    const targetKey = entries.find(([candidate]) => normalized(candidate) === targetNormalized || baseName(candidate) === targetNormalized)?.[0]
    if (targetKey !== undefined) addGraphEdge(graph, graphNode(key), graphNode(targetKey))
  }
  assertAcyclic(graph, label)
  return graph
}

function validateLocalGraphs(overrides: RigOverrides): void {
  localGraphForMap(overrides.layerAliases, "layer alias graph")
  localGraphForMap(overrides.deformationSources, "deformation source graph")
  localGraphForMap(overrides.meshSources, "mesh source graph")
  const orderGraph: Graph = new Map()
  for (const constraint of overrides.layerOrderConstraints ?? []) {
    addGraphEdge(orderGraph, graphNode(constraint.behind), graphNode(constraint.inFrontOf))
  }
  for (const overlay of overrides.maskedLayerOverlays ?? []) {
    addGraphEdge(orderGraph, graphNode(overlay.inFrontOf), graphNode(overlay.name))
  }
  assertAcyclic(orderGraph, "layer order graph")
}

function makeRecords(names: string[]): LayerRecord[] {
  if (names.length > MAX_LAYER_NAMES) fail("layerNames exceeds cap")
  const records: LayerRecord[] = []
  const seen = new Set<string>()
  for (const [index, name] of names.entries()) {
    const item = { name, normalized: normalized(name), base: baseName(name) }
    if (item.normalized.length === 0) fail(`layerNames[${index}] must not be empty`)
    if (seen.has(item.normalized)) fail("layerNames contain duplicate normalized names")
    seen.add(item.normalized)
    records.push(item)
  }
  return records
}

function exactMatches(records: LayerRecord[], requested: string): LayerRecord[] {
  const target = normalized(requested)
  return records.filter(record => record.normalized === target)
}

function baseMatches(records: LayerRecord[], requested: string): LayerRecord[] {
  const target = normalized(requested)
  const exact = exactMatches(records, requested)
  if (exact.length) return exact
  return records.filter(record => record.base === target)
}
function lookupCandidates(record: LayerRecord): string[] {
  return [record.name, record.normalized, record.base]
}

function lookupMapEntries<T>(map: Record<string, T> | undefined, records: LayerRecord[], label: string): Array<{ record: LayerRecord; key: string; value: T }> {
  if (!map) return []
  const used = new Set<string>()
  const output: Array<{ record: LayerRecord; key: string; value: T }> = []
  for (const record of records) {
    const key = lookupCandidates(record).find(candidate => hasOwn.call(map, candidate))
    if (key === undefined) continue
    used.add(key)
    output.push({ record, key, value: map[key] })
  }
  for (const key of Object.keys(map)) if (!used.has(key)) fail(`${label}.${key} does not resolve to a layer`)
  return output
}

function resolveExact(records: LayerRecord[], requested: string, label: string): LayerRecord {
  const matches = exactMatches(records, requested)
  if (!matches.length) fail(`${label} does not resolve to a layer`)
  if (matches.length > 1) fail(`${label} resolves ambiguously`)
  return matches[0]
}

function resolveBase(records: LayerRecord[], requested: string, label: string): LayerRecord {
  const matches = baseMatches(records, requested)
  if (!matches.length) fail(`${label} does not resolve to a layer`)
  if (matches.length > 1) fail(`${label} resolves ambiguously`)
  return matches[0]
}
function loaderCanonicalName(name: string): string {
  const value = normalized(name)
  const eye = value.match(/^(eyewhite|irides|eyelash|eyebrow|eye_close)[-_](l|r)$/)
  if (eye) return eye[1]
  const body = value.match(/^(handwear|ears)[-_](l|r)$/)
  if (body) return `${body[1]}_${body[2] === "l" ? 1 : 2}`
  return value
}
function resolveAliasTarget(records: LayerRecord[], requested: string, label: string): LayerRecord {
  return resolveExact(records, loaderCanonicalName(requested), label)
}

function requireMatches(records: LayerRecord[], requested: string, label: string): LayerRecord[] {
  const matches = baseMatches(records, requested)
  if (!matches.length) fail(`${label} does not resolve to a layer`)
  return matches
}

function assertReferenceStringArray(values: string[] | undefined, records: LayerRecord[], label: string, mode: "exact" | "base"): void {
  if (!values) return
  for (let index = 0; index < values.length; index += 1) {
    if (mode === "exact") resolveExact(records, values[index], `${label}[${index}]`)
    else resolveBase(records, values[index], `${label}[${index}]`)
  }
}

function assertReferenceMapKeys<T>(map: Record<string, T> | undefined, records: LayerRecord[], label: string): Array<{ record: LayerRecord; key: string; value: T }> {
  return lookupMapEntries(map, records, label)
}

function assertAliasReferences(overrides: RigOverrides, raw: LayerRecord[], loader: LayerRecord[]): void {
  const entries = assertReferenceMapKeys(overrides.layerAliases, raw, "layerAliases")
  const graph: Graph = new Map()
  for (const entry of entries) {
    const target = resolveAliasTarget(loader, entry.value, `layerAliases.${entry.key}`)
    addGraphEdge(graph, entry.record.normalized, target.normalized)
  }
  assertAcyclic(graph, "layer alias graph")
}

function assertMapReferenceGraph(
  map: Record<string, string> | undefined,
  records: LayerRecord[],
  label: string,
): void {
  const entries = assertReferenceMapKeys(map, records, label)
  const graph: Graph = new Map()
  for (const entry of entries) {
    const target = resolveExact(records, entry.value, `${label}.${entry.key}`)
    addGraphEdge(graph, entry.record.normalized, target.normalized)
  }
  assertAcyclic(graph, `${label} graph`)
}

function assertPhysicsLayerReferences(overrides: RigOverrides, records: LayerRecord[]): void {
  const layers = overrides.physics?.layers
  if (!layers) return
  const used = new Set<string>()
  for (const record of records) {
    const candidates = [record.name, record.name.replace(/_\d+$/, "")]
    const key = candidates.find(candidate => hasOwn.call(layers, candidate))
    if (key !== undefined) used.add(key)
  }
  for (const key of Object.keys(layers)) if (!used.has(key)) fail(`physics.layers.${key} does not resolve to a layer`)
}

function assertOrderReferences(overrides: RigOverrides, records: LayerRecord[]): void {
  const order = overrides.layerOrder
  if (order) {
    for (let index = 0; index < order.length; index += 1) {
      requireMatches(records, order[index], `layerOrder[${index}]`)
    }
  }
  const graph: Graph = new Map()
  for (const constraint of overrides.layerOrderConstraints ?? []) {
    const behind = requireMatches(records, constraint.behind, "layerOrderConstraints.behind")
    const inFrontOf = requireMatches(records, constraint.inFrontOf, "layerOrderConstraints.inFrontOf")
    for (const from of behind) for (const to of inFrontOf) {
      if (from.normalized === to.normalized) fail("layerOrderConstraints contains a self-cycle")
      addGraphEdge(graph, from.normalized, to.normalized)
    }
  }
  assertAcyclic(graph, "layer order graph")
}

function activeAfterHidden(records: LayerRecord[], hidden: string[] | undefined): LayerRecord[] {
  if (!hidden) return records.slice()
  const hiddenNames = new Set(hidden.map(normalized))
  return records.filter(record => !hiddenNames.has(record.normalized) && !hiddenNames.has(record.base))
}

function assertOverlayReferences(overrides: RigOverrides, initial: LayerRecord[]): LayerRecord[] {
  const active = activeAfterHidden(initial, overrides.hiddenLayers)
  for (const [index, repair] of (overrides.interpolatedPatchRepairs ?? []).entries()) {
    requireMatches(active, repair.layer, `interpolatedPatchRepairs[${index}].layer`)
  }
  if (overrides.mouthExpressions) {
    for (const key of ["neutral", "open", "smile"] as const) {
      resolveExact(active, overrides.mouthExpressions[key], `mouthExpressions.${key}`)
    }
  }
  const layers = active.slice()
  const orderGraph: Graph = new Map()
  for (const [index, overlay] of (overrides.maskedLayerOverlays ?? []).entries()) {
    const itemLabel = `maskedLayerOverlays[${index}]`
    const source = resolveBase(layers, overlay.source, `${itemLabel}.source`)
    const inFrontOf = resolveBase(layers, overlay.inFrontOf, `${itemLabel}.inFrontOf`)
    if (overlay.textureSource !== undefined) {
      resolveExact(layers, overlay.textureSource, `${itemLabel}.textureSource`)
    }
    const synthetic = { name: overlay.name, normalized: normalized(overlay.name), base: baseName(overlay.name) }
    if (synthetic.normalized === source.normalized) fail(`${itemLabel}.name must differ from source`)
    if (overlay.replaceExisting) {
      for (let layerIndex = layers.length - 1; layerIndex >= 0; layerIndex -= 1) {
        if (layers[layerIndex].normalized === synthetic.normalized) layers.splice(layerIndex, 1)
      }
    }
    if (layers.some(layer => layer.normalized === synthetic.normalized)) {
      fail(`${itemLabel}.name collides with a layer`)
    }
    addGraphEdge(orderGraph, inFrontOf.normalized, synthetic.normalized)
    layers.push(synthetic)
    if (overlay.textureSource !== undefined) {
      const texture = resolveExact(layers, overlay.textureSource, `${itemLabel}.textureSource`)
      if (texture.normalized !== source.normalized) {
        const textureIndex = layers.findIndex(layer => layer.normalized === texture.normalized)
        if (textureIndex >= 0) layers.splice(textureIndex, 1)
      }
    }
  }
  assertAcyclic(orderGraph, "layer order graph")
  return layers
}

function collectLayerNames(values: Iterable<string>, label: string): string[] {
  const names: string[] = []
  try {
    for (const name of values) {
      if (names.length >= MAX_LAYER_NAMES) fail(`${label} exceeds cap`)
      assertName(name, `${label}[${names.length}]`)
      names.push(name)
    }
  } catch (error) {
    if (error instanceof Error && error.message.startsWith("RIG_OVERRIDE_INVALID:")) throw error
    fail(`${label} is not a valid bounded iterable`)
  }
  return names
}
function mergeRecords(first: LayerRecord[], second: LayerRecord[]): LayerRecord[] {
  const records = first.slice()
  const seen = new Set(first.map(record => record.normalized))
  for (const record of second) {
    if (seen.has(record.normalized)) continue
    seen.add(record.normalized)
    records.push(record)
  }
  return records
}

function assertReferences(overrides: RigOverrides, layerNames: NativeRigLayerNames): void {
  const raw = makeRecords(collectLayerNames(layerNames.raw, "raw layerNames"))
  const loader = makeRecords(collectLayerNames(layerNames.loader, "loader layerNames"))
  assertReferenceStringArray(overrides.hiddenLayers, mergeRecords(raw, loader), "hiddenLayers", "exact")
  assertAliasReferences(overrides, raw, loader)
  const active = assertOverlayReferences(overrides, loader)
  assertOrderReferences(overrides, active)
  assertReferenceMapKeys(overrides.depthOverrides, active, "depthOverrides")
  assertReferenceMapKeys(overrides.groupOverrides, active, "groupOverrides")
  assertReferenceMapKeys(overrides.hairAttachments, active, "hairAttachments")
  assertReferenceMapKeys(overrides.headFollow, active, "headFollow")
  assertReferenceMapKeys(overrides.cleanupThresholds, active, "cleanupThresholds")
  assertMapReferenceGraph(overrides.deformationSources, active, "deformationSources")
  assertMapReferenceGraph(overrides.meshSources, active, "meshSources")
  assertPhysicsLayerReferences(overrides, active)
  assertReferenceStringArray(overrides.excludeAfterMeshResolution, active, "excludeAfterMeshResolution", "exact")
}

/** Parse, structurally validate, and narrow an untrusted override JSON string. */
export function parseNativeRigOverrides(text: string, maxBytes: number): RigOverrides {
  if (typeof text !== "string" || text.length === 0) fail("override JSON must be a non-empty string")
  if (!Number.isSafeInteger(maxBytes) || maxBytes <= 0) fail("override JSON byte cap is invalid")
  const bytes = utf8ByteLength(text)
  if (bytes > Math.min(maxBytes, MAX_OVERRIDE_BYTES)) fail("override JSON exceeds its UTF-8 byte cap")
  const parsed = new JsonParser(text).parse()
  assertTree(parsed, "overrides")
  assertOverrideShape(parsed, "overrides")
  validateLocalGraphs(parsed)
  return parsed
}

/** Validate references and all loader-facing relationship graphs against known layer names. */
export function validateNativeRigReferences(overrides: RigOverrides, layerNames: NativeRigLayerNames): void {
  assertTree(overrides, "overrides")
  assertOverrideShape(overrides, "overrides")
  validateLocalGraphs(overrides)
  try {
    assertReferences(overrides, layerNames)
  } catch (error) {
    if (error instanceof Error && error.message.startsWith("RIG_OVERRIDE_INVALID:")) throw error
    fail("override reference validation failed")
  }
}
