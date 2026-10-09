import Foundation
import simd

// One contain transform for composition, GPU sampling and native input.
struct RigCanvasMapping {
    let origin: SIMD2<Double>
    let size: SIMD2<Double>
    let scale: Double

    init(canvasWidth: Int, canvasHeight: Int, drawableWidth: Int, drawableHeight: Int) {
        scale = min(Double(drawableWidth) / Double(canvasWidth), Double(drawableHeight) / Double(canvasHeight))
        size = SIMD2(Double(canvasWidth) * scale, Double(canvasHeight) * scale)
        origin = (SIMD2(Double(drawableWidth), Double(drawableHeight)) - size) / 2
    }

    func drawablePoint(modelX: Double, modelY: Double) -> SIMD2<Double> {
        origin + SIMD2(modelX, modelY) * scale
    }

    func modelPoint(drawableX: Double, drawableY: Double) -> SIMD2<Double> {
        (SIMD2(drawableX, drawableY) - origin) / scale
    }

    func sampledModelPoint(modelX: Double, modelY: Double) -> SIMD2<Double> {
        let point = drawablePoint(modelX: modelX, modelY: modelY)
        return modelPoint(drawableX: floor(point.x) + 0.5, drawableY: floor(point.y) + 0.5)
    }
}

let rigEyeAlphaCutoff: Float = 0.25

// Missing side is the legacy unsided aperture, never the opposite eye.
private func rigEyeBit(_ side: String?) -> UInt8 {
    switch side {
    case "L": return 1
    case "R": return 2
    default: return 4
    }
}

private func rigIrisMask(_ side: String?) -> UInt8 {
    switch side {
    case "L": return 1 | 4
    case "R": return 2 | 4
    default: return 4
    }
}

struct RigSemanticHit {
    let region: String
    let layer: String
    let sourceX: Double
    let sourceY: Double
    let coverage: Double
    let renderSlot: Double
}

struct RigHitSample {
    let alpha: Float
    let semantic: RigSemanticHit?
}

// Project a rendered point back through the actual evaluated triangle. Authored
// interaction areas stay in model/rest coordinates; moving parts carry them.
// The caller separately gates this result with the final rendered alpha and the
// matching frame/viewport/token. This function does not approximate GPU alpha.
func rigSemanticHit(rig: RigDefinition, meshes: [RigMesh], rgba: Data,
    modelX: Double, modelY: Double, minimumAlpha: Double
) -> RigSemanticHit? {
    guard modelX.isFinite, modelY.isFinite, minimumAlpha.isFinite else { return nil }
    var result: RigSemanticHit?
    rgba.withUnsafeBytes { raw in
        let pixels = raw.bindMemory(to: UInt8.self)
        var aperture: UInt8 = 0
        var available: UInt8 = 0
        for mesh in meshes where mesh.layer.name.hasPrefix("eyewhite") {
            available |= rigEyeBit(mesh.layer.side)
            let fragment = mesh.positions.withUnsafeBufferPointer {
                rigFragment(
                    layer: mesh.layer, positions: $0, base: mesh.base, uvs: mesh.uvs,
                    indices: mesh.indices, pixels: pixels, x: modelX, y: modelY)
            }
            if let fragment, fragment.alpha >= Double(rigEyeAlphaCutoff) {
                aperture |= rigEyeBit(mesh.layer.side)
            }
        }
        for mesh in meshes {
            let white = mesh.layer.name.hasPrefix("eyewhite")
            if mesh.alpha < 0.004 && !white { continue }
            if mesh.layer.name.hasPrefix("irides") {
                let mask = rigIrisMask(mesh.layer.side)
                if available & mask != 0 && aperture & mask == 0 { continue }
            }
            let fragment = mesh.positions.withUnsafeBufferPointer {
                rigFragment(layer: mesh.layer, positions: $0, base: mesh.base, uvs: mesh.uvs,
                            indices: mesh.indices, pixels: pixels, x: modelX, y: modelY)
            }
            guard let fragment else { continue }
            if white && fragment.alpha < Double(rigEyeAlphaCutoff) { continue }
            let coverage = fragment.alpha * Double(mesh.alpha)
            if coverage > minimumAlpha {
                result = rigRegionHit(rig: rig, layer: mesh.layer, fragment: fragment, coverage: coverage)
            }
        }
    }
    return result
}

private struct RigFragment {
    let alpha: Double
    let sourceX: Double
    let sourceY: Double
}

