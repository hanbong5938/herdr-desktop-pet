import Foundation
import Metal
import QuartzCore
import simd

/// A real Metal renderer for the decoded Anime2.5D rig.
///
/// Mesh deformation remains owned by RigDeformer.  This type owns only GPU
/// resources and the render passes that consume the deformed mesh state.
final class RigMetalRenderer {
    #if RIG_PROBE_FAULTS
    private static let maxFramesInFlight = 3
    #else
    private static let maxFramesInFlight = 1
    #endif

    private struct VertexUniforms {
        var resolution: SIMD2<Float>
    }

    private struct FragmentUniforms {
        var alpha: Float
        var cut: Float
    }

    private struct CompositeUniforms {
        var weight: Float
    }

    private final class MeshResources {
        let source: RigMesh
        let texture: MTLTexture
        let uvBuffer: MTLBuffer
        let indexBuffer: MTLBuffer
        let positionBuffers: [MTLBuffer]
        #if RIG_PROBE_FAULTS
        let hitMeshes: [RigRasterMesh]
        #endif
        let indexCount: Int
        let isEyeWhite: Bool
        let isIris: Bool

        let eyeBit: UInt32
        var irisReadMask: UInt32 = 0
        init(device: MTLDevice, source: RigMesh, rgba: Data) throws {
            self.source = source
            self.indexCount = source.indices.count
            self.isEyeWhite = source.layer.name.hasPrefix("eyewhite")
            self.isIris = source.layer.name.hasPrefix("irides")
            switch source.layer.side {
            case "L": self.eyeBit = 1
            case "R": self.eyeBit = 2
            default: self.eyeBit = 4
            }

            guard source.positions.count == source.uvs.count else {
                throw RigNativeError.invalid("mesh \(source.layer.name) position/UV count mismatch")
            }
            guard !source.positions.isEmpty, !source.indices.isEmpty else {
                throw RigNativeError.invalid("mesh \(source.layer.name) has no drawable vertices")
            }
            for index in source.indices where Int(index) >= source.positions.count {
                throw RigNativeError.invalid("mesh \(source.layer.name) index out of bounds")
            }

            let image = source.layer.img
            guard image.width > 0, image.height > 0 else {
                throw RigNativeError.invalid("mesh \(source.layer.name) has invalid image dimensions")
            }
            guard image.width <= Int.max / image.height else {
                throw RigNativeError.invalid("mesh \(source.layer.name) image dimensions overflow")
            }
            let pixelCount = image.width * image.height
            guard pixelCount <= Int.max / 4 else {
                throw RigNativeError.invalid("mesh \(source.layer.name) image byte count overflow")
            }
            let byteCount = pixelCount * 4
            guard image.length == byteCount else {
                throw RigNativeError.invalid("mesh \(source.layer.name) image length mismatch")
            }
            guard image.offset >= 0, image.offset <= rgba.count,
                  image.length <= rgba.count - image.offset else {
                throw RigNativeError.invalid("mesh \(source.layer.name) image range out of bounds")
            }

            let textureDescriptor = MTLTextureDescriptor.texture2DDescriptor(
                pixelFormat: .rgba8Unorm,
                width: image.width,
                height: image.height,
                mipmapped: false
            )
            textureDescriptor.usage = [.shaderRead]
            // CPU upload is a one-time initialization operation. Shared
            // storage keeps this public replace(region:) path legal on every
            // Metal device; all mutable frame data remains in shared buffers.
            textureDescriptor.storageMode = .shared
            guard let texture = device.makeTexture(descriptor: textureDescriptor) else {
                throw RigNativeError.unavailable("Metal texture allocation failed for \(source.layer.name)")
            }
            texture.label = "Rig layer \(source.layer.name) texture"
            self.texture = texture

            // WebGL's UNPACK_PREMULTIPLY_ALPHA_WEBGL premultiplies before
            // filtering.  Do the same conversion once while the source image
            // is uploaded, never in the per-fragment shader.
            var premultiplied = [UInt8](repeating: 0, count: byteCount)
            rgba.withUnsafeBytes { raw in
                guard let baseAddress = raw.baseAddress else { return }
                let sourceBytes = baseAddress.advanced(by: image.offset).assumingMemoryBound(to: UInt8.self)
                for index in stride(from: 0, to: byteCount, by: 4) {
                    let alpha = Int(sourceBytes[index + 3])
                    premultiplied[index + 3] = UInt8(alpha)
                    premultiplied[index] = UInt8((Int(sourceBytes[index]) * alpha + 127) / 255)
                    premultiplied[index + 1] = UInt8((Int(sourceBytes[index + 1]) * alpha + 127) / 255)
                    premultiplied[index + 2] = UInt8((Int(sourceBytes[index + 2]) * alpha + 127) / 255)
                }
            }
            premultiplied.withUnsafeBytes { raw in
                texture.replace(
                    region: MTLRegionMake2D(0, 0, image.width, image.height),
                    mipmapLevel: 0,
                    withBytes: raw.baseAddress!,
                    bytesPerRow: image.width * 4
                )
            }

            let uvBytes = source.uvs.count * MemoryLayout<SIMD2<Float>>.stride
            let indexBytes = source.indices.count * MemoryLayout<UInt16>.stride
            guard let uvBuffer = device.makeBuffer(length: uvBytes, options: .storageModeShared),
                  let indexBuffer = device.makeBuffer(length: indexBytes, options: .storageModeShared) else {
                throw RigNativeError.unavailable("Metal mesh buffer allocation failed for \(source.layer.name)")
            }
            uvBuffer.label = "Rig layer \(source.layer.name) UVs"
            indexBuffer.label = "Rig layer \(source.layer.name) indices"
            source.uvs.withUnsafeBytes { raw in
                uvBuffer.contents().copyMemory(from: raw.baseAddress!, byteCount: uvBytes)
            }
            source.indices.withUnsafeBytes { raw in
                indexBuffer.contents().copyMemory(from: raw.baseAddress!, byteCount: indexBytes)
            }
            self.uvBuffer = uvBuffer
            self.indexBuffer = indexBuffer

            let positionBytes = source.positions.count * MemoryLayout<SIMD2<Float>>.stride
            var positionBuffers: [MTLBuffer] = []
            positionBuffers.reserveCapacity(RigMetalRenderer.maxFramesInFlight)
            for frame in 0..<RigMetalRenderer.maxFramesInFlight {
                guard let positionBuffer = device.makeBuffer(length: positionBytes, options: .storageModeShared) else {
                    throw RigNativeError.unavailable("Metal position buffer allocation failed for \(source.layer.name) frame \(frame)")
                }
                positionBuffer.label = "Rig layer \(source.layer.name) positions \(frame)"
                positionBuffers.append(positionBuffer)
            }
            self.positionBuffers = positionBuffers
            #if RIG_PROBE_FAULTS
            self.hitMeshes = positionBuffers.map {
                RigRasterMesh(source: source, positions: UnsafeBufferPointer(
                    start: $0.contents().assumingMemoryBound(to: SIMD2<Float>.self),
                    count: source.positions.count))
            }
            #endif
        }
    }

    private final class ModelResources {
        let rig: RigDefinition
        let deformer: RigDeformer
        let meshes: [MeshResources]
        #if RIG_PROBE_FAULTS
        let hitModels: [RigRasterModel]
        #endif

