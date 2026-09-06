use lotus_core::settings::DockSettings;
use lotus_dock::popup::PopupSymbol;
use lotus_ui::embedded_icon::EmbeddedIcon;
use lotus_ui::frame::{FramePass, ScheduledSurface};
use lotus_ui::geometry::NonZeroPhysicalSize;
use lotus_windows::dwm_thumbnail::DwmThumbnailHost;
use lotus_windows::graphics::context_menu_surface::ContextMenuCompositionSurfaceState;
use lotus_windows::graphics::surface::FrameResult;
use lotus_windows::graphics::{DeviceState, GraphicsDevice};
use lotus_windows::window::{
    ContextMenuEvent, ContextMenuWindow, DismissRequest, PopupAlignment, SignedPoint,
};

use crate::app::AppError;
use crate::app::surface_render::frame_outcome;
use crate::app::visuals::ContextMenuScene;

pub(super) struct ContextMenuSurface {
    window: ContextMenuWindow,
    surface: Option<ScheduledSurface<ContextMenuCompositionSurfaceState>>,
    thumbnails: DwmThumbnailHost,
}

impl ContextMenuSurface {
    pub(super) fn new(window: ContextMenuWindow) -> Self {
        Self {
            thumbnails: DwmThumbnailHost::new(window.handle()),
            window,
            surface: None,
        }
    }

    fn handle(&self) -> lotus_windows::WindowHandle {
        self.window.handle()
    }

    pub(super) fn dpi(&self) -> u32 {
        self.window.dpi()
    }

    pub(super) fn diagnostic_state(&self) -> (bool, bool) {
        let surface = self.surface.as_ref();
        (
            surface.is_some_and(ScheduledSurface::is_dirty),
            surface.is_some_and(ScheduledSurface::is_animating),
        )
    }

    pub(super) fn prepare(
        &mut self,
        anchor: SignedPoint,
        alignment: PopupAlignment,
        scene: &mut ContextMenuScene,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if self.surface.is_none() && graphics.ready().is_none() {
            return Err(AppError::GraphicsUnavailable);
        }

        let mut desired = scene.desired_size();
        let dpi = self.window.prepare_at(anchor, alignment, desired)?;
        if scene.set_dpi(dpi) {
            desired = scene.desired_size();
            let _dpi = self.window.prepare_at(anchor, alignment, desired)?;
        }

        if let Some(surface) = &mut self.surface {
            surface.value_mut().resize(desired)?;
        } else {
            let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
            self.surface = Some(ScheduledSurface::new(
                ContextMenuCompositionSurfaceState::create(device, self.handle(), desired)?,
            ));
        }
        Ok(())
    }

    pub(super) fn show(&mut self) {
        self.window.show();
    }

    pub(super) fn hide(&mut self) {
        self.window.hide();
        self.thumbnails.clear();
        self.stop_animation();
    }

    pub(super) fn apply_settings(&self, settings: &DockSettings) {
        lotus_windows::backdrop::apply_context_menu_settings(self.handle(), settings);
    }

    pub(super) fn invalidate(&mut self) {
        if let Some(surface) = &mut self.surface {
            surface.invalidate();
        }
    }

    fn stop_animation(&mut self) {
        if let Some(surface) = &mut self.surface {
            surface.stop_animation();
        }
    }

    pub(super) fn recover(&mut self, device: &GraphicsDevice) -> Result<(), AppError> {
        if let Some(surface) = &mut self.surface {
            surface.value_mut().recover(device)?;
        }
        Ok(())
    }

    pub(super) fn render_frame(
        &mut self,
        pass: &mut FramePass,
        graphics: &mut DeviceState,
        scene: &ContextMenuScene,
        visible: bool,
    ) -> Result<(), AppError> {
        if !visible {
            self.stop_animation();
            return Ok(());
        }

        let surface = self
            .surface
            .as_mut()
            .ok_or(AppError::InvalidContextMenuScene)?;
        pass.render(surface, |surface| {
            let presentation = scene.presentation(popup_asset);
            let result = surface.render_scene(&presentation);
            if matches!(&result, Ok(FrameResult::Presented { .. })) {
                self.thumbnails.reconcile(&scene.picker_previews());
            }
            frame_outcome(graphics, result).map(|frame| frame.with_animation_allowed(false))
        })
    }

    pub(super) fn resize(&mut self, width: u32, height: u32) -> Result<(), AppError> {
        let Some(size) = NonZeroPhysicalSize::new(width, height) else {
            return Ok(());
        };
        self.resize_to_scene(size)
    }

    pub(super) fn resize_to_scene(
        &mut self,
        size: NonZeroPhysicalSize,
    ) -> Result<(), AppError> {
        if let Some(surface) = &mut self.surface {
            surface.value_mut().resize(size)?;
        }
        Ok(())
    }

    pub(super) fn drain_events_up_to(
        &mut self,
        limit: usize,
    ) -> impl Iterator<Item = ContextMenuEvent> + '_ {
        self.window.drain_events_up_to(limit)
    }

    pub(super) fn has_pending_events(&self) -> bool {
        self.window.has_pending_events()
    }

    pub(super) fn interaction_generation(&self) -> usize {
        self.window.interaction_generation()
    }

    pub(super) fn accepts_dismiss(&self, request: DismissRequest) -> bool {
        self.window.accepts_dismiss(request)
    }
}

const fn popup_asset(symbol: PopupSymbol) -> EmbeddedIcon {
    match symbol {
        PopupSymbol::Power => EmbeddedIcon::FluentPower,
        PopupSymbol::Lock => EmbeddedIcon::FluentLock,
        PopupSymbol::Restart => EmbeddedIcon::FluentRestart,
        PopupSymbol::Settings => EmbeddedIcon::FluentSettings,
        PopupSymbol::Quit | PopupSymbol::Close => EmbeddedIcon::FluentDismiss,
        PopupSymbol::Open | PopupSymbol::Image => EmbeddedIcon::FluentOpen,
        PopupSymbol::Pin => EmbeddedIcon::FluentPin,
        PopupSymbol::Unpin => EmbeddedIcon::FluentPinOff,
        PopupSymbol::Previous => EmbeddedIcon::FluentPrevious,
        PopupSymbol::Next => EmbeddedIcon::FluentNext,
    }
}