private func rigFragment(layer: RigLayer, positions: UnsafeBufferPointer<SIMD2<Float>>,
                         base: [SIMD2<Float>], uvs: [SIMD2<Float>], indices: [UInt16],
                         pixels: UnsafeBufferPointer<UInt8>, x: Double, y: Double) -> RigFragment? {
    let image = layer.img
    guard image.width > 0, image.height > 0, image.offset >= 0, image.length >= 0,
          image.offset <= pixels.count, image.length <= pixels.count - image.offset else { return nil }
    for index in stride(from: 0, to: indices.count - 2, by: 3) {
        let i0 = Int(indices[index]), i1 = Int(indices[index + 1]), i2 = Int(indices[index + 2])
        let p0 = positions[i0], p1 = positions[i1], p2 = positions[i2]
        let ax = Double(p1.x - p0.x), ay = Double(p1.y - p0.y)
        let bx = Double(p2.x - p0.x), by = Double(p2.y - p0.y)
        let px = x - Double(p0.x), py = y - Double(p0.y)
        let determinant = ax * by - ay * bx
        if !determinant.isFinite || abs(determinant) < 1e-12 { continue }
        let b = (px * by - py * bx) / determinant
        let c = (ax * py - ay * px) / determinant
        let a = 1 - b - c
        if a < -1e-7 || b < -1e-7 || c < -1e-7 { continue }
        let uv0 = uvs[i0], uv1 = uvs[i1], uv2 = uvs[i2]
        let u = a * Double(uv0.x) + b * Double(uv1.x) + c * Double(uv2.x)
        let v = a * Double(uv0.y) + b * Double(uv1.y) + c * Double(uv2.y)
        let tx = min(Double(image.width - 1), max(0, u * Double(image.width) - 0.5))
        let ty = min(Double(image.height - 1), max(0, v * Double(image.height) - 0.5))
        guard tx.isFinite, ty.isFinite else { return nil }
        let x0 = Int(floor(tx)), y0 = Int(floor(ty))
        let x1 = min(image.width - 1, x0 + 1), y1 = min(image.height - 1, y0 + 1)
        let fx = tx - Double(x0), fy = ty - Double(y0)
        func alpha(_ x: Int, _ y: Int) -> Double {
            let local = (y * image.width + x) * 4 + 3
            return local < image.length ? Double(pixels[image.offset + local]) / 255 : 0
        }
        let top = alpha(x0, y0) * (1 - fx) + alpha(x1, y0) * fx
        let bottom = alpha(x0, y1) * (1 - fx) + alpha(x1, y1) * fx
        let r0 = base[i0], r1 = base[i1], r2 = base[i2]
        return RigFragment(alpha: top * (1 - fy) + bottom * fy,
                           sourceX: a * Double(r0.x) + b * Double(r1.x) + c * Double(r2.x),
                           sourceY: a * Double(r0.y) + b * Double(r1.y) + c * Double(r2.y))
    }
    return nil
}

private func rigRegionHit(rig: RigDefinition, layer: RigLayer, fragment: RigFragment,
                          coverage: Double) -> RigSemanticHit? {
    guard let regions = rig.interactionAreas else { return nil }
    func contains(_ bounds: RigBounds?) -> Bool {
        guard let bounds else { return false }
        return fragment.sourceX >= bounds.x0 && fragment.sourceX < bounds.x1
            && fragment.sourceY >= bounds.y0 && fragment.sourceY < bounds.y1
    }
    let region: String
    if contains(regions["head"]) { region = "head" }
    else if contains(regions["torso"]) { region = "body" }
    else { return nil }
    return RigSemanticHit(region: region, layer: layer.name, sourceX: fragment.sourceX,
                          sourceY: fragment.sourceY, coverage: coverage, renderSlot: layer.z)
}

#if RIG_PROBE_FAULTS
// One immutable-geometry view per existing Metal position-buffer slot. The
// renderer updates only its next slot, and publishes it after GPU completion.
// No vertex copies, GPU waits, texture readback, or per-query arrays are needed.
final class RigRasterMesh {
    let source: RigMesh
    let positions: UnsafeBufferPointer<SIMD2<Float>>
    let eyeWhite: Bool
        let irisMask: UInt8
    var alpha: Float = 0
    private var x0 = Double.infinity, y0 = Double.infinity
    private var x1 = -Double.infinity, y1 = -Double.infinity

    init(source: RigMesh, positions: UnsafeBufferPointer<SIMD2<Float>>) {
        self.source = source
        self.positions = positions
        self.eyeWhite = source.layer.name.hasPrefix("eyewhite")
            self.irisMask = source.layer.name.hasPrefix("irides") ? rigIrisMask(source.layer.side) : 0
    }

    func refreshBounds() {
        x0 = .infinity; y0 = .infinity; x1 = -.infinity; y1 = -.infinity
        for point in positions {
            x0 = min(x0, Double(point.x)); x1 = max(x1, Double(point.x))
            y0 = min(y0, Double(point.y)); y1 = max(y1, Double(point.y))
        }
    }

    fileprivate func fragment(pixels: UnsafeBufferPointer<UInt8>, x: Double, y: Double) -> RigFragment? {
        guard x >= x0, x <= x1, y >= y0, y <= y1 else { return nil }
        return rigFragment(layer: source.layer, positions: positions, base: source.base,
                           uvs: source.uvs, indices: source.indices, pixels: pixels, x: x, y: y)
    }
}

private func rigUNorm8(_ value: Double) -> Double {
    min(255, max(0, (value * 255).rounded())) / 255
}

final class RigRasterModel {
    let rig: RigDefinition
    let meshes: [RigRasterMesh]
        private let availableEyeBits: UInt8