        init(device: MTLDevice, rig: RigDefinition, physics: RigHairPhysicsConfig?, rgba: Data,
             remainingVertices: inout Int) throws {
            self.rig = rig
            self.deformer = try RigDeformer(rig: rig, physics: physics)
            guard !deformer.meshes.isEmpty else {
                throw RigNativeError.invalid("rig contains no drawable meshes")
            }
            for mesh in deformer.meshes {
                guard mesh.positions.count <= remainingVertices else {
                    throw RigNativeError.invalid("combined mesh vertex quota exceeded")
                }
                remainingVertices -= mesh.positions.count
            }

            var eyeWhiteBits: UInt32 = 0
            for mesh in deformer.meshes where mesh.layer.name.hasPrefix("eyewhite") {
                switch mesh.layer.side {
                case "L": eyeWhiteBits |= 1
                case "R": eyeWhiteBits |= 2
                default: eyeWhiteBits |= 4
                }
            }

            var resources: [MeshResources] = []
            resources.reserveCapacity(deformer.meshes.count)
            for mesh in deformer.meshes {
                resources.append(try MeshResources(device: device, source: mesh, rgba: rgba))
            }
            for resource in resources where resource.isIris {
                let allowed: UInt32 = resource.eyeBit == 4 ? 4 : resource.eyeBit | 4
                if allowed & eyeWhiteBits != 0 { resource.irisReadMask = allowed }
            }
            // RigDeformer exposes stable render order.  The secondary index
            // keeps authored order deterministic for equal render slots.
            let ordered = resources.enumerated().sorted {
                if $0.element.source.renderSlot == $1.element.source.renderSlot {
                    return $0.offset < $1.offset
                }
                return $0.element.source.renderSlot < $1.element.source.renderSlot
            }.map(\.element)
            self.meshes = ordered
            #if RIG_PROBE_FAULTS
            self.hitModels = (0..<RigMetalRenderer.maxFramesInFlight).map { slot in
                RigRasterModel(rig: rig, meshes: ordered.map { $0.hitMeshes[slot] })
            }
            #endif
        }
    }

    private final class GroupTarget {
        let color: MTLTexture
        let stencil: MTLTexture

        init(device: MTLDevice, width: Int, height: Int, label: String) throws {
            let colorDescriptor = MTLTextureDescriptor.texture2DDescriptor(
                pixelFormat: .rgba8Unorm,
                width: width,
                height: height,
                mipmapped: false
            )
            colorDescriptor.usage = [.renderTarget, .shaderRead]
            colorDescriptor.storageMode = .private
            guard let color = device.makeTexture(descriptor: colorDescriptor) else {
                throw RigNativeError.unavailable("Metal group color target allocation failed")
            }
            color.label = "\(label) color"

            let stencilDescriptor = MTLTextureDescriptor.texture2DDescriptor(
                pixelFormat: .stencil8,
                width: width,
                height: height,
                mipmapped: false
            )
            stencilDescriptor.usage = [.renderTarget]
            stencilDescriptor.storageMode = .private
            guard let stencil = device.makeTexture(descriptor: stencilDescriptor) else {
                throw RigNativeError.unavailable("Metal group stencil target allocation failed")
            }
            stencil.label = "\(label) stencil"
            self.color = color
            self.stencil = stencil
        }
    }

    private let device: MTLDevice
    private let commandQueue: MTLCommandQueue
    private let sampler: MTLSamplerState
    private let library: MTLLibrary
    private let base: ModelResources
    private let pose: ModelResources?
    private let sourcePixels: Data
    private var latestPoseMix: Float = 0
    private let canvasWidth: Int
    private let canvasHeight: Int
    private let groupTargets: [GroupTarget]
    private let noStencilState: MTLDepthStencilState
    private let eyeWhiteStencilStates: [MTLDepthStencilState]
    private let irisStencilStates: [MTLDepthStencilState]
    private var maskPipeline: MTLRenderPipelineState?
    private var layerPipeline: MTLRenderPipelineState?
    private var compositePipelines: [UInt: MTLRenderPipelineState] = [:]
    #if RIG_PROBE_FAULTS
    private var probeInvalidPipeline = false

    // Deliberately reaches the real Metal error path; absent from shipping builds.
    func probeFailNextCompositePipeline() {
        lock.lock()
        defer { lock.unlock() }
        compositePipelines.removeAll()
        probeInvalidPipeline = true
    }
    #endif
    private var outputTexture: MTLTexture?
    private var outputReadback: MTLBuffer?
    private var captureReadback: MTLBuffer?
    private var captureReadbackCapacity = 0
    private var sampleReadback: MTLBuffer?
    private var latestTexture: MTLTexture?
    private var latestTextureWidth = 0
    private var latestTextureHeight = 0
    private var frameSlot = 0
    private let inFlight = DispatchSemaphore(value: RigMetalRenderer.maxFramesInFlight)
    private let lock = NSLock()
    private struct CompletedHitFrame {
        let slot: Int
        let mix: Float
        let width: Int
        let height: Int
        let timestamp: Double
        let number: UInt64
    }
    private let hitLock = NSLock()
    private var completedHitFrame: CompletedHitFrame?
    private var hitFrameNumber: UInt64 = 0
    private struct HitRequest {
        let x: Double
        let y: Double
        let threshold: Double
        let serial: UInt64
    }
    private var hitRequest: HitRequest?
    private var hitRequestSerial: UInt64 = 0
    private var cachedGPUHit: RigCachedHit?
    private var cachedRequestSerial: UInt64 = 0
    private var hitWorkScheduled = false
    private var nextHitWorkAt = 0.0
    private let hitQueue = DispatchQueue(label: "herdr.rig.hit", qos: .userInteractive)
    private var alphaReadbackPixels = 0

