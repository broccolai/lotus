use std::mem::size_of;

use lotus_ui::geometry::DpiScale;
use windows::Win32::Foundation::{E_FAIL, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromPoint, MonitorFromWindow,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;
use windows::core::{BOOL, Error};

use crate::NativeError;

type Result<T> = std::result::Result<T, NativeError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenArea {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl From<RECT> for ScreenArea {
    fn from(rectangle: RECT) -> Self {
        Self {
            left: rectangle.left,
            top: rectangle.top,
            right: rectangle.right,
            bottom: rectangle.bottom,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Display {
    handle: HMONITOR,
    pub bounds: ScreenArea,
    pub work_area: ScreenArea,
    is_primary: bool,
}

impl Display {
    pub fn dpi(self) -> Result<DpiScale> {
        let mut horizontal = 0;
        let mut vertical = 0;
        // SAFETY: The handle came from live monitor enumeration and both outputs are writable.
        unsafe {
            GetDpiForMonitor(
                self.handle,
                MDT_EFFECTIVE_DPI,
                &raw mut horizontal,
                &raw mut vertical,
            )?;
        }
        Ok(DpiScale::from_system(horizontal))
    }
}

pub fn nearest_display(hwnd: HWND) -> Result<Display> {
    // SAFETY: The live or defensive fallback HWND is used only to select its nearest monitor.
    let handle = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    display_info(handle)
}

pub fn nearest_display_to_point(x: i32, y: i32) -> Result<Display> {
    // SAFETY: The physical screen point is a value and selects, but does not retain, a monitor.
    let handle = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    display_info(handle)
}

pub fn primary_display() -> Result<Display> {
    Ok(all_displays()?.into_iter().find(|display| display.is_primary).ok_or_else(no_display)?)
}

fn all_displays() -> Result<Vec<Display>> {
    let mut displays = Vec::new();
    // SAFETY: Enumeration is synchronous and LPARAM carries a live vector pointer throughout.
    unsafe {
        EnumDisplayMonitors(None, None, Some(collect_display), pointer_lparam(&raw mut displays))
            .ok()?;
    }
    Ok(displays)
}

unsafe extern "system" fn collect_display(
    monitor: HMONITOR,
    _monitor_dc: HDC,
    _bounds: *mut RECT,
    state: LPARAM,
) -> BOOL {
    // SAFETY: `state` is the live vector pointer supplied to synchronous enumeration.
    let displays = unsafe { &mut *(state.0 as *mut Vec<Display>) };
    if let Ok(display) = display_info(monitor) {
        displays.push(display);
    }
    BOOL(1)
}

fn display_info(handle: HMONITOR) -> Result<Display> {
    let mut info = MONITORINFO { cbSize: monitor_info_size(), ..MONITORINFO::default() };
    // SAFETY: The monitor handle is live and `info` is initialized writable ABI storage.
    unsafe { GetMonitorInfoW(handle, &raw mut info).ok()? };
    Ok(Display {
        handle,
        bounds: info.rcMonitor.into(),
        work_area: info.rcWork.into(),
        is_primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
    })
}

fn no_display() -> Error {
    Error::new(E_FAIL, "Lotus could not find a display monitor")
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "MONITORINFO is a fixed Win32 ABI structure far smaller than u32::MAX"
)]
const fn monitor_info_size() -> u32 {
    size_of::<MONITORINFO>() as u32
}

#[allow(
    clippy::cast_possible_wrap,
    reason = "Win32 LPARAM intentionally transports an in-process pointer-sized value"
)]
fn pointer_lparam<T>(pointer: *mut T) -> LPARAM {
    LPARAM(pointer.addr() as isize)
}
