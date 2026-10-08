use crate::assets::ManagedPack;
use crate::character_types::OfficialPackIdentity;
use crate::official_catalog::{
    parse_catalog, OfficialCatalogSnapshot, CATALOG_URL, MAX_CATALOG_BYTES,
};
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, AnyThread, DefinedClass};
use objc2_foundation::{
    NSData, NSError, NSHTTPURLResponse, NSMutableURLRequest, NSObject, NSObjectProtocol,
    NSOperationQueue, NSString, NSURLRequestCachePolicy, NSURLResponse, NSURLSession,
    NSURLSessionConfiguration, NSURLSessionDataDelegate, NSURLSessionDataTask,
    NSURLSessionDelegate, NSURLSessionResponseDisposition, NSURLSessionTask,
    NSURLSessionTaskDelegate, NSURL,
};
use parking_lot::{Condvar, Mutex};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use url::Url;

const PREVIEW_LIMIT: usize = 4 * 1024 * 1024;
const ARCHIVE_LIMIT: u64 = 64 * 1024 * 1024;
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

#[derive(Clone, Debug)]
pub struct OfficialDownload {
    pub identity: OfficialPackIdentity,
    pub url: String,
    pub bytes: u64,
    pub format_version: u32,
    pub render_mode: String,
}

#[derive(Default)]
struct CatalogState {
    requested: bool,
    loading: bool,
    stopping: bool,
    revision: u64,
    snapshot: Option<Arc<OfficialCatalogSnapshot>>,
    error: Option<String>,
}
struct CatalogWorker {
    state: Mutex<CatalogState>,
    wake: Condvar,
}
pub struct OfficialCharacters {
    worker: Arc<CatalogWorker>,
    stop: Arc<AtomicBool>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl OfficialCharacters {
    pub fn new(builtin_id: String) -> Arc<Self> {
        let worker = Arc::new(CatalogWorker {
            state: Mutex::new(CatalogState::default()),
            wake: Condvar::new(),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let thread_worker = Arc::clone(&worker);
        let thread_stop = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("official-catalog".into())
            .spawn(move || loop {
                let mut state = thread_worker.state.lock();
                while !state.requested && !state.stopping {
                    thread_worker.wake.wait(&mut state);
                }
                if state.stopping {
                    break;
                }
                state.requested = false;
                state.loading = true;
                let revision = state.revision.saturating_add(1);
                drop(state);
                let result = fetch_bytes(
                    CATALOG_URL,
                    Source::Catalog,
                    MAX_CATALOG_BYTES as u64,
                    Duration::from_secs(9),
                    &thread_stop,
                    |_, _| {},
                )
                .and_then(|bytes| parse_catalog(&bytes, &builtin_id, revision));
                let mut state = thread_worker.state.lock();
                state.loading = false;
                if state.stopping {
                    break;
                }
                match result {
                    Ok(snapshot) => {
                        state.revision = snapshot.revision;
                        state.snapshot = Some(Arc::new(snapshot));
                        state.error = None;
                    }
                    Err(error) => state.error = Some(error),
                }
            })
            .expect("cannot create official catalog worker");
        Arc::new(Self {
            worker,
            stop,
            handle: Mutex::new(Some(handle)),
        })
    }
    pub fn request_catalog(&self) {
        let mut state = self.worker.state.lock();
        if !state.stopping && !state.requested && !state.loading {
            state.requested = true;
            self.worker.wake.notify_one();
        }
    }
    pub fn cached_snapshot(&self) -> Option<Arc<OfficialCatalogSnapshot>> {
        self.worker.state.lock().snapshot.clone()
    }
    pub fn catalog_revision(&self) -> u64 {
        self.worker.state.lock().revision
    }
    pub fn catalog_error(&self) -> Option<String> {
        self.worker.state.lock().error.clone()
    }
    pub fn resolve(&self, identity: &OfficialPackIdentity) -> Result<OfficialDownload, String> {
        let snapshot = self
            .cached_snapshot()
            .ok_or("official catalog is unavailable")?;
        let entry = snapshot
            .entries
            .iter()
            .find(|entry| &entry.identity == identity)
            .ok_or("official pack identity is not in the current catalog")?;
        if !entry.install_supported {
            return Err("official pack format is not supported".into());
        }
        Ok(OfficialDownload {
            identity: entry.identity.clone(),
            url: entry.download_url.clone(),
            bytes: entry.download_bytes,
            format_version: entry.format_version,
            render_mode: entry.render_mode.clone(),
        })
    }
    /// Worker-only operation. Hash and validate the exact bounded archive snapshot received.
    pub fn download_verified(
        &self,
        identity: &OfficialPackIdentity,
        cancel: &AtomicBool,
        progress: impl FnMut(u64, u64),
    ) -> Result<ManagedPack, String> {
        let pin = self.resolve(identity)?;
        if pin.bytes > ARCHIVE_LIMIT {
            return Err("official archive exceeds limit".into());
        }
        let archive = fetch_bytes(
            &pin.url,
            Source::Release(&pin.url),
            pin.bytes,
            Duration::from_secs(120),
            cancel,
            progress,
        )?;
        if cancel.load(Ordering::Relaxed) {
            return Err("official download canceled".into());
        }
        let managed = validate_archive_snapshot(&archive, &pin)?;
        if cancel.load(Ordering::Relaxed) {
            return Err("official download canceled".into());
        }
        Ok(managed)
    }
    /// Fetches only an idle image present in the latest validated catalog, never a caller-selected URL.
    pub fn fetch_preview(&self, url: &str, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
        let snapshot = self
            .cached_snapshot()
            .ok_or("official catalog is unavailable")?;
        if !snapshot
            .entries
            .iter()
            .any(|entry| entry.preview_idle_url == url)
        {
            return Err("preview URL is not in the official catalog".into());
        }
        let bytes = fetch_bytes(
            url,
            Source::Preview(url),
            PREVIEW_LIMIT as u64,
            Duration::from_secs(9),
            cancel,
            |_, _| {},
        )?;
        if !bytes.starts_with(PNG_SIGNATURE) {
            return Err("official preview is not a PNG".into());
        }
        Ok(bytes)
    }
    pub fn shutdown(&self) {
        {
            let mut state = self.worker.state.lock();
            state.stopping = true;
            self.stop.store(true, Ordering::Relaxed);
            state.requested = false;
            self.worker.wake.notify_all();
        }
        if let Some(handle) = self.handle.lock().take() {
            let _ = handle.join();
        }
    }
}
impl Drop for OfficialCharacters {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Copy, Clone)]
enum Source<'a> {
    Catalog,
    Preview(&'a str),
    Release(&'a str),
}
fn safe_url(text: &str, source: Source<'_>, redirected: bool) -> Result<Url, String> {
    let url = Url::parse(text).map_err(|_| "invalid official URL")?;
    if url.scheme() != "https"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("unsafe official URL authority".into());
    }
    match source {
        Source::Catalog if text == CATALOG_URL && !redirected => (),
        Source::Preview(original)
            if text == original
                && !redirected
                && url.host_str() == Some("raw.githubusercontent.com")
                && url.query().is_none()
                && url
                    .path()
                    .starts_with("/hanbong5938/herdr-characters/main/previews/") =>
        {
            ()
        }
        Source::Release(original)
            if !redirected
                && text == original
                && url.host_str() == Some("github.com")
                && url.query().is_none()
                && url
                    .path()
                    .starts_with("/hanbong5938/herdr-characters/releases/download/") =>
        {
            ()
        }
        Source::Release(_)
            if redirected
                && url.host_str() == Some("release-assets.githubusercontent.com")
                && url.path().starts_with('/')
                && !url.path().contains("..") =>
        {
            ()
        }
        _ => return Err("untrusted official redirect or origin".into()),
    }
    Ok(url)
}
fn check_transfer(deadline: Instant, cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("official transfer canceled".into());
    }
    if Instant::now() >= deadline {
        return Err("official transfer timed out".into());
    }
    Ok(())
}
struct TransferShared {
    state: Mutex<TransferState>,
    wake: Condvar,
}
struct TransferState {
    bytes: Vec<u8>,
    limit: u64,
    received: u64,
    accepted: bool,
    abandoned: bool,
    outcome: Option<Result<Option<String>, String>>,
}
impl TransferState {
    fn accept_chunk(&mut self, chunk: &[u8]) -> Result<(), String> {
        if chunk.len() as u64 > self.limit - self.received {
            return Err("official response exceeds allowed byte count".into());
        }
        self.bytes.extend_from_slice(chunk);
        self.received += chunk.len() as u64;
        Ok(())
    }
}
fn check_transfer_length(source: Source<'_>, received: u64, limit: u64) -> Result<(), String> {
    if matches!(source, Source::Release(_)) && received != limit {
        return Err("official archive byte count differs from catalog".into());
    }
    Ok(())
}
define_class!(
    // SAFETY: Only synchronized, owned Rust state is shared across NSURLSession's serial callback queue.
    #[unsafe(super = NSObject)]
    #[thread_kind = AnyThread]
    #[name = "OMPetOfficialTransferDelegate"]
    #[ivars = Arc<TransferShared>]
    struct TransferDelegate;

    unsafe impl NSObjectProtocol for TransferDelegate {}
    unsafe impl NSURLSessionDelegate for TransferDelegate {}
    unsafe impl NSURLSessionTaskDelegate for TransferDelegate {
        #[unsafe(method(URLSession:task:willPerformHTTPRedirection:newRequest:completionHandler:))]
        unsafe fn will_redirect(
            &self, _session: &NSURLSession, _task: &NSURLSessionTask,
            _response: &NSHTTPURLResponse, _request: &objc2_foundation::NSURLRequest,
            completion_handler: &block2::DynBlock<dyn Fn(*mut objc2_foundation::NSURLRequest)>,
        ) {
            // Manual redirect handling below: URLSession must never contact a new host itself.
            completion_handler.call((std::ptr::null_mut(),));
        }
        #[unsafe(method(URLSession:task:didCompleteWithError:))]
        fn did_complete(
            &self, _session: &NSURLSession, _task: &NSURLSessionTask, error: Option<&NSError>,
        ) {
            let shared = self.ivars();
            let mut state = shared.state.lock();
            if !state.abandoned && state.outcome.is_none() {
                state.outcome = Some(if error.is_some() || !state.accepted {
                    Err("official transfer failed".into())
                } else { Ok(None) });
                shared.wake.notify_one();
            }
        }
    }
    unsafe impl NSURLSessionDataDelegate for TransferDelegate {
        #[unsafe(method(URLSession:dataTask:didReceiveResponse:completionHandler:))]
        unsafe fn did_receive_response(
            &self, _session: &NSURLSession, _task: &NSURLSessionDataTask,
            response: &NSURLResponse,
            completion_handler: &block2::DynBlock<dyn Fn(NSURLSessionResponseDisposition)>,
        ) {
            let shared = self.ivars();
            let mut state = shared.state.lock();
            let mut disposition = NSURLSessionResponseDisposition::Allow;
            if state.abandoned {
                disposition = NSURLSessionResponseDisposition::Cancel;
            } else if let Some(http) = response.downcast_ref::<NSHTTPURLResponse>() {
                let status = http.statusCode();
                if (300..400).contains(&status) {
                    state.outcome = Some(http.valueForHTTPHeaderField(&NSString::from_str("Location"))
                        .map(|value| Some(value.to_string()))
                        .ok_or_else(|| "missing official redirect location".into()));
                    disposition = NSURLSessionResponseDisposition::Cancel;
                } else if status != 200 {
                    state.outcome = Some(Err(format!("official server returned HTTP {status}")));
                    disposition = NSURLSessionResponseDisposition::Cancel;
                } else if http.valueForHTTPHeaderField(&NSString::from_str("Content-Encoding"))
                    .is_some_and(|value| !value.to_string().eq_ignore_ascii_case("identity")) {
                    state.outcome = Some(Err("compressed official HTTP response is not permitted".into()));
                    disposition = NSURLSessionResponseDisposition::Cancel;
                } else if http.valueForHTTPHeaderField(&NSString::from_str("Content-Length"))
                    .and_then(|value| value.to_string().parse::<u64>().ok())
                    .is_some_and(|length| length > state.limit) {
                    state.outcome = Some(Err("official response exceeds allowed byte count".into()));
                    disposition = NSURLSessionResponseDisposition::Cancel;
                } else {
                    state.accepted = true;
                }
            } else {
                state.outcome = Some(Err("official server did not return an HTTP response".into()));
                disposition = NSURLSessionResponseDisposition::Cancel;
            }
            shared.wake.notify_one();
            drop(state);
            completion_handler.call((disposition,));
        }
        #[unsafe(method(URLSession:dataTask:didReceiveData:))]
        fn did_receive_data(
            &self, _session: &NSURLSession, _task: &NSURLSessionDataTask, data: &NSData,
        ) {
            let shared = self.ivars();
            let mut state = shared.state.lock();
            if state.abandoned || state.outcome.is_some() || !state.accepted { return; }
            // SAFETY: NSData is immutable and this slice lives only during the delegate callback.
            if let Err(error) = state.accept_chunk(unsafe { data.as_bytes_unchecked() }) {
                state.outcome = Some(Err(error));
            }
            shared.wake.notify_one();
        }
    }
);

impl TransferDelegate {
    fn new(shared: Arc<TransferShared>) -> objc2::rc::Retained<Self> {
        let this = Self::alloc().set_ivars(shared);
        // SAFETY: NSObject initialization on an AnyThread subclass with owned synchronized ivars.
        unsafe { msg_send![super(this), init] }
    }
}

fn fetch_bytes(
    url: &str,
    source: Source<'_>,
    limit: u64,
    timeout: Duration,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> Result<Vec<u8>, String> {
    objc2::rc::autoreleasepool(|_| {
        let deadline = Instant::now() + timeout;
        let mut next = url.to_owned();
        for hop in 0..=4 {
            check_transfer(deadline, cancel)?;
            let current = safe_url(&next, source, hop != 0)?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            check_transfer(deadline, cancel)?;
            let native_url = NSURL::URLWithString(&NSString::from_str(current.as_str()))
                .ok_or("invalid official URL")?;
            let request = NSMutableURLRequest::requestWithURL_cachePolicy_timeoutInterval(
                &native_url,
                NSURLRequestCachePolicy::ReloadIgnoringLocalCacheData,
                remaining.as_secs_f64(),
            );
            request.setValue_forHTTPHeaderField(
                Some(&NSString::from_str("identity")),
                &NSString::from_str("Accept-Encoding"),
            );
            let config = NSURLSessionConfiguration::ephemeralSessionConfiguration();
            config.setTimeoutIntervalForResource(remaining.as_secs_f64());
            config.setTimeoutIntervalForRequest(remaining.as_secs_f64());
            config.setHTTPShouldSetCookies(false);
            config.setURLCache(None);
            let queue = NSOperationQueue::new();
            queue.setMaxConcurrentOperationCount(1);
            let shared = Arc::new(TransferShared {
                state: Mutex::new(TransferState {
                    bytes: Vec::new(),
                    limit,
                    received: 0,
                    accepted: false,
                    abandoned: false,
                    outcome: None,
                }),
                wake: Condvar::new(),
            });
            let delegate = TransferDelegate::new(Arc::clone(&shared));
            // SAFETY: The serial operation queue invokes only the thread-safe delegate.
            let session = unsafe {
                NSURLSession::sessionWithConfiguration_delegate_delegateQueue(
                    &config,
                    Some(ProtocolObject::from_ref(&*delegate)),
                    Some(&queue),
                )
            };
            let task = session.dataTaskWithRequest(&request);
            task.resume();
            let mut last_progress = 0;
            let result = loop {
                let mut state = shared.state.lock();
                if cancel.load(Ordering::Relaxed) {
                    state.abandoned = true;
                    state.bytes = Vec::new();
                    break Err("official transfer canceled".into());
                }
                if Instant::now() >= deadline {
                    state.abandoned = true;
                    state.bytes = Vec::new();
                    break Err("official transfer timed out".into());
                }
                if state.received != last_progress {
                    last_progress = state.received;
                    drop(state);
                    progress(last_progress, limit);
                    continue;
                }
                if let Some(outcome) = state.outcome.take() {
                    state.abandoned = true;
                    let bytes = std::mem::take(&mut state.bytes);
                    break outcome.map(|redirect| (redirect, bytes));
                }
                shared.wake.wait_for(
                    &mut state,
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(50)),
                );
            };
            task.cancel();
            session.invalidateAndCancel();
            check_transfer(deadline, cancel)?;
            let (redirect, bytes) = result?;
            if let Some(location) = redirect {
                if hop == 4 {
                    return Err("too many official redirects".into());
                }
                let target = current
                    .join(&location)
                    .map_err(|_| "invalid official redirect")?;
                safe_url(target.as_str(), source, true)?;
                next = target.to_string();
                continue;
            }
            check_transfer_length(source, bytes.len() as u64, limit)?;
            return Ok(bytes);
        }
        Err("too many official redirects".into())
    })
}
fn validate_archive_snapshot(
    archive: &[u8],
    pin: &OfficialDownload,
) -> Result<ManagedPack, String> {
    if archive.len() as u64 != pin.bytes {
        return Err("official archive byte count differs from catalog".into());
    }
    let actual_hash = format!("{:x}", Sha256::digest(archive));
    if actual_hash != pin.identity.sha256 {
        return Err("official archive SHA-256 mismatch".into());
    }
    let managed = ManagedPack::load_archive_bytes(archive)
        .map_err(|e| format!("official archive validation failed: {e}"))?;
    if managed.id != pin.identity.id {
        return Err("official archive manifest id does not match catalog".into());
    }
    let manifest: serde_json::Value = serde_json::from_slice(&managed.manifest)
        .map_err(|e| format!("official manifest is invalid: {e}"))?;
    let version = manifest.get("version").and_then(serde_json::Value::as_u64);
    let mode = manifest
        .get("render_mode")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("png");
    if version != Some(u64::from(pin.format_version)) || mode != pin.render_mode {
        return Err("official archive manifest format differs from catalog".into());
    }
    Ok(managed)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_credential_downgrade_and_hostile_redirects() {
        let source = Source::Release(
            "https://github.com/hanbong5938/herdr-characters/releases/download/v1/cat.herdrchar",
        );
        for bad in [
            "http://release-assets.githubusercontent.com/a",
            "https://evil.example/a",
            "https://user@release-assets.githubusercontent.com/a",
            "https://release-assets.githubusercontent.com:444/a",
            "https://github.com/hanbong5938/herdr-characters/releases/download/v1/cat.herdrchar",
        ] {
            assert!(safe_url(bad, source, true).is_err(), "{bad}");
        }
        assert!(safe_url(
            "https://release-assets.githubusercontent.com/download/asset?token=1",
            source,
            true
        )
        .is_ok());
    }
    #[test]
    fn cannot_fetch_other_preview_paths() {
        let source = Source::Preview(
            "https://raw.githubusercontent.com/hanbong5938/herdr-characters/main/previews/cat.png",
        );
        assert!(safe_url(
            "https://raw.githubusercontent.com/hanbong5938/herdr-characters/main/catalog.json",
            source,
            false
        )
        .is_err());
    }
    #[test]
    fn actual_received_count_and_late_completion_are_authoritative() {
        let release = Source::Release(
            "https://github.com/hanbong5938/herdr-characters/releases/download/v1/cat.herdrchar",
        );
        let mut state = TransferState {
            bytes: Vec::new(),
            limit: 4,
            received: 0,
            accepted: true,
            abandoned: false,
            outcome: None,
        };
        state.accept_chunk(&[1, 2, 3]).unwrap();
        assert_eq!(
            check_transfer_length(release, state.received, 4).unwrap_err(),
            "official archive byte count differs from catalog"
        );
        assert_eq!(
            state.accept_chunk(&[4, 5]).unwrap_err(),
            "official response exceeds allowed byte count"
        );
        assert_eq!(state.bytes, vec![1, 2, 3]);
        state.accept_chunk(&[4]).unwrap();
        check_transfer_length(release, state.received, 4).unwrap();
        let cancel = AtomicBool::new(false);
        assert_eq!(
            check_transfer(Instant::now() - Duration::from_millis(1), &cancel).unwrap_err(),
            "official transfer timed out"
        );
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(
            check_transfer(Instant::now() + Duration::from_secs(1), &cancel).unwrap_err(),
            "official transfer canceled"
        );
    }
    #[test]
    fn verified_archive_uses_received_snapshot_and_pinned_manifest_identity() {
        let fixture = std::env::temp_dir().join(format!(
            "herdr-official-snapshot-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&fixture).unwrap();
        let manifest = br#"{"version":2,"format":"herdr.character","id":"snapshot-cat","name":"Snapshot Cat","width":384,"height":512,"poses":{"idle":"idle.png","running":"running.png","waiting":"waiting.png","unknown":"unknown.png"}}"#;
        std::fs::write(fixture.join("manifest.json"), manifest).unwrap();
        let mut image = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut image, 384, 512);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&vec![255u8; 384 * 512 * 4])
                .unwrap();
        }
        for name in ["idle.png", "running.png", "waiting.png", "unknown.png"] {
            std::fs::write(fixture.join(name), &image).unwrap();
        }
        let exported = fixture.join("original.herdrchar");
        ManagedPack::load(&fixture)
            .unwrap()
            .export_archive(&exported)
            .unwrap();
        let archive = std::fs::read(exported).unwrap();
        std::fs::remove_dir_all(&fixture).unwrap();

        let pin = OfficialDownload {
            identity: OfficialPackIdentity {
                id: "snapshot-cat".into(),
                version: "v1".into(),
                release_tag: "v1".into(),
                sha256: format!("{:x}", Sha256::digest(&archive)),
            },
            url:
                "https://github.com/hanbong5938/herdr-characters/releases/download/v1/cat.herdrchar"
                    .into(),
            bytes: archive.len() as u64,
            format_version: 2,
            render_mode: "png".into(),
        };
        assert_eq!(
            validate_archive_snapshot(&archive, &pin).unwrap().id,
            pin.identity.id
        );
        let mut altered = archive.clone();
        altered[40] ^= 1;
        assert_eq!(
            validate_archive_snapshot(&altered, &pin).unwrap_err(),
            "official archive SHA-256 mismatch"
        );
        assert_eq!(
            validate_archive_snapshot(&archive[..archive.len() - 1], &pin).unwrap_err(),
            "official archive byte count differs from catalog"
        );
        let mut wrong_count = pin.clone();
        wrong_count.bytes -= 1;
        assert_eq!(
            validate_archive_snapshot(&archive, &wrong_count).unwrap_err(),
            "official archive byte count differs from catalog"
        );
        let mut wrong_id = pin.clone();
        wrong_id.identity.id = "some-other-pack".into();
        assert_eq!(
            validate_archive_snapshot(&archive, &wrong_id).unwrap_err(),
            "official archive manifest id does not match catalog"
        );
        let mut wrong_format = pin.clone();
        wrong_format.format_version = 3;
        assert_eq!(
            validate_archive_snapshot(&archive, &wrong_format).unwrap_err(),
            "official archive manifest format differs from catalog"
        );
    }
}
