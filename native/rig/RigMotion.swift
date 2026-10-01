import Foundation
import simd

// Motion is deliberately compiled into bounded tracks once at preparation time.
// Display callbacks only walk these arrays; they never decode JSON or allocate a
// dictionary.  The host supplies semantic phase/effect ages and remains the
// source of truth for acceptance, priority, cooldown, and expiry.

enum RigMotionPhase: Int, CaseIterable, Equatable {
    case idle = 0
    case running = 1
    case waiting = 2
    case unknown = 3

    init(checked rawValue: Int) throws {
        guard let value = RigMotionPhase(rawValue: rawValue) else {
            throw RigNativeError.invalid("motion phase is outside the native four-phase set")
        }
        self = value
    }
}

enum RigMotionEffect: Int, CaseIterable, Equatable {
    case headTap = 0
    case bodyTap = 1
    case pet = 2
    case completionObserved = 3

    init(checked rawValue: Int) throws {
        guard let value = RigMotionEffect(rawValue: rawValue) else {
            throw RigNativeError.invalid("motion effect is outside the native four-reaction set")
        }
        self = value
    }
}

private struct RigMotionCodingKey: CodingKey {
    let stringValue: String
    let intValue: Int?

    init(_ string: String) {
        self.stringValue = string
        self.intValue = nil
    }

    init?(stringValue: String) {
        self.init(stringValue)
    }

    init?(intValue: Int) {
        self.stringValue = String(intValue)
        self.intValue = intValue
    }
}

private func rejectUnknownKeys<Key: CodingKey>(_ container: KeyedDecodingContainer<Key>, allowed: Set<String>) throws {
    for key in container.allKeys where !allowed.contains(key.stringValue) {
        throw RigNativeError.invalid("motion contains unknown field \(key.stringValue)")
    }
}

private enum RigMotionParameter: CaseIterable, Hashable {
    case angleX, angleY, angleZ
    case eyeOpenL, eyeOpenR, eyeX, eyeY, irisScale
    case brow, browAngL, browAngR, browAngSym
    case mouthOpen, mouthForm, mouthCY
    case body, armY, armPos
    case physAmp, soft, bust, bustY
    case bangL, bangC, bangR, fhAmp, fhSoft
    case eyeEase, mouthEase, eyeCY, eyeCAng, mouthCAng
    case eyeScaleL, eyeScaleR, mouthScale

    init(name: String) throws {
        switch name {
        case "angleX": self = .angleX
        case "angleY": self = .angleY
        case "angleZ": self = .angleZ
        case "eyeOpenL": self = .eyeOpenL
        case "eyeOpenR": self = .eyeOpenR
        case "eyeX": self = .eyeX
        case "eyeY": self = .eyeY
        case "irisScale": self = .irisScale
        case "brow": self = .brow
        case "browAngL": self = .browAngL
        case "browAngR": self = .browAngR
        case "browAngSym": self = .browAngSym
        case "mouthOpen": self = .mouthOpen
        case "mouthForm": self = .mouthForm
        case "mouthCY": self = .mouthCY
        case "body": self = .body
        case "armY": self = .armY
        case "armPos": self = .armPos
        case "physAmp": self = .physAmp
        case "soft": self = .soft
        case "bust": self = .bust
        case "bustY": self = .bustY
        case "bangL": self = .bangL
        case "bangC": self = .bangC
        case "bangR": self = .bangR
        case "fhAmp": self = .fhAmp
        case "fhSoft": self = .fhSoft
        case "eyeEase": self = .eyeEase
        case "mouthEase": self = .mouthEase
        case "eyeCY": self = .eyeCY
        case "eyeCAng": self = .eyeCAng
        case "mouthCAng": self = .mouthCAng
        case "eyeScaleL": self = .eyeScaleL
        case "eyeScaleR": self = .eyeScaleR
        case "mouthScale": self = .mouthScale
        default:
            throw RigNativeError.invalid("motion track references unknown parameter \(name)")
        }
    }

    var bounds: (Double, Double) {
        switch self {
        case .angleX, .angleY, .angleZ, .eyeX, .eyeY, .brow, .browAngL, .browAngR,
             .browAngSym, .mouthForm, .mouthCY, .body, .armY, .armPos, .bangL,
             .bangC, .bangR, .eyeCY, .eyeCAng, .mouthCAng:
            return (-1, 1)
        case .eyeOpenL, .eyeOpenR, .mouthOpen:
            return (0, 1)
        case .irisScale:
            return (0.5, 1.3)
        case .physAmp, .soft, .fhAmp:
            return (0, 3)
        case .bust:
            return (0, 4)
        case .bustY:
            return (-3, 3)
        case .fhSoft:
            return (0, 2)
        case .eyeEase, .mouthEase:
            return (0, 1)
        case .eyeScaleL, .eyeScaleR, .mouthScale:
            return (0.5, 1.5)
        }
    }