    init(device: MTLDevice, scene: RigDecodedScene) throws {
        guard scene.document.format == "herdr.rig.decoded", scene.document.version == 1 else {
            throw RigNativeError.invalid("unsupported decoded rig document")
        }
        let canvas = scene.document.base.canvas
        guard canvas.w > 0, canvas.h > 0,
              canvas.w <= RigLimits.canvasDimension, canvas.h <= RigLimits.canvasDimension,
              canvas.w <= RigLimits.canvasPixels / canvas.h else {
            throw RigNativeError.invalid("rig canvas quota exceeded")
        }
        guard scene.rgba.count <= RigLimits.outputBytes,
              scene.document.base.layers.count <= RigLimits.outputLayers,
              (scene.document.pose?.layers.count ?? 0) <= RigLimits.outputLayers else {
            throw RigNativeError.invalid("rig layer or RGBA quota exceeded")
        }
        if let pose = scene.document.pose,
           pose.canvas.w != scene.document.base.canvas.w || pose.canvas.h != scene.document.base.canvas.h {
            throw RigNativeError.invalid("base and pose canvases must have equal dimensions")
        }
        guard let commandQueue = device.makeCommandQueue() else {
            throw RigNativeError.unavailable("Metal command queue unavailable")
        }
        let library: MTLLibrary
        do {
            library = try device.makeLibrary(source: RigMetalShaders.source, options: nil)
        } catch {
            throw RigNativeError.unavailable("Metal shader compilation failed: \(error.localizedDescription)")
        }
        let samplerDescriptor = MTLSamplerDescriptor()
        samplerDescriptor.minFilter = .linear
        samplerDescriptor.magFilter = .linear
        samplerDescriptor.mipFilter = .notMipmapped
        samplerDescriptor.sAddressMode = .clampToEdge
        samplerDescriptor.tAddressMode = .clampToEdge
        guard let sampler = device.makeSamplerState(descriptor: samplerDescriptor) else {
            throw RigNativeError.unavailable("Metal sampler allocation failed")
        }

        self.device = device
        self.commandQueue = commandQueue
        self.library = library
        self.sampler = sampler
        self.canvasWidth = scene.document.base.canvas.w
        self.canvasHeight = scene.document.base.canvas.h
        self.sourcePixels = scene.rgba
        var remainingVertices = RigLimits.totalMeshVertices
        self.base = try ModelResources(device: device, rig: scene.document.base, physics: scene.document.physics,
                                       rgba: scene.rgba, remainingVertices: &remainingVertices)
        if let poseRig = scene.document.pose {
            self.pose = try ModelResources(device: device, rig: poseRig, physics: scene.document.physics,
                                           rgba: scene.rgba, remainingVertices: &remainingVertices)
        } else {
            self.pose = nil
        }
        var targets = [try GroupTarget(device: device, width: scene.document.base.canvas.w, height: scene.document.base.canvas.h, label: "Rig base target")]
        if scene.document.pose != nil {
            targets.append(try GroupTarget(device: device, width: scene.document.base.canvas.w, height: scene.document.base.canvas.h, label: "Rig pose target"))
        }
        self.groupTargets = targets
        self.noStencilState = try Self.makeDepthStencilState(device: device, compare: .always, readMask: 0xff, writeMask: 0, passOperation: .keep, label: "Rig no stencil")
        self.eyeWhiteStencilStates = try [1, 2, 4].map {
            try Self.makeDepthStencilState(
                device: device, compare: .always, readMask: 0xff, writeMask: $0,
                passOperation: .replace, label: "Rig eye-white stencil \($0)")
        }
        self.irisStencilStates = try [4, 5, 6].map {
            try Self.makeDepthStencilState(
                device: device, compare: .notEqual, readMask: $0, writeMask: 0,
                passOperation: .keep, label: "Rig iris stencil \($0)")
        }
    }

    func prepareMotion(_ evaluator: RigMotionEvaluator) throws {
        try evaluator.prepare(
            baseMeshes: base.deformer.meshes,
            poseMeshes: pose?.deformer.meshes,
            canvas: base.rig.canvas
        )
    }

    var metrics: [String: Int] {
        lock.lock()
        defer { lock.unlock() }
        let allModels = [base] + (pose.map { [$0] } ?? [])
        let meshCount = allModels.reduce(0) { $0 + $1.meshes.count }
        let textureCount = allModels.reduce(0) { $0 + $1.meshes.count } + groupTargets.count + (outputTexture == nil ? 0 : 1)
        let positionBufferCount = allModels.reduce(0) { count, model in
            count + model.meshes.reduce(0) { $0 + $1.positionBuffers.count }
        }
        let uvBufferCount = meshCount
        let indexBufferCount = meshCount
        let readbackBufferCount = (outputReadback == nil ? 0 : 1) + (captureReadback == nil ? 0 : 1) + (sampleReadback == nil ? 0 : 1)
        return [
            "canvasWidth": canvasWidth,
            "canvasHeight": canvasHeight,
            "meshCount": meshCount,
            "vertexCount": allModels.reduce(0) { count, model in count + model.meshes.reduce(0) { $0 + $1.source.positions.count } },
            "indexCount": allModels.reduce(0) { count, model in count + model.meshes.reduce(0) { $0 + $1.source.indices.count } },
            "deviceAllocatedBytes": device.currentAllocatedSize,
            "cpuSourcePixelBytes": sourcePixels.count,
            "baseResourceCount": base.meshes.count,
            "poseResourceCount": pose?.meshes.count ?? 0,
            "textureCount": textureCount,
            "layerTextureCount": meshCount,
            "positionBufferCount": positionBufferCount,
            "uvBufferCount": uvBufferCount,
            "alphaReadbackPixels": alphaReadbackPixels,
            "indexBufferCount": indexBufferCount,
            "readbackBufferCount": readbackBufferCount,
            "bufferCount": positionBufferCount + uvBufferCount + indexBufferCount + readbackBufferCount,
            "renderTargetCount": groupTargets.count + (outputTexture == nil ? 0 : 1),
            "stencilTargetCount": groupTargets.count,
            "renderbufferCount": groupTargets.count,
            "framebufferCount": 0,
            "pipelineCount": (layerPipeline == nil ? 0 : 1) + (maskPipeline == nil ? 0 : 1) + compositePipelines.count,
            "samplerCount": 1,
            "sampleReadbackBufferCount": sampleReadback == nil ? 0 : 1,
            "latestTextureCount": latestTexture == nil ? 0 : 1,
            "outgoingPoseResourceCount": 0,
            "inFlightCapacity": RigMetalRenderer.maxFramesInFlight
        ]
    }

    /// Samples alpha from the latest completed GPU image without copying the
    /// canvas.  Coordinates are model-space pixels with a top-left origin;
    /// Retina drawable dimensions are handled by the target-to-model scale.
    func sampleAlpha(modelX: Double, modelY: Double) throws -> Float {
        lock.lock()
        defer { lock.unlock() }
        return try sampleAlphaLocked(modelX: modelX, modelY: modelY)
    }

    private func sampleAlphaLocked(modelX: Double, modelY: Double) throws -> Float {
        guard let texture = latestTexture else { return 0 }
        guard let commandBuffer = commandQueue.makeCommandBuffer() else {
            throw RigNativeError.unavailable("Metal sample command buffer unavailable")
        }
        commandBuffer.label = "Rig single-pixel alpha sample"
        guard let readback = try encodeAlphaSample(commandBuffer: commandBuffer, texture: texture,
                                                   modelX: modelX, modelY: modelY) else { return 0 }
        try commitAndWait(commandBuffer)
        return readAlpha(readback)
    }

    private func encodeAlphaSample(commandBuffer: MTLCommandBuffer, texture: MTLTexture,
                                    modelX: Double, modelY: Double) throws -> MTLBuffer? {
        guard modelX.isFinite, modelY.isFinite, modelX >= 0, modelY >= 0,
              modelX < Double(canvasWidth), modelY < Double(canvasHeight) else { return nil }
        let mapping = RigCanvasMapping(canvasWidth: canvasWidth, canvasHeight: canvasHeight,
                                       drawableWidth: texture.width, drawableHeight: texture.height)
        let point = mapping.drawablePoint(modelX: modelX, modelY: modelY)
        let pixelX = min(texture.width - 1, max(0, Int(floor(point.x))))
        let pixelY = min(texture.height - 1, max(0, Int(floor(point.y))))
        let readback = try ensureSampleReadback()
        guard let blit = commandBuffer.makeBlitCommandEncoder() else {
            throw RigNativeError.unavailable("Metal sample blit encoder unavailable")
        }
        defer { blit.endEncoding() }
        blit.label = "Rig single-pixel alpha blit"
        blit.copy(from: texture, sourceSlice: 0, sourceLevel: 0,
                  sourceOrigin: MTLOrigin(x: pixelX, y: pixelY, z: 0),
                  sourceSize: MTLSize(width: 1, height: 1, depth: 1),
                  to: readback, destinationOffset: 0, destinationBytesPerRow: 256, destinationBytesPerImage: 256)
        alphaReadbackPixels += 1
        return readback
    }

