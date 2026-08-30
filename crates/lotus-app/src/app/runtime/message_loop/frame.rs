use lotus_ui::frame::{FrameOutcome, FramePass, FrameTrigger, ScheduledSurface};
use lotus_windows::graphics::{
    CompositionSurfaceState, DeviceState, GraphicsDeviceHealth, SurfaceError,
};
use lotus_windows::window::DockWindow;

use crate::app::modules::ModuleHost;
use crate::app::runtime::presentation;
use crate::app::{AppError, DockRuntime};

pub(super) fn flush_frame(
    dock: &mut DockWindow,
    graphics: &mut DeviceState,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
    trigger: FrameTrigger,
) -> Result<(), AppError> {
    if graphics.health() == GraphicsDeviceHealth::Lost {
        dock.set_animation_active(false)?;
        return Ok(());
    }
    let mut pass = FramePass::new(trigger);
    let device_generation = graphics.generation();
    let animation_allowed = !dock.is_fullscreen_occluded();
    pass.render(surface, |surface| {
        presentation::render_surface(graphics, surface, dock_model).map(|outcome| {
            match outcome {
                FrameOutcome::Complete {
                    continues_animation,
                } => FrameOutcome::complete(continues_animation && animation_allowed),
                FrameOutcome::Retry => FrameOutcome::Retry,
            }
        })
    })?;
    match auxiliary.render_frames(&mut pass, graphics) {
        Ok(()) => {}
        Err(AppError::Surface(SurfaceError::DeviceLost(loss))) => {
            graphics.mark_lost(loss);
            dock.set_animation_active(false)?;
            return Ok(());
        }
        Err(error) => return Err(error),
    }

    if graphics.generation() != device_generation {
        surface.invalidate();
        auxiliary.invalidate_surfaces();
        pass.request_next_frame();
    }

    dock.set_animation_active(pass.animation_active())?;
    let mascot_visible = (dock.is_visible() && !dock.is_fullscreen_occluded())
        || auxiliary.has_visible_monitor_dock();
    dock.set_mascot_animation_delay(
        mascot_visible
            .then(|| dock_model.mascot_animation_delay())
            .flatten(),
    )?;
    Ok(())
}
