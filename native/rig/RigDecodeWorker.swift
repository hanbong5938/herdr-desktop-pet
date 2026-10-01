import Foundation
import JavaScriptCore
import Darwin

private enum WorkerError: Error, CustomStringConvertible {
    case invalid(String)
    case unavailable(String)

    var description: String {
        switch self {
        case .invalid(let detail): return "RIG_WORKER_INVALID: \(detail)"
        case .unavailable(let detail): return "RIG_WORKER_UNAVAILABLE: \(detail)"
        }
    }
}

private enum WorkerLimits {
    static let maxInputBytes = RigLimits.inputFileBytes
    static let maxPackBytes = RigLimits.packBytes
    static let maxBundleBytes = RigLimits.packBytes
    static let maxOverridesBytes = RigLimits.overrideBytes
    static let maxJSONBytes = RigLimits.jsonBytes
    static let maxRGBABytes = RigLimits.outputBytes
    static let maxResponseBytes = maxJSONBytes + maxRGBABytes + 24
    static let maxCanvasWidth = RigLimits.canvasDimension
    static let maxCanvasHeight = RigLimits.canvasDimension
    static let maxCanvasPixels = RigLimits.canvasPixels
    static let maxPSDLayerRecords = RigLimits.psdLayers
    static let maxOutputLayers = RigLimits.outputLayers
    static let maxWarnings = RigLimits.outputLayers * 64
    static let maxWarningLength = 4096
    static let maxNameLength = 512
    static let maxErrorBytes = 64 * 1024
}

private struct WorkerOptions {
    let bundleURL: URL
    let baseURL: URL
    let poseURL: URL?
    let overridesURL: URL
}

@main
struct RigDecodeWorker {
    static func main() {
        do {
            try run()
        } catch {
            writeError(String(describing: error))
            exit(1)
        }
    }

