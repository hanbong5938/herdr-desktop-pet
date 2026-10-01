import AppKit
import Darwin
import Foundation
import WebKit

final class TransparentWebView: WKWebView {
    // This is the public NSView opacity property. No private WebKit KVC is used.
    override var isOpaque: Bool {
        false
    }

    override var wantsUpdateLayer: Bool { true }

    override func updateLayer() {
        super.updateLayer()
        layer?.backgroundColor = NSColor.clear.cgColor
    }
}

final class RigProbeApplicationDelegate: NSObject, NSApplicationDelegate, WKNavigationDelegate, WKUIDelegate, WKScriptMessageHandler {
    private let options: ProbeOptions
    private var panel: NSPanel?
    private var webView: TransparentWebView?
    private var configuration: WKWebViewConfiguration?
    private var schemeHandler: SecureSchemeHandler?
    private var contentRuleList: WKContentRuleList?
    private var stdinStarted = false

    private let appURL = URL(string: "herdr-pet://app/index.html")!
    private let defaultPackURL = "herdr-pet://pack/pilot/"
    private var currentPackURL: String
    private var bridgeReady = false
    private var candidateLoaded = false
    private var inputEnabled = false
    private var disposed = false
    private var scenarioStarted = false
    private var pageEpoch = 0
    private var tokenCounter = 0
    private var captureCounter = 0
    private var recoveryCount = 0
    private let recoveryLimit = 2
    private var activeLoadToken: String?
    private var pendingCommands: [String: (method: String, commandID: String)] = [:]
    private var scheduledWork: [String: DispatchWorkItem] = [:]
    private let outputLock = NSLock()

