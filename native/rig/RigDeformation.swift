// Native port of the pinned Anime25DRenderer deformation path.
// Source/provenance: web/rig/vendor/anime25drig (DAEMONLET adaptation of
// Anime2.5DRig, MIT; see UPSTREAM.txt and LICENSE).
import Foundation
import simd

private let rigBreathPeriodSeconds = 3.8
private let rigBreathAmplitude = 1.6

private func rigClamp(_ value: Double, _ minimum: Double, _ maximum: Double) -> Double {
    max(minimum, min(maximum, value))
}

private func rigSmooth(_ value: Double) -> Double {
    let t = rigClamp(value, 0, 1)
    return t * t * (3 - 2 * t)
}

private func rigFinite(_ value: Double, _ fallback: Double) -> Double {
    value.isFinite ? value : fallback
}

private func rigSign(_ value: Double) -> Int {
    if value > 0 { return 1 }
    if value < 0 { return -1 }
    return 0
}

private func rigBaseName(_ name: String) -> String {
    var value = name.replacingOccurrences(of: "(_l|_r)$", with: "", options: [.regularExpression, .caseInsensitive])
    value = value.replacingOccurrences(of: "_\\d+$", with: "", options: [.regularExpression])
    return value
}

private func rigRotate(_ x: Double, _ y: Double, _ cx: Double, _ cy: Double, _ angle: Double) -> (Double, Double) {
    if angle == 0 { return (x, y) }
    let cosine = cos(angle)
    let sine = sin(angle)
    let rx = x - cx
    let ry = y - cy
    return (cx + rx * cosine - ry * sine, cy + rx * sine + ry * cosine)
}

private struct RigSpringAxis {
    var x: Double
    var v: Double
    var dx: Double
}

private struct RigStrandSpring {
    var stiff: RigSpringAxis
    var soft: RigSpringAxis
    let phase: Double
}

private struct RigHairTuning {
    let amplitude: Double
    let stiffness: Double
    let damping: Double
    let wind: Double
    let inertia: Double
    let rootLock: Double
    let maxOffset: Double
}

private let rigFrontHairTuning = RigHairTuning(
    amplitude: 0.72, stiffness: 78, damping: 12, wind: 0.34,
    inertia: 0.62, rootLock: 0.3, maxOffset: 14
)
private let rigBackHairTuning = RigHairTuning(
    amplitude: 0.86, stiffness: 48, damping: 9.5, wind: 0.46,
    inertia: 0.82, rootLock: 0.24, maxOffset: 22
)

private func rigNameWithoutNumericSuffix(_ name: String) -> String {
    name.replacingOccurrences(of: "_\\d+$", with: "", options: [.regularExpression])
}

private func rigNormalizedHairTuning(_ source: RigHairPhysicsTuning, fallback: RigHairTuning) -> RigHairTuning {
    func bounded(_ value: Double, _ minimum: Double, _ maximum: Double, _ fallbackValue: Double) -> Double {
        let finite = value.isFinite ? value : fallbackValue
        return rigClamp(finite, minimum, maximum)
    }
    return RigHairTuning(
        amplitude: bounded(source.amplitude, 0, 3, fallback.amplitude),
        stiffness: bounded(source.stiffness, 1, 240, fallback.stiffness),
        damping: bounded(source.damping, 0.1, 60, fallback.damping),
        wind: bounded(source.wind, 0, 3, fallback.wind),
        inertia: bounded(source.inertia, 0, 3, fallback.inertia),
        rootLock: bounded(source.rootLock, 0, 0.95, fallback.rootLock),
        maxOffset: bounded(source.maxOffset, 0, 160, fallback.maxOffset)
    )
}

private func rigNormalizedHairTuning(_ source: [String: Double]?, fallback: RigHairTuning) -> RigHairTuning {
    let values = source ?? [:]
    func value(_ name: String, _ fallbackValue: Double) -> Double {
        guard let candidate = values[name], candidate.isFinite else { return fallbackValue }
        return candidate
    }
    return RigHairTuning(
        amplitude: rigClamp(value("amplitude", fallback.amplitude), 0, 3),
        stiffness: rigClamp(value("stiffness", fallback.stiffness), 1, 240),
        damping: rigClamp(value("damping", fallback.damping), 0.1, 60),
        wind: rigClamp(value("wind", fallback.wind), 0, 3),
        inertia: rigClamp(value("inertia", fallback.inertia), 0, 3),
        rootLock: rigClamp(value("rootLock", fallback.rootLock), 0, 0.95),
        maxOffset: rigClamp(value("maxOffset", fallback.maxOffset), 0, 160)
    )
}

private struct RigHairPhysicsState {
    let frontHair: RigHairTuning
    let backHair: RigHairTuning
    let layers: [String: [String: Double]]

    init(_ config: RigHairPhysicsConfig?) {
        if let config {
            self.frontHair = rigNormalizedHairTuning(config.frontHair, fallback: rigFrontHairTuning)
            self.backHair = rigNormalizedHairTuning(config.backHair, fallback: rigBackHairTuning)
            self.layers = config.layers
        } else {
            self.frontHair = rigFrontHairTuning
            self.backHair = rigBackHairTuning
            self.layers = [:]
        }
    }

    func tuning(for layer: RigLayer) -> RigHairTuning {
        let fallback = rigBaseName(layer.name) == "front hair" ? frontHair : backHair
        let override = layers[layer.name] ?? layers[rigNameWithoutNumericSuffix(layer.name)]
        return rigNormalizedHairTuning(override, fallback: fallback)
    }
}

private func rigRootWeightedAmount(_ u: Double, _ rootLock: Double, _ exponent: Double) -> Double {
    let unlocked = rigClamp((u - rootLock) / max(0.001, 1 - rootLock), 0, 1)
    return pow(unlocked, exponent)
}

private func rigBlendSpringOffset(_ stiff: Double, _ soft: Double, _ softness: Double) -> Double {
    let mix = rigClamp(softness, 0, 1)
    return stiff * (1 - mix) + soft * mix
}

private func rigStepSpring(_ spring: inout RigSpringAxis, target: Double, stiffness: Double, damping: Double, maxOffset: Double, dt: Double) {
    var remaining = rigClamp(rigFinite(dt, 0), 0, 0.25)
    var x = rigFinite(spring.x, target)
    var velocity = rigFinite(spring.v, 0)
    let limit = max(0, rigFinite(maxOffset, 0))
    while remaining > 1e-8 {
        let step = min(remaining, 1.0 / 240.0)
        let acceleration = -stiffness * (x - target) - damping * velocity
        velocity = rigFinite(velocity + acceleration * step, 0)
        x = rigFinite(x + velocity * step, target)
        let bounded = rigClamp(x, target - limit, target + limit)
        if bounded != x && rigSign(velocity) == rigSign(x - target) { velocity = 0 }
        x = bounded
        remaining -= step
    }
    let displacement = rigClamp(-(x - target), -limit, limit)
    spring = RigSpringAxis(x: x, v: velocity, dx: displacement == 0 ? 0 : displacement)
}

// L1 impulse-response bound for the fixed-step linear spring. If the result
// plus maximum target magnitude stays below maxOffset, the nonlinear clamp is
// unreachable from reset; otherwise callers retain the hard spring clamp.
private func rigSpringPositionGain(stiffness: Double, damping: Double) -> Double {
    let h = 1.0 / 240
    let a = 1 - stiffness * h * h
    let b = 1 - damping * h
    let c = -stiffness * h * h
    let input = stiffness * h * h
    // State is (position, h*velocity). Sum a whole block of impulse response,
    // then bound the infinite tail using the induced infinity norm of F^N.
    var xx = 1.0, xy = 0.0, yx = 0.0, yy = 1.0
    var ix = input, iy = input
    var response = 0.0
    for _ in 0..<4096 {
        response += max(abs(ix), abs(iy))
        let nextIX = a * ix + b * iy
        let nextIY = c * ix + b * iy
        ix = nextIX
        iy = nextIY
        let nextXX = a * xx + b * yx
        let nextXY = a * xy + b * yy
        let nextYX = c * xx + b * yx
        let nextYY = c * xy + b * yy
        xx = nextXX; xy = nextXY; yx = nextYX; yy = nextYY
    }
    let contraction = max(abs(xx) + abs(xy), abs(yx) + abs(yy)) * 1.000001
    guard contraction < 1 else { return .infinity }
    return response / (1 - contraction) * 1.000001
}

