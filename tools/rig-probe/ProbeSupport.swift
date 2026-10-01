import AppKit
import Darwin
import Foundation
import WebKit

struct ProbeCLIError: Error, CustomStringConvertible {
    let message: String

    var description: String { message }
}

struct ProbeOptions {
    let webRoot: SecureResourceRoot
    let packRoot: SecureResourceRoot
    let outputRoot: URL
    let duration: TimeInterval?
    let autoScenario: Bool

    static func parse(arguments: [String]) throws -> ProbeOptions {
        var webRootPath: String?
        var packRootPath: String?
        var outputPath: String?
        var duration: TimeInterval?
        var autoScenario = true
        var index = 1

        func requireValue(_ name: String, at index: inout Int) throws -> String {
            let next = index + 1
            guard next < arguments.count else {
                throw ProbeCLIError(message: "missing value for \(name)")
            }
            index = next
            return arguments[next]
        }

        while index < arguments.count {
            switch arguments[index] {
            case "--web-root":
                webRootPath = try requireValue("--web-root", at: &index)
            case "--pack-root":
                packRootPath = try requireValue("--pack-root", at: &index)
            case "--output":
                outputPath = try requireValue("--output", at: &index)
            case "--duration":
                let value = try requireValue("--duration", at: &index)
                guard let seconds = Double(value), seconds.isFinite, seconds > 0, seconds <= 86_400 else {
                    throw ProbeCLIError(message: "--duration must be finite and in (0, 86400]")
                }
                duration = seconds
            case "--no-auto-scenario":
                autoScenario = false
            case "--help", "-h":
                throw ProbeCLIError(message: usage)
            default:
                throw ProbeCLIError(message: "unknown argument \(arguments[index])\n\(usage)")
            }
            index += 1
        }

        guard let webRootPath, let packRootPath, let outputPath else {
            throw ProbeCLIError(message: "--web-root, --pack-root, and --output are required\n\(usage)")
        }

        let webRoot = try SecureResourceRoot(path: webRootPath, label: "app")
        let packRoot = try SecureResourceRoot(path: packRootPath, label: "pack")
        let outputRoot = try SecureOutputDirectory(path: outputPath).url
        return ProbeOptions(webRoot: webRoot,
                            packRoot: packRoot,
                            outputRoot: outputRoot,
                            duration: duration,
                            autoScenario: autoScenario)
    }

    static let usage = """
    usage: rig-probe --web-root ABS --pack-root ABS --output ABS [--duration SECONDS] [--no-auto-scenario]

      --web-root ABS       directory containing the browser bundle index.html
      --pack-root ABS      directory served as the pack root (pilot is loaded below it)
      --output ABS         directory for NDJSON evidence and capture artifacts
      --duration SECONDS   bounded automatic exit (1..86400 seconds)
      --no-auto-scenario   wait for JSON commands on stdin instead of running captures automatically

    stdin commands are newline-delimited JSON. Supported commands include load, setParameters,
    capture, sampleHit, runChecks, setVisible, suspend, restore, appkitSnapshot, reload,
    setIgnoresMouseEvents, simulate, and dispose.
    """
}

final class SecureOutputDirectory {
    let url: URL

    init(path: String) throws {
        guard path.hasPrefix("/") else {
            throw ProbeCLIError(message: "output path must be absolute: \(path)")
        }
        let candidate = URL(fileURLWithPath: path, isDirectory: true).standardizedFileURL
        let manager = FileManager.default
        var isDirectory: ObjCBool = false
        if manager.fileExists(atPath: candidate.path, isDirectory: &isDirectory) {
            guard isDirectory.boolValue else {
                throw ProbeCLIError(message: "output path is not a directory: \(candidate.path)")
            }
            guard !SecureResourceRoot.isSymbolicLink(atPath: candidate.path) else {
                throw ProbeCLIError(message: "output path must not be a symbolic link: \(candidate.path)")
            }
        } else {
            do {
                try manager.createDirectory(at: candidate, withIntermediateDirectories: true)
            } catch {
                throw ProbeCLIError(message: "cannot create output directory \(candidate.path): \(error.localizedDescription)")
            }
        }
        self.url = candidate
    }
}

final class SecureResourceRoot {
    let url: URL
    let label: String
    init(path: String, label: String) throws {
        guard path.hasPrefix("/") else {
            throw ProbeCLIError(message: "\(label) root must be absolute: \(path)")
        }
        let candidate = URL(fileURLWithPath: path, isDirectory: true).standardizedFileURL
        let manager = FileManager.default
        var isDirectory: ObjCBool = false
        guard manager.fileExists(atPath: candidate.path, isDirectory: &isDirectory), isDirectory.boolValue else {
            throw ProbeCLIError(message: "\(label) root is not a directory: \(candidate.path)")
        }
        guard !Self.isSymbolicLink(atPath: candidate.path) else {
            throw ProbeCLIError(message: "\(label) root must not be a symbolic link: \(candidate.path)")
        }
        self.url = candidate
        self.label = label
    }

