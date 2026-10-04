#!/usr/bin/env node
// Source-pinned transparency cleanup of explicitly reviewed hair background pockets.
import fs from 'node:fs'
import path from 'node:path'
import { createHash } from 'node:crypto'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

const require = createRequire(new URL('../web/rig/package.json', import.meta.url))
const { initializeCanvas, readPsd, writePsd } = require('ag-psd')
initializeCanvas(() => { throw Error('Canvas is not used') },
  (width, height) => ({ width, height, data: new Uint8ClampedArray(width * height * 4), colorSpace: 'srgb' }))
const sha256 = value => createHash('sha256').update(value).digest('hex')
const decoderOptions = { useImageData: true, skipThumbnail: true }

// Match the canonical source PSD exporter's neutral composite, not the runtime's
// all-visible expression-texture inventory. Never trim or re-bbox a repaired layer.
export function neutralComposite(psd) {
  const data = new Uint8ClampedArray(psd.width * psd.height * 4)
  let previous
  for (const layer of psd.children) {
    if (layer.children || !layer.imageData || layer.opacity !== 1 || layer.blendMode !== 'normal') {
      throw Error('Expected flat, fully opaque normal-mode source rig layers')
    }
    const image = layer.imageData
    const hidden = layer.hidden || /^eye_close[-_]/.test(layer.name) || ['mouth_open', 'mouth_smile'].includes(layer.name)
    if (!hidden) for (let y = 0; y < image.height; y++) for (let x = 0; x < image.width; x++) {
      const xx = layer.left + x, yy = layer.top + y
      if (xx < 0 || yy < 0 || xx >= psd.width || yy >= psd.height) continue
      const s = (y * image.width + x) * 4, d = (yy * psd.width + xx) * 4
      let alpha = image.data[s + 3] / 255
      if (layer.clipping) {
        if (!previous) throw Error('Clipping layer without adjacent base')
        const px = xx - previous.left, py = yy - previous.top, base = previous.imageData
        const baseAlpha = px >= 0 && py >= 0 && px < base.width && py < base.height
          ? base.data[(py * base.width + px) * 4 + 3] : 0
        alpha = Math.round(image.data[s + 3] * baseAlpha / 255) / 255
      }
      const prior = data[d + 3] / 255, combined = alpha + prior * (1 - alpha)
      if (!combined) continue
      for (let c = 0; c < 3; c++) data[d + c] = Math.round((image.data[s + c] * alpha + data[d + c] * prior * (1 - alpha)) / combined)
      data[d + 3] = Math.round(combined * 255)
    }
    previous = layer
  }
  return { width: psd.width, height: psd.height, data }
}