private let rigBouncePositionGain = rigSpringPositionGain(stiffness: 140, damping: 4.2)

private func rigEyeFrameOffset(_ x: Double, _ y: Double, _ profile: RigEyeBlinkProfile) -> (Double, Double) {
    let angle = profile.angleDeg * Double.pi / 180
    let cosine = cos(angle)
    let sine = sin(angle)
    return (x * cosine - y * sine, x * sine + y * cosine)
}

private func rigSample(_ values: [Double], _ u: Double, _ profile: RigEyeBlinkProfile) -> Double {
    let position = rigClamp((u - profile.u0) / (profile.u1 - profile.u0), 0, 1) * Double(values.count - 1)
    let index = min(values.count - 2, Int(floor(position)))
    return values[index] + (values[index + 1] - values[index]) * (position - Double(index))
}

private func rigDeformEyeAperture(_ x: Double, _ y: Double, _ eyeOpen: Double, _ profile: RigEyeBlinkProfile) -> (Double, Double) {
    if eyeOpen >= 1 { return (x, y) }
    let angle = profile.angleDeg * Double.pi / 180
    let cosine = cos(angle)
    let sine = sin(angle)
    let dx = x - profile.center.cx
    let dy = y - profile.center.cy
    let u = dx * cosine + dy * sine
    let v = -dx * sine + dy * cosine
    let upper = rigSample(profile.upper, u, profile)
    let lower = rigSample(profile.lower, u, profile)
    let closed = rigSample(profile.closed, u, profile)
    let aperture = rigClamp((eyeOpen - 0.08) / 0.92, 0, 1)
    let along = rigClamp((v - upper) / max(0.01, lower - upper), 0, 1)
    let boundary = upper + (lower - upper) * along
    let shifted = v + (closed - boundary) * (1 - aperture)
    return (
        profile.center.cx + u * cosine - shifted * sine,
        profile.center.cy + u * sine + shifted * cosine
    )
}

private func rigAlignClosedEye(_ x: Double, _ y: Double, _ profile: RigEyeBlinkProfile) -> (Double, Double) {
    let angle = profile.closedRotationDeg * Double.pi / 180
    let cosine = cos(angle)
    let sine = sin(angle)
    let dx = x - profile.closedSource.cx
    let dy = y - profile.closedSource.cy
    return (
        profile.closedTarget.cx + dx * cosine - dy * sine,
        profile.closedTarget.cy + dx * sine + dy * cosine
    )
}

private func rigMouthFrameOffset(_ u: Double, _ v: Double, _ profile: RigMouthMorphProfile) -> (Double, Double) {
    let angle = profile.angleDeg * Double.pi / 180
    let cosine = cos(angle)
    let sine = sin(angle)
    return (u * cosine - v * sine, u * sine + v * cosine)
}

private func rigSample(_ values: [Double], _ t: Double) -> Double {
    let index = rigClamp(t, 0, 1) * Double(values.count - 1)
    let left = Int(floor(index))
    return values[left] + (values[min(left + 1, values.count - 1)] - values[left]) * (index - Double(left))
}

private func rigMouthMorphWeights(_ mouthOpen: Double, _ mouthForm: Double, _ profile: RigMouthMorphProfile?) -> (aperture: Double, neutral: Double, open: Double, smile: Double) {
    let smile = rigSmooth(mouthForm)
    let aperture = rigSmooth(mouthOpen) * (1 - smile)
    let open = rigSmooth(aperture / 0.16)
    if let profile {
        var hasOpenNeutralGap = false
        for index in profile.neutral.lower.indices {
            if profile.neutral.lower[index] - profile.neutral.upper[index] > 0.2 {
                hasOpenNeutralGap = true
                break
            }
        }
        if hasOpenNeutralGap {
            let closed = rigSmooth((smile - 0.84) / 0.16)
            return (aperture, (1 - closed) * (1 - open), (1 - closed) * open, closed)
        }
    }
    return (aperture, (1 - open) * (1 - smile), open, (1 - open) * smile)
}

private func rigMouthShape(_ expression: String, _ profile: RigMouthMorphProfile) -> RigMouthShape {
    switch expression {
    case "neutral": return profile.neutral
    case "open": return profile.open
    case "smile": return profile.smile
    default: return profile.neutral
    }
}

private func rigDeformMouthPoint(_ x: Double, _ y: Double, _ expression: String, _ mouthOpen: Double, _ mouthForm: Double, _ profile: RigMouthMorphProfile) -> (Double, Double) {
    let shape = rigMouthShape(expression, profile)
    let angle = profile.angleDeg * Double.pi / 180
    let cosine = cos(angle)
    let sine = sin(angle)
    let dx = x - profile.center.cx
    let dy = y - profile.center.cy
    let u = dx * cosine + dy * sine
    let v = -dx * sine + dy * cosine
    let t = (u - shape.u0) / (shape.u1 - shape.u0)
    let smile = rigSmooth(mouthForm)
    let aperture = rigMouthMorphWeights(mouthOpen, mouthForm, nil).aperture
    let closedTop = rigSample(profile.neutral.upper, t) + (rigSample(profile.smile.upper, t) - rigSample(profile.neutral.upper, t)) * smile
    let closedBottom = rigSample(profile.neutral.lower, t) + (rigSample(profile.smile.lower, t) - rigSample(profile.neutral.lower, t)) * smile
    let targetTop = closedTop + (rigSample(profile.open.upper, t) - closedTop) * aperture
    let targetBottom = closedBottom + (rigSample(profile.open.lower, t) - closedBottom) * aperture
    let neutralSmileU0 = profile.neutral.u0 + (profile.smile.u0 - profile.neutral.u0) * smile
    let neutralSmileU1 = profile.neutral.u1 + (profile.smile.u1 - profile.neutral.u1) * smile
    let u0 = neutralSmileU0 + (profile.open.u0 - neutralSmileU0) * aperture
    let u1 = neutralSmileU1 + (profile.open.u1 - neutralSmileU1) * aperture
    let sourceTop = rigSample(shape.upper, t)
    let sourceBottom = rigSample(shape.lower, t)
    let mappedV: Double
    if expression != "open" && sourceBottom - sourceTop < 0.1 {
        mappedV = v - sourceTop + (targetTop + targetBottom) / 2
    } else if v <= sourceTop {
        mappedV = v - sourceTop + targetTop
    } else if v >= sourceBottom {
        mappedV = v - sourceBottom + targetBottom
    } else {
        mappedV = targetTop + (targetBottom - targetTop) * ((v - sourceTop) / (sourceBottom - sourceTop))
    }
    let frame = rigMouthFrameOffset(u0 + (u1 - u0) * t, mappedV, profile)
    return (profile.center.cx + frame.0, profile.center.cy + frame.1)
}

private func rigHeadFollowWeight(_ x: Double, _ y: Double, _ profile: RigHeadFollow?) -> Double {
    guard let profile else { return 0 }
    let distance = hypot(x - profile.center.cx, y - profile.center.cy)
    let t = rigClamp((profile.falloffRadius - distance) / (profile.falloffRadius - profile.radius), 0, 1)
    return rigSmooth(t)
}

final class RigMesh {
    let layer: RigLayer
    let nx: Int
    let ny: Int
    let base: [SIMD2<Float>]
    var positions: [SIMD2<Float>]
    let uvs: [SIMD2<Float>]
    let indices: [UInt16]
    var renderSlot: Double
    var alpha: Float

    fileprivate init(layer: RigLayer, nx: Int, ny: Int, base: [SIMD2<Float>], uvs: [SIMD2<Float>], indices: [UInt16]) {
        self.layer = layer
        self.nx = nx
        self.ny = ny
        self.base = base
        self.positions = base
        self.uvs = uvs
        self.indices = indices
        self.renderSlot = layer.z
        self.alpha = 1
    }
}

