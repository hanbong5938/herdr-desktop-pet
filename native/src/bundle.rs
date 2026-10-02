//! Locating the running executable and its enclosing `.app` bundle.
//!
//! The Homebrew cask links the CLI onto `PATH`, and on macOS
//! `std::env::current_exe()` returns that link path rather than the binary
//! inside the bundle, so symlinks must be resolved before inspecting the
//! surrounding directory layout.

use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

/// Canonical path of the running executable, with symlinks resolved.
pub fn executable() -> io::Result<PathBuf> {
    fs::canonicalize(env::current_exe()?)
}

/// `Contents` directory of the enclosing `.app` bundle, if any.
pub fn contents_dir() -> Option<PathBuf> {
    contents_dir_for(&env::current_exe().ok()?)
}

fn contents_dir_for(executable: &Path) -> Option<PathBuf> {
    let executable = fs::canonicalize(executable).ok()?;
    let contents = executable.parent()?.parent()?;
    (contents.file_name()? == "Contents").then(|| contents.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symlinked_executable_resolves_to_bundle_contents() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!(
            "herdr-bundle-symlink-{}-{nonce}",
            std::process::id()
        ));
        let macos = root.join("Pet.app/Contents/MacOS");
        let bin = root.join("bin");
        fs::create_dir_all(&macos).unwrap();
        fs::create_dir_all(&bin).unwrap();
        let target = macos.join("herdr-desktop-pet");
        fs::write(&target, b"").unwrap();
        let link = bin.join("herdr-desktop-pet");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let other = bin.join("other-regular-file");
        fs::write(&other, b"").unwrap();

        assert_eq!(
            contents_dir_for(&link),
            Some(fs::canonicalize(root.join("Pet.app/Contents")).unwrap())
        );
        assert_eq!(contents_dir_for(&other), None);

        fs::remove_dir_all(&root).unwrap();
    }
}
