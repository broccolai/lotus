use lotus_core::fullscreen::ScreenRect;
use lotus_ui::geometry::DpiScale;
use thiserror::Error;

type Result<T> = std::result::Result<T, AppBarGeometryError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppBarLayout {
    reserved_rect: ScreenRect,
    content_rect: ScreenRect,
    reserved_height: i32,
    content_width: i32,
    content_height: i32,
    bottom_offset: i32,
}

impl AppBarLayout {
    pub fn new(
        monitor: ScreenRect,
        content_width: u32,
        content_height: u32,
        bottom_offset_dips: u32,
        dpi: u32,
    ) -> Result<Self> {
        let _ = dimension(monitor.left, monitor.right, "monitor width")?;
        let monitor_height = dimension(monitor.top, monitor.bottom, "monitor height")?;
        let content_width = physical_dimension(content_width, "dock content width")?;
        let content_height = physical_dimension(content_height, "dock content height")?;
        let bottom_offset = DpiScale::from_system(dpi).physical_i32(bottom_offset_dips);
        let requested_height = i64::from(content_height) + i64::from(bottom_offset);
        let reserved_height = i32::try_from(requested_height.min(monitor_height))
            .map_err(|_| invalid_geometry("AppBar reserved height exceeds Win32 limits"))?;

        Self::from_bottom_edge(
            monitor.left,
            monitor.right,
            monitor.bottom,
            reserved_height,
            content_width,
            content_height,
            bottom_offset,
        )
    }

    pub fn with_shell_bounds(self, bounds: ScreenRect) -> Result<Self> {
        let _ = dimension(bounds.left, bounds.right, "negotiated AppBar width")?;
        Self::from_bottom_edge(
            bounds.left,
            bounds.right,
            bounds.bottom,
            self.reserved_height,
            self.content_width,
            self.content_height,
            self.bottom_offset,
        )
    }

    pub const fn reserved_rect(self) -> ScreenRect {
        self.reserved_rect
    }

    pub const fn content_rect(self) -> ScreenRect {
        self.content_rect
    }

    fn from_bottom_edge(
        left: i32,
        right: i32,
        bottom: i32,
        reserved_height: i32,
        content_width: i32,
        content_height: i32,
        bottom_offset: i32,
    ) -> Result<Self> {
        let available_width = dimension(left, right, "AppBar width")?;
        let reserved_top = checked_i32(
            i64::from(bottom) - i64::from(reserved_height),
            "AppBar top exceeds Win32 coordinates",
        )?;
        let content_left = checked_i32(
            i64::from(left) + (available_width - i64::from(content_width)) / 2,
            "dock horizontal position exceeds Win32 coordinates",
        )?;
        let vertical_gutter = i64::from(reserved_height) - i64::from(content_height);
        let content_top = checked_i32(
            i64::from(reserved_top) + vertical_gutter / 2,
            "dock vertical position exceeds Win32 coordinates",
        )?;
        let content_right = checked_i32(
            i64::from(content_left) + i64::from(content_width),
            "dock right edge exceeds Win32 coordinates",
        )?;
        let content_bottom = checked_i32(
            i64::from(content_top) + i64::from(content_height),
            "dock bottom edge exceeds Win32 coordinates",
        )?;

        Ok(Self {
            reserved_rect: ScreenRect { left, top: reserved_top, right, bottom },
            content_rect: ScreenRect {
                left: content_left,
                top: content_top,
                right: content_right,
                bottom: content_bottom,
            },
            reserved_height,
            content_width,
            content_height,
            bottom_offset,
        })
    }
}

fn dimension(start: i32, end: i32, label: &str) -> Result<i64> {
    let value = i64::from(end) - i64::from(start);
    if value <= 0 {
        return Err(invalid_geometry(&format!("{label} must be positive")));
    }
    Ok(value)
}

fn physical_dimension(value: u32, label: &str) -> Result<i32> {
    if value == 0 {
        return Err(invalid_geometry(&format!("{label} must be positive")));
    }
    i32::try_from(value).map_err(|_| invalid_geometry(&format!("{label} exceeds Win32 limits")))
}

fn checked_i32(value: i64, message: &str) -> Result<i32> {
    i32::try_from(value).map_err(|_| invalid_geometry(message))
}

#[derive(Debug, Error)]
#[error("invalid AppBar geometry: {0}")]
pub struct AppBarGeometryError(String);

fn invalid_geometry(message: &str) -> AppBarGeometryError {
    AppBarGeometryError(message.to_owned())
}