    init(options: ProbeOptions) {
        self.options = options
        self.currentPackURL = defaultPackURL
        super.init()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        emit("probe_started", [
            "webRoot": options.webRoot.url.path,
            "packRoot": options.packRoot.url.path,
            "outputRoot": options.outputRoot.path,
            "durationSeconds": jsonOptional(options.duration),
            "autoScenario": options.autoScenario,
            "resourceScheme": "herdr-pet",
            "resourcePolicy": "declared-roots-only"
        ])
        emitRuntimePermissions()
        guard preflightRoots() else {
            terminateSoon()
            return
        }

        configurePanel()
        startStdinReader()
        if let duration = options.duration {
            schedule(after: duration, label: "duration") { [weak self] in
                guard let self else { return }
                self.emit("duration_elapsed", ["seconds": duration, "bounded": true])
                NSApp.terminate(nil)
            }
        }
        compileNetworkRulesAndCreateWebView()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    func applicationWillTerminate(_ notification: Notification) {
        configuration?.userContentController.removeScriptMessageHandler(forName: "rigProbe")
        cancelAllScheduledWork()
        emit("probe_terminated", [
            "recoveryCount": recoveryCount,
            "candidateLoaded": candidateLoaded,
            "inputEnabled": inputEnabled
        ])
    }

    // MARK: - AppKit and WebKit setup

    private func configurePanel() {
        let size = NSSize(width: 384, height: 512)
        let rect = NSRect(origin: .zero, size: size)
        let style: NSWindow.StyleMask = [.borderless, .nonactivatingPanel, .resizable]
        let newPanel = NSPanel(contentRect: rect,
                               styleMask: style,
                               backing: .buffered,
                               defer: false)
        newPanel.isOpaque = false
        newPanel.backgroundColor = .clear
        newPanel.hasShadow = false
        newPanel.alphaValue = 1
        newPanel.level = .floating
        newPanel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        newPanel.titleVisibility = .hidden
        newPanel.titlebarAppearsTransparent = true
        newPanel.isMovableByWindowBackground = true
        newPanel.acceptsMouseMovedEvents = true
        newPanel.ignoresMouseEvents = false
        newPanel.isReleasedWhenClosed = false

        let content = NSView(frame: rect)
        content.wantsLayer = true
        content.layer?.backgroundColor = NSColor.clear.cgColor
        newPanel.contentView = content
        if let screen = NSScreen.main {
            let visible = screen.visibleFrame
            newPanel.setFrameOrigin(NSPoint(x: visible.midX - size.width / 2,
                                            y: visible.midY - size.height / 2))
        }
        panel = newPanel
        emitTransparencyFacts(stage: "panel_configured", snapshotSaved: false)
    }

    private func compileNetworkRulesAndCreateWebView() {
        guard let contentView = panel?.contentView else {
            emit("error", ["stage": "panel", "message": "panel content view unavailable"])
            terminateSoon()
            return
        }

        let userContentController = WKUserContentController()
        userContentController.add(self, name: "rigProbe")
        userContentController.addUserScript(WKUserScript(source: Self.networkGuardScript,
                                                         injectionTime: .atDocumentStart,
                                                         forMainFrameOnly: false))

        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        config.userContentController = userContentController
        config.defaultWebpagePreferences.allowsContentJavaScript = true

        let handler = SecureSchemeHandler(roots: [options.webRoot, options.packRoot]) { [weak self] code, message in
            self?.emit("resource_denied", ["code": code, "message": message])
        }
        schemeHandler = handler
        config.setURLSchemeHandler(handler, forURLScheme: "herdr-pet")
        configuration = config

        let rules = """
        [
          {"trigger":{"url-filter":"^https?:"},"action":{"type":"block"}},
          {"trigger":{"url-filter":"^wss?:"},"action":{"type":"block"}},
          {"trigger":{"url-filter":"^ftp:"},"action":{"type":"block"}}
        ]
        """
        WKContentRuleListStore.default().compileContentRuleList(forIdentifier: "herdr-pet-network-block",
                                                                  encodedContentRuleList: rules) { [weak self] ruleList, error in
            DispatchQueue.main.async {
                guard let self else { return }
                if let error {
                    self.emit("error", [
                        "stage": "network_policy",
                        "message": "public WKContentRuleList compilation failed; refusing to start",
                        "detail": error.localizedDescription
                    ])
                    self.terminateSoon()
                    return
                }
                guard let ruleList else {
                    self.emit("error", [
                        "stage": "network_policy",
                        "message": "public WKContentRuleList returned no rule; refusing to start"
                    ])
                    self.terminateSoon()
                    return
                }
                self.contentRuleList = ruleList
                userContentController.add(ruleList)
                self.emit("network_policy", [
                    "externalNavigation": "cancelled",
                    "externalSubresources": "blocked_by_public_content_rule",
                    "fetchWebSocketGuard": "installed",
                    "scheme": "herdr-pet",
                    "status": "installed"
                ])
                self.createWebView(configuration: config, in: contentView)
            }
        }
    }

    private func createWebView(configuration: WKWebViewConfiguration, in contentView: NSView) {
        guard webView == nil else { return }
        let view = TransparentWebView(frame: contentView.bounds, configuration: configuration)
        view.autoresizingMask = [.width, .height]
        view.navigationDelegate = self
        view.uiDelegate = self
        view.allowsMagnification = false
        view.underPageBackgroundColor = NSColor.clear
        view.wantsLayer = true
        view.layer?.backgroundColor = NSColor.clear.cgColor
        contentView.addSubview(view)
        webView = view

        guard let indexFile = options.webRoot.resolve(url: appURL),
              FileManager.default.isReadableFile(atPath: indexFile.path) else {
            emit("error", ["stage": "web_root", "message": "index.html is not readable"])
            terminateSoon()
            return
        }
        guard let pilotManifest = URL(string: "herdr-pet://pack/pilot/pilot.json"),
              let manifestFile = options.packRoot.resolve(url: pilotManifest),
              FileManager.default.isReadableFile(atPath: manifestFile.path) else {
            emit("error", ["stage": "pack_root", "message": "pilot/pilot.json is not readable"])
            terminateSoon()
            return
        }

        emit("browser_host_created", [
            "url": appURL.absoluteString,
            "webKitDataStore": "nonPersistent",
            "webViewIsOpaque": view.isOpaque,
            "underPageBackgroundAlpha": jsonOptional(view.underPageBackgroundColor?.alphaComponent),
            "webLayerBackgroundAlpha": jsonOptional(view.layer?.backgroundColor?.alpha),
            "publicAPIOnly": true
        ])
        panel?.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        view.load(URLRequest(url: appURL, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 30))
    }

    // MARK: - Navigation policy

    func webView(_ webView: WKWebView,
                 decidePolicyFor navigationAction: WKNavigationAction,
                 decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        guard let url = navigationAction.request.url else {
            emit("navigation_denied", ["reason": "missing_url"])
            decisionHandler(.cancel)
            return
        }
        let isAppDocument = url.scheme?.lowercased() == "herdr-pet" && url.host?.lowercased() == "app"
        if isAppDocument {
            decisionHandler(.allow)
        } else {
            emit("navigation_denied", [
                "url": url.absoluteString,
                "reason": "only herdr-pet://app documents may navigate"
            ])
            decisionHandler(.cancel)
        }
    }

    func webView(_ webView: WKWebView,
                 decidePolicyFor navigationResponse: WKNavigationResponse,
                 decisionHandler: @escaping (WKNavigationResponsePolicy) -> Void) {
        guard let url = navigationResponse.response.url,
              url.scheme?.lowercased() == "herdr-pet",
              url.host?.lowercased() == "app" else {
            emit("navigation_denied", ["reason": "response_outside_app_origin"])
            decisionHandler(.cancel)
            return
        }
        decisionHandler(.allow)
    }

    func webView(_ webView: WKWebView, didCommit navigation: WKNavigation!) {
        emit("navigation_committed", ["url": jsonOptional(webView.url?.absoluteString), "pageEpoch": pageEpoch])
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        emit("app_document_loaded", ["url": jsonOptional(webView.url?.absoluteString), "pageEpoch": pageEpoch])
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        emit("navigation_error", ["phase": "committed", "message": error.localizedDescription, "pageEpoch": pageEpoch])
    }

    func webView(_ webView: WKWebView,
                 didFailProvisionalNavigation navigation: WKNavigation!,
                 withError error: Error) {
        emit("navigation_error", ["phase": "provisional", "message": error.localizedDescription, "pageEpoch": pageEpoch])
    }

    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        // This callback is the only termination signal accepted by the probe. A reload
        // command or a test timer is never reported as a process crash.
        let previousPhase = candidateLoaded ? "loaded" : (bridgeReady ? "ready" : "navigation")
        emit("web_process_terminated", [
            "actual": true,
            "source": "WKNavigationDelegate.webViewWebContentProcessDidTerminate",
            "previousPhase": previousPhase,
            "recoveryCount": recoveryCount,
            "recoveryLimit": recoveryLimit
        ])
        pageEpoch += 1
        bridgeReady = false
        candidateLoaded = false
        inputEnabled = false
        activeLoadToken = nil
        pendingCommands.removeAll()
        cancelAllScheduledWork(except: "duration")
        webView.isHidden = true

        guard recoveryCount < recoveryLimit, !disposed else {
            emit("web_unavailable", [
                "reason": "bounded_recovery_limit_reached",
                "recoveryCount": recoveryCount,
                "recoveryLimit": recoveryLimit,
                "inputEnabled": false
            ])
            return
        }
        recoveryCount += 1
        let recoveryToken = makeToken(prefix: "recovery")
        emit("web_recovery_scheduled", [
            "actual": true,
            "token": recoveryToken,
            "attempt": recoveryCount,
            "limit": recoveryLimit,
            "inputEnabled": false
        ])
        schedule(after: 0.25, label: recoveryToken) { [weak self] in
            guard let self, !self.disposed else { return }
            guard self.pageEpoch > 0 else { return }
            self.emit("web_recovery_started", ["actual": true, "token": recoveryToken])
            self.webView?.load(URLRequest(url: self.appURL,
                                           cachePolicy: .reloadIgnoringLocalCacheData,
                                           timeoutInterval: 30))
        }
    }

