import { initializeCanvas, readPsd } from "ag-psd"
import { Inflate } from "pako"
import { PsdRigLoader } from "../../web/rig/vendor/anime25drig/PsdRigLoader"
import { DEFAULT_HAIR_PHYSICS, mergeHairPhysics } from "../../web/rig/vendor/anime25drig/HairPhysicsConfig"
import type {
  HairPhysicsConfig,
  RigAnchors,
  RigDefinition,
  RigLayer,
  RigLoadResult,
} from "../../web/rig/vendor/anime25drig/types"
import type { RigOverrides } from "../../web/rig/vendor/anime25drig/RigOverrides"
import limits from "./limits.json"
import {
  parseNativeRigOverrides,
  validateNativeRigReferences,
  type NativeRigLayerNames,
} from "./override-validation"

// These values are the single native decoder resource contract. The generated
// Swift consumer is produced from the same limits.json by build-limits.mjs.
export const DECODER_CAPS = Object.freeze({
  maxInputBytes: limits.inputFileBytes,
  maxPackBytes: limits.packBytes,
  maxOverrideBytes: limits.overrideBytes,
  maxCanvasWidth: limits.canvasDimension,
  maxCanvasHeight: limits.canvasDimension,
  maxCanvasPixels: limits.canvasPixels,
  maxLayerDimension: limits.canvasDimension,
  maxPSDChannels: 32,
  maxPSDLayerRecords: limits.psdLayers,
  maxPSDChannelBytes: limits.channelDecodedBytes,
  maxDecodedLayerBytes: limits.decodedBytes,
  maxOutputLayers: limits.outputLayers,
  maxStrandWeightEntries: limits.strandWeightEntries,
  maxOutputBytes: limits.outputBytes,
  maxJSONBytes: limits.jsonBytes,
  maxImageAllocationBytes: limits.imageAllocationBytes,
  maxWarnings: limits.outputLayers * 64,
  maxWarningLength: 4096,
  maxNameLength: 512,
})


type DecoderCaps = typeof DECODER_CAPS
type ByteInput = ArrayBuffer | Uint8Array
type PackedLayer = Omit<RigLayer, "img"> & { img: { width: number; height: number; offset: number; length: number } }
type ImageDataLike = { width: number; height: number; data: Uint8ClampedArray }

type PackedRig = Omit<RigDefinition, "layers"> & { layers: PackedLayer[] }

type CompactRig = { rig: RigDefinition; missing: string[]; psdLayerCount: number; warnings: string[] }

type PackedDocument = {
  format: "herdr.rig.decoded"
  version: 1
  base: PackedRig
  pose: PackedRig | null
  physics: PackedPhysics
  diagnostics: {
    baseMissingRequired: string[]
    poseMissingRequired: string[]
    basePSDLayerCount: number
    posePSDLayerCount: number
    warnings: string[]
  }
}

type NativeDecodeResult = { json: string; rgba: Uint8Array }

const caps: DecoderCaps = DECODER_CAPS
const nativeClampedArray = Uint8ClampedArray
let imageAllocationBytes = 0

function reserveImageAllocation(bytes: number): void {
  if (!Number.isSafeInteger(bytes) || bytes < 0 || imageAllocationBytes > caps.maxImageAllocationBytes - bytes) {
    fail("cumulative ImageData allocation exceeds cap")
  }
  imageAllocationBytes += bytes
}

function arrayBufferLike(value: unknown): boolean {
  return value instanceof ArrayBuffer
    || (typeof SharedArrayBuffer !== "undefined" && value instanceof SharedArrayBuffer)
}

function clampedArrayLength(argumentsList: readonly unknown[]): number {
  if (argumentsList.length === 0) return 0
  const source = argumentsList[0]
  if (typeof source === "number") return source
  if (arrayBufferLike(source)) return 0
  if (Array.isArray(source)) return source.length
  if (ArrayBuffer.isView(source)) {
    if ("length" in source && typeof source.length === "number") return source.length
    return 0
  }
  if (source && typeof source === "object" && "length" in source) {
    const length = source.length
    return typeof length === "number" ? length : 0
  }
  return 0
}

function installBoundedImageAllocator(): void {
  const bounded = new Proxy(nativeClampedArray, {
    construct(target, argumentsList, newTarget) {
      const length = clampedArrayLength(argumentsList)
      if (!Number.isSafeInteger(length) || length < 0) fail("ImageData allocation length is invalid")
      reserveImageAllocation(length)
      return Reflect.construct(target, argumentsList, newTarget)
    },
  })
  try {
    Object.defineProperty(globalThis, "Uint8ClampedArray", {
      value: bounded,
      configurable: true,
      writable: true,
    })
  } catch {
    fail("ImageData allocator cannot be bounded")
  }
}

function fail(message: string): never {
  throw new Error(`RIG_DECODE_INVALID: ${message}`)
}

function integer(value: unknown, label: string, min: number, max: number): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < min || value > max) {
    fail(`${label} must be an integer in [${min}, ${max}]`)
  }
  return value
}

function finite(value: unknown, label: string, limit = 1e9): number {
  if (typeof value !== "number" || !Number.isFinite(value) || Math.abs(value) > limit) {
    fail(`${label} must be finite and bounded`)
  }
  return value
}

function safeAdd(left: number, right: number, label: string, max = Number.MAX_SAFE_INTEGER): number {
  if (!Number.isSafeInteger(left) || !Number.isSafeInteger(right) || right < 0 || left > max - right) {
    fail(`${label} exceeds bounded integer range`)
  }
  return left + right
}

function bytesOf(value: ByteInput, label: string): Uint8Array {
  if (value instanceof ArrayBuffer) return new Uint8Array(value)
  if (ArrayBuffer.isView(value) && !(value instanceof DataView)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength)
  }
  fail(`${label} must be an ArrayBuffer or byte typed array`)
}
function fullArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  if (bytes.buffer instanceof ArrayBuffer && bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength) {
    return bytes.buffer
  }
  const copy = new Uint8Array(bytes.byteLength)
  copy.set(bytes)
  return copy.buffer
}


