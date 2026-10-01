import Foundation
import Darwin

private enum RigDecodeLimits {
    static let maxInputBytes = RigLimits.inputFileBytes
    static let maxPackBytes = RigLimits.packBytes
    static let maxBundleBytes = RigLimits.packBytes
    static let maxOverridesBytes = RigLimits.overrideBytes
    static let maxJSONBytes = RigLimits.jsonBytes
    static let maxRGBABytes = RigLimits.outputBytes
    static let maxResponseBytes = maxJSONBytes + maxRGBABytes + 24
    static let maxStderrBytes = 64 * 1024
    static let maxCanvasWidth = RigLimits.canvasDimension
    static let maxCanvasHeight = RigLimits.canvasDimension
    static let maxCanvasPixels = RigLimits.canvasPixels
    static let maxPSDLayerRecords = RigLimits.psdLayers
    static let maxOutputLayers = RigLimits.outputLayers
    static let maxWarnings = RigLimits.outputLayers * 64
    static let maxWarningLength = 4096
    static let maxNameLength = 512
}

private final class RigBoundedCollector {
    private let limit: Int
    private(set) var data = Data()
    private(set) var overflowed = false
    private(set) var failed = false

    init(limit: Int) {
        self.limit = limit
    }

    func append(_ chunk: Data) {
        guard !chunk.isEmpty else { return }
        let remaining = limit - data.count
        if remaining <= 0 {
            overflowed = true
            return
        }
        if chunk.count > remaining {
            data.append(chunk.prefix(remaining))
            overflowed = true
        } else {
            data.append(chunk)
        }
    }

    func markFailed() {
        failed = true
    }
}

final class RigDecodeJob {
    private let helperURL: URL
    private let bundleURL: URL
    private let baseURL: URL
    private let poseURL: URL?
    private let overridesURL: URL
    private let timeout: TimeInterval
    private let stateLock = NSLock()
    private var ownedProcess: Process?
    private var cancellationRequested = false

    init(helperURL: URL, bundleURL: URL, baseURL: URL, poseURL: URL?, overridesURL: URL, timeout: TimeInterval = TimeInterval(RigLimits.decodeSeconds)) {
        self.helperURL = helperURL
        self.bundleURL = bundleURL
        self.baseURL = baseURL
        self.poseURL = poseURL
        self.overridesURL = overridesURL
        self.timeout = timeout
    }