export function repairHairPockets(input, plan) {
  if (plan.version !== 1 || plan.inputSha256 !== sha256(input)) throw Error('Plan does not match the immutable input PSD')
  if (!Number.isInteger(plan.minimumChannel) || !Number.isInteger(plan.maximumChroma)
      || plan.minimumChannel < 0 || plan.minimumChannel > 255 || plan.maximumChroma < 0 || plan.maximumChroma > 255
      || !Array.isArray(plan.pockets) || !plan.pockets.length) throw Error('Invalid reviewed pocket plan')
  const psd = readPsd(input, decoderOptions)
  const before = psd.children.map(layer => ({
    metadata: JSON.stringify(Object.fromEntries(Object.entries(layer).filter(([key]) => key !== 'imageData' && key !== 'canvas'))),
    data: new Uint8ClampedArray(layer.imageData.data),
  }))
  const originalComposite = neutralComposite(psd)
  if (!psd.imageData) throw Error('Source PSD needs its neutral composite')
  const selected = new Map()
  for (const pocket of plan.pockets) {
    const handBoundary = plan.matteRepairs && plan.handBoundaryLayers?.includes(pocket.layer)
      && ['handwear-l', 'handwear-r'].includes(pocket.layer)
    if (!['back hair', 'front hair'].includes(pocket.layer) && !handBoundary) {
      throw Error('Pocket cleanup only accepts named hair layers or explicitly reviewed hand boundaries')
    }
    const matches = psd.children.filter(layer => layer.name === pocket.layer)
    if (matches.length !== 1) throw Error(`Ambiguous or missing hair layer: ${pocket.layer}`)
    const layer = matches[0], image = layer.imageData, [sx, sy] = pocket.seed
    if (!Number.isInteger(sx) || !Number.isInteger(sy)) throw Error('Seeds must be integer canvas coordinates')
    const x = sx - layer.left, y = sy - layer.top
    if (x < 0 || y < 0 || x >= image.width || y >= image.height) throw Error('Seed outside its hair raster')
    let mask = selected.get(layer)
    if (!mask) { mask = new Uint8Array(image.width * image.height); selected.set(layer, mask) }
    const eligible = index => {
      const offset = index * 4, rgb = image.data.subarray(offset, offset + 3)
      return image.data[offset + 3] > 0 && Math.min(...rgb) >= plan.minimumChannel
        && Math.max(...rgb) - Math.min(...rgb) <= plan.maximumChroma
    }
    const start = y * image.width + x
    if (mask[start]) continue
    if (!eligible(start)) throw Error(`Reviewed seed is not background-like: ${pocket.layer} ${sx},${sy}`)
    const queue = [start]
    mask[start] = 1
    for (let i = 0; i < queue.length; i++) {
      const index = queue[i], px = index % image.width, py = Math.floor(index / image.width)
      const neighbors = []
      if (px > 0) neighbors.push(index - 1)
      if (px + 1 < image.width) neighbors.push(index + 1)
      if (py > 0) neighbors.push(index - image.width)
      if (py + 1 < image.height) neighbors.push(index + image.width)
      for (const next of neighbors) if (!mask[next] && eligible(next)) { mask[next] = 1; queue.push(next) }
    }
  }
  const layers = []
  for (const [layer, mask] of selected) {
    if (!plan.matteRepairs) {
      // Zero transparent RGB too, so texture filtering cannot bleed discarded white.
      for (let i = 0; i < mask.length; i++) if (mask[i]) layer.imageData.data.fill(0, i * 4, i * 4 + 4)
    }
  }
  for (const repair of plan.matteRepairs ?? []) {
    const layer = psd.children.find(item => item.name === repair.layer), mask = selected.get(layer)
    if (!mask || !Array.isArray(repair.pixels)) throw Error('Matte repair requires a reviewed hair layer')
    const image = layer.imageData
    const original = before[psd.children.indexOf(layer)].data
    const record = plan.matteRecords?.find(item => item.layer === repair.layer)
    if (record?.sourceRgbaSha256 !== sha256(original)) throw Error('Matte source raster does not match the frozen PSD hair layer')
    for (const pixel of repair.pixels) {
      if (pixel.length !== 6 || !pixel.every(Number.isInteger)) throw Error('Matte pixels require canvas x/y and byte RGBA')
      const [sx, sy, ...rgba] = pixel, x = sx - layer.left, y = sy - layer.top
      if (x < 0 || y < 0 || x >= image.width || y >= image.height || rgba.some(value => value < 0 || value > 255)) {
        throw Error('Matte pixel outside its source hair raster')
      }
      const index = y * image.width + x
      if (rgba[3] > original[index * 4 + 3]) throw Error('Background matte cannot add alpha')
      image.data.set(rgba, index * 4)
      mask[index] = 1
    }
  }
  for (const [layer] of selected) {
    const old = before[psd.children.indexOf(layer)].data, data = layer.imageData.data
    let pixelsCleared = 0, pixelsChanged = 0
    for (let i = 0; i < data.length; i += 4) {
      if (old[i + 3] && !data[i + 3]) pixelsCleared++
      if (data[i] !== old[i] || data[i + 1] !== old[i + 1] || data[i + 2] !== old[i + 2] || data[i + 3] !== old[i + 3]) pixelsChanged++
    }
    layers.push({ name: layer.name, pixelsCleared, pixelsChanged, left: layer.left, top: layer.top,
      width: layer.imageData.width, height: layer.imageData.height })
  }
  const rebuilt = neutralComposite(psd)
  // PSD's merged-preview white-matte encoding quantizes low-alpha RGB. Preserve
  // the decoded original preview outside the pixels changed by the layer repair.
  for (let i = 0; i < rebuilt.data.length; i += 4) {
    if (rebuilt.data[i] !== originalComposite.data[i] || rebuilt.data[i + 1] !== originalComposite.data[i + 1]
        || rebuilt.data[i + 2] !== originalComposite.data[i + 2] || rebuilt.data[i + 3] !== originalComposite.data[i + 3]) {
      psd.imageData.data.set(rebuilt.data.subarray(i, i + 4), i)
    }
  }
  const output = Buffer.from(writePsd(psd, { noBackground: true, trimImageData: false }))
  const decoded = readPsd(output, decoderOptions)
  if (decoded.children.length !== psd.children.length) throw Error('PSD layer loss')
  for (let i = 0; i < decoded.children.length; i++) {
    const layer = decoded.children[i], expected = psd.children[i], mask = selected.get(expected), old = before[i].data
    const structure = JSON.stringify(Object.fromEntries(Object.entries(layer).filter(([key]) => key !== 'imageData' && key !== 'canvas')))
    if (structure !== before[i].metadata || sha256(layer.imageData.data) !== sha256(expected.imageData.data)) {
      throw Error(`PSD metadata or raster round-trip changed: ${layer.name}`)
    }
    for (let p = 0; p < old.length; p++) if (old[p] !== layer.imageData.data[p]
        && !mask?.[Math.floor(p / 4)]) {
      throw Error(`Non-pocket pixel changed: ${layer.name}`)
    }
  }
  return { output, report: { version: 1, method: plan.matteRepairs
      ? 'Reviewed source-guided alpha matte and background colour unmixing; no re-bbox'
      : 'Reviewed four-neighbour near-neutral hair components; transparent black; no re-bbox',
    inputSha256: plan.inputSha256, outputSha256: sha256(output), plan, layers,
    allLayerMetadataPreserved: true, allNonSelectedRgbaPreserved: true } }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2)
  if (args.length !== 4) throw Error('Usage: node tools/repair-hair-pockets.mjs INPUT.psd PLAN.json OUTPUT.psd REPORT.json')
  const [source, planPath, destination, reportPath] = args
  if (fs.existsSync(destination) || fs.existsSync(reportPath)) throw Error('Repair outputs must be new; never overwrite an input or installed pack')
  const { output, report } = repairHairPockets(fs.readFileSync(source), JSON.parse(fs.readFileSync(planPath, 'utf8')))
  report.planSha256 = sha256(fs.readFileSync(planPath))
  fs.writeFileSync(destination, output, { flag: 'wx' })
  fs.writeFileSync(reportPath, JSON.stringify(report, null, 2) + '\n', { flag: 'wx' })
  console.log(JSON.stringify(report.layers))
}