function readU16(view: DataView, offset: number, label: string): number {
  if (offset < 0 || offset + 2 > view.byteLength) fail(`${label} is truncated`)
  return view.getUint16(offset, false)
}

function readI16(view: DataView, offset: number, label: string): number {
  if (offset < 0 || offset + 2 > view.byteLength) fail(`${label} is truncated`)
  return view.getInt16(offset, false)
}

function readU32(view: DataView, offset: number, label: string): number {
  if (offset < 0 || offset + 4 > view.byteLength) fail(`${label} is truncated`)
  return view.getUint32(offset, false)
}

function readI32(view: DataView, offset: number, label: string): number {
  if (offset < 0 || offset + 4 > view.byteLength) fail(`${label} is truncated`)
  return view.getInt32(offset, false)
}

function hasSignature(bytes: Uint8Array, offset: number, signature: string): boolean {
  if (offset < 0 || offset + signature.length > bytes.length) return false
  for (let index = 0; index < signature.length; index++) {
    if (bytes[offset + index] !== signature.charCodeAt(index)) return false
  }
  return true
}

function boundedRange(offset: number, length: number, limit: number, label: string): number {
  if (!Number.isSafeInteger(offset) || !Number.isSafeInteger(length) || offset < 0 || length < 0 || offset > limit - length) {
    fail(`${label} exceeds PSD bounds`)
  }
  return offset + length
}

// ag-psd's readAdditionalLayerInfo realigns each block signature by trying
// these offsets in order before giving up.
const ADDITIONAL_INFO_REALIGN_OFFSETS = [0, 1, -1, 2, -2, 3, -3, 4, -4] as const
// ag-psd parses Lr16/Lr32 as a further, uncapped layer-info section regardless
// of bit depth; Layr is the reserved 8-bit variant of the same layer data.
const NESTED_LAYER_INFO_KEYS: Record<string, true> = { Lr16: true, Lr32: true, Layr: true }

/**
 * Inspect one tagged block exactly where ag-psd would parse it and return the
 * offset of the next block. Blocks must end inside their enclosing section.
 */
function preflightAdditionalInfo(bytes: Uint8Array, view: DataView, offset: number, end: number, label: string): number {
  let start = -1
  for (const shift of ADDITIONAL_INFO_REALIGN_OFFSETS) {
    if (hasSignature(bytes, offset + shift, "8BIM") || hasSignature(bytes, offset + shift, "8B64")) {
      start = offset + shift
      break
    }
  }
  if (start < 0) fail(`${label} additional info signature is invalid`)
  const large = hasSignature(bytes, start, "8B64")
  const dataStart = boundedRange(start, large ? 16 : 12, end, `${label} additional info header`)
  const key = String.fromCharCode(bytes[start + 4], bytes[start + 5], bytes[start + 6], bytes[start + 7])
  if (NESTED_LAYER_INFO_KEYS[key] === true) fail(`${label} additional info ${key} nested layer data is not supported`)
  if (large && readU32(view, start + 8, `${label} additional info length`) !== 0) {
    fail(`${label} additional info length exceeds 4 GiB`)
  }
  const length = readU32(view, dataStart - 4, `${label} additional info length`)
  const blockEnd = boundedRange(dataStart, length, end, `${label} additional info data`)
  // ag-psd pads every tagged block to an even length.
  return blockEnd + (length % 2)
}

function preflightChannel(bytes: Uint8Array, offset: number, length: number,
  width: number, height: number, depth: number, label: string): void {
  if (length === 0) return
  if (length < 2) fail(`${label} channel compression is truncated`)
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const compression = readU16(view, offset, label)
  const end = boundedRange(offset, length, bytes.length, label)
  const rowBytes = depth === 1 ? Math.ceil(width / 8) : width
  const expected = rowBytes * height
  let cursor = offset + 2
  if (compression === 0) {
    if (end - cursor !== expected) fail(`${label} raw channel length is inconsistent`)
  } else if (compression === 1) {
    let rowStart = boundedRange(cursor, height * 2, end, `${label} RLE row table`)
    for (let row = 0; row < height; row++, cursor += 2) {
      const rowEnd = boundedRange(rowStart, readU16(view, cursor, label), end, `${label} RLE row`)
      let decoded = 0
      while (rowStart < rowEnd) {
        const control = bytes[rowStart++]
        if (control < 128) {
          const count = control + 1
          rowStart = boundedRange(rowStart, count, rowEnd, `${label} RLE literal`)
          decoded = safeAdd(decoded, count, `${label} RLE output`, rowBytes)
        } else if (control > 128) {
          rowStart = boundedRange(rowStart, 1, rowEnd, `${label} RLE repeat`)
          decoded = safeAdd(decoded, 257 - control, `${label} RLE output`, rowBytes)
        }
      }
      if (decoded !== rowBytes) fail(`${label} RLE row has an inconsistent decoded length`)
    }
    if (rowStart !== end) fail(`${label} RLE channel has trailing bytes`)
  } else if (compression === 2 || compression === 3) {
    // ag-psd inflates into an unbounded result before examining pixel dimensions.
    // Count a streaming pass first; never retain its output or permit expansion
    // beyond this already-validated channel's exact pixel storage.
    const inflater = new Inflate({ chunkSize: 16 * 1024 })
    let decoded = 0
    let finished = false
    inflater.onData = chunk => {
      if (!(chunk instanceof Uint8Array)) fail(`${label} ZIP channel produced non-byte output`)
      decoded = safeAdd(decoded, chunk.length, `${label} ZIP output`, expected)
    }
    inflater.onEnd = status => {
      if (status !== 0) fail(`${label} ZIP channel is invalid`)
      finished = true
    }
    if (!inflater.push(bytes.subarray(cursor, end), true) || !finished || decoded !== expected) {
      fail(`${label} ZIP channel is truncated or has an inconsistent decoded length`)
    }
  } else {
    fail(`${label} channel compression is unsupported`)
  }
}

