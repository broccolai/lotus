use lotus_windows::graphics::launcher_surface::LauncherCompositionSurfaceState;
use lotus_windows::graphics::scene::DockScene;
use lotus_windows::graphics::{
    CompositionSurfaceState, DeviceState, SurfaceError, SurfaceSize,
};
use lotus_windows::window::DockWindow;
use lotus_windows::window_tracker::WindowTracker;

use crate::app::launcher::LauncherRuntime;
use crate::app::status::StatusRuntime;
use crate::app::switcher::AuxiliaryWindows;
use crate::app::{AppError, DockRuntime, RuntimePolicy};

pub(super) fn sync_monitor_presentation(
    runtime: &RuntimePolicy<'_>,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    window_tracker: &WindowTracker,
    dock_model: &mut DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    auxiliary
        .monitors
        .sync(dock, dock_model, graphics, window_tracker)?;
    if runtime.onboarding_required {
        return Ok(());
    }
    apply_fullscreen_visibility(
        dock,
        window_tracker,
        dock_model,
        &mut auxiliary.launcher,
        &auxiliary.status,
    )
}

pub(crate) fn apply_fullscreen_visibility(
    dock: &DockWindow,
    tracker: &WindowTracker,
    model: &DockRuntime,
    launcher: &mut LauncherRuntime,
    status: &StatusRuntime,
) -> Result<(), AppError> {
    let fullscreen_present = tracker.fullscreen_on_same_monitor(dock.handle());
    let temporarily_revealed = launcher.is_visible();
    let occluded = !dock_visible(
        model.settings().hide_when_fullscreen,
        fullscreen_present && !temporarily_revealed,
    );
    if occluded {
        launcher.hide();
        dock.set_animation_active(false)?;
    }
    let _changed = dock.set_fullscreen_occluded(occluded)?;
    status.set_fullscreen_occluded(occluded)?;
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
    scene: &DockScene,
) -> Result<bool, AppError> {
    match surface.render_scene(scene) {
        Ok(frame) => Ok(frame.needs_animation()),
        Err(SurfaceError::DeviceLost(_)) => {
            recover_graphics(graphics, surface)?;
            Ok(surface.render_scene(scene)?.needs_animation())
        }
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn render_and_schedule(
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut CompositionSurfaceState,
    scene: &DockScene,
    launcher_needs_animation: bool,
) -> Result<(), AppError> {
    let needs_animation =
        render_surface(graphics, surface, scene)? || launcher_needs_animation;
    dock.set_animation_active(needs_animation)?;
    Ok(())
}

pub(crate) fn resize_dock(
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut CompositionSurfaceState,
    model: &DockRuntime,
) -> Result<(), AppError> {
    let size = model.scene().desired_size();
    dock.resize_content(size.width(), size.height(), model.settings())?;
    resize_surface(graphics, surface, SurfaceSize::from(size))
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
