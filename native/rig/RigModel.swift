// Native representation of the pinned Anime2.5DRig/DAEMONLET model.
// Upstream MIT notices are preserved under web/rig/vendor/.
import Foundation
import simd

struct RigPoint: Codable { let cx: Double; let cy: Double }
struct RigBounds: Codable { let x0: Double; let y0: Double; let x1: Double; let y1: Double }
struct RigFaceBounds: Codable {
    let x0: Double; let y0: Double; let x1: Double; let y1: Double
    let cx: Double; let cy: Double
}
struct RigEyeBlinkProfile: Codable {
    let center: RigPoint
    let angleDeg: Double
    let u0: Double; let u1: Double
    let upper: [Double]; let lower: [Double]; let closed: [Double]
    let closedSource: RigPoint; let closedTarget: RigPoint
    let closedRotationDeg: Double
}
struct RigEyeAnchor: Codable {
    let x0: Double; let y0: Double; let x1: Double; let y1: Double
    let icx: Double; let icy: Double; let closeY: Double
    let blink: RigEyeBlinkProfile?
}
struct RigMouthShape: Codable {
    let u0: Double; let u1: Double
    let upper: [Double]; let lower: [Double]
}
struct RigMouthMorphProfile: Codable {
    let center: RigPoint; let angleDeg: Double
    let neutral: RigMouthShape; let open: RigMouthShape; let smile: RigMouthShape
}
struct RigMouthAnchor: Codable {
    let x0: Double; let y0: Double; let x1: Double; let y1: Double
    let cx: Double; let cy: Double
    let morph: RigMouthMorphProfile?
}
struct RigAnchors: Codable {
    let face: RigFaceBounds
    let eyeL: RigEyeAnchor?; let eyeR: RigEyeAnchor?
    let mouth: RigMouthAnchor
    let neckPivot: RigPoint; let bodyPivot: RigPoint
    let neckTop: Double; let neckBottom: Double
    let hairRootY: Double; let faceScale: Double
}
struct RigHairStrand: Codable { let x: Double; let rootY: Double; let tipY: Double }
struct RigHairAttachment: Codable { let rootY: Double; let bodyY: Double }
struct RigHeadFollow: Codable { let center: RigPoint; let radius: Double; let falloffRadius: Double }

// Offsets refer only to the bounded worker response's shared straight-RGBA blob.
// This is internal IPC, not an author-supplied mesh/shader or character pack format.
struct RigImage: Codable { let width: Int; let height: Int; let offset: Int; let length: Int }
struct RigLayer: Codable {
    let name: String
    let x: Double; let y: Double; let w: Double; let h: Double
    let z: Double; let depth: Double
    let group: String; let phys: String?; let fade: String?
    let mouthExpression: String?
    let deformationSource: String?; let meshSource: String?
    let hairAttachment: RigHairAttachment?; let headFollow: RigHeadFollow?
    let side: String?; let strands: [RigHairStrand]?
    let synthetic: Bool?
    let img: RigImage
}
struct RigCanvas: Codable { let w: Int; let h: Int }
struct RigSynthesis: Codable { let eye: Bool; let mouth: Bool }
struct RigDefinition: Codable {
    let canvas: RigCanvas
    let layers: [RigLayer]
    let anchors: RigAnchors
    let interactionAreas: [String: RigBounds]?
    let warnings: [String]
    let synth: RigSynthesis
}
struct RigDecodeDiagnostics: Codable {
    let baseMissingRequired: [String]
    let poseMissingRequired: [String]
    let basePSDLayerCount: Int
    let posePSDLayerCount: Int
    let warnings: [String]
}
struct RigHairPhysicsTuning: Codable {
    let amplitude: Double
    let stiffness: Double
    let damping: Double
    let wind: Double
    let inertia: Double
    let rootLock: Double
    let maxOffset: Double
}
struct RigHairPhysicsConfig: Codable {
    let frontHair: RigHairPhysicsTuning
    let backHair: RigHairPhysicsTuning
    let layers: [String: [String: Double]]
}
struct RigDecodedDocument: Codable {
    let format: String
    let version: Int
    let base: RigDefinition
    let pose: RigDefinition?
    let physics: RigHairPhysicsConfig?
    let diagnostics: RigDecodeDiagnostics
}
struct RigDecodedScene {
    let document: RigDecodedDocument
    let rgba: Data
}