private final class RigMeshState {
    let baseName: String
    let tuning: RigHairTuning
    var parentIndex = -1
    var strandWeights: [Float] = []
    var strandU: [Float] = []
    var bangWeights: [Float] = []
    var springs: [RigStrandSpring] = []
    var lastDeformedGeneration = -1

    init(baseName: String, tuning: RigHairTuning) {
        self.baseName = baseName
        self.tuning = tuning
    }
}

private struct RigFrameParameters {
    let values: RigParameters
    let breath: Double
    let breathHead: Double
}

final class RigDeformer {
    let rig: RigDefinition
    let meshes: [RigMesh]

    private var states: [RigMeshState]
    private var bounce = RigSpringAxis(x: 0, v: 0, dx: 0)
    private var lastTime: Double?
    private var generation = 0

    init(rig: RigDefinition, physics: RigHairPhysicsConfig? = nil) throws {
        try RigDeformer.validate(rig)
        let resolvedPhysics = RigHairPhysicsState(physics)
        let ordered = rig.layers.enumerated().sorted {
            if $0.element.z != $1.element.z { return $0.element.z < $1.element.z }
            return $0.offset < $1.offset
        }
        var grids: [(nx: Int, ny: Int, vertexCount: Int)] = []
        grids.reserveCapacity(ordered.count)
        var aggregateVertexCount = 0
        var aggregateStrandEntries = 0
        for entry in ordered {
            let layer = entry.element
            let grid = try RigDeformer.meshGrid(for: layer, rig: rig)
            grids.append(grid)
            aggregateVertexCount = try RigDeformer.checkedAdd(
                aggregateVertexCount,
                grid.vertexCount,
                label: "model mesh vertex count"
            )
            guard aggregateVertexCount <= RigLimits.meshVertices else {
                throw RigNativeError.invalid(
                    "deformation model mesh vertex count \(aggregateVertexCount) exceeds RigLimits.meshVertices \(RigLimits.meshVertices)"
                )
            }
            if layer.meshSource == nil {
                let cacheEntries = try RigDeformer.strandCacheEntryCounts(
                    layer: layer,
                    vertexCount: grid.vertexCount
                )
                aggregateStrandEntries = try RigDeformer.checkedAdd(
                    aggregateStrandEntries,
                    cacheEntries.totalEntries,
                    label: "model strand influence entries"
                )
                guard aggregateStrandEntries <= RigLimits.strandWeightEntries else {
                    throw RigNativeError.invalid(
                        "deformation model strand influence entries \(aggregateStrandEntries) exceed RigLimits.strandWeightEntries \(RigLimits.strandWeightEntries)"
                    )
                }
            }
        }

        var builtMeshes: [RigMesh] = []
        var builtStates: [RigMeshState] = []
        builtMeshes.reserveCapacity(ordered.count)
        builtStates.reserveCapacity(ordered.count)

        for (entry, grid) in zip(ordered, grids) {
            let layer = entry.element
            var base: [SIMD2<Float>] = []
            var uvs: [SIMD2<Float>] = []
            base.reserveCapacity(grid.vertexCount)
            uvs.reserveCapacity(grid.vertexCount)
            for y in 0...grid.ny {
                for x in 0...grid.nx {
                    let px = layer.x + layer.w * Double(x) / Double(grid.nx)
                    let py = layer.y + layer.h * Double(y) / Double(grid.ny)
                    let floatX = Float(px)
                    let floatY = Float(py)
                    guard floatX.isFinite, floatY.isFinite else {
                        throw RigNativeError.invalid("deformation layer \(layer.name) geometry exceeds Float bounds")
                    }
                    base.append(SIMD2<Float>(floatX, floatY))
                    uvs.append(SIMD2<Float>(Float(Double(x) / Double(grid.nx)), Float(Double(y) / Double(grid.ny))))
                }
            }

            let indexCount = try RigDeformer.checkedProduct(grid.nx, grid.ny, 6, label: "layer \(layer.name) index count")
            var indices: [UInt16] = []
            indices.reserveCapacity(indexCount)
            for y in 0..<grid.ny {
                for x in 0..<grid.nx {
                    let a = y * (grid.nx + 1) + x
                    let b = a + 1
                    let c = a + grid.nx + 1
                    let d = c + 1
                    try RigDeformer.appendIndex(a, into: &indices, layerName: layer.name)
                    try RigDeformer.appendIndex(b, into: &indices, layerName: layer.name)
                    try RigDeformer.appendIndex(c, into: &indices, layerName: layer.name)
                    try RigDeformer.appendIndex(b, into: &indices, layerName: layer.name)
                    try RigDeformer.appendIndex(d, into: &indices, layerName: layer.name)
                    try RigDeformer.appendIndex(c, into: &indices, layerName: layer.name)
                }
            }
            let mesh = RigMesh(layer: layer, nx: grid.nx, ny: grid.ny, base: base, uvs: uvs, indices: indices)
            let state = RigMeshState(
                baseName: rigBaseName(layer.deformationSource ?? layer.name),
                tuning: resolvedPhysics.tuning(for: layer)
            )
            if layer.meshSource == nil {
                try RigDeformer.populateHair(mesh: mesh, state: state, rig: rig)
            }
            builtMeshes.append(mesh)
            builtStates.append(state)
        }

        self.rig = rig
        self.meshes = builtMeshes
        self.states = builtStates
        try self.connectMeshSources()
    }

    func envelopeSourceLayer(at index: Int) -> RigLayer {
        let parent = states[index].parentIndex
        return parent >= 0 ? envelopeSourceLayer(at: parent) : meshes[index].layer
    }

