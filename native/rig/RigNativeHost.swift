import AppKit
import CoreGraphics
import Foundation
import ImageIO
import Metal
import QuartzCore

final class RigSurfaceView: NSView {
    override func makeBackingLayer() -> CALayer { CAMetalLayer() }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        configureLayer()
        isHidden = true
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        wantsLayer = true
        configureLayer()
        isHidden = true
    }

    override func hitTest(_ point: NSPoint) -> NSView? {
        // Rust PetView owns mouse capture and click-through policy.  The rig
        // surface must never become a competing AppKit event target.
        nil
    }

    override var acceptsFirstResponder: Bool { false }

    var metalLayer: CAMetalLayer {
        layer as! CAMetalLayer
    }

    private func configureLayer() {
        let metal = metalLayer
        metal.isOpaque = false
        metal.backgroundColor = NSColor.clear.cgColor
        metal.pixelFormat = .bgra8Unorm
        metal.framebufferOnly = false
        metal.presentsWithTransaction = false
        metal.displaySyncEnabled = true
    }
}

private struct RigHostViewport {
    var width: Double = 0
    var height: Double = 0
    var backingScale: Double = 1
    var epoch: UInt64 = 0
}

private struct RigModelSource {
    let id: String
    let fileURL: URL
    let overridesURL: URL
    let motionURL: URL
}

private struct RigPreparedModel {
    let scene: RigDecodedScene
    let motion: RigMotionEvaluator
    let displayBounds: RigBounds?
    let geometry: RigAnchorGeometry
}

private struct RigSurfaceRequest {
    let layer: CAMetalLayer
    let evaluated: RigEvaluatedMotion
    let serial: UInt64
    let viewport: RigHostViewport
    let intent: HerdrRigIntent
    let createdAt: TimeInterval
}

private enum RigHostState {
    case decoding
    case ready
    case cancelled
    case failed(String)
}

private struct RigAnchorExtent {
    var x0 = Double.infinity
    var y0 = Double.infinity
    var x1 = -Double.infinity
    var y1 = -Double.infinity

    mutating func include(_ x: Double, _ y: Double) {
        guard x.isFinite, y.isFinite else { return }
        x0 = min(x0, x)
        y0 = min(y0, y)
        x1 = max(x1, x)
        y1 = max(y1, y)
    }

    var bounds: RigBounds? {
        guard x0 < x1, y0 < y1 else { return nil }
        return RigBounds(x0: x0, y0: y0, x1: x1, y1: y1)
    }
}

/// Source-alpha bounds are prepared once per model. The separate CPU deformers
/// are evaluated only on a bubble cache miss; neither GPU state nor pointer-hit
/// requests are touched.
private final class RigAnchorGeometry {
    private let base: RigDeformer
    private let pose: RigDeformer?
    private let baseBounds: [(visible: RigBounds?, head: RigBounds?, display: RigBounds?)]
    private let poseBounds: [(visible: RigBounds?, head: RigBounds?, display: RigBounds?)]

    init(scene: RigDecodedScene) throws {
        base = try RigDeformer(rig: scene.document.base, physics: scene.document.physics)
        pose = try scene.document.pose.map { try RigDeformer(rig: $0, physics: scene.document.physics) }
        baseBounds = Self.alphaBounds(meshes: base.meshes, rig: base.rig, pixels: scene.rgba)
        poseBounds = pose.map { Self.alphaBounds(meshes: $0.meshes, rig: $0.rig, pixels: scene.rgba) } ?? []
    }
    func prepareMotion(_ motion: RigMotionEvaluator) throws {
        try motion.prepare(baseMeshes: base.meshes, poseMeshes: pose?.meshes,
                           canvas: base.rig.canvas)
    }


    private static func alphaBounds(
        meshes: [RigMesh], rig: RigDefinition, pixels: Data
    ) -> [(visible: RigBounds?, head: RigBounds?, display: RigBounds?)] {
        pixels.withUnsafeBytes { raw in
            let bytes = raw.bindMemory(to: UInt8.self)
            return meshes.map { mesh in
                let layer = mesh.layer
                let image = layer.img
                let head = rig.interactionAreas?["head"]
                var visible = RigAnchorExtent()
                var headVisible = RigAnchorExtent()
                var displayVisible = RigAnchorExtent()
                guard image.width > 0, image.height > 0 else { return (nil, nil, nil) }
                for row in 0..<image.height {
                    for column in 0..<image.width {
                        let offset = image.offset + (row * image.width + column) * 4 + 3
                        if bytes[offset] == 0 { continue }
                        let x0 = layer.x + layer.w * Double(column) / Double(image.width)
                        let y0 = layer.y + layer.h * Double(row) / Double(image.height)
                        let x1 = layer.x + layer.w * Double(column + 1) / Double(image.width)
                        let y1 = layer.y + layer.h * Double(row + 1) / Double(image.height)
                        displayVisible.include(x0, y0)
                        displayVisible.include(x1, y1)
                        if bytes[offset] < 8 { continue }
                        visible.include(x0, y0)
                        visible.include(x1, y1)
                        let cx = (x0 + x1) * 0.5
                        let cy = (y0 + y1) * 0.5
                        if let head, cx >= head.x0, cx < head.x1, cy >= head.y0, cy < head.y1 {
                            headVisible.include(x0, y0)
                            headVisible.include(x1, y1)
                        }
                    }
                }
                return (visible.bounds, headVisible.bounds, displayVisible.bounds)
            }
        }
    }

    private static func project(
        _ source: RigBounds, mesh: RigMesh, into extent: inout RigAnchorExtent
    ) {
        let layer = mesh.layer
        guard layer.w > 0, layer.h > 0 else { return }
        let u0 = max(0, min(1, (source.x0 - layer.x) / layer.w))
        let u1 = max(0, min(1, (source.x1 - layer.x) / layer.w))
        let v0 = max(0, min(1, (source.y0 - layer.y) / layer.h))
        let v1 = max(0, min(1, (source.y1 - layer.y) / layer.h))
        guard u0 < u1, v0 < v1 else { return }
        // Include cropped edges as well as every intervening mesh-grid line:
        // bilinear deformations can bulge between the four outer corners.
        for row in 0...(mesh.ny + 2) {
            let v = row == 0 ? v0 : (row == mesh.ny + 2 ? v1 : Double(row - 1) / Double(mesh.ny))
            if v < v0 || v > v1 { continue }
            for column in 0...(mesh.nx + 2) {
                let u = column == 0 ? u0 : (column == mesh.nx + 2 ? u1 : Double(column - 1) / Double(mesh.nx))
                if u < u0 || u > u1 { continue }
                let gridX = u * Double(mesh.nx), gridY = v * Double(mesh.ny)
                let ix = min(mesh.nx - 1, Int(gridX))
                let iy = min(mesh.ny - 1, Int(gridY))
                let tx = gridX - Double(ix), ty = gridY - Double(iy)
                let stride = mesh.nx + 1
                let a = mesh.positions[iy * stride + ix]
                let b = mesh.positions[iy * stride + ix + 1]
                let c = mesh.positions[(iy + 1) * stride + ix]
                let d = mesh.positions[(iy + 1) * stride + ix + 1]
                let x = (1 - ty) * ((1 - tx) * Double(a.x) + tx * Double(b.x))
                    + ty * ((1 - tx) * Double(c.x) + tx * Double(d.x))
                let y = (1 - ty) * ((1 - tx) * Double(a.y) + tx * Double(b.y))
                    + ty * ((1 - tx) * Double(c.y) + tx * Double(d.y))
                extent.include(x, y)
            }
        }
    }

