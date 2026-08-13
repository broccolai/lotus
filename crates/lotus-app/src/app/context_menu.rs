use lotus_core::settings::DockSettings;
use lotus_settings::appearance::theme_for;
use lotus_ui::theme::Theme;

use super::{
    AppError, ContextMenuCompositionSurfaceState, ContextMenuEvent, ContextMenuScene,
    ContextMenuWindow, DeviceState, NonZeroPhysicalSize, SignedPoint, SurfaceError,
};

pub(super) struct ContextMenuRuntime {
    pub(super) window: ContextMenuWindow,
    pub(super) scene: ContextMenuScene,
    pub(super) surface: Option<ContextMenuCompositionSurfaceState>,
    pub(super) visible: bool,
}

impl ContextMenuRuntime {
    pub(super) fn new(window: ContextMenuWindow, theme: &Theme) -> Result<Self, AppError> {
        let mut scene =
            ContextMenuScene::new(window.dpi()).ok_or(AppError::InvalidContextMenuScene)?;
        let _ = scene.set_theme(*theme);
        Ok(Self {
            window,
            scene,
            surface: None,
            visible: false,
        })
    }

    pub(super) fn apply_settings(&mut self, settings: &DockSettings) {
        let _ = self.scene.set_theme(theme_for(settings));
        lotus_windows::backdrop::apply_popup_settings(self.window.handle(), settings);
    }

    pub(super) fn open(
        &mut self,
        anchor: SignedPoint,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let mut desired = self.scene.desired_size();
        let dpi = self.window.prepare_at(anchor, desired)?;
        if self.scene.set_dpi(dpi) {
            desired = self.scene.desired_size();
            let _dpi = self.window.prepare_at(anchor, desired)?;
        }
        if let Some(surface) = &mut self.surface {
            surface.resize(desired)?;
        } else {
            let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
            self.surface = Some(ContextMenuCompositionSurfaceState::create(
                device,
                self.window.handle(),
                desired,
            )?);
        }
        self.visible = true;
        self.render(graphics)?;
        self.window.show();
        Ok(())
    }

    pub(super) fn hide(&mut self) {
        if self.visible {
            self.window.hide();
            self.visible = false;
            let _ = self.scene.pointer_left();
        }
    }

    pub(super) fn render(&mut self, graphics: &mut DeviceState) -> Result<(), AppError> {
        if !self.visible {
            return Ok(());
        }
        let surface = self
            .surface
            .as_mut()
            .ok_or(AppError::InvalidContextMenuScene)?;
        match surface.render_scene(&self.scene) {
            Ok(_) => Ok(()),
            Err(SurfaceError::DeviceLost(_)) => {
                let _ = graphics.poll();
                graphics.recover()?;
                let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
                surface.recover(device)?;
                let _ = surface.render_scene(&self.scene)?;
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }

    pub(super) fn resize(&mut self, width: u32, height: u32) -> Result<(), AppError> {
        let Some(size) = NonZeroPhysicalSize::new(width, height) else {
            return Ok(());
        };
        if let Some(surface) = &mut self.surface {
            surface.resize(size)?;
        }
        Ok(())
    }

    pub(super) fn drain_events(&mut self) -> Vec<ContextMenuEvent> {
        self.window.drain_events().collect()
    }
}