    private func readAlpha(_ buffer: MTLBuffer?) -> Float {
        guard let buffer else { return 0 }
        return Float(buffer.contents().advanced(by: 3).assumingMemoryBound(to: UInt8.self).pointee) / 255
    }

    // Caller snapshots this result off the AppKit mouse callback path and tags
    // it with its frame, cursor sequence, viewport and renderer epochs.
    func sampleHit(modelX: Double, modelY: Double, minimumAlpha: Double = 0.1) throws -> RigHitSample {
        lock.lock()
        defer { lock.unlock() }
        return try sampleHitLocked(modelX: modelX, modelY: modelY, minimumAlpha: minimumAlpha)
    }

    private func sampleHitLocked(modelX: Double, modelY: Double, minimumAlpha: Double) throws -> RigHitSample {
        guard minimumAlpha.isFinite, (0...1).contains(minimumAlpha) else {
            throw RigNativeError.invalid("hit alpha threshold")
        }
        let alpha = try sampleAlphaLocked(modelX: modelX, modelY: modelY)
        return semanticSample(alpha: alpha, modelX: modelX, modelY: modelY, minimumAlpha: minimumAlpha)
    }

    private func semanticSample(alpha: Float, modelX: Double, modelY: Double, minimumAlpha: Double) -> RigHitSample {
        guard Double(alpha) > minimumAlpha else { return RigHitSample(alpha: alpha, semantic: nil) }
        let mapping = RigCanvasMapping(canvasWidth: canvasWidth, canvasHeight: canvasHeight,
                                       drawableWidth: latestTextureWidth, drawableHeight: latestTextureHeight)
        let point = mapping.sampledModelPoint(modelX: modelX, modelY: modelY)
        let x = point.x, y = point.y
        let baseWeight = pose == nil ? 1 : Double(1 - latestPoseMix)
        let poseWeight = Double(latestPoseMix)
        var result: RigSemanticHit?
        func consider(_ model: ModelResources, weight: Double) {
            guard weight > minimumAlpha,
                  let hit = rigSemanticHit(rig: model.rig, meshes: model.deformer.meshes, rgba: sourcePixels,
                    modelX: x, modelY: y, minimumAlpha: minimumAlpha / weight)
            else { return }
            let coverage = hit.coverage * weight
            if let previous = result, coverage < previous.coverage { return }
            result = RigSemanticHit(region: hit.region, layer: hit.layer, sourceX: hit.sourceX,
                                    sourceY: hit.sourceY, coverage: coverage, renderSlot: hit.renderSlot)
        }
        consider(base, weight: baseWeight)
        if let pose { consider(pose, weight: poseWeight) }
        return RigHitSample(alpha: alpha, semantic: result)
    }

    #if RIG_PROBE_FAULTS
    // Diagnostic CPU estimate, not an authoritative opacity decision: GPU
    // interpolation/UNorm quantization differs at threshold-adjacent pixels.
    func rasterHitEstimate(modelX: Double, modelY: Double, minimumAlpha: Double = 0.1,
                           maximumAge: Double = 0.08) -> RigCachedHit? {
        guard minimumAlpha.isFinite, (0...1).contains(minimumAlpha),
              maximumAge.isFinite, maximumAge > 0, maximumAge <= 1,
              hitLock.try() else { return nil }
        defer { hitLock.unlock() }
        guard let frame = completedHitFrame else { return nil }
        let age = ProcessInfo.processInfo.systemUptime - frame.timestamp
        guard age >= 0, age <= maximumAge else { return nil }
        let hit = rigRasterHit(base: base.hitModels[frame.slot], pose: pose?.hitModels[frame.slot],
                               rgba: sourcePixels, poseMix: frame.mix, modelX: modelX, modelY: modelY,
                               drawableWidth: frame.width, drawableHeight: frame.height, minimumAlpha: minimumAlpha)
        return RigCachedHit(sample: hit, frameNumber: frame.number, capturedAt: frame.timestamp,
                            drawableWidth: frame.width, drawableHeight: frame.height)
    }
    #endif

    // At most one coalesced request. The pointer path never waits for the
    // rendering lock/GPU, and never substitutes a CPU opacity estimate.
    func cachedHit(modelX: Double, modelY: Double, minimumAlpha: Double = 0.1) -> RigCachedHit? {
        guard modelX.isFinite, modelY.isFinite, minimumAlpha.isFinite,
              (0...1).contains(minimumAlpha), hitLock.try() else { return nil }
        defer { hitLock.unlock() }
        if hitRequest?.x != modelX || hitRequest?.y != modelY || hitRequest?.threshold != minimumAlpha {
            hitRequestSerial &+= 1
            hitRequest = HitRequest(x: modelX, y: modelY, threshold: minimumAlpha, serial: hitRequestSerial)
            cachedGPUHit = nil
        }
        guard let frame = completedHitFrame,
              ProcessInfo.processInfo.systemUptime - frame.timestamp <= 0.08 else { return nil }
        if let cachedGPUHit, cachedRequestSerial == hitRequestSerial, cachedGPUHit.frameNumber == frame.number {
            return cachedGPUHit
        }
        scheduleHitWorkLocked()
        return nil
    }

    private func scheduleHitWorkLocked() {
        guard !hitWorkScheduled else { return }
        hitWorkScheduled = true
        let delay = max(0, nextHitWorkAt - ProcessInfo.processInfo.systemUptime)
        hitQueue.asyncAfter(deadline: .now() + delay) { [weak self] in self?.sampleRequestedHit() }
    }

    private func sampleRequestedHit() {
        lock.lock()
        defer { lock.unlock() }
        hitLock.lock()
        guard let request = hitRequest, let frame = completedHitFrame,
              ProcessInfo.processInfo.systemUptime - frame.timestamp <= 0.08 else {
            hitWorkScheduled = false
            hitLock.unlock()
            return
        }
        if cachedRequestSerial == request.serial, cachedGPUHit?.frameNumber == frame.number {
            hitWorkScheduled = false
            hitLock.unlock()
            return
        }
        nextHitWorkAt = ProcessInfo.processInfo.systemUptime + 1.0 / 120.0
        hitLock.unlock()
        let sampled = try? sampleHitLocked(modelX: request.x, modelY: request.y, minimumAlpha: request.threshold)
        hitLock.lock()
        if hitRequest?.serial == request.serial, completedHitFrame?.number == frame.number, let sampled {
            cachedRequestSerial = request.serial
            cachedGPUHit = RigCachedHit(sample: sampled, frameNumber: frame.number, capturedAt: frame.timestamp,
                                        drawableWidth: frame.width, drawableHeight: frame.height)
        }
        hitWorkScheduled = false
        if hitRequest?.serial != request.serial { scheduleHitWorkLocked() }
        hitLock.unlock()
    }

    private func requestedHit() -> HitRequest? {
        hitLock.lock()
        defer { hitLock.unlock() }
        return hitRequest
    }

