import AppKit
import CoreGraphics
import Darwin
import Foundation
import ImageIO
import Metal
import MetalKit
import QuartzCore

struct NativeProbeOptions {
    let helper: URL
    let decoderBundle: URL
    let pilot: URL
    let output: URL
    let duration: Double
    let checks: Bool

    static func parse() throws -> NativeProbeOptions {
        var values: [String: String] = [:]
        var checks = true
        let arguments = Array(CommandLine.arguments.dropFirst())
        var index = 0
        while index < arguments.count {
            let name = arguments[index]
            if name == "--no-checks" { checks = false; index += 1; continue }
            guard ["--helper", "--decoder-bundle", "--pilot", "--output", "--duration"].contains(name),
                  index + 1 < arguments.count, values[name] == nil else {
                throw RigNativeError.invalid("unknown, duplicate or incomplete option \(name)")
            }
            values[name] = arguments[index + 1]
            index += 2
        }
        func requiredURL(_ key: String) throws -> URL {
            guard let value = values[key], !value.isEmpty else { throw RigNativeError.invalid("missing \(key)") }
            return URL(fileURLWithPath: value).standardizedFileURL
        }
        let duration = Double(values["--duration"] ?? "8") ?? .nan
        guard duration.isFinite, duration > 0, duration <= 3600 else { throw RigNativeError.invalid("duration") }
        return NativeProbeOptions(helper: try requiredURL("--helper"), decoderBundle: try requiredURL("--decoder-bundle"),
                                  pilot: try requiredURL("--pilot"), output: try requiredURL("--output"), duration: duration, checks: checks)
    }
}

final class TransparentRigView: MTKView {
    override var isOpaque: Bool { false }
    override func makeBackingLayer() -> CALayer {
        let result = super.makeBackingLayer()
        result.isOpaque = false
        result.backgroundColor = NSColor.clear.cgColor
        return result
    }
}

func frameAlpha(_ frame: RigFrame, x: Int, y: Int) -> Double {
    guard x >= 0, y >= 0, x < frame.width, y < frame.height else { return 0 }
    return frame.rgba.withUnsafeBytes { bytes in
        Double(bytes.bindMemory(to: UInt8.self)[(y * frame.width + x) * 4 + 3]) / 255
    }
}

func frameFacts(_ frame: RigFrame) -> [String: Any] {
    ["width": frame.width, "height": frame.height, "bytes": frame.rgba.count,
     "cornerAlpha": [frameAlpha(frame, x: 0, y: 0), frameAlpha(frame, x: frame.width - 1, y: 0),
                     frameAlpha(frame, x: 0, y: frame.height - 1), frameAlpha(frame, x: frame.width - 1, y: frame.height - 1)],
     "centerAlpha": frameAlpha(frame, x: frame.width / 2, y: frame.height / 2)]
}

func writeFrame(_ frame: RigFrame, to url: URL) throws {
    guard frame.width > 0, frame.height > 0, frame.rgba.count == frame.width * frame.height * 4,
          let provider = CGDataProvider(data: frame.rgba as CFData),
          let colorSpace = CGColorSpace(name: CGColorSpace.sRGB),
          let image = CGImage(width: frame.width, height: frame.height, bitsPerComponent: 8, bitsPerPixel: 32,
                              bytesPerRow: frame.width * 4, space: colorSpace,
                              bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue),
                              provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent),
          let destination = CGImageDestinationCreateWithURL(url as CFURL, "public.png" as CFString, 1, nil) else {
        throw RigNativeError.invalid("cannot encode native frame")
    }
    CGImageDestinationAddImage(destination, image, nil)
    guard CGImageDestinationFinalize(destination) else { throw RigNativeError.unavailable("PNG write failed") }
}

func frameDifference(_ base: RigFrame, _ frame: RigFrame) throws -> [String: Any] {
    guard base.width == frame.width, base.height == frame.height else { throw RigNativeError.invalid("comparison size") }
    var changed = 0, torso = 0, minX = frame.width, minY = frame.height, maxX = -1, maxY = -1
    base.rgba.withUnsafeBytes { left in
        frame.rgba.withUnsafeBytes { right in
            let a = left.bindMemory(to: UInt8.self), b = right.bindMemory(to: UInt8.self)
            for y in 0..<frame.height {
                for x in 0..<frame.width {
                    let offset = (y * frame.width + x) * 4
                    if a[offset] != b[offset] || a[offset + 1] != b[offset + 1] || a[offset + 2] != b[offset + 2] || a[offset + 3] != b[offset + 3] {
                        changed += 1
                        if y >= 260 { torso += 1 }
                        minX = min(minX, x); minY = min(minY, y); maxX = max(maxX, x); maxY = max(maxY, y)
                    }
                }
            }
        }
    }
    return ["changedPixels": changed, "torsoChangedPixels": torso, "changedBounds": [minX, minY, maxX, maxY]]
}