    func displayBounds(motion: RigMotionEvaluator) -> RigBounds? {
        var extent = RigAnchorExtent()
        func collect(_ deformer: RigDeformer,
                     _ sources: [(visible: RigBounds?, head: RigBounds?, display: RigBounds?)], pose: Bool) {
            for (index, mesh) in deformer.meshes.enumerated() {
                guard let visible = sources[index].display else { continue }
                let source = deformer.envelopeSourceLayer(at: index)
                let linked = source.name != mesh.layer.name
                let alpha = linked
                    ? RigBounds(x0: source.x, y0: source.y,
                                x1: source.x + source.w, y1: source.y + source.h)
                    : visible
                let deformation = deformer.envelopeDisplacement(at: index, motion: motion)
                let local = motion.layerEnvelope(pose: pose, meshIndex: index)
                // The deformed origin is a bilinear interpolation of vertices
                // on this same mesh. Every vertex lies at most the mesh diagonal
                // plus twice the deformation radius from that origin.
                let radial = hypot(source.w, source.h) + 2 * deformation
                // Linear texture filtering extends alpha by at most one source
                // texel; account for that before/after the local affine motion.
                let texel = max(mesh.layer.w / Double(mesh.layer.img.width),
                                mesh.layer.h / Double(mesh.layer.img.height))
                let margin = deformation + local.translation + local.linear * radial
                    + texel * (1 + local.linear)
                extent.include(alpha.x0 - margin, alpha.y0 - margin)
                extent.include(alpha.x1 + margin, alpha.y1 + margin)
            }
        }
        collect(base, baseBounds, pose: false)
        if let pose { collect(pose, poseBounds, pose: true) }
        return extent.bounds
    }

    func bounds(evaluated: RigEvaluatedMotion, neutral: Bool, canvas: (Int, Int)) -> HerdrRigAnchor? {
        var head = RigAnchorExtent()
        var visible = RigAnchorExtent()
        func collect(
            _ deformer: RigDeformer, _ sources: [(visible: RigBounds?, head: RigBounds?, display: RigBounds?)],
            local: [RigLocalTransform]
        ) {
            deformer.update(
                parameters: evaluated.parameters, time: evaluated.timeMS, neutral: neutral,
                localTransforms: local
            )
            for (mesh, source) in zip(deformer.meshes, sources) where mesh.alpha >= 8.0 / 255.0 {
                if let bounds = source.visible { Self.project(bounds, mesh: mesh, into: &visible) }
                if let bounds = source.head { Self.project(bounds, mesh: mesh, into: &head) }
            }
        }
        if evaluated.poseMix < 1 || pose == nil {
            collect(base, baseBounds, local: evaluated.local.base)
        }
        if evaluated.poseMix > 0, let pose {
            collect(pose, poseBounds, local: evaluated.local.pose)
        }
        guard let bounds = head.bounds ?? visible.bounds else { return nil }
        let x0 = max(0, min(Double(canvas.0), bounds.x0))
        let y0 = max(0, min(Double(canvas.1), bounds.y0))
        let x1 = max(0, min(Double(canvas.0), bounds.x1))
        let y1 = max(0, min(Double(canvas.1), bounds.y1))
        guard x0 < x1, y0 < y1 else { return nil }
        var anchor = HerdrRigAnchor()
        anchor.x0 = x0
        anchor.y0 = y0
        anchor.x1 = x1
        anchor.y1 = y1
        return anchor
    }
}

final class RigNativeHost {
    private static let surfaceFreshness: TimeInterval = 0.25
    private static let displayFrameInterval: DispatchTimeInterval = .nanoseconds(16_666_667)
    // Finish CPU preparation before exposing a catalog. A cold PSD decode can
    // outlast an entire interaction; model entry must only upload prepared data.
    private static let catalogDecodeQueue: OperationQueue = {
        let queue = OperationQueue()
        queue.name = "herdr.rig.catalog-decode"
        queue.qualityOfService = .utility
        queue.maxConcurrentOperationCount = RigLimits.catalogDecodeWorkers
        return queue
    }()
    let view: RigSurfaceView
    let token: HerdrRigTokenSnapshot

    private let baseURL: URL
    private let poseURL: URL?
    private let overridesURL: URL

    private let modelSources: [RigModelSource]
    private let modelBindings: [Int]
    private let independentModels: Bool
    private var selectedModelIndex = 0
    private var preparedModels: [Int: RigPreparedModel] = [:]
    private var preparedModelRGBABytes = 0
    private var catalogDecodeJobs: [RigDecodeJob] = []
    private var modelDecodeGeneration: UInt64 = 0
    private var pendingModelIndex: Int?
    private var rejectedModelIndex: Int?
    private let helperURL: URL
    private let bundleURL: URL
    private let canvasWidth: Int
    private let canvasHeight: Int
    private let basePoseID: String
    private let alternatePoseID: String?
    private let device: MTLDevice
    private let surfaceLayer: CAMetalLayer
    private let decodeQueue = DispatchQueue(label: "herdr.rig.decode", qos: .utility)
    private let renderQueue = DispatchQueue(label: "herdr.rig.render", qos: .userInteractive)
    private let stateLock = NSLock()
    private var state: RigHostState = .decoding
    private var cancelled = false
    private var active = false
    private var hitsDisabled = true
    private var recoveryUsed = false
    private var decodeJob: RigDecodeJob?
    private var scene: RigDecodedScene?
    private var renderer: RigMetalRenderer?
    private var anchorGeometry: RigAnchorGeometry?
    private var preparedDisplayBounds: RigBounds?
    private var evaluator: RigMotionEvaluator
    private var latestIntent: HerdrRigIntent?
    private var latestEvaluated: RigEvaluatedMotion?
    private var viewport = RigHostViewport()
    // This generation fences semantic phase/effect transitions and viewport
    // changes.  Scalar clock progression intentionally does not advance it:
    // callers may poll prepareSurface at a higher rate than the display driver.
    private var intentSerial: UInt64 = 0
    private var inputGeneration: UInt64 = 1
    private var renderPending = false
    private var surfaceReady = false
    private var surfaceReadySerial: UInt64 = 0
    private var surfaceReadyViewportEpoch: UInt64 = 0
    private var surfaceReadyAt: TimeInterval = 0
    // Positive geometry only: an observed stale/invalid surface drops it so a
    // same-serial recovery must sample the current evaluation, never old bounds.
    private var cachedSpeechAnchor: (inputGeneration: UInt64, intentSerial: UInt64,
                                     viewport: RigHostViewport, anchor: HerdrRigAnchor)?
    private var latestIntentUptime: TimeInterval = 0
    private var animatedIntent: HerdrRigIntent?
    private var displayTimer: DispatchSourceTimer?
    private var visible = false
    private var lastError: String?