    // An analytic distance bound from a source mesh vertex to its deformed
    // position. This deliberately uses parameter limits, spring clamps and
    // profile extrema, never sampled animation times.
    func envelopeDisplacement(at index: Int, motion: RigMotionEvaluator) -> Double {
        let state = states[index]
        if state.parentIndex >= 0 {
            return envelopeDisplacement(at: state.parentIndex, motion: motion)
        }
        let layer = meshes[index].layer
        let a = rig.anchors
        let face = abs(a.faceScale)
        let x0 = layer.x, x1 = layer.x + layer.w
        let y0 = layer.y, y1 = layer.y + layer.h
        func radius(_ cx: Double, _ cy: Double) -> Double {
            max(hypot(x0 - cx, y0 - cy), hypot(x0 - cx, y1 - cy),
                hypot(x1 - cx, y0 - cy), hypot(x1 - cx, y1 - cy))
        }
        var distance = 0.0
        let eye = layer.side == "L" ? a.eyeL : (layer.side == "R" ? a.eyeR : nil)
        if state.baseName == "eye_close", let eye {
            if let blink = eye.blink {
                distance += 2 * radius(blink.closedSource.cx, blink.closedSource.cy)
                    + hypot(blink.closedTarget.cx - blink.closedSource.cx,
                            blink.closedTarget.cy - blink.closedSource.cy)
            }
            let cx = eye.blink?.closedTarget.cx ?? (eye.x0 + eye.x1) / 2
            let cy = eye.blink?.closedTarget.cy ?? (eye.y0 + eye.y1) / 2
            distance += 0.5 * (radius(cx, cy) + distance)
        }
        if let expression = layer.mouthExpression, let morph = a.mouth.morph {
            let shape = expression == "open" ? morph.open
                : (expression == "smile" ? morph.smile : morph.neutral)
            let r = radius(morph.center.cx, morph.center.cy) + distance
            let t = max(abs(-r - shape.u0), abs(r - shape.u0))
                / abs(shape.u1 - shape.u0)
            let u = max(abs(morph.neutral.u0), abs(morph.neutral.u1),
                        abs(morph.open.u0), abs(morph.open.u1),
                        abs(morph.smile.u0), abs(morph.smile.u1))
            let mappedU = u * (1 + 2 * t)
            func magnitude(_ curve: [Double]) -> Double {
                curve.reduce(0.0) { max($0, abs($1)) }
            }
            let v = max(
                magnitude(morph.neutral.upper), magnitude(morph.neutral.lower),
                magnitude(morph.open.upper), magnitude(morph.open.lower),
                magnitude(morph.smile.upper), magnitude(morph.smile.lower))
            // For points outside the source aperture, mappedV retains their
            // original v; inside, it interpolates between the target curves.
            distance += r + hypot(mappedU, r + 3 * v)
        }
        if state.baseName == "mouth_open" || state.baseName == "mouth_close"
            || layer.mouthExpression != nil {
            distance += 0.5 * (radius(a.mouth.cx, a.mouth.cy) + distance)
        }
        if layer.fade == "eyeOpen", let eye {
            if state.baseName == "irides" {
                distance += 0.3 * (radius(eye.icx, eye.icy) + distance) + hypot(11, 6) * face
                if eye.blink == nil {
                    distance += 0.8 * (radius(eye.icx, eye.closeY) + distance)
                }
            } else if let blink = eye.blink {
                let magnitude = max(
                    blink.upper.reduce(0.0) { max($0, abs($1)) },
                    blink.lower.reduce(0.0) { max($0, abs($1)) },
                    blink.closed.reduce(0.0) { max($0, abs($1)) })
                distance += 2 * magnitude
            } else {
                distance += 0.85 * (radius(eye.icx, eye.closeY) + distance)
            }
        }
        if layer.fade == "eyeClose", let eye {
            distance += 3 + 14 * face
            let cx = eye.blink?.closedTarget.cx ?? (x0 + x1) / 2
            let cy = eye.blink?.closedTarget.cy ?? (y0 + y1) / 2
            distance += 2 * sin(0.15) * (radius(cx, cy) + distance)
        }
        if state.baseName == "eyebrow" {
            distance += 12.5 * face
            distance += 2 * sin(0.3) * (radius((x0 + x1) / 2, (y0 + y1) / 2) + distance)
        }
        if layer.fade == "mouthOpen", layer.mouthExpression == nil {
            let curve = pow((max(abs(x0 - a.mouth.cx), abs(x1 - a.mouth.cx)) + distance)
                / ((a.mouth.x1 - a.mouth.x0) / 2 + 4), 1.5)
            distance += 0.5 * (radius(a.mouth.cx, a.mouth.y0) + distance)
                + 6 * face * (curve + 0.35)
        }
        if layer.fade == "mouthClose" || layer.mouthExpression != nil {
            distance += 14 * face
            distance += 2 * sin(0.175) * (radius(a.mouth.cx, a.mouth.cy) + distance)
        }
        if state.baseName == "face" {
            distance += 6 * face
        }
        // Head-follow weights are derived from fixed source vertices. Hair
        // attachment and neck blending may vary with deformed y, so retain
        // their exact [0,1] and [0,0.55] upper limits respectively.
        let baseline = state.baseName == "neck" ? 0.55
            : (layer.hairAttachment != nil ? 1.0
               : (layer.group == "head" ? 1.0 : (layer.group == "body" ? 0.16 : 0.0)))
        let headWeight = meshes[index].base.reduce(baseline) { result, vertex in
            let contact = rigHeadFollowWeight(Double(vertex.x), Double(vertex.y), layer.headFollow)
            return max(result, baseline + (1 - baseline) * contact)
        }
        let depthRadius = abs(layer.depth - 1)
        if headWeight > 0 {
            let z = motion.parameterMagnitude("angleZ")
            let x = motion.parameterMagnitude("angleX")
            let y = motion.parameterMagnitude("angleY")
            distance += headWeight * 2 * sin(min(Double.pi, z * 0.07) / 2)
                * (radius(a.neckPivot.cx, a.neckPivot.cy) + distance)
            distance += headWeight * face * x * (14 + 40 * depthRadius
                + 0.028 * (max(abs(y0 - a.neckPivot.cy), abs(y1 - a.neckPivot.cy)) + distance))
            distance += headWeight * face * y * (9 + 30 * depthRadius
                + 0.05 * depthRadius * (max(abs(y0 - a.face.cy), abs(y1 - a.face.cy)) + distance))
        }
        distance += 3.2 * face
        if state.baseName == "topwear" {
            distance += 3.52 * face
            distance += 0.0048 * (radius(a.neckPivot.cx, a.neckPivot.cy) + distance)
            let target = (4.8 + 6 * motion.parameterMagnitude("angleY")
                + 4 * motion.parameterMagnitude("body")) * face
            let unclamped = (rigBouncePositionGain + 1) * target
            let bounceLimit = unclamped < 160 ? unclamped : 160
            distance += 3 * bounceLimit * motion.parameterMagnitude("bust")
        }
        if state.baseName == "handwear" {
            distance += (36 * motion.parameterMagnitude("armY")
                + 40 * motion.parameterMagnitude("armPos")) * face
        }
        if !state.bangWeights.isEmpty {
            distance += 22 * face * (motion.parameterMagnitude("bangL")
                + motion.parameterMagnitude("bangC") + motion.parameterMagnitude("bangR"))
        }
        if !state.springs.isEmpty {
            distance += state.tuning.maxOffset * state.tuning.amplitude
                * motion.parameterMagnitude(state.baseName == "front hair"
                    && layer.hairAttachment == nil ? "fhAmp" : "physAmp") * 2.1 * 1.12
        }
        let bodyAngle = motion.parameterMagnitude("body") * 0.028
        distance += 2 * sin(min(Double.pi, bodyAngle) / 2)
            * (radius(a.bodyPivot.cx, a.bodyPivot.cy) + distance)
        return distance + 1 // Float vertex rounding and bilinear interpolation.
    }

    // Explicit controlled replay only; ordinary resume/swap must preserve age.
    func resetSimulation() {
        lastTime = nil
        bounce = RigSpringAxis(x: 0, v: 0, dx: 0)
        for state in states {
            for index in state.springs.indices {
                state.springs[index].stiff = RigSpringAxis(x: 0, v: 0, dx: 0)
                state.springs[index].soft = RigSpringAxis(x: 0, v: 0, dx: 0)
            }
        }
    }

    func update(
        parameters: RigParameters, time: Double, neutral: Bool,
        localTransforms: [RigLocalTransform]? = nil
    ) {
        let now: Double
        if time.isFinite { now = time } else { now = lastTime ?? 0 }
        let dt: Double
        if let previous = lastTime {
            dt = rigClamp((now - previous) / 1000, 0, 0.25)
        } else {
            dt = 0
        }
        lastTime = now
        let seconds = now / 1000
        let phase = seconds * 2 * Double.pi / rigBreathPeriodSeconds
        let frame = RigFrameParameters(
            values: parameters,
            breath: neutral ? 0 : rigBreathAmplitude * sin(phase),
            breathHead: neutral ? 0 : rigBreathAmplitude * 0.72 * sin(phase - 0.18)
        )

        updatePhysics(frame: frame, seconds: seconds, dt: dt)
        generation &+= 1
        for state in states { state.lastDeformedGeneration = -1 }
        for index in meshes.indices {
            deformMesh(at: index, frame: frame, localTransforms: localTransforms)
        }
        for mesh in meshes { mesh.alpha = Float(alpha(for: mesh.layer, parameters: parameters)) }
    }

    private func updatePhysics(frame: RigFrameParameters, seconds: Double, dt: Double) {
        let anchors = rig.anchors
        for index in states.indices {
            let state = states[index]
            guard !state.springs.isEmpty else { continue }
            let headDX = (frame.values.angleX * 14 + frame.values.angleZ * 0.07 * (anchors.neckPivot.cy - anchors.face.cy)) * anchors.faceScale
            let tuning = state.tuning
            for springIndex in state.springs.indices {
                var spring = state.springs[springIndex]
                let wind = (1.3 * sin(seconds * 0.8 + spring.phase) + 0.7 * sin(seconds * 1.9 + spring.phase * 2.3)) * tuning.wind
                let target = headDX * tuning.inertia + wind * anchors.faceScale
                rigStepSpring(&spring.stiff, target: target, stiffness: tuning.stiffness, damping: tuning.damping, maxOffset: tuning.maxOffset, dt: dt)
                rigStepSpring(&spring.soft, target: target, stiffness: tuning.stiffness * 0.34, damping: tuning.damping * 0.58, maxOffset: tuning.maxOffset, dt: dt)
                state.springs[springIndex] = spring
            }
        }

        let bounceTarget = (frame.breath * 3 - frame.values.angleY * 6 + frame.values.body * 4) * anchors.faceScale
        rigStepSpring(&bounce, target: bounceTarget, stiffness: 140, damping: 4.2, maxOffset: 160, dt: dt)
        bounce.dx *= 3
    }