    private func publishHitFrame(slot: Int, mix: Float, width: Int, height: Int,
                                 request: HitRequest?, alpha: Float,
                                 scheduleRequest: Bool = true) {
        let sampled = request.map {
            semanticSample(alpha: alpha, modelX: $0.x, modelY: $0.y, minimumAlpha: $0.threshold)
        }
        hitLock.lock()
        defer { hitLock.unlock() }
        hitFrameNumber &+= 1
        let now = ProcessInfo.processInfo.systemUptime
        completedHitFrame = CompletedHitFrame(slot: slot, mix: mix, width: width, height: height,
                                              timestamp: now, number: hitFrameNumber)
        cachedGPUHit = nil
        if let request, request.serial == hitRequest?.serial, let sampled {
            cachedRequestSerial = request.serial
            cachedGPUHit = RigCachedHit(sample: sampled, frameNumber: hitFrameNumber, capturedAt: now,
                                        drawableWidth: width, drawableHeight: height)
        } else if scheduleRequest, hitRequest != nil {
            scheduleHitWorkLocked()
        }
    }

    private func invalidateHitFrame() {
        hitLock.lock()
        completedHitFrame = nil
        cachedGPUHit = nil
        hitLock.unlock()
    }

    private func ensureSampleReadback() throws -> MTLBuffer {
        if let sampleReadback { return sampleReadback }
        // A 256-byte row stride satisfies Metal's blit alignment requirements
        // while keeping this hit path bounded to one reusable pixel buffer.
        guard let buffer = device.makeBuffer(length: 256, options: .storageModeShared) else {
            throw RigNativeError.unavailable("Metal sample readback allocation failed")
        }
        buffer.label = "Rig single-pixel readback"
        sampleReadback = buffer
        return buffer
    }

    private func markLatestTexture(_ texture: MTLTexture) {
        latestTexture = texture
        latestTextureWidth = texture.width
        latestTextureHeight = texture.height
    }
    private func alignedRowBytes(width: Int) throws -> Int {
        guard width > 0, width <= (Int.max - 255) / 4 else {
            throw RigNativeError.invalid("readback width overflow")
        }
        let raw = width * 4
        return ((raw + 255) / 256) * 256
    }
    private func alignedReadbackSize(width: Int, height: Int) throws -> (rowBytes: Int, length: Int) {
        guard height > 0 else { throw RigNativeError.invalid("readback height is invalid") }
        let rowBytes = try alignedRowBytes(width: width)
        guard rowBytes <= Int.max / height else {
            throw RigNativeError.invalid("readback byte count overflow")
        }
        return (rowBytes, rowBytes * height)
    }

    private func captureData(buffer: MTLBuffer, width: Int, height: Int, pixelFormat: MTLPixelFormat, bytesPerRow: Int) -> Data {
        let byteCount = width * height * 4
        var data = Data(count: byteCount)
        data.withUnsafeMutableBytes { destinationRaw in
            guard let destination = destinationRaw.baseAddress?.assumingMemoryBound(to: UInt8.self) else { return }
            let source = buffer.contents().assumingMemoryBound(to: UInt8.self)
            for row in 0..<height {
                destination.advanced(by: row * width * 4).update(from: source.advanced(by: row * bytesPerRow), count: width * 4)
            }
        }
        guard pixelFormat == .bgra8Unorm || pixelFormat == .bgra8Unorm_srgb else {
            return data
        }
        data.withUnsafeMutableBytes { raw in
            guard let bytes = raw.baseAddress?.assumingMemoryBound(to: UInt8.self) else { return }
            for index in stride(from: 0, to: byteCount, by: 4) {
                let blue = bytes[index]
                bytes[index] = bytes[index + 2]
                bytes[index + 2] = blue
            }
        }
        return data
    }

    /// Paints a bounded 16px hit grid over the completed offscreen capture.
    /// Alpha comes from the captured RGBA frame; semantic regions come from
    /// the same evaluated, deformed meshes used by the completed hit snapshot.
    /// No diagnostic CPU raster or per-point GPU readback is used here.
    private func paintHitOverlay(rgba: inout Data, width: Int, height: Int) throws {
        guard width > 0, height > 0, width <= Int.max / height else {
            throw RigNativeError.invalid("hit overlay dimensions overflow")
        }
        let pixelCount = width * height
        guard pixelCount <= Int.max / 4, rgba.count == pixelCount * 4 else {
            throw RigNativeError.invalid("hit overlay capture dimensions are inconsistent")
        }

        let minimumAlpha = 7.5 / 255.0
        rgba.withUnsafeMutableBytes { (raw: UnsafeMutableRawBufferPointer) -> Void in
            guard let pixels = raw.baseAddress?.assumingMemoryBound(to: UInt8.self) else { return }
            for top in stride(from: 0, to: height, by: 16) {
                for left in stride(from: 0, to: width, by: 16) {
                    let center = (top * width + left) * 4
                    let alphaByte = pixels[center + 3]
                    guard alphaByte >= 8 else { continue }
                    let sample = semanticSample(alpha: Float(alphaByte) / 255.0,
                                                modelX: Double(left) + 0.5,
                                                modelY: Double(top) + 0.5,
                                                minimumAlpha: minimumAlpha)
                    let tint: (UInt32, UInt32, UInt32)
                    if sample.semantic?.region == "head" {
                        tint = (255, 64, 64)
                    } else if sample.semantic?.region == "body" {
                        tint = (64, 224, 96)
                    } else {
                        tint = (64, 144, 255)
                    }
                    let x0 = max(0, left - 1)
                    let x1 = min(width - 1, left + 1)
                    let y0 = max(0, top - 1)
                    let y1 = min(height - 1, top + 1)
                    for markY in y0...y1 {
                        for markX in x0...x1 {
                            let index = (markY * width + markX) * 4
                            let alpha = UInt32(pixels[index + 3])
                            guard alpha >= 8 else { continue }
                            for channel in 0..<3 {
                                let tintChannel = channel == 0 ? tint.0 : (channel == 1 ? tint.1 : tint.2)
                                let tinted = alpha * tintChannel / 255
                                let original = UInt32(pixels[index + channel])
                                let weightedOriginal = original * 95
                                let weightedTint = tinted * 160
                                let blended = (weightedOriginal + weightedTint + 127) / 255
                                pixels[index + channel] = UInt8(min(UInt32(255), blended))
                            }
                        }
                    }
                }
            }
        }
    }

    func resetSimulation() {
        lock.lock()
        defer { lock.unlock() }
        base.deformer.resetSimulation()
        pose?.deformer.resetSimulation()
        latestTexture = nil
        latestPoseMix = 0
        invalidateHitFrame()
    }

    func render(parameters: RigParameters, time: Double, neutral: Bool, poseMix: Double,
                localMotion: RigLocalMotionFrame? = nil, hitOverlay: Bool = false) throws -> RigFrame {
        lock.lock()
        defer { lock.unlock() }
        var completed = false
        defer { if !completed { invalidateHitFrame() } }
        try validate(time: time, poseMix: poseMix)
        let mix = try normalizedPoseMix(poseMix)
        let slot = acquireFrameSlot()
        defer { inFlight.signal() }

        latestTexture = nil
        updateDeformers(parameters: parameters, time: time, neutral: neutral, localMotion: localMotion)
        let target = try ensureOutputTexture()
        guard let commandBuffer = commandQueue.makeCommandBuffer() else {
            throw RigNativeError.unavailable("Metal command buffer unavailable")
        }
        commandBuffer.label = "Rig offscreen render"
        try encodeScene(commandBuffer: commandBuffer, slot: slot, output: target, outputPixelFormat: target.pixelFormat, poseMix: mix)
        let readbackSize = try alignedReadbackSize(width: canvasWidth, height: canvasHeight)
        let readback = try ensureOutputReadback(length: readbackSize.length)
        try encodeReadback(commandBuffer: commandBuffer, texture: target, buffer: readback, width: canvasWidth, height: canvasHeight, bytesPerRow: readbackSize.rowBytes)
        let cursorRequest: HitRequest? = hitOverlay ? nil : requestedHit()
        let cursorBuffer = try cursorRequest.flatMap {
            try encodeAlphaSample(commandBuffer: commandBuffer, texture: target, modelX: $0.x, modelY: $0.y)
        }
        try commitAndWait(commandBuffer)
        markLatestTexture(target)
        latestPoseMix = mix
        publishHitFrame(slot: slot, mix: mix, width: target.width, height: target.height,
                        request: cursorRequest, alpha: readAlpha(cursorBuffer),
                        scheduleRequest: !hitOverlay)
        var data = captureData(buffer: readback, width: canvasWidth, height: canvasHeight,
                               pixelFormat: .rgba8Unorm, bytesPerRow: readbackSize.rowBytes)
        if hitOverlay {
            try paintHitOverlay(rgba: &data, width: canvasWidth, height: canvasHeight)
        }
        completed = true

        return RigFrame(width: canvasWidth, height: canvasHeight, rgba: data)
    }