/**
 * Inspect the PSD layer records without allocating any layer image. ag-psd's
 * readPsd call is intentionally below this function. PSD files are big-endian;
 * all offsets here are checked before each DataView read and addition.
 */
export function preflightPSD(input: Uint8Array, label: string): { rgba: number; channels: number } {
  if (input.byteLength === 0 || input.byteLength > caps.maxInputBytes) {
    fail(`${label} byte length exceeds input cap`)
  }
  const bytes = input
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  if (bytes.byteLength < 26 || !hasSignature(bytes, 0, "8BPS")) fail(`${label} is not a PSD`) 
  const version = readU16(view, 4, `${label} version`)
  if (version !== 1) fail(`${label} PSB/unsupported PSD version ${version}`)
  for (let offset = 6; offset < 12; offset++) if (bytes[offset] !== 0) fail(`${label} reserved header bytes are non-zero`)
  const channels = readU16(view, 12, `${label} channels`)
  const height = readU32(view, 14, `${label} height`)
  const width = readU32(view, 18, `${label} width`)
  const depth = readU16(view, 22, `${label} depth`)
  const colorMode = readU16(view, 24, `${label} color mode`)
  if (colorMode === 4) fail(`${label} CMYK PSD is not supported by native RGBA output`)
  if (colorMode > 9) fail(`${label} color mode is unsupported`)
  if (channels < 1 || channels > caps.maxPSDChannels) fail(`${label} channel count exceeds cap`)
  if (width < 1 || width > caps.maxCanvasWidth || height < 1 || height > caps.maxCanvasHeight) {
    fail(`${label} canvas dimensions exceed cap`)
  }
  const canvasPixels = width * height
  if (!Number.isSafeInteger(canvasPixels) || canvasPixels > caps.maxCanvasPixels) fail(`${label} canvas pixels exceed cap`)
  // The native pack path requires straight 8-bit RGBA image data. Rejecting
  // other depths before ag-psd prevents its 16/32-bit allocators from running.
  if (depth !== 1 && depth !== 8) fail(`${label} bit depth ${depth} is not supported by native RGBA output`)

  let offset = 26
  const colorModeLength = readU32(view, offset, `${label} color mode length`)
  offset = boundedRange(offset + 4, colorModeLength, bytes.length, `${label} color mode data`)
  const resourcesLength = readU32(view, offset, `${label} image resources length`)
  offset = boundedRange(offset + 4, resourcesLength, bytes.length, `${label} image resources`)
  const layerMaskLength = readU32(view, offset, `${label} layer/mask length`)
  const layerMaskStart = offset + 4
  const layerMaskEnd = boundedRange(layerMaskStart, layerMaskLength, bytes.length, `${label} layer/mask data`)

  let decodedLayerBytes = 0
  let decodedChannelBytes = 0
  let compressedChannelBytes = 0
  let records = 0
  if (layerMaskLength > 0) {
    if (layerMaskLength < 4) fail(`${label} layer/mask section is truncated`)
    const layerInfoLength = readU32(view, layerMaskStart, `${label} layer info length`)
    const layerInfoStart = layerMaskStart + 4
    const layerInfoEnd = boundedRange(layerInfoStart, layerInfoLength, layerMaskEnd, `${label} layer info`)
    if (layerInfoLength > 0) {
      if (layerInfoLength < 2) fail(`${label} layer info count is truncated`)
      const rawCount = readI16(view, layerInfoStart, `${label} layer count`)
      records = Math.abs(rawCount)
      if (records > caps.maxPSDLayerRecords) fail(`${label} layer record count exceeds cap`)
      let cursor = layerInfoStart + 2
      const channelLengths: Array<{ length: number; id: number; width: number; height: number }> = []
      for (let record = 0; record < records; record++) {
        boundedRange(cursor, 16, layerInfoEnd, `${label} layer ${record} rectangle`)
        const top = readI32(view, cursor, `${label} layer ${record} top`)
        const left = readI32(view, cursor + 4, `${label} layer ${record} left`)
        const bottom = readI32(view, cursor + 8, `${label} layer ${record} bottom`)
        const right = readI32(view, cursor + 12, `${label} layer ${record} right`)
        const layerWidth = right - left
        const layerHeight = bottom - top
        if (!Number.isSafeInteger(layerWidth) || !Number.isSafeInteger(layerHeight) || layerWidth < 0 || layerHeight < 0 || layerWidth > caps.maxLayerDimension || layerHeight > caps.maxLayerDimension) {
          fail(`${label} layer ${record} dimensions exceed cap`)
        }
        const layerPixels = layerWidth * layerHeight
        if (!Number.isSafeInteger(layerPixels) || layerPixels > caps.maxCanvasPixels) fail(`${label} layer ${record} pixels exceed cap`)
        decodedLayerBytes = safeAdd(decodedLayerBytes, safeAdd(layerPixels, layerPixels, `${label} layer ${record} pixel bound`) * 2, `${label} decoded layer bound`, caps.maxDecodedLayerBytes)
        cursor += 16
        const channelCount = readU16(view, cursor, `${label} layer ${record} channel count`)
        if (channelCount > caps.maxPSDChannels) fail(`${label} layer ${record} channel count exceeds cap`)
        cursor += 2
        const firstChannel = channelLengths.length
        for (let channel = 0; channel < channelCount; channel++) {
          boundedRange(cursor, 6, layerInfoEnd, `${label} layer ${record} channel ${channel}`)
          const channelLength = readU32(view, cursor + 2, `${label} layer ${record} channel ${channel} length`)
          channelLengths.push({ length: channelLength, id: readI16(view, cursor, label),
            width: layerWidth, height: layerHeight })
          compressedChannelBytes = safeAdd(compressedChannelBytes, channelLength, `${label} compressed channel bytes`, caps.maxInputBytes)
          cursor += 6
        }
        let maxChannelPixels = layerPixels
        boundedRange(cursor, 12, layerInfoEnd, `${label} layer ${record} blend header`)
        cursor += 12
        const extraStart = cursor + 4
        const extraLength = readU32(view, cursor, `${label} layer ${record} extra length`)
        const extraEnd = boundedRange(extraStart, extraLength, layerInfoEnd, `${label} layer ${record} extra data`)
        if (extraLength >= 4) {
          const maskLength = readU32(view, extraStart, `${label} layer ${record} mask length`)
          if (maskLength > extraLength - 4) fail(`${label} layer ${record} mask data is truncated`)
          if (maskLength > 0 && maskLength < 18) fail(`${label} layer ${record} mask rectangle is truncated`)
          if (maskLength >= 18) {
            const maskStart = extraStart + 4
            const maskTop = readI32(view, maskStart, `${label} layer ${record} mask top`)
            const maskLeft = readI32(view, maskStart + 4, `${label} layer ${record} mask left`)
            const maskBottom = readI32(view, maskStart + 8, `${label} layer ${record} mask bottom`)
            const maskRight = readI32(view, maskStart + 12, `${label} layer ${record} mask right`)
            const maskWidth = maskRight - maskLeft
            const maskHeight = maskBottom - maskTop
            if (!Number.isSafeInteger(maskWidth) || !Number.isSafeInteger(maskHeight) || maskWidth < 0 || maskHeight < 0 || maskWidth > caps.maxLayerDimension || maskHeight > caps.maxLayerDimension) {
              fail(`${label} layer ${record} mask dimensions exceed cap`)
            }
            const maskPixels = maskWidth * maskHeight
            if (!Number.isSafeInteger(maskPixels)) fail(`${label} layer ${record} mask pixels exceed cap`)
            maxChannelPixels = Math.max(maxChannelPixels, maskPixels)
            for (let index = firstChannel; index < channelLengths.length; index++) {
              if (channelLengths[index].id === -2) {
                channelLengths[index].width = maskWidth
                channelLengths[index].height = maskHeight
              }
            }
            decodedLayerBytes = safeAdd(decodedLayerBytes, safeAdd(maskPixels, maskPixels, `${label} layer ${record} mask pixel bound`) * 2, `${label} decoded layer bound`, caps.maxDecodedLayerBytes)
            if (maskLength >= 36) {
              const realStart = maskStart + 18
              const realTop = readI32(view, realStart + 2, `${label} layer ${record} real mask top`)
              const realLeft = readI32(view, realStart + 6, `${label} layer ${record} real mask left`)
              const realBottom = readI32(view, realStart + 10, `${label} layer ${record} real mask bottom`)
              const realRight = readI32(view, realStart + 14, `${label} layer ${record} real mask right`)
              const realWidth = realRight - realLeft
              const realHeight = realBottom - realTop
              if (!Number.isSafeInteger(realWidth) || !Number.isSafeInteger(realHeight) || realWidth < 0 || realHeight < 0 || realWidth > caps.maxLayerDimension || realHeight > caps.maxLayerDimension) {
                fail(`${label} layer ${record} real mask dimensions exceed cap`)
              }
              const realPixels = realWidth * realHeight
              if (!Number.isSafeInteger(realPixels)) fail(`${label} layer ${record} real mask pixels exceed cap`)
              maxChannelPixels = Math.max(maxChannelPixels, realPixels)
              for (let index = firstChannel; index < channelLengths.length; index++) {
                if (channelLengths[index].id === -3) {
                  channelLengths[index].width = realWidth
                  channelLengths[index].height = realHeight
                }
              }
              decodedLayerBytes = safeAdd(decodedLayerBytes, safeAdd(realPixels, realPixels, `${label} layer ${record} real mask pixel bound`) * 2, `${label} decoded layer bound`, caps.maxDecodedLayerBytes)
            }
          }
          // ag-psd then reads blending ranges and the Pascal name, scans to
          // the first tagged-block signature, and parses blocks to extraEnd.
          const blendingStart = extraStart + 4 + maskLength
          boundedRange(blendingStart, 4, extraEnd, `${label} layer ${record} blending ranges length`)
          const blendingLength = readU32(view, blendingStart, `${label} layer ${record} blending ranges length`)
          const nameStart = boundedRange(blendingStart + 4, blendingLength, extraEnd, `${label} layer ${record} blending ranges`)
          boundedRange(nameStart, 1, extraEnd, `${label} layer ${record} name length`)
          let tagged = boundedRange(nameStart + 1, bytes[nameStart], extraEnd, `${label} layer ${record} name`)
          while (extraEnd - tagged > 4 && !hasSignature(bytes, tagged, "8BIM") && !hasSignature(bytes, tagged, "8B64")) tagged++
          while (extraEnd - tagged >= 12) {
            tagged = preflightAdditionalInfo(bytes, view, tagged, extraEnd, `${label} layer ${record}`)
          }
        }
        decodedChannelBytes = safeAdd(decodedChannelBytes, maxChannelPixels * channelCount,
          `${label} decoded channel allocation`, caps.maxPSDChannelBytes)
        cursor = extraEnd
      }
      if (cursor > layerInfoEnd) fail(`${label} layer records exceed layer info`) 
      // Channel payloads follow all records inside layer info. Their declared
      // lengths must fit there, even when compressed data is much smaller than
      // the decoded image allocation bound above.
      const channelPayloadBytes = layerInfoEnd - cursor
      if (compressedChannelBytes > channelPayloadBytes) fail(`${label} layer channel payload is truncated`)
      for (const channel of channelLengths) {
        boundedRange(cursor, channel.length, layerInfoEnd, `${label} channel payload`)
        preflightChannel(bytes, cursor, channel.length, channel.width, channel.height, depth, label)
        cursor += channel.length
      }
    }
    // ag-psd pads layer info to an even length, then reads global mask info
    // and tagged blocks through the end of the layer/mask section.
    let trailer = layerInfoEnd + (layerInfoLength % 2)
    if (trailer < layerMaskEnd) {
      boundedRange(trailer, 4, layerMaskEnd, `${label} global layer mask length`)
      const globalMaskLength = readU32(view, trailer, `${label} global layer mask length`)
      trailer = boundedRange(trailer + 4, globalMaskLength, layerMaskEnd, `${label} global layer mask info`)
      while (trailer < layerMaskEnd) {
        while (trailer < layerMaskEnd && bytes[trailer] === 0) trailer++
        if (layerMaskEnd - trailer >= 12) {
          trailer = preflightAdditionalInfo(bytes, view, trailer, layerMaskEnd, `${label} layer/mask trailer`)
        } else {
          trailer = layerMaskEnd
        }
      }
    }
  }
  if (records === 0) fail(`${label} has no layer records`)
  if (decodedLayerBytes > caps.maxDecodedLayerBytes) fail(`${label} decoded layer allocation exceeds cap`)
  if (layerMaskEnd > bytes.length) fail(`${label} layer/mask data exceeds file`) 
  return { rgba: decodedLayerBytes, channels: decodedChannelBytes }
}