    init(asset: HerdrRigAssetInput, token: HerdrRigTokenInput) throws {
        guard Thread.isMainThread else {
            throw RigNativeError.invalid("rig host must be created on the AppKit main thread")
        }
        guard asset.width > 0, asset.height > 0,
              asset.width <= UInt32(RigLimits.canvasDimension),
              asset.height <= UInt32(RigLimits.canvasDimension),
              asset.width <= UInt32(RigLimits.canvasPixels) / max(1, asset.height) else {
            throw RigNativeError.invalid("rig canvas quota exceeded")
        }
        let modelCount = Int(asset.model_count)
        guard modelCount <= 16 else {
            throw RigNativeError.invalid("rig model catalog exceeds the 16-model quota")
        }
        var sourceModels: [RigModelSource] = []
        var modelBindings: [Int] = []
        if modelCount > 0 {
            guard let rawModels = asset.models, let rawBindings = asset.bindings,
                  modelCount >= 10 else {
                throw RigNativeError.invalid("independent rig catalog is missing models or bindings")
            }
            sourceModels.reserveCapacity(modelCount)
            for index in 0..<modelCount {
                let raw = rawModels[index]
                guard let idPointer = raw.id, let filePointer = raw.file_path,
                      let overridesPointer = raw.overrides_path,
                      let motionPointer = raw.motion_path,
                      let id = String(validatingUTF8: idPointer),
                      let file = String(validatingUTF8: filePointer),
                      let overrides = String(validatingUTF8: overridesPointer),
                      let motion = String(validatingUTF8: motionPointer),
                      !id.isEmpty else {
                    throw RigNativeError.invalid("independent rig model descriptor is invalid UTF-8")
                }
                sourceModels.append(RigModelSource(
                    id: id,
                    fileURL: URL(fileURLWithPath: file, isDirectory: false),
                    overridesURL: URL(fileURLWithPath: overrides, isDirectory: false),
                    motionURL: URL(fileURLWithPath: motion, isDirectory: false)
                ))
            }
            modelBindings = (0..<10).map { Int(rawBindings[$0]) }
            guard modelBindings.allSatisfy({ $0 >= 0 && $0 < modelCount }),
                  Set(modelBindings).count == modelBindings.count else {
                throw RigNativeError.invalid("independent rig semantic bindings are invalid")
            }
        }
        guard let legacyBase = asset.base_path, let legacyOverrides = asset.overrides_path,
              let legacyPoseID = asset.base_pose_id,
              let legacyBaseString = String(validatingUTF8: legacyBase),
              let legacyOverridesString = String(validatingUTF8: legacyOverrides),
              let legacyBasePoseID = String(validatingUTF8: legacyPoseID),
              !legacyBasePoseID.isEmpty else {
            throw RigNativeError.invalid("rig source paths or base pose id are invalid UTF-8")
        }
        let legacyPoseString = asset.pose_path.flatMap { String(validatingUTF8: $0) }
        let legacyAlternateID = asset.pose_id.flatMap { String(validatingUTF8: $0) }
        if legacyPoseString != nil && (legacyAlternateID == nil || legacyAlternateID?.isEmpty == true) {
            throw RigNativeError.invalid("alternate pose path requires an alternate pose id")
        }
        if legacyPoseString == nil && legacyAlternateID != nil {
            throw RigNativeError.invalid("alternate pose id requires an alternate pose path")
        }
        let independent = !sourceModels.isEmpty
        let initialModelIndex: Int
        let basePathString: String
        let overridesPathString: String
        let basePoseID: String
        let posePathString: String?
        let alternatePoseID: String?
        if independent {
            let initialKind = Int(asset.initial_pose_kind)
            guard (0..<10).contains(initialKind) else {
                throw RigNativeError.invalid("independent rig initial pose kind is invalid")
            }
            initialModelIndex = modelBindings[initialKind]
            let source = sourceModels[initialModelIndex]
            basePathString = source.fileURL.path
            overridesPathString = source.overridesURL.path
            basePoseID = source.id
            posePathString = nil
            alternatePoseID = nil
        } else {
            guard asset.initial_pose_kind == -1 else {
                throw RigNativeError.invalid("legacy rig must use the legacy pose kind")
            }
            initialModelIndex = 0
            basePathString = legacyBaseString
            overridesPathString = legacyOverridesString
            basePoseID = legacyBasePoseID
            posePathString = legacyPoseString
            alternatePoseID = legacyAlternateID
        }
        guard let operationID = assetTokenString(token.operation_id),
              let referenceID = assetTokenString(token.reference_id),
              let digest = assetTokenString(token.content_digest),
              !operationID.isEmpty, !referenceID.isEmpty, !digest.isEmpty else {
            throw RigNativeError.invalid("renderer token contains invalid UTF-8")
        }
        self.token = HerdrRigTokenSnapshot(operationID: operationID, referenceID: referenceID,
                                           referenceRevision: token.reference_revision,
                                           contentDigest: digest, backendEpoch: token.backend_epoch)
        self.modelSources = sourceModels
        self.modelBindings = modelBindings
        self.independentModels = independent
        self.selectedModelIndex = initialModelIndex
        self.baseURL = URL(fileURLWithPath: basePathString, isDirectory: false)
        self.poseURL = posePathString.map { URL(fileURLWithPath: $0, isDirectory: false) }
        self.overridesURL = URL(fileURLWithPath: overridesPathString, isDirectory: false)
        self.canvasWidth = Int(asset.width)
        self.canvasHeight = Int(asset.height)
        self.basePoseID = basePoseID
        self.alternatePoseID = alternatePoseID
        self.helperURL = try Self.trustedRuntime().helper
        self.bundleURL = try Self.trustedRuntime().bundle
        guard let device = MTLCreateSystemDefaultDevice() else {
            throw RigNativeError.unavailable("Metal device unavailable")
        }
        self.device = device
        guard asset.initial_motion_length <= 64 * 1024,
              let motionBytes = asset.initial_motion_bytes else {
            throw RigNativeError.invalid("motion source is not a bounded regular file")
        }
        // Copy while the Rust snapshot still owns the borrowed asset bytes.
        let motionData = Data(bytes: motionBytes, count: asset.initial_motion_length)
        self.evaluator = try RigMotionEvaluator(data: motionData, basePoseID: basePoseID,
                                                alternatePoseID: alternatePoseID)
        self.view = RigSurfaceView(frame: NSRect(x: 0, y: 0, width: 1, height: 1))
        self.view.setFrameSize(NSSize(width: 1, height: 1))
        self.surfaceLayer = self.view.metalLayer
        self.surfaceLayer.device = device
        self.surfaceLayer.drawableSize = CGSize(width: canvasWidth, height: canvasHeight)
        self.startDecode()
    }

    deinit {
        cancel()
        renderer = nil
        scene = nil
    }

    func poll(cancelRequested: Bool) -> Int32 {
        if cancelRequested { cancel() }
        stateLock.lock()
        defer { stateLock.unlock() }
        switch state {
        case .decoding: return cancelled ? 2 : 0
        case .ready: return 1
        case .cancelled: return 2
        case .failed: return -1
        }
    }

    func errorDescription() -> String? {
        stateLock.lock(); defer { stateLock.unlock() }
        if let lastError { return lastError }
        if case .failed(let message) = state { return message }
        return nil
    }

    func cancel() {
        stateLock.lock()
        inputGeneration &+= 1
        cancelled = true
        active = false
        visible = false
        surfaceReady = false
        hitsDisabled = true
        cachedSpeechAnchor = nil
        stopDisplayDriverLocked()
        if case .ready = state {
            state = .cancelled
        } else if case .decoding = state {
            state = .cancelled
        }
        let job = decodeJob
        let catalogJobs = catalogDecodeJobs
        stateLock.unlock()
        job?.cancel()
        for catalogJob in catalogJobs { catalogJob.cancel() }
        if Thread.isMainThread {
            view.isHidden = true
        } else {
            let view = self.view
            DispatchQueue.main.async { view.isHidden = true }
        }
    }

    func update(_ intent: HerdrRigIntent) throws {
        guard Thread.isMainThread else { throw RigNativeError.invalid("rig update must run on the AppKit main thread") }
        stateLock.lock()
        defer { stateLock.unlock() }
        if case .decoding = state, recoveryUsed, !cancelled {
            // A recovering renderer still accepts the current semantic clock.
        } else {
            try ensureUsableLocked()
        }
        _ = try acceptIntentLocked(intent, at: ProcessInfo.processInfo.systemUptime)
    }

