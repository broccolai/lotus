use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::mem::size_of;
use std::path::PathBuf;

use lotus_core::window::{WindowId, WindowInfo};
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GA_ROOT, GW_OWNER, GWL_EXSTYLE, GetAncestor, GetClassNameW, GetWindow,
    GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    IsWindowVisible, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, Result as WindowsResult};

use crate::application_identity::window_application_identity_in_apartment;
use crate::launch::ComApartment;
use crate::responsiveness::METRICS;
const IMAGE_PATH_CAPACITY: usize = 32_768;
const CLASS_NAME_CAPACITY: usize = 128;
pub(super) fn enumerate_windows(
    own_process_id: u32,
    process_cache: &mut ProcessMetadataCache,
) -> WindowsResult<Vec<WindowInfo>> {
    let _apartment = ComApartment::enter();
    let mut state = EnumerationState {
        own_process_id,
        windows: Vec::new(),
        process_cache,
        observed_processes: HashSet::new(),
    };
    unsafe { EnumWindows(Some(visit_window), pointer_lparam(&raw mut state))? };
    unsafe { &mut *state.process_cache }.retain_processes(&state.observed_processes);
    Ok(state.windows)
}
struct EnumerationState {
    own_process_id: u32,
    windows: Vec<WindowInfo>,
    process_cache: *mut ProcessMetadataCache,
    observed_processes: HashSet<u32>,
}
unsafe extern "system" fn visit_window(hwnd: HWND, state: LPARAM) -> BOOL {
    // EnumWindows invokes this callback synchronously, preserving the unique state borrow.
    let state = unsafe { &mut *(state.0 as *mut EnumerationState) };
    let process_cache = unsafe { &mut *state.process_cache };
    if let Some(window) = window_info(
        hwnd,
        state.own_process_id,
        process_cache,
        &mut state.observed_processes,
    ) {
        state.windows.push(window);
    }
    BOOL(1)
}
fn window_info(
    hwnd: HWND,
    own_process_id: u32,
    process_cache: &mut ProcessMetadataCache,
    observed_processes: &mut HashSet<u32>,
) -> Option<WindowInfo> {
    if !should_include_window(hwnd) {
        return None;
    }
    let mut process_id = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut process_id)) };
    if process_id == 0 || process_id == own_process_id {
        return None;
    }
    let process = if observed_processes.contains(&process_id) {
        process_cache.cached(process_id)?
    } else {
        let process = process_cache.metadata(process_id)?;
        observed_processes.insert(process_id);
        process
    };
    let title = window_title(hwnd);
    let executable_path = window_icon_identity(&title, process.executable_path.clone());
    let id = window_id(hwnd)?;
    let application_identity = window_application_identity_in_apartment(id);
    Some(WindowInfo {
        id,
        process_id,
        title,
        executable_path,
        app_user_model_id: application_identity
            .and_then(|identity| identity.app_user_model_id)
            .or_else(|| process.app_user_model_id.clone()),
    })
}

#[derive(Default)]
pub(super) struct ProcessMetadataCache {
    entries: HashMap<u32, ProcessMetadata>,
}

struct ProcessMetadata {
    creation_time: u64,
    executable_path: PathBuf,
    app_user_model_id: Option<String>,
}

impl ProcessMetadataCache {
    fn cached(&self, process_id: u32) -> Option<&ProcessMetadata> {
        self.entries.get(&process_id)
    }

    fn metadata(&mut self, process_id: u32) -> Option<&ProcessMetadata> {
        let process = open_process(process_id)?;
        let creation_time = process_creation_time(process.get())?;
        let current = self
            .entries
            .get(&process_id)
            .is_some_and(|entry| entry.creation_time == creation_time);
        METRICS.record_process_metadata(current);
        if !current {
            let executable_path = process_image_path_from_handle(process.get())?;
            let app_user_model_id = process_application_id(process.get());
            self.entries.insert(
                process_id,
                ProcessMetadata {
                    creation_time,
                    executable_path,
                    app_user_model_id,
                },
            );
        }

        self.entries.get(&process_id)
    }