    func value(from parameters: RigParameters) -> Double {
        switch self {
        case .angleX: return parameters.angleX
        case .angleY: return parameters.angleY
        case .angleZ: return parameters.angleZ
        case .eyeOpenL: return parameters.eyeOpenL
        case .eyeOpenR: return parameters.eyeOpenR
        case .eyeX: return parameters.eyeX
        case .eyeY: return parameters.eyeY
        case .irisScale: return parameters.irisScale
        case .brow: return parameters.brow
        case .browAngL: return parameters.browAngL
        case .browAngR: return parameters.browAngR
        case .browAngSym: return parameters.browAngSym
        case .mouthOpen: return parameters.mouthOpen
        case .mouthForm: return parameters.mouthForm
        case .mouthCY: return parameters.mouthCY
        case .body: return parameters.body
        case .armY: return parameters.armY
        case .armPos: return parameters.armPos
        case .physAmp: return parameters.physAmp
        case .soft: return parameters.soft
        case .bust: return parameters.bust
        case .bustY: return parameters.bustY
        case .bangL: return parameters.bangL
        case .bangC: return parameters.bangC
        case .bangR: return parameters.bangR
        case .fhAmp: return parameters.fhAmp
        case .fhSoft: return parameters.fhSoft
        case .eyeEase: return parameters.eyeEase
        case .mouthEase: return parameters.mouthEase
        case .eyeCY: return parameters.eyeCY
        case .eyeCAng: return parameters.eyeCAng
        case .mouthCAng: return parameters.mouthCAng
        case .eyeScaleL: return parameters.eyeScaleL
        case .eyeScaleR: return parameters.eyeScaleR
        case .mouthScale: return parameters.mouthScale
        }
    }

    func set(_ value: Double, on parameters: inout RigParameters) {
        switch self {
        case .angleX: parameters.angleX = value
        case .angleY: parameters.angleY = value
        case .angleZ: parameters.angleZ = value
        case .eyeOpenL: parameters.eyeOpenL = value
        case .eyeOpenR: parameters.eyeOpenR = value
        case .eyeX: parameters.eyeX = value
        case .eyeY: parameters.eyeY = value
        case .irisScale: parameters.irisScale = value
        case .brow: parameters.brow = value
        case .browAngL: parameters.browAngL = value
        case .browAngR: parameters.browAngR = value
        case .browAngSym: parameters.browAngSym = value
        case .mouthOpen: parameters.mouthOpen = value
        case .mouthForm: parameters.mouthForm = value
        case .mouthCY: parameters.mouthCY = value
        case .body: parameters.body = value
        case .armY: parameters.armY = value
        case .armPos: parameters.armPos = value
        case .physAmp: parameters.physAmp = value
        case .soft: parameters.soft = value
        case .bust: parameters.bust = value
        case .bustY: parameters.bustY = value
        case .bangL: parameters.bangL = value
        case .bangC: parameters.bangC = value
        case .bangR: parameters.bangR = value
        case .fhAmp: parameters.fhAmp = value
        case .fhSoft: parameters.fhSoft = value
        case .eyeEase: parameters.eyeEase = value
        case .mouthEase: parameters.mouthEase = value
        case .eyeCY: parameters.eyeCY = value
        case .eyeCAng: parameters.eyeCAng = value
        case .mouthCAng: parameters.mouthCAng = value
        case .eyeScaleL: parameters.eyeScaleL = value
        case .eyeScaleR: parameters.eyeScaleR = value
        case .mouthScale: parameters.mouthScale = value
        }
    }
}

private struct RigMotionKey: Decodable {
    let atMS: UInt32
    let value: Double

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: RigMotionCodingKey.self)
        try rejectUnknownKeys(container, allowed: ["at_ms", "value"])
        guard let atMS = try container.decodeIfPresent(UInt32.self, forKey: RigMotionCodingKey("at_ms")),
              let value = try container.decodeIfPresent(Double.self, forKey: RigMotionCodingKey("value")) else {
            throw RigNativeError.invalid("motion key requires at_ms and value")
        }
        guard value.isFinite else { throw RigNativeError.invalid("motion key value is not finite") }
        self.atMS = atMS
        self.value = value
    }
}

private enum RigMotionInterpolation: String, Decodable {
    case linear
    case smoothstep
}

private enum RigMotionPlayback: String, Decodable {
    case loop
    case once
}

private struct RigDecodedMotionTrack: Decodable {
    let keys: [RigMotionKey]
    let interpolation: RigMotionInterpolation
    let isV2: Bool

    init(from decoder: Decoder) throws {
        if let keys = try? [RigMotionKey](from: decoder) {
            self.keys = keys
            self.interpolation = .linear
            self.isV2 = false
            return
        }
        let container = try decoder.container(keyedBy: RigMotionCodingKey.self)
        try rejectUnknownKeys(container, allowed: ["interpolation", "keys"])
        guard let interpolation = try container.decodeIfPresent(
            RigMotionInterpolation.self, forKey: RigMotionCodingKey("interpolation")
        ), let keys = try container.decodeIfPresent(
            [RigMotionKey].self, forKey: RigMotionCodingKey("keys")
        ) else {
            throw RigNativeError.invalid("motion v2 track requires interpolation and keys")
        }
        self.keys = keys
        self.interpolation = interpolation
        self.isV2 = true
    }

    func evaluate(at timeMS: Double) -> Double {
        if keys.count == 1 || timeMS <= Double(keys[0].atMS) { return keys[0].value }
        guard let last = keys.last else { return 0 }
        if timeMS >= Double(last.atMS) { return last.value }
        var low = 0
        var high = keys.count - 1
        while low + 1 < high {
            let middle = (low + high) / 2
            if Double(keys[middle].atMS) <= timeMS { low = middle } else { high = middle }
        }
        let left = keys[low]
        let right = keys[high]
        let span = Double(right.atMS - left.atMS)
        guard span > 0 else { return right.value }
        var amount = (timeMS - Double(left.atMS)) / span
        if interpolation == .smoothstep { amount = amount * amount * (3 - 2 * amount) }
        return left.value + (right.value - left.value) * amount
    }
}