    func webView(_ webView: WKWebView,
                 createWebViewWith configuration: WKWebViewConfiguration,
                 for navigationAction: WKNavigationAction,
                 windowFeatures: WKWindowFeatures) -> WKWebView? {
        emit("navigation_denied", ["reason": "popup_creation_is_disabled"])
        return nil
    }

    // MARK: - Script bridge

    func userContentController(_ userContentController: WKUserContentController,
                               didReceive message: WKScriptMessage) {
        guard message.name == "rigProbe" else {
            emit("bridge_error", ["message": "unexpected script message handler name", "name": message.name])
            return
        }
        guard message.frameInfo.isMainFrame else {
            emit("bridge_error", ["message": "bridge message came from a subframe"])
            return
        }
        let origin = message.frameInfo.securityOrigin
        guard let expectedScheme = appURL.scheme?.lowercased(),
              let expectedHost = appURL.host?.lowercased(),
              origin.protocol.lowercased() == expectedScheme,
              origin.host.lowercased() == expectedHost,
              origin.port == (appURL.port ?? 0) else {
            emit("bridge_error", [
                "message": "bridge message came from an unexpected origin",
                "scheme": origin.protocol,
                "host": origin.host,
                "port": origin.port
            ])
            return
        }
        guard let payload = jsonDictionary(message.body),
              let type = payload["type"] as? String else {
            emit("bridge_error", ["message": "bridge message was not a JSON object"])
            return
        }
        if let token = payload["token"] as? String, !isCurrentBridgeToken(token) {
            emit("stale_bridge_message", ["type": type, "token": token, "pageEpoch": pageEpoch])
            return
        }

        var fields = payload
        fields.removeValue(forKey: "type")
        switch type {
        case "ready":
            guard !disposed else { return }
            guard !bridgeReady else {
                emit("stale_bridge_message", ["type": "ready", "reason": "duplicate ready for current page", "pageEpoch": pageEpoch])
                return
            }
            bridgeReady = true
            emit("web_ready", fields.merging(["pageEpoch": pageEpoch], uniquingKeysWith: { current, _ in current }))
            beginPackLoad(reason: "ready")
        case "loaded":
            guard !disposed else { return }
            candidateLoaded = true
            inputEnabled = true
            emit("web_loaded", fields.merging(["pageEpoch": pageEpoch, "phase": "candidate_ready"], uniquingKeysWith: { current, _ in current }))
            webView?.isHidden = false
            panel?.makeKeyAndOrderFront(nil)
            callAPI(method: "setVisible", arguments: [true], commandID: "visible-after-load")
            if !scenarioStarted { startAutomaticScenarioIfEnabled() }
        case "metrics":
            if fields["kind"] as? String == "scenario-capture", let name = fields["name"] as? String {
                let artifacts = saveDataURLs(in: fields, stem: "check-\(name)")
                emit("scenario_capture", [
                    "name": name,
                    "artifacts": artifacts,
                    "metrics": fields["metrics"] ?? NSNull(),
                    "pageEpoch": pageEpoch
                ])
            } else {
                emit("web_metrics", fields.merging(["pageEpoch": pageEpoch], uniquingKeysWith: { current, _ in current }))
            }
        case "error":
            inputEnabled = false
            emit("web_error", fields.merging(["pageEpoch": pageEpoch], uniquingKeysWith: { current, _ in current }))
        case "nativeResult":
            handleNativeResult(fields)
        case "nativeError":
            handleNativeError(fields)
        default:
            emit("web_event", fields.merging(["pageEpoch": pageEpoch], uniquingKeysWith: { current, _ in current }))
        }
    }

    private func beginPackLoad(reason: String) {
        beginPackLoad(baseURL: currentPackURL, reason: reason)
    }

    private func beginPackLoad(baseURL: String, reason: String) {
        guard bridgeReady, !disposed else { return }
        guard let validated = validatePackBaseURL(baseURL) else {
            emit("error", ["stage": "pack_url", "message": "pack base URL is outside herdr-pet://pack root", "baseUrl": baseURL])
            return
        }
        currentPackURL = validated
        scenarioStarted = false
        candidateLoaded = false
        inputEnabled = false
        let token = makeToken(prefix: "load")
        activeLoadToken = token
        emit("web_load_requested", ["baseUrl": validated, "reason": reason, "token": token, "pageEpoch": pageEpoch])
        callAPI(method: "load", arguments: [validated], commandID: "load", explicitToken: token)
    }

