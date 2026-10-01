use flate2::read::DeflateDecoder;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::Path;

pub(crate) const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_EXPANDED_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_ARCHIVE_ENTRIES: usize = 129;

#[derive(Debug)]
pub(crate) struct ArchiveSnapshot {
    pub entries: BTreeMap<String, Vec<u8>>,
}

#[derive(Clone, Copy, Debug)]
struct CentralEntry<'a> {
    name: &'a [u8],
    method: u16,
    flags: u16,
    crc32: u32,
    compressed_size: u32,
    uncompressed_size: u32,
    local_offset: u32,
    required_version: u16,
}

fn read_u16(bytes: &[u8], offset: usize, label: &str) -> Result<u16, String> {
    let end = offset
        .checked_add(2)
        .ok_or_else(|| format!("{label} offset overflow"))?;
    let value = bytes
        .get(offset..end)
        .ok_or_else(|| format!("{label} is truncated"))?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize, label: &str) -> Result<u32, String> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| format!("{label} offset overflow"))?;
    let value = bytes
        .get(offset..end)
        .ok_or_else(|| format!("{label} is truncated"))?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn validate_extra(bytes: &[u8]) -> Result<(), String> {
    let mut offset = 0usize;
    while offset < bytes.len() {
        let kind = read_u16(bytes, offset, "archive extra-field type")?;
        let length = usize::from(read_u16(bytes, offset + 2, "archive extra-field length")?);
        let end = offset + 4 + length;
        if end > bytes.len() {
            return Err("archive extra field is truncated".to_string());
        }
        // Only inert timestamps and Unix numeric ownership are supported.
        // ZIP64, link records, alternate paths, and unknown extensions are not.
        if !matches!(kind, 0x5455 | 0x7875) {
            return Err("archive contains an unsupported extra field".to_string());
        }
        offset = end;
    }
    Ok(())
}

fn safe_archive_name(bytes: &[u8]) -> Result<String, String> {
    if bytes.is_empty() || bytes.len() > 255 || !bytes.is_ascii() {
        return Err("archive entry path must be a non-empty ASCII name".to_string());
    }
    if bytes.iter().any(|byte| *byte < 0x20 || *byte == 0x7f) {
        return Err("archive entry path contains a control character".to_string());
    }
    let name =
        std::str::from_utf8(bytes).map_err(|_| "archive entry path is not UTF-8".to_string())?;
    if name == "."
        || name == ".."
        || name.starts_with('/')
        || name.ends_with('/')
        || name.contains('/')
        || name.contains('\\')
        || name.starts_with('.')
        || name.ends_with('.')
    {
        return Err("archive entry path must be a root-only normal filename".to_string());
    }
    Ok(name.to_string())
}

fn case_key(name: &str) -> String {
    name.bytes()
        .map(|byte| byte.to_ascii_lowercase() as char)
        .collect()
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320u32 & mask);
        }
    }
    !crc
}