    func run() throws -> RigDecodedScene {
        try validateURLs()
        let process = Process()
        process.executableURL = helperURL
        var arguments = ["--bundle", bundleURL.path, "--base", baseURL.path]
        if let poseURL { arguments.append(contentsOf: ["--pose", poseURL.path]) }
        arguments.append(contentsOf: ["--overrides", overridesURL.path])
        process.arguments = arguments
        let stdout = Pipe()
        let stderr = Pipe()
        process.standardOutput = stdout
        process.standardError = stderr

        let stdoutCollector = RigBoundedCollector(limit: RigDecodeLimits.maxResponseBytes)
        let stderrCollector = RigBoundedCollector(limit: RigDecodeLimits.maxStderrBytes)
        let readerGroup = DispatchGroup()
        readerGroup.enter()
        DispatchQueue.global(qos: .utility).async {
            Self.drain(stdout.fileHandleForReading, into: stdoutCollector, group: readerGroup)
        }
        readerGroup.enter()
        DispatchQueue.global(qos: .utility).async {
            Self.drain(stderr.fileHandleForReading, into: stderrCollector, group: readerGroup)
        }

        stateLock.lock()
        if cancellationRequested {
            stateLock.unlock()
            stdout.fileHandleForWriting.closeFile()
            stderr.fileHandleForWriting.closeFile()
            readerGroup.wait()
            throw RigNativeError.cancelled
        }
        do {
            try process.run()
            ownedProcess = process
            stdout.fileHandleForWriting.closeFile()
            stderr.fileHandleForWriting.closeFile()
        let cancelBeforeUnlock = cancellationRequested
        stateLock.unlock()
        if cancelBeforeUnlock { terminate(process, hard: false) }
        } catch {
            stateLock.unlock()
            stdout.fileHandleForWriting.closeFile()
            stderr.fileHandleForWriting.closeFile()
            readerGroup.wait()
            throw RigNativeError.unavailable("unable to launch decoder: \(error)")
        }

        let processDone = DispatchSemaphore(value: 0)
        DispatchQueue.global(qos: .utility).async {
            process.waitUntilExit()
            processDone.signal()
        }
        let deadline = DispatchTime.now() + max(0.001, timeout)
        var didTimeout = false
        if processDone.wait(timeout: deadline) == .timedOut {
            didTimeout = true
            terminate(process, hard: true)
            _ = processDone.wait(timeout: .now() + 2)
        }
        readerGroup.wait()
        clearOwnedProcess(process)

        if cancellationWasRequested() { throw RigNativeError.cancelled }
        if didTimeout { throw RigNativeError.timeout }
        if stdoutCollector.overflowed || stdoutCollector.failed {
            throw RigNativeError.invalid("decoder stdout exceeded bounded response envelope")
        }
        if stderrCollector.overflowed || stderrCollector.failed {
            throw RigNativeError.invalid("decoder stderr exceeded bounded diagnostic output")
        }
        guard process.terminationStatus == 0 else {
            let cause = process.terminationReason == .uncaughtSignal
                ? "decoder terminated by signal \(process.terminationStatus)"
                : "decoder exited with status \(process.terminationStatus)"
            let diagnostic = String(data: stderrCollector.data, encoding: .utf8)?
                .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            throw RigNativeError.unavailable(diagnostic.isEmpty ? cause : "\(cause): \(diagnostic.prefix(RigDecodeLimits.maxStderrBytes))")
        }
        return try Self.parseEnvelope(stdoutCollector.data, poseWasRequested: poseURL != nil)
    }

    func cancel() {
        stateLock.lock()
        cancellationRequested = true
        let process = ownedProcess
        stateLock.unlock()
        if let process { terminate(process, hard: false) }
    }

    private func validateURLs() throws {
        let urls = [helperURL, bundleURL, baseURL, poseURL, overridesURL].compactMap { $0 }
        guard urls.allSatisfy({ $0.isFileURL && !$0.path.contains("\0") }) else {
            throw RigNativeError.invalid("decoder paths must be local file URLs")
        }
        guard FileManager.default.isExecutableFile(atPath: helperURL.path) else {
            throw RigNativeError.unavailable("decoder helper is not executable")
        }
        _ = try boundedFileSize(bundleURL, limit: RigDecodeLimits.maxBundleBytes, label: "decoder bundle")
        let baseBytes = try boundedFileSize(baseURL, limit: RigDecodeLimits.maxInputBytes, label: "base PSD")
        let poseBytes = try poseURL.map { try boundedFileSize($0, limit: RigDecodeLimits.maxInputBytes, label: "pose PSD") } ?? 0
        _ = try boundedFileSize(overridesURL, limit: RigDecodeLimits.maxOverridesBytes, label: "overrides")
        guard baseBytes <= RigDecodeLimits.maxPackBytes - poseBytes else {
            throw RigNativeError.invalid("combined PSD input bytes exceed cap")
        }
    }

    private func boundedFileSize(_ url: URL, limit: Int, label: String) throws -> Int {
        guard let attributes = try? FileManager.default.attributesOfItem(atPath: url.path),
              let type = attributes[.type] as? FileAttributeType,
              type == .typeRegular,
              let number = attributes[.size] as? NSNumber,
              number.int64Value >= 0,
              number.int64Value <= Int64(limit) else {
            throw RigNativeError.invalid("\(label) is not a bounded regular file")
        }
        return Int(number.int64Value)
    }

    private func clearOwnedProcess(_ process: Process) {
        stateLock.lock()
        if ownedProcess === process { ownedProcess = nil }
        stateLock.unlock()
    }

    private func cancellationWasRequested() -> Bool {
        stateLock.lock()
        defer { stateLock.unlock() }
        return cancellationRequested
    }