private struct RigMotionTrack {
    let parameter: RigMotionParameter
    let track: RigDecodedMotionTrack
}

private struct RigMotionOrigin: Decodable {
    let x: Double
    let y: Double

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: RigMotionCodingKey.self)
        try rejectUnknownKeys(container, allowed: ["x", "y"])
        guard let x = try container.decodeIfPresent(Double.self, forKey: RigMotionCodingKey("x")),
              let y = try container.decodeIfPresent(Double.self, forKey: RigMotionCodingKey("y")),
              x.isFinite, y.isFinite, (0...1).contains(x), (0...1).contains(y) else {
            throw RigNativeError.invalid("motion layer origin must be finite normalized coordinates")
        }
        self.x = x
        self.y = y
    }
}

private struct RigMotionInfluence: Decodable {
    let axisX: Double
    let axisY: Double
    let start: Double
    let end: Double

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: RigMotionCodingKey.self)
        try rejectUnknownKeys(container, allowed: ["axisX", "axisY", "start", "end"])
        guard let axisX = try container.decodeIfPresent(Double.self, forKey: RigMotionCodingKey("axisX")),
              let axisY = try container.decodeIfPresent(Double.self, forKey: RigMotionCodingKey("axisY")),
              let start = try container.decodeIfPresent(Double.self, forKey: RigMotionCodingKey("start")),
              let end = try container.decodeIfPresent(Double.self, forKey: RigMotionCodingKey("end")),
              axisX.isFinite, axisY.isFinite, start.isFinite, end.isFinite,
              abs(axisX) <= 1, abs(axisY) <= 1, hypot(axisX, axisY) >= 1e-6,
              start >= -2, end <= 2, start < end else {
            throw RigNativeError.invalid("motion layer influence is invalid or out of range")
        }
        self.axisX = axisX
        self.axisY = axisY
        self.start = start
        self.end = end
    }
}
final class RigPreparedLayerMotion {
    let meshIndex: Int
    let origin: SIMD2<Double>
    let weights: [Float]

    init(meshIndex: Int, origin: SIMD2<Double>, weights: [Float]) {
        self.meshIndex = meshIndex
        self.origin = origin
        self.weights = weights
    }
}

private final class RigMotionLayer: Decodable {
    let origin: RigMotionOrigin
    let influence: RigMotionInfluence
    let translateX: RigDecodedMotionTrack?
    let translateY: RigDecodedMotionTrack?
    let rotationDeg: RigDecodedMotionTrack?
    let scale: RigDecodedMotionTrack?
    var prepared: RigPreparedLayerMotion?

    required init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: RigMotionCodingKey.self)
        try rejectUnknownKeys(container, allowed: [
            "origin", "influence", "translateX", "translateY", "rotationDeg", "scale",
        ])
        guard let origin = try container.decodeIfPresent(
            RigMotionOrigin.self, forKey: RigMotionCodingKey("origin")
        ), let influence = try container.decodeIfPresent(
            RigMotionInfluence.self, forKey: RigMotionCodingKey("influence")
        ) else {
            throw RigNativeError.invalid("motion layer requires origin and influence")
        }
        self.origin = origin
        self.influence = influence
        self.translateX = try container.decodeIfPresent(
            RigDecodedMotionTrack.self, forKey: RigMotionCodingKey("translateX")
        )
        self.translateY = try container.decodeIfPresent(
            RigDecodedMotionTrack.self, forKey: RigMotionCodingKey("translateY")
        )
        self.rotationDeg = try container.decodeIfPresent(
            RigDecodedMotionTrack.self, forKey: RigMotionCodingKey("rotationDeg")
        )
        self.scale = try container.decodeIfPresent(
            RigDecodedMotionTrack.self, forKey: RigMotionCodingKey("scale")
        )
        guard [translateX, translateY, rotationDeg, scale].contains(where: { $0 != nil }) else {
            throw RigNativeError.invalid("motion layer requires at least one transform track")
        }
        guard [translateX, translateY, rotationDeg, scale]
            .compactMap({ $0 }).allSatisfy(\.isV2) else {
            throw RigNativeError.invalid("motion layer tracks must use the v2 track shape")
        }
    }

    var tracks: [RigDecodedMotionTrack] {
        [translateX, translateY, rotationDeg, scale].compactMap { $0 }
    }
}

