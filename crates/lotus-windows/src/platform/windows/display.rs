use std::mem::size_of;

use lotus_ui::geometry::DpiScale;
use windows::Win32::Foundation::{E_FAIL, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST,
    MONITORINFO, MonitorFromPoint, MonitorFromWindow,
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

impl ScreenArea {
    pub const fn width(self) -> i32 {
        self.right.saturating_sub(self.left)
    }

    pub const fn height(self) -> i32 {
        self.bottom.saturating_sub(self.top)
    }

    pub const fn inset(self, amount: i32) -> Self {
        Self {
            left: self.left.saturating_add(amount),
            top: self.top.saturating_add(amount),
            right: self.right.saturating_sub(amount),
            bottom: self.bottom.saturating_sub(amount),
        }
    }

    pub const fn centered_origin(self, width: i32, height: i32) -> (i32, i32) {
        (
            self.left
                .saturating_add(self.width().saturating_sub(width) / 2),
            self.top
                .saturating_add(self.height().saturating_sub(height) / 2),
        )
    }

    pub fn clamp_origin_for_size(
        self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> (i32, i32) {
        let maximum_x = self.right.saturating_sub(width).max(self.left);
        let maximum_y = self.bottom.saturating_sub(height).max(self.top);
        (x.clamp(self.left, maximum_x), y.clamp(self.top, maximum_y))
    }
}

pub fn fit_aspect_ratio(
    width: i32,
    height: i32,
    maximum_width: i32,
    maximum_height: i32,
) -> (i32, i32) {
    let width = width.max(1);
    let height = height.max(1);
    let maximum_width = maximum_width.max(1);
    let maximum_height = maximum_height.max(1);
    if width <= maximum_width && height <= maximum_height {
        return (width, height);
    }

    let width_limited = i64::from(maximum_width) * i64::from(height)
        <= i64::from(maximum_height) * i64::from(width);
    if width_limited {
        let height = i64::from(height) * i64::from(maximum_width) / i64::from(width);
        (
            maximum_width,
            i32::try_from(height)
                .unwrap_or(maximum_height)
                .clamp(1, maximum_height),
        )
    } else {
        let width = i64::from(width) * i64::from(maximum_height) / i64::from(height);
        (
            i32::try_from(width)
                .unwrap_or(maximum_width)
                .clamp(1, maximum_width),
            maximum_height,
        )
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
    let handle = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    display_info(handle)
}

pub fn nearest_display_to_point(x: i32, y: i32) -> Result<Display> {
    let handle = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    display_info(handle)
}

pub fn primary_display() -> Result<Display> {
    Ok(all_displays()?
        .into_iter()
        .find(|display| display.is_primary)
        .ok_or_else(no_display)?)
}

pub(crate) fn secondary_displays() -> Result<Vec<Display>> {
    Ok(all_displays()?
        .into_iter()
        .filter(|display| !display.is_primary)
        .collect())
}

fn all_displays() -> Result<Vec<Display>> {
    let mut displays = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect_display),
            pointer_lparam(&raw mut displays),
        )
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
    let displays = unsafe { &mut *(state.0 as *mut Vec<Display>) };
    if let Ok(display) = display_info(monitor) {
        displays.push(display);
    }
    BOOL(1)
}

fn display_info(handle: HMONITOR) -> Result<Display> {
    let mut info = MONITORINFO {
        cbSize: monitor_info_size(),
        ..MONITORINFO::default()
    };
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