    private func handleNativeResult(_ fields: [String: Any]) {
        guard let command = fields["command"] as? String,
              let token = fields["token"] as? String else {
            emit("bridge_error", ["message": "nativeResult missing command or token"])
            return
        }
        guard let pending = pendingCommands.removeValue(forKey: token) else {
            emit("stale_bridge_message", ["type": "nativeResult", "command": command, "token": token])
            return
        }
        let result = fields["result"] ?? NSNull()
        switch pending.method {
        case "capture":
            let artifacts = saveDataURLs(in: result, stem: pending.commandID)
            let metrics = jsonDictionary(result)?["metrics"] ?? NSNull()
            emit("capture_result", [
                "command": pending.commandID,
                "token": token,
                "artifacts": artifacts,
                "metrics": metrics
            ])
            captureAppKitSnapshot(reason: pending.commandID)
        case "runChecks":
            let artifacts = saveDataURLs(in: result, stem: "run-checks")
            saveJSONArtifact(result, stem: "run-checks")
            emit("run_checks_result", ["token": token, "artifacts": artifacts, "result": result])
        case "load":
            emit("web_load_result", ["token": token, "result": result, "pageEpoch": pageEpoch])
        default:
            emit("command_result", ["command": pending.commandID, "token": token, "result": result])
        }
    }

    private func handleNativeError(_ fields: [String: Any]) {
        if let token = fields["token"] as? String { pendingCommands.removeValue(forKey: token) }
        inputEnabled = false
        emit("command_error", fields)
    }

    private func callAPI(method: String,
                         arguments: [Any],
                         commandID: String,
                         explicitToken: String? = nil) {
        guard let webView, bridgeReady, !disposed else {
            emit("command_rejected", ["command": commandID, "method": method, "reason": "bridge_not_ready"])
            return
        }
        let token = explicitToken ?? makeToken(prefix: method)
        let argumentLiterals = arguments.compactMap(jsonLiteral)
        guard argumentLiterals.count == arguments.count,
              let methodLiteral = jsonLiteral(method) else {
            emit("command_rejected", ["command": commandID, "method": method, "reason": "arguments_not_json"])
            return
        }
        pendingCommands[token] = (method: method, commandID: commandID)
        let commandLiteral = jsonLiteral(commandID) ?? "\"command\""
        let tokenLiteral = jsonLiteral(token) ?? "\"token\""
        let argumentsLiteral = "[" + argumentLiterals.joined(separator: ",") + "]"
        let script = """
        (() => {
          const send = (value) => {
            try { window.webkit.messageHandlers.rigProbe.postMessage(value); } catch (_) {}
          };
          try {
            const api = window.rigProbe;
            if (!api || typeof api[\(methodLiteral)] !== 'function') {
              throw new Error('rigProbe.' + \(methodLiteral) + ' is unavailable');
            }
            const value = api[\(methodLiteral)].apply(api, \(argumentsLiteral));
            Promise.resolve(value).then(
              (result) => send({type:'nativeResult', command:\(commandLiteral), token:\(tokenLiteral), result: result === undefined ? null : result}),
              (error) => send({type:'nativeError', command:\(commandLiteral), token:\(tokenLiteral), message:String(error && error.message || error)})
            );
          } catch (error) {
            send({type:'nativeError', command:\(commandLiteral), token:\(tokenLiteral), message:String(error && error.message || error)});
          }
        })();
        """
        webView.evaluateJavaScript(script) { [weak self] _, error in
            guard let self, let error else { return }
            DispatchQueue.main.async {
                self.pendingCommands.removeValue(forKey: token)
                self.emit("evaluate_error", [
                    "command": commandID,
                    "method": method,
                    "token": token,
                    "message": error.localizedDescription,
                    "pageEpoch": self.pageEpoch
                ])
            }
        }
    }

    // MARK: - Deterministic scenario and stdin controls

    private func startAutomaticScenarioIfEnabled() {
        guard options.autoScenario, !scenarioStarted, candidateLoaded else { return }
        scenarioStarted = true
        let neutral: [String: Any] = [
            "eyeX": 0,
            "eyeY": 0,
            "eyeOpenL": 1,
            "eyeOpenR": 1,
            "mouthOpen": 0,
            "mouthForm": 0,
            "angleX": 0,
            "angleY": 0,
            "angleZ": 0
        ]
        let alternate: [String: Any] = [
            "eyeX": 0.35,
            "eyeY": -0.2,
            "eyeOpenL": 0.1,
            "eyeOpenR": 1,
            "mouthOpen": 0.65,
            "mouthForm": 0.6,
            "angleX": -0.3,
            "angleY": 0,
            "angleZ": 0.2
        ]
        callAPI(method: "setParameters", arguments: [neutral], commandID: "scenario-neutral")
        schedule(after: 0.25, label: "scenario-capture") { [weak self] in
            guard let self, self.candidateLoaded else { return }
            self.callAPI(method: "capture", arguments: [], commandID: "scenario-capture")
        }
        schedule(after: 0.5, label: "scenario-alternate") { [weak self] in
            guard let self, self.candidateLoaded else { return }
            self.callAPI(method: "setParameters", arguments: [alternate], commandID: "scenario-alternate")
        }
        schedule(after: 0.75, label: "scenario-capture-alternate") { [weak self] in
            guard let self, self.candidateLoaded else { return }
            self.callAPI(method: "capture", arguments: [], commandID: "scenario-capture-alternate")
        }
        schedule(after: 1.0, label: "scenario-checks") { [weak self] in
            guard let self, self.candidateLoaded else { return }
            self.callAPI(method: "runChecks", arguments: [], commandID: "scenario-checks")
        }
        emit("scenario_started", [
            "bounded": true,
            "usesActualAPI": true,
            "perFrameEvaluateJavaScript": false,
            "parameters": ["neutral": neutral, "alternate": alternate]
        ])
    }