private struct RigMotionSpec: Decodable {
    let durationMS: UInt32
    let pose: String
    let transitionMS: UInt32
    let playback: RigMotionPlayback?
    let tracks: [RigMotionTrack]
    let layers: [(String, RigMotionLayer)]
    let usesV2Shape: Bool

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: RigMotionCodingKey.self)
        try rejectUnknownKeys(container, allowed: [
            "duration_ms", "pose", "transition_ms", "playback", "tracks", "layers",
        ])
        guard let durationMS = try container.decodeIfPresent(UInt32.self, forKey: RigMotionCodingKey("duration_ms")),
              let pose = try container.decodeIfPresent(String.self, forKey: RigMotionCodingKey("pose")),
              let transitionMS = try container.decodeIfPresent(UInt32.self, forKey: RigMotionCodingKey("transition_ms")) else {
            throw RigNativeError.invalid("motion requires duration_ms, pose, transition_ms, and tracks")
        }
        let tracksContainer: KeyedDecodingContainer<RigMotionCodingKey>
        do {
            tracksContainer = try container.nestedContainer(
                keyedBy: RigMotionCodingKey.self, forKey: RigMotionCodingKey("tracks")
            )
        } catch {
            throw RigNativeError.invalid("motion requires a tracks object")
        }
        guard (1...30_000).contains(durationMS) else {
            throw RigNativeError.invalid("motion duration must be 1..30000ms")
        }
        guard transitionMS <= min(durationMS, 2_000) else {
            throw RigNativeError.invalid("motion transition exceeds its duration or 2000ms")
        }
        guard !pose.isEmpty, pose.utf8.count <= 32,
              pose.utf8.allSatisfy({ (65...90).contains($0) || (97...122).contains($0)
                  || (48...57).contains($0) || $0 == 45 || $0 == 95 }) else {
            throw RigNativeError.invalid("motion pose reference is invalid")
        }
        let playback = try container.decodeIfPresent(
            RigMotionPlayback.self, forKey: RigMotionCodingKey("playback")
        )
        var tracks: [RigMotionTrack] = []
        tracks.reserveCapacity(tracksContainer.allKeys.count)
        for key in tracksContainer.allKeys.sorted(by: { $0.stringValue < $1.stringValue }) {
            tracks.append(RigMotionTrack(
                parameter: try RigMotionParameter(name: key.stringValue),
                track: try tracksContainer.decode(RigDecodedMotionTrack.self, forKey: key)
            ))
        }
        var layers: [(String, RigMotionLayer)] = []
        if container.contains(RigMotionCodingKey("layers")) {
            let layerContainer: KeyedDecodingContainer<RigMotionCodingKey>
            do {
                layerContainer = try container.nestedContainer(
                    keyedBy: RigMotionCodingKey.self, forKey: RigMotionCodingKey("layers")
                )
            } catch {
                throw RigNativeError.invalid("motion layers must be an object")
            }
            layers.reserveCapacity(layerContainer.allKeys.count)
            for key in layerContainer.allKeys.sorted(by: { $0.stringValue < $1.stringValue }) {
                guard !key.stringValue.isEmpty, key.stringValue.utf8.count <= 128 else {
                    throw RigNativeError.invalid("motion layer name is invalid")
                }
                layers.append((key.stringValue, try layerContainer.decode(RigMotionLayer.self, forKey: key)))
            }
        }
        let usesV2Shape = playback != nil || container.contains(RigMotionCodingKey("layers"))
            || tracks.contains(where: \.track.isV2)
        let allTracks = tracks.map(\.track) + layers.flatMap { $0.1.tracks }
        guard allTracks.count <= 40 else { throw RigNativeError.invalid("motion has too many tracks") }
        var keyCount = 0
        for track in allTracks {
            guard !track.keys.isEmpty, track.keys.count <= 32 else {
                throw RigNativeError.invalid("motion track must contain 1..32 keys")
            }
            guard track.keys[0].atMS == 0 else {
                throw RigNativeError.invalid("motion track must start at 0ms")
            }
            var previous: UInt32 = 0
            for (index, key) in track.keys.enumerated() {
                guard key.atMS <= durationMS,
                      index == 0 ? key.atMS == 0 : key.atMS > previous else {
                    throw RigNativeError.invalid("motion track key times must be strictly increasing")
                }
                previous = key.atMS
            }
            keyCount += track.keys.count
        }
        guard keyCount <= 256 else { throw RigNativeError.invalid("motion contains more than 256 keys") }
        for track in tracks {
            let bounds = track.parameter.bounds
            guard track.track.keys.allSatisfy({ $0.value >= bounds.0 && $0.value <= bounds.1 }) else {
                throw RigNativeError.invalid("motion parameter value would be clamped by native apply")
            }
        }
        for (_, layer) in layers {
            guard layer.rotationDeg?.keys.allSatisfy({ abs($0.value) <= 360 }) ?? true,
                  layer.scale?.keys.allSatisfy({ (0...4).contains($0.value) }) ?? true else {
                throw RigNativeError.invalid("motion layer rotation or scale is out of range")
            }
        }
        self.durationMS = durationMS
        self.pose = pose
        self.transitionMS = transitionMS
        self.playback = playback
        self.tracks = tracks
        self.layers = layers
        self.usesV2Shape = usesV2Shape
    }

    func sampleTime(ageSeconds: Double, reaction: Bool, version: Int) -> Double {
        let elapsedMS = max(0, ageSeconds * 1_000)
        // Preserve legacy millisecond quantization, but sample v2 source motion
        // continuously at the host's real frame age.
        let rawMS = version == 1 ? floor(elapsedMS) : elapsedMS
        if version == 1, reaction {
            return min(Double(durationMS), rawMS)
        }
        if version == 2, playback == .once {
            return min(Double(durationMS), rawMS)
        }
        return rawMS.truncatingRemainder(dividingBy: Double(durationMS))
    }

    func apply(to parameters: inout RigParameters, at timeMS: Double) {
        for track in tracks { track.parameter.set(track.track.evaluate(at: timeMS), on: &parameters) }
    }

    func overlay(on parameters: inout RigParameters, at timeMS: Double, weight: Double) {
        guard weight > 0 else { return }
        for track in tracks {
            let next = track.track.evaluate(at: timeMS)
            let current = track.parameter.value(from: parameters)
            track.parameter.set(current + (next - current) * weight, on: &parameters)
        }
    }
}