    func draw(to drawable: CAMetalDrawable, parameters: RigParameters, time: Double, neutral: Bool,
              poseMix: Double, localMotion: RigLocalMotionFrame? = nil, capture: Bool) throws -> RigFrame? {
        lock.lock()
        defer { lock.unlock() }
        var completed = false
        defer { if !completed { invalidateHitFrame() } }
        try validate(time: time, poseMix: poseMix)
        let mix = try normalizedPoseMix(poseMix)
        let width = drawable.texture.width
        let height = drawable.texture.height
        guard width > 0, height > 0, width <= RigLimits.drawableDimension, height <= RigLimits.drawableDimension,
              width <= RigLimits.drawablePixels / height else {
            throw RigNativeError.invalid("drawable dimension or pixel quota exceeded")
        }
        guard drawable.texture.pixelFormat == .bgra8Unorm || drawable.texture.pixelFormat == .rgba8Unorm ||
                drawable.texture.pixelFormat == .bgra8Unorm_srgb || drawable.texture.pixelFormat == .rgba8Unorm_srgb else {
            throw RigNativeError.invalid("unsupported drawable pixel format")
        }
        let slot = acquireFrameSlot()
        defer { inFlight.signal() }

        latestTexture = nil
        updateDeformers(parameters: parameters, time: time, neutral: neutral, localMotion: localMotion)
        guard let commandBuffer = commandQueue.makeCommandBuffer() else {
            throw RigNativeError.unavailable("Metal command buffer unavailable")
        }
        commandBuffer.label = capture ? "Rig drawable render and capture" : "Rig drawable render"
        try encodeScene(commandBuffer: commandBuffer, slot: slot, output: drawable.texture, outputPixelFormat: drawable.texture.pixelFormat, poseMix: mix)

        let captureBuffer: MTLBuffer?
        let captureRowBytes: Int?
        if capture {
            let readbackSize = try alignedReadbackSize(width: width, height: height)
            captureRowBytes = readbackSize.rowBytes
            captureBuffer = try ensureCaptureReadback(length: readbackSize.length)
            try encodeReadback(commandBuffer: commandBuffer, texture: drawable.texture, buffer: captureBuffer!, width: width, height: height, bytesPerRow: readbackSize.rowBytes)
        } else {
            captureBuffer = nil
            captureRowBytes = nil
        }
        let cursorRequest = requestedHit()
        let cursorBuffer = try cursorRequest.flatMap {
            try encodeAlphaSample(commandBuffer: commandBuffer, texture: drawable.texture, modelX: $0.x, modelY: $0.y)
        }
        commandBuffer.present(drawable)
        try commitAndWait(commandBuffer)
        markLatestTexture(drawable.texture)
        latestPoseMix = mix
        publishHitFrame(slot: slot, mix: mix, width: width, height: height,
                        request: cursorRequest, alpha: readAlpha(cursorBuffer))
        completed = true
        guard let captureBuffer, let captureRowBytes else { return nil }
        return RigFrame(width: width, height: height, rgba: captureData(buffer: captureBuffer, width: width, height: height, pixelFormat: drawable.texture.pixelFormat, bytesPerRow: captureRowBytes))
    }

    private func validate(time: Double, poseMix: Double) throws {
        guard time.isFinite else { throw RigNativeError.invalid("render time is not finite") }
        guard poseMix.isFinite else { throw RigNativeError.invalid("pose mix is not finite") }
        guard canvasWidth <= Int.max / canvasHeight, canvasWidth * canvasHeight <= Int.max / 4 else {
            throw RigNativeError.invalid("rig canvas byte count overflow")
        }
    }

    private func normalizedPoseMix(_ poseMix: Double) throws -> Float {
        if pose == nil {
            guard abs(poseMix) < 1e-9 else {
                throw RigNativeError.invalid("pose mix requested without a pose rig")
            }
            return 0
        }
        return Float(min(1, max(0, poseMix)))
    }

    private func updateDeformers(
        parameters: RigParameters, time: Double, neutral: Bool,
        localMotion: RigLocalMotionFrame?
    ) {
        base.deformer.update(
            parameters: parameters, time: time, neutral: neutral,
            localTransforms: localMotion?.base
        )
        pose?.deformer.update(
            parameters: parameters, time: time, neutral: neutral,
            localTransforms: localMotion?.pose
        )
    }

    private func acquireFrameSlot() -> Int {
        inFlight.wait()
        let slot = frameSlot
        frameSlot = (frameSlot + 1) % RigMetalRenderer.maxFramesInFlight
        return slot
    }

    private func ensureOutputTexture() throws -> MTLTexture {
        if let outputTexture { return outputTexture }
        let descriptor = MTLTextureDescriptor.texture2DDescriptor(
            pixelFormat: .rgba8Unorm,
            width: canvasWidth,
            height: canvasHeight,
            mipmapped: false
        )
        descriptor.usage = [.renderTarget, .shaderRead]
        descriptor.storageMode = .private
        guard let texture = device.makeTexture(descriptor: descriptor) else {
            throw RigNativeError.unavailable("Metal offscreen output allocation failed")
        }
        texture.label = "Rig offscreen output"
        outputTexture = texture
        return texture
    }

    private func ensureOutputReadback(length: Int) throws -> MTLBuffer {
        if let outputReadback, outputReadback.length >= length { return outputReadback }
        guard let buffer = device.makeBuffer(length: length, options: .storageModeShared) else {
            throw RigNativeError.unavailable("Metal offscreen readback allocation failed")
        }
        buffer.label = "Rig offscreen readback"
        outputReadback = buffer
        return buffer
    }

    private func ensureCaptureReadback(length: Int) throws -> MTLBuffer {
        if let captureReadback, captureReadbackCapacity >= length { return captureReadback }
        guard let buffer = device.makeBuffer(length: length, options: .storageModeShared) else {
            throw RigNativeError.unavailable("Metal drawable readback allocation failed")
        }
        buffer.label = "Rig drawable readback"
        captureReadback = buffer
        captureReadbackCapacity = length
        return buffer
    }