    private func startStdinReader() {
        guard !stdinStarted else { return }
        stdinStarted = true
        DispatchQueue.global(qos: .utility).async { [weak self] in
            while let line = readLine(strippingNewline: true) {
                guard !line.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                      let data = line.data(using: .utf8),
                      let object = try? JSONSerialization.jsonObject(with: data),
                      let command = object as? [String: Any] else {
                    DispatchQueue.main.async { self?.emit("stdin_error", ["message": "expected one JSON object per line"]) }
                    continue
                }
                DispatchQueue.main.async { self?.handleCommand(command) }
            }
            DispatchQueue.main.async { self?.emit("stdin_closed", [:]) }
        }
    }

    private func handleCommand(_ command: [String: Any]) {
        guard let name = command["command"] as? String else {
            emit("command_rejected", ["reason": "missing command"])
            return
        }
        if let token = command["token"] as? String, !isCurrentBridgeToken(token) {
            emit("command_rejected", ["command": name, "reason": "stale token", "token": token])
            return
        }
        switch name {
        case "load":
            guard let baseURL = command["baseUrl"] as? String else {
                emit("command_rejected", ["command": name, "reason": "baseUrl is required"])
                return
            }
            beginPackLoad(baseURL: baseURL, reason: "stdin")
        case "setParameters":
            guard let parameters = jsonDictionary(command["parameters"] as Any) else {
                emit("command_rejected", ["command": name, "reason": "parameters must be a JSON object"])
                return
            }
            callAPI(method: "setParameters", arguments: [parameters], commandID: command["requestId"] as? String ?? name)
        case "capture":
            callAPI(method: "capture", arguments: [], commandID: command["requestId"] as? String ?? name)
        case "sampleHit":
            guard let x = finiteDouble(command["x"]), let y = finiteDouble(command["y"]) else {
                emit("command_rejected", ["command": name, "reason": "finite x and y are required"])
                return
            }
            callAPI(method: "sampleHit", arguments: [x, y], commandID: command["requestId"] as? String ?? name)
        case "runChecks":
            callAPI(method: "runChecks", arguments: [], commandID: command["requestId"] as? String ?? name)
        case "setVisible":
            guard let visible = command["visible"] as? Bool else {
                emit("command_rejected", ["command": name, "reason": "visible is required"])
                return
            }
            if visible {
                panel?.makeKeyAndOrderFront(nil)
                webView?.isHidden = false
            } else {
                webView?.isHidden = true
                panel?.orderOut(nil)
                inputEnabled = false
            }
            callAPI(method: "setVisible", arguments: [visible], commandID: command["requestId"] as? String ?? name)
            emit("visibility_changed", ["visible": visible, "inputEnabled": inputEnabled])
        case "suspend":
            webView?.isHidden = true
            panel?.orderOut(nil)
            inputEnabled = false
            callAPI(method: "setVisible", arguments: [false], commandID: "suspend")
            emit("lifecycle", ["state": "suspended", "inputEnabled": false])
        case "restore":
            panel?.makeKeyAndOrderFront(nil)
            webView?.isHidden = false
            callAPI(method: "setVisible", arguments: [true], commandID: "restore")
            inputEnabled = candidateLoaded
            emit("lifecycle", ["state": "restored", "inputEnabled": inputEnabled])
        case "appkitSnapshot":
            captureAppKitSnapshot(reason: "stdin")
        case "setIgnoresMouseEvents":
            guard let value = command["value"] as? Bool, let panel else {
                emit("command_rejected", ["command": name, "reason": "value is required"])
                return
            }
            panel.ignoresMouseEvents = value
            emit("mouse_routing_changed", [
                "ignoresMouseEvents": value,
                "inputEnabled": inputEnabled,
                "note": "physical WindowServer routing is not asserted by this in-process probe"
            ])
        case "reload":
            guard let webView else { return }
            pageEpoch += 1
            bridgeReady = false
            candidateLoaded = false
            inputEnabled = false
            activeLoadToken = nil
            pendingCommands.removeAll()
            cancelAllScheduledWork(except: "duration")
            scenarioStarted = false
            emit("manual_reload", ["actualCrash": false, "reason": "explicit stdin reload", "pageEpoch": pageEpoch])
            webView.reload()
        case "simulate":
            handleSimulation(command)
        case "dispose":
            dispose(reason: "stdin")
        case "terminate":
            emit("terminate_requested", ["source": "stdin"])
            NSApp.terminate(nil)
        default:
            emit("command_rejected", ["command": name, "reason": "unsupported command"])
        }
    }