private struct RigMotionEnvelope: Decodable {
    let version: Int
    let phases: [String: RigMotionSpec]
    let reactions: [String: RigMotionSpec]

    init(from decoder: Decoder) throws {
        let root = try decoder.container(keyedBy: RigMotionCodingKey.self)
        try rejectUnknownKeys(root, allowed: ["version", "phases", "reactions"])
        guard let version = try root.decodeIfPresent(Int.self, forKey: RigMotionCodingKey("version")) else {
            throw RigNativeError.invalid("motion version is missing")
        }
        let phaseContainer: KeyedDecodingContainer<RigMotionCodingKey>
        let reactionContainer: KeyedDecodingContainer<RigMotionCodingKey>
        do {
            phaseContainer = try root.nestedContainer(
                keyedBy: RigMotionCodingKey.self, forKey: RigMotionCodingKey("phases")
            )
            reactionContainer = try root.nestedContainer(
                keyedBy: RigMotionCodingKey.self, forKey: RigMotionCodingKey("reactions")
            )
        } catch {
            throw RigNativeError.invalid("motion phases and reactions are required")
        }
        var phases: [String: RigMotionSpec] = [:]
        phases.reserveCapacity(phaseContainer.allKeys.count)
        for key in phaseContainer.allKeys {
            phases[key.stringValue] = try phaseContainer.decode(RigMotionSpec.self, forKey: key)
        }
        var reactions: [String: RigMotionSpec] = [:]
        reactions.reserveCapacity(reactionContainer.allKeys.count)
        for key in reactionContainer.allKeys {
            reactions[key.stringValue] = try reactionContainer.decode(RigMotionSpec.self, forKey: key)
        }
        self.version = version
        self.phases = phases
        self.reactions = reactions
    }
}

private struct RigMotionDocument {
    let version: Int
    let phases: [RigMotionSpec]
    let reactions: [RigMotionSpec]

    init(data: Data, basePoseID: String, alternatePoseID: String?) throws {
        let root = try JSONDecoder().decode(RigMotionEnvelope.self, from: data)
        guard root.version == 1 || root.version == 2 else {
            throw RigNativeError.invalid("motion version must be 1 or 2")
        }
        let phaseNames = ["idle", "running", "waiting", "unknown"]
        let reactionNames = ["head_tap", "body_tap", "pet", "completion_observed"]
        guard root.phases.count == phaseNames.count, root.reactions.count == reactionNames.count else {
            throw RigNativeError.invalid("motion must contain all four phases and reactions")
        }
        func ordered(_ names: [String], _ source: [String: RigMotionSpec]) throws -> [RigMotionSpec] {
            var values: [RigMotionSpec] = []
            values.reserveCapacity(names.count)
            for name in names {
                guard let value = source[name] else {
                    throw RigNativeError.invalid("motion is missing \(name)")
                }
                guard value.pose == basePoseID || value.pose == alternatePoseID else {
                    throw RigNativeError.invalid("motion pose \(value.pose) does not reference a declared rig pose")
                }
                if root.version == 1 {
                    guard !value.usesV2Shape, value.layers.isEmpty, value.playback == nil else {
                        throw RigNativeError.invalid("motion v1 contains v2 fields")
                    }
                } else {
                    guard value.usesV2Shape, value.playback != nil,
                          value.tracks.allSatisfy(\.track.isV2) else {
                        throw RigNativeError.invalid("motion v2 requires playback and v2 tracks")
                    }
                }
                values.append(value)
            }
            for key in source.keys where !names.contains(key) {
                throw RigNativeError.invalid("motion contains unknown phase or reaction \(key)")
            }
            return values
        }
        self.version = root.version
        self.phases = try ordered(phaseNames, root.phases)
        self.reactions = try ordered(reactionNames, root.reactions)
        let allSpecs = phases + reactions
        let totalKeys = allSpecs.reduce(0) { count, spec in
            count + spec.tracks.reduce(0) { $0 + $1.track.keys.count }
                + spec.layers.reduce(0) { $0 + $1.1.tracks.reduce(0) { $0 + $1.keys.count } }
        }
        guard totalKeys <= 1_024 else { throw RigNativeError.invalid("motion exceeds the total key quota") }
    }
}

struct RigLocalTransform {
    var binding: RigPreparedLayerMotion?
    var translateX = 0.0
    var translateY = 0.0
    var rotationDeg = 0.0
    var scale = 1.0

    fileprivate mutating func reset() {
        binding = nil
        translateX = 0
        translateY = 0
        rotationDeg = 0
        scale = 1
    }
}
struct RigLocalMotionFrame {
    var base: [RigLocalTransform] = []
    var pose: [RigLocalTransform] = []
}

struct RigEvaluatedMotion {
    let parameters: RigParameters
    let timeMS: Double
    let poseMix: Double
    let local: RigLocalMotionFrame
}