fn source_file(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("archive is unavailable: {error}"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("archive must be a regular file".to_string());
    }
    if metadata.len() > MAX_ARCHIVE_BYTES as u64 {
        return Err("archive exceeds the 64 MiB limit".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::fd::FromRawFd;
        use std::os::unix::ffi::OsStrExt;
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
            .map_err(|_| "archive path contains NUL".to_string())?;
        let raw = unsafe {
            libc::open(
                c_path.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            )
        };
        if raw < 0 {
            return Err(format!(
                "archive cannot be opened: {}",
                io::Error::last_os_error()
            ));
        }
        let mut file = unsafe { File::from_raw_fd(raw) };
        let metadata = file
            .metadata()
            .map_err(|error| format!("archive metadata cannot be read: {error}"))?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err("archive must be a single-link regular file".to_string());
        }
        if metadata.len() > MAX_ARCHIVE_BYTES as u64 {
            return Err("archive exceeds the 64 MiB limit".to_string());
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        Read::by_ref(&mut file)
            .take(MAX_ARCHIVE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("archive cannot be read: {error}"))?;
        if bytes.len() > MAX_ARCHIVE_BYTES {
            return Err("archive exceeds the 64 MiB limit".to_string());
        }
        Ok(bytes)
    }
    #[cfg(not(unix))]
    {
        let mut file =
            File::open(path).map_err(|error| format!("archive cannot be opened: {error}"))?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(MAX_ARCHIVE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("archive cannot be read: {error}"))?;
        if bytes.len() > MAX_ARCHIVE_BYTES {
            return Err("archive exceeds the 64 MiB limit".to_string());
        }
        Ok(bytes)
    }
}

pub(crate) fn load(path: &Path) -> Result<ArchiveSnapshot, String> {
    let bytes = source_file(path)?;
    parse(&bytes)
}

pub(crate) fn parse(bytes: &[u8]) -> Result<ArchiveSnapshot, String> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err("archive exceeds the 64 MiB limit".to_string());
    }
    if bytes.len() < 22 {
        return Err("archive end record is missing".to_string());
    }
    let search_start = bytes.len().saturating_sub(65_557);
    let eocd = (search_start..=bytes.len() - 22)
        .rev()
        .find(|offset| bytes[*offset..].starts_with(b"PK\x05\x06"))
        .ok_or_else(|| "archive end record is missing".to_string())?;
    let disk = read_u16(bytes, eocd + 4, "archive disk")?;
    let central_disk = read_u16(bytes, eocd + 6, "archive central directory disk")?;
    let disk_count = read_u16(bytes, eocd + 8, "archive disk entry count")?;
    let total_count = read_u16(bytes, eocd + 10, "archive entry count")?;
    let central_size = usize::try_from(read_u32(bytes, eocd + 12, "archive central size")?)
        .map_err(|_| "archive central directory size overflows usize".to_string())?;
    let central_offset = usize::try_from(read_u32(bytes, eocd + 16, "archive central offset")?)
        .map_err(|_| "archive central directory offset overflows usize".to_string())?;
    let comment_len = usize::from(read_u16(bytes, eocd + 20, "archive comment length")?);
    if disk != 0 || central_disk != 0 || disk_count != total_count {
        return Err("multi-disk archives are not supported".to_string());
    }
    if total_count == u16::MAX
        || central_size == u32::MAX as usize
        || central_offset == u32::MAX as usize
        || comment_len > bytes.len().saturating_sub(eocd + 22)
        || eocd + 22 + comment_len != bytes.len()
    {
        return Err("ZIP64 or malformed archive end record is not supported".to_string());
    }
    let count = usize::from(total_count);
    if count == 0 || count > MAX_ARCHIVE_ENTRIES {
        return Err("archive must contain 1..129 entries including manifest".to_string());
    }
    let central_end = central_offset
        .checked_add(central_size)
        .ok_or_else(|| "archive central directory offset overflows usize".to_string())?;
    if central_offset < 22 || central_end != eocd {
        return Err("archive central directory is outside the file".to_string());
    }

    let mut entries = Vec::with_capacity(count);
    let mut offset = central_offset;
    let mut names = BTreeSet::new();
    let mut expanded_total = 0usize;
    for _ in 0..count {
        if bytes
            .get(offset..offset + 46)
            .map(|value| value.starts_with(b"PK\x01\x02"))
            != Some(true)
        {
            return Err("archive central directory entry is malformed".to_string());
        }
        let required_version = read_u16(bytes, offset + 6, "archive required version")?;
        let flags = read_u16(bytes, offset + 8, "archive entry flags")?;
        let method = read_u16(bytes, offset + 10, "archive entry method")?;
        let crc = read_u32(bytes, offset + 16, "archive entry CRC")?;
        let compressed_size = read_u32(bytes, offset + 20, "archive compressed size")?;
        let uncompressed_size = read_u32(bytes, offset + 24, "archive uncompressed size")?;
        let name_len = usize::from(read_u16(bytes, offset + 28, "archive entry name length")?);
        let extra_len = usize::from(read_u16(bytes, offset + 30, "archive entry extra length")?);
        let comment_len = usize::from(read_u16(
            bytes,
            offset + 32,
            "archive entry comment length",
        )?);
        if read_u16(bytes, offset + 34, "archive entry disk")? != 0
            || !matches!(required_version, 10 | 20)
        {
            return Err(
                "archive entry requires ZIP64, another disk, or an unsupported version".to_string(),
            );
        }
        let external_attributes = read_u32(bytes, offset + 38, "archive entry attributes")?;
        let local_offset = read_u32(bytes, offset + 42, "archive local offset")?;
        let record_len = 46usize
            .checked_add(name_len)
            .and_then(|value| value.checked_add(extra_len))
            .and_then(|value| value.checked_add(comment_len))
            .ok_or_else(|| "archive central entry length overflows usize".to_string())?;
        let record_end = offset
            .checked_add(record_len)
            .ok_or_else(|| "archive central entry offset overflows usize".to_string())?;
        if record_end > central_end {
            return Err("archive central directory entry is truncated".to_string());
        }
        let name_bytes = &bytes[offset + 46..offset + 46 + name_len];
        validate_extra(&bytes[offset + 46 + name_len..offset + 46 + name_len + extra_len])?;
        let name = safe_archive_name(name_bytes)?;
        if !names.insert(case_key(&name)) {
            return Err("archive contains duplicate or case-colliding entry names".to_string());
        }
        if flags != 0 || !matches!(method, 0 | 8) {
            return Err(
                "archive entry uses encryption, descriptors, or unsupported compression"
                    .to_string(),
            );
        }
        // DOS directory bit and Unix symlink/non-regular types are never valid
        // payloads.  The lower 16 bits are the DOS attributes; Unix mode is in
        // the high 16 bits when the creator is Unix.
        if external_attributes & 0x10 != 0 {
            return Err("archive directories are not payloads".to_string());
        }
        let unix_mode = (external_attributes >> 16) & 0xffff;
        let unix_type = unix_mode & 0xf000;
        if unix_type != 0 && unix_type != 0x8000 {
            return Err("archive symlink or special entry is not allowed".to_string());
        }
        if unix_mode & 0o111 != 0 {
            return Err("archive payload must not be executable".to_string());
        }
        if uncompressed_size as u64 > super::MAX_FILE_BYTES as u64
            || (method == 0 && compressed_size != uncompressed_size)
        {
            return Err(
                "archive entry exceeds its file limit or has inconsistent stored sizes".to_string(),
            );
        }
        let next_total = expanded_total
            .checked_add(usize::try_from(uncompressed_size).unwrap_or(usize::MAX))
            .ok_or_else(|| "archive expanded size overflows usize".to_string())?;
        if next_total > MAX_EXPANDED_BYTES {
            return Err("archive expanded data exceeds the 64 MiB limit".to_string());
        }
        expanded_total = next_total;
        entries.push(CentralEntry {
            name: name_bytes,
            method,
            flags,
            crc32: crc,
            compressed_size,
            uncompressed_size,
            local_offset,
            required_version,
        });
        offset = record_end;
    }
    if offset != central_end {
        return Err("archive central directory contains trailing bytes".to_string());
    }

    let mut output = BTreeMap::new();
    let mut ranges = Vec::with_capacity(entries.len());
    for entry in entries {
        let local = usize::try_from(entry.local_offset)
            .map_err(|_| "archive local offset overflows usize".to_string())?;
        if bytes
            .get(local..local + 30)
            .map(|value| value.starts_with(b"PK\x03\x04"))
            != Some(true)
        {
            return Err("archive local file header is malformed".to_string());
        }
        let local_flags = read_u16(bytes, local + 6, "archive local flags")?;
        let local_method = read_u16(bytes, local + 8, "archive local method")?;
        let local_name_len = usize::from(read_u16(bytes, local + 26, "archive local name length")?);
        let local_extra_len =
            usize::from(read_u16(bytes, local + 28, "archive local extra length")?);
        if local_flags != entry.flags
            || local_method != entry.method
            || read_u16(bytes, local + 4, "archive local version")? != entry.required_version
            || read_u32(bytes, local + 14, "archive local CRC")? != entry.crc32
            || read_u32(bytes, local + 18, "archive local compressed size")?
                != entry.compressed_size
            || read_u32(bytes, local + 22, "archive local expanded size")?
                != entry.uncompressed_size
        {
            return Err("archive local and central headers disagree".to_string());
        }
        let local_name_end = local
            .checked_add(30)
            .and_then(|value| value.checked_add(local_name_len))
            .ok_or_else(|| "archive local name offset overflows usize".to_string())?;
        let data_start = local_name_end
            .checked_add(local_extra_len)
            .ok_or_else(|| "archive local data offset overflows usize".to_string())?;
        let compressed_size = usize::try_from(entry.compressed_size)
            .map_err(|_| "archive compressed size overflows usize".to_string())?;
        let data_end = data_start
            .checked_add(compressed_size)
            .ok_or_else(|| "archive compressed data offset overflows usize".to_string())?;
        if data_end > central_offset
            || local_name_end > bytes.len()
            || local_extra_len > bytes.len()
        {
            return Err("archive compressed data is outside the file".to_string());
        }
        if bytes.get(local + 30..local_name_end) != Some(entry.name) {
            return Err("archive local and central names disagree".to_string());
        }
        validate_extra(&bytes[local_name_end..data_start])?;
        ranges.push((local, data_end));
        let expected_size = usize::try_from(entry.uncompressed_size)
            .map_err(|_| "archive expanded size overflows usize".to_string())?;
        let compressed = &bytes[data_start..data_end];
        let mut decoded = Vec::with_capacity(expected_size);
        match entry.method {
            0 => decoded.extend_from_slice(compressed),
            8 => {
                let mut decoder = DeflateDecoder::new(compressed);
                Read::by_ref(&mut decoder)
                    .take(expected_size as u64 + 1)
                    .read_to_end(&mut decoded)
                    .map_err(|error| format!("archive deflate stream is invalid: {error}"))?;
                if decoder.total_in() != compressed.len() as u64 {
                    return Err("archive deflate stream contains trailing data".to_string());
                }
            }
            _ => unreachable!(),
        }
        if decoded.len() != expected_size || decoded.len() > MAX_EXPANDED_BYTES {
            return Err(
                "archive decompressed size does not match its central directory".to_string(),
            );
        }
        if crc32(&decoded) != entry.crc32 {
            return Err("archive CRC does not match its payload".to_string());
        }
        let name = std::str::from_utf8(entry.name)
            .map_err(|_| "archive entry path is not UTF-8".to_string())?
            .to_string();
        output.insert(name, decoded);
    }
    ranges.sort_unstable();
    let mut local_end = 0;
    for (start, end) in ranges {
        if start != local_end {
            return Err("archive local entries overlap or contain unlisted bytes".to_string());
        }
        local_end = end;
    }
    if local_end != central_offset {
        return Err("archive contains unlisted bytes before its central directory".to_string());
    }
    Ok(ArchiveSnapshot { entries: output })
}