    private func encodeScene(commandBuffer: MTLCommandBuffer, slot: Int, output: MTLTexture, outputPixelFormat: MTLPixelFormat, poseMix: Float) throws {
        let baseWeight: Float
        let poseWeight: Float
        if pose == nil {
            baseWeight = 1
            poseWeight = 0
        } else {
            baseWeight = 1 - poseMix
            poseWeight = poseMix
        }

        if baseWeight > 0 {
            try encodeModel(commandBuffer: commandBuffer, slot: slot, model: base, target: groupTargets[0])
        }
        if let pose, poseWeight > 0 {
            try encodeModel(commandBuffer: commandBuffer, slot: slot, model: pose, target: groupTargets[1])
        }
        try encodeComposite(commandBuffer: commandBuffer, output: output, outputPixelFormat: outputPixelFormat, groups: [
            (groupTargets[0], baseWeight),
            (groupTargets.count > 1 ? groupTargets[1] : nil, poseWeight)
        ])
    }

    private func encodeModel(commandBuffer: MTLCommandBuffer, slot: Int, model: ModelResources, target: GroupTarget) throws {
        let pass = MTLRenderPassDescriptor()
        guard let colorAttachment = pass.colorAttachments[0] else {
            throw RigNativeError.unavailable("Metal model color attachment unavailable")
        }
        colorAttachment.texture = target.color
        colorAttachment.loadAction = .clear
        colorAttachment.storeAction = .store
        colorAttachment.clearColor = MTLClearColor(red: 0, green: 0, blue: 0, alpha: 0)
        let stencilAttachment = MTLRenderPassStencilAttachmentDescriptor()
        pass.stencilAttachment = stencilAttachment
        stencilAttachment.texture = target.stencil
        stencilAttachment.loadAction = .clear
        stencilAttachment.storeAction = .store
        stencilAttachment.clearStencil = 0

        guard let encoder = commandBuffer.makeRenderCommandEncoder(descriptor: pass) else {
            throw RigNativeError.unavailable("Metal model render encoder unavailable")
        }
        defer { encoder.endEncoding() }
        encoder.label = "Rig model pass"
        encoder.setViewport(MTLViewport(originX: 0, originY: 0, width: Double(canvasWidth), height: Double(canvasHeight), znear: 0, zfar: 1))
        encoder.setRenderPipelineState(try layerRenderPipeline())
        encoder.setFragmentSamplerState(sampler, index: 0)
        var vertexUniforms = VertexUniforms(resolution: SIMD2<Float>(Float(canvasWidth), Float(canvasHeight)))
        withUnsafePointer(to: &vertexUniforms) {
            encoder.setVertexBytes($0, length: MemoryLayout<VertexUniforms>.stride, index: 2)
        }

        // Every authored white prepares its aperture before any ordered color draw;
        // source alpha defines the mask even while the white fades out.
        for mesh in model.meshes {
            let alpha = mesh.source.alpha
            guard alpha.isFinite else {
                throw RigNativeError.invalid("mesh \(mesh.source.layer.name) alpha is not finite")
            }
            #if RIG_PROBE_FAULTS
            mesh.hitMeshes[slot].alpha = alpha
            #endif
            if alpha >= 0.004 || mesh.isEyeWhite { uploadPositions(mesh: mesh, slot: slot) }
        }
        encoder.setRenderPipelineState(try maskRenderPipeline())
        for mesh in model.meshes where mesh.isEyeWhite {
            encoder.setDepthStencilState(eyeWhiteStencilStates[mesh.eyeBit == 1 ? 0 : mesh.eyeBit == 2 ? 1 : 2])
            encoder.setStencilReferenceValue(mesh.eyeBit)
            encoder.setVertexBuffer(mesh.positionBuffers[slot], offset: 0, index: 0)
            encoder.setVertexBuffer(mesh.uvBuffer, offset: 0, index: 1)
            var uniforms = FragmentUniforms(alpha: 1, cut: rigEyeAlphaCutoff)
            withUnsafePointer(to: &uniforms) {
                encoder.setFragmentBytes($0, length: MemoryLayout<FragmentUniforms>.stride, index: 0)
            }
            encoder.setFragmentTexture(mesh.texture, index: 0)
            encoder.drawIndexedPrimitives(type: .triangle, indexCount: mesh.indexCount, indexType: .uint16, indexBuffer: mesh.indexBuffer, indexBufferOffset: 0)
        }
        encoder.setRenderPipelineState(try layerRenderPipeline())
        for mesh in model.meshes {
            let alpha = mesh.source.alpha
            if alpha < 0.004 && !mesh.isEyeWhite { continue }
            if mesh.irisReadMask != 0 {
                encoder.setDepthStencilState(irisStencilStates[Int(mesh.irisReadMask) - 4])
                encoder.setStencilReferenceValue(0)
            } else {
                encoder.setDepthStencilState(noStencilState)
            }
            encoder.setVertexBuffer(mesh.positionBuffers[slot], offset: 0, index: 0)
            encoder.setVertexBuffer(mesh.uvBuffer, offset: 0, index: 1)
            var uniforms = FragmentUniforms(alpha: alpha, cut: mesh.isEyeWhite ? rigEyeAlphaCutoff : 0)
            withUnsafePointer(to: &uniforms) {
                encoder.setFragmentBytes($0, length: MemoryLayout<FragmentUniforms>.stride, index: 0)
            }
            encoder.setFragmentTexture(mesh.texture, index: 0)
            encoder.drawIndexedPrimitives(type: .triangle, indexCount: mesh.indexCount, indexType: .uint16, indexBuffer: mesh.indexBuffer, indexBufferOffset: 0)
        }
    }

    private func uploadPositions(mesh: MeshResources, slot: Int) {
        let buffer = mesh.positionBuffers[slot]
        mesh.source.positions.withUnsafeBytes { raw in
            buffer.contents().copyMemory(from: raw.baseAddress!, byteCount: raw.count)
        }
        #if RIG_PROBE_FAULTS
        mesh.hitMeshes[slot].refreshBounds()
        #endif
    }

    private func encodeComposite(commandBuffer: MTLCommandBuffer, output: MTLTexture, outputPixelFormat: MTLPixelFormat, groups: [(GroupTarget?, Float)]) throws {
        let pass = MTLRenderPassDescriptor()
        guard let colorAttachment = pass.colorAttachments[0] else {
            throw RigNativeError.unavailable("Metal composite color attachment unavailable")
        }
        colorAttachment.texture = output
        colorAttachment.loadAction = .clear
        colorAttachment.storeAction = .store
        colorAttachment.clearColor = MTLClearColor(red: 0, green: 0, blue: 0, alpha: 0)
        guard let encoder = commandBuffer.makeRenderCommandEncoder(descriptor: pass) else {
            throw RigNativeError.unavailable("Metal composite render encoder unavailable")
        }
        defer { encoder.endEncoding() }
        encoder.label = "Rig weighted model composite"
        encoder.setRenderPipelineState(try compositePipeline(for: outputPixelFormat))
        let mapping = RigCanvasMapping(canvasWidth: canvasWidth, canvasHeight: canvasHeight,
                                       drawableWidth: output.width, drawableHeight: output.height)
        encoder.setViewport(MTLViewport(originX: mapping.origin.x, originY: mapping.origin.y,
                                        width: mapping.size.x, height: mapping.size.y, znear: 0, zfar: 1))
        encoder.setFragmentSamplerState(sampler, index: 0)
        // The weighted output has no depth/stencil attachment.
        for (target, weight) in groups where weight > 0 {
            guard let target else { continue }
            var uniforms = CompositeUniforms(weight: weight)
            withUnsafePointer(to: &uniforms) {
                encoder.setFragmentBytes($0, length: MemoryLayout<CompositeUniforms>.stride, index: 0)
            }
            encoder.setFragmentTexture(target.color, index: 0)
            encoder.drawPrimitives(type: .triangle, vertexStart: 0, vertexCount: 6)
        }
    }

