use lotus_core::settings::DockSettings;
use lotus_search::scene::LauncherScene;
use lotus_ui::embedded_icon::EmbeddedIcon;
use lotus_ui::frame::{FramePass, ScheduledSurface};
use lotus_windows::graphics::launcher_surface::LauncherCompositionSurfaceState;
use lotus_windows::graphics::{DeviceState, GraphicsDevice, SurfaceError, SurfaceSize};
use lotus_windows::window::{DismissRequest, DockWindow, SearchEvent, SearchWindow};

use crate::app::AppError;
use crate::app::surface_render::frame_outcome;

/// Owns the native Search window and its optional composition surface.
///
/// Launcher scene, search interaction, and presentation state remain in
/// `LauncherRuntime`. This type only commits that state to the native surface.
pub(super) struct LauncherSurface {
    window: SearchWindow,
    surface: Option<ScheduledSurface<LauncherCompositionSurfaceState>>,
    visible: bool,
    last_applied_size: Option<SurfaceSize>,
    child_popup_open: bool,
}

impl LauncherSurface {
    pub(super) const fn new(window: SearchWindow) -> Self {
        Self {
            window,
            surface: None,
            visible: false,
            last_applied_size: None,
            child_popup_open: false,
        }
    }

    pub(super) const fn is_visible(&self) -> bool {
        self.visible
    }

    pub(super) fn has_graphics_surface(&self) -> bool {
        self.surface.is_some()
    }

    pub(super) fn diagnostic_state(&self) -> (bool, bool, bool) {
        let surface = self.surface.as_ref();
        (
            surface.is_some_and(ScheduledSurface::is_dirty),
            surface.is_some_and(ScheduledSurface::is_animating),
            self.visible,
        )
    }

    pub(super) fn dpi(&self) -> u32 {
        self.window.dpi()
    }

    pub(super) fn open_window(
        &mut self,
        dock: &DockWindow,
        size: lotus_search::scene::LauncherSize,
    ) -> Result<(), AppError> {
        self.window
            .open(dock.handle(), size.width(), size.height())?;
        Ok(())
    }

    pub(super) fn commit_open(
        &mut self,
        size: lotus_search::scene::LauncherSize,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let size = surface_size(size)?;
        if let Some(surface) = &mut self.surface {
            resize_surface(graphics, surface.value_mut(), size)?;
        } else {
            let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
            self.surface = Some(ScheduledSurface::new(
                LauncherCompositionSurfaceState::create(
                    device,
                    self.window.handle(),
                    size,
                )?,
            ));
        }
        self.visible = true;
        self.last_applied_size = Some(size);
        Ok(())
    }

    pub(super) fn correct_open_geometry(
        &self,
        dock: &DockWindow,
        desired: lotus_search::scene::LauncherSize,
    ) -> Result<(), AppError> {
        self.window
            .apply_geometry(dock.handle(), desired.width(), desired.height())?;
        Ok(())
    }

    pub(super) fn apply_geometry(
        &mut self,
        dock: &DockWindow,
        desired: lotus_search::scene::LauncherSize,
        graphics: &mut DeviceState,
        force_placement: bool,
    ) -> Result<(), AppError> {
        let size = surface_size(desired)?;
        let size_changed = self.last_applied_size != Some(size);
        if size_changed || force_placement {
            self.window
                .apply_geometry(dock.handle(), desired.width(), desired.height())?;
        }
        if size_changed && let Some(surface) = &mut self.surface {
            resize_surface(graphics, surface.value_mut(), size)?;
        }
        self.last_applied_size = Some(size);
        Ok(())
    }

    pub(super) fn resize(
        &mut self,
        graphics: &mut DeviceState,
        width: u32,
        height: u32,
    ) -> Result<bool, AppError> {
        let Some(size) = SurfaceSize::new(width, height) else {
            return Ok(false);
        };
        if self.last_applied_size == Some(size) {
            return Ok(false);
        }
        let Some(surface) = &mut self.surface else {
            return Ok(false);
        };
        resize_surface(graphics, surface.value_mut(), size)?;
        self.last_applied_size = Some(size);
        Ok(true)
    }

    pub(super) fn hide(&mut self) {
        self.window.hide();
        self.visible = false;
        self.last_applied_size = None;
        self.child_popup_open = false;
        self.stop_animation();
    }

    pub(super) fn suspend_for_child_popup(&mut self) {
        self.child_popup_open = true;
        self.window.suspend_for_child_popup();
    }

    pub(super) fn resume_after_child_popup_if_visible(&mut self, restore_focus: bool) {
        self.child_popup_open = false;
        if self.visible {
            self.window.resume_after_child_popup(restore_focus);
        }
    }

    pub(super) fn focus_if_visible(&mut self) {
        if self.visible && !self.child_popup_open {
            let _ = self.window.focus();
        }
    }

    pub(super) fn accepts_dismiss(&self, request: DismissRequest) -> bool {
        self.window.accepts_dismiss(request)
    }

    pub(super) fn use_material(&self, settings: &DockSettings) {
        lotus_windows::backdrop::apply_search_settings(self.window.handle(), settings);
    }

    pub(super) fn invalidate(&mut self) {
        if let Some(surface) = &mut self.surface {
            surface.invalidate();
        }
    }

    pub(super) fn is_animating(&self) -> bool {
        self.surface
            .as_ref()
            .is_some_and(ScheduledSurface::is_animating)
    }

    pub(super) fn stop_animation(&mut self) {
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
        scene: &LauncherScene<EmbeddedIcon>,
    ) -> Result<(), AppError> {
        let surface = self
            .surface
            .as_mut()
            .ok_or(AppError::InvalidLauncherScene)?;
        pass.render(surface, |surface| {
            let content = scene.render_presentation(EmbeddedIcon::FluentSearch);
            let motion = scene.presentation();
            let result = surface.render_scene(
                &content,
                motion.scale,
                motion.opacity,
                scene.needs_animation(),
            );
            frame_outcome(graphics, result)
        })
    }

    pub(super) fn drain_events(&mut self) -> Vec<SearchEvent> {
        self.window.drain_events().collect()
    }

    pub(super) fn has_pending_events(&self) -> bool {
        self.window.has_pending_events()
    }
}

fn surface_size(size: lotus_search::scene::LauncherSize) -> Result<SurfaceSize, AppError> {
    SurfaceSize::new(size.width(), size.height()).ok_or(AppError::ZeroSizedSurface)
}

fn resize_surface(
    graphics: &mut DeviceState,
    surface: &mut LauncherCompositionSurfaceState,
    size: SurfaceSize,
) -> Result<(), AppError> {
    match surface.resize(size) {
        Ok(()) => Ok(()),
        Err(SurfaceError::DeviceLost(loss)) => {
            graphics.mark_lost(loss);
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}
