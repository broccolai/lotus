use lotus_ui::embedded_icon::EmbeddedIcon;
use lotus_ui::frame::{FramePass, ScheduledSurface};
use lotus_ui::geometry::NonZeroPhysicalSize;
use lotus_ui::presentation::Presentation;
use lotus_windows::graphics::switcher_surface::SwitcherCompositionSurfaceState;
use lotus_windows::graphics::{DeviceState, GraphicsDevice, SurfaceError};

use crate::app::AppError;
use crate::app::surface_render::frame_outcome;

impl super::SwitcherRuntime {
    pub(in crate::app) fn diagnostic_surface_state(&self) -> (bool, bool, bool) {
        let surface = self.surface.as_ref();
        (
            surface.is_some_and(ScheduledSurface::is_dirty),
            surface.is_some_and(ScheduledSurface::is_animating),
            self.session.is_some(),
        )
    }

    pub(in crate::app) const fn presentation_ready(&self) -> bool {
        self.presentation_ready
    }

    pub(in crate::app) fn prepare_presentation(
        &mut self,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if self.surface.is_some() {
            return Ok(());
        }

        let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
        let size = NonZeroPhysicalSize::new(1, 1).expect("warm-up size is nonzero");
        self.surface = Some(ScheduledSurface::new(
            SwitcherCompositionSurfaceState::create(device, self.window.handle(), size)?,
        ));
        self.presentation_ready = false;
        Ok(())
    }

    pub(super) fn ensure_surface(
        &mut self,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let size = self
            .scene
            .as_ref()
            .ok_or(AppError::InvalidSwitcherScene)?
            .desired_size();
        if let Some(surface) = &mut self.surface {
            surface.value_mut().resize(size)?;
            return Ok(());
        }
        let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
        self.surface = Some(ScheduledSurface::new(
            SwitcherCompositionSurfaceState::create(device, self.window.handle(), size)?,
        ));
        self.presentation_ready = false;
        Ok(())
    }

    pub(super) fn resize_surface(
        &mut self,
        size: NonZeroPhysicalSize,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let Some(surface) = &mut self.surface else {
            return Ok(());
        };
        match surface.value_mut().resize(size) {
            Ok(()) => Ok(()),
            Err(SurfaceError::DeviceLost(loss)) => {
                graphics.mark_lost(loss);
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }

    pub(in crate::app) fn invalidate(&mut self) {
        if let Some(surface) = &mut self.surface {
            surface.invalidate();
        }
    }

    pub(in crate::app) fn recover_surface(
        &mut self,
        device: &GraphicsDevice,
    ) -> Result<(), AppError> {
        self.presentation_ready = false;
        if let Some(surface) = &mut self.surface {
            surface.value_mut().recover(device)?;
        }
        Ok(())
    }

    pub(in crate::app) fn render_frame(
        &mut self,
        pass: &mut FramePass,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if self.session.is_some() && self.scene.is_none() {
            return Err(AppError::InvalidSwitcherScene);
        }

        let scene = self.scene.as_ref();
        let Some(surface) = &mut self.surface else {
            if scene.is_some() {
                return Err(AppError::InvalidSwitcherScene);
            }
            return Ok(());
        };
        if scene.is_none() && self.presentation_ready && !surface.is_dirty() {
            surface.stop_animation();
            return Ok(());
        }
        let presentation = if let Some(scene) = scene {
            scene.presentation(EmbeddedIcon::FluentDismiss)
        } else {
            Presentation::new(self.theme.canvas.with_alpha(0.0))
        };
        let result = pass.render(surface, |surface| {
            frame_outcome(graphics, surface.render_scene(&presentation))
        });

        self.presentation_ready =
            result.is_ok() && graphics.ready().is_some() && !surface.is_dirty();
        result
    }
}