    func prepareSurface(_ intent: HerdrRigIntent, width: Double, height: Double,
                        backingScale: Double, viewportEpoch: UInt64) throws -> Bool {
        guard Thread.isMainThread else { throw RigNativeError.invalid("surface preparation must run on the AppKit main thread") }
        let drawableSize = try Self.checkedDrawableSize(width, height, backingScale)
        let now = ProcessInfo.processInfo.systemUptime
        stateLock.lock()
        do {
            try ensureUsableLocked()
            let evaluated = try acceptIntentLocked(intent, at: now)
            let viewportChanged = viewport.width != width || viewport.height != height
                || viewport.backingScale != backingScale || viewport.epoch != viewportEpoch
            if viewportChanged {
                viewport = RigHostViewport(width: width, height: height,
                                           backingScale: backingScale, epoch: viewportEpoch)
                intentSerial &+= 1
                surfaceReady = false
                hitsDisabled = true
                cachedSpeechAnchor = nil
            }
            let currentSerial = intentSerial
            let currentViewport = viewport
            let ready = isCurrentSurfaceLocked(now, serial: currentSerial, viewport: currentViewport)
            if ready {
                stateLock.unlock()
                return true
            }
            if renderPending {
                stateLock.unlock()
                return false
            }
            let layer = surfaceLayer
            layer.contentsScale = backingScale
            layer.drawableSize = drawableSize
            let renderIntent = animatedIntent ?? intent
            let request = RigSurfaceRequest(layer: layer, evaluated: evaluated,
                                            serial: currentSerial, viewport: currentViewport,
                                            intent: renderIntent, createdAt: now)
            guard let renderer = self.renderer else {
                renderPending = false
                stateLock.unlock()
                markFailure("GPU renderer disappeared before surface preparation")
                return false
            }
            renderPending = true
            stateLock.unlock()
            enqueueRender(request, renderer: renderer)
            return false
        } catch {
            stateLock.unlock()
            throw error
        }
    }

    func activate() throws {
        guard Thread.isMainThread else { throw RigNativeError.invalid("rig activation must run on the AppKit main thread") }
        stateLock.lock()
        do {
            try ensureUsableLocked()
            guard surfaceReady else {
                throw RigNativeError.unavailable("rig candidate has no current drawable")
            }
            active = true
            if visible { startDisplayDriverLocked() }
            stateLock.unlock()
        } catch {
            stateLock.unlock()
            throw error
        }
        // Main owns global visibility.  It attaches this view hidden, calls
        // activate only after the current-intent drawable is ready, and then
        // applies the existing pet visibility policy through setVisible.
    }

    func setViewportEpoch(_ epoch: UInt64) {
        guard Thread.isMainThread else { return }
        let size = view.bounds.size
        let scale = min(4, max(1, view.window.map { Double($0.backingScaleFactor) } ?? viewport.backingScale))
        guard let drawableSize = try? Self.checkedDrawableSize(Double(size.width), Double(size.height), scale) else {
            markFailure("rig viewport exceeds drawable limits")
            return
        }
        stateLock.lock()
        let geometryChanged = viewport.width != size.width || viewport.height != size.height
            || viewport.backingScale != scale || viewport.epoch != epoch
        if geometryChanged {
            viewport = RigHostViewport(width: size.width, height: size.height,
                                       backingScale: scale, epoch: epoch)
            intentSerial &+= 1
            surfaceReady = false
            hitsDisabled = true
            cachedSpeechAnchor = nil
            surfaceLayer.contentsScale = scale
            surfaceLayer.drawableSize = drawableSize
        }
        stateLock.unlock()
    }

    func setVisible(_ visible: Bool) {
        guard Thread.isMainThread else { return }
        stateLock.lock()
        if self.visible != visible { inputGeneration &+= 1 }
        if self.visible != visible { cachedSpeechAnchor = nil }
        self.visible = visible
        if visible {
            if active { startDisplayDriverLocked() }
        } else {
            stopDisplayDriverLocked()
            intentSerial &+= 1
            surfaceReady = false
            hitsDisabled = true
            cachedSpeechAnchor = nil
        }
        stateLock.unlock()
        view.isHidden = !visible
    }

    func inputEpoch() -> UInt64 {
        stateLock.lock()
        defer { stateLock.unlock() }
        guard active, visible, !cancelled, case .ready = state else { return 0 }
        return inputGeneration
    }

    func inputReady() -> Bool {
        stateLock.lock()
        defer { stateLock.unlock() }
        let age = ProcessInfo.processInfo.systemUptime - surfaceReadyAt
        return active && visible && !cancelled
            && !hitsDisabled && surfaceReady && age >= 0 && age <= 0.08
    }


    func displayBounds() -> HerdrRigAnchor? {
        stateLock.lock()
        defer { stateLock.unlock() }
        guard case .ready = state, let bounds = preparedDisplayBounds else { return nil }
        let width = Double(canvasWidth), height = Double(canvasHeight)
        let x0 = max(0, min(width, bounds.x0))
        let y0 = max(0, min(height, bounds.y0))
        let x1 = max(0, min(width, bounds.x1))
        let y1 = max(0, min(height, bounds.y1))
        guard x0 < x1, y0 < y1 else { return nil }
        var output = HerdrRigAnchor()
        output.x0 = x0 / width
        output.y0 = y0 / height
        output.x1 = x1 / width
        output.y1 = y1 / height
        return output
    }

    func speechAnchorSnapshot() throws -> HerdrRigSpeechAnchorSnapshot {
        guard Thread.isMainThread else {
            throw RigNativeError.invalid("rig speech anchor snapshot must run on the AppKit main thread")
        }
        stateLock.lock()
        defer { stateLock.unlock() }
        let now = ProcessInfo.processInfo.systemUptime
        var snapshot = HerdrRigSpeechAnchorSnapshot()
        snapshot.backend_epoch = token.backendEpoch
        guard active, visible, !cancelled, lastError == nil,
              case .ready = state, scene != nil, renderer != nil else {
            cachedSpeechAnchor = nil
            return snapshot
        }
        snapshot.input_epoch = inputGeneration
        snapshot.anchor_epoch = intentSerial
        snapshot.viewport_epoch = viewport.epoch
        snapshot.canvas_width = UInt32(canvasWidth)
        snapshot.canvas_height = UInt32(canvasHeight)
        snapshot.viewport_width = viewport.width
        snapshot.viewport_height = viewport.height
        snapshot.backing_scale = viewport.backingScale
        snapshot.status = 1
        let age = now - surfaceReadyAt
        guard pendingModelIndex == nil, let anchorGeometry, let evaluated = latestEvaluated,
              animatedIntent != nil, !hitsDisabled, surfaceReady,
              surfaceReadySerial == intentSerial, surfaceReadyViewportEpoch == viewport.epoch,
              age.isFinite, age >= 0, age <= 0.08 else {
            cachedSpeechAnchor = nil
            return snapshot
        }
        // Reuse only after freshness has been established under this same lock.
        if let cached = cachedSpeechAnchor,
           cached.inputGeneration == inputGeneration, cached.intentSerial == intentSerial,
           cached.viewport.width == viewport.width, cached.viewport.height == viewport.height,
           cached.viewport.backingScale == viewport.backingScale,
           cached.viewport.epoch == viewport.epoch {
            snapshot.status = 3
            snapshot.anchor = cached.anchor
            return snapshot
        }
        guard let anchor = anchorGeometry.bounds(evaluated: evaluated,
                                                  neutral: animatedIntent?.frozen != 0,
                                                  canvas: (canvasWidth, canvasHeight)) else {
            // Empty bounds are never cached; a later current evaluation may
            // expose geometry without changing the semantic serial.
            cachedSpeechAnchor = nil
            snapshot.status = 2
            return snapshot
        }
        cachedSpeechAnchor = (inputGeneration, intentSerial, viewport, anchor)
        snapshot.status = 3
        snapshot.anchor = anchor
        return snapshot
    }