    private func deformMesh(
        at index: Int, frame: RigFrameParameters, localTransforms: [RigLocalTransform]?
    ) {
        let state = states[index]
        if state.lastDeformedGeneration == generation { return }
        if state.parentIndex >= 0 {
            let parent = state.parentIndex
            deformMesh(at: parent, frame: frame, localTransforms: localTransforms)
            let source = meshes[parent]
            let destination = meshes[index]
            for vertex in destination.positions.indices { destination.positions[vertex] = source.positions[vertex] }
            state.lastDeformedGeneration = generation
            return
        }

        let mesh = meshes[index]
        let layer = mesh.layer
        let anchors = rig.anchors
        let values = frame.values
        let isHead = layer.group == "head"
        let headAngle = values.angleZ * 0.07
        let headCos = cos(headAngle)
        let headSin = sin(headAngle)
        let bodyAngle = values.body * 0.028
        let bodyCos = cos(bodyAngle)
        let bodySin = sin(bodyAngle)
        let eyeAnchor: RigEyeAnchor?
        if layer.side == "L" { eyeAnchor = anchors.eyeL }
        else if layer.side == "R" { eyeAnchor = anchors.eyeR }
        else { eyeAnchor = nil }
        let eyeOpen = layer.side == "L" ? values.eyeOpenL : values.eyeOpenR
        let mouthHalfWidth = (anchors.mouth.x1 - anchors.mouth.x0) / 2
        let faceScale = anchors.faceScale
        let layerCenterX = layer.x + layer.w / 2
        let layerCenterY = layer.y + layer.h / 2
        let chestCX = anchors.neckPivot.cx
        let chestCY = anchors.neckBottom + (anchors.face.y1 - anchors.face.y0) * 0.6
        let chestRX = (anchors.face.x1 - anchors.face.x0) * 0.6
        let chestRY = (anchors.face.y1 - anchors.face.y0) * 0.45

        for vertex in mesh.base.indices {
            let basePoint = mesh.base[vertex]
            var x = Double(basePoint.x)
            var y = Double(basePoint.y)
            if let eyeAnchor, state.baseName == "eye_close" {
                if let blink = eyeAnchor.blink {
                    let aligned = rigAlignClosedEye(x, y, blink)
                    x = aligned.0; y = aligned.1
                }
                let scale = layer.side == "L" ? values.eyeScaleL : values.eyeScaleR
                let cx = eyeAnchor.blink?.closedTarget.cx ?? (eyeAnchor.x0 + eyeAnchor.x1) / 2
                let cy = eyeAnchor.blink?.closedTarget.cy ?? (eyeAnchor.y0 + eyeAnchor.y1) / 2
                x = cx + (x - cx) * scale
                y = cy + (y - cy) * scale
            }
            if let expression = layer.mouthExpression, let morph = anchors.mouth.morph {
                let point = rigDeformMouthPoint(x, y, expression, values.mouthOpen, values.mouthForm, morph)
                x = point.0; y = point.1
            }
            if state.baseName == "mouth_open" || state.baseName == "mouth_close" || layer.mouthExpression != nil {
                x = anchors.mouth.cx + (x - anchors.mouth.cx) * values.mouthScale
                y = anchors.mouth.cy + (y - anchors.mouth.cy) * values.mouthScale
            }
            if layer.fade == "eyeOpen", let eyeAnchor {
                if state.baseName == "irides" {
                    let gaze: (Double, Double)
                    if let blink = eyeAnchor.blink {
                        gaze = rigEyeFrameOffset(values.eyeX * 11 * faceScale, values.eyeY * 6 * faceScale, blink)
                    } else {
                        gaze = (values.eyeX * 11 * faceScale, values.eyeY * 6 * faceScale)
                    }
                    x = eyeAnchor.icx + (x - eyeAnchor.icx) * values.irisScale + gaze.0
                    y = eyeAnchor.icy + (y - eyeAnchor.icy) * values.irisScale + gaze.1
                    if eyeAnchor.blink == nil {
                        let closing = rigSmooth((0.32 - eyeOpen) / 0.32)
                        y = eyeAnchor.closeY + (y - eyeAnchor.closeY) * (1 - 0.8 * closing)
                    }
                } else if let blink = eyeAnchor.blink {
                    let point = rigDeformEyeAperture(x, y, eyeOpen, blink)
                    x = point.0; y = point.1
                } else {
                    y = eyeAnchor.closeY + (y - eyeAnchor.closeY) * (1 - 0.85 * (1 - eyeOpen))
                }
            }
            if layer.fade == "eyeClose", let eyeAnchor {
                let offset = -eyeOpen * 3 + values.eyeCY * 14 * faceScale
                if let blink = eyeAnchor.blink {
                    let shifted = rigEyeFrameOffset(0, offset, blink)
                    x += shifted.0; y += shifted.1
                } else {
                    y += offset
                }
                let angle = values.eyeCAng * 0.3 * (layer.side == "L" ? 1 : -1)
                let rotated = rigRotate(x, y, eyeAnchor.blink?.closedTarget.cx ?? layerCenterX, eyeAnchor.blink?.closedTarget.cy ?? layerCenterY, angle)
                x = rotated.0; y = rotated.1
            }
            if state.baseName == "eyebrow" {
                y += (-values.brow * 9 + (1 - eyeOpen) * 3.5) * faceScale
                let angle = (layer.side == "L" ? values.browAngL + values.browAngSym : values.browAngR - values.browAngSym) * 0.3
                let rotated = rigRotate(x, y, layerCenterX, layerCenterY, angle)
                x = rotated.0; y = rotated.1
            }
            if layer.fade == "mouthOpen", layer.mouthExpression == nil {
                y = anchors.mouth.y0 + (y - anchors.mouth.y0) * (0.5 + 0.5 * values.mouthOpen)
                let curve = pow(abs(x - anchors.mouth.cx) / (mouthHalfWidth + 4), 1.5)
                y -= values.mouthForm * 6 * faceScale * (curve - 0.35)
            }
            if layer.fade == "mouthClose" || layer.mouthExpression != nil {
                if let morph = anchors.mouth.morph {
                    let shifted = rigMouthFrameOffset(0, values.mouthCY * 14 * faceScale, morph)
                    x += shifted.0; y += shifted.1
                } else {
                    y += values.mouthCY * 14 * faceScale
                }
                let rotated = rigRotate(x, y, anchors.mouth.cx, anchors.mouth.cy, values.mouthCAng * 0.35)
                x = rotated.0; y = rotated.1
            }
            if state.baseName == "face", y > anchors.mouth.cy {
                let aperture = anchors.mouth.morph != nil
                    ? rigMouthMorphWeights(values.mouthOpen, values.mouthForm, nil).aperture
                    : values.mouthOpen
                y += aperture * 6 * faceScale * rigSmooth((y - anchors.mouth.cy) / (anchors.face.y1 - anchors.mouth.cy))
            }

            var headWeight = isHead ? 1.0 : (layer.group == "body" ? 0.16 : 0)
            let contactWeight = rigHeadFollowWeight(Double(basePoint.x), Double(basePoint.y), layer.headFollow)
            let depth = layer.depth + (1 - layer.depth) * contactWeight
            if let attachment = layer.hairAttachment {
                headWeight = 1 - 0.84 * rigSmooth((y - attachment.rootY) / max(1, attachment.bodyY - attachment.rootY))
            }
            if state.baseName == "neck" {
                headWeight = 0.55 * rigSmooth((anchors.neckBottom - y) / max(1, anchors.neckBottom - anchors.neckTop))
            }
            headWeight += (1 - headWeight) * contactWeight
            if headWeight > 0 {
                let rx = x - anchors.neckPivot.cx
                let ry = y - anchors.neckPivot.cy
                x += (rx * headCos - ry * headSin - rx) * headWeight
                y += (rx * headSin + ry * headCos - ry) * headWeight
                x += headWeight * faceScale * (values.angleX * (14 + 40 * (depth - 1)) + values.angleX * (anchors.neckPivot.cy - y) * 0.028)
                y += headWeight * faceScale * (-values.angleY * (9 + 30 * (depth - 1)) - values.angleY * (depth - 1) * (y - anchors.face.cy) * 0.05)
            }
            let breath = layer.group == "body" ? frame.breath * 2 : frame.breathHead * 1.6
            y -= (breath + (frame.breathHead * 1.6 - breath) * contactWeight) * faceScale
            if state.baseName == "topwear", y < chestCY {
                y -= (1 - contactWeight) * frame.breath * 2.2 * faceScale * rigSmooth((chestCY - y) / (chestRY * 2))
            }
            if state.baseName == "topwear" {
                x = anchors.neckPivot.cx + (x - anchors.neckPivot.cx) * (1 + (1 - contactWeight) * frame.breath * 0.003)
                let gx = (x - chestCX) / chestRX
                let gy = (y - (chestCY + values.bustY * 70 * faceScale)) / chestRY
                y += (1 - contactWeight) * bounce.dx * values.bust * exp(-gx * gx - gy * gy)
            }
            if state.baseName == "handwear" {
                let weight = rigSmooth((y - layer.y) / layer.h * 1.15)
                y += -values.armY * 30 * faceScale * weight + values.armPos * 40 * faceScale
                x += values.armY * 6 * faceScale * weight * (x < anchors.neckPivot.cx ? 1 : -1)
            }
            if !state.bangWeights.isEmpty && !state.strandU.isEmpty {
                let amount = pow(Double(state.strandU[vertex]), 1.4) * 22 * faceScale
                x += (values.bangL * Double(state.bangWeights[vertex * 3]) + values.bangC * Double(state.bangWeights[vertex * 3 + 1]) + values.bangR * Double(state.bangWeights[vertex * 3 + 2])) * amount
            }
            if !state.springs.isEmpty && !state.strandWeights.isEmpty && !state.strandU.isEmpty {
                let frontHair = state.baseName == "front hair" && layer.hairAttachment == nil
                let tuning = state.tuning
                let strandU = Double(state.strandU[vertex])
                let u = frontHair ? min(1, strandU * 1.35) : strandU
                let amplitude = rigRootWeightedAmount(u, tuning.rootLock, frontHair ? 1.8 : 2.1) * tuning.amplitude * (frontHair ? values.fhAmp : values.physAmp)
                let softMix = rigClamp(pow(u, 1.2) * (frontHair ? values.fhSoft : values.soft), 0, 1)
                var offset = 0.0
                for strand in state.springs.indices {
                    let weight = Double(state.strandWeights[vertex * state.springs.count + strand])
                    let spring = state.springs[strand]
                    offset += weight * rigBlendSpringOffset(spring.stiff.dx, spring.soft.dx, softMix)
                }
                offset = rigClamp(offset, -tuning.maxOffset, tuning.maxOffset)
                x += offset * amplitude
                y += abs(offset) * amplitude * 0.12
            }
            mesh.positions[vertex] = SIMD2<Float>(Float(x), Float(y))
        }

        if abs(bodyAngle) > 1e-4 {
            for vertex in mesh.positions.indices {
                let x = Double(mesh.positions[vertex].x)
                let y = Double(mesh.positions[vertex].y)
                let rx = x - anchors.bodyPivot.cx
                let ry = y - anchors.bodyPivot.cy
                mesh.positions[vertex] = SIMD2<Float>(
                    Float(anchors.bodyPivot.cx + rx * bodyCos - ry * bodySin),
                    Float(anchors.bodyPivot.cy + rx * bodySin + ry * bodyCos)
                )
            }
        }
        if let localTransforms, index < localTransforms.count {
            applyLocalTransform(localTransforms[index], to: mesh)
        }
        state.lastDeformedGeneration = generation
    }

