use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::{fs, ptr};

use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::Storage::FileSystem::SearchPathW;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, IPersistFile, STGM_READ,
};
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::UI::Shell::{IShellLinkW, SLGP_RAWPATH, ShellLink};
use windows::core::{Interface, PCWSTR};

const WINDOWS_PATH_CAPACITY: usize = 32_768;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurableLaunch {
    pub target: String,
    pub arguments: Option<String>,
    pub icon_source: Option<String>,
}

pub fn durable_launch_for_executable(executable: &str) -> Option<DurableLaunch> {
    let executable = Path::new(executable);

    squirrel_launch(executable).or_else(|| steam_launch(executable))
}

fn squirrel_launch(executable: &Path) -> Option<DurableLaunch> {
    let version_directory = executable.parent()?;
    let version_name = version_directory.file_name()?.to_str()?;
    if !version_name
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("app-"))
    {
        return None;
    }

    let install_directory = version_directory.parent()?;
    let updater = install_directory.join("Update.exe");
    if !updater.is_file() {
        return None;
    }

    let executable_name = executable.file_name()?.to_str()?;
    let icon = install_directory.join("app.ico");
    Some(DurableLaunch {
        target: updater.to_string_lossy().into_owned(),
        arguments: Some(format!(
            "--processStart {}",
            quoted_argument(executable_name)
        )),
        icon_source: icon.is_file().then(|| icon.to_string_lossy().into_owned()),
    })
}

fn steam_launch(executable: &Path) -> Option<DurableLaunch> {
    let executable_name = executable.file_name()?.to_str()?;
    if !executable_name.eq_ignore_ascii_case("steamwebhelper.exe") {
        return None;
    }

    let steam = executable
        .ancestors()
        .skip(1)
        .map(|directory| directory.join("steam.exe"))
        .find(|candidate| candidate.is_file())?;
    let target = steam.to_string_lossy().into_owned();

    Some(DurableLaunch {
        icon_source: Some(target.clone()),
        target,
        arguments: None,
    })
}

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

pub(crate) fn resolve_shortcut_icon(shortcut_path: &Path) -> Option<(PathBuf, i32)> {
    if !has_extension(shortcut_path, "lnk") {
        return None;
    }
    let _apartment = ComApartment::enter()?;
    // SAFETY: COM is initialized for this thread and ShellLink is an in-process COM class.
    let shell_link: IShellLinkW =
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }.ok()?;
    let persist: IPersistFile = shell_link.cast().ok()?;
    let shortcut = wide_path_null(shortcut_path);
    // SAFETY: `shortcut` is a live null-terminated path for this synchronous COM call.
    unsafe { persist.Load(PCWSTR(shortcut.as_ptr()), STGM_READ) }.ok()?;

    let mut icon_path = vec![0u16; WINDOWS_PATH_CAPACITY];
    let mut icon_index = 0;
    // SAFETY: The output buffer and icon-index pointer are valid for this synchronous call.
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
    // SAFETY: COM is initialized for this thread and ShellLink is an in-process COM class.
    let shell_link: IShellLinkW =
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }.ok()?;
    let persist: IPersistFile = shell_link.cast().ok()?;
    let shortcut = wide_path_null(shortcut_path);
    // SAFETY: `shortcut` is a live null-terminated path for this synchronous COM call.
    unsafe { persist.Load(PCWSTR(shortcut.as_ptr()), STGM_READ) }.ok()?;

    let mut arguments = vec![0_u16; WINDOWS_PATH_CAPACITY];
    // SAFETY: `arguments` is valid writable UTF-16 storage for this synchronous COM call.
    unsafe { shell_link.GetArguments(&mut arguments) }.ok()?;
    let arguments = String::from_utf16_lossy(utf16_without_nul(&arguments));
    let arguments = arguments.trim();
    (!arguments.is_empty()).then(|| arguments.to_owned())
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

fn quoted_argument(value: &str) -> String {
    if value.contains(char::is_whitespace) {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_owned()
    }
}

pub(super) fn expand_environment_variables(source: &str) -> Option<String> {
    let source = wide_null(source);
    // SAFETY: `source` is a valid, null-terminated UTF-16 string. A null
    // destination is the documented size-query form.
    let required = unsafe { ExpandEnvironmentStringsW(PCWSTR(source.as_ptr()), None) };
    if required == 0 {
        return None;
    }

    let mut expanded = vec![0u16; usize::try_from(required).ok()?];
    // SAFETY: Both UTF-16 buffers are live and the destination has exactly the
    // capacity requested by the preceding call.
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

    // SAFETY: Input strings are null-terminated and `buffer` is valid writable
    // storage for the full slice passed to SearchPathW.
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
    // SAFETY: COM is initialized for this thread (or was already initialized
    // in a different apartment), and ShellLink is an in-process COM class.
    let shell_link: IShellLinkW =
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }.ok()?;
    let persist: IPersistFile = shell_link.cast().ok()?;
    let shortcut_path = wide_path_null(shortcut_path);
    // SAFETY: `shortcut_path` is null-terminated and the COM interfaces remain
    // alive for the synchronous Load call.
    unsafe { persist.Load(PCWSTR(shortcut_path.as_ptr()), STGM_READ) }.ok()?;

    let mut target = vec![0u16; WINDOWS_PATH_CAPACITY];
    // SAFETY: `target` is valid writable UTF-16 storage. A null find-data
    // pointer is permitted when only the resolved path is requested.
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
        // SAFETY: The reserved pointer must be null. The returned status tells
        // us whether this call owns a matching CoUninitialize obligation.
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
            // SAFETY: `enter` successfully initialized COM on this thread and
            // every COM interface declared after the guard has already dropped.
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