    private func terminate(_ process: Process, hard: Bool) {
        guard process.isRunning else { return }
        process.terminate()
        guard hard else { return }
        let gracefulDeadline = Date().addingTimeInterval(0.25)
        while process.isRunning && Date() < gracefulDeadline {
            usleep(10_000)
        }
        guard process.isRunning else { return }
        let pid = process.processIdentifier
        if pid > 0 { _ = Darwin.kill(pid, SIGKILL) }
        let reapDeadline = Date().addingTimeInterval(1)
        while process.isRunning && Date() < reapDeadline {
            usleep(10_000)
        }
    }

    private static func drain(_ handle: FileHandle, into collector: RigBoundedCollector, group: DispatchGroup) {
        defer { group.leave() }
        while true {
            do {
                guard let chunk = try handle.read(upToCount: 64 * 1024), !chunk.isEmpty else { return }
                collector.append(chunk)
            } catch {
                collector.markFailed()
                return
            }
        }
    }

    private static func parseEnvelope(_ response: Data, poseWasRequested: Bool) throws -> RigDecodedScene {
        guard response.count >= 24 else { throw RigNativeError.invalid("decoder response is truncated") }
        let magic = Data(response.prefix(8))
        guard magic == Data("HRDRRIG1".utf8) else { throw RigNativeError.invalid("decoder response magic is invalid") }
        let jsonLength = readUInt64LE(response, offset: 8)
        let rgbaLength = readUInt64LE(response, offset: 16)
        guard jsonLength <= UInt64(RigDecodeLimits.maxJSONBytes), rgbaLength <= UInt64(RigDecodeLimits.maxRGBABytes) else {
            throw RigNativeError.invalid("decoder response lengths exceed caps")
        }
        let header = UInt64(24)
        let (jsonEnd, jsonOverflow) = header.addingReportingOverflow(jsonLength)
        let (responseEnd, responseOverflow) = jsonEnd.addingReportingOverflow(rgbaLength)
        guard !jsonOverflow, !responseOverflow, responseEnd == UInt64(response.count), jsonEnd <= UInt64(response.count) else {
            throw RigNativeError.invalid("decoder response envelope length is inconsistent")
        }
        let jsonStart = 24
        let jsonEndInt = Int(jsonEnd)
        let rgbaStart = jsonEndInt
        let rgbaEnd = Int(responseEnd)
        let json = response.subdata(in: jsonStart..<jsonEndInt)
        let rgba = response.subdata(in: rgbaStart..<rgbaEnd)
        guard let document = try? JSONDecoder().decode(RigDecodedDocument.self, from: json) else {
            throw RigNativeError.invalid("decoder JSON does not match RigDecodedDocument")
        }
        try validate(document, rgbaBytes: rgba.count, poseWasRequested: poseWasRequested)
        return RigDecodedScene(document: document, rgba: rgba)
    }

    private static func readUInt64LE(_ data: Data, offset: Int) -> UInt64 {
        var value: UInt64 = 0
        for index in 0..<8 {
            value |= UInt64(data[offset + index]) << UInt64(index * 8)
        }
        return value
    }

    private static func validate(_ document: RigDecodedDocument, rgbaBytes: Int, poseWasRequested: Bool) throws {
        guard document.format == "herdr.rig.decoded", document.version == RigLimits.version else {
            throw RigNativeError.invalid("decoded document format/version is unsupported")
        }
        guard (document.pose != nil) == poseWasRequested else {
            throw RigNativeError.invalid("decoded pose presence does not match request")
        }
        try validateDefinition(document.base, label: "base", rgbaBytes: rgbaBytes)
        if let pose = document.pose { try validateDefinition(pose, label: "pose", rgbaBytes: rgbaBytes) }
        if let physics = document.physics {
            try validatePhysics(physics)
        }
        try validateDiagnostics(document.diagnostics)
    }

