use lotus_ui::frame::FramePass;
use lotus_windows::graphics::DeviceState;
use lotus_windows::window::DockWindow;

use super::ModuleHost;
use crate::app::dock::DockRuntime;
use crate::app::{AppError, PresentationSurface};

impl ModuleHost {
    pub(in crate::app) fn prepare_input_presentations(
        &mut self,
        dock: &DockWindow,
        dock_model: &DockRuntime,
        graphics: &mut DeviceState,
    ) {
        if self.lifecycle.search_takeover_requested() {
            self.applications.refresh_launcher_catalog_if_stale();
            let catalog = self.applications.prepare_launcher_catalog(
                dock_model.items(),
                &dock_model.settings().hidden_executables,
            );
            if let Err(error) = self.launcher.prepare_presentation(
                dock_model,
                catalog,
                dock.dpi(),
                graphics,
            ) {
                self.lifecycle.set_search_presentation_ready(false);
                lotus_windows::diagnostics::record_error("search.prepare", &error);
            }
        }

        if self.lifecycle.switcher_takeover_requested()
            && let Err(error) = self.switcher.prepare_presentation(graphics)
        {
            self.lifecycle.set_switcher_presentation_ready(false);
            lotus_windows::diagnostics::record_error("alt_tab.prepare", &error);
        }
    }

    pub(in crate::app) fn diagnostic_surface_masks(&self) -> (u32, u32, u32) {
        let states = [
            (
                PresentationSurface::Launcher.bit(),
                self.launcher.diagnostic_surface_state(),
            ),
            (
                PresentationSurface::ContextMenu.bit(),
                self.context_menu.diagnostic_surface_state(),
            ),
            (
                PresentationSurface::Settings.bit(),
                self.settings.diagnostic_surface_state(),
            ),
            (
                PresentationSurface::Switcher.bit(),
                self.switcher.diagnostic_surface_state(),
            ),
            (
                PresentationSurface::Status.bit(),
                self.status.diagnostic_surface_masks(),
            ),
            (
                PresentationSurface::Monitors.bit(),
                self.monitors.diagnostic_surface_masks(),
            ),
        ];
        states.into_iter().fold(
            (0, 0, 0),
            |(dirty, animating, visible), (bit, (is_dirty, is_animating, is_visible))| {
                (
                    dirty | (u32::from(is_dirty) * bit),
                    animating | (u32::from(is_animating) * bit),
                    visible | (u32::from(is_visible) * bit),
                )
            },
        )
    }

    pub(in crate::app) fn render_frames(
        &mut self,
        pass: &mut FramePass,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        match self.launcher.render_frame(pass, graphics) {
            Ok(()) => self
                .lifecycle
                .set_search_presentation_ready(self.launcher.presentation_ready()),
            Err(error) => {
                self.lifecycle.set_search_presentation_ready(false);
                lotus_windows::diagnostics::record_error("search.render", &error);
                self.hide_launcher();
            }
        }
        if let Err(error) = self.context_menu.render_frame(pass, graphics) {
            lotus_windows::diagnostics::record_error("popup.render", &error);
            self.hide_context_menu();
        }
        if let Err(error) = self.settings.render_frame(pass, graphics) {
            lotus_windows::diagnostics::record_error("settings.render", &error);
            self.settings.hide();
        }
        match self.switcher.render_frame(pass, graphics) {
            Ok(()) => self
                .lifecycle
                .set_switcher_presentation_ready(self.switcher.presentation_ready()),
            Err(error) => {
                self.lifecycle.set_switcher_presentation_ready(false);
                lotus_windows::diagnostics::record_error("alt_tab.render", &error);
                self.switcher.abandon();
            }
        }
        self.status.render_frame(pass, graphics)?;
        self.monitors.render_frame(pass, graphics)
    }

    pub(in crate::app) fn invalidate_surfaces(&mut self) {
        self.launcher.invalidate();
        self.settings.invalidate();
        self.context_menu.invalidate();
        self.switcher.invalidate();
        self.status.invalidate();
        self.monitors.invalidate();
    }

    pub(in crate::app) fn recover_surfaces(
        &mut self,
        device: &lotus_windows::graphics::GraphicsDevice,
    ) -> Result<(), AppError> {
        self.launcher.recover_surface(device)?;
        self.context_menu.recover_surface(device)?;
        self.settings.recover_surface(device)?;
        self.switcher.recover_surface(device)?;
        self.status.recover_surfaces(device)?;
        self.monitors.recover_surfaces(device)
    }
}
