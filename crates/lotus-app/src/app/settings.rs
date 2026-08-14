use lotus_windows::interaction::PointerCursor;
use lotus_windows::update::{Release, UpdateChecker, UpdateResult, UpdateStartError};

use super::{
    AppError, DeviceState, DockSettings, SettingsCompositionSurfaceState, SettingsEvent,
    SettingsScene, SettingsSize, SettingsUpdateActivity, SettingsWindow, SurfaceError,
};

pub(super) struct SettingsRuntime {
    pub(super) window: SettingsWindow,
    pub(super) scene: SettingsScene,
    pub(super) surface: Option<SettingsCompositionSurfaceState>,
    pub(super) visible: bool,
    pub(super) dragging_slider: Option<super::SettingsSlider>,
    update_checker: UpdateChecker,
}

impl SettingsRuntime {
    pub(super) fn new(
        window: SettingsWindow,
        settings: DockSettings,
        installed: bool,
    ) -> Result<Self, AppError> {
        let scene = SettingsScene::new(window.dpi(), settings, installed)
            .ok_or(AppError::InvalidSettingsScene)?;
        Ok(Self {
            window,
            scene,
            surface: None,
            visible: false,
            dragging_slider: None,
            update_checker: UpdateChecker::new(),
        })
    }

    pub(super) fn open(
        &mut self,
        applied: &DockSettings,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if self.visible {
            self.window.focus();
            return Ok(());
        }
        self.window.use_material();
        self.scene.end_onboarding();
        self.scene.mark_applied(applied.clone());
        self.show(graphics)
    }

    pub(super) fn open_onboarding(
        &mut self,
        applied: &DockSettings,
        required: bool,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.window.use_material();
        self.scene.begin_onboarding(applied.clone(), required);
        self.show(graphics)
    }

    fn show(&mut self, graphics: &mut DeviceState) -> Result<(), AppError> {
        let _ = self.scene.set_dpi(self.window.dpi());
        let (width, height) = self.window.client_size()?;
        let size =
            SettingsSize::new(width, height).ok_or(AppError::InvalidSettingsScene)?;
        if let Some(surface) = &mut self.surface {
            surface.resize(size)?;
        } else {
            let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
            self.surface = Some(SettingsCompositionSurfaceState::create(
                device,
                self.window.handle(),
                size,
            )?);
        }
        self.window.show()?;
        self.visible = true;
        self.render(graphics)
    }

    pub(super) fn hide(&mut self) {
        self.window.hide();
        self.window.set_pointer_cursor(PointerCursor::Arrow);
        self.visible = false;
        self.dragging_slider = None;
    }

    pub(super) fn drain_events(&mut self) -> Vec<SettingsEvent> {
        self.window.drain_events().collect()
    }

    pub(super) fn start_update_check(&mut self) -> Result<bool, UpdateStartError> {
        let started = self.update_checker.start_check(env!("CARGO_PKG_VERSION"))?;
        if started {
            let _ = self
                .scene
                .set_update_activity(SettingsUpdateActivity::Checking);
        }
        Ok(started)
    }

    pub(super) fn start_update_download(
        &mut self,
        release: Release,
    ) -> Result<bool, UpdateStartError> {
        let started = self.update_checker.start_download(release)?;
        if started {
            let _ = self
                .scene
                .set_update_activity(SettingsUpdateActivity::Installing);
        }
        Ok(started)
    }

    pub(super) fn drain_update_results(&self) -> Vec<UpdateResult> {
        self.update_checker.drain().collect()
    }

    pub(super) fn render(&mut self, graphics: &mut DeviceState) -> Result<(), AppError> {
        if !self.visible {
            return Ok(());
        }
        let surface = self
            .surface
            .as_mut()
            .ok_or(AppError::InvalidSettingsScene)?;
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

    pub(super) fn resize(
        &mut self,
        graphics: &mut DeviceState,
        width: u32,
        height: u32,
    ) -> Result<(), AppError> {
        let Some(size) = SettingsSize::new(width, height) else {
            return Ok(());
        };
        let Some(surface) = &mut self.surface else {
            return Ok(());
        };
        match surface.resize(size) {
            Ok(()) => self.render(graphics),
            Err(SurfaceError::DeviceLost(_)) => {
                let _ = graphics.poll();
                graphics.recover()?;
                let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
                surface.recover(device)?;
                surface.resize(size)?;
                self.render(graphics)
            }
            Err(error) => Err(error.into()),
        }
    }
}