function rawImageData(width: number, height: number): ImageData {
  integer(width, "ImageData width", 1, caps.maxLayerDimension)
  integer(height, "ImageData height", 1, caps.maxLayerDimension)
  const byteLength = safeAdd(0, safeAdd(width * height, width * height, "ImageData pixels") * 2, "ImageData bytes", caps.maxDecodedLayerBytes)
  if (byteLength > caps.maxDecodedLayerBytes) fail("ImageData allocation exceeds decoded-byte cap")
  return { width, height, data: new Uint8ClampedArray(byteLength), colorSpace: "srgb" }
}

// ag-psd's raw ImageData mode only needs the allocator; no canvas or drawing
// context exists in the JSC child. This is an actual typed ImageData-shaped
// buffer consumed by the published ag-psd parser, not a canvas fake.
initializeCanvas(() => { throw new Error("RIG_DECODE_CANVAS_UNAVAILABLE") }, rawImageData)
installBoundedImageAllocator()

function decodeBase64(value: string): string {
  if (typeof value !== "string" || value.length % 4 === 1) fail("invalid base64 payload")
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
  let output = ""
  for (let index = 0; index < value.length; index += 4) {
    const a = value.charCodeAt(index)
    const b = value.charCodeAt(index + 1)
    const c = value.charCodeAt(index + 2)
    const d = value.charCodeAt(index + 3)
    const va = a === 61 ? 0 : alphabet.indexOf(String.fromCharCode(a))
    const vb = b === 61 ? 0 : alphabet.indexOf(String.fromCharCode(b))
    const vc = c === 61 ? 0 : alphabet.indexOf(String.fromCharCode(c))
    const vd = d === 61 ? 0 : alphabet.indexOf(String.fromCharCode(d))
    if (va < 0 || vb < 0 || vc < 0 || vd < 0) fail("invalid base64 character")
    const value24 = (va << 18) | (vb << 12) | (vc << 6) | vd
    output += String.fromCharCode((value24 >>> 16) & 255)
    if (c !== 61) output += String.fromCharCode((value24 >>> 8) & 255)
    if (d !== 61) output += String.fromCharCode(value24 & 255)
  }
  return output
}