    private static func run() throws {
        let options = try parseArguments(Array(CommandLine.arguments.dropFirst()))
        let bundleData = try readBounded(options.bundleURL, limit: WorkerLimits.maxBundleBytes, label: "decoder bundle")
        guard let bundle = String(data: bundleData, encoding: .utf8), !bundle.isEmpty else {
            throw WorkerError.invalid("decoder bundle is not UTF-8")
        }
        let baseData = try readBounded(options.baseURL, limit: WorkerLimits.maxInputBytes, label: "base PSD")
        let poseData = try options.poseURL.map { try readBounded($0, limit: WorkerLimits.maxInputBytes, label: "pose PSD") }
        guard baseData.count <= WorkerLimits.maxPackBytes - (poseData?.count ?? 0) else {
            throw WorkerError.invalid("combined PSD input bytes exceed cap")
        }
        let overridesData = try readBounded(options.overridesURL, limit: WorkerLimits.maxOverridesBytes, label: "overrides")
        guard let overrides = String(data: overridesData, encoding: .utf8), !overrides.isEmpty else {
            throw WorkerError.invalid("overrides are not UTF-8")
        }

        guard let context = JSContext() else {
            throw WorkerError.unavailable("JavaScriptCore context unavailable")
        }
        var scriptException: String?
        context.exceptionHandler = { _, exception in
            scriptException = exception?.toString()
        }
        _ = context.evaluateScript(bundle, withSourceURL: options.bundleURL)
        if let scriptException {
            throw WorkerError.invalid("decoder bundle evaluation failed: \(scriptException)")
        }

        guard let contextRef = context.jsGlobalContextRef else {
            throw WorkerError.unavailable("JavaScriptCore C context unavailable")
        }
        let global = JSContextGetGlobalObject(contextRef)
        let decodeName = JSStringCreateWithUTF8CString("__herdrRigDecode")
        defer { JSStringRelease(decodeName) }
        var propertyException: JSValueRef?
        guard let decodeValue = JSObjectGetProperty(contextRef, global, decodeName, &propertyException),
              JSValueIsObject(contextRef, decodeValue),
              let decodeFunction = JSValueToObject(contextRef, decodeValue, &propertyException) else {
            throw WorkerError.invalid("decoder entrypoint is missing")
        }

        var protectedValues: [JSValueRef] = []
        func protect(_ value: JSValueRef) -> JSValueRef {
            JSValueProtect(contextRef, value)
            protectedValues.append(value)
            return value
        }
        defer { for value in protectedValues { JSValueUnprotect(contextRef, value) } }
        let baseValue = protect(try makeByteArray(baseData, context: contextRef))
        let poseValue = try poseData.map { protect(try makeByteArray($0, context: contextRef)) }
        let baseName = protect(try makeStringValue(options.baseURL.lastPathComponent, context: contextRef))
        let poseName = try options.poseURL.map { protect(try makeStringValue($0.lastPathComponent, context: contextRef)) }
        let overridesValue = protect(try makeStringValue(overrides, context: contextRef))
        let nullValue = JSValueMakeNull(contextRef)
        let arguments: [JSValueRef?] = [
            baseValue,
            baseName,
            poseValue ?? nullValue,
            poseName ?? nullValue,
            overridesValue,
        ]
        var callException: JSValueRef?
        guard let resultValue = arguments.withUnsafeBufferPointer({ pointer in
            JSObjectCallAsFunction(contextRef, decodeFunction, global, pointer.count, pointer.baseAddress, &callException)
        }),
        callException == nil,
        let resultObject = JSValueToObject(contextRef, resultValue, &callException) else {
            throw WorkerError.invalid("decoder call failed: \(jsException(callException, context: contextRef))")
        }
        _ = protect(resultValue)

        let jsonName = JSStringCreateWithUTF8CString("json")
        let rgbaName = JSStringCreateWithUTF8CString("rgba")
        defer {
            JSStringRelease(jsonName)
            JSStringRelease(rgbaName)
        }
        guard let jsonValue = JSObjectGetProperty(contextRef, resultObject, jsonName, &callException), callException == nil,
              let rgbaValue = JSObjectGetProperty(contextRef, resultObject, rgbaName, &callException), callException == nil else {
            throw WorkerError.invalid("decoder result fields are missing")
        }
        let jsonData = try copyUTF8String(jsonValue, context: contextRef)
        guard jsonData.count <= WorkerLimits.maxJSONBytes else { throw WorkerError.invalid("decoded JSON exceeds cap") }
        let rgbaData = try copyTypedBytes(rgbaValue, context: contextRef)
        guard rgbaData.count <= WorkerLimits.maxRGBABytes else { throw WorkerError.invalid("decoded RGBA exceeds cap") }
        guard jsonData.count + rgbaData.count <= WorkerLimits.maxResponseBytes else {
            throw WorkerError.invalid("decoded response exceeds cap")
        }
        let document = try JSONDecoder().decode(RigDecodedDocument.self, from: jsonData)
        try validate(document, rgbaBytes: rgbaData.count, poseWasRequested: poseData != nil)
        try writeEnvelope(json: jsonData, rgba: rgbaData)
    }

    private static func parseArguments(_ arguments: [String]) throws -> WorkerOptions {
        var bundlePath: String?
        var basePath: String?
        var posePath: String?
        var overridesPath: String?
        var index = 0
        while index < arguments.count {
            let option = arguments[index]
            guard index + 1 < arguments.count else { throw WorkerError.invalid("missing value for \(option)") }
            let value = arguments[index + 1]
            guard !value.isEmpty, !value.contains("\0") else { throw WorkerError.invalid("invalid path value") }
            switch option {
            case "--bundle":
                guard bundlePath == nil else { throw WorkerError.invalid("duplicate --bundle") }
                bundlePath = value
            case "--base":
                guard basePath == nil else { throw WorkerError.invalid("duplicate --base") }
                basePath = value
            case "--pose":
                guard posePath == nil else { throw WorkerError.invalid("duplicate --pose") }
                posePath = value
            case "--overrides":
                guard overridesPath == nil else { throw WorkerError.invalid("duplicate --overrides") }
                overridesPath = value
            default:
                throw WorkerError.invalid("unknown option \(option)")
            }
            index += 2
        }
        guard let bundlePath, let basePath, let overridesPath else {
            throw WorkerError.invalid("required options are --bundle, --base and --overrides")
        }
        return WorkerOptions(
            bundleURL: URL(fileURLWithPath: bundlePath),
            baseURL: URL(fileURLWithPath: basePath),
            poseURL: posePath.map { URL(fileURLWithPath: $0) },
            overridesURL: URL(fileURLWithPath: overridesPath),
        )
    }

