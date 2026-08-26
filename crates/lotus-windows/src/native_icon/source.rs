use std::path::{Path, PathBuf};

use super::{MAX_ICON_SIZE, NativeIconError};
use crate::launch::{
    resolve_executable, resolve_internet_shortcut_icon, resolve_shortcut_icon,
};

pub(super) fn icon_extraction_source(source: &Path) -> Option<(PathBuf, i32)> {
    if has_extension(source, "lnk") {
        let target = resolve_executable(&source.to_string_lossy());
        let shortcut_icon = resolve_shortcut_icon(source);
        return select_shortcut_extraction(target, shortcut_icon);
    }
    if has_extension(source, "url") {
        return resolve_internet_shortcut_icon(source);
    }
    let source_text = source.to_string_lossy();
    let resolved = resolve_executable(&source_text);
    select_extraction_path(source.to_owned(), resolved).map(|path| (path, 0))
}

pub(super) fn is_shell_namespace_path(source: &Path) -> bool {
    source
        .to_string_lossy()
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("shell:"))
}

fn select_shortcut_extraction(
    target: Option<PathBuf>,
    shortcut_icon: Option<(PathBuf, i32)>,
) -> Option<(PathBuf, i32)> {
    shortcut_icon.or_else(|| target.map(|path| (path, 0)))
}

fn select_extraction_path(source: PathBuf, resolved: Option<PathBuf>) -> Option<PathBuf> {
    resolved.or_else(|| (!has_extension(&source, "lnk")).then_some(source))
}

fn has_extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
}

pub(super) fn normalize_path(path: &Path) -> Result<String, NativeIconError> {
    let path = path.to_string_lossy();
    let path = path.trim();
    if path.is_empty() || path.contains('\0') {
        return Err(NativeIconError::InvalidPath);
    }
    Ok(path.replace('/', "\\").to_lowercase())
}

pub(super) fn sanitized_path(path: &Path) -> Result<PathBuf, NativeIconError> {
    let path = path.to_string_lossy();
    let path = path.trim();
    if path.is_empty() || path.contains('\0') {
        return Err(NativeIconError::InvalidPath);
    }
    Ok(PathBuf::from(path))
}

pub(super) fn validate_size(size: u32) -> Result<(), NativeIconError> {
    if size == 0 || size > MAX_ICON_SIZE {
        Err(NativeIconError::InvalidSize)
    } else {
        Ok(())
    }
}
