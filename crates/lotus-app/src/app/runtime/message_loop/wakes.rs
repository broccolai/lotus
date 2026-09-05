use lotus_windows::icon_hydrator::is_icon_hydration_wake;
use lotus_windows::media::is_media_wake;
use lotus_windows::search_catalog::is_search_catalog_wake;
use lotus_windows::taskbar_badges::is_taskbar_badge_wake;
use lotus_windows::update::is_update_wake;

use super::MessageLoop;
use crate::app::runtime::{present_dock_change, search_events, update_events};
use crate::app::{AppError, RuntimeServices};

#[derive(Clone, Copy)]
pub(super) struct WakeEvents {
    search_catalog: bool,
    update: bool,
    media: bool,
    badges: bool,
    icon_hydration: bool,
}

impl WakeEvents {
    pub(super) const fn any(self) -> bool {
        self.search_catalog
            || self.update
            || self.media
            || self.badges
            || self.icon_hydration
    }

    pub(super) fn from_message(runtime: &RuntimeServices<'_>, message: u32) -> Self {
        Self {
            search_catalog: is_search_catalog_wake(message),
            update: is_update_wake(message),
            media: is_media_wake(message),
            badges: runtime.taskbar_badges.is_some() && is_taskbar_badge_wake(message),
            icon_hydration: is_icon_hydration_wake(message),
        }
    }
}

impl MessageLoop<'_, '_> {
    pub(super) fn process_wakes(&mut self, wakes: WakeEvents) -> Result<bool, AppError> {
        let mut changed = false;
        let mut presented_size = self.dock_model.scene().desired_size();
        if wakes.update {
            update_events::handle_update_results(self.auxiliary, self.runtime.startup_mode);
            changed = true;
        }
        if wakes.badges
            && let Some(controller) = self.runtime.taskbar_badges
            && let Ok(snapshot) = controller.snapshot()
        {
            self.dock_model.set_notifications(snapshot);
            self.render_dock();
            changed = true;
        }
        if wakes.media && self.auxiliary.drain_media(self.dock_model) {
            present_dock_change(
                self.primary_dock,
                self.graphics,
                self.auxiliary,
                self.dock_model,
            )?;
            presented_size = self.dock_model.scene().desired_size();
            self.render_dock();
            changed = true;
        }
        if wakes.search_catalog {
            let catalog_changed = search_events::refresh_catalog(
                self.primary_dock,
                self.graphics,
                self.window_tracker.current_windows(),
                self.dock_model,
                self.auxiliary,
                &self.runtime.settings_persistence,
            )?;
            if catalog_changed {
                presented_size = self.dock_model.scene().desired_size();
                changed = true;
            }
        }
        if wakes.icon_hydration {
            let hydration = self.auxiliary.drain_hydrated_icons(self.dock_model)?;
            if hydration.dock_presentation_changed() {
                self.render_dock();
            }
            changed |= hydration.requests_frame();
        }

        if self.dock_model.scene().desired_size() != presented_size {
            present_dock_change(
                self.primary_dock,
                self.graphics,
                self.auxiliary,
                self.dock_model,
            )?;
            changed = true;
        }

        Ok(changed)
    }

    pub(super) fn handle_input_wake(&mut self) -> bool {
        self.auxiliary
            .handle_input_actions(
                self.primary_dock.window(),
                self.window_tracker,
                self.dock_model,
                self.graphics,
            )
            .requests_frame()
    }

    fn render_dock(&mut self) {
        self.primary_dock.invalidate();
    }
}

pub(super) use lotus_windows::input::is_input_wake;