// The CommonJS genericparts module installs a Node Buffer fallback during
// import. Replace that fallback with the real bounded native-worker primitive.
Object.defineProperty(globalThis, "atob", { value: decodeBase64 })
const decoderWarnings: string[] = []
let diagnosticBytes = 0
function recordDiagnostic(...values: unknown[]): void {
  const message = values.map(String).join(" ")
  const bytes = utf8ByteLength(message)
  diagnosticBytes += bytes
  if (bytes > caps.maxWarningLength || diagnosticBytes > 64 * 1024 || decoderWarnings.length >= caps.maxWarnings) {
    fail("decoder diagnostics exceed cap")
  }
  decoderWarnings.push(message)
}
// Diagnostics travel in the bounded document, never binary stdout.
Object.defineProperty(globalThis, "console", {
  value: { log: recordDiagnostic, warn: recordDiagnostic, error: recordDiagnostic },
})



function stringValue(value: unknown, label: string, max: number = caps.maxNameLength): string {
  if (typeof value !== "string" || value.length === 0 || value.includes("\u0000") || utf8ByteLength(value) > max) fail(`${label} is invalid`)
  return value
}
const PHYSICS_KEYS = ["amplitude", "stiffness", "damping", "wind", "inertia", "rootLock", "maxOffset"] as const

type PackedPhysics = {
  frontHair: HairPhysicsConfig["frontHair"]
  backHair: HairPhysicsConfig["backHair"]
  layers: Record<string, Record<string, number>>
}

function compactPhysics(overrides: RigOverrides): PackedPhysics {
  const merged = mergeHairPhysics({
    ...overrides.physics,
    frontHair: { ...DEFAULT_HAIR_PHYSICS.frontHair, ...overrides.physics?.frontHair },
    backHair: { ...DEFAULT_HAIR_PHYSICS.backHair, ...overrides.physics?.backHair },
  })
  const entries = Object.entries(merged.layers)
  if (entries.length > caps.maxOutputLayers) fail("hair physics layer tuning exceeds cap")
  const layers: Record<string, Record<string, number>> = {}
  for (const [name, tuning] of entries) {
    stringValue(name, "hair physics layer name")
    if (!tuning || typeof tuning !== "object" || Array.isArray(tuning)) fail(`hair physics tuning for ${name} is invalid`)
    const values: Record<string, number> = {}
    for (const [key, value] of Object.entries(tuning)) {
      if (!(PHYSICS_KEYS as readonly string[]).includes(key)) continue
      if (value === undefined) continue
      values[key] = finite(value, `hair physics ${name}.${key}`)
    }
    layers[name] = values
  }
  return { frontHair: merged.frontHair, backHair: merged.backHair, layers }
}


