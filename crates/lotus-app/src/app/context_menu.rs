use lotus_core::settings::{DockSettings, WindowPickerStyle};
use lotus_dock::popup::PopupSymbol;
use lotus_settings::appearance::theme_for;
use lotus_ui::frame::{FrameOutcome, FramePass, ScheduledSurface};
use lotus_ui::geometry::NonZeroPhysicalSize;
use lotus_ui::theme::Theme;
use lotus_windows::dwm_thumbnail::DwmThumbnailHost;
use lotus_windows::graphics::assets::SvgAsset;
use lotus_windows::graphics::context_menu_surface::ContextMenuCompositionSurfaceState;
use lotus_windows::graphics::surface::FrameResult;
use lotus_windows::graphics::{DeviceState, SurfaceError};
use lotus_windows::window::{
    ContextMenuEvent, ContextMenuWindow, PopupAlignment, SignedPoint,
};

use crate::app::AppError;
use crate::app::visuals::{ContextMenuScene, NativePickerWindow};

pub(super) struct ContextMenuRuntime {
    pub(super) window: ContextMenuWindow,
    pub(super) scene: ContextMenuScene,
    pub(super) surface: Option<ScheduledSurface<ContextMenuCompositionSurfaceState>>,
    pub(super) visible: bool,
    thumbnails: DwmThumbnailHost,
    theme: Theme,
    anchor: Option<SignedPoint>,
    alignment: PopupAlignment,
    picker_identity: Option<String>,
}

impl ContextMenuRuntime {
    pub(super) fn new(window: ContextMenuWindow, theme: &Theme) -> Result<Self, AppError> {
        let mut scene = ContextMenuScene::system(window.dpi())
            .ok_or(AppError::InvalidContextMenuScene)?;
        let _ = scene.set_theme(*theme);
        Ok(Self {
            thumbnails: DwmThumbnailHost::new(window.handle()),
            window,
            scene,
            surface: None,
            visible: false,
            theme: *theme,
            anchor: None,
            alignment: PopupAlignment::Center,
            picker_identity: None,
        })
    }

    pub(super) fn apply_settings(&mut self, settings: &DockSettings) {
        let _ = self.scene.set_theme(theme_for(settings));
        self.theme = theme_for(settings);
        lotus_windows::backdrop::apply_popup_settings(self.window.handle(), settings);
    }

    pub(super) fn open(
        &mut self,
        anchor: SignedPoint,
        alignment: PopupAlignment,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let mut scene = ContextMenuScene::system(self.window.dpi())
            .ok_or(AppError::InvalidContextMenuScene)?;
        let _ = scene.set_theme(self.theme);
        self.scene = scene;
        self.picker_identity = None;
        self.alignment = alignment;
        self.open_current(anchor, graphics)
    }

    pub(super) fn open_app(
        &mut self,
        anchor: SignedPoint,
        source_index: usize,
        running_windows: usize,
        pinned: bool,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let mut scene =
            ContextMenuScene::app(self.window.dpi(), source_index, running_windows, pinned)
                .ok_or(AppError::InvalidContextMenuScene)?;
        let _ = scene.set_theme(self.theme);
        self.scene = scene;
        self.picker_identity = None;
        self.alignment = PopupAlignment::Center;
        self.open_current(anchor, graphics)
    }

    pub(super) fn open_power(
        &mut self,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let anchor = self.anchor.ok_or(AppError::InvalidContextMenuScene)?;
        let mut scene = ContextMenuScene::power(self.window.dpi())
            .ok_or(AppError::InvalidContextMenuScene)?;
        let _ = scene.set_theme(self.theme);
        self.scene = scene;
        self.picker_identity = None;
        self.open_current(anchor, graphics)
    }

    pub(super) fn open_picker(
        &mut self,
        anchor: SignedPoint,
        source_index: usize,
        identity: String,
        style: WindowPickerStyle,
        windows: Vec<NativePickerWindow>,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let mut scene =
            ContextMenuScene::picker(self.window.dpi(), source_index, style, windows)
                .ok_or(AppError::InvalidContextMenuScene)?;
        let _ = scene.set_theme(self.theme);
        self.scene = scene;
        self.picker_identity = Some(identity);
        self.alignment = PopupAlignment::Center;
        self.open_current(anchor, graphics)
    }

    pub(super) fn picker_identity(&self) -> Option<&str> {
        self.picker_identity.as_deref()
    }

