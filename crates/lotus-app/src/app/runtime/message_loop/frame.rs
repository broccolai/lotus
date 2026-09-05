use lotus_ui::frame::{FramePass, FrameTrigger};
use lotus_windows::graphics::{DeviceState, GraphicsDeviceHealth, SurfaceError};

use crate::app::modules::ModuleHost;
use crate::app::primary_dock::PrimaryDock;
use crate::app::{AppError, DockRuntime};

pub(super) fn flush_frame(
    primary_dock: &mut PrimaryDock,
    graphics: &mut DeviceState,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
    trigger: FrameTrigger,
) -> Result<(), AppError> {
    if graphics.health() == GraphicsDeviceHealth::Lost {
        primary_dock.window().set_animation_active(false)?;
        return Ok(());
    }
    let mut pass = FramePass::new(trigger);
    let device_generation = graphics.generation();
    primary_dock.render_in_frame(&mut pass, graphics, dock_model)?;
    match auxiliary.render_frames(&mut pass, graphics) {
        Ok(()) => {}
        Err(AppError::Surface(SurfaceError::DeviceLost(loss))) => {
            graphics.mark_lost(loss);
            primary_dock.window().set_animation_active(false)?;
            return Ok(());
        }
        Err(error) => return Err(error),
    }

    if graphics.generation() != device_generation {
        primary_dock.invalidate();
        auxiliary.invalidate_surfaces();
        pass.request_next_frame();
    }

    primary_dock
        .window()
        .set_animation_active(pass.animation_active())?;
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