function point(value: unknown, label: string): void {
  if (!value || typeof value !== "object") fail(`${label} is missing`)
  const item = value as Record<string, unknown>
  finite(item.cx, `${label}.cx`)
  finite(item.cy, `${label}.cy`)
}

function bounds(value: unknown, label: string): void {
  if (!value || typeof value !== "object") fail(`${label} is missing`)
  const item = value as Record<string, unknown>
  finite(item.x0, `${label}.x0`); finite(item.y0, `${label}.y0`)
  finite(item.x1, `${label}.x1`); finite(item.y1, `${label}.y1`)
  if ((item.x1 as number) < (item.x0 as number) || (item.y1 as number) < (item.y0 as number)) fail(`${label} has inverted bounds`)
}

function numericArray(value: unknown, label: string, minLength = 0, maxLength = 4096): void {
  if (!Array.isArray(value) || value.length < minLength || value.length > maxLength) fail(`${label} array length is invalid`)
  for (let index = 0; index < value.length; index++) finite(value[index], `${label}[${index}]`)
}

function validateAnchors(anchors: RigAnchors, label: string): void {
  if (!anchors || typeof anchors !== "object") fail(`${label} anchors are missing`)
  bounds(anchors.face, `${label}.face`); point(anchors.face, `${label}.face.center`)
  for (const side of ["eyeL", "eyeR"] as const) {
    const eye = anchors[side]
    if (!eye) continue
    bounds(eye, `${label}.${side}`)
    finite(eye.icx, `${label}.${side}.icx`); finite(eye.icy, `${label}.${side}.icy`); finite(eye.closeY, `${label}.${side}.closeY`)
    if (eye.blink) {
      point(eye.blink.center, `${label}.${side}.blink.center`)
      finite(eye.blink.angleDeg, `${label}.${side}.blink.angleDeg`)
      finite(eye.blink.u0, `${label}.${side}.blink.u0`); finite(eye.blink.u1, `${label}.${side}.blink.u1`)
      numericArray(eye.blink.upper, `${label}.${side}.blink.upper`, 2)
      numericArray(eye.blink.lower, `${label}.${side}.blink.lower`, 2)
      numericArray(eye.blink.closed, `${label}.${side}.blink.closed`, 2)
      point(eye.blink.closedSource, `${label}.${side}.blink.closedSource`)
      point(eye.blink.closedTarget, `${label}.${side}.blink.closedTarget`)
      finite(eye.blink.closedRotationDeg, `${label}.${side}.blink.closedRotationDeg`)
    }
  }
  bounds(anchors.mouth, `${label}.mouth`); point(anchors.mouth, `${label}.mouth.center`)
  if (anchors.mouth.morph) {
    const morph = anchors.mouth.morph
    point(morph.center, `${label}.mouth.morph.center`); finite(morph.angleDeg, `${label}.mouth.morph.angleDeg`)
    for (const shapeName of ["neutral", "open", "smile"] as const) {
      const shape = morph[shapeName]
      finite(shape.u0, `${label}.mouth.morph.${shapeName}.u0`); finite(shape.u1, `${label}.mouth.morph.${shapeName}.u1`)
      numericArray(shape.upper, `${label}.mouth.morph.${shapeName}.upper`, 2)
      numericArray(shape.lower, `${label}.mouth.morph.${shapeName}.lower`, 2)
    }
  }
  point(anchors.neckPivot, `${label}.neckPivot`); point(anchors.bodyPivot, `${label}.bodyPivot`)
  for (const key of ["neckTop", "neckBottom", "hairRootY", "faceScale"] as const) finite(anchors[key], `${label}.${key}`)
  if (anchors.faceScale <= 0) fail(`${label}.faceScale must be positive`)
}

function imageData(value: unknown, label: string): ImageDataLike {
  if (!value || typeof value !== "object" || !("width" in value) || !("height" in value) || !("data" in value)) {
    fail(`${label} image is missing`)
  }
  const width = integer(value.width, `${label}.width`, 1, caps.maxLayerDimension)
  const height = integer(value.height, `${label}.height`, 1, caps.maxLayerDimension)
  if (!(value.data instanceof nativeClampedArray)) fail(`${label}.data must be Uint8ClampedArray`)
  const expected = safeAdd(0, safeAdd(width * height, width * height, `${label} pixels`) * 2, `${label} byte length`, caps.maxDecodedLayerBytes)
  if (value.data.byteLength !== expected) fail(`${label}.data length does not match dimensions`)
  return { width, height, data: value.data }
}