    fn retain_processes(&mut self, observed: &HashSet<u32>) {
        self.entries
            .retain(|process_id, _| observed.contains(process_id));
    }
}
fn window_icon_identity(title: &str, executable_path: PathBuf) -> PathBuf {
    let executable = executable_path.file_name().and_then(|name| name.to_str());
    if title.eq_ignore_ascii_case("Settings")
        && executable.is_some_and(|name| {
            name.eq_ignore_ascii_case("ApplicationFrameHost.exe")
                || name.eq_ignore_ascii_case("SystemSettings.exe")
        })
    {
        return PathBuf::from(
            r"shell:AppsFolder\windows.immersivecontrolpanel_cw5n1h2txyewy!microsoft.windows.immersivecontrolpanel",
        );
    }
    executable_path
}
pub(super) fn should_include_window(hwnd: HWND) -> bool {
    let (is_visible, root, has_owner, extended_style) = unsafe {
        (
            IsWindowVisible(hwnd).as_bool(),
            GetAncestor(hwnd, GA_ROOT),
            GetWindow(hwnd, GW_OWNER).is_ok(),
            GetWindowLongPtrW(hwnd, GWL_EXSTYLE).cast_unsigned(),
        )
    };
    let tool_window = usize::try_from(WS_EX_TOOLWINDOW.0).unwrap_or_default();
    let app_window = usize::try_from(WS_EX_APPWINDOW.0).unwrap_or_default();
    let is_tool_window = extended_style & tool_window != 0;
    let is_app_window = extended_style & app_window != 0;
    if !is_visible
        || root != hwnd
        || (has_owner && !is_app_window)
        || (is_tool_window && !is_app_window)
    {
        return false;
    }
    if excluded_window_class(&window_class(hwnd)) {
        return false;
    }
    let mut cloaked = 0_u32;
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast::<c_void>(),
            u32_size::<u32>(),
        )
    }
    .is_err()
        || cloaked == 0
}

fn excluded_window_class(class_name: &str) -> bool {
    matches!(
        class_name,
        "Progman"
            | "WorkerW"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "XamlExplorerHostIslandWindow"
    )
}
fn window_title(hwnd: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    let mut buffer = vec![
        0_u16;
        usize::try_from(length.max(0))
            .unwrap_or_default()
            .saturating_add(1)
    ];
    let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..usize::try_from(copied.max(0)).unwrap_or_default()])
}
fn window_class(hwnd: HWND) -> String {
    let mut buffer = [0_u16; CLASS_NAME_CAPACITY];
    let copied = unsafe { GetClassNameW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..usize::try_from(copied.max(0)).unwrap_or_default()])
}
fn window_id(hwnd: HWND) -> Option<WindowId> {
    (!hwnd.0.is_null())
        .then(|| u64::try_from(hwnd.0.addr()).ok())
        .flatten()
        .map(WindowId::new)
}
pub(crate) fn process_image_path(process_id: u32) -> Option<PathBuf> {
    let process = open_process(process_id)?;
    process_image_path_from_handle(process.get())
}

fn open_process(process_id: u32) -> Option<OwnedHandle> {
    unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }
        .ok()
        .map(OwnedHandle)
}

fn process_image_path_from_handle(process: HANDLE) -> Option<PathBuf> {
    let mut buffer = vec![0_u16; IMAGE_PATH_CAPACITY];
    let mut length = u32::try_from(buffer.len()).ok()?;
    unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &raw mut length,
        )
    }
    .ok()?;
    buffer.truncate(usize::try_from(length).ok()?);
    Some(PathBuf::from(String::from_utf16_lossy(&buffer)))
}

fn process_application_id(process: HANDLE) -> Option<String> {
    let mut buffer = vec![0_u16; 512];
    let mut length = u32::try_from(buffer.len()).ok()?;
    let result = unsafe {
        GetApplicationUserModelId(
            process,
            &raw mut length,
            Some(windows::core::PWSTR(buffer.as_mut_ptr())),
        )
    };
    if result.0 != 0 || length == 0 {
        return None;
    }
    let text_length = usize::try_from(length.saturating_sub(1)).ok()?;
    Some(String::from_utf16_lossy(&buffer[..text_length]))
}

fn process_creation_time(process: HANDLE) -> Option<u64> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe {
        GetProcessTimes(
            process,
            &raw mut creation,
            &raw mut exit,
            &raw mut kernel,
            &raw mut user,
        )
    }
    .ok()?;
    Some((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
}
struct OwnedHandle(HANDLE);
impl OwnedHandle {
    fn get(&self) -> HANDLE {
        self.0
    }
}
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}
#[allow(
    clippy::cast_possible_wrap,
    reason = "Win32 LPARAM intentionally transports an in-process pointer-sized value"
)]
fn pointer_lparam<T>(pointer: *mut T) -> LPARAM {
    LPARAM(pointer.addr() as isize)
}
#[allow(
    clippy::cast_possible_truncation,
    reason = "Win32 ABI scalar sizes are fixed and far below u32::MAX"
)]
const fn u32_size<T>() -> u32 {
    size_of::<T>() as u32
}
