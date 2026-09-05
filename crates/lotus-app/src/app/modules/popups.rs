use lotus_core::application::WindowApplicationAssignments;
use lotus_core::window::{WindowId, WindowInfo};
use lotus_windows::WindowHandle;
use lotus_windows::graphics::{DeviceState, GraphicsDeviceHealth};
use lotus_windows::search_catalog::ApplicationCatalogSnapshot;
use lotus_windows::window::{DismissReason, PopupAlignment, SignedPoint};

use super::ModuleHost;
use crate::app::AppError;
use crate::app::context_menu::{
    AppMenuOptions, ContextMenuEventOutcome, PopupEvent, PopupOwner,
};
use crate::app::dock::DockRuntime;

impl ModuleHost {
    pub(in crate::app) fn switcher_owns_window(&self, window: WindowHandle) -> bool {
        self.switcher.window.handle() == window
    }

    pub(in crate::app) fn record_switcher_foreground(
        &mut self,
        foreground: Option<WindowId>,
        windows: &[WindowInfo],
    ) {
        self.switcher.record_foreground(foreground.and_then(|id| {
            windows
                .iter()
                .find(|window| window.id == id)
                .map(WindowInfo::key)
        }));
    }

    pub(in crate::app) fn reconcile_switcher_windows(
        &mut self,
        windows: &[WindowInfo],
        application_catalog: std::sync::Arc<ApplicationCatalogSnapshot>,
        application_assignments: &WindowApplicationAssignments,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.switcher.reconcile_windows(
            windows,
            application_catalog,
            application_assignments,
            graphics,
        )
    }

    pub(in crate::app) fn drain_switcher_events(
        &mut self,
        graphics: &mut DeviceState,
    ) -> bool {
        let events = self.switcher.drain_events();
        let had_events = !events.is_empty();
        for event in events {
            if let Err(error) = self.switcher.handle_window_event(event, graphics) {
                if error.mark_graphics_lost(graphics)
                    || graphics.health() == GraphicsDeviceHealth::Lost
                {
                    continue;
                }
                lotus_windows::diagnostics::record_error("alt_tab.event", &error);
                self.switcher.abandon();
            }
        }
        had_events
    }

    pub(in crate::app) fn has_pending_switcher_events(&self) -> bool {
        self.switcher.window.has_pending_events()
    }

    pub(in crate::app) fn drain_context_menu_events(&mut self) -> Vec<PopupEvent> {
        self.context_menu.drain_events()
    }

    pub(in crate::app) fn handle_context_menu_event(
        &mut self,
        event: PopupEvent,
    ) -> Result<ContextMenuEventOutcome, AppError> {
        self.context_menu.handle_event(event)
    }

    pub(in crate::app) fn hide_context_menu(&mut self) {
        let owner = self.context_menu.hide();
        self.resume_popup_parent(owner, None);
    }

    pub(in crate::app) fn open_context_menu(
        &mut self,
        anchor: SignedPoint,
        alignment: PopupAlignment,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.context_menu.open(anchor, alignment, graphics)
    }

    pub(in crate::app) fn open_application_context_menu(
        &mut self,
        anchor: SignedPoint,
        source_index: usize,
        shift_held: bool,
        dock_model: &DockRuntime,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let Some(item) = dock_model.item(source_index) else {
            return Ok(());
        };
        self.context_menu.open_app(
            anchor,
            AppMenuOptions {
                identity: item.id.clone(),
                running_windows: item.windows.len(),
                pinned: item.is_pinned,
                shift_held,
            },
            graphics,
        )
    }

    pub(in crate::app) fn open_search_file_location_menu(
        &mut self,
        anchor: SignedPoint,
        path: String,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.launcher.suspend_for_child_popup();
        if let Err(error) = self.context_menu.open_file_location(anchor, path, graphics) {
            self.launcher.resume_after_child_popup_if_visible(true);
            return Err(error);
        }
        Ok(())
    }

    pub(in crate::app) fn open_window_picker(
        &mut self,
        anchor: SignedPoint,
        source_index: usize,
        dock_model: &mut DockRuntime,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let foreground = lotus_windows::activation::foreground_window()
            .and_then(|id| dock_model.tracked_key_for_window_id(id));
        let entries = dock_model.picker_windows(source_index, foreground);
        let identity = dock_model
            .item(source_index)
            .map(|item| item.id.clone())
            .unwrap_or_default();
        let style = dock_model.settings().window_picker_style;
        self.context_menu
            .open_picker(anchor, identity, style, entries, graphics)
    }

    pub(in crate::app) fn open_power_menu(
        &mut self,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.context_menu.open_power(graphics)
    }

    pub(in crate::app) fn reconcile_visible_window_picker(
        &mut self,
        dock_model: &mut DockRuntime,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let Some(identity) = self.context_menu.picker_identity().map(str::to_owned) else {
            return Ok(());
        };
        let Some(source_index) = dock_model.source_index(&identity) else {
            lotus_windows::diagnostics::record_diagnostic(
                "activation.picker_entries_pruned",
                "window picker source disappeared during snapshot reconciliation",
            );
            self.hide_context_menu();
            return Ok(());
        };
        let foreground = lotus_windows::activation::foreground_window()
            .and_then(|id| dock_model.tracked_key_for_window_id(id));
        let windows = dock_model.picker_windows(source_index, foreground);
        if windows.is_empty() {
            lotus_windows::diagnostics::record_diagnostic(
                "activation.picker_entries_pruned",
                "all window picker entries disappeared during snapshot reconciliation",
            );
        }
        let style = dock_model.settings().window_picker_style;
        self.context_menu.replace_picker(style, windows, graphics)
    }

    pub(in crate::app) fn complete_popup_action(&mut self, owner: Option<PopupOwner>) {
        if owner == Some(PopupOwner::Search) {
            self.launcher.resume_after_child_popup_if_visible(true);
            self.launcher.focus_if_visible();
        }
    }

    pub(in crate::app) fn resume_popup_parent(
        &mut self,
        owner: Option<PopupOwner>,
        reason: Option<DismissReason>,
    ) {
        if matches!(owner, Some(PopupOwner::Search)) {
            if matches!(
                reason,
                Some(DismissReason::Deactivated | DismissReason::OutsideClick)
            ) {
                self.hide_launcher();
                return;
            }
            let restore_focus = reason == Some(DismissReason::Escape);
            self.launcher
                .resume_after_child_popup_if_visible(restore_focus);
            if restore_focus {
                self.launcher.focus_if_visible();
            }
        }
    }
}