function validateDefinition(value: RigDefinition, label: string): void {
  if (!value || typeof value !== "object") fail(`${label} definition is missing`)
  const width = integer(value.canvas?.w, `${label}.canvas.w`, 1, caps.maxCanvasWidth)
  const height = integer(value.canvas?.h, `${label}.canvas.h`, 1, caps.maxCanvasHeight)
  if (width * height > caps.maxCanvasPixels) fail(`${label} canvas pixels exceed cap`)
  if (!Array.isArray(value.layers) || value.layers.length === 0 || value.layers.length > caps.maxOutputLayers) fail(`${label} layer count exceeds cap`)
  validateAnchors(value.anchors, label)
  if (!Array.isArray(value.warnings) || value.warnings.length > caps.maxWarnings) fail(`${label} warnings are invalid`)
  for (const warning of value.warnings) stringValue(warning, `${label}.warning`, caps.maxWarningLength)
  if (!value.synth || typeof value.synth.eye !== "boolean" || typeof value.synth.mouth !== "boolean") fail(`${label}.synth is invalid`)
  let imageBytes = 0
  let strandEntries = 0
  const semantic = new Set<string>()
  for (let index = 0; index < value.layers.length; index++) {
    const layer = value.layers[index]
    const layerLabel = `${label}.layers[${index}]`
    stringValue(layer.name, `${layerLabel}.name`)
    semantic.add(layer.name.normalize("NFKC").trim().toLowerCase())
    for (const key of ["x", "y", "w", "h", "z", "depth"] as const) finite(layer[key], `${layerLabel}.${key}`)
    if (layer.w <= 0 || layer.h <= 0) fail(`${layerLabel} dimensions must be positive`)
    stringValue(layer.group, `${layerLabel}.group`)
    if (layer.phys !== null && layer.phys !== undefined) stringValue(layer.phys, `${layerLabel}.phys`)
    if (layer.fade !== null && layer.fade !== undefined) stringValue(layer.fade, `${layerLabel}.fade`)
    if (layer.mouthExpression !== null && layer.mouthExpression !== undefined) stringValue(layer.mouthExpression, `${layerLabel}.mouthExpression`)
    if (layer.deformationSource !== null && layer.deformationSource !== undefined) stringValue(layer.deformationSource, `${layerLabel}.deformationSource`)
    if (layer.meshSource !== null && layer.meshSource !== undefined) stringValue(layer.meshSource, `${layerLabel}.meshSource`)
    if (layer.side !== null && layer.side !== undefined && layer.side !== "L" && layer.side !== "R") fail(`${layerLabel}.side is invalid`)
    if (layer.hairAttachment) {
      finite(layer.hairAttachment.rootY, `${layerLabel}.hairAttachment.rootY`); finite(layer.hairAttachment.bodyY, `${layerLabel}.hairAttachment.bodyY`)
      if (layer.hairAttachment.bodyY <= layer.hairAttachment.rootY) fail(`${layerLabel}.hairAttachment is inverted`)
    }
    if (layer.headFollow) {
      point(layer.headFollow.center, `${layerLabel}.headFollow.center`)
      finite(layer.headFollow.radius, `${layerLabel}.headFollow.radius`); finite(layer.headFollow.falloffRadius, `${layerLabel}.headFollow.falloffRadius`)
      if (layer.headFollow.radius < 0 || layer.headFollow.falloffRadius <= 0) fail(`${layerLabel}.headFollow radii are invalid`)
    }
    if (layer.strands) {
      if (layer.strands.length > Math.floor(caps.maxStrandWeightEntries / 3)) fail(`${layerLabel}.strands exceeds cap`)
      strandEntries = safeAdd(strandEntries, layer.strands.length * 3, `${label} strand influence entries`, caps.maxStrandWeightEntries)
      for (const strand of layer.strands) {
        finite(strand.x, `${layerLabel}.strand.x`); finite(strand.rootY, `${layerLabel}.strand.rootY`); finite(strand.tipY, `${layerLabel}.strand.tipY`)
      }
    }
    const img = imageData(layer.img, `${layerLabel}.img`)
    imageBytes = safeAdd(imageBytes, img.data.byteLength, `${label} image bytes`, caps.maxOutputBytes)
  }
  for (const required of ["face", "eyewhite", "irides", "eyelash"]) {
    if (![...semantic].some(name => name === required || name.startsWith(`${required}_`))) fail(`${label} required semantic layer ${required} is missing`)
  }
  if (!value.interactionAreas || typeof value.interactionAreas !== "object") return
  for (const [name, area] of Object.entries(value.interactionAreas)) bounds(area, `${label}.interactionAreas.${name}`)
}

function compactRig(result: RigLoadResult, label: string): CompactRig {
  const model = result.model
  if (!model || !model.rig) fail(`${label} loader returned no rig`)
  if (!Array.isArray(model.missingRequiredLayers) || model.missingRequiredLayers.length > caps.maxWarnings) fail(`${label} missing-layer diagnostics are invalid`)
  for (const missing of model.missingRequiredLayers) stringValue(missing, `${label}.missingRequired`)
  if (!Array.isArray(model.psdLayers) || model.psdLayers.length > caps.maxPSDLayerRecords) fail(`${label} PSD diagnostics exceed cap`)
  const warnings = [...model.rig.warnings, ...(model.assetDiagnostics?.warnings ?? [])]
  if (warnings.length > caps.maxWarnings) fail(`${label} warning count exceeds cap`)
  for (const warning of warnings) stringValue(warning, `${label}.warning`, caps.maxWarningLength)
  validateDefinition(model.rig, label)
  return { rig: model.rig, missing: [...model.missingRequiredLayers], psdLayerCount: model.psdLayers.length, warnings }
}
function normalizedLayerName(name: string): string {
  return name.normalize("NFKC").trim().toLowerCase()
}

function baseLayerName(name: string): string {
  return normalizedLayerName(name).replace(/[-_]([lr])$/i, "").replace(/_\d+$/, "")
}

function loaderLayerName(name: string): string {
  const normalized = normalizedLayerName(name)
  const eye = normalized.match(/^(eyewhite|irides|eyelash|eyebrow|eye_close)[-_](l|r)$/)
  if (eye) return eye[1]
  const body = normalized.match(/^(handwear|ears)[-_](l|r)$/)
  if (body) return `${body[1]}_${body[2] === "l" ? 1 : 2}`
  return normalized
}