    private static func readBounded(_ url: URL, limit: Int, label: String) throws -> Data {
        guard url.isFileURL, !url.path.contains("\0") else { throw WorkerError.invalid("\(label) is not a file URL") }
        let descriptor = Darwin.open(url.path, O_RDONLY | O_CLOEXEC | O_NOFOLLOW | O_NONBLOCK)
        guard descriptor >= 0 else { throw WorkerError.invalid("\(label) cannot be opened without following links") }
        let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        defer { try? handle.close() }
        var before = stat()
        guard fstat(descriptor, &before) == 0, (before.st_mode & S_IFMT) == S_IFREG,
              before.st_nlink == 1, before.st_size >= 0, before.st_size <= limit else {
            throw WorkerError.invalid("\(label) is not a bounded unlinked regular file")
        }
        let size = Int(before.st_size)
        var data = Data(capacity: size)
        while data.count < size {
            let amount = min(1024 * 1024, size - data.count)
            guard let chunk = try handle.read(upToCount: amount), !chunk.isEmpty else {
                throw WorkerError.invalid("\(label) changed while reading")
            }
            data.append(chunk)
        }
        var after = stat()
        guard fstat(descriptor, &after) == 0, before.st_size == after.st_size,
              before.st_mtimespec.tv_sec == after.st_mtimespec.tv_sec,
              before.st_mtimespec.tv_nsec == after.st_mtimespec.tv_nsec,
              before.st_ctimespec.tv_sec == after.st_ctimespec.tv_sec,
              before.st_ctimespec.tv_nsec == after.st_ctimespec.tv_nsec else {
            throw WorkerError.invalid("\(label) changed while reading")
        }
        return data
    }

    private static func makeByteArray(_ data: Data, context: JSContextRef) throws -> JSObjectRef {
        var exception: JSValueRef?
        guard let array = JSObjectMakeTypedArray(context, kJSTypedArrayTypeUint8Array, data.count, &exception),
              let destination = JSObjectGetTypedArrayBytesPtr(context, array, &exception), exception == nil else {
            throw WorkerError.unavailable("cannot allocate JavaScript byte array")
        }
        data.withUnsafeBytes { source in
            guard let sourceBase = source.baseAddress, data.count > 0 else { return }
            destination.copyMemory(from: sourceBase, byteCount: data.count)
        }
        return array
    }

    private static func makeStringValue(_ value: String, context: JSContextRef) throws -> JSValueRef {
        guard !value.contains("\0") else { throw WorkerError.invalid("string contains NUL") }
        let string = value.withCString { JSStringCreateWithUTF8CString($0) }
        defer { JSStringRelease(string) }
        guard let string else { throw WorkerError.invalid("cannot allocate JavaScript string") }
        return JSValueMakeString(context, string)
    }

    private static func copyUTF8String(_ value: JSValueRef, context: JSContextRef) throws -> Data {
        var exception: JSValueRef?
        guard let string = JSValueToStringCopy(context, value, &exception), exception == nil else {
            throw WorkerError.invalid("decoder JSON is not a string")
        }
        defer { JSStringRelease(string) }
        let maximum = JSStringGetMaximumUTF8CStringSize(string)
        guard maximum > 0, maximum <= WorkerLimits.maxJSONBytes * 3 + 1 else {
            throw WorkerError.invalid("decoder JSON UTF-8 allocation exceeds cap")
        }
        var bytes = Data(count: maximum)
        let written = bytes.withUnsafeMutableBytes { pointer in
            JSStringGetUTF8CString(string, pointer.bindMemory(to: CChar.self).baseAddress, maximum)
        }
        guard written > 0, written - 1 <= WorkerLimits.maxJSONBytes else {
            throw WorkerError.invalid("decoder JSON UTF-8 conversion exceeds cap")
        }
        bytes.count = written - 1
        return bytes
    }