    private func handleSimulation(_ command: [String: Any]) {
        guard let kind = command["kind"] as? String else {
            emit("simulation_rejected", ["reason": "kind is required", "actual": false])
            return
        }
        switch kind {
        case "bridge-delay":
            let milliseconds = min(max(finiteDouble(command["milliseconds"]) ?? 250, 0), 10_000)
            let id = command["id"] as? String ?? makeToken(prefix: "bridge-delay")
            let method = (command["method"] as? String).flatMap { ["capture", "runChecks"].contains($0) ? $0 : nil } ?? "capture"
            emit("simulation_scheduled", [
                "simulation": "bridge_delay",
                "id": id,
                "milliseconds": milliseconds,
                "method": method,
                "actualCrash": false,
                "label": "test-only bridge delay; not a web-process fault"
            ])
            schedule(after: milliseconds / 1_000, label: id) { [weak self] in
                guard let self, self.candidateLoaded else { return }
                self.emit("simulation_executed", ["simulation": "bridge_delay", "id": id, "actualCrash": false])
                self.callAPI(method: method, arguments: [], commandID: "delayed-\(method)")
            }
        case "cancel":
            guard let id = command["id"] as? String, let item = scheduledWork.removeValue(forKey: id) else {
                emit("simulation_rejected", ["simulation": "cancel", "actual": false, "reason": "unknown scheduled id"])
                return
            }
            item.cancel()
            emit("simulation_cancelled", [
                "simulation": "bridge_delay",
                "id": id,
                "actualCrash": false,
                "label": "test-only delayed bridge cancellation"
            ])
        case "web-process-termination":
            emit("simulation_rejected", [
                "simulation": "web_process_termination",
                "actual": false,
                "reason": "only the public WKNavigationDelegate termination callback is accepted; no fake reload or process signal was sent"
            ])
        default:
            emit("simulation_rejected", ["simulation": kind, "actual": false, "reason": "unsupported simulation"])
        }
    }

    // MARK: - Artifacts and facts

    private func captureAppKitSnapshot(reason: String) {
        guard let panel, let view = panel.contentView else {
            emit("appkit_snapshot_error", ["reason": "content view unavailable"])
            return
        }
        view.displayIfNeeded()
        let bounds = view.bounds
        guard bounds.width > 0, bounds.height > 0,
              let bitmap = view.bitmapImageRepForCachingDisplay(in: bounds) else {
            emit("appkit_snapshot_error", ["reason": "bitmap representation unavailable"])
            return
        }
        view.cacheDisplay(in: bounds, to: bitmap)
        guard let png = bitmap.representation(using: .png, properties: [:]) else {
            emit("appkit_snapshot_error", ["reason": "PNG representation unavailable"])
            return
        }
        let stem = nextArtifactStem(prefix: "snapshot-appkit")
        let path = options.outputRoot.appendingPathComponent(stem + ".png")
        do {
            try png.write(to: path, options: .atomic)
            emitTransparencyFacts(stage: "appkit_snapshot", snapshotSaved: true)
            emit("appkit_snapshot", [
                "reason": reason,
                "artifact": path.path,
                "pixelsWide": bitmap.pixelsWide,
                "pixelsHigh": bitmap.pixelsHigh,
                "bitsPerSample": bitmap.bitsPerSample,
                "samplesPerPixel": bitmap.samplesPerPixel,
                "hasAlpha": bitmap.hasAlpha,
                "bytesPerRow": bitmap.bytesPerRow,
                "backingScaleFactor": panel.backingScaleFactor,
                "assessment": "observed AppKit bitmap facts only; transparency is not declared a pass"
            ])
        } catch {
            emit("appkit_snapshot_error", ["reason": "write failed", "message": error.localizedDescription])
        }

        let snapshotConfiguration = WKSnapshotConfiguration()
        snapshotConfiguration.rect = webView?.bounds ?? .zero
        webView?.takeSnapshot(with: snapshotConfiguration) { [weak self] image, error in
            guard let self else { return }
            if let error {
                self.emit("webkit_snapshot_error", ["message": error.localizedDescription])
                return
            }
            guard let image,
                  let tiff = image.tiffRepresentation,
                  let imageRep = NSBitmapImageRep(data: tiff),
                  let data = imageRep.representation(using: .png, properties: [:]) else {
                self.emit("webkit_snapshot_error", ["message": "snapshot did not contain a PNG representation"])
                return
            }
            let webStem = self.nextArtifactStem(prefix: "snapshot-wk")
            let webPath = self.options.outputRoot.appendingPathComponent(webStem + ".png")
            do {
                try data.write(to: webPath, options: .atomic)
                self.emit("webkit_snapshot", [
                    "reason": reason,
                    "artifact": webPath.path,
                    "pixelsWide": imageRep.pixelsWide,
                    "pixelsHigh": imageRep.pixelsHigh,
                    "hasAlpha": imageRep.hasAlpha,
                    "assessment": "observed WKSnapshot bitmap facts only; transparency is not declared a pass"
                ])
            } catch {
                self.emit("webkit_snapshot_error", ["message": error.localizedDescription])
            }
        }
    }

    private func emitTransparencyFacts(stage: String, snapshotSaved: Bool) {
        let panelBackgroundAlpha = panel?.backgroundColor?.alphaComponent
        let webBackgroundAlpha = webView?.underPageBackgroundColor?.alphaComponent
        let panelLayerAlpha = panel?.contentView?.layer?.backgroundColor?.alpha
        let webLayerAlpha = webView?.layer?.backgroundColor?.alpha
        emit("transparency_facts", [
            "stage": stage,
            "panelIsOpaque": jsonOptional(panel?.isOpaque),
            "panelBackgroundAlpha": jsonOptional(panelBackgroundAlpha),
            "webViewIsOpaque": jsonOptional(webView?.isOpaque),
            "underPageBackgroundAlpha": jsonOptional(webBackgroundAlpha),
            "panelLayerBackgroundAlpha": jsonOptional(panelLayerAlpha),
            "webLayerBackgroundAlpha": jsonOptional(webLayerAlpha),
            "snapshotSaved": snapshotSaved,
            "publicAPIs": ["NSPanel.isOpaque", "NSView.isOpaque", "WKWebView.underPageBackgroundColor", "WKSnapshotConfiguration"],
            "assessment": "factual host observations; actual display compositing remains an acceptance observation"
        ])
    }

