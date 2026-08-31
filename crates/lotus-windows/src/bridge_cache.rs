use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::{env, fs};

use atomic_write_file::AtomicWriteFile;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy)]
pub(crate) enum BridgeBinary {
    Explorer,
    ShellHost,
}

impl BridgeBinary {
    fn file_name(self) -> &'static str {
        match self {
            Self::Explorer => "lotus_explorer_bridge.dll",
            Self::ShellHost => "lotus_shell_bridge.dll",
        }
    }

    fn cache_prefix(self) -> &'static str {
        match self {
            Self::Explorer => "lotus_explorer_bridge-",
            Self::ShellHost => "lotus_shell_bridge-",
        }
    }
}

pub(crate) fn cached_bridge_path(bridge: BridgeBinary) -> Option<PathBuf> {
    let source = env::current_exe().ok()?.parent()?.join(bridge.file_name());
    let bytes = fs::read(source).ok()?;
    let destination = cache_directory()?.join(format!(
        "{}{hash}.dll",
        bridge.cache_prefix(),
        hash = content_hash(&bytes)
    ));

    ensure_cached_copy(&destination, &bytes)?;
    cleanup_old_bridge_copies(destination.parent()?, &destination, bridge.cache_prefix());
    Some(destination)
}

fn cache_directory() -> Option<PathBuf> {
    let directory = PathBuf::from(env::var_os("LOCALAPPDATA")?)
        .join("Lotus")
        .join("bridge-cache");
    fs::create_dir_all(&directory).ok()?;
    Some(directory)
}

fn content_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hash = String::with_capacity(64);
    for byte in digest {
        let _ = write!(hash, "{byte:02x}");
    }
    hash
}

fn ensure_cached_copy(destination: &Path, bytes: &[u8]) -> Option<()> {
    if fs::read(destination).ok().as_deref() == Some(bytes) {
        return Some(());
    }

    let mut output = AtomicWriteFile::open(destination).ok()?;
    output.write_all(bytes).ok()?;
    output.commit().ok()?;
    (fs::read(destination).ok()? == bytes).then_some(())
}

fn cleanup_old_bridge_copies(directory: &Path, current: &Path, prefix: &str) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path != current && has_bridge_name(&path, prefix) {
            let _ = fs::remove_file(path);
        }
    }
}

fn has_bridge_name(path: &Path, prefix: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.starts_with(prefix)
                && Path::new(name)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"))
        })
}
