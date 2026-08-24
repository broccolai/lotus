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

#[derive(Clone, Copy)]
pub(super) struct PlacementOutcome {
    pub discovery_wait: Duration,
    pub bridge_configuration: Duration,
    pub positioning: Duration,
    pub timed_out: bool,
    pub success: bool,
}

impl PlacementOutcome {
    pub(super) const fn cancelled(discovery_wait: Duration) -> Self {
        Self {
            discovery_wait,
            bridge_configuration: Duration::ZERO,
            positioning: Duration::ZERO,
            timed_out: false,
            success: false,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct PlacementRequest<'a> {
    pub screen_x: Option<i32>,
    pub anchor_x: i32,
    pub anchor_y: i32,
    pub bridge: Option<&'a ShellBridgeLease>,
    pub bridge_setup: Duration,
}

#[derive(Clone, Copy)]
struct PositionAttempt {
    requested: (i32, i32),
    accepted: bool,
}

pub(super) fn place_flyout(
    request: PlacementRequest<'_>,
    mut find_window: impl FnMut() -> Option<HWND>,
    mut is_current: impl FnMut() -> bool,
    mut set_position: impl FnMut(HWND, i32, i32) -> bool,
    mut configure_bridge: impl FnMut(&ShellBridgeLease, i32, i32) -> bool,
) -> PlacementOutcome {
    let deadline = Instant::now() + WINDOW_SETTLE_TIMEOUT;
    let mut previous_size = None;
    let mut stable_samples = 0;
    let mut bridge_configuration = request.bridge_setup;
    let mut positioning = Duration::ZERO;
    let mut discovery_wait = Duration::ZERO;
    let mut bridge_configured = false;
    let mut previous_position = None;
    while Instant::now() < deadline {
        if !is_current() {
            return cancelled_outcome(discovery_wait, bridge_configuration, positioning);
        }
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
        if let Some(bridge) = request.bridge
            && !bridge_configured
        {
            if !is_current() {
                return cancelled_outcome(
                    discovery_wait,
                    bridge_configuration,
                    positioning,
                );
            }
            let configured = Instant::now();
            bridge_configured = configure_bridge(
                bridge,
                request.screen_x.unwrap_or(request.anchor_x),
                request.anchor_y,
            );
            bridge_configuration =
                bridge_configuration.saturating_add(configured.elapsed());
        }
        let observed_requested_position = previous_position == Some((rect.left, rect.top));
        if !is_current() {
            return cancelled_outcome(discovery_wait, bridge_configuration, positioning);
        }
        let positioning_started = Instant::now();
        let attempt = position_window(
            window,
            request.screen_x,
            request.anchor_x,
            request.anchor_y,
            size.0,
            size.1,
            &mut set_position,
        );
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

fn cancelled_outcome(
    discovery_wait: Duration,
    bridge_configuration: Duration,
    positioning: Duration,
) -> PlacementOutcome {
    PlacementOutcome {
        discovery_wait,
        bridge_configuration,
        positioning,
        timed_out: false,
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
    set_position: &mut impl FnMut(HWND, i32, i32) -> bool,
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
    let accepted = set_position(window, x, y);
    Some(PositionAttempt {
        requested: (x, y),
        accepted,
    })
}

pub(super) fn set_window_position(window: HWND, x: i32, y: i32) -> bool {
    unsafe {
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
    .is_ok()
}