final class RigMotionEvaluator {
    private let document: RigMotionDocument
    private let envelopeMagnitudes: [RigMotionParameter: Double]
    private let basePoseID: String
    private let alternatePoseID: String?
    private var lastNow: Double?
    private var lastPhase: RigMotionPhase?
    private var lastPhaseAge: Double = 0
    private var lastEffect: Int32 = -1
    private var lastEffectAge: Double = 0
    private var previousParameters: RigParameters?
    private var previousPoseMix: Double = 0
    private var phaseTransitionStart: Double?
    private var phaseTransitionFrom: RigParameters?
    private var phaseTransitionFromPose: Double = 0
    private var local = RigLocalMotionFrame()
    private var transitionBase: [RigLocalTransform] = []
    private var transitionPose: [RigLocalTransform] = []
    private var prepared = false

    init(data: Data, basePoseID: String, alternatePoseID: String?) throws {
        self.basePoseID = basePoseID
        self.alternatePoseID = alternatePoseID
        self.document = try RigMotionDocument(
            data: data, basePoseID: basePoseID, alternatePoseID: alternatePoseID
        )
        var magnitudes: [RigMotionParameter: Double] = [:]
        let defaults = RigParameters()
        for parameter in RigMotionParameter.allCases {
            magnitudes[parameter] = document.phases.contains(where: {
                !$0.tracks.contains(where: { $0.parameter == parameter })
            }) ? abs(parameter.value(from: defaults)) : 0
        }
        func include(_ specs: [RigMotionSpec]) {
            for spec in specs {
                for track in spec.tracks {
                    for key in track.track.keys {
                        magnitudes[track.parameter] = max(
                            magnitudes[track.parameter] ?? 0, abs(key.value))
                    }
                }
            }
        }
        include(document.phases)
        include(document.reactions)
        envelopeMagnitudes = magnitudes
    }

    func prepare(baseMeshes: [RigMesh], poseMeshes: [RigMesh]?, canvas: RigCanvas) throws {
        guard !prepared else { return }
        local.base = Array(repeating: RigLocalTransform(), count: baseMeshes.count)
        local.pose = Array(repeating: RigLocalTransform(), count: poseMeshes?.count ?? 0)
        transitionBase = Array(repeating: RigLocalTransform(), count: baseMeshes.count)
        transitionPose = Array(repeating: RigLocalTransform(), count: poseMeshes?.count ?? 0)
        for spec in document.phases + document.reactions {
            let targetsPose = alternatePoseID != nil && spec.pose == alternatePoseID
            let meshes = targetsPose ? poseMeshes : baseMeshes
            guard let meshes else {
                throw RigNativeError.invalid("motion references a pose model that was not decoded")
            }
            var names: [String: Int] = [:]
            names.reserveCapacity(meshes.count)
            for (index, mesh) in meshes.enumerated() {
                guard names.updateValue(index, forKey: mesh.layer.name) == nil else {
                    throw RigNativeError.invalid("motion model contains duplicate layer \(mesh.layer.name)")
                }
            }
            for (name, layer) in spec.layers {
                guard let index = names[name] else {
                    throw RigNativeError.invalid("motion references unknown layer \(name) in pose \(spec.pose)")
                }
                let mesh = meshes[index]
                let axisLength = hypot(layer.influence.axisX, layer.influence.axisY)
                var weights: [Float] = []
                weights.reserveCapacity(mesh.uvs.count)
                for uv in mesh.uvs {
                    let projection = ((Double(uv.x) - layer.origin.x) * layer.influence.axisX
                        + (Double(uv.y) - layer.origin.y) * layer.influence.axisY) / axisLength
                    let raw = (projection - layer.influence.start)
                        / (layer.influence.end - layer.influence.start)
                    let t = min(1, max(0, raw))
                    weights.append(Float(t * t * (3 - 2 * t)))
                }
                let translationLimit = Double(max(canvas.w, canvas.h))
                guard layer.translateX?.keys.allSatisfy({ abs($0.value) <= translationLimit }) ?? true,
                      layer.translateY?.keys.allSatisfy({ abs($0.value) <= translationLimit }) ?? true else {
                    throw RigNativeError.invalid("motion layer translation exceeds the native canvas")
                }
                layer.prepared = RigPreparedLayerMotion(
                    meshIndex: index,
                    origin: SIMD2<Double>(layer.origin.x, layer.origin.y),
                    weights: weights
                )
            }
        }
        prepared = true
    }

    // Tracks interpolate inside their authored key extrema; transitions and
    // reactions blend convexly. Native defaults are reachable only when some
    // phase omits the parameter, not simply because a parameter has a default.
    func parameterMagnitude(_ name: String) -> Double {
        guard let parameter = try? RigMotionParameter(name: name) else { return 0 }
        return envelopeMagnitudes[parameter] ?? 0
    }

    // Local transforms also interpolate only between keys and identity.
    func layerEnvelope(pose: Bool, meshIndex: Int) -> (translation: Double, linear: Double) {
        var translation = 0.0
        var linear = 0.0
        func include(_ specs: [RigMotionSpec]) {
            for spec in specs {
                guard (alternatePoseID != nil && spec.pose == alternatePoseID) == pose else { continue }
                for (_, layer) in spec.layers where layer.prepared?.meshIndex == meshIndex {
                    let dx = layer.translateX?.keys.reduce(0.0) { max($0, abs($1.value)) } ?? 0
                    let dy = layer.translateY?.keys.reduce(0.0) { max($0, abs($1.value)) } ?? 0
                    translation = max(translation, hypot(dx, dy))
                    let degrees = layer.rotationDeg?.keys.reduce(0.0) { max($0, abs($1.value)) } ?? 0
                    let cosine = cos(min(180, degrees) * Double.pi / 180)
                    // |s R(theta) - I|, including transition to identity.
                    let factor = layer.scale?.keys.reduce(0.0) { result, key in
                        let scale = key.value
                        return max(result, sqrt(max(0, scale * scale + 1 - 2 * scale * cosine)))
                    } ?? sqrt(max(0, 2 - 2 * cosine))
                    linear = max(linear, factor)
                }
            }
        }
        include(document.phases)
        include(document.reactions)
        return (translation, linear)
    }