final class NativeRigProbe: NSObject, NSApplicationDelegate, MTKViewDelegate {
    let options: NativeProbeOptions
    let device: MTLDevice
    private let logFile: FileHandle
    private var panel: NSPanel?
    private var view: TransparentRigView?
    private var renderer: RigMetalRenderer?
    private var candidate: (token: Int, renderer: RigMetalRenderer, pilot: URL)?
    private var decodeJob: RigDecodeJob?
    private let decodeQueue = DispatchQueue(label: "herdr.native-rig-probe.decode", qos: .userInitiated)
    private var generation = 0
    private var committedToken: Int?
    private var committedPilot: URL?
    private var activeToken: Int?
    private var pendingAppliedToken: Int?
    private var renderFault = false
    private var parameters = RigParameters()
    private var manualParameters = Set<String>()
    private var poseMix = 0.0
    private var frameNumber = 0
    private var viewportEpoch = 0
    private var renderedViewportEpoch = -1
    private var drawMilliseconds = 0.0
    private var captureSequence = 0
    private var hasChecked = false
    private var pendingCapture: String?
    private var captureObserver: ((RigFrame) -> Void)?
    private var forcedFrame: (RigParameters, Double, Bool, Double)?
    private var started = ProcessInfo.processInfo.systemUptime
    private var suspended = false
    private var disposed = false
    private(set) var exitStatus: Int32 = 0

    init(options: NativeProbeOptions) throws {
        self.options = options
        guard let device = MTLCreateSystemDefaultDevice() else { throw RigNativeError.unavailable("Metal device") }
        self.device = device
        try FileManager.default.createDirectory(at: options.output, withIntermediateDirectories: true)
        let logURL = options.output.appendingPathComponent("events.ndjson")
        guard FileManager.default.createFile(atPath: logURL.path, contents: nil) else { throw RigNativeError.unavailable("event log") }
        logFile = try FileHandle(forWritingTo: logURL)
        super.init()
    }