    static func isSymbolicLink(atPath path: String) -> Bool {
        var info = stat()
        let result = path.withCString { pointer in
            lstat(pointer, &info)
        }
        guard result == 0 else { return false }
        return (info.st_mode & S_IFMT) == S_IFLNK
    }

    private static func isDisallowedLink(atPath path: String) -> Bool {
        var info = stat()
        let result = path.withCString { pointer in
            lstat(pointer, &info)
        }
        guard result == 0 else { return false }
        let type = info.st_mode & S_IFMT
        if type == S_IFLNK { return true }
        // A hard-linked regular file can alias content outside the declared tree.
        if type == S_IFREG && info.st_nlink > 1 { return true }
        return false
    }

    func resolve(url requestURL: URL) -> URL? {
        guard requestURL.scheme?.lowercased() == "herdr-pet",
              let host = requestURL.host?.lowercased(), host == label else {
            return nil
        }
        guard requestURL.user == nil, requestURL.password == nil, requestURL.port == nil,
              requestURL.query == nil, requestURL.fragment == nil else {
            return nil
        }

        let encodedPath = requestURL.path(percentEncoded: true)
        guard let decodedPath = encodedPath.removingPercentEncoding,
              !decodedPath.isEmpty,
              !decodedPath.contains("\0"),
              !decodedPath.contains("\\") else {
            return nil
        }

        var components = decodedPath.split(separator: "/", omittingEmptySubsequences: false).map(String.init)
        if components.first == "" { components.removeFirst() }
        if components.last == "" { components.removeLast() }
        guard !components.isEmpty else { return nil }
        guard components.allSatisfy({ component in
            !component.isEmpty && component != "." && component != ".."
        }) else { return nil }

        var candidate = self.url
        for component in components {
            candidate.appendPathComponent(component, isDirectory: false)
        }
        candidate = candidate.standardizedFileURL

        let rootPath = self.url.path.hasSuffix("/") ? self.url.path : self.url.path + "/"
        guard candidate.path == self.url.path || candidate.path.hasPrefix(rootPath) else {
            return nil
        }
        guard !pathContainsDisallowedLink(candidate) else { return nil }

        var isDirectory: ObjCBool = false
        guard FileManager.default.fileExists(atPath: candidate.path, isDirectory: &isDirectory), !isDirectory.boolValue else {
            return nil
        }
        guard !Self.isDisallowedLink(atPath: candidate.path) else { return nil }
        return candidate
    }

    /// lstat-checks only the components below the declared root; OS-level links above it (/tmp, /var) are trusted.
    private func pathContainsDisallowedLink(_ candidate: URL) -> Bool {
        var current = self.url
        for component in candidate.pathComponents.dropFirst(self.url.pathComponents.count) {
            if component == "/" || component.isEmpty { continue }
            current.appendPathComponent(component, isDirectory: false)
            if Self.isDisallowedLink(atPath: current.path) { return true }
        }
        return false
    }
}

final class SecureSchemeHandler: NSObject, WKURLSchemeHandler {
    private let roots: [String: SecureResourceRoot]
    private let maxBytes: Int64 = 128 * 1024 * 1024
    private let lock = NSLock()
    private var activeTasks = Set<ObjectIdentifier>()
    private let onViolation: ((String, String) -> Void)?

    init(roots: [SecureResourceRoot], onViolation: ((String, String) -> Void)? = nil) {
        var map: [String: SecureResourceRoot] = [:]
        for root in roots { map[root.label] = root }
        self.roots = map
        self.onViolation = onViolation
        super.init()
    }