    init(rig: RigDefinition, meshes: [RigRasterMesh]) {
        self.rig = rig
        self.meshes = meshes
            self.availableEyeBits = meshes.reduce(UInt8(0)) {
                $0 | ($1.eyeWhite ? rigEyeBit($1.source.layer.side) : 0)
        }
    }

    fileprivate func sample(pixels: UnsafeBufferPointer<UInt8>, x: Double, y: Double,
                            minimumAlpha: Double) -> RigHitSample {
            var aperture: UInt8 = 0
            for mesh in meshes where mesh.eyeWhite {
                if let fragment = mesh.fragment(pixels: pixels, x: x, y: y),
                    fragment.alpha >= Double(rigEyeAlphaCutoff)
                {
                    aperture |= rigEyeBit(mesh.source.layer.side)
                }
            }
        var alpha = 0.0
        var semantic: RigSemanticHit?
        for mesh in meshes {
            if mesh.alpha < 0.004 && !mesh.eyeWhite { continue }
                if mesh.irisMask & availableEyeBits != 0 && mesh.irisMask & aperture == 0 { continue }
            guard let fragment = mesh.fragment(pixels: pixels, x: x, y: y) else { continue }
                if mesh.eyeWhite && fragment.alpha < Double(rigEyeAlphaCutoff) { continue }
            let coverage = fragment.alpha * Double(mesh.alpha)
            alpha = rigUNorm8(coverage + alpha * (1 - coverage))
            if coverage > minimumAlpha,
               let hit = rigRegionHit(rig: rig, layer: mesh.source.layer, fragment: fragment, coverage: coverage) {
                semantic = hit
            }
        }
        return RigHitSample(alpha: Float(alpha), semantic: semantic)
    }

    fileprivate func filteredAlpha(pixels: UnsafeBufferPointer<UInt8>, x: Double, y: Double) -> Double {
        let tx = min(Double(rig.canvas.w - 1), max(0, x - 0.5))
        let ty = min(Double(rig.canvas.h - 1), max(0, y - 0.5))
        let ix = floor(tx), iy = floor(ty), fx = tx - ix, fy = ty - iy
        func alpha(_ x: Double, _ y: Double) -> Double {
            Double(sample(pixels: pixels, x: x + 0.5, y: y + 0.5, minimumAlpha: 1).alpha)
        }
        let a = alpha(ix, iy)
        let b = fx == 0 ? a : alpha(min(Double(rig.canvas.w - 1), ix + 1), iy)
        if fy == 0 { return a * (1 - fx) + b * fx }
        let c = alpha(ix, min(Double(rig.canvas.h - 1), iy + 1))
        let d = fx == 0 ? c : alpha(min(Double(rig.canvas.w - 1), ix + 1), min(Double(rig.canvas.h - 1), iy + 1))
        return (a * (1 - fx) + b * fx) * (1 - fy) + (c * (1 - fx) + d * fx) * fy
    }
}

func rigRasterHit(base: RigRasterModel, pose: RigRasterModel?, rgba: Data, poseMix: Float,
                  modelX: Double, modelY: Double, drawableWidth: Int, drawableHeight: Int,
                  minimumAlpha: Double) -> RigHitSample {
    let width = Double(base.rig.canvas.w), height = Double(base.rig.canvas.h)
    guard modelX.isFinite, modelY.isFinite, modelX >= 0, modelY >= 0,
          modelX < width, modelY < height, drawableWidth > 0, drawableHeight > 0 else {
        return RigHitSample(alpha: 0, semantic: nil)
    }
    let mapping = RigCanvasMapping(canvasWidth: Int(base.rig.canvas.w), canvasHeight: Int(base.rig.canvas.h),
                                   drawableWidth: drawableWidth, drawableHeight: drawableHeight)
    let point = mapping.sampledModelPoint(modelX: modelX, modelY: modelY)
    let x = point.x, y = point.y
    return rgba.withUnsafeBytes { raw in
        let pixels = raw.bindMemory(to: UInt8.self)
        let weight = pose == nil ? 0 : Double(poseMix)
        var alpha = 0.0
        var semantic: RigSemanticHit?
        func blend(_ model: RigRasterModel, _ weight: Double) {
            guard weight > 0 else { return }
            alpha = rigUNorm8(alpha + model.filteredAlpha(pixels: pixels, x: x, y: y) * weight)
            if let hit = model.sample(pixels: pixels, x: x, y: y, minimumAlpha: minimumAlpha).semantic {
                let coverage = hit.coverage * weight
                if coverage > minimumAlpha && coverage > (semantic?.coverage ?? 0) {
                    semantic = RigSemanticHit(region: hit.region, layer: hit.layer, sourceX: hit.sourceX,
                                              sourceY: hit.sourceY, coverage: coverage, renderSlot: hit.renderSlot)
                }
            }
        }
        blend(base, 1 - weight)
        if let pose { blend(pose, weight) }
        return RigHitSample(alpha: Float(alpha), semantic: alpha > minimumAlpha ? semantic : nil)
    }
}
#endif

struct RigCachedHit {
    let sample: RigHitSample
    let frameNumber: UInt64
    let capturedAt: Double
    let drawableWidth: Int
    let drawableHeight: Int
}