struct RigParameters: Codable {
    var angleX = 0.0; var angleY = 0.0; var angleZ = 0.0
    var eyeOpenL = 1.0; var eyeOpenR = 1.0; var eyeX = 0.0; var eyeY = 0.0
    var irisScale = 1.0
    var brow = 0.0; var browAngL = 0.0; var browAngR = 0.0; var browAngSym = 0.0
    var mouthOpen = 0.0; var mouthForm = 0.0; var mouthCY = 0.0
    var body = 0.0; var armY = 0.0; var armPos = 0.0
    var physAmp = 2.0; var soft = 2.0; var bust = 2.5; var bustY = 1.0
    var bangL = 0.0; var bangC = 0.0; var bangR = 0.0; var fhAmp = 2.0; var fhSoft = 0.4
    var eyeEase = 0.3; var mouthEase = 0.45; var eyeCY = 0.0; var eyeCAng = 0.0; var mouthCAng = 0.0
    var eyeScaleL = 1.0; var eyeScaleR = 1.0; var mouthScale = 1.0
}

struct RigFrame {
    let width: Int
    let height: Int
    // Premultiplied RGBA, top-left origin, tightly packed rows.
    let rgba: Data
}

enum RigNativeError: Error, CustomStringConvertible {
    case invalid(String)
    case unavailable(String)
    case cancelled
    case timeout
    var description: String {
        switch self {
        case .invalid(let detail): return "RIG_INVALID: \(detail)"
        case .unavailable(let detail): return "RIG_UNAVAILABLE: \(detail)"
        case .cancelled: return "RIG_CANCELLED"
        case .timeout: return "RIG_TIMEOUT"
        }
    }
}

extension RigParameters {
    mutating func apply(_ values: [String: Double]) throws {
        var next = self
        for (name, value) in values {
            guard value.isFinite else { throw RigNativeError.invalid("nonfinite parameter \(name)") }
            func bounded(_ lower: Double, _ upper: Double) -> Double { min(upper, max(lower, value)) }
            switch name {
            case "angleX": next.angleX = bounded(-1, 1)
            case "angleY": next.angleY = bounded(-1, 1)
            case "angleZ": next.angleZ = bounded(-1, 1)
            case "eyeOpenL": next.eyeOpenL = bounded(0, 1)
            case "eyeOpenR": next.eyeOpenR = bounded(0, 1)
            case "eyeX": next.eyeX = bounded(-1, 1)
            case "eyeY": next.eyeY = bounded(-1, 1)
            case "irisScale": next.irisScale = bounded(0.5, 1.3)
            case "brow": next.brow = bounded(-1, 1)
            case "browAngL": next.browAngL = bounded(-1, 1)
            case "browAngR": next.browAngR = bounded(-1, 1)
            case "browAngSym": next.browAngSym = bounded(-1, 1)
            case "mouthOpen": next.mouthOpen = bounded(0, 1)
            case "mouthForm": next.mouthForm = bounded(-1, 1)
            case "mouthCY": next.mouthCY = bounded(-1, 1)
            case "body": next.body = bounded(-1, 1)
            case "armY": next.armY = bounded(-1, 1)
            case "armPos": next.armPos = bounded(-1, 1)
            case "physAmp": next.physAmp = bounded(0, 3)
            case "soft": next.soft = bounded(0, 3)
            case "bust": next.bust = bounded(0, 4)
            case "bustY": next.bustY = bounded(-3, 3)
            case "bangL": next.bangL = bounded(-1, 1)
            case "bangC": next.bangC = bounded(-1, 1)
            case "bangR": next.bangR = bounded(-1, 1)
            case "fhAmp": next.fhAmp = bounded(0, 3)
            case "fhSoft": next.fhSoft = bounded(0, 2)
            case "eyeEase": next.eyeEase = bounded(0, 1)
            case "mouthEase": next.mouthEase = bounded(0, 1)
            case "eyeCY": next.eyeCY = bounded(-1, 1)
            case "eyeCAng": next.eyeCAng = bounded(-1, 1)
            case "mouthCAng": next.mouthCAng = bounded(-1, 1)
            case "eyeScaleL": next.eyeScaleL = bounded(0.5, 1.5)
            case "eyeScaleR": next.eyeScaleR = bounded(0.5, 1.5)
            case "mouthScale": next.mouthScale = bounded(0.5, 1.5)
            default: throw RigNativeError.invalid("unknown parameter \(name)")
            }
        }
        self = next
    }
}
