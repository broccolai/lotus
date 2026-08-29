use std::ffi::{OsString, c_void};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::{fs, ptr};

use windows::Win32::Foundation::{HLOCAL, LocalFree, RPC_E_CHANGED_MODE};
use windows::Win32::Storage::FileSystem::SearchPathW;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, IPersistFile, STGM_READ,
};
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::UI::Shell::{CommandLineToArgvW, IShellLinkW, SLGP_RAWPATH, ShellLink};
use windows::core::{Interface, PCWSTR, PWSTR};

const WINDOWS_PATH_CAPACITY: usize = 32_768;

pub fn resolve_executable(target: &str) -> Option<PathBuf> {
    let target = prepare_target(target)?;
    let expanded = expand_environment_variables(target)?;
    let expanded_path = Path::new(&expanded);

    if expanded_path.is_file() {
        if has_extension(expanded_path, "lnk") {
            return resolve_shortcut(expanded_path);
        }
        return std::path::absolute(expanded_path).ok();
    }

    if is_path_like_or_absolute_uri(&expanded) {
        return None;
    }

    search_path(&expanded)
}

pub fn application_file_location(
    launch_target: &str,
    icon_source: &str,
) -> Option<PathBuf> {
    existing_file(launch_target)
        .or_else(|| existing_file(icon_source).filter(|path| has_extension(path, "exe")))
        .or_else(|| resolve_executable(launch_target))
}

fn existing_file(value: &str) -> Option<PathBuf> {
    let value = prepare_target(value)?;
    let expanded = expand_environment_variables(value)?;
    let path = Path::new(&expanded);
    if path.is_file() {
        std::path::absolute(path).ok()
    } else {
        None
    }
}

pub(crate) fn resolve_shortcut_icon(shortcut_path: &Path) -> Option<(PathBuf, i32)> {
    if !has_extension(shortcut_path, "lnk") {
        return None;
    }
    let _apartment = ComApartment::enter()?;
    let shell_link: IShellLinkW =
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }.ok()?;
    let persist: IPersistFile = shell_link.cast().ok()?;
    let shortcut = wide_path_null(shortcut_path);
    unsafe { persist.Load(PCWSTR(shortcut.as_ptr()), STGM_READ) }.ok()?;

    let mut icon_path = vec![0u16; WINDOWS_PATH_CAPACITY];
    let mut icon_index = 0;
    unsafe { shell_link.GetIconLocation(&mut icon_path, &raw mut icon_index) }.ok()?;
    let icon_path = String::from_utf16_lossy(utf16_without_nul(&icon_path));
    let expanded = expand_environment_variables(icon_path.trim().trim_matches('"'))?;
    let candidate = PathBuf::from(expanded);
    let candidate = if candidate.is_absolute() {
        candidate
    } else {
        shortcut_path.parent()?.join(candidate)
    };
    if !candidate.is_file() {
        return None;
    }
    Some((std::path::absolute(candidate).ok()?, icon_index))
}

pub(crate) fn shortcut_arguments(shortcut_path: &Path) -> Option<String> {
    if !has_extension(shortcut_path, "lnk") {
        return None;
    }

    let _apartment = ComApartment::enter()?;
    let shell_link: IShellLinkW =
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }.ok()?;
    let persist: IPersistFile = shell_link.cast().ok()?;
    let shortcut = wide_path_null(shortcut_path);
    unsafe { persist.Load(PCWSTR(shortcut.as_ptr()), STGM_READ) }.ok()?;

    let mut arguments = vec![0_u16; WINDOWS_PATH_CAPACITY];
    unsafe { shell_link.GetArguments(&mut arguments) }.ok()?;
    let arguments = String::from_utf16_lossy(utf16_without_nul(&arguments));
    let arguments = arguments.trim();
    (!arguments.is_empty()).then(|| arguments.to_owned())
}

pub(crate) fn command_line_arguments(arguments: &str) -> Vec<String> {
    if arguments.trim().is_empty() {
        return Vec::new();
    }
    let arguments = wide_null(arguments);
    let mut argument_count = 0;
    let raw =
        unsafe { CommandLineToArgvW(PCWSTR(arguments.as_ptr()), &raw mut argument_count) };
    if raw.is_null() || argument_count <= 0 {
        return Vec::new();
    }
    let arguments = LocalArguments(raw);
    unsafe {
        std::slice::from_raw_parts(
            arguments.0,
            usize::try_from(argument_count).unwrap_or_default(),
        )
    }
    .iter()
    .map(|value| unsafe { value.to_string() }.ok())
    .collect::<Option<Vec<_>>>()
    .unwrap_or_default()
}

struct LocalArguments(*mut PWSTR);

impl Drop for LocalArguments {
    fn drop(&mut self) {
        let _ = unsafe { LocalFree(Some(HLOCAL(self.0.cast::<c_void>()))) };
    }
}