    func evaluate(_ intent: HerdrRigIntent) throws -> RigEvaluatedMotion {
        guard prepared || document.version == 1 else {
            throw RigNativeError.invalid("motion v2 was evaluated before layer preparation")
        }
        guard intent.now_seconds.isFinite, intent.phase_age_seconds.isFinite,
              intent.effect_age_seconds.isFinite, intent.effect_duration_seconds.isFinite else {
            throw RigNativeError.invalid("motion intent contains a nonfinite clock")
        }
        guard intent.now_seconds >= 0, intent.phase_age_seconds >= 0, intent.effect_age_seconds >= 0,
              intent.effect_duration_seconds >= 0 else {
            throw RigNativeError.invalid("motion intent clock is negative")
        }
        guard intent.visible <= 1, intent.frozen <= 1 else {
            throw RigNativeError.invalid("motion intent flags are not boolean")
        }
        let phase = try RigMotionPhase(checked: Int(intent.phase))
        let effect: RigMotionEffect? = intent.effect == -1 ? nil : try RigMotionEffect(checked: Int(intent.effect))
        if let lastNow, intent.now_seconds < lastNow { throw RigNativeError.invalid("motion clock moved backwards") }
        if phase == lastPhase, intent.phase_age_seconds < lastPhaseAge {
            throw RigNativeError.invalid("phase age moved backwards")
        }
        if intent.effect == lastEffect, intent.effect_age_seconds < lastEffectAge {
            throw RigNativeError.invalid("effect age moved backwards")
        }
        let phaseSpec = document.phases[phase.rawValue]
        if phase != lastPhase {
            phaseTransitionStart = intent.now_seconds
            phaseTransitionFrom = previousParameters
            phaseTransitionFromPose = previousPoseMix
            copy(local.base, into: &transitionBase)
            copy(local.pose, into: &transitionPose)
        }
        reset(&local.base)
        reset(&local.pose)
        let phaseTime = phaseSpec.sampleTime(
            ageSeconds: intent.phase_age_seconds, reaction: false, version: document.version
        )
        var parameters = RigParameters()
        phaseSpec.apply(to: &parameters, at: phaseTime)
        sampleLayers(phaseSpec, at: phaseTime)
        var poseMix = poseValue(for: phaseSpec.pose)

        if let from = phaseTransitionFrom, let started = phaseTransitionStart, phaseSpec.transitionMS > 0 {
            let elapsed = max(0, intent.now_seconds - started) * 1_000
            let amount = min(1, elapsed / Double(phaseSpec.transitionMS))
            parameters = blend(from, parameters, amount)
            blend(transitionBase, into: &local.base, amount: amount)
            blend(transitionPose, into: &local.pose, amount: amount)
            poseMix = phaseTransitionFromPose + (poseMix - phaseTransitionFromPose) * amount
            if amount >= 1 { phaseTransitionFrom = nil; phaseTransitionStart = nil }
        } else if phaseSpec.transitionMS == 0 {
            phaseTransitionFrom = nil
            phaseTransitionStart = nil
        }

        if let effect, intent.effect_age_seconds < intent.effect_duration_seconds {
            let reactionSpec = document.reactions[effect.rawValue]
            let reactionTime = reactionSpec.sampleTime(
                ageSeconds: intent.effect_age_seconds, reaction: true, version: document.version
            )
            let amount = reactionSpec.transitionMS == 0 ? 1
                : min(1, intent.effect_age_seconds * 1_000 / Double(reactionSpec.transitionMS))
            reactionSpec.overlay(on: &parameters, at: reactionTime, weight: amount)
            overlayLayers(reactionSpec, at: reactionTime, weight: amount)
            let reactionPose = poseValue(for: reactionSpec.pose)
            poseMix += (reactionPose - poseMix) * amount
        }

        lastNow = intent.now_seconds
        lastPhase = phase
        lastPhaseAge = intent.phase_age_seconds
        lastEffect = intent.effect
        lastEffectAge = intent.effect_age_seconds
        previousParameters = parameters
        previousPoseMix = poseMix
        return RigEvaluatedMotion(
            parameters: parameters, timeMS: intent.now_seconds * 1_000, poseMix: poseMix, local: local
        )
    }

    func restartEffectClock() {
        lastEffect = -1
        lastEffectAge = 0
    }

    func restartModelClock() {
        lastNow = nil
        lastPhase = nil
        lastPhaseAge = 0
        lastEffect = -1
        lastEffectAge = 0
        previousParameters = nil
        previousPoseMix = 0
        phaseTransitionStart = nil
        phaseTransitionFrom = nil
        phaseTransitionFromPose = 0
        reset(&local.base)
        reset(&local.pose)
    }

    func resynchronize() {
        lastNow = nil
        lastPhaseAge = 0
        lastEffectAge = 0
        phaseTransitionStart = nil
        phaseTransitionFrom = nil
    }