    private func applyLocalTransform(_ local: RigLocalTransform, to mesh: RigMesh) {
        guard let binding = local.binding,
              binding.weights.count == mesh.positions.count,
              local.translateX != 0 || local.translateY != 0
                || local.rotationDeg != 0 || local.scale != 1 else { return }
        let gridX = binding.origin.x * Double(mesh.nx)
        let gridY = binding.origin.y * Double(mesh.ny)
        let x0 = min(mesh.nx - 1, max(0, Int(floor(gridX))))
        let y0 = min(mesh.ny - 1, max(0, Int(floor(gridY))))
        let tx = gridX - Double(x0)
        let ty = gridY - Double(y0)
        let stride = mesh.nx + 1
        let topLeft = mesh.positions[y0 * stride + x0]
        let topRight = mesh.positions[y0 * stride + x0 + 1]
        let bottomLeft = mesh.positions[(y0 + 1) * stride + x0]
        let bottomRight = mesh.positions[(y0 + 1) * stride + x0 + 1]
        let cx = (1 - ty) * ((1 - tx) * Double(topLeft.x) + tx * Double(topRight.x))
            + ty * ((1 - tx) * Double(bottomLeft.x) + tx * Double(bottomRight.x))
        let cy = (1 - ty) * ((1 - tx) * Double(topLeft.y) + tx * Double(topRight.y))
            + ty * ((1 - tx) * Double(bottomLeft.y) + tx * Double(bottomRight.y))
        let angle = local.rotationDeg * Double.pi / 180
        let cosine = cos(angle)
        let sine = sin(angle)
        for vertex in mesh.positions.indices {
            let point = mesh.positions[vertex]
            let x = Double(point.x)
            let y = Double(point.y)
            let rx = (x - cx) * local.scale
            let ry = (y - cy) * local.scale
            let transformedX = cx + rx * cosine - ry * sine + local.translateX
            let transformedY = cy + rx * sine + ry * cosine + local.translateY
            let weight = Double(binding.weights[vertex])
            mesh.positions[vertex] = SIMD2<Float>(
                Float(x + (transformedX - x) * weight),
                Float(y + (transformedY - y) * weight)
            )
        }
    }

    private func alpha(for layer: RigLayer, parameters: RigParameters) -> Double {
        if let expression = layer.mouthExpression {
            if let morph = rig.anchors.mouth.morph {
                let weights = rigMouthMorphWeights(parameters.mouthOpen, parameters.mouthForm, morph)
                switch expression {
                case "neutral": return weights.neutral
                case "open": return weights.open
                case "smile": return weights.smile
                default: return 0
                }
            }
            let open = rigSmooth(parameters.mouthOpen)
            let smile = rigSmooth(parameters.mouthForm)
            switch expression {
            case "smile": return smile
            case "open": return (1 - smile) * open
            case "neutral": return (1 - smile) * (1 - open)
            default: return 0
            }
        }
        guard let fade = layer.fade else { return 1 }
        switch fade {
        case "eyeOpen":
            let value = layer.side == "L" ? parameters.eyeOpenL : parameters.eyeOpenR
            return rigSmooth((value - 0.08) / 0.06)
        case "eyeClose":
            let value = layer.side == "L" ? parameters.eyeOpenL : parameters.eyeOpenR
            return 1 - rigSmooth((value - 0.08) / 0.06)
        case "mouthOpen":
            return rigSmooth((parameters.mouthOpen - (0.05 + parameters.mouthEase * 0.35)) / 0.12)
        case "mouthClose":
            return 1 - rigSmooth((parameters.mouthOpen - (0.05 + parameters.mouthEase * 0.35)) / 0.12)
        default:
            return 1
        }
    }

    private func connectMeshSources() throws {
        for child in meshes.indices {
            guard let sourceName = meshes[child].layer.meshSource else { continue }
            guard let parent = meshes.indices.first(where: { meshes[$0].layer.name == sourceName }) else {
                throw RigNativeError.invalid("deformation layer \(meshes[child].layer.name) references unknown meshSource \(sourceName)")
            }
            guard parent != child else {
                throw RigNativeError.invalid("deformation layer \(meshes[child].layer.name) meshSource references itself")
            }
            guard meshes[parent].nx == meshes[child].nx, meshes[parent].ny == meshes[child].ny else {
                throw RigNativeError.invalid("deformation layer \(meshes[child].layer.name) meshSource \(sourceName) has incompatible topology")
            }
            for vertex in meshes[parent].base.indices {
                guard meshes[parent].base[vertex] == meshes[child].base[vertex] else {
                    throw RigNativeError.invalid("deformation layer \(meshes[child].layer.name) meshSource \(sourceName) has incompatible geometry")
                }
            }
            states[child].parentIndex = parent
        }
        for index in states.indices {
            var current = index
            var hops = 0
            while states[current].parentIndex >= 0 {
                current = states[current].parentIndex
                hops += 1
                if hops > states.count {
                    throw RigNativeError.invalid("deformation meshSource cycle at layer \(meshes[index].layer.name)")
                }
            }
        }
    }