    private static func validateDiagnostics(_ diagnostics: RigDecodeDiagnostics) throws {
        guard diagnostics.basePSDLayerCount >= 1,
              diagnostics.basePSDLayerCount <= RigDecodeLimits.maxPSDLayerRecords,
              diagnostics.posePSDLayerCount >= 0,
              diagnostics.posePSDLayerCount <= RigDecodeLimits.maxPSDLayerRecords,
              diagnostics.baseMissingRequired.count <= RigDecodeLimits.maxWarnings,
              diagnostics.poseMissingRequired.count <= RigDecodeLimits.maxWarnings,
              diagnostics.warnings.count <= RigDecodeLimits.maxWarnings else {
            throw RigNativeError.invalid("decoded diagnostics are invalid")
        }
        for missing in diagnostics.baseMissingRequired + diagnostics.poseMissingRequired {
            guard !missing.isEmpty, missing.utf8.count <= RigDecodeLimits.maxNameLength else {
                throw RigNativeError.invalid("decoded missing-layer diagnostics are invalid")
            }
        }
        for warning in diagnostics.warnings {
            guard !warning.isEmpty, warning.utf8.count <= RigDecodeLimits.maxWarningLength else {
                throw RigNativeError.invalid("decoded warning diagnostics are invalid")
            }
        }
    }

    private static func validateDefinition(_ rig: RigDefinition, label: String, rgbaBytes: Int) throws {
        guard rig.canvas.w >= 1, rig.canvas.w <= RigDecodeLimits.maxCanvasWidth,
              rig.canvas.h >= 1, rig.canvas.h <= RigDecodeLimits.maxCanvasHeight,
              rig.canvas.w <= RigDecodeLimits.maxCanvasPixels / max(1, rig.canvas.h),
              !rig.layers.isEmpty, rig.layers.count <= RigDecodeLimits.maxOutputLayers else {
            throw RigNativeError.invalid("\(label) canvas/layer bounds are invalid")
        }
        let required = ["face", "eyewhite", "irides", "eyelash"]
        for semantic in required where !rig.layers.contains(where: { layer in
            let name = layer.name.lowercased()
            return name == semantic || name.hasPrefix("\(semantic)_")
        }) {
            throw RigNativeError.invalid("\(label) required semantic layer \(semantic) is missing")
        }
        for (index, layer) in rig.layers.enumerated() {
            guard !layer.name.isEmpty, layer.name.utf8.count <= RigDecodeLimits.maxNameLength,
                  layer.w > 0, layer.h > 0,
                  [layer.x, layer.y, layer.w, layer.h, layer.z, layer.depth].allSatisfy(\.isFinite),
                  layer.img.width > 0, layer.img.height > 0 else {
                throw RigNativeError.invalid("\(label) layer \(index) geometry is invalid")
            }
            let (pixels, pixelOverflow) = layer.img.width.multipliedReportingOverflow(by: layer.img.height)
            let (expectedLength, lengthOverflow) = pixels.multipliedReportingOverflow(by: 4)
            guard !pixelOverflow, !lengthOverflow,
                  layer.img.width <= RigDecodeLimits.maxCanvasWidth,
                  layer.img.height <= RigDecodeLimits.maxCanvasHeight,
                  layer.img.length == expectedLength,
                  layer.img.offset >= 0, layer.img.length >= 0,
                  layer.img.offset <= rgbaBytes,
                  layer.img.length <= rgbaBytes - layer.img.offset else {
                throw RigNativeError.invalid("\(label) layer \(index) image range is invalid")
            }
        }
    }

    private static func validatePhysics(_ physics: RigHairPhysicsConfig) throws {
        let tuning = [physics.frontHair, physics.backHair]
        for item in tuning {
            guard [item.amplitude, item.stiffness, item.damping, item.wind, item.inertia, item.rootLock, item.maxOffset].allSatisfy(\.isFinite) else {
                throw RigNativeError.invalid("hair physics contains non-finite values")
            }
        }
        guard physics.layers.count <= RigDecodeLimits.maxOutputLayers else {
            throw RigNativeError.invalid("hair physics layer tuning exceeds cap")
        }
        for (name, values) in physics.layers {
            guard !name.isEmpty, name.utf8.count <= RigDecodeLimits.maxNameLength, values.count <= 7 else {
                throw RigNativeError.invalid("hair physics layer tuning is invalid")
            }
            guard values.values.allSatisfy(\.isFinite) else {
                throw RigNativeError.invalid("hair physics layer tuning contains non-finite values")
            }
        }
    }
}
