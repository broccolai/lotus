use lotus_ui::frame::ScheduledSurface;
use lotus_windows::graphics::{
    CompositionSurfaceState, DeviceState, SurfaceError, SurfaceSize,
};
use lotus_windows::window::{DockEvent, DockWindow};

use crate::app::AppError;
use crate::app::dock::DockRuntime;
use crate::app::surface_render::frame_outcome;
use crate::app::visuals::surface_size;

/// Owns the native primary dock window and its matching composition surface.
///
/// The dock model remains separate: this type applies native layout, surface, and
/// frame effects for the model's already-computed presentation.
pub(super) struct PrimaryDock {
    window: DockWindow,
    surface: ScheduledSurface<CompositionSurfaceState>,
}

impl PrimaryDock {
    pub(super) fn create(
        graphics: &DeviceState,
        settings: &lotus_core::settings::DockSettings,
    ) -> Result<Self, AppError> {
        let window = DockWindow::create()?;
        lotus_windows::backdrop::apply_dock_settings(window.handle(), settings);
        window.prepare(settings)?;
        let (width, height) = window.client_size()?;
        let size = SurfaceSize::new(width, height).ok_or(AppError::ZeroSizedSurface)?;
        let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
        let surface = CompositionSurfaceState::create(device, window.handle(), size)?;

        Ok(Self {
            window,
            surface: ScheduledSurface::new(surface),
        })
    }

    pub(super) const fn window(&self) -> &DockWindow {
        &self.window
    }

    pub(super) fn drain_events(&mut self) -> Vec<DockEvent> {
        self.window.drain_events().collect()
    }

    pub(super) fn resize_for_model(
        &mut self,
        graphics: &mut DeviceState,
        model: &DockRuntime,
    ) -> Result<(), AppError> {
        let size = model.scene().desired_size();
        self.window
            .resize_content(size.width(), size.height(), model.settings())?;
        self.resize_surface(graphics, surface_size(size))
    }

    pub(super) fn resize_surface(
        &mut self,
        graphics: &mut DeviceState,
        size: SurfaceSize,
    ) -> Result<(), AppError> {
        match self.surface.value_mut().resize(size) {
            Ok(()) => Ok(()),
            Err(SurfaceError::DeviceLost(loss)) => {
                graphics.mark_lost(loss);
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }

    pub(super) fn render_in_frame(
        &mut self,
        pass: &mut lotus_ui::frame::FramePass,
        graphics: &mut DeviceState,
        model: &mut DockRuntime,
    ) -> Result<(), AppError> {
        let animation_allowed = !self.window.is_fullscreen_occluded();
        pass.render(&mut self.surface, |surface| {
            let (presentation, needs_animation) = model.presentation();
            frame_outcome(
                graphics,
                surface.render_scene(&presentation, needs_animation),
            )
            .map(|outcome| outcome.with_animation_allowed(animation_allowed))
        })?;
        Ok(())
    }

    pub(super) fn invalidate(&mut self) {
        self.surface.invalidate();
    }

    pub(super) fn stop_animation(&mut self) {
        self.surface.stop_animation();
    }

    pub(super) fn is_dirty(&self) -> bool {
        self.surface.is_dirty()
    }

    pub(super) fn is_animating(&self) -> bool {
        self.surface.is_animating()
    }

    pub(super) fn recover_surface(
        &mut self,
        device: &lotus_windows::graphics::GraphicsDevice,
    ) -> Result<(), AppError> {
        self.surface.value_mut().recover(device)?;
        Ok(())
    }
}
