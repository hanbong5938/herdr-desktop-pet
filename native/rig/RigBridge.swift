import AppKit
import Darwin
import Foundation

private let rigBridgeOK: Int32 = 0
private let rigBridgeInvalid: Int32 = -1

private func clearError(_ output: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) {
    output?.pointee = nil
}

private func assignError(_ output: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?, _ message: String) {
    guard let output else { return }
    output.pointee = strdup(message)
}

private func assignError(_ output: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?, _ error: Error) {
    assignError(output, String(describing: error))
}

private func bridgeHost(_ raw: HerdrRigHandle?) -> RigNativeHost? {
    guard let raw else { return nil }
    return Unmanaged<RigNativeHost>.fromOpaque(raw).takeUnretainedValue()
}

@_cdecl("herdr_rig_error_free")
public func herdrRigErrorFree(_ error: UnsafeMutablePointer<CChar>?) {
    guard let error else { return }
    free(error)
}

@_cdecl("herdr_rig_create")
public func herdrRigCreate(
    _ asset: UnsafePointer<HerdrRigAssetInput>?,
    _ token: UnsafePointer<HerdrRigTokenInput>?,
    _ output: UnsafeMutablePointer<HerdrRigHandle?>?,
    _ error: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> Int32 {
    clearError(error)
    output?.pointee = nil
    guard Thread.isMainThread else {
        assignError(error, "rig host creation must run on the AppKit main thread")
        return rigBridgeInvalid
    }
    guard let asset, let token, let output else {
        assignError(error, "rig host creation received a null argument")
        return rigBridgeInvalid
    }
    do {
        let host = try RigNativeHost(asset: asset.pointee, token: token.pointee)
        output.pointee = Unmanaged.passRetained(host).toOpaque()
        return rigBridgeOK
    } catch let failure {
        assignError(error, failure)
        return rigBridgeInvalid
    }
}

@_cdecl("herdr_rig_poll")
public func herdrRigPoll(
    _ handle: HerdrRigHandle?,
    _ cancelRequested: UInt32,
    _ state: UnsafeMutablePointer<UInt32>?,
    _ error: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> Int32 {
    clearError(error)
    state?.pointee = 0
    guard let host = bridgeHost(handle), let state else {
        assignError(error, "rig poll received an invalid handle")
        return rigBridgeInvalid
    }
    let value = host.poll(cancelRequested: cancelRequested != 0)
    state.pointee = UInt32(bitPattern: value)
    if value < 0 {
        assignError(error, host.errorDescription() ?? "rig preparation failed")
        return rigBridgeInvalid
    }
    return rigBridgeOK
}

@_cdecl("herdr_rig_cancel")
public func herdrRigCancel(_ handle: HerdrRigHandle?) {
    bridgeHost(handle)?.cancel()
}

@_cdecl("herdr_rig_destroy")
public func herdrRigDestroy(_ handle: HerdrRigHandle?) {
    guard let handle else { return }
    let host = Unmanaged<RigNativeHost>.fromOpaque(handle).takeRetainedValue()
    host.cancel()
}

@_cdecl("herdr_rig_view")
public func herdrRigView(
    _ handle: HerdrRigHandle?,
    _ output: UnsafeMutablePointer<UnsafeMutableRawPointer?>?,
    _ error: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> Int32 {
    clearError(error)
    output?.pointee = nil
    guard let host = bridgeHost(handle), let output else {
        assignError(error, "rig view received an invalid handle")
        return rigBridgeInvalid
    }
    guard Thread.isMainThread else {
        assignError(error, "rig view must be acquired on the AppKit main thread")
        return rigBridgeInvalid
    }
    output.pointee = Unmanaged.passRetained(host.view).toOpaque()
    return rigBridgeOK
}

@_cdecl("herdr_rig_update")
public func herdrRigUpdate(
    _ handle: HerdrRigHandle?,
    _ intent: UnsafePointer<HerdrRigIntent>?,
    _ error: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> Int32 {
    clearError(error)
    guard let host = bridgeHost(handle), let intent else {
        assignError(error, "rig update received an invalid handle or intent")
        return rigBridgeInvalid
    }
    do {
        try host.update(intent.pointee)
        return rigBridgeOK
    } catch let failure {
        assignError(error, failure)
        return rigBridgeInvalid
    }
}
@_cdecl("herdr_rig_set_viewport_epoch")
public func herdrRigSetViewportEpoch(_ handle: HerdrRigHandle?, _ epoch: UInt64) {
    bridgeHost(handle)?.setViewportEpoch(epoch)
}

@_cdecl("herdr_rig_prepare_surface")
public func herdrRigPrepareSurface(
    _ handle: HerdrRigHandle?,
    _ intent: UnsafePointer<HerdrRigIntent>?,
    _ width: Double,
    _ height: Double,
    _ backingScale: Double,
    _ viewportEpoch: UInt64,
    _ ready: UnsafeMutablePointer<UInt32>?,
    _ error: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> Int32 {
    clearError(error)
    ready?.pointee = 0
    guard let host = bridgeHost(handle), let intent, let ready else {
        assignError(error, "rig surface preparation received an invalid handle or intent")
        return rigBridgeInvalid
    }
    do {
        ready.pointee = try host.prepareSurface(intent.pointee, width: width, height: height,
                                                backingScale: backingScale,
                                                viewportEpoch: viewportEpoch) ? 1 : 0
        return rigBridgeOK
    } catch let failure {
        assignError(error, failure)
        return rigBridgeInvalid
    }
}

@_cdecl("herdr_rig_activate")
public func herdrRigActivate(
    _ handle: HerdrRigHandle?,
    _ error: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> Int32 {
    clearError(error)
    guard let host = bridgeHost(handle) else {
        assignError(error, "rig activation received an invalid handle")
        return rigBridgeInvalid
    }
    do {
        try host.activate()
        return rigBridgeOK
    } catch let failure {
        assignError(error, failure)
        return rigBridgeInvalid
    }
}

@_cdecl("herdr_rig_set_visible")
public func herdrRigSetVisible(_ handle: HerdrRigHandle?, _ visible: UInt32) {
    bridgeHost(handle)?.setVisible(visible != 0)
}

@_cdecl("herdr_rig_input_epoch")
public func herdrRigInputEpoch(_ handle: HerdrRigHandle?) -> UInt64 {
    bridgeHost(handle)?.inputEpoch() ?? 0
}

@_cdecl("herdr_rig_input_ready")
public func herdrRigInputReady(_ handle: HerdrRigHandle?) -> UInt32 {
    bridgeHost(handle)?.inputReady() == true ? 1 : 0
}

@_cdecl("herdr_rig_speech_anchor_snapshot")
public func herdrRigSpeechAnchorSnapshot(
    _ handle: HerdrRigHandle?,
    _ output: UnsafeMutablePointer<HerdrRigSpeechAnchorSnapshot>?,
    _ error: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> Int32 {
    clearError(error)
    output?.pointee = HerdrRigSpeechAnchorSnapshot()
    guard let host = bridgeHost(handle), let output else {
        assignError(error, "rig speech anchor snapshot received an invalid handle or output")
        return rigBridgeInvalid
    }
    do {
        output.pointee = try host.speechAnchorSnapshot()
        return rigBridgeOK
    } catch let failure {
        assignError(error, failure)
        return rigBridgeInvalid
    }
}

@_cdecl("herdr_rig_hit")
public func herdrRigHit(
    _ handle: HerdrRigHandle?,
    _ x: Double,
    _ y: Double,
    _ output: UnsafeMutablePointer<HerdrRigHit>?,
    _ hasHit: UnsafeMutablePointer<UInt32>?,
    _ error: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> Int32 {
    clearError(error)
    hasHit?.pointee = 0
    guard let host = bridgeHost(handle), let output, let hasHit else {
        assignError(error, "rig hit received an invalid handle or output")
        return rigBridgeInvalid
    }
    if let hit = host.hit(normalizedX: x, normalizedY: y) {
        output.pointee = hit
        hasHit.pointee = 1
    }
    return rigBridgeOK
}


@_cdecl("herdr_rig_display_bounds")
public func herdrRigDisplayBounds(
    _ handle: HerdrRigHandle?,
    _ output: UnsafeMutablePointer<HerdrRigAnchor>?
) -> UInt32 {
    guard let host = bridgeHost(handle), let output, let bounds = host.displayBounds() else { return 0 }
    output.pointee = bounds
    return 1
}

@_cdecl("herdr_rig_preview_png")
public func herdrRigPreviewPNG(
    _ handle: HerdrRigHandle?,
    _ intent: UnsafePointer<HerdrRigIntent>?,
    _ hitOverlay: UInt32,
    _ output: UnsafeMutablePointer<UnsafeMutablePointer<UInt8>?>?,
    _ length: UnsafeMutablePointer<Int>?,
    _ error: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> Int32 {
    clearError(error)
    output?.pointee = nil
    length?.pointee = 0
    guard let host = bridgeHost(handle), let intent, let output, let length else {
        assignError(error, "rig preview received an invalid handle or intent")
        return rigBridgeInvalid
    }
    do {
        let data = try host.previewPNG(intent.pointee, hitOverlay: hitOverlay != 0)
        guard !data.isEmpty, let bytes = malloc(data.count) else {
            throw RigNativeError.unavailable("preview PNG allocation failed")
        }
        data.copyBytes(to: bytes.assumingMemoryBound(to: UInt8.self), count: data.count)
        output.pointee = bytes.assumingMemoryBound(to: UInt8.self)
        length.pointee = data.count
        return rigBridgeOK
    } catch let failure {
        assignError(error, failure)
        return rigBridgeInvalid
    }
}

@_cdecl("herdr_rig_bytes_free")
public func herdrRigBytesFree(_ bytes: UnsafeMutablePointer<UInt8>?, _ length: Int) {
    guard let bytes else { return }
    _ = length
    free(bytes)
}

@_cdecl("herdr_rig_last_error")
public func herdrRigLastError(_ handle: HerdrRigHandle?) -> UnsafeMutablePointer<CChar>? {
    guard let message = bridgeHost(handle)?.errorDescription() else { return nil }
    return strdup(message)
}
