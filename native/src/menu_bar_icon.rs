use crate::lifecycle::{config_directory, validate_directory};
use crate::preferences::MenuBarIconPreference;
use flate2::{Crc, Decompress, FlushDecompress, Status};
use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{NSBitmapFormat, NSBitmapImageRep, NSDeviceRGBColorSpace, NSImage};
use objc2_foundation::{NSSize, NSString};
use sha2::{Digest, Sha256};
use std::ffi::{CStr, CString};
#[cfg(test)]
use std::fs;
use std::fs::{File, OpenOptions, Permissions};
use std::io::{Cursor, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_ENCODED: usize = 4 * 1024 * 1024;
const MAX_PIXELS: u64 = 1_000_000;
const ICON_DIRECTORY: &str = "menu-bar-icons";
const ICON_SIZE: u32 = 18;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) struct PreparedMenuBarIcon {
    pub(crate) preference: MenuBarIconPreference,
    pub(crate) image: Retained<NSImage>,
    bytes: Vec<u8>,
    directory: std::path::PathBuf,
}

impl PreparedMenuBarIcon {
    /// Publish the immutable asset before committing `preference` to preferences.json.
    /// A failed preferences commit may leave an unreferenced asset, never a broken old icon.
    pub(crate) fn publish(&self) -> Result<(), String> {
        publish_in_directory(&self.directory, &self.preference.asset, &self.bytes)
    }
}

pub(crate) fn default_image(
    _mtm: MainThreadMarker,
    description: &str,
) -> Result<Retained<NSImage>, String> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str("pawprint.fill"),
        Some(&NSString::from_str(description)),
    )
    .ok_or_else(|| "the default menu bar symbol is unavailable".to_owned())?;
    image.setTemplate(true);
    image.setSize(NSSize::new(f64::from(ICON_SIZE), f64::from(ICON_SIZE)));
    Ok(image)
}

pub(crate) fn prepare_source(
    path: &Path,
    mtm: MainThreadMarker,
) -> Result<PreparedMenuBarIcon, String> {
    let directory = config_directory()?;
    prepare_source_in_directory(path, mtm, &directory)
}

