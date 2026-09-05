use lotus_windows::graphics::launcher_surface::LauncherCompositionSurfaceState;
use lotus_windows::graphics::{DeviceState, SurfaceError, SurfaceSize};
use lotus_windows::window_tracker::WindowTracker;

use crate::app::modules::ModuleHost;
use crate::app::primary_dock::PrimaryDock;
use crate::app::{AppError, DockRuntime, RuntimeServices};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MonitorPresentationKey {
    tracker: u64,
    dock: u64,
    topology: u64,
    launcher_visible: bool,
}

pub(super) fn monitor_presentation_key(
    window_tracker: &WindowTracker,
    dock_model: &DockRuntime,
    auxiliary: &ModuleHost,
) -> MonitorPresentationKey {
    MonitorPresentationKey {
        tracker: window_tracker.presentation_revision(),
        dock: dock_model.revision(),
        topology: auxiliary.monitor_topology_generation(),
        launcher_visible: auxiliary.launcher_is_visible(),
    }
}

pub(super) fn sync_monitor_presentation(
    runtime: &RuntimeServices<'_>,
    primary_dock: &mut PrimaryDock,
    graphics: &mut DeviceState,
    window_tracker: &WindowTracker,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    auxiliary.sync_monitor_docks(
        primary_dock.window(),
        dock_model,
        graphics,
        window_tracker,
    )?;
    if runtime.onboarding_required {
        primary_dock.window().set_mascot_animation_delay(None)?;
        return Ok(());
    }
    if !runtime.startup_mode.allows_shell_integration() {
        let _changed = primary_dock.window().set_fullscreen_occluded(false)?;
        auxiliary.set_status_fullscreen_occluded(false)?;
        primary_dock.window().set_mascot_animation_delay(
            primary_dock
                .window()
                .is_visible()
                .then(|| dock_model.mascot_animation_delay())
                .flatten(),
        )?;
        return Ok(());
    }
    apply_fullscreen_visibility(primary_dock, window_tracker, dock_model, auxiliary)?;
    let mascot_visible = (primary_dock.window().is_visible()
        && !primary_dock.window().is_fullscreen_occluded())
        || auxiliary.has_visible_monitor_dock();
    primary_dock.window().set_mascot_animation_delay(
        mascot_visible
            .then(|| dock_model.mascot_animation_delay())
            .flatten(),
    )?;
    Ok(())
}

pub(crate) fn apply_fullscreen_visibility(
    primary_dock: &mut PrimaryDock,
    tracker: &WindowTracker,
    model: &DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    let fullscreen_present =
        tracker.fullscreen_on_same_monitor(primary_dock.window().handle());
    let temporarily_revealed = auxiliary.launcher_is_visible();
    let occluded = !dock_visible(
        model.settings().hide_when_fullscreen,
        fullscreen_present && !temporarily_revealed,
    );
    if occluded {
        auxiliary.hide_launcher();
        primary_dock.stop_animation();
    }
    let changed = primary_dock.window().set_fullscreen_occluded(occluded)?;
    if changed {
        lotus_windows::diagnostics::record_state(
            "dock.visibility",
            &[
                ("occluded", u64::from(occluded)),
                ("fullscreen_present", u64::from(fullscreen_present)),
                ("search_reveal", u64::from(temporarily_revealed)),
                (
                    "dock_visible",
                    u64::from(primary_dock.window().is_visible()),
                ),
            ],
        );
    }
    auxiliary.set_status_fullscreen_occluded(occluded)?;
    Ok(())
}

const fn dock_visible(hide_when_fullscreen: bool, fullscreen_present: bool) -> bool {
    !hide_when_fullscreen || !fullscreen_present
}

pub(crate) fn resize_launcher_surface(
    graphics: &mut DeviceState,
    surface: &mut LauncherCompositionSurfaceState,
    size: SurfaceSize,
) -> Result<(), AppError> {
    match surface.resize(size) {
        Ok(()) => Ok(()),
        Err(SurfaceError::DeviceLost(loss)) => {
            graphics.mark_lost(loss);
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn resize_surface(
    graphics: &mut DeviceState,
    surface: &mut lotus_windows::graphics::CompositionSurfaceState,
    size: SurfaceSize,
) -> Result<(), AppError> {
    match surface.resize(size) {
        Ok(()) => Ok(()),
        Err(SurfaceError::DeviceLost(loss)) => {
            graphics.mark_lost(loss);
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn present_dock_change(
    primary_dock: &mut PrimaryDock,
    graphics: &mut DeviceState,
    host: &mut ModuleHost,
    model: &mut DockRuntime,
) -> Result<(), AppError> {
    primary_dock.resize_for_model(graphics, model)?;
    host.sync_status(primary_dock.window(), model, graphics)?;
    primary_dock.invalidate();
    Ok(())
}