    func hit(normalizedX: Double, normalizedY: Double) -> HerdrRigHit? {
        guard normalizedX.isFinite, normalizedY.isFinite,
              normalizedX >= 0, normalizedX <= 1, normalizedY >= 0, normalizedY <= 1 else { return nil }
        guard stateLock.try() else { return nil }
        guard active, visible, !cancelled, !hitsDisabled, surfaceReady,
              let renderer, viewport.width > 0, viewport.height > 0 else {
            stateLock.unlock()
            return nil
        }
        let currentViewport = viewport
        let currentInputEpoch = inputGeneration
        let currentSerial = intentSerial
        stateLock.unlock()
        let width = Int(max(1, (currentViewport.width * currentViewport.backingScale).rounded()))
        let height = Int(max(1, (currentViewport.height * currentViewport.backingScale).rounded()))
        let mapping = RigCanvasMapping(canvasWidth: Int(canvasWidth), canvasHeight: Int(canvasHeight),
                                       drawableWidth: width, drawableHeight: height)
        let point = mapping.modelPoint(drawableX: normalizedX * Double(width), drawableY: normalizedY * Double(height))
        guard point.x >= 0, point.y >= 0, point.x < Double(canvasWidth), point.y < Double(canvasHeight) else { return nil }
        let modelX = point.x, modelY = point.y
        guard let cached = renderer.cachedHit(modelX: modelX, modelY: modelY,
                                              minimumAlpha: 7.5 / 255.0) else { return nil }
        let age = ProcessInfo.processInfo.systemUptime - cached.capturedAt
        guard age >= 0, age <= 0.08 else { return nil }
        guard stateLock.try() else { return nil }
        defer { stateLock.unlock() }
        guard active, visible, !hitsDisabled, surfaceReady,
              inputGeneration == currentInputEpoch, intentSerial == currentSerial else { return nil }
        let alpha = UInt8(min(255, max(0, Int((Double(cached.sample.alpha) * 255).rounded()))))
        let semantic = alpha >= 8 ? cached.sample.semantic : nil
        let region: UInt8 = semantic?.region == "head" ? 1 : (semantic?.region == "body" ? 2 : 0)
        var output = HerdrRigHit()
        output.alpha = alpha
        output.region = region
        output.reserved = 0
        output.source_x = semantic?.sourceX ?? .nan
        output.source_y = semantic?.sourceY ?? .nan
        output.frame = cached.frameNumber
        output.viewport_epoch = currentViewport.epoch
        output.input_epoch = currentInputEpoch
        return output

    }
    private func acceptIntentLocked(_ raw: HerdrRigIntent, at uptime: TimeInterval) throws -> RigEvaluatedMotion {
        if independentModels {
            guard raw.pose_kind == -1 || (0..<10).contains(Int(raw.pose_kind)) else {
                throw RigNativeError.invalid("pose kind is outside the v5 semantic range")
            }
            if raw.pose_kind >= 0 {
                guard raw.pose_age_seconds.isFinite, raw.pose_age_seconds >= 0 else {
                    throw RigNativeError.invalid("v5 pose age is not finite or is negative")
                }
                let requested = modelBindings[Int(raw.pose_kind)]
                if requested == selectedModelIndex {
                    if pendingModelIndex != nil {
                        cancelPendingModelSwitchLocked()
                    }
                } else {
                    if rejectedModelIndex == requested {
                        throw RigNativeError.unavailable("requested v5 model previously failed to decode")
                    }
                    requestModelSwitchLocked(requested)
                }
            }
        }
        let effectChanged = latestIntent.map { !sameEffect($0, raw) } ?? true
        let poseChanged = independentModels
            && (latestIntent.map { $0.pose_kind != raw.pose_kind } ?? true)
        let semanticChanged = latestIntent.map { !sameSemanticIntent($0, raw) } ?? true
        var intent = raw
        if independentModels && raw.pose_kind >= 0 {
            intent.phase_age_seconds = raw.pose_age_seconds
        }
        if let previous = animatedIntent {
            if intent.now_seconds.isFinite, previous.now_seconds.isFinite {
                intent.now_seconds = max(intent.now_seconds, previous.now_seconds)
            }
            if !poseChanged, intent.phase == previous.phase,
               intent.phase_age_seconds.isFinite, previous.phase_age_seconds.isFinite {
                intent.phase_age_seconds = max(intent.phase_age_seconds, previous.phase_age_seconds)
            }
            if !effectChanged, intent.effect == previous.effect,
               intent.effect_age_seconds.isFinite, previous.effect_age_seconds.isFinite {
                intent.effect_age_seconds = max(intent.effect_age_seconds, previous.effect_age_seconds)
            }
        }
        if effectChanged { evaluator.restartEffectClock() }
        if poseChanged { evaluator.resynchronize() }
        let evaluated = try evaluator.evaluate(intent)
        latestIntent = raw
        latestEvaluated = evaluated
        animatedIntent = intent
        latestIntentUptime = uptime
        if semanticChanged {
            intentSerial &+= 1
            surfaceReady = false
            hitsDisabled = true
            cachedSpeechAnchor = nil
        }
        return evaluated
    }

    private func advancedIntentLocked(at uptime: TimeInterval) -> HerdrRigIntent? {
        guard var intent = latestIntent else { return nil }
        let elapsed = intent.frozen == 0 ? max(0, uptime - latestIntentUptime) : 0
        guard elapsed.isFinite else { return nil }
        intent.now_seconds += elapsed
        intent.phase_age_seconds += elapsed
        intent.effect_age_seconds += elapsed
        if independentModels && intent.pose_kind >= 0 {
            intent.pose_age_seconds += elapsed
            guard intent.pose_age_seconds.isFinite, intent.pose_age_seconds >= 0 else { return nil }
            intent.phase_age_seconds = intent.pose_age_seconds
        }
        if let previous = animatedIntent {
            if intent.now_seconds.isFinite, previous.now_seconds.isFinite {
                intent.now_seconds = max(intent.now_seconds, previous.now_seconds)
            }
            if intent.phase == previous.phase,
               intent.phase_age_seconds.isFinite, previous.phase_age_seconds.isFinite {
                intent.phase_age_seconds = max(intent.phase_age_seconds, previous.phase_age_seconds)
            }
            if intent.effect == previous.effect,
               intent.effect_age_seconds.isFinite, previous.effect_age_seconds.isFinite {
                intent.effect_age_seconds = max(intent.effect_age_seconds, previous.effect_age_seconds)
            }
        }
        return intent
    }
    private func cancelPendingModelSwitchLocked() {
        guard pendingModelIndex != nil else { return }
        modelDecodeGeneration &+= 1
        pendingModelIndex = nil
        inputGeneration &+= 1
        intentSerial &+= 1
        surfaceReady = false
        hitsDisabled = true
        cachedSpeechAnchor = nil
        lastError = nil
    }


    private func requestModelSwitchLocked(_ index: Int) {
        guard independentModels, index != selectedModelIndex else { return }
        if pendingModelIndex == index { return }
        pendingModelIndex = index
        rejectedModelIndex = nil
        modelDecodeGeneration &+= 1
        inputGeneration &+= 1
        intentSerial &+= 1
        surfaceReady = false
        hitsDisabled = true
        cachedSpeechAnchor = nil
        lastError = nil
        decodeJob?.cancel()
        let generation = modelDecodeGeneration
        DispatchQueue.main.async { [weak self] in
            self?.startPendingModelPreparation(index: index, generation: generation)
        }
    }

    private func sameEffect(_ lhs: HerdrRigIntent, _ rhs: HerdrRigIntent) -> Bool {
        lhs.effect == rhs.effect && lhs.effect_duration_seconds == rhs.effect_duration_seconds
            && (lhs.effect < 0 || abs((lhs.now_seconds - lhs.effect_age_seconds)
                - (rhs.now_seconds - rhs.effect_age_seconds)) <= 0.000_001)
    }