fn read_source_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| format!("cannot open image: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("cannot inspect image: {error}"))?;
    if !metadata.is_file() {
        return Err("image source must be a regular file".to_owned());
    }
    let length = metadata.len();
    if length == 0 || length > MAX_ENCODED as u64 {
        return Err("PNG must be nonempty and at most 4 MiB".to_owned());
    }
    let mut bytes = Vec::with_capacity(length as usize);
    Read::by_ref(&mut file)
        .take((MAX_ENCODED + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read image: {error}"))?;
    Ok(bytes)
}

fn prepare_source_in_directory(
    path: &Path,
    mtm: MainThreadMarker,
    directory: &Path,
) -> Result<PreparedMenuBarIcon, String> {
    let bytes = read_source_bytes(path)?;
    let decoded = decode_png(&bytes)?;
    let image = native_image(&decoded, mtm)?;
    let asset = name_for_bytes(&bytes);
    Ok(PreparedMenuBarIcon {
        preference: MenuBarIconPreference { asset },
        image,
        bytes,
        directory: directory.to_path_buf(),
    })
}

pub(crate) fn load_saved(
    preference: &MenuBarIconPreference,
    mtm: MainThreadMarker,
) -> Result<Retained<NSImage>, String> {
    load_saved_in_directory(preference, mtm, &config_directory()?)
}

fn load_saved_in_directory(
    preference: &MenuBarIconPreference,
    mtm: MainThreadMarker,
    directory: &Path,
) -> Result<Retained<NSImage>, String> {
    native_image(
        &decode_png(&load_bytes_in_directory(preference, directory)?)?,
        mtm,
    )
}

fn load_bytes_in_directory(
    preference: &MenuBarIconPreference,
    directory: &Path,
) -> Result<Vec<u8>, String> {
    validate_asset_name(&preference.asset)?;
    let managed = managed_directory(directory, false)?;
    let bytes = read_managed(&managed, &preference.asset)?;
    if name_for_bytes(&bytes) != preference.asset {
        return Err("saved menu bar icon has an unexpected digest".to_owned());
    }
    Ok(bytes)
}

pub(crate) fn validate_asset_name(name: &str) -> Result<(), String> {
    let Some(hash) = name
        .strip_prefix("icon-")
        .and_then(|name| name.strip_suffix(".png"))
    else {
        return Err("invalid managed menu bar icon name".to_owned());
    };
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("invalid managed menu bar icon digest".to_owned());
    }
    Ok(())
}

fn name_for_bytes(bytes: &[u8]) -> String {
    format!("icon-{:x}.png", Sha256::digest(bytes))
}

/// Called only after a successful preferences rename AND a confirmed preferences directory sync.
/// Refuses to follow links or unlink an asset not owned by this user; cleanup failure is not a
/// preferences failure, since the new choice is already committed.
pub(crate) fn cleanup_previous_in_directory(directory: &Path, name: &str) {
    if validate_asset_name(name).is_err() {
        return;
    }
    let Ok(managed) = managed_directory(directory, false) else {
        return;
    };
    if open_managed(&managed, name).is_err() {
        return;
    }
    if let Ok(name) = CString::new(name) {
        if unsafe { libc::unlinkat(managed.as_raw_fd(), name.as_ptr(), 0) } == 0 {
            let _ = managed.sync_all();
        }
    }
}

fn managed_directory(root: &Path, create: bool) -> Result<File, String> {
    validate_directory(root, create)?;
    let root_name = CString::new(root.as_os_str().as_bytes())
        .map_err(|_| "invalid config directory path".to_owned())?;
    let root_fd = unsafe {
        libc::open(
            root_name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if root_fd < 0 {
        return Err(format!(
            "cannot open config directory: {}",
            std::io::Error::last_os_error()
        ));
    }
    let root = unsafe { File::from_raw_fd(root_fd) };
    let name = cstring(ICON_DIRECTORY)?;
    let mut created = false;
    if create {
        let result = unsafe { libc::mkdirat(root.as_raw_fd(), name.as_ptr(), 0o700) };
        if result == 0 {
            created = true;
        } else if std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists {
            return Err(format!(
                "cannot create menu bar icon directory: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    let fd = unsafe {
        libc::openat(
            root.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(format!(
            "cannot open menu bar icon directory: {}",
            std::io::Error::last_os_error()
        ));
    }
    let managed = unsafe { File::from_raw_fd(fd) };
    let metadata = managed
        .metadata()
        .map_err(|error| format!("cannot inspect menu bar icon directory: {error}"))?;
    if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
        return Err("menu bar icon directory must belong to the current user".to_owned());
    }
    if metadata.permissions().mode() & 0o7777 != 0o700 {
        if !create {
            return Err("menu bar icon directory must be private (0700)".to_owned());
        }
        managed
            .set_permissions(Permissions::from_mode(0o700))
            .map_err(|error| format!("cannot secure menu bar icon directory: {error}"))?;
        managed
            .sync_all()
            .map_err(|error| format!("cannot sync menu bar icon directory: {error}"))?;
    }
    if created {
        managed
            .sync_all()
            .map_err(|error| format!("cannot sync menu bar icon directory: {error}"))?;
        root.sync_all()
            .map_err(|error| format!("cannot sync config directory: {error}"))?;
    }
    Ok(managed)
}

fn cstring(name: &str) -> Result<CString, String> {
    CString::new(name).map_err(|_| "invalid menu bar icon filename".to_owned())
}

fn open_managed_links(directory: &File, name: &str, links: u64) -> Result<File, String> {
    validate_asset_name(name)?;
    let name = cstring(name)?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(format!(
            "saved menu bar icon cannot be opened: {}",
            std::io::Error::last_os_error()
        ));
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file
        .metadata()
        .map_err(|error| format!("cannot inspect saved menu bar icon: {error}"))?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o7777 != 0o600
        || metadata.nlink() != links
    {
        return Err("saved menu bar icon must be an owned private regular file".to_owned());
    }
    if metadata.len() == 0 || metadata.len() > MAX_ENCODED as u64 {
        return Err("saved menu bar icon exceeds the PNG size budget".to_owned());
    }
    Ok(file)
}

fn open_managed(directory: &File, name: &str) -> Result<File, String> {
    open_managed_links(directory, name, 1)
}

fn read_managed(directory: &File, name: &str) -> Result<Vec<u8>, String> {
    let file = open_managed(directory, name)?;
    let length = file
        .metadata()
        .map_err(|error| format!("cannot inspect saved menu bar icon: {error}"))?
        .len();
    let mut bytes = Vec::with_capacity(length as usize);
    file.take((MAX_ENCODED + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read saved menu bar icon: {error}"))?;
    if bytes.is_empty() || bytes.len() > MAX_ENCODED {
        return Err("saved menu bar icon exceeds the PNG size budget".to_owned());
    }
    Ok(bytes)
}

fn matching_asset(directory: &File, name: &str, bytes: &[u8], links: u64) -> Result<File, String> {
    let mut file = open_managed_links(directory, name, links)?;
    if file
        .metadata()
        .map_err(|error| format!("cannot inspect saved menu bar icon: {error}"))?
        .len()
        != bytes.len() as u64
    {
        return Err("managed menu bar icon name is occupied by different contents".to_owned());
    }
    let mut scratch = [0u8; 8192];
    for expected in bytes.chunks(scratch.len()) {
        let actual = &mut scratch[..expected.len()];
        file.read_exact(&mut *actual)
            .map_err(|error| format!("cannot read saved menu bar icon: {error}"))?;
        if &*actual != expected {
            return Err("managed menu bar icon name is occupied by different contents".to_owned());
        }
    }
    if file
        .read(&mut scratch[..1])
        .map_err(|error| format!("cannot read saved menu bar icon: {error}"))?
        != 0
    {
        return Err("managed menu bar icon name is occupied by different contents".to_owned());
    }
    Ok(file)
}

fn legacy_temp_name(name: &str) -> bool {
    let Some(suffix) = name.strip_prefix(".icon-") else {
        return false;
    };
    let mut parts = suffix.split('-');
    parts.clone().count() == 3
        && parts.all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

// The old linkat publisher left a hard link to a temp filename if it was interrupted.
// Repair only an identical asset on an explicit reimport; saved loads remain nlink=1 only.
fn repair_legacy_alias(directory: &File, name: &str, bytes: &[u8]) -> Result<(), String> {
    let final_file = matching_asset(directory, name, bytes, 2)?;
    let target = final_file
        .metadata()
        .map_err(|error| format!("cannot inspect saved menu bar icon: {error}"))?;
    let duplicate = unsafe { libc::dup(directory.as_raw_fd()) };
    if duplicate < 0 {
        return Err(format!(
            "cannot enumerate icon directory: {}",
            std::io::Error::last_os_error()
        ));
    }
    let stream = unsafe { libc::fdopendir(duplicate) };
    if stream.is_null() {
        let error = std::io::Error::last_os_error();
        unsafe { libc::close(duplicate) };
        return Err(format!("cannot enumerate icon directory: {error}"));
    }
    let found = (|| {
        let mut alias = None;
        loop {
            unsafe { *libc::__error() = 0 };
            let entry = unsafe { libc::readdir(stream) };
            if entry.is_null() {
                if std::io::Error::last_os_error().raw_os_error() != Some(0) {
                    return Err(format!(
                        "cannot enumerate icon directory: {}",
                        std::io::Error::last_os_error()
                    ));
                }
                break;
            }
            let raw = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
            let Ok(candidate) = raw.to_str() else {
                continue;
            };
            if !legacy_temp_name(candidate) {
                continue;
            }
            let fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    raw.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                continue;
            }
            let file = unsafe { File::from_raw_fd(fd) };
            let metadata = file
                .metadata()
                .map_err(|error| format!("cannot inspect old icon alias: {error}"))?;
            if metadata.is_file()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.permissions().mode() & 0o7777 == 0o600
                && metadata.nlink() == 2
                && metadata.dev() == target.dev()
                && metadata.ino() == target.ino()
            {
                if alias.replace(raw.to_owned()).is_some() {
                    return Err("old icon has ambiguous temporary aliases".to_owned());
                }
            }
        }
        alias.ok_or_else(|| "old icon has no safe temporary alias".to_owned())
    })();
    unsafe { libc::closedir(stream) };
    let alias = found?;
    if unsafe { libc::unlinkat(directory.as_raw_fd(), alias.as_ptr(), 0) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(format!("cannot remove old icon alias: {error}"));
        }
    }
    // An ENOENT race is safe only if another repair already produced this exact nlink=1 asset.
    matching_asset(directory, name, bytes, 1)?;
    directory
        .sync_all()
        .map_err(|error| format!("cannot sync menu bar icon directory: {error}"))
}

fn publish_in_directory(root: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    validate_asset_name(name)?;
    if bytes.is_empty() || bytes.len() > MAX_ENCODED || name_for_bytes(bytes) != name {
        return Err("menu bar icon contents do not match the managed asset name".to_owned());
    }
    let directory = managed_directory(root, true)?;
    let basename = cstring(name)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp = cstring(&format!(
        ".icon-{}-{nonce}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            temp.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(format!(
            "cannot create temporary menu bar icon: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    let renamed = (|| {
        file.set_permissions(Permissions::from_mode(0o600))
            .map_err(|error| format!("cannot secure temporary menu bar icon: {error}"))?;
        file.write_all(bytes)
            .map_err(|error| format!("cannot write menu bar icon: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("cannot sync menu bar icon: {error}"))?;
        drop(file);
        let result = unsafe {
            libc::renameatx_np(
                directory.as_raw_fd(),
                temp.as_ptr(),
                directory.as_raw_fd(),
                basename.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        if result == 0 {
            return Ok(true);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            Ok(false)
        } else {
            Err(format!("cannot publish menu bar icon: {error}"))
        }
    })();
    if matches!(&renamed, Ok(true)) {
        // The rename has committed an immutable nlink=1 file. Never remove it if syncing fails.
        return directory
            .sync_all()
            .map_err(|error| format!("cannot sync menu bar icon directory: {error}"));
    }
    if unsafe { libc::unlinkat(directory.as_raw_fd(), temp.as_ptr(), 0) } != 0 {
        return Err(format!(
            "cannot remove temporary menu bar icon: {}",
            std::io::Error::last_os_error()
        ));
    }
    directory
        .sync_all()
        .map_err(|error| format!("cannot sync menu bar icon directory: {error}"))?;
    renamed?;
    if matching_asset(&directory, name, bytes, 1).is_ok() {
        return Ok(());
    }
    repair_legacy_alias(&directory, name, bytes)
}

#[derive(Debug)]
struct DecodedPng {
    width: u32,
    height: u32,
    channels: usize,
    pixels: Vec<u8>,
    first_visible: usize,
}

fn png_raw_size(
    width: u64,
    height: u64,
    samples: u64,
    depth: u64,
    interlace: u8,
) -> Result<usize, String> {
    fn pass_bytes(width: u64, height: u64, samples: u64, depth: u64) -> Option<u64> {
        if width == 0 || height == 0 {
            return Some(0);
        }
        let row_bits = width.checked_mul(samples)?.checked_mul(depth)?;
        let row = row_bits.checked_add(7)?.checked_div(8)?.checked_add(1)?;
        height.checked_mul(row)
    }
    let count = if interlace == 0 {
        pass_bytes(width, height, samples, depth)
    } else {
        let mut total = Some(0u64);
        for (x, y, dx, dy) in [
            (0, 0, 8, 8),
            (4, 0, 8, 8),
            (0, 4, 4, 8),
            (2, 0, 4, 4),
            (0, 2, 2, 4),
            (1, 0, 2, 2),
            (0, 1, 1, 2),
        ] {
            let pass_width = if width <= x {
                0
            } else {
                1 + (width - 1 - x) / dx
            };
            let pass_height = if height <= y {
                0
            } else {
                1 + (height - 1 - y) / dy
            };
            total = total.and_then(|sum| {
                pass_bytes(pass_width, pass_height, samples, depth)
                    .and_then(|pass| sum.checked_add(pass))
            });
        }
        total
    };
    usize::try_from(count.ok_or("PNG raw image size overflows")?)
        .map_err(|_| "PNG raw image size overflows".to_owned())
}

fn check_png_chunks(bytes: &[u8]) -> Result<(), String> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("image is not a PNG".to_owned());
    }
    let mut offset = 8;
    let mut first = true;
    let mut seen_idat = false;
    let mut past_idat = false;
    let mut expected_raw = 0;
    let mut decompressed = 0;
    let mut finished = false;
    let mut inflater = None;
    let mut scratch = [0u8; 8192];
    let mut crc = Crc::new();
    loop {
        let header = bytes
            .get(offset..offset + 8)
            .ok_or("PNG is truncated before IEND")?;
        let length = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
        let end = offset
            .checked_add(12)
            .and_then(|n| n.checked_add(length))
            .ok_or("PNG chunk size overflows")?;
        let chunk = bytes.get(offset..end).ok_or("PNG chunk is truncated")?;
        let kind = &header[4..8];
        let payload = &chunk[8..8 + length];
        crc.reset();
        crc.update(kind);
        crc.update(payload);
        if crc.sum() != u32::from_be_bytes(chunk[8 + length..].try_into().unwrap()) {
            return Err("PNG chunk checksum is invalid".to_owned());
        }
        if first {
            if kind != b"IHDR" || length != 13 {
                return Err("PNG must begin with a 13-byte IHDR".to_owned());
            }
            first = false;
            let width = u32::from_be_bytes(payload[0..4].try_into().unwrap()) as u64;
            let height = u32::from_be_bytes(payload[4..8].try_into().unwrap()) as u64;
            if width == 0
                || height == 0
                || width.checked_mul(height).map_or(true, |n| n > MAX_PIXELS)
            {
                return Err("PNG exceeds the 1,000,000 pixel budget".to_owned());
            }
            let (depth, color) = (payload[8], payload[9]);
            let samples = match color {
                0 if matches!(depth, 1 | 2 | 4 | 8 | 16) => 1,
                3 if matches!(depth, 1 | 2 | 4 | 8) => 1,
                2 if matches!(depth, 8 | 16) => 3,
                4 if matches!(depth, 8 | 16) => 2,
                6 if matches!(depth, 8 | 16) => 4,
                _ => return Err("PNG has an unsupported color format".to_owned()),
            };
            if payload[10] != 0 || payload[11] != 0 || !matches!(payload[12], 0 | 1) {
                return Err("PNG has unsupported image methods".to_owned());
            }
            expected_raw = png_raw_size(width, height, samples, u64::from(depth), payload[12])?;
            inflater = Some(Decompress::new(true));
        } else if kind == b"IHDR" {
            return Err("PNG has multiple image headers".to_owned());
        }
        if matches!(kind, b"acTL" | b"fcTL" | b"fdAT") {
            return Err("animated PNG images are not supported".to_owned());
        }
        if kind == b"IDAT" {
            if past_idat {
                return Err("PNG image data is not contiguous".to_owned());
            }
            seen_idat = true;
            let inflater = inflater.as_mut().expect("validated IHDR precedes IDAT");
            let mut input = payload;
            if finished && !input.is_empty() {
                return Err("PNG contains extra compressed image data".to_owned());
            }
            while !input.is_empty() {
                let limit = scratch.len().min(expected_raw - decompressed + 1);
                let before_in = inflater.total_in();
                let before_out = inflater.total_out();
                let status = inflater
                    .decompress(input, &mut scratch[..limit], FlushDecompress::None)
                    .map_err(|error| format!("PNG image stream is invalid: {error}"))?;
                let consumed = (inflater.total_in() - before_in) as usize;
                let produced = (inflater.total_out() - before_out) as usize;
                decompressed += produced;
                if decompressed > expected_raw {
                    return Err("PNG image stream expands beyond its dimensions".to_owned());
                }
                input = &input[consumed..];
                if status == Status::StreamEnd {
                    finished = true;
                    if !input.is_empty() {
                        return Err("PNG contains extra compressed image data".to_owned());
                    }
                    break;
                }
                if consumed == 0 && produced == 0 {
                    return Err("PNG image stream made no progress".to_owned());
                }
            }
            // An inflater may consume the final compressed byte while still holding decoded
            // output in its internal buffer. Drain it with the same one-byte overflow sentinel;
            // no progress simply means the next IDAT must provide more input.
            while !finished {
                let limit = scratch.len().min(expected_raw - decompressed + 1);
                let before_out = inflater.total_out();
                let status = inflater
                    .decompress(&[], &mut scratch[..limit], FlushDecompress::None)
                    .map_err(|error| format!("PNG image stream is invalid: {error}"))?;
                let produced = (inflater.total_out() - before_out) as usize;
                decompressed += produced;
                if decompressed > expected_raw {
                    return Err("PNG image stream expands beyond its dimensions".to_owned());
                }
                if status == Status::StreamEnd {
                    finished = true;
                } else if produced == 0 {
                    break;
                }
            }
        } else if seen_idat {
            past_idat = true;
        }
        if kind == b"IEND" {
            if length != 0
                || !seen_idat
                || end != bytes.len()
                || !finished
                || decompressed != expected_raw
            {
                return Err("PNG has an incomplete image stream or invalid end marker".to_owned());
            }
            return Ok(());
        }
        offset = end;
    }
}

fn decode_png(bytes: &[u8]) -> Result<DecodedPng, String> {
    if bytes.is_empty() || bytes.len() > MAX_ENCODED {
        return Err("PNG must be nonempty and at most 4 MiB".to_owned());
    }
    check_png_chunks(bytes)?;
    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(bytes),
        png::Limits {
            bytes: 8 * MAX_ENCODED,
        },
    );
    // Preflight verified every chunk CRC and the IDAT zlib Adler on these same immutable bytes;
    // only duplicate checksum work is skipped in the decoder.
    decoder.ignore_checksums(true);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("invalid PNG: {error}"))?;
    let (width, height) = (reader.info().width, reader.info().height);
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("PNG exceeds the 1,000,000 pixel budget".to_owned());
    }
    if reader.info().animation_control.is_some() {
        return Err("animated PNG images are not supported".to_owned());
    }
    let (color, depth) = reader.output_color_type();
    if depth != png::BitDepth::Eight {
        return Err("PNG cannot be normalized to 8-bit color".to_owned());
    }
    let channels = match color {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err("PNG palette could not be expanded".to_owned()),
    };
    let expected = width as usize * height as usize * channels;
    if reader.output_buffer_size() != expected {
        return Err("PNG decoded output has an unexpected size".to_owned());
    }
    let mut pixels = vec![0; expected];
    let info = reader
        .next_frame(&mut pixels)
        .map_err(|error| format!("cannot decode PNG: {error}"))?;
    if info.buffer_size() != expected {
        return Err("PNG decoded output is incomplete".to_owned());
    }
    reader
        .finish()
        .map_err(|error| format!("PNG is incomplete: {error}"))?;
    let first_visible = if matches!(channels, 2 | 4) {
        pixels
            .chunks_exact(channels)
            .position(|pixel| pixel[channels - 1] != 0)
            .ok_or_else(|| "PNG is fully transparent".to_owned())?
    } else {
        0
    };
    Ok(DecodedPng {
        width,
        height,
        channels,
        pixels,
        first_visible,
    })
}

#[derive(Debug, Eq, PartialEq)]
struct Fit {
    width: u32,
    height: u32,
    left: u32,
    top: u32,
}

fn fit(width: u32, height: u32, canvas: u32) -> Fit {
    let (fitted_width, fitted_height) = if width >= height {
        (
            canvas,
            (u64::from(height) * u64::from(canvas) / u64::from(width)).max(1) as u32,
        )
    } else {
        (
            (u64::from(width) * u64::from(canvas) / u64::from(height)).max(1) as u32,
            canvas,
        )
    };
    Fit {
        width: fitted_width,
        height: fitted_height,
        left: (canvas - fitted_width) / 2,
        top: (canvas - fitted_height) / 2,
    }
}

fn color_at(source: &DecodedPng, x: usize, y: usize) -> [u8; 4] {
    let index = (y * source.width as usize + x) * source.channels;
    let p = &source.pixels[index..index + source.channels];
    match source.channels {
        1 => [p[0], p[0], p[0], 255],
        2 => [p[0], p[0], p[0], p[1]],
        3 => [p[0], p[1], p[2], 255],
        4 => [p[0], p[1], p[2], p[3]],
        _ => unreachable!(),
    }
}

fn paint_canvas(source: &DecodedPng, canvas: u32) -> Vec<u8> {
    let bounds = fit(source.width, source.height, canvas);
    let mut output = vec![0u8; canvas as usize * canvas as usize * 4];
    for y in 0..bounds.height {
        for x in 0..bounds.width {
            let sx = ((f64::from(x) + 0.5) * f64::from(source.width) / f64::from(bounds.width)
                - 0.5)
                .clamp(0.0, f64::from(source.width - 1));
            let sy = ((f64::from(y) + 0.5) * f64::from(source.height) / f64::from(bounds.height)
                - 0.5)
                .clamp(0.0, f64::from(source.height - 1));
            let (x0, y0) = (sx.floor() as usize, sy.floor() as usize);
            let (x1, y1) = (
                (x0 + 1).min(source.width as usize - 1),
                (y0 + 1).min(source.height as usize - 1),
            );
            let (fx, fy) = (sx.fract(), sy.fract());
            let corners = [
                (color_at(source, x0, y0), (1.0 - fx) * (1.0 - fy)),
                (color_at(source, x1, y0), fx * (1.0 - fy)),
                (color_at(source, x0, y1), (1.0 - fx) * fy),
                (color_at(source, x1, y1), fx * fy),
            ];
            let alpha: f64 = corners
                .iter()
                .map(|(pixel, weight)| f64::from(pixel[3]) * weight)
                .sum();
            let index = ((y + bounds.top) * canvas + x + bounds.left) as usize * 4;
            output[index + 3] = alpha.round() as u8;
            if alpha > 0.0 {
                for channel in 0..3 {
                    let premultiplied: f64 = corners
                        .iter()
                        .map(|(pixel, weight)| {
                            f64::from(pixel[channel]) * f64::from(pixel[3]) * weight
                        })
                        .sum();
                    output[index + channel] = (premultiplied / alpha).round() as u8;
                }
            }
        }
    }
    // A tiny isolated opaque pixel can fall between both downsampling grids.
    // Preserve one original-color sample if the entire raster would vanish.
    if output.chunks_exact(4).all(|pixel| pixel[3] == 0) {
        let sx = source.first_visible % source.width as usize;
        let sy = source.first_visible / source.width as usize;
        let x = (sx as u64 * bounds.width as u64 / source.width as u64) as u32;
        let y = (sy as u64 * bounds.height as u64 / source.height as u64) as u32;
        let index = ((bounds.top + y) * canvas + bounds.left + x) as usize * 4;
        output[index..index + 4].copy_from_slice(&color_at(source, sx, sy));
    }
    output
}

fn native_image(decoded: &DecodedPng, _mtm: MainThreadMarker) -> Result<Retained<NSImage>, String> {
    let image = NSImage::initWithSize(NSImage::alloc(), NSSize::new(18.0, 18.0));
    for dimension in [ICON_SIZE, ICON_SIZE * 2] {
        let rgba = paint_canvas(decoded, dimension);
        // Null planes ask AppKit to allocate bitmap storage owned by the representation.
        let rep = unsafe {
            NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
                NSBitmapImageRep::alloc(), std::ptr::null_mut(), dimension as isize, dimension as isize,
                8, 4, true, false, NSDeviceRGBColorSpace, NSBitmapFormat::AlphaNonpremultiplied,
                0, 0,
            )
        }.ok_or_else(|| "cannot allocate menu bar bitmap".to_owned())?;
        let stride =
            usize::try_from(rep.bytesPerRow()).map_err(|_| "invalid menu bar bitmap stride")?;
        if stride < dimension as usize * 4 || rep.bitmapData().is_null() {
            return Err("invalid menu bar bitmap storage".to_owned());
        }
        for (row, pixels) in rgba.chunks_exact(dimension as usize * 4).enumerate() {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    pixels.as_ptr(),
                    rep.bitmapData().add(row * stride),
                    pixels.len(),
                );
            }
        }
        rep.setSize(NSSize::new(18.0, 18.0));
        image.addRepresentation(&rep);
    }
    image.setTemplate(false);
    if !image.isValid() {
        return Err("menu bar bitmap is invalid".to_owned());
    }
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn png(
        width: u32,
        height: u32,
        color: png::ColorType,
        depth: png::BitDepth,
        pixels: &[u8],
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, width, height);
            encoder.set_color(color);
            encoder.set_depth(depth);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(pixels).unwrap();
        }
        bytes
    }

    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        let mut crc = Crc::new();
        crc.update(kind);
        crc.update(payload);
        out.extend_from_slice(&crc.sum().to_be_bytes());
    }

    fn zlib(raw: &[u8]) -> Vec<u8> {
        let mut compressor =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        compressor.write_all(raw).unwrap();
        compressor.finish().unwrap()
    }

    fn assembled_png(
        width: u32,
        height: u32,
        depth: u8,
        color: u8,
        interlace: u8,
        idat: &[&[u8]],
    ) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = [0u8; 13];
        ihdr[..4].copy_from_slice(&width.to_be_bytes());
        ihdr[4..8].copy_from_slice(&height.to_be_bytes());
        ihdr[8] = depth;
        ihdr[9] = color;
        ihdr[12] = interlace;
        chunk(&mut bytes, b"IHDR", &ihdr);
        for data in idat {
            chunk(&mut bytes, b"IDAT", data);
        }
        chunk(&mut bytes, b"IEND", &[]);
        bytes
    }

    fn insert_before(bytes: &mut Vec<u8>, before: &[u8; 4], kind: &[u8; 4], payload: &[u8]) {
        let position = bytes
            .windows(4)
            .position(|window| window == before)
            .unwrap()
            - 4;
        let mut inserted = Vec::new();
        chunk(&mut inserted, kind, payload);
        bytes.splice(position..position, inserted);
    }

    fn corrupt_chunk_crc(bytes: &mut [u8], kind: &[u8; 4]) {
        let start = bytes.windows(4).position(|window| window == kind).unwrap() - 4;
        let length = u32::from_be_bytes(bytes[start..start + 4].try_into().unwrap()) as usize;
        bytes[start + 8 + length] ^= 1;
    }

    fn isolated_directory() -> std::path::PathBuf {
        let id = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("herdr-icon-test-{}-{id}", std::process::id()));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn source_accepts_regular_png_and_symlink_and_rejects_nonfiles_and_size_bounds() {
        let directory = isolated_directory();
        let source = directory.join("source.png");
        let link = directory.join("link.png");
        let bytes = png(
            2,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &[3, 20, 70, 80, 50, 1],
        );
        fs::write(&source, &bytes).unwrap();
        symlink(&source, &link).unwrap();
        for path in [&source, &link] {
            let imported = read_source_bytes(path).unwrap();
            let decoded = decode_png(&imported).unwrap();
            assert_eq!(color_at(&decoded, 0, 0), [3, 20, 70, 255]);
            assert_eq!(color_at(&decoded, 1, 0), [80, 50, 1, 255]);
        }
        assert!(read_source_bytes(&directory).is_err());
        fs::write(&source, []).unwrap();
        assert!(read_source_bytes(&source).is_err());
        fs::write(&source, vec![0; MAX_ENCODED]).unwrap();
        assert_eq!(read_source_bytes(&source).unwrap().len(), MAX_ENCODED);
        fs::write(&source, vec![0; MAX_ENCODED + 1]).unwrap();
        assert!(read_source_bytes(&source).is_err());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn fifo_source_without_writer_is_rejected_before_read() {
        use std::process::Command;
        use std::time::{Duration, Instant};
        if let Some(path) = std::env::var_os("HERDR_ICON_FIFO_CHILD_PATH") {
            assert!(read_source_bytes(Path::new(&path)).is_err());
            return;
        }

        let directory = isolated_directory();
        let fifo = directory.join("source.png");
        let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "menu_bar_icon::tests::fifo_source_without_writer_is_rejected_before_read",
            ])
            .env("HERDR_ICON_FIFO_CHILD_PATH", &fifo)
            .spawn()
            .unwrap();
        let started = Instant::now();
        let result = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if started.elapsed() < Duration::from_secs(10) => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    let killed = child.kill();
                    let reaped = child.wait();
                    break Err(format!(
                        "FIFO child timed out: kill={killed:?}, wait={reaped:?}"
                    ));
                }
                Err(error) => {
                    let killed = child.kill();
                    let reaped = child.wait();
                    break Err(format!(
                        "cannot inspect FIFO child: {error}; kill={killed:?}, wait={reaped:?}"
                    ));
                }
            }
        };
        let _ = fs::remove_dir_all(directory);
        assert!(result.unwrap().success(), "FIFO child failed");
    }

    #[test]
    fn normalizes_grayscale_palette_and_sixteen_bit_without_losing_color_or_alpha() {
        let gray = png(
            2,
            1,
            png::ColorType::Grayscale,
            png::BitDepth::Eight,
            &[0x33, 0x99],
        );
        let gray = decode_png(&gray).unwrap();
        assert_eq!(color_at(&gray, 0, 0), [0x33, 0x33, 0x33, 255]);
        assert_eq!(color_at(&gray, 1, 0), [0x99, 0x99, 0x99, 255]);

        let mut indexed = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut indexed, 2, 1);
            encoder.set_color(png::ColorType::Indexed);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_palette(vec![255, 20, 4, 0, 80, 200]);
            encoder.set_trns(vec![0, 200]);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[0, 1]).unwrap();
        }
        let palette = decode_png(&indexed).unwrap();
        assert_eq!(color_at(&palette, 0, 0), [255, 20, 4, 0]);
        assert_eq!(color_at(&palette, 1, 0), [0, 80, 200, 200]);

        let sixteen = png(
            1,
            1,
            png::ColorType::Rgba,
            png::BitDepth::Sixteen,
            &[0x81, 0x11, 0x42, 0xff, 0x09, 0x10, 0x80, 0x00],
        );
        let decoded = decode_png(&sixteen).unwrap();
        assert_eq!(color_at(&decoded, 0, 0), [0x81, 0x42, 0x09, 0x80]);
        let gray_alpha = png(
            1,
            1,
            png::ColorType::GrayscaleAlpha,
            png::BitDepth::Sixteen,
            &[0xb1, 0x23, 0x9f, 0x40],
        );
        assert_eq!(
            color_at(&decode_png(&gray_alpha).unwrap(), 0, 0),
            [0xb1, 0xb1, 0xb1, 0x9f]
        );
        let rgb = png(
            1,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Sixteen,
            &[0x21, 0xff, 0x62, 0x01, 0xd3, 0x00],
        );
        assert_eq!(
            color_at(&decode_png(&rgb).unwrap(), 0, 0),
            [0x21, 0x62, 0xd3, 255]
        );
    }

    #[test]
    fn rejects_invalid_transparent_animated_and_oversized_pngs() {
        let rgba = png(
            1,
            1,
            png::ColorType::Rgba,
            png::BitDepth::Eight,
            &[10, 20, 30, 0],
        );
        assert!(decode_png(&rgba).is_err());
        let visible = png(
            1,
            1,
            png::ColorType::Rgba,
            png::BitDepth::Eight,
            &[10, 20, 30, 255],
        );
        let mut corrupt = visible.clone();
        corrupt_chunk_crc(&mut corrupt, b"IDAT");
        assert!(decode_png(&corrupt).is_err());
        assert!(decode_png(&rgba[..rgba.len() - 1]).is_err());
        assert!(decode_png(&[0; 40]).is_err());
        let static_png = png(
            1,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &[255, 20, 0],
        );
        for (kind, payload) in [
            (&b"acTL", &[0, 0, 0, 2, 0, 0, 0, 0][..]),
            (&b"fcTL", &[0, 0, 0, 0][..]),
            (&b"fdAT", &[0, 0, 0, 0][..]),
        ] {
            let mut animated = static_png.clone();
            insert_before(&mut animated, b"IDAT", kind, payload);
            assert!(decode_png(&animated).is_err());
        }
        let large = png(
            1001,
            1000,
            png::ColorType::Grayscale,
            png::BitDepth::Eight,
            &vec![255; 1_001_000],
        );
        assert!(decode_png(&large).is_err());
        assert!(decode_png(&vec![0; MAX_ENCODED + 1]).is_err());
        let mut trailing = static_png.clone();
        trailing.push(0);
        assert!(decode_png(&trailing).is_err());
    }

    #[test]
    fn bounded_png_stream_requires_exact_raw_bytes_and_a_single_complete_zlib_stream() {
        let raw = [0, 15, 40, 90];
        let compressed = zlib(&raw);
        let valid = assembled_png(1, 1, 8, 2, 0, &[&compressed]);
        assert_eq!(
            color_at(&decode_png(&valid).unwrap(), 0, 0),
            [15, 40, 90, 255]
        );
        for bad_raw in [&raw[..3], &[0, 15, 40, 90, 0][..]] {
            let stream = zlib(bad_raw);
            assert!(decode_png(&assembled_png(1, 1, 8, 2, 0, &[&stream])).is_err());
        }
        let mut truncated = compressed.clone();
        truncated.pop();
        assert!(decode_png(&assembled_png(1, 1, 8, 2, 0, &[&truncated])).is_err());
        let mut adler = compressed.clone();
        *adler.last_mut().unwrap() ^= 1;
        assert!(decode_png(&assembled_png(1, 1, 8, 2, 0, &[&adler])).is_err());
        let mut appended = compressed.clone();
        appended.extend_from_slice(&zlib(&raw));
        assert!(decode_png(&assembled_png(1, 1, 8, 2, 0, &[&appended])).is_err());
        assert!(decode_png(&assembled_png(1, 1, 8, 2, 0, &[&compressed, &zlib(&raw)])).is_err());
        appended = compressed.clone();
        appended.push(0);
        assert!(decode_png(&assembled_png(1, 1, 8, 2, 0, &[&appended])).is_err());
        assert!(decode_png(&assembled_png(1, 1, 8, 2, 0, &[&compressed, &[0]])).is_err());
        let parts: Vec<&[u8]> = vec![
            &compressed[..1],
            &[],
            &compressed[1..2],
            &compressed[2..compressed.len() - 3],
            &[],
            &compressed[compressed.len() - 3..compressed.len() - 1],
            &compressed[compressed.len() - 1..],
            &[],
        ];
        assert_eq!(
            color_at(
                &decode_png(&assembled_png(1, 1, 8, 2, 0, &parts)).unwrap(),
                0,
                0
            ),
            [15, 40, 90, 255]
        );
    }

    #[test]
    fn drained_large_scanline_stream_preserves_exact_pixels_across_split_idat() {
        let row = [0, 15, 40, 90];
        let mut raw = Vec::with_capacity(40_000);
        for _ in 0..10_000 {
            raw.extend_from_slice(&row);
        }
        let compressed = zlib(&raw);
        let end = compressed.len();
        let parts = [
            &compressed[..1],
            &[][..],
            &compressed[1..end - 4],
            &[][..],
            &compressed[end - 4..],
            &[][..],
        ];
        let decoded = decode_png(&assembled_png(1, 10_000, 8, 2, 0, &parts)).unwrap();
        assert_eq!(color_at(&decoded, 0, 0), [15, 40, 90, 255]);
        assert_eq!(color_at(&decoded, 0, 9_999), [15, 40, 90, 255]);
    }

    #[test]
    fn compressed_bomb_is_rejected_without_materializing_its_sixty_four_megabytes() {
        let mut writer =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        writer.write_all(&[0, 5, 10, 15]).unwrap();
        let zeros = [0u8; 8192];
        for _ in 0..(64 * 1024 * 1024 / zeros.len()) {
            writer.write_all(&zeros).unwrap();
        }
        let compressed = writer.finish().unwrap();
        assert!(decode_png(&assembled_png(1, 1, 8, 2, 0, &[&compressed])).is_err());
    }

    #[test]
    fn png_checks_every_chunk_crc_and_requires_valid_structure() {
        let stream = zlib(&[0, 5, 10, 15]);
        let original = assembled_png(1, 1, 8, 2, 0, &[&stream]);
        let mut iccp = b"Profile\0\0".to_vec();
        iccp.extend_from_slice(&zlib(&[42; 128]));
        for (kind, data) in [
            (&b"tEXt"[..], &b"Comment\0hello"[..]),
            (&b"iCCP"[..], iccp.as_slice()),
            (&b"vpAg"[..], &b"unknown"[..]),
            (&b"tRNS"[..], &[0, 9, 0, 9, 0, 9][..]),
        ] {
            let mut candidate = original.clone();
            let name: &[u8; 4] = kind.try_into().unwrap();
            insert_before(&mut candidate, b"IDAT", name, data);
            assert_eq!(
                color_at(&decode_png(&candidate).unwrap(), 0, 0),
                [5, 10, 15, 255]
            );
            corrupt_chunk_crc(&mut candidate, name);
            assert!(decode_png(&candidate).is_err());
        }
        let mut header = original.clone();
        corrupt_chunk_crc(&mut header, b"IHDR");
        assert!(decode_png(&header).is_err());
        let mut end = original.clone();
        corrupt_chunk_crc(&mut end, b"IEND");
        assert!(decode_png(&end).is_err());
        let mut duplicate = original.clone();
        insert_before(&mut duplicate, b"IDAT", b"IHDR", &[0; 13]);
        assert!(decode_png(&duplicate).is_err());
        let mut scattered = original.clone();
        insert_before(&mut scattered, b"IEND", b"vpAg", b"padding");
        insert_before(&mut scattered, b"IEND", b"IDAT", &[]);
        assert!(decode_png(&scattered).is_err());
        for (depth, color, compression, filter, interlace, zero_width) in [
            (4, 2, 0, 0, 0, false),
            (16, 3, 0, 0, 0, false),
            (8, 2, 1, 0, 0, false),
            (8, 2, 0, 1, 0, false),
            (8, 2, 0, 0, 2, false),
            (8, 2, 0, 0, 0, true),
        ] {
            let mut candidate = original.clone();
            let ihdr = candidate
                .windows(4)
                .position(|part| part == b"IHDR")
                .unwrap();
            candidate[ihdr + 12] = depth;
            candidate[ihdr + 13] = color;
            candidate[ihdr + 14] = compression;
            candidate[ihdr + 15] = filter;
            candidate[ihdr + 16] = interlace;
            if zero_width {
                candidate[ihdr + 4..ihdr + 8].fill(0);
            }
            let start = ihdr - 4;
            let mut crc = Crc::new();
            crc.update(&candidate[ihdr..ihdr + 17]);
            candidate[start + 21..start + 25].copy_from_slice(&crc.sum().to_be_bytes());
            assert!(decode_png(&candidate).is_err());
        }
    }

    #[test]
    fn exact_encoded_and_pixel_budgets_accept_visible_images_only_within_bounds() {
        let original = png(
            1,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &[12, 30, 90],
        );
        let padding = vec![0x42; MAX_ENCODED - original.len() - 12];
        let mut exact = original.clone();
        insert_before(&mut exact, b"IDAT", b"vpAg", &padding);
        assert_eq!(exact.len(), MAX_ENCODED);
        assert_eq!(
            color_at(&decode_png(&exact).unwrap(), 0, 0),
            [12, 30, 90, 255]
        );
        let mut oversized = original.clone();
        let oversized_padding = vec![0x42; MAX_ENCODED + 1 - original.len() - 12];
        insert_before(&mut oversized, b"IDAT", b"vpAg", &oversized_padding);
        assert_eq!(oversized.len(), MAX_ENCODED + 1);
        assert!(decode_png(&oversized).is_err());
        let million = png(
            1000,
            1000,
            png::ColorType::Grayscale,
            png::BitDepth::Eight,
            &vec![8; 1_000_000],
        );
        assert_eq!(
            color_at(&decode_png(&million).unwrap(), 0, 0),
            [8, 8, 8, 255]
        );
        let million_plus_one = png(
            1_000_001,
            1,
            png::ColorType::Grayscale,
            png::BitDepth::Eight,
            &vec![8; 1_000_001],
        );
        assert!(decode_png(&million_plus_one).is_err());
    }

    #[test]
    fn subbyte_gray_indexed_transparency_and_real_adam7_passes_keep_pixel_colors() {
        let stream = zlib(&[0, 0b0100_0000]);
        let mut gray = assembled_png(2, 1, 1, 0, 0, &[&stream]);
        insert_before(&mut gray, b"IDAT", b"tRNS", &[0, 0]);
        let gray = decode_png(&gray).unwrap();
        assert_eq!(color_at(&gray, 0, 0), [0, 0, 0, 0]);
        assert_eq!(color_at(&gray, 1, 0), [255, 255, 255, 255]);

        let mut indexed = assembled_png(2, 1, 1, 3, 0, &[&stream]);
        insert_before(&mut indexed, b"IDAT", b"PLTE", &[220, 2, 1, 7, 91, 180]);
        insert_before(&mut indexed, b"IDAT", b"tRNS", &[0, 210]);
        let indexed = decode_png(&indexed).unwrap();
        assert_eq!(color_at(&indexed, 0, 0), [220, 2, 1, 0]);
        assert_eq!(color_at(&indexed, 1, 0), [7, 91, 180, 210]);

        for (width, height) in [(9u32, 5u32), (1, 9), (9, 1)] {
            let mut raw = Vec::new();
            for (x, y, dx, dy) in [
                (0, 0, 8, 8),
                (4, 0, 8, 8),
                (0, 4, 4, 8),
                (2, 0, 4, 4),
                (0, 2, 2, 4),
                (1, 0, 2, 2),
                (0, 1, 1, 2),
            ] {
                let rows = (y..height).step_by(dy as usize);
                for py in rows {
                    if x >= width {
                        continue;
                    }
                    raw.push(0);
                    for px in (x..width).step_by(dx as usize) {
                        raw.extend_from_slice(&[px as u8 + 10, py as u8 + 20, 44]);
                    }
                }
            }
            let compressed = zlib(&raw);
            let decoded =
                decode_png(&assembled_png(width, height, 8, 2, 1, &[&compressed])).unwrap();
            assert_eq!(decoded.width, width);
            assert_eq!(decoded.height, height);
            for py in 0..height {
                for px in 0..width {
                    assert_eq!(
                        color_at(&decoded, px as usize, py as usize),
                        [px as u8 + 10, py as u8 + 20, 44, 255]
                    );
                }
            }
        }
    }

    #[test]
    fn preserves_geometry_transparency_and_sparse_content_at_both_scales() {
        let wide = decode_png(&png(
            8,
            4,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &vec![80; 8 * 4 * 3],
        ))
        .unwrap();
        for (canvas, expected_height, top) in [(18, 9, 4), (36, 18, 9)] {
            let painted = paint_canvas(&wide, canvas);
            assert_eq!(
                fit(8, 4, canvas),
                Fit {
                    width: canvas,
                    height: expected_height,
                    left: 0,
                    top
                }
            );
            assert_eq!(painted[3], 0);
            let sample = ((top * canvas + canvas / 2) * 4) as usize;
            assert_eq!(&painted[sample..sample + 4], &[80, 80, 80, 255]);
        }
        let tall = fit(4, 8, 18);
        assert_eq!(
            tall,
            Fit {
                width: 9,
                height: 18,
                left: 4,
                top: 0
            }
        );
        let sprite = decode_png(&png(
            2,
            1,
            png::ColorType::Rgba,
            png::BitDepth::Eight,
            &[255, 0, 0, 0, 0, 220, 40, 255],
        ))
        .unwrap();
        let middle = paint_canvas(&sprite, 18);
        assert!(middle[((4 * 18 + 8) * 4 + 3) as usize] > 0);
        assert_eq!(middle[((4 * 18 + 8) * 4) as usize], 0);

        let mut pixels = vec![0; 1000 * 1000 * 4];
        pixels[(531 * 1000 + 527) * 4..(531 * 1000 + 527) * 4 + 4]
            .copy_from_slice(&[30, 80, 220, 255]);
        let sparse = decode_png(&png(
            1000,
            1000,
            png::ColorType::Rgba,
            png::BitDepth::Eight,
            &pixels,
        ))
        .unwrap();
        for canvas in [18, 36] {
            assert!(paint_canvas(&sparse, canvas)
                .chunks_exact(4)
                .any(|pixel| pixel[3] > 0));
        }
    }

    #[test]
    fn managed_asset_survives_source_removal_and_refuses_unsafe_paths() {
        let directory = isolated_directory();
        let source = directory.join("from-outside.png");
        let bytes = png(
            2,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &[3, 20, 70, 80, 50, 1],
        );
        fs::write(&source, &bytes).unwrap();
        let imported = read_source_bytes(&source).unwrap();
        let preference = MenuBarIconPreference {
            asset: name_for_bytes(&imported),
        };
        publish_in_directory(&directory, &preference.asset, &imported).unwrap();
        fs::remove_file(&source).unwrap();
        assert_eq!(
            load_bytes_in_directory(&preference, &directory).unwrap(),
            bytes
        );
        assert_eq!(
            decode_png(&load_bytes_in_directory(&preference, &directory).unwrap())
                .unwrap()
                .width,
            2
        );
        let managed = directory.join(ICON_DIRECTORY);
        assert_eq!(
            fs::metadata(&managed).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let asset = managed.join(&preference.asset);
        assert_eq!(
            fs::metadata(&asset).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(validate_asset_name("../icon-unsafe.png").is_err());
        assert!(validate_asset_name(&preference.asset.to_uppercase()).is_err());
        let missing = MenuBarIconPreference {
            asset: format!("icon-{}.png", "f".repeat(64)),
        };
        assert!(load_bytes_in_directory(&missing, &directory).is_err());
        let symlink_name = format!("icon-{}.png", "e".repeat(64));
        symlink(&asset, managed.join(&symlink_name)).unwrap();
        assert!(load_bytes_in_directory(
            &MenuBarIconPreference {
                asset: symlink_name
            },
            &directory
        )
        .is_err());
        fs::set_permissions(&asset, Permissions::from_mode(0o644)).unwrap();
        assert!(load_bytes_in_directory(&preference, &directory).is_err());
        fs::set_permissions(&asset, Permissions::from_mode(0o600)).unwrap();
        let sibling = directory.join("other");
        fs::create_dir(&sibling).unwrap();
        let replacement = directory.join("other-icons");
        symlink(&sibling, &replacement).unwrap();
        assert!(managed_directory(&replacement, true).is_err());
        let real = directory.join("real-icons");
        fs::rename(&managed, &real).unwrap();
        symlink(&real, &managed).unwrap();
        assert!(managed_directory(&directory, false).is_err());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn publish_failure_leaves_no_new_asset_and_cleanup_only_removes_known_previous() {
        let directory = isolated_directory();
        let first = png(
            1,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &[30, 40, 50],
        );
        let old = name_for_bytes(&first);
        publish_in_directory(&directory, &old, &first).unwrap();
        let bad = png(
            1,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &[255, 40, 50],
        );
        assert!(publish_in_directory(&directory, &old, &bad).is_err());
        let managed = directory.join(ICON_DIRECTORY);
        assert_eq!(fs::read(managed.join(&old)).unwrap(), first);
        let unknown = managed.join("unknown-user-file");
        fs::write(&unknown, b"do not sweep").unwrap();
        cleanup_previous_in_directory(&directory, &old);
        assert!(!managed.join(old).exists());
        assert_eq!(fs::read(unknown).unwrap(), b"do not sweep");
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn exclusive_publication_preserves_existing_bytes_and_creates_one_link_only() {
        let root = isolated_directory();
        let first = png(
            1,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &[20, 30, 40],
        );
        let name = name_for_bytes(&first);
        publish_in_directory(&root, &name, &first).unwrap();
        let directory = root.join(ICON_DIRECTORY);
        let final_path = directory.join(&name);
        assert_eq!(fs::metadata(&final_path).unwrap().nlink(), 1);
        assert_eq!(fs::read(&final_path).unwrap(), first);
        publish_in_directory(&root, &name, &first).unwrap();
        assert_eq!(fs::metadata(&final_path).unwrap().nlink(), 1);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_file(&final_path).unwrap();
        let incorrect = png(
            1,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &[40, 30, 20],
        );
        fs::write(&final_path, &incorrect).unwrap();
        fs::set_permissions(&final_path, Permissions::from_mode(0o600)).unwrap();
        assert!(publish_in_directory(&root, &name, &first).is_err());
        assert_eq!(fs::read(&final_path).unwrap(), incorrect);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_file(&final_path).unwrap();
        symlink(&root, &final_path).unwrap();
        assert!(publish_in_directory(&root, &name, &first).is_err());
        assert_eq!(fs::read_link(&final_path).unwrap(), root);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn old_two_link_alias_repairs_only_after_identical_reimport() {
        let root = isolated_directory();
        let bytes = png(
            1,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &[31, 51, 71],
        );
        let name = name_for_bytes(&bytes);
        let directory = root.join(ICON_DIRECTORY);
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, Permissions::from_mode(0o700)).unwrap();
        let final_path = directory.join(&name);
        fs::write(&final_path, &bytes).unwrap();
        fs::set_permissions(&final_path, Permissions::from_mode(0o600)).unwrap();
        let alias = directory.join(".icon-123-456-789");
        fs::hard_link(&final_path, &alias).unwrap();
        let unrelated = directory.join(".icon-123-456-790");
        fs::write(&unrelated, b"unrelated").unwrap();
        let pref = MenuBarIconPreference {
            asset: name.clone(),
        };
        assert!(load_bytes_in_directory(&pref, &root).is_err());
        assert_eq!(fs::metadata(&final_path).unwrap().nlink(), 2);
        publish_in_directory(&root, &name, &bytes).unwrap();
        assert_eq!(fs::read(&final_path).unwrap(), bytes);
        assert_eq!(fs::metadata(&final_path).unwrap().nlink(), 1);
        assert!(!alias.exists());
        assert_eq!(fs::read(&unrelated).unwrap(), b"unrelated");
        assert_eq!(load_bytes_in_directory(&pref, &root).unwrap(), bytes);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unsafe_old_links_and_names_remain_untouched_on_identical_retry() {
        let bytes = png(
            1,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &[11, 22, 33],
        );
        let name = name_for_bytes(&bytes);
        for scenario in [
            "external",
            "wrong-pattern",
            "different-inode",
            "symlink",
            "bad-mode",
            "bad-digest",
            "three-links",
        ] {
            let root = isolated_directory();
            let directory = root.join(ICON_DIRECTORY);
            fs::create_dir(&directory).unwrap();
            fs::set_permissions(&directory, Permissions::from_mode(0o700)).unwrap();
            let final_path = directory.join(&name);
            let contents = if scenario == "bad-digest" {
                b"wrong bytes".as_slice()
            } else {
                &bytes
            };
            fs::write(&final_path, contents).unwrap();
            fs::set_permissions(&final_path, Permissions::from_mode(0o600)).unwrap();
            let correct_alias = directory.join(".icon-123-456-789");
            let external = root.join("outside");
            match scenario {
                "external" => fs::hard_link(&final_path, &external).unwrap(),
                "wrong-pattern" => {
                    fs::hard_link(&final_path, directory.join(".icon-abc-456-789")).unwrap()
                }
                "different-inode" => {
                    fs::hard_link(&final_path, &external).unwrap();
                    fs::write(&correct_alias, &bytes).unwrap();
                }
                "symlink" => {
                    fs::hard_link(&final_path, &external).unwrap();
                    symlink(&final_path, &correct_alias).unwrap();
                }
                "three-links" => {
                    fs::hard_link(&final_path, &correct_alias).unwrap();
                    fs::hard_link(&final_path, directory.join(".icon-123-456-790")).unwrap();
                }
                _ => fs::hard_link(&final_path, &correct_alias).unwrap(),
            }
            if scenario == "bad-mode" {
                fs::set_permissions(&final_path, Permissions::from_mode(0o644)).unwrap();
            }
            let links_before = fs::metadata(&final_path).unwrap().nlink();
            let before: std::collections::BTreeSet<_> = fs::read_dir(&directory)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            assert!(
                publish_in_directory(&root, &name, &bytes).is_err(),
                "{scenario}"
            );
            assert_eq!(fs::read(&final_path).unwrap(), contents, "{scenario}");
            assert_eq!(
                fs::metadata(&final_path).unwrap().nlink(),
                links_before,
                "{scenario}"
            );
            assert_eq!(
                correct_alias.exists(),
                !matches!(scenario, "external" | "wrong-pattern"),
                "{scenario}"
            );
            let after: std::collections::BTreeSet<_> = fs::read_dir(&directory)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            assert_eq!(after, before, "{scenario}");
            let _ = fs::remove_dir_all(root);
        }
    }
}