    func webView(_ webView: WKWebView, start urlSchemeTask: WKURLSchemeTask) {
        let taskID = ObjectIdentifier(urlSchemeTask as AnyObject)
        lock.lock()
        activeTasks.insert(taskID)
        lock.unlock()

        let requestURL = urlSchemeTask.request.url
        guard let requestURL,
              let host = requestURL.host?.lowercased(),
              let root = roots[host],
              let fileURL = root.resolve(url: requestURL) else {
            fail(urlSchemeTask,
                 taskID: taskID,
                 code: "resource_denied",
                 message: "URL is outside the declared \(requestURL?.host ?? "unknown") root")
            return
        }

        let values: [FileAttributeKey: Any]
        do {
            values = try FileManager.default.attributesOfItem(atPath: fileURL.path)
        } catch {
            fail(urlSchemeTask, taskID: taskID, code: "resource_stat_failed", message: error.localizedDescription)
            return
        }
        if let size = values[.size] as? NSNumber, size.int64Value > maxBytes {
            fail(urlSchemeTask, taskID: taskID, code: "resource_too_large", message: "resource exceeds 128 MiB")
            return
        }

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self else { return }
            do {
                let data = try Data(contentsOf: fileURL, options: [.mappedIfSafe])
                DispatchQueue.main.async {
                    guard self.isActive(taskID) else { return }
                    let mime = Self.mimeType(for: fileURL.pathExtension)
                    guard let response = HTTPURLResponse(
                        url: requestURL,
                        statusCode: 200,
                        httpVersion: "HTTP/1.1",
                        headerFields: [
                            "Content-Type": mime,
                            "Content-Length": String(data.count),
                            "Access-Control-Allow-Origin": "herdr-pet://app",
                            "Cache-Control": "no-store"
                        ]
                    ) else {
                        self.fail(urlSchemeTask, taskID: taskID, code: "resource_response_failed",
                                  message: "could not construct local resource response")
                        return
                    }
                    urlSchemeTask.didReceive(response)
                    urlSchemeTask.didReceive(data)
                    urlSchemeTask.didFinish()
                    self.remove(taskID)
                }
            } catch {
                DispatchQueue.main.async {
                    self.fail(urlSchemeTask, taskID: taskID, code: "resource_read_failed", message: error.localizedDescription)
                }
            }
        }
    }

    func webView(_ webView: WKWebView, stop urlSchemeTask: WKURLSchemeTask) {
        remove(ObjectIdentifier(urlSchemeTask as AnyObject))
    }

    private func isActive(_ taskID: ObjectIdentifier) -> Bool {
        lock.lock(); defer { lock.unlock() }
        return activeTasks.contains(taskID)
    }

    private func remove(_ taskID: ObjectIdentifier) {
        lock.lock(); activeTasks.remove(taskID); lock.unlock()
    }

    private func fail(_ task: WKURLSchemeTask,
                      taskID: ObjectIdentifier,
                      code: String,
                      message: String) {
        guard isActive(taskID) else { return }
        onViolation?(code, message)
        let error = NSError(domain: "com.herdr.rig-probe.resource",
                            code: 1,
                            userInfo: [NSLocalizedDescriptionKey: message])
        task.didFailWithError(error)
        remove(taskID)
    }

    private static func mimeType(for extensionName: String) -> String {
        switch extensionName.lowercased() {
        case "html", "htm": return "text/html"
        case "css": return "text/css"
        case "js", "cjs": return "application/javascript"
        case "mjs": return "text/javascript"
        case "json", "map": return "application/json"
        case "wasm": return "application/wasm"
        case "png": return "image/png"
        case "jpg", "jpeg": return "image/jpeg"
        case "webp": return "image/webp"
        case "gif": return "image/gif"
        case "svg": return "image/svg+xml"
        case "ico": return "image/x-icon"
        case "txt", "md": return "text/plain"
        case "psd": return "application/octet-stream"
        case "bin", "glsl", "vert", "frag": return "application/octet-stream"
        default: return "application/octet-stream"
        }
    }
}

func jsonLiteral(_ value: Any) -> String? {
    guard JSONSerialization.isValidJSONObject([value]) else { return nil }
    guard let data = try? JSONSerialization.data(withJSONObject: [value], options: []) else { return nil }
    guard let text = String(data: data, encoding: .utf8), text.count >= 2 else { return nil }
    return String(text.dropFirst().dropLast())
}

func jsonDictionary(_ value: Any) -> [String: Any]? {
    if let dictionary = value as? [String: Any] { return dictionary }
    if let dictionary = value as? NSDictionary {
        var result: [String: Any] = [:]
        for (key, item) in dictionary {
            guard let key = key as? String else { return nil }
            result[key] = item
        }
        return result
    }
    if let text = value as? String, let data = text.data(using: .utf8),
       let parsed = try? JSONSerialization.jsonObject(with: data),
       let dictionary = parsed as? [String: Any] {
        return dictionary
    }
    return nil
}

func jsonSafeValue(_ value: Any) -> Any {
    if value is NSNull { return NSNull() }
    if let value = value as? String { return value }
    if let value = value as? NSNumber {
        return value.doubleValue.isFinite ? value : NSNull()
    }
    if let value = value as? [String: Any] {
        return value.mapValues(jsonSafeValue)
    }
    if let value = value as? [Any] {
        return value.map(jsonSafeValue)
    }
    if let value = value as? NSDictionary {
        var result: [String: Any] = [:]
        for (key, item) in value {
            if let key = key as? String { result[key] = jsonSafeValue(item) }
        }
        return result
    }
    return String(describing: value)
}

func jsonOptional<T>(_ value: T?) -> Any {
    value.map { $0 as Any } ?? NSNull()
}

func isoTimestamp() -> String {
    ISO8601DateFormatter().string(from: Date())
}