    private static func copyTypedBytes(_ value: JSValueRef, context: JSContextRef) throws -> Data {
        var exception: JSValueRef?
        guard let object = JSValueToObject(context, value, &exception), exception == nil else {
            throw WorkerError.invalid("decoder RGBA is not an object")
        }
        let length = JSObjectGetTypedArrayByteLength(context, object, &exception)
        guard exception == nil, length > 0, length <= WorkerLimits.maxRGBABytes,
              let bytes = JSObjectGetTypedArrayBytesPtr(context, object, &exception), exception == nil else {
            throw WorkerError.invalid("decoder RGBA is not a bounded typed array")
        }
        return Data(bytes: bytes, count: length)
    }

    private static func validate(_ document: RigDecodedDocument, rgbaBytes: Int, poseWasRequested: Bool) throws {
        guard document.format == "herdr.rig.decoded", document.version == RigLimits.version else {
            throw WorkerError.invalid("decoded document format/version is unsupported")
        }
        guard (document.pose != nil) == poseWasRequested else {
            throw WorkerError.invalid("decoded pose presence does not match request")
        }
        try validateDefinition(document.base, label: "base", rgbaBytes: rgbaBytes)
        if let pose = document.pose { try validateDefinition(pose, label: "pose", rgbaBytes: rgbaBytes) }
        if let physics = document.physics { try validatePhysics(physics) }
        try validateDiagnostics(document.diagnostics)
    }

    private static func validateDiagnostics(_ diagnostics: RigDecodeDiagnostics) throws {
        guard diagnostics.basePSDLayerCount >= 1,
              diagnostics.basePSDLayerCount <= WorkerLimits.maxPSDLayerRecords,
              diagnostics.posePSDLayerCount >= 0,
              diagnostics.posePSDLayerCount <= WorkerLimits.maxPSDLayerRecords,
              diagnostics.baseMissingRequired.count <= WorkerLimits.maxWarnings,
              diagnostics.poseMissingRequired.count <= WorkerLimits.maxWarnings,
              diagnostics.warnings.count <= WorkerLimits.maxWarnings else {
            throw WorkerError.invalid("decode diagnostics are invalid")
        }
        for missing in diagnostics.baseMissingRequired + diagnostics.poseMissingRequired {
            guard !missing.isEmpty, missing.utf8.count <= WorkerLimits.maxNameLength else {
                throw WorkerError.invalid("decoded missing-layer diagnostics are invalid")
            }
        }
        for warning in diagnostics.warnings {
            guard !warning.isEmpty, warning.utf8.count <= WorkerLimits.maxWarningLength else {
                throw WorkerError.invalid("decoded warning diagnostics are invalid")
            }
        }
    }