    private func sameSemanticIntent(_ lhs: HerdrRigIntent, _ rhs: HerdrRigIntent) -> Bool {
        lhs.phase == rhs.phase && sameEffect(lhs, rhs)
            && lhs.frozen == rhs.frozen
            && (!independentModels || lhs.pose_kind == rhs.pose_kind)
    }


    private func isCurrentSurfaceLocked(_ now: TimeInterval, serial: UInt64,
                                        viewport: RigHostViewport) -> Bool {
        guard surfaceReady, surfaceReadySerial == serial,
              surfaceReadyViewportEpoch == viewport.epoch else { return false }
        let age = now - surfaceReadyAt
        return age.isFinite && age >= 0 && age <= Self.surfaceFreshness
    }

    private func startDisplayDriverLocked() {
        guard displayTimer == nil else { return }
        guard active, visible, !cancelled, case .ready = state else { return }
        let timer = DispatchSource.makeTimerSource(queue: renderQueue)
        timer.schedule(deadline: .now() + .milliseconds(1),
                       repeating: Self.displayFrameInterval, leeway: .milliseconds(2))
        timer.setEventHandler { [weak self] in self?.displayTick() }
        timer.resume()
        displayTimer = timer
    }

    private func stopDisplayDriverLocked() {
        guard let timer = displayTimer else { return }
        timer.setEventHandler(handler: {})
        timer.cancel()
        displayTimer = nil
    }

    private func displayTick() {
        let now = ProcessInfo.processInfo.systemUptime
        stateLock.lock()
        guard active, visible, !cancelled, !renderPending,
              let renderer, let intent = advancedIntentLocked(at: now) else {
            stateLock.unlock()
            return
        }
        guard case .ready = state else {
            stateLock.unlock()
            return
        }
        let evaluated: RigEvaluatedMotion
        do {
            evaluated = try evaluator.evaluate(intent)
        } catch {
            stateLock.unlock()
            markFailure(String(describing: error))
            return
        }
        latestEvaluated = evaluated
        animatedIntent = intent
        renderPending = true
        let request = RigSurfaceRequest(layer: surfaceLayer, evaluated: evaluated,
                                        serial: intentSerial, viewport: viewport, intent: intent,
                                        createdAt: now)
        stateLock.unlock()
        enqueueRender(request, renderer: renderer)
    }

    private func enqueueRender(_ request: RigSurfaceRequest, renderer: RigMetalRenderer) {
        renderQueue.async { [weak self] in
            guard let self else { return }
            self.stateLock.lock()
            let canRender = !self.cancelled
                && self.intentSerial == request.serial
                && self.viewport.epoch == request.viewport.epoch
            self.stateLock.unlock()
            guard canRender, let drawable = request.layer.nextDrawable() else {
                self.stateLock.lock()
                self.renderPending = false
                self.stateLock.unlock()
                return
            }
            self.stateLock.lock()
            let stillCurrent = !self.cancelled
                && self.intentSerial == request.serial
                && self.viewport.epoch == request.viewport.epoch
            if !stillCurrent { self.renderPending = false }
            self.stateLock.unlock()
            guard stillCurrent else { return }
            do {
                _ = try renderer.draw(
                    to: drawable, parameters: request.evaluated.parameters,
                    time: request.evaluated.timeMS, neutral: request.intent.frozen != 0,
                    poseMix: request.evaluated.poseMix, localMotion: request.evaluated.local,
                    capture: false
                )
                self.stateLock.lock()
                self.renderPending = false
                if !self.cancelled && self.intentSerial == request.serial,
                   self.viewport.epoch == request.viewport.epoch {
                    self.surfaceReady = true
                    self.surfaceReadySerial = request.serial
                    self.surfaceReadyViewportEpoch = request.viewport.epoch
                    self.surfaceReadyAt = request.createdAt
                    self.hitsDisabled = false
                }
                self.stateLock.unlock()
            } catch {
                self.stateLock.lock()
                self.renderPending = false
                self.stateLock.unlock()
                self.markFailure(String(describing: error))
            }
        }
    }

    func previewPNG(_ intent: HerdrRigIntent, hitOverlay: Bool) throws -> Data {
        let deadline = ProcessInfo.processInfo.systemUptime + TimeInterval(RigLimits.decodeSeconds)
        while true {
            stateLock.lock()
            let evaluated: RigEvaluatedMotion
            let renderer: RigMetalRenderer
            let renderIntent: HerdrRigIntent
            let requestedIndex: Int?
            do {
                try ensureUsableLocked()
                evaluated = try acceptIntentLocked(intent, at: ProcessInfo.processInfo.systemUptime)
                requestedIndex = independentModels && intent.pose_kind >= 0
                    ? modelBindings[Int(intent.pose_kind)] : nil
                if pendingModelIndex != nil {
                    stateLock.unlock()
                    try waitForPendingModel(deadline: deadline)
                    continue
                }
                if let requestedIndex, selectedModelIndex != requestedIndex {
                    throw RigNativeError.unavailable("requested v5 model was not selected")
                }
                guard let current = self.renderer else {
                    throw RigNativeError.unavailable("GPU renderer is unavailable")
                }
                renderer = current
                renderIntent = animatedIntent ?? intent
            } catch {
                stateLock.unlock()
                throw error
            }
            stateLock.unlock()
            var frame: RigFrame?
            var renderError: Error?
            renderQueue.sync {
                do {
                    frame = try renderer.render(
                        parameters: evaluated.parameters, time: evaluated.timeMS,
                        neutral: renderIntent.frozen != 0, poseMix: evaluated.poseMix,
                        localMotion: evaluated.local, hitOverlay: hitOverlay
                    )
                } catch {
                    renderError = error
                }
            }
            if let renderError {
                markFailure(String(describing: renderError))
                throw renderError
            }
            guard let frame else { throw RigNativeError.unavailable("preview produced no frame") }
            return try Self.encodePNG(frame)
        }
    }

    private func waitForPendingModel(deadline: TimeInterval) throws {
        guard Thread.isMainThread else {
            throw RigNativeError.invalid("v5 model preparation must run on the AppKit main thread")
        }
        while ProcessInfo.processInfo.systemUptime < deadline {
            stateLock.lock()
            let pending = pendingModelIndex != nil
            let isCancelled = cancelled
            let failure = lastError
            stateLock.unlock()
            if isCancelled { throw RigNativeError.cancelled }
            if !pending {
                if let failure { throw RigNativeError.unavailable(failure) }
                return
            }
            let remaining = max(0.001, deadline - ProcessInfo.processInfo.systemUptime)
            RunLoop.main.run(mode: .default, before: Date(timeIntervalSinceNow: min(0.01, remaining)))
        }
        stateLock.lock()
        if pendingModelIndex != nil {
            cancelPendingModelSwitchLocked()
        }
        stateLock.unlock()
        throw RigNativeError.unavailable("timed out waiting for requested v5 model")
    }