function referenceLayerNames(result: RigLoadResult, overrides: RigOverrides): NativeRigLayerNames {
  const raw = result.model.psdLayers.map(layer => layer.name)
  const loader: string[] = []
  const seen = new Set<string>()
  const add = (name: string): void => {
    const key = normalizedLayerName(name)
    if (seen.has(key)) return
    seen.add(key)
    loader.push(name)
  }
  const hidden = new Set((overrides.hiddenLayers ?? []).map(normalizedLayerName))
  const overlayNames = new Set((overrides.maskedLayerOverlays ?? []).map(overlay => normalizedLayerName(overlay.name)))
  for (const layer of result.model.psdLayers) {
    if (!layer.visible || hidden.has(normalizedLayerName(layer.name))) continue
    const canonical = loaderLayerName(layer.normalizedName)
    if (overrides.hairSplit?.backHairLeftRight && baseLayerName(canonical) === "back hair") {
      add(`${canonical}_1`)
      add(`${canonical}_2`)
    } else {
      add(canonical)
    }
  }
  for (const layer of result.model.rig.layers) {
    if (!overlayNames.has(normalizedLayerName(layer.name))) add(layer.name)
  }
  return { raw, loader }
}

function utf8ByteLength(value: string): number {
  let bytes = 0
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index)
    if (code <= 0x7f) bytes += 1
    else if (code <= 0x7ff) bytes += 2
    else if (code >= 0xd800 && code <= 0xdbff) {
      const next = value.charCodeAt(index + 1)
      if (next < 0xdc00 || next > 0xdfff) fail("JSON contains an unpaired surrogate")
      bytes += 4
      index += 1
    } else if (code >= 0xdc00 && code <= 0xdfff) {
      fail("JSON contains an unpaired surrogate")
    } else {
      bytes += 3
    }
    if (bytes > Number.MAX_SAFE_INTEGER - 4) fail("JSON byte length overflows")
  }
  return bytes
}

function loadValidated(
  loader: PsdRigLoader,
  input: Uint8Array,
  name: string,
  overrides: RigOverrides,
  label: string,
): CompactRig {
  const parsed = readPsd(fullArrayBuffer(input), {
    useImageData: true, skipThumbnail: true, skipCompositeImageData: true,
    skipLinkedFilesData: true, strict: true, throwForMissingFeatures: true,
  })
  const result = loader.build(parsed, name, overrides)
  validateNativeRigReferences(overrides, referenceLayerNames(result, overrides))
  return compactRig(result, label)
}

function packDocument(base: CompactRig, pose: CompactRig | null, physics: PackedPhysics): NativeDecodeResult {
  const plans: Array<{ rig: RigDefinition; layers: Array<{ source: ImageDataLike; offset: number }> }> = []
  let total = 0
  for (const item of pose ? [base, pose] : [base]) {
    const layers: Array<{ source: ImageDataLike; offset: number }> = []
    for (const [index, layer] of item.rig.layers.entries()) {
      const source = imageData(layer.img, `packed.layers[${index}]`)
      layers.push({ source, offset: total })
      total = safeAdd(total, source.data.byteLength, "packed RGBA bytes", caps.maxOutputBytes)
    }
    plans.push({ rig: item.rig, layers })
  }
  const rgba = new Uint8Array(total)
  const packed = plans.map(({ rig, layers }) => ({
    ...rig,
    layers: rig.layers.map((layer, index) => {
      const plan = layers[index]
      rgba.set(plan.source.data, plan.offset)
      return { ...layer, img: { width: plan.source.width, height: plan.source.height, offset: plan.offset, length: plan.source.data.byteLength } }
    }),
  }))
  const warnings = [...new Set([...base.warnings, ...(pose?.warnings ?? []), ...decoderWarnings])]
  if (warnings.length > caps.maxWarnings) fail("decoded diagnostics exceed cap")
  let warningBytes = 0
  for (const warning of warnings) {
    stringValue(warning, "decoded warning", caps.maxWarningLength)
    warningBytes = safeAdd(warningBytes, utf8ByteLength(warning), "decoded diagnostic bytes", caps.maxJSONBytes)
  }
  const document: PackedDocument = {
    format: "herdr.rig.decoded",
    version: 1,
    physics,
    base: packed[0],
    pose: packed.length > 1 ? packed[1] : null,
    diagnostics: {
      baseMissingRequired: base.missing,
      poseMissingRequired: pose?.missing ?? [],
      basePSDLayerCount: base.psdLayerCount,
      posePSDLayerCount: pose?.psdLayerCount ?? 0,
      warnings,
    },
  }
  const json = JSON.stringify(document)
  const jsonBytes = utf8ByteLength(json)
  if (jsonBytes > caps.maxJSONBytes) fail("decoded JSON exceeds cap")
  return { json, rgba }
}

export function decodeRig(
  baseInput: ByteInput,
  baseName: string,
  poseInput: ByteInput | null,
  poseName: string | null,
  overridesJSON: string,
): NativeDecodeResult {
  decoderWarnings.length = 0
  diagnosticBytes = 0
  imageAllocationBytes = 0
  const baseBytes = bytesOf(baseInput, "base PSD")
  const poseBytes = poseInput === null ? null : bytesOf(poseInput, "pose PSD")
  safeAdd(baseBytes.byteLength, poseBytes?.byteLength ?? 0, "combined PSD input bytes", caps.maxPackBytes)
  const baseAllocation = preflightPSD(baseBytes, "base PSD")
  const poseAllocation = poseBytes ? preflightPSD(poseBytes, "pose PSD") : null
  safeAdd(baseAllocation.rgba, poseAllocation?.rgba ?? 0, "combined PSD decoded allocation", caps.maxDecodedLayerBytes)
  safeAdd(baseAllocation.channels, poseAllocation?.channels ?? 0, "combined PSD decoded channel allocation", caps.maxPSDChannelBytes)
  const overrides = parseNativeRigOverrides(overridesJSON, caps.maxOverrideBytes)
  const loader = new PsdRigLoader()
  const base = loadValidated(loader, baseBytes, stringValue(baseName, "base name"), overrides, "base")
  const pose = poseBytes
    ? loadValidated(loader, poseBytes, stringValue(poseName ?? "pose.psd", "pose name"), overrides, "pose")
    : null
  return packDocument(base, pose, compactPhysics(overrides))
}


Object.defineProperty(globalThis, "__herdrRigDecode", { value: decodeRig })
