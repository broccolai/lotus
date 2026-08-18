use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    SET_WINDOW_POS_FLAGS, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
};

use super::discovery::visible_window_rect;
use crate::platform::windows::display::nearest_display_to_point;
use crate::shell_bridge::ShellBridgeLease;

const WINDOW_SETTLE_TIMEOUT: Duration = Duration::from_millis(400);
const WINDOW_SETTLE_RETRY: Duration = Duration::from_millis(16);
const REQUIRED_STABLE_SAMPLES: u8 = 5;
const EDGE_INSET_DIP: i32 = 12;

pub(super) fn place_flyout(
    screen_x: Option<i32>,
    anchor_x: i32,
    anchor_y: i32,
    bridge: Option<&ShellBridgeLease>,
    mut find_window: impl FnMut() -> Option<HWND>,
) {
    let deadline = Instant::now() + WINDOW_SETTLE_TIMEOUT;
    let mut previous_size = None;
    let mut stable_samples = 0;
    while Instant::now() < deadline {
        let Some(window) = find_window() else {
            thread::sleep(WINDOW_SETTLE_RETRY);
            continue;
        };
        let Some(rect) = visible_window_rect(window) else {
            thread::sleep(WINDOW_SETTLE_RETRY);
            continue;
        };
        let size = (
            rect.right.saturating_sub(rect.left),
            rect.bottom.saturating_sub(rect.top),
        );
        if size.0 <= 0 || size.1 <= 0 {
            thread::sleep(WINDOW_SETTLE_RETRY);
            continue;
        }
        if let Some(bridge) = bridge {
            let _ = bridge.configure(screen_x.unwrap_or(anchor_x), anchor_y);
        }
        position_window(window, screen_x, anchor_x, anchor_y, size.0, size.1);
        if previous_size == Some(size) {
            stable_samples += 1;
            if stable_samples >= REQUIRED_STABLE_SAMPLES && bridge.is_none() {
                return;
            }
        } else {
            previous_size = Some(size);
            stable_samples = 1;
        }
        thread::sleep(WINDOW_SETTLE_RETRY);
    }
}

fn position_window(
    window: HWND,
    screen_x: Option<i32>,
    anchor_x: i32,
    anchor_y: i32,
    width: i32,
    height: i32,
) {
    let display_x = screen_x.unwrap_or(anchor_x);
    let Ok(display) = nearest_display_to_point(display_x, anchor_y) else {
        return;
    };
    let dpi = display.dpi().map_or(96, lotus_ui::geometry::DpiScale::dpi);
    let inset = EDGE_INSET_DIP.saturating_mul(i32::try_from(dpi).unwrap_or(96)) / 96;
    let maximum_x = display.work_area.right.saturating_sub(width);
    let maximum_y = display.work_area.bottom.saturating_sub(height);
    let x = screen_x.map_or_else(
        || {
            display
                .work_area
                .right
                .saturating_sub(width)
                .saturating_sub(inset)
        },
        |screen_x| screen_x.saturating_sub(width / 2),
    );
    let x = x.clamp(
        display.work_area.left.saturating_add(inset),
        maximum_x
            .saturating_sub(inset)
            .max(display.work_area.left.saturating_add(inset)),
    );
    let y = anchor_y
        .saturating_sub(height)
        .clamp(display.work_area.top, maximum_y.max(display.work_area.top));
    let _ = unsafe {
        SetWindowPos(
            window,
            None,
            x,
            y,
            0,
            0,
            SET_WINDOW_POS_FLAGS(SWP_NOSIZE.0 | SWP_NOZORDER.0 | SWP_NOACTIVATE.0),
        )
    };
}