    private static func populateHair(mesh: RigMesh, state: RigMeshState, rig: RigDefinition) throws {
        guard let strands = mesh.layer.strands, !strands.isEmpty else { return }
        let vertexCount = mesh.base.count
        let cacheCounts = try RigDeformer.strandCacheEntryCounts(
            layer: mesh.layer,
            vertexCount: vertexCount
        )
        guard cacheCounts.totalEntries <= RigLimits.strandWeightEntries else {
            throw RigNativeError.invalid(
                "deformation layer \(mesh.layer.name) strand influence entries \(cacheCounts.totalEntries) exceed RigLimits.strandWeightEntries \(RigLimits.strandWeightEntries)"
            )
        }
        var spacing = 120.0
        if strands.count > 1 {
            var distances: [Double] = []
            distances.reserveCapacity(strands.count - 1)
            for index in 1..<strands.count { distances.append(strands[index].x - strands[index - 1].x) }
            distances.sort()
            spacing = distances[distances.count / 2]
        }
        let sigma = spacing * 0.6
        state.strandWeights = [Float](repeating: 0, count: cacheCounts.weightEntries)
        state.strandU = [Float](repeating: 0, count: cacheCounts.strandUEntries)
        state.springs.reserveCapacity(strands.count)
        for index in strands.indices {
            state.springs.append(RigStrandSpring(stiff: RigSpringAxis(x: 0, v: 0, dx: 0), soft: RigSpringAxis(x: 0, v: 0, dx: 0), phase: Double(index) * 1.37 + mesh.layer.z))
        }

        for vertex in 0..<vertexCount {
            let x = Double(mesh.base[vertex].x)
            let y = Double(mesh.base[vertex].y)
            var total = 0.0
            for strand in strands.indices {
                let ratio = sigma == 0 ? (x == strands[strand].x ? 0 : .infinity) : (x - strands[strand].x) / sigma
                let weight = exp(-pow(ratio, 2))
                state.strandWeights[vertex * strands.count + strand] = Float(weight.isFinite ? weight : 0)
                total += weight.isFinite ? weight : 0
            }
            var rootY = 0.0
            var tipY = 0.0
            if total > 1e-6 {
                for strand in strands.indices {
                    let offset = vertex * strands.count + strand
                    let normalized = Double(state.strandWeights[offset]) / total
                    state.strandWeights[offset] = Float(normalized)
                    rootY += normalized * strands[strand].rootY
                    tipY += normalized * strands[strand].tipY
                }
            } else {
                state.strandWeights[vertex * strands.count] = 1
                rootY = strands[0].rootY
                tipY = strands[0].tipY
            }
            state.strandU[vertex] = Float(rigClamp((y - rootY) / max(1, tipY - rootY), 0, 1))
        }

        if cacheCounts.bangEntries > 0 {
            let faceWidth = rig.anchors.face.x1 - rig.anchors.face.x0
            let left = rig.anchors.face.cx - faceWidth * 0.22
            let right = rig.anchors.face.cx + faceWidth * 0.22
            state.bangWeights = [Float](repeating: 0, count: cacheCounts.bangEntries)
            for vertex in 0..<vertexCount {
                let x = Double(mesh.base[vertex].x)
                let first = rigSmooth((x - left) / 36 + 0.5)
                let second = rigSmooth((x - right) / 36 + 0.5)
                state.bangWeights[vertex * 3] = Float(1 - first)
                state.bangWeights[vertex * 3 + 1] = Float(first * (1 - second))
                state.bangWeights[vertex * 3 + 2] = Float(second)
            }
        }
    }

    private static func strandCacheEntryCounts(
        layer: RigLayer,
        vertexCount: Int
    ) throws -> (weightEntries: Int, strandUEntries: Int, bangEntries: Int, totalEntries: Int) {
        guard vertexCount >= 0 else {
            throw RigNativeError.invalid("deformation layer \(layer.name) strand cache vertex count is negative")
        }
        guard let strands = layer.strands, !strands.isEmpty else {
            return (weightEntries: 0, strandUEntries: 0, bangEntries: 0, totalEntries: 0)
        }
        let weightEntries = try RigDeformer.checkedProduct(
            vertexCount,
            strands.count,
            1,
            label: "layer \(layer.name) strand weight entries"
        )
        let strandUEntries = vertexCount
        let bangEntries: Int
        if rigBaseName(layer.deformationSource ?? layer.name) == "front hair" && layer.hairAttachment == nil {
            bangEntries = try RigDeformer.checkedProduct(
                vertexCount,
                3,
                1,
                label: "layer \(layer.name) bang weight entries"
            )
        } else {
            bangEntries = 0
        }
        var totalEntries = try RigDeformer.checkedAdd(
            weightEntries,
            strandUEntries,
            label: "layer \(layer.name) strand cache entries"
        )
        totalEntries = try RigDeformer.checkedAdd(
            totalEntries,
            bangEntries,
            label: "layer \(layer.name) strand cache entries"
        )
        return (
            weightEntries: weightEntries,
            strandUEntries: strandUEntries,
            bangEntries: bangEntries,
            totalEntries: totalEntries
        )
    }

    private static func appendIndex(_ value: Int, into indices: inout [UInt16], layerName: String) throws {
        guard value >= 0 && value <= Int(UInt16.max) else {
            throw RigNativeError.invalid("deformation layer \(layerName) index \(value) exceeds Uint16 bounds")
        }
        indices.append(UInt16(value))
    }
    private static func checkedAdd(_ a: Int, _ b: Int, label: String) throws -> Int {
        guard a >= 0, b >= 0 else { throw RigNativeError.invalid("deformation \(label) is negative") }
        guard b <= Int.max - a else { throw RigNativeError.invalid("deformation \(label) overflows Int") }
        return a + b
    }

    private static func checkedProduct(_ a: Int, _ b: Int, _ c: Int, label: String) throws -> Int {
        guard a >= 0, b >= 0, c >= 0 else { throw RigNativeError.invalid("deformation \(label) is negative") }
        guard a == 0 || b <= Int.max / a else { throw RigNativeError.invalid("deformation \(label) overflows Int") }
        let ab = a * b
        guard ab == 0 || c <= Int.max / ab else { throw RigNativeError.invalid("deformation \(label) overflows Int") }
        return ab * c
    }

