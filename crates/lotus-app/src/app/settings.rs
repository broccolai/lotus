use lotus_core::settings::DockSettings;
use lotus_ui::frame::{FrameOutcome, FramePass, ScheduledSurface};
use lotus_windows::custom_image::CustomImageCache;
use lotus_windows::graphics::settings_surface::SettingsCompositionSurfaceState;
use lotus_windows::graphics::surface::FrameResult;
use lotus_windows::graphics::{
    DeviceState, SettingsScene, SettingsSize, SettingsSlider, SettingsUpdateActivity,
    SurfaceError,
};
use lotus_windows::interaction::PointerCursor;
use lotus_windows::native_icon::NativeIconCache;
use lotus_windows::update::{Release, UpdateChecker, UpdateResult, UpdateStartError};
use lotus_windows::window::{SettingsEvent, SettingsWindow};

use crate::app::AppError;

pub(super) struct SettingsRuntime {
    pub(super) window: SettingsWindow,
    pub(super) scene: SettingsScene,
    pub(super) surface: Option<ScheduledSurface<SettingsCompositionSurfaceState>>,
    pub(super) visible: bool,
    pub(super) dragging_slider: Option<SettingsSlider>,
    pub(super) native_icons: NativeIconCache,
    pub(super) custom_images: CustomImageCache,
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
            native_icons: NativeIconCache::default(),
            custom_images: CustomImageCache::default(),
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
        self.window.use_material(applied);
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
        self.window.use_material(applied);
        self.scene.begin_onboarding(applied.clone(), required);
        self.show(graphics)
    }

    fn show(&mut self, graphics: &mut DeviceState) -> Result<(), AppError> {
        let _ = self.scene.set_dpi(self.window.dpi());
        let (width, height) = self.window.client_size()?;
        let size =
            SettingsSize::new(width, height).ok_or(AppError::InvalidSettingsScene)?;
        if let Some(surface) = &mut self.surface {
            surface.value_mut().resize(size)?;
        } else {
            let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
            self.surface = Some(ScheduledSurface::new(
                SettingsCompositionSurfaceState::create(
                    device,
                    self.window.handle(),
                    size,
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
        self.window.set_pointer_cursor(PointerCursor::Arrow);
        self.visible = false;
        self.dragging_slider = None;
        if let Some(surface) = &mut self.surface {
            surface.stop_animation();
        }
    }

    pub(super) fn invalidate(&mut self) {
        if let Some(surface) = &mut self.surface {
            surface.invalidate();
        }
    }

    pub(super) fn drain_events(&mut self) -> Vec<SettingsEvent> {
        self.window.drain_events().collect()
    }

    pub(super) fn start_update_check(&mut self) -> Result<bool, UpdateStartError> {
        let started = self
            .update_checker
            .start_check(env!("CARGO_PKG_VERSION"), self.scene.draft().update_channel)?;
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

    pub(super) fn render_frame(
        &mut self,
        pass: &mut FramePass,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if !self.visible {
            if let Some(surface) = &mut self.surface {
                surface.stop_animation();
            }
            return Ok(());
        }
        let surface = self
            .surface
            .as_mut()
            .ok_or(AppError::InvalidSettingsScene)?;
        pass.render(surface, |surface| match surface.render_scene(&self.scene) {
            Ok(FrameResult::Presented { .. }) => Ok(FrameOutcome::complete(false)),
            Ok(FrameResult::TargetRecreated) => Ok(FrameOutcome::Retry),
            Err(SurfaceError::DeviceLost(_)) => {
                let _ = graphics.poll();
                graphics.recover()?;
                let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
                surface.recover(device)?;
                match surface.render_scene(&self.scene)? {
                    FrameResult::Presented { .. } => Ok(FrameOutcome::complete(false)),
                    FrameResult::TargetRecreated => Ok(FrameOutcome::Retry),
                }
            }
            Err(error) => Err(error.into()),
        })
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
        match surface.value_mut().resize(size) {
            Ok(()) => {
                self.invalidate();
                Ok(())
            }
            Err(SurfaceError::DeviceLost(_)) => {
                let _ = graphics.poll();
                graphics.recover()?;
                let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
                surface.value_mut().recover(device)?;
                surface.value_mut().resize(size)?;
                self.invalidate();
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }
}