    private func saveDataURLs(in value: Any, stem: String) -> [String] {
        var artifacts: [String] = []
        func visit(_ item: Any, path: String) {
            if let dictionary = jsonDictionary(item) {
                for (key, child) in dictionary {
                    if key == "dataUrl", let dataURL = child as? String,
                       let artifact = saveDataURL(dataURL, stem: stem + "-" + path) {
                        artifacts.append(artifact)
                    } else {
                        visit(child, path: path + "-" + safeName(key))
                    }
                }
            } else if let array = item as? [Any] {
                for (index, child) in array.enumerated() { visit(child, path: path + "-\(index)") }
            }
        }
        visit(value, path: "result")
        return artifacts
    }

    private func saveDataURL(_ dataURL: String, stem: String) -> String? {
        guard dataURL.hasPrefix("data:") else {
            emit("capture_error", ["reason": "capture result dataUrl did not use data: URL"])
            return nil
        }
        let parts = dataURL.split(separator: ",", maxSplits: 1, omittingEmptySubsequences: false)
        guard parts.count == 2 else {
            emit("capture_error", ["reason": "malformed data URL"])
            return nil
        }
        let metadata = String(parts[0])
        guard metadata.lowercased().contains(";base64") else {
            emit("capture_error", ["reason": "capture dataUrl was not base64"])
            return nil
        }
        let base = safeName(stem)
        let textURL = options.outputRoot.appendingPathComponent(base + ".data-url.txt")
        let imageURL = options.outputRoot.appendingPathComponent(base + ".png")
        do {
            try Data(dataURL.utf8).write(to: textURL, options: .atomic)
            guard let imageData = Data(base64Encoded: String(parts[1]), options: [.ignoreUnknownCharacters]) else {
                emit("capture_error", ["reason": "base64 decode failed", "artifact": textURL.path])
                return textURL.path
            }
            try imageData.write(to: imageURL, options: .atomic)
            return imageURL.path
        } catch {
            emit("capture_error", ["reason": "capture artifact write failed", "message": error.localizedDescription])
            return nil
        }
    }

    private func saveJSONArtifact(_ value: Any, stem: String) {
        let safe = jsonSafeValue(value)
        guard JSONSerialization.isValidJSONObject(safe),
              let data = try? JSONSerialization.data(withJSONObject: safe, options: [.prettyPrinted, .sortedKeys]) else {
            emit("artifact_error", ["reason": "result was not JSON serializable", "stem": stem])
            return
        }
        let path = options.outputRoot.appendingPathComponent(safeName(stem) + ".json")
        do {
            try data.write(to: path, options: .atomic)
            emit("json_artifact", ["stem": stem, "artifact": path.path])
        } catch {
            emit("artifact_error", ["reason": "JSON artifact write failed", "message": error.localizedDescription])
        }
    }

    // MARK: - Lifecycle, tokens, and utility

    private func dispose(reason: String) {
        guard !disposed else { return }
        let shouldDisposeRenderer = bridgeReady
        if shouldDisposeRenderer {
            callAPI(method: "dispose", arguments: [], commandID: "dispose")
        }
        configuration?.userContentController.removeScriptMessageHandler(forName: "rigProbe")
        disposed = true
        inputEnabled = false
        cancelAllScheduledWork()
        webView?.navigationDelegate = nil
        webView?.uiDelegate = nil
        webView?.stopLoading()
        webView?.isHidden = true
        panel?.orderOut(nil)
        emit("disposed", ["reason": reason, "actualRendererDisposeRequested": shouldDisposeRenderer])
        schedule(after: 0.1, label: "dispose-exit") { NSApp.terminate(nil) }
    }

    private func validatePackBaseURL(_ string: String) -> String? {
        guard let url = URL(string: string),
              url.scheme?.lowercased() == "herdr-pet",
              url.host?.lowercased() == "pack",
              url.user == nil, url.password == nil, url.port == nil,
              url.query == nil, url.fragment == nil,
              url.path(percentEncoded: true).hasPrefix("/"),
              url.path(percentEncoded: true).hasSuffix("/") else { return nil }
        guard let encodedManifestURL = URL(string: string + "pilot.json"),
              options.packRoot.resolve(url: encodedManifestURL) != nil else { return nil }
        return url.absoluteString
    }

    private func preflightRoots() -> Bool {
        guard let index = options.webRoot.resolve(url: appURL), FileManager.default.isReadableFile(atPath: index.path) else {
            emit("error", ["stage": "preflight", "message": "web root does not contain readable index.html"])
            return false
        }
        guard let manifest = URL(string: defaultPackURL + "pilot.json"),
              let manifestPath = options.packRoot.resolve(url: manifest),
              FileManager.default.isReadableFile(atPath: manifestPath.path) else {
            emit("error", ["stage": "preflight", "message": "pack root does not contain readable pilot/pilot.json"])
            return false
        }
        return true
    }

