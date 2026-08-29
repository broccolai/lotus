use lotus_ui::frame::FramePass;
use lotus_windows::graphics::DeviceState;

use super::ModuleHost;
use crate::app::AppError;

impl ModuleHost {
    pub(in crate::app) fn diagnostic_surface_masks(&self) -> (u32, u32, u32) {
        let states = [
            self.launcher.diagnostic_surface_state(),
            self.context_menu.diagnostic_surface_state(),
            self.settings.diagnostic_surface_state(),
            self.switcher.diagnostic_surface_state(),
            self.status.diagnostic_surface_masks(),
            self.monitors.diagnostic_surface_masks(),
        ];
        states.into_iter().enumerate().fold(
            (0, 0, 0),
            |(dirty, animating, visible), (index, (is_dirty, is_animating, is_visible))| {
                let bit = 1_u32 << (index + 1);
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
        self.launcher.render_frame(pass, graphics)?;
        self.context_menu.render_frame(pass, graphics)?;
        self.settings.render_frame(pass, graphics)?;
        if let Err(error) = self.switcher.render_frame(pass, graphics) {
            lotus_windows::diagnostics::record_error("alt_tab.render", &error);
            self.switcher.abandon();
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
