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
use crate::app::{AppError, DockRuntime, RuntimePolicy};

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
    runtime: &RuntimePolicy<'_>,
    dock: &DockWindow,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    graphics: &mut DeviceState,
    window_tracker: &WindowTracker,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    auxiliary.sync_monitor_docks(dock, dock_model, graphics, window_tracker)?;
    if runtime.onboarding_required {
        return Ok(());
    }
    apply_fullscreen_visibility(dock, surface, window_tracker, dock_model, auxiliary)
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
    let _changed = dock.set_fullscreen_occluded(occluded)?;
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
        Err(SurfaceError::DeviceLost(_)) => recover_graphics(graphics, surface),
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
        Err(SurfaceError::DeviceLost(_)) => {
            let _ = graphics.poll();
            graphics.recover()?;
            let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
            surface.recover(device)?;
            surface.resize(size)?;
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
        Err(SurfaceError::DeviceLost(_)) => {
            recover_graphics(graphics, surface)?;
            match surface.render_scene(&presentation, needs_animation)? {
                FrameResult::Presented { needs_animation } => {
                    Ok(FrameOutcome::complete(needs_animation))
                }
                FrameResult::TargetRecreated => Ok(FrameOutcome::Retry),
            }
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

fn recover_graphics(
    graphics: &mut DeviceState,
    surface: &mut CompositionSurfaceState,
) -> Result<(), AppError> {
    let _ = graphics.poll();
    graphics.recover()?;
    let graphics_device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
    surface.recover(graphics_device)?;
    Ok(())
}