    pub(super) fn replace_picker(
        &mut self,
        source_index: usize,
        style: WindowPickerStyle,
        windows: Vec<NativePickerWindow>,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if windows.is_empty() {
            self.hide();
            return Ok(());
        }
        let Some(anchor) = self.anchor else {
            self.hide();
            return Ok(());
        };
        let mut scene =
            ContextMenuScene::picker(self.window.dpi(), source_index, style, windows)
                .ok_or(AppError::InvalidContextMenuScene)?;
        let _ = scene.set_theme(self.theme);
        self.scene = scene;
        self.prepare_surface(anchor, graphics)?;
        self.invalidate();
        Ok(())
    }

    fn open_current(
        &mut self,
        anchor: SignedPoint,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.anchor = Some(anchor);
        self.prepare_surface(anchor, graphics)?;
        self.visible = true;
        self.invalidate();
        self.window.show();
        Ok(())
    }

    fn prepare_surface(
        &mut self,
        anchor: SignedPoint,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let mut desired = self.scene.desired_size();
        let dpi = self.window.prepare_at(anchor, self.alignment, desired)?;
        if self.scene.set_dpi(dpi) {
            desired = self.scene.desired_size();
            let _dpi = self.window.prepare_at(anchor, self.alignment, desired)?;
        }
        if let Some(surface) = &mut self.surface {
            surface.value_mut().resize(desired)?;
        } else {
            let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
            self.surface = Some(ScheduledSurface::new(
                ContextMenuCompositionSurfaceState::create(
                    device,
                    self.window.handle(),
                    desired,
                )?,
            ));
        }
        Ok(())
    }

    pub(super) fn hide(&mut self) {
        if self.visible {
            self.window.hide();
            self.visible = false;
            self.thumbnails.clear();
            self.anchor = None;
            self.picker_identity = None;
            let _ = self.scene.pointer_left();
            if let Some(surface) = &mut self.surface {
                surface.stop_animation();
            }
        }
    }

    pub(super) fn invalidate(&mut self) {
        if let Some(surface) = &mut self.surface {
            surface.invalidate();
        }
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
            .ok_or(AppError::InvalidContextMenuScene)?;
        let presentation = self.scene.presentation(popup_asset);
        pass.render(surface, |surface| {
            match surface.render_scene(&presentation) {
                Ok(FrameResult::Presented { .. }) => {
                    self.thumbnails.reconcile(&self.scene.picker_previews());
                    Ok(FrameOutcome::complete(false))
                }
                Ok(FrameResult::TargetRecreated) => Ok(FrameOutcome::Retry),
                Err(SurfaceError::DeviceLost(_)) => {
                    let _ = graphics.poll();
                    graphics.recover()?;
                    let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
                    surface.recover(device)?;
                    match surface.render_scene(&presentation)? {
                        FrameResult::Presented { .. } => {
                            self.thumbnails.reconcile(&self.scene.picker_previews());
                            Ok(FrameOutcome::complete(false))
                        }
                        FrameResult::TargetRecreated => Ok(FrameOutcome::Retry),
                    }
                }
                Err(error) => Err(error.into()),
            }
        })
    }

    pub(super) fn resize(&mut self, width: u32, height: u32) -> Result<(), AppError> {
        let Some(size) = NonZeroPhysicalSize::new(width, height) else {
            return Ok(());
        };
        if let Some(surface) = &mut self.surface {
            surface.value_mut().resize(size)?;
        }
        Ok(())
    }

    pub(super) fn drain_events(&mut self) -> Vec<ContextMenuEvent> {
        self.window.drain_events().collect()
    }
}

const fn popup_asset(symbol: PopupSymbol) -> SvgAsset {
    match symbol {
        PopupSymbol::Power => SvgAsset::FluentPower,
        PopupSymbol::Lock => SvgAsset::FluentLock,
        PopupSymbol::Restart => SvgAsset::FluentRestart,
        PopupSymbol::Settings => SvgAsset::FluentSettings,
        PopupSymbol::Quit | PopupSymbol::Close => SvgAsset::FluentDismiss,
        PopupSymbol::Open | PopupSymbol::Image => SvgAsset::FluentOpen,
        PopupSymbol::Pin => SvgAsset::FluentPin,
        PopupSymbol::Unpin => SvgAsset::FluentPinOff,
        PopupSymbol::Previous => SvgAsset::FluentPrevious,
        PopupSymbol::Next => SvgAsset::FluentNext,
    }
}