fn write_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn write_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

/// Apply the archive entry-name rules: every name must be a safe root-only
/// ASCII filename, and no two names may collide case-insensitively.
pub(crate) fn validate_entry_names<'a>(
    names: impl IntoIterator<Item = &'a str>,
) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for name in names {
        let _ = safe_archive_name(name.as_bytes())?;
        if !seen.insert(case_key(name)) {
            return Err("archive contains duplicate or case-colliding entry names".to_string());
        }
    }
    Ok(())
}

/// Write a deterministic, uncompressed ZIP archive.  The caller has already
/// validated each path and payload; this function still performs the same
/// path checks because it is an externally reachable export boundary.
pub(crate) fn write(path: &Path, entries: &[(String, &[u8])]) -> Result<(), String> {
    if path.extension().and_then(|value| value.to_str()) != Some("herdrchar") {
        return Err("archive export must use the .herdrchar extension".to_string());
    }
    if entries.is_empty()
        || entries.len() > MAX_ARCHIVE_ENTRIES
        || entries
            .iter()
            .filter(|(name, _)| name == "manifest.json")
            .count()
            != 1
    {
        return Err(
            "archive must contain one manifest and at most 128 payload entries".to_string(),
        );
    }
    validate_entry_names(entries.iter().map(|(name, _)| name.as_str()))?;
    let mut expanded = 0usize;
    for (_, bytes) in entries {
        expanded = expanded
            .checked_add(bytes.len())
            .ok_or_else(|| "archive expanded size overflows usize".to_string())?;
        if expanded > MAX_EXPANDED_BYTES {
            return Err("archive expanded data exceeds the 64 MiB limit".to_string());
        }
    }

    let mut bytes = Vec::new();
    let mut central = Vec::new();
    for (name, payload) in entries {
        let name_bytes = name.as_bytes();
        let checksum = crc32(payload);
        let local_offset = u32::try_from(bytes.len())
            .map_err(|_| "archive offset exceeds ZIP32 limits".to_string())?;
        bytes.extend_from_slice(b"PK\x03\x04");
        write_u16(&mut bytes, 20);
        write_u16(&mut bytes, 0);
        write_u16(&mut bytes, 0);
        write_u16(&mut bytes, 0);
        write_u16(&mut bytes, 0);
        write_u32(&mut bytes, checksum);
        write_u32(
            &mut bytes,
            u32::try_from(payload.len())
                .map_err(|_| "archive payload exceeds ZIP32 limits".to_string())?,
        );
        write_u32(
            &mut bytes,
            u32::try_from(payload.len())
                .map_err(|_| "archive payload exceeds ZIP32 limits".to_string())?,
        );
        write_u16(
            &mut bytes,
            u16::try_from(name_bytes.len())
                .map_err(|_| "archive filename is too long".to_string())?,
        );
        write_u16(&mut bytes, 0);
        bytes.extend_from_slice(name_bytes);
        bytes.extend_from_slice(payload);

        central.extend_from_slice(b"PK\x01\x02");
        write_u16(&mut central, 20);
        write_u16(&mut central, 20);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u32(&mut central, checksum);
        write_u32(
            &mut central,
            u32::try_from(payload.len())
                .map_err(|_| "archive payload exceeds ZIP32 limits".to_string())?,
        );
        write_u32(
            &mut central,
            u32::try_from(payload.len())
                .map_err(|_| "archive payload exceeds ZIP32 limits".to_string())?,
        );
        write_u16(
            &mut central,
            u16::try_from(name_bytes.len())
                .map_err(|_| "archive filename is too long".to_string())?,
        );
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u32(&mut central, 0);
        write_u32(&mut central, local_offset);
        central.extend_from_slice(name_bytes);
    }
    let central_offset =
        u32::try_from(bytes.len()).map_err(|_| "archive exceeds ZIP32 limits".to_string())?;
    let central_size =
        u32::try_from(central.len()).map_err(|_| "archive exceeds ZIP32 limits".to_string())?;
    bytes.extend_from_slice(&central);
    bytes.extend_from_slice(b"PK\x05\x06");
    write_u16(&mut bytes, 0);
    write_u16(&mut bytes, 0);
    let count =
        u16::try_from(entries.len()).map_err(|_| "archive has too many entries".to_string())?;
    write_u16(&mut bytes, count);
    write_u16(&mut bytes, count);
    write_u32(&mut bytes, central_size);
    write_u32(&mut bytes, central_offset);
    write_u16(&mut bytes, 0);
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err("archive exceeds the 64 MiB limit".to_string());
    }

    if path.exists() {
        return Err("archive export destination already exists".to_string());
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file: File = options
        .open(path)
        .map_err(|error| format!("cannot create archive export: {error}"))?;
    file.write_all(&bytes)
        .map_err(|error| format!("cannot write archive export: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("cannot sync archive export: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_zip_payload_is_not_accepted_as_data() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "herdr-executable-{}-{nonce}.herdrchar",
            std::process::id()
        ));
        write(&path, &[("manifest.json".to_string(), b"{}".as_slice())]).unwrap();
        let regular = load(&path);
        let mut bytes = std::fs::read(&path).unwrap();
        let central = bytes
            .windows(4)
            .position(|signature| signature == b"PK\x01\x02")
            .unwrap();
        bytes[central + 38..central + 42].copy_from_slice(&(0o100755_u32 << 16).to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        let executable = load(&path);
        std::fs::remove_file(&path).unwrap();
        assert!(regular.is_ok());
        assert!(executable.is_err());
    }
}