    private func sampleLayers(_ spec: RigMotionSpec, at timeMS: Double) {
        for (_, layer) in spec.layers {
            guard let binding = layer.prepared else { continue }
            var transform = RigLocalTransform()
            transform.binding = binding
            transform.translateX = layer.translateX?.evaluate(at: timeMS) ?? 0
            transform.translateY = layer.translateY?.evaluate(at: timeMS) ?? 0
            transform.rotationDeg = layer.rotationDeg?.evaluate(at: timeMS) ?? 0
            transform.scale = layer.scale?.evaluate(at: timeMS) ?? 1
            if spec.pose == alternatePoseID { local.pose[binding.meshIndex] = transform }
            else { local.base[binding.meshIndex] = transform }
        }
    }

    private func overlayLayers(_ spec: RigMotionSpec, at timeMS: Double, weight: Double) {
        for (_, layer) in spec.layers {
            guard let binding = layer.prepared else { continue }
            if spec.pose == alternatePoseID {
                overlay(layer, binding: binding, at: timeMS, weight: weight, into: &local.pose[binding.meshIndex])
            } else {
                overlay(layer, binding: binding, at: timeMS, weight: weight, into: &local.base[binding.meshIndex])
            }
        }
    }

    private func overlay(
        _ layer: RigMotionLayer, binding: RigPreparedLayerMotion, at timeMS: Double,
        weight: Double, into transform: inout RigLocalTransform
    ) {
        transform.binding = binding
        transform.translateX += ((layer.translateX?.evaluate(at: timeMS) ?? 0) - transform.translateX) * weight
        transform.translateY += ((layer.translateY?.evaluate(at: timeMS) ?? 0) - transform.translateY) * weight
        transform.rotationDeg += ((layer.rotationDeg?.evaluate(at: timeMS) ?? 0) - transform.rotationDeg) * weight
        transform.scale += ((layer.scale?.evaluate(at: timeMS) ?? 1) - transform.scale) * weight
    }

    private func reset(_ values: inout [RigLocalTransform]) {
        for index in values.indices { values[index].reset() }
    }

    private func copy(_ source: [RigLocalTransform], into target: inout [RigLocalTransform]) {
        for index in source.indices { target[index] = source[index] }
    }

    private func blend(
        _ from: [RigLocalTransform], into target: inout [RigLocalTransform], amount: Double
    ) {
        let t = min(1, max(0, amount))
        for index in target.indices {
            let old = from[index]
            var next = target[index]
            next.binding = next.binding ?? old.binding
            next.translateX = old.translateX + (next.translateX - old.translateX) * t
            next.translateY = old.translateY + (next.translateY - old.translateY) * t
            next.rotationDeg = old.rotationDeg + (next.rotationDeg - old.rotationDeg) * t
            next.scale = old.scale + (next.scale - old.scale) * t
            target[index] = next
        }
    }

    private func poseValue(for pose: String) -> Double {
        guard alternatePoseID != nil else { return 0 }
        return pose == alternatePoseID ? 1 : 0
    }

    private func blend(_ from: RigParameters, _ to: RigParameters, _ amount: Double) -> RigParameters {
        let t = min(1, max(0, amount))
        func mix(_ a: Double, _ b: Double) -> Double { a + (b - a) * t }
        var result = to
        result.angleX = mix(from.angleX, to.angleX); result.angleY = mix(from.angleY, to.angleY); result.angleZ = mix(from.angleZ, to.angleZ)
        result.eyeOpenL = mix(from.eyeOpenL, to.eyeOpenL); result.eyeOpenR = mix(from.eyeOpenR, to.eyeOpenR)
        result.eyeX = mix(from.eyeX, to.eyeX); result.eyeY = mix(from.eyeY, to.eyeY); result.irisScale = mix(from.irisScale, to.irisScale)
        result.brow = mix(from.brow, to.brow); result.browAngL = mix(from.browAngL, to.browAngL); result.browAngR = mix(from.browAngR, to.browAngR); result.browAngSym = mix(from.browAngSym, to.browAngSym)
        result.mouthOpen = mix(from.mouthOpen, to.mouthOpen); result.mouthForm = mix(from.mouthForm, to.mouthForm); result.mouthCY = mix(from.mouthCY, to.mouthCY)
        result.body = mix(from.body, to.body); result.armY = mix(from.armY, to.armY); result.armPos = mix(from.armPos, to.armPos)
        result.physAmp = mix(from.physAmp, to.physAmp); result.soft = mix(from.soft, to.soft); result.bust = mix(from.bust, to.bust); result.bustY = mix(from.bustY, to.bustY)
        result.bangL = mix(from.bangL, to.bangL); result.bangC = mix(from.bangC, to.bangC); result.bangR = mix(from.bangR, to.bangR); result.fhAmp = mix(from.fhAmp, to.fhAmp); result.fhSoft = mix(from.fhSoft, to.fhSoft)
        result.eyeEase = mix(from.eyeEase, to.eyeEase); result.mouthEase = mix(from.mouthEase, to.mouthEase); result.eyeCY = mix(from.eyeCY, to.eyeCY); result.eyeCAng = mix(from.eyeCAng, to.eyeCAng); result.mouthCAng = mix(from.mouthCAng, to.mouthCAng)
        result.eyeScaleL = mix(from.eyeScaleL, to.eyeScaleL); result.eyeScaleR = mix(from.eyeScaleR, to.eyeScaleR); result.mouthScale = mix(from.mouthScale, to.mouthScale)
        return result
    }
}