    private func emitRuntimePermissions() {
        let environment = ProcessInfo.processInfo.environment
#if arch(arm64)
        let architecture = "arm64"
#elseif arch(x86_64)
        let architecture = "x86_64"
#else
        let architecture = "unknown"
#endif
        emit("runtime_permissions", [
            "sandboxContainerPresent": environment["APP_SANDBOX_CONTAINER_ID"] != nil,
            "accessibilityRequested": false,
            "screenCaptureRequested": false,
            "networkEntitlement": "not requested by probe; public WK content-rule installation is reported separately",
            "webRootReadable": FileManager.default.isReadableFile(atPath: options.webRoot.url.path),
            "packRootReadable": FileManager.default.isReadableFile(atPath: options.packRoot.url.path),
            "outputWritable": FileManager.default.isWritableFile(atPath: options.outputRoot.path),
            "processIdentifier": ProcessInfo.processInfo.processIdentifier,
            "architecture": architecture
        ])
    }

    private func schedule(after delay: TimeInterval, label: String, action: @escaping () -> Void) {
        let item = DispatchWorkItem { [weak self] in
            guard let self, self.scheduledWork[label] != nil else { return }
            self.scheduledWork.removeValue(forKey: label)
            action()
        }
        scheduledWork[label] = item
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: item)
    }


    private func cancelAllScheduledWork(except label: String? = nil) {
        for (key, item) in scheduledWork where key != label { item.cancel() }
        if let label {
            scheduledWork = scheduledWork.filter { $0.key == label }
        } else {
            scheduledWork.removeAll()
        }
    }

    private func terminateSoon() {
        DispatchQueue.main.async { [weak self] in
            self?.emit("probe_blocked", ["reason": "precondition_failed"])
            NSApp.terminate(nil)
        }
    }

    private func makeToken(prefix: String) -> String {
        tokenCounter += 1
        return "\(prefix)-e\(pageEpoch)-\(tokenCounter)"
    }

    private func isCurrentBridgeToken(_ token: String) -> Bool {
        if token == activeLoadToken { return true }
        return pendingCommands[token] != nil
    }

    private func finiteDouble(_ value: Any?) -> Double? {
        if let value = value as? Double, value.isFinite { return value }
        if let value = value as? NSNumber {
            let number = value.doubleValue
            return number.isFinite ? number : nil
        }
        return nil
    }

    private func nextArtifactStem(prefix: String) -> String {
        captureCounter += 1
        return "\(prefix)-\(String(format: "%04d", captureCounter))"
    }

    private func safeName(_ value: String) -> String {
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-_"))
        return value.unicodeScalars.map { allowed.contains($0) ? String($0) : "_" }.joined()
    }

    private func emit(_ type: String, _ fields: [String: Any]) {
        var payload: [String: Any] = [
            "type": type,
            "timestamp": isoTimestamp(),
            "pid": ProcessInfo.processInfo.processIdentifier
        ]
        for (key, value) in fields { payload[key] = jsonSafeValue(value) }
        guard let data = try? JSONSerialization.data(withJSONObject: payload, options: [.sortedKeys]) else { return }
        outputLock.lock()
        FileHandle.standardOutput.write(data)
        FileHandle.standardOutput.write(Data([0x0A]))
        outputLock.unlock()
    }

    private static let networkGuardScript = """
    (() => {
      const send = (value) => {
        try { window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.rigProbe && window.webkit.messageHandlers.rigProbe.postMessage(value); } catch (_) {}
      };
      const isExternal = (candidate) => {
        try {
          const url = new URL(String(candidate), window.location.href);
          return /^(http|https|ws|wss|ftp):$/i.test(url.protocol);
        } catch (_) { return true; }
      };
      const originalFetch = window.fetch;
      if (typeof originalFetch === 'function') {
        window.fetch = function(input, init) {
          const candidate = input && input.url ? input.url : input;
          if (isExternal(candidate)) {
            send({type:'network_denied', method:'fetch', url:String(candidate)});
            return Promise.reject(new TypeError('external network is disabled by rig-probe'));
          }
          return originalFetch.call(this, input, init);
        };
      }
      const OriginalXHR = window.XMLHttpRequest;
      if (OriginalXHR && OriginalXHR.prototype && OriginalXHR.prototype.open) {
        const originalOpen = OriginalXHR.prototype.open;
        OriginalXHR.prototype.open = function(method, url) {
          if (isExternal(url)) {
            send({type:'network_denied', method:'XMLHttpRequest', url:String(url)});
            throw new TypeError('external network is disabled by rig-probe');
          }
          return originalOpen.apply(this, arguments);
        };
      }
      const OriginalWebSocket = window.WebSocket;
      if (OriginalWebSocket) {
        window.WebSocket = function(url, protocols) {
          if (isExternal(url)) {
            send({type:'network_denied', method:'WebSocket', url:String(url)});
            throw new TypeError('external network is disabled by rig-probe');
          }
          return protocols === undefined ? new OriginalWebSocket(url) : new OriginalWebSocket(url, protocols);
        };
        window.WebSocket.prototype = OriginalWebSocket.prototype;
      }
    })();
    """
}

var retainedDelegate: RigProbeApplicationDelegate?
do {
    let options = try ProbeOptions.parse(arguments: CommandLine.arguments)
    let application = NSApplication.shared
    let delegate = RigProbeApplicationDelegate(options: options)
    retainedDelegate = delegate
    application.delegate = delegate
    application.setActivationPolicy(.accessory)
    application.run()
} catch {
    let message = (error as? ProbeCLIError)?.message ?? error.localizedDescription
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(64)
}