    private static func meshGrid(for layer: RigLayer, rig: RigDefinition) throws -> (nx: Int, ny: Int, vertexCount: Int) {
        guard layer.x.isFinite, layer.y.isFinite, layer.w.isFinite, layer.h.isFinite,
              layer.w > 0, layer.h > 0 else {
            throw RigNativeError.invalid("deformation layer \(layer.name) geometry must be finite with positive dimensions")
        }
        let scale = max(0.6, Double(rig.canvas.w) / 1280)
        let eye: RigEyeAnchor?
        if layer.side == "L" { eye = rig.anchors.eyeL }
        else if layer.side == "R" { eye = rig.anchors.eyeR }
        else { eye = nil }
        let detailedEye = eye?.blink != nil && layer.fade == "eyeOpen" && !layer.name.hasPrefix("irides")
        let detailedMouth = rig.anchors.mouth.morph != nil && layer.mouthExpression != nil
        let cell: Double
        if detailedMouth { cell = max(0.6, Double(rig.canvas.w) / 1280) }
        else if detailedEye { cell = 2 * scale }
        else if layer.headFollow != nil { cell = 12 * scale }
        else if layer.phys != nil { cell = 30 * max(0.6, Double(rig.canvas.w) / 768) }
        else { cell = 42 * max(0.6, Double(rig.canvas.w) / 768) }
        guard scale.isFinite, scale > 0, cell.isFinite, cell > 0 else {
            throw RigNativeError.invalid("deformation layer \(layer.name) mesh cell geometry is not finite and positive")
        }
        let rawNX = layer.w / cell
        let rawNY = layer.h / cell
        let roundedNX = rawNX.rounded(.toNearestOrAwayFromZero)
        let roundedNY = rawNY.rounded(.toNearestOrAwayFromZero)
        guard rawNX.isFinite, rawNY.isFinite, rawNX > 0, rawNY > 0,
              roundedNX.isFinite, roundedNY.isFinite, roundedNX >= 0, roundedNY >= 0,
              roundedNX < Double(Int.max), roundedNY < Double(Int.max) else {
            throw RigNativeError.invalid("deformation layer \(layer.name) mesh dimensions exceed Int bounds")
        }
        let nx = max(2, Int(roundedNX))
        let ny = max(2, Int(roundedNY))
        guard nx < Int.max, ny < Int.max else {
            throw RigNativeError.invalid("deformation layer \(layer.name) mesh dimensions exceed Int bounds")
        }
        let columns = nx + 1
        let rows = ny + 1
        guard columns <= Int.max / rows else {
            throw RigNativeError.invalid("deformation layer \(layer.name) mesh vertex count overflows Int")
        }
        let vertexCount = columns * rows
        guard vertexCount <= RigLimits.meshVertices else {
            throw RigNativeError.invalid(
                "deformation layer \(layer.name) has \(vertexCount) vertices; RigLimits.meshVertices allows at most \(RigLimits.meshVertices)"
            )
        }
        guard vertexCount <= Int(UInt16.max) else {
            throw RigNativeError.invalid("deformation layer \(layer.name) has \(vertexCount) vertices; UInt16 supports at most \(UInt16.max)")
        }
        return (nx, ny, vertexCount)
    }

    private static func validate(_ rig: RigDefinition) throws {
        guard rig.canvas.w > 0, rig.canvas.h > 0 else { throw RigNativeError.invalid("deformation canvas dimensions must be positive") }
        let anchors = rig.anchors
        let anchorValues = [
            anchors.face.x0, anchors.face.y0, anchors.face.x1, anchors.face.y1, anchors.face.cx, anchors.face.cy,
            anchors.mouth.x0, anchors.mouth.y0, anchors.mouth.x1, anchors.mouth.y1, anchors.mouth.cx, anchors.mouth.cy,
            anchors.neckPivot.cx, anchors.neckPivot.cy, anchors.bodyPivot.cx, anchors.bodyPivot.cy,
            anchors.neckTop, anchors.neckBottom, anchors.hairRootY, anchors.faceScale,
        ]
        guard anchorValues.allSatisfy({ $0.isFinite }) && anchors.faceScale.isFinite else {
            throw RigNativeError.invalid("deformation anchors contain non-finite geometry")
        }
        try validateEye(anchors.eyeL, label: "eyeL")
        try validateEye(anchors.eyeR, label: "eyeR")
        if let morph = anchors.mouth.morph { try validateMouth(morph, label: "mouth") }
        for layerIndex in rig.layers.indices {
            let layer = rig.layers[layerIndex]
            for previousIndex in 0..<layerIndex where rig.layers[previousIndex].name == layer.name {
                throw RigNativeError.invalid("deformation layers contain duplicate name \(layer.name)")
            }
            if let source = layer.deformationSource,
               !rig.layers.contains(where: { $0.name == source }) {
                throw RigNativeError.invalid("deformation layer \(layer.name) references unknown deformationSource \(source)")
            }
            if let source = layer.meshSource,
               !rig.layers.contains(where: { $0.name == source }) {
                throw RigNativeError.invalid("deformation layer \(layer.name) references unknown meshSource \(source)")
            }
            let values = [layer.x, layer.y, layer.w, layer.h, layer.z, layer.depth]
            guard values.allSatisfy({ $0.isFinite }) else { throw RigNativeError.invalid("deformation layer \(layer.name) contains non-finite geometry") }
            guard layer.w > 0, layer.h > 0 else { throw RigNativeError.invalid("deformation layer \(layer.name) dimensions must be positive") }
            if let attachment = layer.hairAttachment {
                guard attachment.rootY.isFinite, attachment.bodyY.isFinite else { throw RigNativeError.invalid("deformation layer \(layer.name) hair attachment is non-finite") }
            }
            if let follow = layer.headFollow {
                guard follow.center.cx.isFinite, follow.center.cy.isFinite, follow.radius.isFinite, follow.falloffRadius.isFinite,
                      follow.radius > 0, follow.falloffRadius > follow.radius else {
                    throw RigNativeError.invalid("deformation layer \(layer.name) headFollow is invalid")
                }
            }
            if let expression = layer.mouthExpression, expression != "neutral" && expression != "open" && expression != "smile" {
                throw RigNativeError.invalid("deformation layer \(layer.name) has unsupported mouth expression \(expression)")
            }
            if let strands = layer.strands {
                for strand in strands {
                    guard strand.x.isFinite, strand.rootY.isFinite, strand.tipY.isFinite else { throw RigNativeError.invalid("deformation layer \(layer.name) strand is non-finite") }
                }
            }
        }
        for start in rig.layers.indices {
            var current = start
            var hops = 0
            while let sourceName = rig.layers[current].deformationSource {
                guard let next = rig.layers.firstIndex(where: { $0.name == sourceName }) else { break }
                current = next
                hops += 1
                if hops > rig.layers.count {
                    throw RigNativeError.invalid("deformationSource cycle at layer \(rig.layers[start].name)")
                }
            }
        }
    }

    private static func validateEye(_ eye: RigEyeAnchor?, label: String) throws {
        guard let eye else { return }
        let values = [eye.x0, eye.y0, eye.x1, eye.y1, eye.icx, eye.icy, eye.closeY]
        guard values.allSatisfy({ $0.isFinite }) else { throw RigNativeError.invalid("deformation \(label) anchor is non-finite") }
        if let blink = eye.blink { try validateEyeProfile(blink, label: label) }
    }

    private static func validateEyeProfile(_ profile: RigEyeBlinkProfile, label: String) throws {
        let points = [profile.center, profile.closedSource, profile.closedTarget]
        guard profile.angleDeg.isFinite, profile.u0.isFinite, profile.u1.isFinite, profile.closedRotationDeg.isFinite,
              profile.u1 > profile.u0, points.allSatisfy({ $0.cx.isFinite && $0.cy.isFinite }),
              profile.upper.count >= 2, profile.upper.count <= 256,
              profile.lower.count == profile.upper.count, profile.closed.count == profile.upper.count else {
            throw RigNativeError.invalid("deformation \(label) eye blink profile is invalid")
        }
        for index in profile.upper.indices {
            guard profile.upper[index].isFinite, profile.lower[index].isFinite, profile.closed[index].isFinite,
                  profile.lower[index] > profile.upper[index] else {
                throw RigNativeError.invalid("deformation \(label) eye blink samples are invalid")
            }
        }
    }

    private static func validateMouth(_ profile: RigMouthMorphProfile, label: String) throws {
        guard profile.center.cx.isFinite, profile.center.cy.isFinite, profile.angleDeg.isFinite else {
            throw RigNativeError.invalid("deformation \(label) mouth morph profile is invalid")
        }
        try validateMouthShape(profile.neutral, label: "\(label) neutral", closed: true)
        try validateMouthShape(profile.open, label: "\(label) open", closed: false)
        try validateMouthShape(profile.smile, label: "\(label) smile", closed: true)
    }

    private static func validateMouthShape(_ shape: RigMouthShape, label: String, closed: Bool) throws {
        guard shape.u0.isFinite, shape.u1.isFinite, shape.u1 > shape.u0,
              shape.upper.count >= 2, shape.upper.count <= 129, shape.lower.count == shape.upper.count else {
            throw RigNativeError.invalid("deformation \(label) mouth shape is invalid")
        }
        for index in shape.upper.indices {
            let upper = shape.upper[index]
            let lower = shape.lower[index]
            guard upper.isFinite, lower.isFinite, (closed ? lower >= upper : lower > upper) else {
                throw RigNativeError.invalid("deformation \(label) mouth samples are invalid")
            }
        }
    }
}