    private func recover() {
        stateLock.lock()
        guard !cancelled, !recoveryUsed, let retainedScene = scene else {
            stateLock.unlock()
            return
        }
        // Rebuild exactly the installed model. A model switch may install a new
        // scene, evaluator and renderer together while this rebuild runs.
        let motion = evaluator
        let failedRenderer = renderer
        recoveryUsed = true
        inputGeneration &+= 1
        stopDisplayDriverLocked()
        state = .decoding
        hitsDisabled = true
        surfaceReady = false
        intentSerial &+= 1
        cachedSpeechAnchor = nil
        stateLock.unlock()
        renderQueue.async { [weak self] in
            guard let self else { return }
            do {
                let next = try RigMetalRenderer(device: self.device, scene: retainedScene)
                try next.prepareMotion(motion)
                _ = try next.render(parameters: RigParameters(), time: 0, neutral: true, poseMix: 0)
                _ = try next.sampleHit(modelX: Double(self.canvasWidth) / 2, modelY: Double(self.canvasHeight) / 2)
                self.stateLock.lock()
                if !self.cancelled && self.renderer === failedRenderer {
                    self.renderer = next
                    self.lastError = nil
                    self.state = .ready
                    self.intentSerial &+= 1
                    self.surfaceReady = false
                    self.surfaceReadyAt = 0
                    self.cachedSpeechAnchor = nil
                    self.renderPending = false
                }
                // A superseding install does not restart the driver stopped above.
                self.startDisplayDriverLocked()
                self.stateLock.unlock()
            } catch {
                self.stateLock.lock()
                if self.renderer === failedRenderer {
                    // recoveryUsed is set, so this cannot start another recovery.
                    _ = self.markFailureLocked(String(describing: error))
                } else {
                    self.startDisplayDriverLocked()
                }
                self.stateLock.unlock()
            }
        }
    }

    private func startDecode() {
        if independentModels {
            decodeQueue.async { [weak self] in self?.prepareCatalog() }
            return
        }
        decodeQueue.async { [weak self] in
            guard let self else { return }
            let job = RigDecodeJob(helperURL: self.helperURL, bundleURL: self.bundleURL,
                                   baseURL: self.baseURL, poseURL: self.poseURL,
                                   overridesURL: self.overridesURL)
            self.stateLock.lock()
            self.decodeJob = job
            let shouldCancel = self.cancelled
            let generation = self.modelDecodeGeneration
            self.stateLock.unlock()
            if shouldCancel { job.cancel() }
            do {
                let decoded = try job.run()
                self.finishDecode(decoded, preparedGeometry: nil, generation: generation)
            } catch {
                self.stateLock.lock()
                if self.cancelled {
                    self.state = .cancelled
                } else if self.modelDecodeGeneration == generation {
                    self.state = .failed(String(describing: error))
                    self.lastError = String(describing: error)
                }
                self.decodeJob = nil
                self.stateLock.unlock()
            }
        }
    }

    private func prepareCatalog() {
        stateLock.lock()
        let initialIndex = selectedModelIndex
        let initialMotion = evaluator
        let generation = modelDecodeGeneration
        let indices = Array(Set(modelBindings)).sorted()
        let jobs = indices.map { index in
            let source = modelSources[index]
            return RigDecodeJob(helperURL: helperURL, bundleURL: bundleURL,
                                baseURL: source.fileURL, poseURL: nil,
                                overridesURL: source.overridesURL)
        }
        catalogDecodeJobs = jobs
        let wasCancelled = cancelled
        stateLock.unlock()
        if wasCancelled {
            for job in jobs { job.cancel() }
            return
        }
        let group = DispatchGroup()
        let batches = (indices.count + RigLimits.catalogDecodeWorkers - 1) / RigLimits.catalogDecodeWorkers
        let deadline = DispatchTime.now() + TimeInterval(RigLimits.decodeSeconds * batches)
        for (index, job) in zip(indices, jobs) {
            group.enter()
            Self.catalogDecodeQueue.addOperation { [self] in
                defer { group.leave() }
                stateLock.lock()
                let shouldRun: Bool
                if case .decoding = state { shouldRun = !cancelled } else { shouldRun = false }
                stateLock.unlock()
                guard shouldRun else { return }
                do {
                    let decoded = try job.run()
                    guard decoded.document.base.canvas.w == canvasWidth,
                          decoded.document.base.canvas.h == canvasHeight else {
                        throw RigNativeError.invalid("catalog model canvas differs from its pack")
                    }
                    let source = modelSources[index]
                    let motion = index == initialIndex ? initialMotion
                        : try RigMotionEvaluator(data: Self.readMotion(source.motionURL),
                                                 basePoseID: source.id, alternatePoseID: nil)
                    let geometry = try RigAnchorGeometry(scene: decoded)
                    try geometry.prepareMotion(motion)
                    let displayBounds = geometry.displayBounds(motion: motion)
                    stateLock.lock()
                    guard !cancelled, case .decoding = state else {
                        stateLock.unlock()
                        return
                    }
                    guard decoded.rgba.count <= RigLimits.catalogDecodedBytes - preparedModelRGBABytes else {
                        stateLock.unlock()
                        throw RigNativeError.invalid("catalog decoded RGBA exceeds its byte quota")
                    }
                    preparedModels[index] = RigPreparedModel(
                        scene: decoded, motion: motion, displayBounds: displayBounds, geometry: geometry
                    )
                    preparedModelRGBABytes += decoded.rgba.count
                    stateLock.unlock()
                } catch {
                    failCatalogPreparation(error)
                }
            }
        }
        if group.wait(timeout: deadline) == .timedOut {
            failCatalogPreparation(RigNativeError.timeout)
            return
        }
        stateLock.lock()
        catalogDecodeJobs.removeAll()
        guard !cancelled, case .decoding = state,
              modelDecodeGeneration == generation,
              preparedModels.count == indices.count,
              let initial = preparedModels[initialIndex] else {
            stateLock.unlock()
            return
        }
        stateLock.unlock()
        finishDecode(initial.scene, preparedGeometry: initial.geometry, generation: generation)
    }

    private func failCatalogPreparation(_ error: Error) {
        stateLock.lock()
        if !cancelled, case .decoding = state {
            let message = String(describing: error)
            state = .failed(message)
            lastError = message
            cachedSpeechAnchor = nil
            preparedModels.removeAll()
            preparedModelRGBABytes = 0
        }
        let jobs = catalogDecodeJobs
        stateLock.unlock()
        for job in jobs { job.cancel() }
    }

    private func finishDecode(_ decoded: RigDecodedScene, preparedGeometry: RigAnchorGeometry?,
                              generation: UInt64) {
        stateLock.lock()
        guard !cancelled, modelDecodeGeneration == generation,
              pendingModelIndex == nil else {
            if cancelled { state = .cancelled }
            decodeJob = nil
            stateLock.unlock()
            return
        }
        stateLock.unlock()
        do {
            let geometry: RigAnchorGeometry
            let displayBounds: RigBounds?
            if let preparedGeometry {
                // Catalog preparation already built this model's geometry; the
                // display envelope spans every prepared catalog model.
                geometry = preparedGeometry
                var extent = RigAnchorExtent()
                var complete = true
                for model in preparedModels.values {
                    guard let bounds = model.displayBounds else {
                        complete = false
                        break
                    }
                    extent.include(bounds.x0, bounds.y0)
                    extent.include(bounds.x1, bounds.y1)
                }
                displayBounds = complete ? extent.bounds : nil
            } else {
                geometry = try RigAnchorGeometry(scene: decoded)
                try geometry.prepareMotion(evaluator)
                displayBounds = geometry.displayBounds(motion: evaluator)
            }
            let next = try RigMetalRenderer(device: device, scene: decoded)
            try next.prepareMotion(evaluator)
            _ = try next.render(parameters: RigParameters(), time: 0, neutral: true, poseMix: 0)
            _ = try next.sampleHit(modelX: Double(canvasWidth) / 2, modelY: Double(canvasHeight) / 2)
            stateLock.lock()
            if cancelled || modelDecodeGeneration != generation {
                if cancelled { state = .cancelled }
            } else {
                scene = decoded
                anchorGeometry = geometry
                preparedDisplayBounds = displayBounds
                renderer = next
                cachedSpeechAnchor = nil
                state = .ready
                hitsDisabled = false
                lastError = nil
            }
            decodeJob = nil
            stateLock.unlock()
        } catch {
            stateLock.lock()
            if modelDecodeGeneration == generation {
                state = .failed(String(describing: error))
                cachedSpeechAnchor = nil
                lastError = String(describing: error)
            }
            decodeJob = nil
            stateLock.unlock()
        }
    }

