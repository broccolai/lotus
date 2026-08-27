use std::mem::size_of;

use thiserror::Error;
use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
    DISPLAYCONFIG_DEVICE_INFO_SET_ADVANCED_COLOR_STATE,
    DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_MODE_INFO,
    DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, DisplayConfigGetDeviceInfo,
    DisplayConfigSetDeviceInfo, GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS,
    QueryDisplayConfig,
};
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, HWND};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MONITORINFOEXW,
    MonitorFromWindow,
};
use windows::core::{Error, HRESULT};

use crate::{NativeError, WindowHandle};

const ADVANCED_COLOR_SUPPORTED: u32 = 1 << 0;
const ADVANCED_COLOR_ENABLED: u32 = 1 << 1;
const DISPLAY_CONFIG_RETRIES: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdvancedColorState {
    Sdr,
    Hdr,
}

impl AdvancedColorState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sdr => "SDR",
            Self::Hdr => "HDR",
        }
    }
}

#[derive(Debug, Error)]
pub enum AdvancedColorError {
    #[error("Lotus could not match that window to an active display")]
    DisplayNotFound,
    #[error("HDR is not supported by this display")]
    Unsupported,
    #[error(transparent)]
    Native(#[from] NativeError),
}

pub fn state(window: WindowHandle) -> Result<AdvancedColorState, AdvancedColorError> {
    let target = target_for_window(window.raw())?;
    target.state()
}

pub fn toggle(window: WindowHandle) -> Result<AdvancedColorState, AdvancedColorError> {
    let target = target_for_window(window.raw())?;
    let state = target.state()?;
    let next = if state == AdvancedColorState::Sdr {
        AdvancedColorState::Hdr
    } else {
        AdvancedColorState::Sdr
    };
    target.set_enabled(next == AdvancedColorState::Hdr)?;
    Ok(next)
}

#[derive(Clone, Copy)]
struct DisplayTarget {
    adapter_id: windows::Win32::Foundation::LUID,
    target_id: u32,
}

impl DisplayTarget {
    fn state(self) -> Result<AdvancedColorState, AdvancedColorError> {
        let mut info = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
            header: device_info_header(
                DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
                size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>(),
                self.adapter_id,
                self.target_id,
            ),
            ..DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO::default()
        };
        let status = unsafe { DisplayConfigGetDeviceInfo(&raw mut info.header) };
        check_status(status)?;

        // SAFETY: DisplayConfigGetDeviceInfo initialized the active `value` union field.
        let flags = unsafe { info.Anonymous.value };
        if flags & ADVANCED_COLOR_SUPPORTED == 0 {
            return Err(AdvancedColorError::Unsupported);
        }
        Ok(if flags & ADVANCED_COLOR_ENABLED == 0 {
            AdvancedColorState::Sdr
        } else {
            AdvancedColorState::Hdr
        })
    }

    fn set_enabled(self, enabled: bool) -> Result<(), AdvancedColorError> {
        let mut request = DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE {
            header: device_info_header(
                DISPLAYCONFIG_DEVICE_INFO_SET_ADVANCED_COLOR_STATE,
                size_of::<DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE>(),
                self.adapter_id,
                self.target_id,
            ),
            ..DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE::default()
        };
        request.Anonymous.value = u32::from(enabled);
        let status = unsafe { DisplayConfigSetDeviceInfo(&raw const request.header) };
        check_status(status)
    }
}

fn target_for_window(window: HWND) -> Result<DisplayTarget, AdvancedColorError> {
    let device_name = monitor_device_name(window)?;
    let paths = active_display_paths()?;
    for path in paths {
        if source_device_name(&path)? == device_name {
            return Ok(DisplayTarget {
                adapter_id: path.targetInfo.adapterId,
                target_id: path.targetInfo.id,
            });
        }
    }
    Err(AdvancedColorError::DisplayNotFound)
}

fn monitor_device_name(window: HWND) -> Result<[u16; 32], AdvancedColorError> {
    let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFOEXW {
        monitorInfo: MONITORINFO {
            cbSize: u32_size::<MONITORINFOEXW>(),
            ..MONITORINFO::default()
        },
        ..MONITORINFOEXW::default()
    };
    unsafe { GetMonitorInfoW(monitor, (&raw mut info.monitorInfo).cast()).ok() }
        .map_err(NativeError::from)?;
    Ok(info.szDevice)
}

fn active_display_paths() -> Result<Vec<DISPLAYCONFIG_PATH_INFO>, AdvancedColorError> {
    for _ in 0..DISPLAY_CONFIG_RETRIES {
        let mut path_count = 0;
        let mut mode_count = 0;
        let status = unsafe {
            GetDisplayConfigBufferSizes(
                QDC_ONLY_ACTIVE_PATHS,
                &raw mut path_count,
                &raw mut mode_count,
            )
        };
        check_status(status.0.cast_signed())?;

        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        let status = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &raw mut path_count,
                paths.as_mut_ptr(),
                &raw mut mode_count,
                modes.as_mut_ptr(),
                None,
            )
        };
        if status == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        if status != ERROR_SUCCESS {
            return Err(native_status(status.0.cast_signed()));
        }
        paths.truncate(path_count as usize);
        return Ok(paths);
    }
    Err(native_status(ERROR_INSUFFICIENT_BUFFER.0.cast_signed()))
}

fn source_device_name(
    path: &DISPLAYCONFIG_PATH_INFO,
) -> Result<[u16; 32], AdvancedColorError> {
    let mut name = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
        header: device_info_header(
            DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
            size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>(),
            path.sourceInfo.adapterId,
            path.sourceInfo.id,
        ),
        ..DISPLAYCONFIG_SOURCE_DEVICE_NAME::default()
    };
    let status = unsafe { DisplayConfigGetDeviceInfo(&raw mut name.header) };
    check_status(status)?;
    Ok(name.viewGdiDeviceName)
}

fn device_info_header(
    kind: windows::Win32::Devices::Display::DISPLAYCONFIG_DEVICE_INFO_TYPE,
    size: usize,
    adapter_id: windows::Win32::Foundation::LUID,
    id: u32,
) -> DISPLAYCONFIG_DEVICE_INFO_HEADER {
    DISPLAYCONFIG_DEVICE_INFO_HEADER {
        r#type: kind,
        size: u32::try_from(size).expect("display configuration packets fit in u32"),
        adapterId: adapter_id,
        id,
    }
}

fn check_status(status: i32) -> Result<(), AdvancedColorError> {
    if status == 0 {
        Ok(())
    } else {
        Err(native_status(status))
    }
}

fn native_status(status: i32) -> AdvancedColorError {
    AdvancedColorError::Native(
        Error::from_hresult(HRESULT::from_win32(status.cast_unsigned())).into(),
    )
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "Win32 display configuration structures are fixed and far below u32::MAX"
)]
const fn u32_size<T>() -> u32 {
    size_of::<T>() as u32
}
