use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::mem::offset_of;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

/// Keep an endpoint's identity stable while its listener (or symlink target)
/// is absent; a lexical fallback alone would change /private/tmp back to /tmp.
pub(crate) fn canonical_endpoint(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() || path.as_os_str().as_bytes().contains(&0) {
        return Err("Herdr endpoint must be an absolute path without NUL".to_owned());
    }
    if let Ok(canonical) = fs::canonicalize(path) {
        return Ok(canonical);
    }
    let mut pending: VecDeque<OsString> = owned_components(path).collect();
    let mut resolved = PathBuf::from("/");
    let mut followed = 0;
    while let Some(component) = pending.pop_front() {
        if component == ".." {
            resolved.pop();
            continue;
        }
        resolved.push(component);
        match fs::symlink_metadata(&resolved) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                followed += 1;
                if followed > 40 {
                    return Err("Herdr endpoint contains too many symbolic links".to_owned());
                }
                let target = fs::read_link(&resolved)
                    .map_err(|error| format!("cannot resolve Herdr endpoint: {error}"))?;
                resolved.pop();
                if target.is_absolute() {
                    resolved = PathBuf::from("/");
                }
                for component in owned_components(&target).rev() {
                    pending.push_front(component);
                }
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("cannot resolve Herdr endpoint: {error}")),
        }
    }
    Ok(resolved)
}

fn owned_components(path: &Path) -> impl DoubleEndedIterator<Item = OsString> + '_ {
    path.components().filter_map(|component| match component {
        Component::Normal(value) => Some(value.to_owned()),
        Component::ParentDir => Some(OsString::from("..")),
        Component::RootDir | Component::CurDir | Component::Prefix(_) => None,
    })
}

pub(crate) fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "socket deadline elapsed"))
}

/// Read one nonblocking chunk without changing the caller's socket mode.
pub(crate) fn read_with_deadline(
    stream: &mut UnixStream,
    buffer: &mut [u8],
    deadline: Instant,
) -> io::Result<usize> {
    if buffer.is_empty() {
        return Ok(0);
    }
    let fd = stream.as_raw_fd();
    // SAFETY: fd remains owned by stream for this function's duration.
    let original_flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if original_flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let originally_nonblocking = original_flags & libc::O_NONBLOCK != 0;
    if !originally_nonblocking {
        // SAFETY: fd remains owned by stream and original_flags came from F_GETFL.
        if unsafe { libc::fcntl(fd, libc::F_SETFL, original_flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }

    let result = loop {
        let timeout = match remaining(deadline) {
            Ok(timeout) => timeout,
            Err(error) => break Err(error),
        };
        let milliseconds = timeout.as_millis().saturating_add(1).min(i32::MAX as u128);
        let mut descriptor = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: descriptor points to one initialized pollfd for a live fd.
        let polled = unsafe { libc::poll(&mut descriptor, 1, milliseconds as i32) };
        if polled < 0 {
            let error = io::Error::last_os_error();
            if matches!(
                error.kind(),
                io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
            ) {
                continue;
            }
            break Err(error);
        }
        if polled == 0 {
            break Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "socket read timed out",
            ));
        }
        match stream.read(buffer) {
            Ok(read) => break Ok(read),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) =>
            {
                continue;
            }
            Err(error) => break Err(error),
        }
    };

    if !originally_nonblocking {
        // SAFETY: fd remains owned by stream and original_flags came from F_GETFL.
        if unsafe { libc::fcntl(fd, libc::F_SETFL, original_flags) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    result
}

/// Connect without allowing a stalled/full Unix listener to block its caller.
/// Callers must renew write I/O timeouts from the same deadline before subsequent I/O.
pub(crate) fn connect(path: &Path, deadline: Instant) -> io::Result<UnixStream> {
    remaining(deadline)?;
    let bytes = path.as_os_str().as_bytes();
    // SAFETY: sockaddr_un consists only of integer fields and a byte array.
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.is_empty() || bytes.contains(&0) || bytes.len() >= address.sun_path.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Unix socket path is empty, contains NUL, or exceeds the platform limit",
        ));
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (target, source) in address.sun_path.iter_mut().zip(bytes) {
        *target = *source as libc::c_char;
    }
    let length = offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1;
    #[cfg(target_os = "macos")]
    {
        address.sun_len = length as u8;
    }
    // SAFETY: socket has no pointer arguments; ownership transfers immediately
    // into UnixStream so every subsequent error closes the descriptor.
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd is a newly created, uniquely owned stream socket.
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    // SAFETY: the descriptor remains valid for the lifetime of stream.
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    stream.set_nonblocking(true)?;
    // SAFETY: address is initialized, its path is NUL-terminated, and length
    // covers only initialized bytes within sockaddr_un.
    let connected = unsafe {
        libc::connect(
            stream.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            length as libc::socklen_t,
        )
    };
    if connected < 0 {
        let error = io::Error::last_os_error();
        if !matches!(
            error.raw_os_error(),
            Some(libc::EINPROGRESS | libc::EALREADY | libc::EINTR)
        ) {
            // In particular, EAGAIN on a full Unix accept queue is a bounded
            // retryable failure, not evidence that a connection has started.
            return Err(error);
        }
        loop {
            let timeout = remaining(deadline)?;
            let milliseconds = timeout.as_millis().saturating_add(1).min(i32::MAX as u128);
            let mut descriptor = libc::pollfd {
                fd: stream.as_raw_fd(),
                events: libc::POLLOUT,
                revents: 0,
            };
            // SAFETY: descriptor points to one initialized pollfd for a live fd.
            let result = unsafe { libc::poll(&mut descriptor, 1, milliseconds as i32) };
            if result < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if result == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "socket connect timed out",
                ));
            }
            remaining(deadline)?;
            if let Some(error) = stream.take_error()? {
                return Err(error);
            }
            break;
        }
    }
    stream.set_nonblocking(false)?;
    let timeout = remaining(deadline)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expired_connect_deadline_is_not_a_filesystem_probe() {
        let deadline = Instant::now() - Duration::from_millis(1);
        let error = connect(Path::new("/not/a/socket"), deadline).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn overlong_socket_path_is_rejected_before_connect() {
        let path = "/".repeat(std::mem::size_of::<libc::sockaddr_un>());
        let error = connect(Path::new(&path), Instant::now() + Duration::from_secs(1)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn endpoint_identity_survives_missing_listener_and_broken_alias() {
        use std::os::unix::fs::symlink;
        use std::os::unix::net::UnixListener;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = PathBuf::from(format!("/tmp/hdp-path-{}-{nonce}", std::process::id()));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("actual")).unwrap();
        symlink("actual", root.join("alias")).unwrap();
        symlink("actual/herdr.sock", root.join("socket-alias")).unwrap();
        let path = root.join("actual/herdr.sock");
        let expected = canonical_endpoint(&path).unwrap();
        assert_eq!(
            canonical_endpoint(&root.join("alias/herdr.sock")).unwrap(),
            expected
        );
        assert_eq!(
            canonical_endpoint(&root.join("socket-alias")).unwrap(),
            expected
        );
        let listener = UnixListener::bind(&path).unwrap();
        assert_eq!(
            canonical_endpoint(&root.join("socket-alias")).unwrap(),
            expected
        );
        drop(listener);
        fs::remove_file(&path).unwrap();
        assert_eq!(
            canonical_endpoint(&root.join("socket-alias")).unwrap(),
            expected
        );
        fs::remove_dir_all(root).unwrap();
    }
}