    private func startPendingModelPreparation(index: Int, generation: UInt64) {
        stateLock.lock()
        guard !cancelled, independentModels, modelDecodeGeneration == generation,
              pendingModelIndex == index else {
            stateLock.unlock()
            return
        }
        guard let prepared = preparedModels[index] else {
            stateLock.unlock()
            markFailure("requested catalog model was not prepared")
            return
        }
        stateLock.unlock()
        decodeQueue.async { [weak self] in
            self?.finishModelPreparation(prepared, index: index, generation: generation)
        }
    }

    private func finishModelPreparation(_ prepared: RigPreparedModel, index: Int,
                                        generation: UInt64) {
        stateLock.lock()
        let shouldPrepare = !cancelled && modelDecodeGeneration == generation && pendingModelIndex == index
        stateLock.unlock()
        guard shouldPrepare else { return }
        do {
            let nextRenderer = try RigMetalRenderer(device: device, scene: prepared.scene)
            try nextRenderer.prepareMotion(prepared.motion)
            _ = try nextRenderer.render(parameters: RigParameters(), time: 0,
                                        neutral: true, poseMix: 0)
            _ = try nextRenderer.sampleHit(modelX: Double(canvasWidth) / 2,
                                           modelY: Double(canvasHeight) / 2)
            stateLock.lock()
            guard !cancelled, modelDecodeGeneration == generation,
                  pendingModelIndex == index else {
                stateLock.unlock()
                return
            }
            scene = prepared.scene
            anchorGeometry = prepared.geometry
            renderer = nextRenderer
            prepared.motion.restartModelClock()
            evaluator = prepared.motion
            selectedModelIndex = index
            pendingModelIndex = nil
            rejectedModelIndex = nil
            state = .ready
            lastError = nil
            intentSerial &+= 1
            surfaceReady = false
            surfaceReadyAt = 0
            hitsDisabled = true
            cachedSpeechAnchor = nil
            latestEvaluated = nil
            animatedIntent = nil
            stateLock.unlock()
        } catch {
            stateLock.lock()
            if !cancelled && modelDecodeGeneration == generation
                && pendingModelIndex == index {
                let message = String(describing: error)
                pendingModelIndex = nil
                rejectedModelIndex = index
                lastError = message
                inputGeneration &+= 1
                intentSerial &+= 1
                surfaceReady = false
                hitsDisabled = true
                cachedSpeechAnchor = nil
                stopDisplayDriverLocked()
                state = .failed(message)
            }
            stateLock.unlock()
        }
    }

    private func ensureUsableLocked() throws {
        switch state {
        case .ready:
            if cancelled { throw RigNativeError.cancelled }
            if let lastError { throw RigNativeError.unavailable(lastError) }
        case .decoding:
            throw RigNativeError.unavailable("rig preparation is still decoding")
        case .cancelled:
            throw RigNativeError.cancelled
        case .failed(let message):
            throw RigNativeError.unavailable(message)
        }
    }

    private func markFailure(_ message: String) {
        stateLock.lock()
        let shouldRecover = markFailureLocked(message)
        stateLock.unlock()
        if shouldRecover { recover() }
    }

    /// Returns whether the caller should start bounded recovery after unlocking.
    private func markFailureLocked(_ message: String) -> Bool {
        inputGeneration &+= 1
        stopDisplayDriverLocked()
        renderPending = false
        intentSerial &+= 1
        hitsDisabled = true
        surfaceReady = false
        cachedSpeechAnchor = nil
        lastError = message
        state = .failed(message)
        return active && !cancelled && !recoveryUsed && scene != nil
    }

    private static func checkedDrawableSize(_ width: Double, _ height: Double, _ scale: Double) throws -> CGSize {
        let pixelsWide = max(1, (width * scale).rounded())
        let pixelsHigh = max(1, (height * scale).rounded())
        guard width.isFinite, height.isFinite, scale.isFinite,
              width > 0, height > 0, scale >= 1, scale <= 4,
              pixelsWide.isFinite, pixelsHigh.isFinite,
              pixelsWide <= Double(RigLimits.drawableDimension),
              pixelsHigh <= Double(RigLimits.drawableDimension),
              pixelsWide <= Double(RigLimits.drawablePixels) / pixelsHigh else {
            throw RigNativeError.invalid("rig viewport exceeds drawable limits")
        }
        return CGSize(width: pixelsWide, height: pixelsHigh)
    }

    private static func readMotion(_ url: URL) throws -> Data {
        let values = try url.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey])
        guard values.isRegularFile == true, values.isSymbolicLink != true,
              let size = values.fileSize, size <= 64 * 1024 else {
            throw RigNativeError.invalid("motion source is not a bounded regular file")
        }
        return try Data(contentsOf: url, options: [.mappedIfSafe])
    }

    private static func trustedRuntime() throws -> (helper: URL, bundle: URL) {
        let appURL = Bundle.main.bundleURL
        let helper: URL
        let bundle: URL
        if appURL.pathExtension == "app" {
            helper = appURL.appendingPathComponent("Contents/MacOS/rig-decode-worker")
            bundle = appURL.appendingPathComponent("Contents/Resources/rig/decoder.js")
        } else {
            // Source builds resolve only beside this already-loaded trusted
            // library. Packs and process environment cannot redirect decoding.
            guard let image = class_getImageName(RigSurfaceView.self) else {
                throw RigNativeError.unavailable("cannot locate the trusted rig library")
            }
            let root = URL(fileURLWithPath: String(cString: image)).deletingLastPathComponent()
            helper = root.appendingPathComponent("rig-decode-worker")
            bundle = root.appendingPathComponent("Resources/rig/decoder.js")
        }
        let helperValues = try? helper.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey])
        let bundleValues = try? bundle.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey])
        guard helperValues?.isRegularFile == true, helperValues?.isSymbolicLink != true,
              bundleValues?.isRegularFile == true, bundleValues?.isSymbolicLink != true,
              FileManager.default.isExecutableFile(atPath: helper.path) else {
            throw RigNativeError.unavailable("trusted rig decoder resources are incomplete")
        }
        return (helper, bundle)
    }

    private static func encodePNG(_ frame: RigFrame) throws -> Data {
        guard frame.width > 0, frame.height > 0,
              frame.rgba.count == frame.width * frame.height * 4 else {
            throw RigNativeError.invalid("preview frame dimensions are inconsistent")
        }
        let mutable = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(mutable, "public.png" as CFString, 1, nil),
              let colorSpace = CGColorSpace(name: CGColorSpace.sRGB),
              let provider = CGDataProvider(data: frame.rgba as CFData),
              let image = CGImage(width: frame.width, height: frame.height, bitsPerComponent: 8,
                                  bitsPerPixel: 32, bytesPerRow: frame.width * 4, space: colorSpace,
                                  bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue
                                      | CGImageByteOrderInfo.order32Big.rawValue), provider: provider,
                                  decode: nil, shouldInterpolate: true, intent: .defaultIntent) else {
            throw RigNativeError.unavailable("ImageIO PNG encoder is unavailable")
        }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else {
            throw RigNativeError.unavailable("ImageIO PNG encoding failed")
        }
        return mutable as Data
    }
}

struct HerdrRigTokenSnapshot {
    let operationID: String
    let referenceID: String
    let referenceRevision: UInt64
    let contentDigest: String
    let backendEpoch: UInt64
}

private func assetTokenString(_ pointer: UnsafePointer<CChar>?) -> String? {
    guard let pointer else { return nil }
    return String(validatingUTF8: pointer)
}