pub(crate) fn resolve_internet_shortcut_icon(
    shortcut_path: &Path,
) -> Option<(PathBuf, i32)> {
    if !has_extension(shortcut_path, "url") {
        return None;
    }
    let contents = decode_internet_shortcut(&fs::read(shortcut_path).ok()?)?;
    let mut in_internet_shortcut = false;
    let mut icon_path = None;
    let mut icon_index = 0;
    for line in contents.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            in_internet_shortcut =
                line[1..line.len() - 1].eq_ignore_ascii_case("InternetShortcut");
            continue;
        }
        if !in_internet_shortcut {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("IconFile") {
            icon_path = Some(value.trim().trim_matches('"'));
        } else if key.trim().eq_ignore_ascii_case("IconIndex") {
            icon_index = value.trim().parse().unwrap_or(0);
        }
    }

    let expanded = expand_environment_variables(icon_path?)?;
    let candidate = PathBuf::from(expanded);
    let candidate = if candidate.is_absolute() {
        candidate
    } else {
        shortcut_path.parent()?.join(candidate)
    };
    if !candidate.is_file() {
        return None;
    }
    Some((std::path::absolute(candidate).ok()?, icon_index))
}

fn decode_internet_shortcut(bytes: &[u8]) -> Option<String> {
    if let Some(bytes) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        return String::from_utf16(&units).ok();
    }
    if let Some(bytes) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        return String::from_utf16(&units).ok();
    }
    Some(
        String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes))
            .into(),
    )
}

fn prepare_target(target: &str) -> Option<&str> {
    let target = target.trim().trim_matches('"');
    (!target.trim().is_empty()).then_some(target)
}

fn is_path_like_or_absolute_uri(target: &str) -> bool {
    target.contains(['\\', '/']) || has_uri_scheme(target)
}

fn has_uri_scheme(target: &str) -> bool {
    let Some((scheme, _)) = target.split_once(':') else {
        return false;
    };
    let mut characters = scheme.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
}

fn has_extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
}

pub(super) fn expand_environment_variables(source: &str) -> Option<String> {
    let source = wide_null(source);
    let required = unsafe { ExpandEnvironmentStringsW(PCWSTR(source.as_ptr()), None) };
    if required == 0 {
        return None;
    }

    let mut expanded = vec![0u16; usize::try_from(required).ok()?];
    let written = unsafe {
        ExpandEnvironmentStringsW(PCWSTR(source.as_ptr()), Some(expanded.as_mut_slice()))
    };
    if written == 0 || written > required {
        return None;
    }

    Some(String::from_utf16_lossy(utf16_without_nul(&expanded)))
}

fn search_path(file_name: &str) -> Option<PathBuf> {
    let file_name = wide_null(file_name);
    let extension = wide_null(".exe");
    let mut buffer = vec![0u16; WINDOWS_PATH_CAPACITY];

    let length = unsafe {
        SearchPathW(
            PCWSTR::null(),
            PCWSTR(file_name.as_ptr()),
            PCWSTR(extension.as_ptr()),
            Some(buffer.as_mut_slice()),
            None,
        )
    };
    let length = usize::try_from(length).ok()?;
    if length == 0 || length >= buffer.len() {
        return None;
    }

    buffer.truncate(length);
    Some(PathBuf::from(OsString::from_wide(&buffer)))
}

fn resolve_shortcut(shortcut_path: &Path) -> Option<PathBuf> {
    let _apartment = ComApartment::enter()?;
    let shell_link: IShellLinkW =
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }.ok()?;
    let persist: IPersistFile = shell_link.cast().ok()?;
    let shortcut_path = wide_path_null(shortcut_path);
    unsafe { persist.Load(PCWSTR(shortcut_path.as_ptr()), STGM_READ) }.ok()?;

    let mut target = vec![0u16; WINDOWS_PATH_CAPACITY];
    unsafe {
        shell_link.GetPath(&mut target, ptr::null_mut(), SLGP_RAWPATH.0.cast_unsigned())
    }
    .ok()?;
    let target = String::from_utf16_lossy(utf16_without_nul(&target));
    if target.is_empty() {
        return None;
    }

    let expanded = expand_environment_variables(&target)?;
    let expanded = Path::new(&expanded);
    expanded
        .is_file()
        .then(|| std::path::absolute(expanded).ok())
        .flatten()
}

pub(super) struct ComApartment {
    uninitialize: bool,
}

impl ComApartment {
    pub(super) fn enter() -> Option<Self> {
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if result.is_ok() {
            Some(Self { uninitialize: true })
        } else if result == RPC_E_CHANGED_MODE {
            Some(Self {
                uninitialize: false,
            })
        } else {
            None
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.uninitialize {
            unsafe { CoUninitialize() };
        }
    }
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

fn wide_path_null(value: &Path) -> Vec<u16> {
    value.as_os_str().encode_wide().chain([0]).collect()
}

fn utf16_without_nul(value: &[u16]) -> &[u16] {
    let length = value
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(value.len());
    &value[..length]
}