    private static func validateDefinition(_ rig: RigDefinition, label: String, rgbaBytes: Int) throws {
        guard rig.canvas.w >= 1, rig.canvas.w <= WorkerLimits.maxCanvasWidth,
              rig.canvas.h >= 1, rig.canvas.h <= WorkerLimits.maxCanvasHeight,
              rig.canvas.w <= WorkerLimits.maxCanvasPixels / max(1, rig.canvas.h),
              !rig.layers.isEmpty, rig.layers.count <= WorkerLimits.maxOutputLayers else {
            throw WorkerError.invalid("\(label) canvas/layer bounds are invalid")
        }
        let required = ["face", "eyewhite", "irides", "eyelash"]
        for semantic in required where !rig.layers.contains(where: { name in
            let normalized = name.name.lowercased()
            return normalized == semantic || normalized.hasPrefix("\(semantic)_")
        }) {
            throw WorkerError.invalid("\(label) required semantic layer \(semantic) is missing")
        }
        for (index, layer) in rig.layers.enumerated() {
            guard !layer.name.isEmpty, layer.name.utf8.count <= WorkerLimits.maxNameLength,
                  layer.w > 0, layer.h > 0,
                  [layer.x, layer.y, layer.w, layer.h, layer.z, layer.depth].allSatisfy(\.isFinite),
                  layer.img.width > 0, layer.img.height > 0 else {
                throw WorkerError.invalid("\(label) layer \(index) geometry is invalid")
            }
            let (pixels, pixelOverflow) = layer.img.width.multipliedReportingOverflow(by: layer.img.height)
            let (expectedLength, lengthOverflow) = pixels.multipliedReportingOverflow(by: 4)
            guard !pixelOverflow, !lengthOverflow,
                  layer.img.width <= WorkerLimits.maxCanvasWidth,
                  layer.img.height <= WorkerLimits.maxCanvasHeight,
                  layer.img.length == expectedLength,
                  layer.img.offset >= 0, layer.img.length >= 0,
                  layer.img.offset <= rgbaBytes,
                  layer.img.length <= rgbaBytes - layer.img.offset else {
                throw WorkerError.invalid("\(label) layer \(index) image range is invalid")
            }
        }
    }
    private static func validatePhysics(_ physics: RigHairPhysicsConfig) throws {
        let tuning = [physics.frontHair, physics.backHair]
        for item in tuning {
            guard [item.amplitude, item.stiffness, item.damping, item.wind, item.inertia, item.rootLock, item.maxOffset].allSatisfy(\.isFinite) else {
                throw WorkerError.invalid("hair physics contains non-finite values")
            }
        }
        guard physics.layers.count <= WorkerLimits.maxOutputLayers else {
            throw WorkerError.invalid("hair physics layer tuning exceeds cap")
        }
        for (name, values) in physics.layers {
            guard !name.isEmpty, name.utf8.count <= WorkerLimits.maxNameLength, values.count <= 7,
                  values.values.allSatisfy(\.isFinite) else {
                throw WorkerError.invalid("hair physics layer tuning is invalid")
            }
        }
    }


    private static func writeEnvelope(json: Data, rgba: Data) throws {
        var header = Data("HRDRRIG1".utf8)
        var jsonLength = UInt64(json.count).littleEndian
        var rgbaLength = UInt64(rgba.count).littleEndian
        header.append(Data(bytes: &jsonLength, count: MemoryLayout<UInt64>.size))
        header.append(Data(bytes: &rgbaLength, count: MemoryLayout<UInt64>.size))
        let output = FileHandle.standardOutput
        try output.write(contentsOf: header)
        try output.write(contentsOf: json)
        try output.write(contentsOf: rgba)
    }

    private static func jsException(_ exception: JSValueRef?, context: JSContextRef) -> String {
        guard let exception else { return "unknown JavaScript exception" }
        var conversionException: JSValueRef?
        guard let string = JSValueToStringCopy(context, exception, &conversionException), conversionException == nil else {
            return "unknown JavaScript exception"
        }
        let maximum = JSStringGetMaximumUTF8CStringSize(string)
        guard maximum > 0, maximum <= WorkerLimits.maxErrorBytes * 3 + 1 else {
            return "JavaScript exception exceeds diagnostic cap"
        }
        var bytes = [CChar](repeating: 0, count: maximum)
        let written = bytes.withUnsafeMutableBufferPointer { pointer in
            JSStringGetUTF8CString(string, pointer.baseAddress, maximum)
        }
        guard written > 1 else { return "unknown JavaScript exception" }
        return String(decoding: bytes.prefix(written - 1).map { UInt8(bitPattern: $0) }, as: UTF8.self)
    }

    private static func writeError(_ message: String) {
        let bounded = String(decoding: message.utf8.prefix(WorkerLimits.maxErrorBytes), as: UTF8.self)
        let data = Data((bounded + "\n").utf8)
        try? FileHandle.standardError.write(contentsOf: data)
    }
}
