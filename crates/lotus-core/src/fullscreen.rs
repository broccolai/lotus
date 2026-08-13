#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

pub fn is_fullscreen_foreground(
    is_eligible_foreground: bool,
    is_maximized: bool,
    window: ScreenRect,
    monitor: ScreenRect,
    tolerance: i32,
) -> bool {
    is_eligible_foreground
        && !is_maximized
        && tolerance >= 0
        && covers_monitor(window, monitor, tolerance)
}

fn covers_monitor(window: ScreenRect, monitor: ScreenRect, tolerance: i32) -> bool {
    let tolerance = i64::from(tolerance);
    i64::from(window.left) <= i64::from(monitor.left) + tolerance
        && i64::from(window.top) <= i64::from(monitor.top) + tolerance
        && i64::from(window.right) >= i64::from(monitor.right) - tolerance
        && i64::from(window.bottom) >= i64::from(monitor.bottom) - tolerance
}
