use lotus_core::settings::DockSettings;
use lotus_settings::scene::{SettingsScene, SettingsSize};
use lotus_ui::embedded_icon::EmbeddedIcon;
use lotus_ui::frame::{FramePass, ScheduledSurface};
use lotus_ui::presentation::Presentation;
use lotus_windows::WindowHandle;
use lotus_windows::graphics::settings_surface::SettingsCompositionSurfaceState;
use lotus_windows::graphics::{DeviceState, GraphicsDevice, SurfaceError, SurfaceSize};
use lotus_windows::interaction::PointerCursor;
use lotus_windows::window::{SettingsEvent, SettingsWindow};

use crate::app::AppError;
use crate::app::surface_render::frame_outcome;

/// Owns the native Settings window and its optional composition surface.
///
/// Settings scene and interaction state remain in `SettingsRuntime`. This type
/// only applies their computed layout and presentation to the native window.
pub(super) struct SettingsSurface {
    window: SettingsWindow,
    surface: Option<ScheduledSurface<SettingsCompositionSurfaceState>>,
    visible: bool,
}

impl SettingsSurface {
    pub(super) const fn new(window: SettingsWindow) -> Self {
        Self {
            window,
            surface: None,
            visible: false,
        }
    }

    pub(super) fn focus(&self) {
        self.window.focus();
    }

    pub(super) fn use_material(&self, settings: &DockSettings) {
        self.window.use_material(settings);
    }

    pub(super) fn show(
        &mut self,
        scene: &mut SettingsScene,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let _ = scene.set_dpi(self.window.dpi());

        let (width, height) = self.window.client_size()?;
        let _ = scene.set_available_size(width, height);
        self.window.set_layout_dpi(scene.effective_dpi());
        let size =
            SettingsSize::new(width, height).ok_or(AppError::InvalidSettingsScene)?;
        let surface_size = SurfaceSize::new(size.width(), size.height())
            .ok_or(AppError::ZeroSizedSurface)?;

        if let Some(surface) = &mut self.surface {
            surface.value_mut().resize(surface_size)?;
        } else {
            let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
            self.surface = Some(ScheduledSurface::new(
                SettingsCompositionSurfaceState::create(
                    device,
                    self.window.handle(),
                    surface_size,
                )?,
            ));
        }

        self.window.show()?;
        self.visible = true;
        self.invalidate();
        Ok(())
    }

    pub(super) fn hide(&mut self) {
        self.window.hide();
        self.visible = false;
        if let Some(surface) = &mut self.surface {
            surface.stop_animation();
        }
    }

    pub(super) const fn is_visible(&self) -> bool {
        self.visible
    }

    pub(super) fn diagnostic_state(&self) -> (bool, bool, bool) {
        let surface = self.surface.as_ref();
        (
            surface.is_some_and(ScheduledSurface::is_dirty),
            surface.is_some_and(ScheduledSurface::is_animating),
            self.visible,
        )
    }

    pub(super) fn owner(&self) -> WindowHandle {
        self.window.handle()
    }

    pub(super) fn set_layout_dpi(&self, dpi: u32) {
        self.window.set_layout_dpi(dpi);
    }

    pub(super) fn set_pointer_cursor(&self, cursor: PointerCursor) {
        self.window.set_pointer_cursor(cursor);
    }

    pub(super) fn invalidate(&mut self) {
        if let Some(surface) = &mut self.surface {
            surface.invalidate();
        }
    }

    pub(super) fn recover(&mut self, device: &GraphicsDevice) -> Result<(), AppError> {
        if let Some(surface) = &mut self.surface {
            surface.value_mut().recover(device)?;
        }
        Ok(())
    }

    pub(super) fn drain_events(&mut self) -> Vec<SettingsEvent> {
        self.window.drain_events().collect()
    }

    pub(super) fn has_pending_events(&self) -> bool {
        self.window.has_pending_events()
    }

    pub(super) fn resize(
        &mut self,
        scene: &mut SettingsScene,
        graphics: &mut DeviceState,
        width: u32,
        height: u32,
    ) -> Result<(), AppError> {
        let Some(size) = SettingsSize::new(width, height) else {
            return Ok(());
        };
        let _ = scene.set_available_size(width, height);
        self.window.set_layout_dpi(scene.effective_dpi());
        let Some(surface) = &mut self.surface else {
            return Ok(());
        };

        let size = SurfaceSize::new(size.width(), size.height())
            .ok_or(AppError::ZeroSizedSurface)?;
        match surface.value_mut().resize(size) {
            Ok(()) => {
                self.invalidate();
                Ok(())
            }
            Err(SurfaceError::DeviceLost(loss)) => {
                graphics.mark_lost(loss);
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }

    pub(super) fn render_frame(
        &mut self,
        pass: &mut FramePass,
        graphics: &mut DeviceState,
        presentation: &Presentation<EmbeddedIcon>,
    ) -> Result<(), AppError> {
        let Some(surface) = &mut self.surface else {
            return Err(AppError::InvalidSettingsScene);
        };
        pass.render(surface, |surface| {
            frame_outcome(graphics, surface.render_scene(presentation))
                .map(|frame| frame.with_animation_allowed(false))
        })
    }
}
