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

pub(super) struct PlacementOutcome {
    pub discovery_wait: Duration,
    pub bridge_configuration: Duration,
    pub positioning: Duration,
    pub timed_out: bool,
    pub success: bool,
}

#[derive(Clone, Copy)]
struct PositionAttempt {
    requested: (i32, i32),
    accepted: bool,
}

pub(super) fn place_flyout(
    screen_x: Option<i32>,
    anchor_x: i32,
    anchor_y: i32,
    bridge: Option<&ShellBridgeLease>,
    bridge_setup: Duration,
    mut find_window: impl FnMut() -> Option<HWND>,
) -> PlacementOutcome {
    let deadline = Instant::now() + WINDOW_SETTLE_TIMEOUT;
    let mut previous_size = None;
    let mut stable_samples = 0;
    let mut bridge_configuration = bridge_setup;
    let mut positioning = Duration::ZERO;
    let mut discovery_wait = Duration::ZERO;
    let mut bridge_configured = false;
    let mut previous_position = None;
    while Instant::now() < deadline {
        let Some(window) = find_window() else {
            discovery_wait = discovery_wait.saturating_add(sleep_for_settle());
            continue;
        };
        let Some(rect) = visible_window_rect(window) else {
            discovery_wait = discovery_wait.saturating_add(sleep_for_settle());
            continue;
        };
        let size = (
            rect.right.saturating_sub(rect.left),
            rect.bottom.saturating_sub(rect.top),
        );
        if size.0 <= 0 || size.1 <= 0 {
            discovery_wait = discovery_wait.saturating_add(sleep_for_settle());
            continue;
        }
        if let Some(bridge) = bridge
            && !bridge_configured
        {
            let configured = Instant::now();
            bridge_configured = bridge.configure(screen_x.unwrap_or(anchor_x), anchor_y);
            bridge_configuration =
                bridge_configuration.saturating_add(configured.elapsed());
        }
        let observed_requested_position = previous_position == Some((rect.left, rect.top));
        let positioning_started = Instant::now();
        let attempt = position_window(window, screen_x, anchor_x, anchor_y, size.0, size.1);
        positioning = positioning.saturating_add(positioning_started.elapsed());
        let placement_attempted =
            bridge_configured || attempt.is_some_and(|item| item.accepted);
        previous_position = attempt.map(|item| item.requested);
        if !(placement_attempted && observed_requested_position) {
            previous_size = Some(size);
            stable_samples = 0;
            discovery_wait = discovery_wait.saturating_add(sleep_for_settle());
            continue;
        }
        if previous_size == Some(size) {
            stable_samples += 1;
            if stable_samples >= REQUIRED_STABLE_SAMPLES {
                return PlacementOutcome {
                    discovery_wait,
                    bridge_configuration,
                    positioning,
                    timed_out: false,
                    success: true,
                };
            }
        } else {
            previous_size = Some(size);
            stable_samples = 1;
        }
        discovery_wait = discovery_wait.saturating_add(sleep_for_settle());
    }
    PlacementOutcome {
        discovery_wait,
        bridge_configuration,
        positioning,
        timed_out: true,
        success: false,
    }
}

fn sleep_for_settle() -> Duration {
    let started = Instant::now();
    thread::sleep(WINDOW_SETTLE_RETRY);
    started.elapsed()
}

fn position_window(
    window: HWND,
    screen_x: Option<i32>,
    anchor_x: i32,
    anchor_y: i32,
    width: i32,
    height: i32,
) -> Option<PositionAttempt> {
    let display_x = screen_x.unwrap_or(anchor_x);
    let Ok(display) = nearest_display_to_point(display_x, anchor_y) else {
        return None;
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
    let accepted = unsafe {
        SetWindowPos(
            window,
            None,
            x,
            y,
            0,
            0,
            SET_WINDOW_POS_FLAGS(SWP_NOSIZE.0 | SWP_NOZORDER.0 | SWP_NOACTIVATE.0),
        )
    }
    .is_ok();
    Some(PositionAttempt {
        requested: (x, y),
        accepted,
    })
}
