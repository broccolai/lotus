use lotus_ui::frame::{FrameOutcome, ScheduledSurface};
use lotus_windows::graphics::launcher_surface::LauncherCompositionSurfaceState;
use lotus_windows::graphics::surface::FrameResult;
use lotus_windows::graphics::{
    CompositionSurfaceState, DeviceState, SurfaceError, SurfaceSize,
};
use lotus_windows::window::DockWindow;
use lotus_windows::window_tracker::WindowTracker;

use crate::app::modules::ModuleHost;
use crate::app::visuals::surface_size;
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
    dock: &DockWindow,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    graphics: &mut DeviceState,
    window_tracker: &WindowTracker,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    auxiliary.sync_monitor_docks(dock, dock_model, graphics, window_tracker)?;
    if runtime.onboarding_required {
        dock.set_mascot_animation_delay(None)?;
        return Ok(());
    }
    if !runtime.startup_mode.allows_shell_integration() {
        let _changed = dock.set_fullscreen_occluded(false)?;
        auxiliary.set_status_fullscreen_occluded(false)?;
        dock.set_mascot_animation_delay(
            dock.is_visible()
                .then(|| dock_model.mascot_animation_delay())
                .flatten(),
        )?;
        return Ok(());
    }
    apply_fullscreen_visibility(dock, surface, window_tracker, dock_model, auxiliary)?;
    let mascot_visible = (dock.is_visible() && !dock.is_fullscreen_occluded())
        || auxiliary.has_visible_monitor_dock();
    dock.set_mascot_animation_delay(
        mascot_visible
            .then(|| dock_model.mascot_animation_delay())
            .flatten(),
    )?;
    Ok(())
}

pub(crate) fn apply_fullscreen_visibility(
    dock: &DockWindow,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    tracker: &WindowTracker,
    model: &DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    let fullscreen_present = tracker.fullscreen_on_same_monitor(dock.handle());
    let temporarily_revealed = auxiliary.launcher_is_visible();
    let occluded = !dock_visible(
        model.settings().hide_when_fullscreen,
        fullscreen_present && !temporarily_revealed,
    );
    if occluded {
        auxiliary.hide_launcher();
        surface.stop_animation();
    }
    let changed = dock.set_fullscreen_occluded(occluded)?;
    if changed {
        lotus_windows::diagnostics::record_state(
            "dock.visibility",
            &[
                ("occluded", u64::from(occluded)),
                ("fullscreen_present", u64::from(fullscreen_present)),
                ("search_reveal", u64::from(temporarily_revealed)),
                ("dock_visible", u64::from(dock.is_visible())),
            ],
        );
    }
    auxiliary.set_status_fullscreen_occluded(occluded)?;
    Ok(())
}

const fn dock_visible(hide_when_fullscreen: bool, fullscreen_present: bool) -> bool {
    !hide_when_fullscreen || !fullscreen_present
}

pub(crate) fn resize_surface(
    graphics: &mut DeviceState,
    surface: &mut CompositionSurfaceState,
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

pub(crate) fn render_surface(
    graphics: &mut DeviceState,
    surface: &mut CompositionSurfaceState,
    model: &mut DockRuntime,
) -> Result<FrameOutcome, AppError> {
    let (presentation, needs_animation) = model.presentation();
    match surface.render_scene(&presentation, needs_animation) {
        Ok(FrameResult::Presented { needs_animation }) => {
            Ok(FrameOutcome::complete(needs_animation))
        }
        Ok(FrameResult::TargetRecreated) => Ok(FrameOutcome::Retry),
        Err(SurfaceError::DeviceLost(loss)) => {
            graphics.mark_lost(loss);
            Ok(FrameOutcome::complete(false))
        }
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn resize_dock(
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    model: &DockRuntime,
) -> Result<(), AppError> {
    let size = model.scene().desired_size();
    dock.resize_content(size.width(), size.height(), model.settings())?;
    resize_surface(graphics, surface.value_mut(), surface_size(size))
}

pub(crate) fn present_dock_change(
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    host: &mut ModuleHost,
    model: &mut DockRuntime,
) -> Result<(), AppError> {
    resize_dock(dock, graphics, surface, model)?;
    host.sync_status(dock, model, graphics)?;
    surface.invalidate();
    Ok(())
}