    private func encodeReadback(commandBuffer: MTLCommandBuffer, texture: MTLTexture, buffer: MTLBuffer, width: Int, height: Int, bytesPerRow: Int) throws {
        guard let blit = commandBuffer.makeBlitCommandEncoder() else {
            throw RigNativeError.unavailable("Metal readback blit encoder unavailable")
        }
        blit.label = "Rig explicit pixel capture"
        blit.copy(
            from: texture,
            sourceSlice: 0,
            sourceLevel: 0,
            sourceOrigin: MTLOrigin(x: 0, y: 0, z: 0),
            sourceSize: MTLSize(width: width, height: height, depth: 1),
            to: buffer,
            destinationOffset: 0,
            destinationBytesPerRow: bytesPerRow,
            destinationBytesPerImage: bytesPerRow * height
        )
        blit.endEncoding()
    }

    private func commitAndWait(_ commandBuffer: MTLCommandBuffer) throws {
        commandBuffer.commit()
        commandBuffer.waitUntilCompleted()
        if let error = commandBuffer.error {
            throw RigNativeError.unavailable("Metal command buffer failed: \(error.localizedDescription)")
        }
        guard commandBuffer.status == .completed else {
            throw RigNativeError.unavailable("Metal command buffer ended with status \(String(describing: commandBuffer.status))")
        }
    }

    private func layerRenderPipeline() throws -> MTLRenderPipelineState {
        if let layerPipeline { return layerPipeline }
        guard let vertexFunction = library.makeFunction(name: "rigLayerVertex"),
              let fragmentFunction = library.makeFunction(name: "rigLayerFragment") else {
            throw RigNativeError.unavailable("Metal layer shader functions unavailable")
        }
        let vertexDescriptor = MTLVertexDescriptor()
        vertexDescriptor.attributes[0].format = .float2
        vertexDescriptor.attributes[0].offset = 0
        vertexDescriptor.attributes[0].bufferIndex = 0
        vertexDescriptor.attributes[1].format = .float2
        vertexDescriptor.attributes[1].offset = 0
        vertexDescriptor.attributes[1].bufferIndex = 1
        vertexDescriptor.layouts[0].stride = MemoryLayout<SIMD2<Float>>.stride
        vertexDescriptor.layouts[0].stepFunction = .perVertex
        vertexDescriptor.layouts[1].stride = MemoryLayout<SIMD2<Float>>.stride
        vertexDescriptor.layouts[1].stepFunction = .perVertex

        let descriptor = MTLRenderPipelineDescriptor()
        descriptor.label = "Rig layer pipeline"
        descriptor.vertexFunction = vertexFunction
        descriptor.fragmentFunction = fragmentFunction
        descriptor.vertexDescriptor = vertexDescriptor
        descriptor.colorAttachments[0].pixelFormat = .rgba8Unorm
        descriptor.colorAttachments[0].isBlendingEnabled = true
        descriptor.colorAttachments[0].sourceRGBBlendFactor = .one
        descriptor.colorAttachments[0].destinationRGBBlendFactor = .oneMinusSourceAlpha
        descriptor.colorAttachments[0].sourceAlphaBlendFactor = .one
        descriptor.colorAttachments[0].destinationAlphaBlendFactor = .oneMinusSourceAlpha
        descriptor.depthAttachmentPixelFormat = .invalid
        descriptor.stencilAttachmentPixelFormat = .stencil8
        do {
            let pipeline = try device.makeRenderPipelineState(descriptor: descriptor)
            descriptor.label = "Rig eye aperture pipeline"
            descriptor.colorAttachments[0].writeMask = []
            maskPipeline = try device.makeRenderPipelineState(descriptor: descriptor)
            layerPipeline = pipeline
            return pipeline
        } catch {
            throw RigNativeError.unavailable("Metal layer pipeline creation failed: \(error.localizedDescription)")
        }
    }

    private func maskRenderPipeline() throws -> MTLRenderPipelineState {
        if let maskPipeline { return maskPipeline }
        _ = try layerRenderPipeline()
        return maskPipeline!
    }

    private func compositePipeline(for pixelFormat: MTLPixelFormat) throws -> MTLRenderPipelineState {
        let key = pixelFormat.rawValue
        if let pipeline = compositePipelines[key] { return pipeline }
        guard pixelFormat == .rgba8Unorm || pixelFormat == .bgra8Unorm || pixelFormat == .rgba8Unorm_srgb || pixelFormat == .bgra8Unorm_srgb else {
            throw RigNativeError.invalid("unsupported composite pixel format")
        }
        guard let vertexFunction = library.makeFunction(name: "rigCompositeVertex"),
              let fragmentFunction = library.makeFunction(name: "rigCompositeFragment") else {
            throw RigNativeError.unavailable("Metal composite shader functions unavailable")
        }
        let descriptor = MTLRenderPipelineDescriptor()
        descriptor.label = "Rig weighted composite pipeline"
        descriptor.vertexFunction = vertexFunction
        descriptor.fragmentFunction = fragmentFunction
        descriptor.colorAttachments[0].pixelFormat = pixelFormat
        descriptor.colorAttachments[0].isBlendingEnabled = true
        descriptor.colorAttachments[0].sourceRGBBlendFactor = .one
        descriptor.colorAttachments[0].destinationRGBBlendFactor = .one
        descriptor.colorAttachments[0].sourceAlphaBlendFactor = .one
        descriptor.colorAttachments[0].destinationAlphaBlendFactor = .one
        descriptor.depthAttachmentPixelFormat = .invalid
        descriptor.stencilAttachmentPixelFormat = .invalid
        do {
            #if RIG_PROBE_FAULTS
            if probeInvalidPipeline {
                probeInvalidPipeline = false
                _ = try device.makeLibrary(source: "kernel void invalidRigProbe( {", options: nil)
            }
            #endif
            let pipeline = try device.makeRenderPipelineState(descriptor: descriptor)
            compositePipelines[key] = pipeline
            return pipeline
        } catch {
            throw RigNativeError.unavailable("Metal composite pipeline creation failed: \(error.localizedDescription)")
        }
    }

    private static func makeDepthStencilState(device: MTLDevice, compare: MTLCompareFunction, readMask: UInt32, writeMask: UInt32, passOperation: MTLStencilOperation, label: String) throws
        -> MTLDepthStencilState
    {
        let descriptor = MTLDepthStencilDescriptor()
        descriptor.label = label
        descriptor.depthCompareFunction = .always
        descriptor.isDepthWriteEnabled = false
        let stencil = MTLStencilDescriptor()
        stencil.stencilCompareFunction = compare
        stencil.stencilFailureOperation = .keep
        stencil.depthFailureOperation = .keep
        stencil.depthStencilPassOperation = passOperation
        stencil.readMask = readMask
        stencil.writeMask = writeMask
        descriptor.frontFaceStencil = stencil
        descriptor.backFaceStencil = stencil
        guard let state = device.makeDepthStencilState(descriptor: descriptor) else {
            throw RigNativeError.unavailable("Metal depth/stencil state allocation failed")
        }
        return state
    }

}