    func emit(_ type: String, _ fields: [String: Any] = [:]) {
        var event = fields
        event["type"] = type
        event["pid"] = ProcessInfo.processInfo.processIdentifier
        event["uptime"] = ProcessInfo.processInfo.systemUptime
        do {
            var data = try JSONSerialization.data(withJSONObject: event, options: [.sortedKeys])
            data.append(10)
            try logFile.write(contentsOf: data)
            FileHandle.standardOutput.write(data)
        } catch {
            FileHandle.standardError.write(Data("native rig probe event write failed: \(error)\n".utf8))
            Darwin.exit(1)
        }
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        let rect = NSRect(x: 0, y: 0, width: 384, height: 512)
        let panel = NSPanel(contentRect: rect, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.isReleasedWhenClosed = false
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        panel.acceptsMouseMovedEvents = true
        let view = TransparentRigView(frame: rect, device: device)
        view.colorPixelFormat = .bgra8Unorm
        view.clearColor = MTLClearColorMake(0, 0, 0, 0)
        view.framebufferOnly = false
        view.autoResizeDrawable = true
        view.preferredFramesPerSecond = 60
        view.isPaused = true
        view.enableSetNeedsDisplay = false
        view.delegate = self
        view.layer?.isOpaque = false
        view.layer?.backgroundColor = NSColor.clear.cgColor
        panel.contentView = view
        panel.center()
        self.panel = panel
        self.view = view
        emit("native_probe_started", ["device": device.name, "publicAPIsOnly": true, "pilot": options.pilot.path,
                                      "output": options.output.path, "duration": options.duration, "checks": options.checks])
        prepare(pilot: options.pilot, autoApply: true)
        startCommands()
        DispatchQueue.main.asyncAfter(deadline: .now() + options.duration) { [weak self] in self?.finish() }
    }

    private func resource(_ name: String, under root: URL) throws -> URL {
        let parts = name.split(separator: "/", omittingEmptySubsequences: false)
        guard !parts.isEmpty, parts.allSatisfy({ !$0.isEmpty && $0 != "." && $0 != ".." }),
              !name.contains("\\"), !name.contains("\0") else { throw RigNativeError.invalid("pilot resource path") }
        var current = root
        for part in parts {
            current.appendPathComponent(String(part))
            let attributes = try current.resourceValues(forKeys: [.isSymbolicLinkKey])
            guard attributes.isSymbolicLink != true else { throw RigNativeError.invalid("symbolic link resource") }
        }
        return current
    }

    private func prepare(pilot: URL, autoApply: Bool, timeout: TimeInterval = 10) {
        guard !disposed else { return }
        if let candidate, candidate.token == committedToken {
            emit("command_error", ["message": "committed candidate must apply before replacement"])
            return
        }
        generation += 1
        let token = generation
        decodeJob?.cancel()
        decodeJob = nil
        candidate = nil
        do {
            let root = pilot.standardizedFileURL.resolvingSymlinksInPath()
            let manifestURL = try resource("pilot.json", under: root)
            let size = try manifestURL.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
            guard size > 0, size <= 2 * 1024 * 1024 else { throw RigNativeError.invalid("pilot metadata size") }
            guard let manifest = try JSONSerialization.jsonObject(with: Data(contentsOf: manifestURL)) as? [String: Any],
                  let base = manifest["model"] as? String, let overrides = manifest["overrides"] as? String else {
                throw RigNativeError.invalid("pilot model/overrides")
            }
            let poseURL = try (manifest["alternate"] as? String).map { try resource($0, under: root) }
            let job = RigDecodeJob(helperURL: options.helper, bundleURL: options.decoderBundle,
                                   baseURL: try resource(base, under: root), poseURL: poseURL,
                                   overridesURL: try resource(overrides, under: root), timeout: timeout)
            decodeJob = job
            emit("candidate_preparing", ["token": token, "oldActiveToken": activeToken as Any? ?? NSNull(), "oldVisible": renderer != nil])
            decodeQueue.async { [weak self] in
                let result: Result<RigDecodedScene, Error>
                do { result = .success(try job.run()) } catch { result = .failure(error) }
                DispatchQueue.main.async {
                    guard let self, !self.disposed, token == self.generation else { return }
                    self.decodeJob = nil
                    do {
                        let scene = try result.get()
                        let prepared = try RigMetalRenderer(device: self.device, scene: scene)
                        var neutral = RigParameters(); neutral.physAmp = 0; neutral.fhAmp = 0
                        let firstFrame = try prepared.render(parameters: neutral, time: 0, neutral: true, poseMix: 0)
                        self.candidate = (token, prepared, root)
                        self.emit("candidate_prepared", ["token": token, "oldActiveToken": self.activeToken as Any? ?? NSNull(),
                                                        "oldVisible": self.renderer != nil, "frame": frameFacts(firstFrame),
                                                        "resources": prepared.metrics, "baseLayers": scene.document.base.layers.count,
                                                        "poseLayers": scene.document.pose?.layers.count ?? 0])
                        if autoApply { try self.commit(token); try self.apply(token) }
                    } catch { self.fail(error, stage: "prepare", token: token) }
                }
            }
        } catch { fail(error, stage: "prepare", token: token) }
    }

    private func commit(_ token: Int) throws {
        guard candidate?.token == token, generation == token else { throw RigNativeError.invalid("stale candidate commit") }
        committedToken = token
        committedPilot = candidate?.pilot
        emit("candidate_committed", ["token": token, "scope": "in-memory feasibility coordinator, not product store"])
    }

    private func apply(_ token: Int) throws {
        guard let candidate, candidate.token == token, committedToken == token, generation == token else {
            throw RigNativeError.invalid("uncommitted or stale candidate apply")
        }
        pendingAppliedToken = token
        view?.isPaused = suspended
        if !suspended {
            panel?.orderFrontRegardless()
            view?.draw()
        }
        if options.checks && !hasChecked {
            hasChecked = true
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { [weak self] in
                guard let self else { return }
                do { try self.runChecks() } catch { self.fail(error, stage: "checks", token: token) }
            }
        }
    }

    private func animated(_ milliseconds: Double) -> RigParameters {
        var result = parameters
        let seconds = milliseconds / 1000
        if !manualParameters.contains("eyeX") { result.eyeX = sin(seconds * 0.73) * 0.38 }
        if !manualParameters.contains("eyeY") { result.eyeY = sin(seconds * 0.49 + 0.4) * 0.18 }
        if !manualParameters.contains("angleX") { result.angleX = sin(seconds * 0.37) * 0.12 }
        if !manualParameters.contains("angleY") { result.angleY = sin(seconds * 0.29 + 0.8) * 0.1 }
        if !manualParameters.contains("angleZ") { result.angleZ = sin(seconds * 0.23) * 0.08 }
        return result
    }

    func draw(in view: MTKView) {
        var selected = renderer
        if let token = pendingAppliedToken, let candidate, candidate.token == token { selected = candidate.renderer }
        guard !disposed, !suspended, let renderer = selected, let drawable = view.currentDrawable else { return }
        let begin = ProcessInfo.processInfo.systemUptime
        let time = (begin - started) * 1000
        let chosen = forcedFrame ?? (animated(time), time, false, poseMix)
        let captureName = pendingCapture
        do {
            let frame = try renderer.draw(to: drawable, parameters: chosen.0, time: chosen.1,
                                          neutral: chosen.2, poseMix: chosen.3, capture: captureName != nil)
            frameNumber += 1
            renderedViewportEpoch = viewportEpoch
            drawMilliseconds += (ProcessInfo.processInfo.systemUptime - begin) * 1000
            renderFault = false
            if let token = pendingAppliedToken {
                self.renderer = renderer
                activeToken = token
                candidate = nil
                pendingAppliedToken = nil
                emit("candidate_applied", ["token": token, "frameNumber": frameNumber, "resources": renderer.metrics])
            }
            if let captureName {
                guard let frame else { throw RigNativeError.unavailable("actual drawable capture missing") }
                pendingCapture = nil
                forcedFrame = nil
                let path = options.output.appendingPathComponent(captureName + ".png")
                try writeFrame(frame, to: path)
                captureObserver?(frame)
                compareCachedHits(frame, renderer: renderer, name: captureName)
                emit("native_drawable_capture", ["artifact": path.path, "frame": frameFacts(frame), "frameNumber": frameNumber,
                                                 "backingScale": panel?.backingScaleFactor ?? 0,
                                                 "panelOpaque": panel?.isOpaque ?? true, "viewOpaque": view.isOpaque,
                                                 "metalLayerOpaque": view.layer?.isOpaque ?? true])
            }
        } catch {
            pendingCapture = nil
            forcedFrame = nil
            view.isPaused = true
            renderFault = true
            fail(error, stage: "draw", token: activeToken)
        }
    }

    func mtkView(_ view: MTKView, drawableSizeWillChange size: CGSize) {
        viewportEpoch &+= 1
        emit("drawable_size", ["width": size.width, "height": size.height, "viewportEpoch": viewportEpoch])
    }

    private func compareCachedHits(_ frame: RigFrame, renderer: RigMetalRenderer, name: String) {
        let resources = renderer.metrics
        let modelWidth = Double(resources["canvasWidth"]!)
        let modelHeight = Double(resources["canvasHeight"]!)
        var points = Set<Int>()
        for y in stride(from: 0, to: frame.height, by: max(1, frame.height / 16)) {
            for x in stride(from: 0, to: frame.width, by: max(1, frame.width / 16)) {
                points.insert(y * frame.width + x)
            }
        }
        var unknown = 0, maximumDelta = 0, overOne = 0, mismatches = 0
        var times: [Double] = []
        var differences: [[String: Any]] = []
        frame.rgba.withUnsafeBytes { pixels in
            var edges: [Int] = []
            for index in 0..<(frame.width * frame.height) {
                let alpha = pixels[index * 4 + 3]
                if (index % frame.width > 0 && alpha != pixels[(index - 1) * 4 + 3])
                    || (index >= frame.width && alpha != pixels[(index - frame.width) * 4 + 3]) {
                    edges.append(index)
                }
            }
            if !edges.isEmpty {
                for item in stride(from: 0, to: edges.count, by: max(1, edges.count / 256)).prefix(256) {
                    points.insert(edges[item])
                }
            }
            for index in points.sorted() {
                let x = index % frame.width, y = index / frame.width
                let mapping = RigCanvasMapping(canvasWidth: Int(modelWidth), canvasHeight: Int(modelHeight),
                                               drawableWidth: frame.width, drawableHeight: frame.height)
                let point = mapping.modelPoint(drawableX: Double(x) + 0.5, drawableY: Double(y) + 0.5)
                let mx = point.x, my = point.y
                let begin = ProcessInfo.processInfo.systemUptime
                // Frozen capture calibration, not the production 80 ms input policy.
                guard let cached = renderer.rasterHitEstimate(modelX: mx, modelY: my, maximumAge: 1) else {
                    unknown += 1
                    continue
                }
                times.append((ProcessInfo.processInfo.systemUptime - begin) * 1_000_000)
                let expected = Int(pixels[index * 4 + 3])
                let actual = Int((cached.sample.alpha * 255).rounded())
                let delta = abs(expected - actual)
                maximumDelta = max(maximumDelta, delta)
                if delta > 1 { overOne += 1 }
                if (Double(expected) / 255 > 0.1) != (cached.sample.alpha > 0.1) { mismatches += 1 }
                if delta > 1 && differences.count < 16 {
                    differences.append(["x": x, "y": y, "gpuAlpha": expected, "cpuAlpha": actual])
                }
            }
        }
        times.sort()
        emit("cached_hit_comparison", ["name": name, "points": points.count, "unknown": unknown,
                                      "maximumAlphaByteDelta": maximumDelta, "overOneByte": overOne,
                                      "thresholdMismatches": mismatches, "differences": differences,
                                      "meanMicroseconds": times.reduce(0, +) / Double(max(1, times.count)),
                                      "p99Microseconds": times.isEmpty ? 0 : times[min(times.count - 1, times.count * 99 / 100)],
                                      "gpuReference": "captured completed frame", "resources": resources])
    }

    private func runChecks() throws {
        guard let renderer else { throw RigNativeError.unavailable("active renderer") }
        view?.isPaused = true
        defer { view?.isPaused = suspended }
        renderer.resetSimulation()
        var baselineParameters = RigParameters()
        baselineParameters.physAmp = 0; baselineParameters.fhAmp = 0
        let baseline = try renderer.render(parameters: baselineParameters, time: 0, neutral: true, poseMix: 0)
        try writeFrame(baseline, to: options.output.appendingPathComponent("check-neutral.png"))
        compareCachedHits(baseline, renderer: renderer, name: "neutral")
        let scenarios: [(String, [String: Double], Double, Bool, Double)] = [
            ("blink-left", ["eyeOpenL": 0.1], 0, true, 0),
            ("blink-right", ["eyeOpenR": 0.1], 0, true, 0),
            ("gaze-independent", ["eyeX": 0.78, "eyeY": -0.34], 0, true, 0),
            ("mouth-independent", ["mouthOpen": 0.88, "mouthForm": 0.62, "mouthCY": 0.16], 0, true, 0),
            ("head-independent", ["angleX": 0.72, "angleY": -0.48, "angleZ": 0.45], 0, true, 0),
            ("breath-engine", [:], 1200, false, 0),
            ("pose-crossfade-fbo", [:], 0, true, 0.5),
            ("alternate-pose", [:], 0, true, 1)
        ]
        for (name, values, time, neutral, mix) in scenarios {
            var state = baselineParameters
            try state.apply(values)
            let frame = try renderer.render(parameters: state, time: time, neutral: neutral, poseMix: mix)
            let artifact = options.output.appendingPathComponent("check-" + name + ".png")
            try writeFrame(frame, to: artifact)
            compareCachedHits(frame, renderer: renderer, name: name)
            emit("scenario_capture", ["name": name, "artifact": artifact.path, "frame": frameFacts(frame),
                                      "difference": try frameDifference(baseline, frame), "resources": renderer.metrics,
                                      "parameters": try JSONSerialization.jsonObject(with: JSONEncoder().encode(state))])
        }
        pendingCapture = "native-drawable-neutral"
        forcedFrame = (baselineParameters, 0, true, 0)
        view?.draw()
        guard pendingCapture == nil else { throw RigNativeError.unavailable("no actual Metal drawable; host transparency remains unverified") }
        for (name, x, y) in [("transparent", 0.0, 0.0), ("head", 192.0, 150.0), ("body", 192.0, 300.0), ("outside", -1.0, 256.0)] {
            let begin = ProcessInfo.processInfo.systemUptime
            let hit = try renderer.sampleHit(modelX: x, modelY: y)
            emit("native_hit_sample", ["name": name, "x": x, "y": y, "alpha": hit.alpha,
                                       "region": hit.semantic?.region as Any? ?? NSNull(),
                                       "layer": hit.semantic?.layer as Any? ?? NSNull(),
                                       "milliseconds": (ProcessInfo.processInfo.systemUptime - begin) * 1000,
                                       "frameNumber": frameNumber, "token": activeToken as Any? ?? NSNull()])
        }
    }

    private func startCommands() {
        DispatchQueue.global(qos: .utility).async { [weak self] in
            while let line = readLine() {
                guard line.utf8.count <= 65536, let data = line.data(using: .utf8) else { continue }
                do {
                    guard let command = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { continue }
                    DispatchQueue.main.async { self?.handle(command) }
                } catch { DispatchQueue.main.async { self?.emit("command_error", ["message": "invalid JSON command"]) } }
            }
        }
    }

    private func handle(_ command: [String: Any]) {
        do {
            guard let name = command["command"] as? String else { throw RigNativeError.invalid("command name") }
            switch name {
            case "prepare":
                let pilot = (command["pilot"] as? String).map { URL(fileURLWithPath: $0) } ?? options.pilot
                let timeout = command["timeout"] as? Double ?? 10
                guard timeout.isFinite, timeout > 0, timeout <= 30 else { throw RigNativeError.invalid("decode deadline") }
                prepare(pilot: pilot, autoApply: false, timeout: timeout)
            case "commit", "apply":
                guard let token = command["token"] as? Int else { throw RigNativeError.invalid("candidate token") }
                if name == "commit" { try commit(token) } else { try apply(token) }
            case "cancel":
                guard candidate?.token != committedToken || candidate == nil else {
                    throw RigNativeError.invalid("committed candidate cancellation is pending, not rollback")
                }
                generation += 1
                decodeJob?.cancel(); decodeJob = nil; candidate = nil
                emit("candidate_cancelled", ["activeToken": activeToken as Any? ?? NSNull()])
            case "setParameters", "captureFrozen":
                guard let raw = command["parameters"] as? [String: Any] else { throw RigNativeError.invalid("parameters") }
                var values: [String: Double] = [:]
                for (key, value) in raw {
                    guard let number = value as? NSNumber, CFGetTypeID(number) != CFBooleanGetTypeID() else {
                        throw RigNativeError.invalid("numeric parameter \(key)")
                    }
                    values[key] = number.doubleValue
                }
                if name == "captureFrozen" {
                    guard renderer != nil, !suspended, !renderFault, pendingAppliedToken == nil else {
                        throw RigNativeError.invalid("frozen capture requires an active renderer")
                    }
                    var state = RigParameters()
                    try state.apply(values)
                    captureSequence += 1
                    pendingCapture = "native-frozen-\(captureSequence)"
                    forcedFrame = (state, 0, true, poseMix)
                    view?.draw()
                    guard pendingCapture == nil else { throw RigNativeError.unavailable("drawable") }
                } else {
                    try parameters.apply(values)
                    manualParameters.formUnion(values.keys)
                    emit("parameters_applied", ["names": values.keys.sorted()])
                }
            case "setPoseMix":
                guard let value = command["value"] as? Double, value.isFinite, (0...1).contains(value) else {
                    throw RigNativeError.invalid("poseMix")
                }
                poseMix = value
            case "capture":
                captureSequence += 1
                pendingCapture = "native-drawable-\(captureSequence)"
                view?.draw()
                guard pendingCapture == nil else { throw RigNativeError.unavailable("drawable") }
            case "benchmarkCachedHit":
                guard let renderer, !suspended, !renderFault,
                      let x = command["x"] as? Double, let y = command["y"] as? Double,
                      x.isFinite, y.isFinite else { throw RigNativeError.invalid("benchmark point") }
                let minimumAlpha = (command["minimumAlpha"] as? Double) ?? (7.5 / 255)
                guard minimumAlpha.isFinite, (0...1).contains(minimumAlpha) else {
                    throw RigNativeError.invalid("benchmark alpha threshold")
                }
                _ = renderer.cachedHit(modelX: x, modelY: y, minimumAlpha: minimumAlpha)
                var captured: RigFrame?
                captureObserver = { captured = $0 }
                defer { captureObserver = nil }
                captureSequence += 1
                pendingCapture = "native-hit-benchmark-\(captureSequence)"
                view?.draw()
                guard pendingCapture == nil, let captured else { throw RigNativeError.unavailable("benchmark drawable") }
                let before = renderer.metrics
                let width = Double(before["canvasWidth"]!), height = Double(before["canvasHeight"]!)
                let expected: Int
                if x < 0 || y < 0 || x >= width || y >= height {
                    expected = 0
                } else {
                    let mapping = RigCanvasMapping(canvasWidth: Int(width), canvasHeight: Int(height),
                                                   drawableWidth: captured.width, drawableHeight: captured.height)
                    let point = mapping.drawablePoint(modelX: x, modelY: y)
                    expected = Int((frameAlpha(captured, x: Int(floor(point.x)),
                                               y: Int(floor(point.y))) * 255).rounded())
                }
                var durations: [Double] = []
                durations.reserveCapacity(10_000)
                var unknown = 0, mismatches = 0
                for _ in 0..<10_000 {
                    let start = ProcessInfo.processInfo.systemUptime
                    let hit = renderer.cachedHit(modelX: x, modelY: y, minimumAlpha: minimumAlpha)
                    durations.append((ProcessInfo.processInfo.systemUptime - start) * 1_000_000)
                    if let hit {
                        if Int((Double(hit.sample.alpha) * 255).rounded()) != expected { mismatches += 1 }
                    } else { unknown += 1 }
                }
                let after = renderer.metrics
                durations.sort()
                emit("gpu_hit_benchmark", ["x": x, "y": y, "queries": durations.count,
                                           "expectedAlphaByte": expected, "mismatches": mismatches, "unknown": unknown,
                                           "minimumAlpha": minimumAlpha,
                                           "meanMicroseconds": durations.reduce(0, +) / Double(durations.count),
                                           "p99Microseconds": durations[Int(Double(durations.count - 1) * 0.99)],
                                           "gpuPixelsDuringQueries": after["alphaReadbackPixels"]! - before["alphaReadbackPixels"]!,
                                           "scope": "actual drawable full capture versus nonblocking one-pixel GPU cache"])
            case "sampleHit":
                guard !suspended, !renderFault, pendingAppliedToken == nil,
                      let x = command["x"] as? Double, let y = command["y"] as? Double, let renderer else {
                    throw RigNativeError.invalid("hit sample")
                }
                let hit = try renderer.sampleHit(modelX: x, modelY: y)
                emit("native_hit_sample", ["x": x, "y": y, "alpha": hit.alpha, "frameNumber": frameNumber,
                                           "region": hit.semantic?.region as Any? ?? NSNull(),
                                           "layer": hit.semantic?.layer as Any? ?? NSNull(),
                                           "token": activeToken as Any? ?? NSNull()])
            case "sampleCachedHit":
                guard let x = command["x"] as? Double, let y = command["y"] as? Double else {
                    throw RigNativeError.invalid("cached hit coordinates")
                }
                let requestedToken = command["token"] as? Int ?? activeToken
                let requestedViewport = command["viewportEpoch"] as? Int ?? viewportEpoch
                var cached: RigCachedHit?
                if !suspended, !renderFault, pendingAppliedToken == nil, requestedToken == activeToken,
                   requestedViewport == viewportEpoch, renderedViewportEpoch == viewportEpoch {
                    cached = renderer?.cachedHit(modelX: x, modelY: y)
                    if let hit = cached, let view,
                       hit.drawableWidth != Int(view.drawableSize.width) || hit.drawableHeight != Int(view.drawableSize.height) {
                        cached = nil
                    }
                }
                emit("cached_hit_sample", ["known": cached != nil, "x": x, "y": y,
                                           "alpha": cached?.sample.alpha as Any? ?? NSNull(),
                                           "region": cached?.sample.semantic?.region as Any? ?? NSNull(),
                                           "rendererFrame": cached?.frameNumber as Any? ?? NSNull(),
                                           "token": activeToken as Any? ?? NSNull(), "viewportEpoch": viewportEpoch])
            case "resize":
                guard let width = command["width"] as? Double, let height = command["height"] as? Double,
                      width.isFinite, height.isFinite, (64...1280).contains(width), (64...1280).contains(height) else {
                    throw RigNativeError.invalid("probe viewport size")
                }
                viewportEpoch &+= 1
                panel?.setContentSize(NSSize(width: width, height: height))
            case "suspend":
                suspended = true; view?.isPaused = true; panel?.orderOut(nil)
                emit("suspended", ["frameNumber": frameNumber])
            case "restore":
                suspended = false; panel?.orderFrontRegardless(); view?.isPaused = false
                emit("restored", ["frameNumber": frameNumber])
            case "status":
                emit("status", ["activeToken": activeToken as Any? ?? NSNull(),
                                "committedToken": committedToken as Any? ?? NSNull(),
                                "committedPilot": committedPilot?.path as Any? ?? NSNull(),
                                "candidateToken": candidate?.token as Any? ?? NSNull(),
                                "preparing": decodeJob != nil, "pendingAppliedToken": pendingAppliedToken as Any? ?? NSNull(),
                                "frameNumber": frameNumber, "suspended": suspended, "renderFault": renderFault,
                                "inputAvailable": renderer != nil && !renderFault && !suspended && pendingAppliedToken == nil,
                                "resources": renderer?.metrics ?? [:]])
            case "faultUnsupportedDrawable":
                guard !suspended, renderer != nil, pendingAppliedToken == nil else {
                    throw RigNativeError.invalid("drawable fault requires an active renderer")
                }
                emit("fault_injected", ["kind": "real rgba16Float drawable outside renderer contract",
                                       "scope": "probe-only API failure, not GPU hardware crash"])
                view?.colorPixelFormat = .rgba16Float
                view?.draw()
            #if RIG_PROBE_FAULTS
            case "faultMetalPipeline":
                guard !suspended, let renderer, pendingAppliedToken == nil else {
                    throw RigNativeError.invalid("pipeline fault requires an active renderer")
                }
                emit("fault_injected", ["kind": "real Metal shader compiler failure during pipeline creation",
                                       "scope": "probe-only GPU API failure, not hardware device removal"])
                renderer.probeFailNextCompositePipeline()
                view?.draw()
            #endif
            case "recover":
                guard let committedPilot, candidate == nil, decodeJob == nil else {
                    throw RigNativeError.invalid("no recoverable committed rig or preparation busy")
                }
                view?.colorPixelFormat = .bgra8Unorm
                emit("rehydrating_committed", ["pilot": committedPilot.path, "oldToken": committedToken as Any? ?? NSNull()])
                prepare(pilot: committedPilot, autoApply: true)
            case "runChecks": try runChecks()
            case "dispose", "terminate": finish()
            default: throw RigNativeError.invalid("unknown command \(name)")
            }
        } catch { emit("command_error", ["message": String(describing: error)]) }
    }

    private func fail(_ error: Error, stage: String, token: Int?) {
        exitStatus = 1
        emit("error", ["stage": stage, "message": String(describing: error), "token": token as Any? ?? NSNull(),
                       "oldActiveRetained": renderer != nil])
        if renderer == nil { finish() }
    }

    private func finish() {
        guard !disposed else { return }
        disposed = true
        generation += 1
        decodeJob?.cancel(); decodeJob = nil
        view?.isPaused = true
        view?.delegate = nil
        let resources = renderer?.metrics ?? [:]
        candidate = nil
        renderer = nil
        panel?.orderOut(nil)
        emit("native_probe_finished", ["frames": frameNumber, "meanDrawMilliseconds": frameNumber > 0 ? drawMilliseconds / Double(frameNumber) : 0,
                                       "lastActiveResources": resources, "metalDeviceAllocatedBytes": device.currentAllocatedSize,
                                       "exitStatus": exitStatus])
        NSApp.terminate(nil)
    }

    func applicationWillTerminate(_ notification: Notification) {
        decodeJob?.cancel()
        try? logFile.close()
        Darwin.exit(exitStatus)
    }
}

var retainedDelegate: NativeRigProbe?
do {
    let options = try NativeProbeOptions.parse()
    let application = NSApplication.shared
    let delegate = try NativeRigProbe(options: options)
    retainedDelegate = delegate
    application.delegate = delegate
    application.run()
} catch {
    FileHandle.standardError.write(Data("native-rig-probe: \(error)\n".utf8))
    Darwin.exit(1)
}
