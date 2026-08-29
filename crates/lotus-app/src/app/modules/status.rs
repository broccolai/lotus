use lotus_media::MediaHitTarget;
use lotus_windows::WindowHandle;
use lotus_windows::graphics::DeviceState;
use lotus_windows::window::StatusEvent;

use super::{ModuleHost, StatusZoneActivation};
use crate::app::AppError;
use crate::app::dock::DockRuntime;

impl ModuleHost {
    pub(in crate::app) fn refresh_media(&mut self, dock_model: &mut DockRuntime) -> bool {
        if self.modules.is_enabled(lotus_core::module::ModuleId::Media) {
            self.media.refresh(dock_model)
        } else {
            self.media.drain(dock_model)
        }
    }

    pub(in crate::app) fn drain_media(&mut self, dock_model: &mut DockRuntime) -> bool {
        self.media.drain(dock_model)
    }

    pub(in crate::app) fn activate_media(
        &mut self,
        target: MediaHitTarget,
        dock_model: &mut DockRuntime,
        owner: WindowHandle,
    ) {
        self.media.activate(target, dock_model, owner);
    }

    pub(in crate::app) fn set_status_visible(&mut self, visible: bool) {
        self.status.set_visible(visible);
    }

    pub(in crate::app) fn set_status_fullscreen_occluded(
        &mut self,
        occluded: bool,
    ) -> Result<(), AppError> {
        self.status.set_fullscreen_occluded(occluded)
    }

    pub(in crate::app) fn drain_status_events(&mut self) -> Vec<(usize, StatusEvent)> {
        self.status.drain_events()
    }

    pub(in crate::app) fn has_pending_window_events(&self) -> bool {
        self.launcher.has_pending_events()
            || self.context_menu.window.has_pending_events()
            || self.status.has_pending_events()
    }

    pub(in crate::app) fn handle_status_event(
        &mut self,
        zone_index: usize,
        event: StatusEvent,
        graphics: &mut DeviceState,
    ) -> Result<Option<StatusZoneActivation>, AppError> {
        self.status
            .handle_event(zone_index, event, graphics)
            .map(|activation| {
                activation.map(|(action, owner, anchor)| StatusZoneActivation {
                    action,
                    owner,
                    anchor,
                })
            })
    }
}
