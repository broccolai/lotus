use lotus_core::dock::DockItem;
use lotus_core::module::{ModuleId, ModuleSet};
use lotus_core::search::SearchUsage;
use lotus_core::settings::DockSettings;
use lotus_search::usage::SearchUsageStore;
use lotus_settings::appearance::theme_for;
use lotus_settings::scene::{SettingsApplicationRecord, SettingsScene};
use lotus_ui::frame::FramePass;
use lotus_windows::WindowHandle;
use lotus_windows::graphics::{DeviceState, GraphicsDeviceHealth};
use lotus_windows::icon_hydrator::{IconHydrationResult, IconHydrator};
use lotus_windows::input::{InputConfig, InputController};
use lotus_windows::search_catalog::{ApplicationCatalogSnapshot, SearchCatalogCache};
use lotus_windows::update::is_installed;
use lotus_windows::window::{
    ContextMenuEvent, DockWindow, PointerEvent, PopupAlignment, SettingsEvent, SignedPoint,
    WindowEvent,
};
use lotus_windows::window_tracker::WindowTracker;

use crate::app::AppError;
use crate::app::context_menu::{AppMenuOptions, ContextMenuRuntime};
use crate::app::dock::DockRuntime;
use crate::app::launcher::LauncherRuntime;
use crate::app::media::MediaRuntime;
use crate::app::monitors::MonitorDocks;
use crate::app::settings::{SettingsRuntime, application_records};
use crate::app::status::{AuxiliaryZoneAction, StatusRuntime};
use crate::app::switcher::SwitcherRuntime;

pub(super) struct ModuleHost {
    modules: ModuleRuntime,
    icon_hydrator: IconHydrator,
    applications: SearchCatalogCache,
    launcher: LauncherRuntime,
    settings: SettingsRuntime,
    context_menu: ContextMenuRuntime,
    media: MediaRuntime,
    status: StatusRuntime,
    monitors: MonitorDocks,
    switcher: SwitcherRuntime,
}

impl ModuleHost {
    pub(super) fn create(
        dock: &DockWindow,
        dock_model: &mut DockRuntime,
        usage: SearchUsage,
        usage_store: SearchUsageStore,
        modules_active: bool,
    ) -> Result<Self, AppError> {
        let search_window = dock.create_search_window()?;
        let icon_hydrator = IconHydrator::start()?;
        dock_model.attach_icon_hydrator(icon_hydrator.dock_client());
        lotus_windows::backdrop::apply_search_settings(
            search_window.handle(),
            dock_model.settings(),
        );
        let launcher = LauncherRuntime::new(
            search_window,
            dock_model.settings().clone(),
            &theme_for(dock_model.settings()),
            usage,
            usage_store,
            icon_hydrator.launcher_client(),
        );
        let settings = SettingsRuntime::new(
            dock.create_settings_window()?,
            dock_model.settings().clone(),
            is_installed().unwrap_or(false),
        )?;
        let context_menu_window = dock.create_context_menu_window()?;
        lotus_windows::backdrop::apply_context_menu_settings(
            context_menu_window.handle(),
            dock_model.settings(),
        );
        let context_menu = ContextMenuRuntime::new(
            context_menu_window,
            &theme_for(dock_model.settings()),
        )?;
        let switcher_window = dock.create_switcher_window()?;
        lotus_windows::backdrop::apply_popup_settings(
            switcher_window.handle(),
            dock_model.settings(),
        );
        let switcher = SwitcherRuntime::new(
            switcher_window,
            dock_model.settings(),
            &theme_for(dock_model.settings()),
            icon_hydrator.switcher_client(),
        );
        let status = StatusRuntime::new(
            [dock.create_status_window()?, dock.create_status_window()?],
            dock_model.settings(),
        )?;

        let mut host = Self {
            modules: ModuleRuntime::new(),
            icon_hydrator,
            applications: SearchCatalogCache::new(),
            launcher,
            settings,
            context_menu,
            media: MediaRuntime::new(false),
            status,
            monitors: MonitorDocks::new(),
            switcher,
        };
        host.reconcile(dock, dock_model.settings(), modules_active)?;
        Ok(host)
    }

    pub(super) fn input(&self) -> Option<&InputController> {
        self.modules.input()
    }

    pub(super) fn with_input_modules<R>(
        &mut self,
        handle: impl FnOnce(
            &InputController,
            &mut LauncherRuntime,
            &mut SwitcherRuntime,
            &SearchCatalogCache,
        ) -> R,
    ) -> Option<R> {
        let controller = self.modules.input()?;
        Some(handle(
            controller,
            &mut self.launcher,
            &mut self.switcher,
            &self.applications,
        ))
    }

    pub(super) fn reconcile(
        &mut self,
        dock: &DockWindow,
        settings: &DockSettings,
        active: bool,
    ) -> Result<(), AppError> {
        self.modules.reconcile(
            settings,
            active,
            &mut ModuleResources {
                dock,
                launcher: &mut self.launcher,
                switcher: &mut self.switcher,
                media: &mut self.media,
                status: &mut self.status,
            },
        )
    }

    pub(super) fn propagate_settings(
        &mut self,
        settings: &DockSettings,
        dock: &DockWindow,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.launcher.apply_settings(settings, dock, graphics)?;
        self.context_menu.apply_settings(settings);
        self.switcher.apply_settings(settings);
        Ok(())
    }

    pub(super) fn refresh_media(&mut self, dock_model: &mut DockRuntime) -> bool {
        if self.modules.is_enabled(ModuleId::Media) {
            self.media.refresh(dock_model)
        } else {
            self.media.drain(dock_model)
        }
    }

    pub(super) fn drain_media(&mut self, dock_model: &mut DockRuntime) -> bool {
        self.media.drain(dock_model)
    }

    pub(super) fn sync_status(
        &mut self,
        dock: &DockWindow,
        dock_model: &DockRuntime,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.status
            .sync(dock, dock_model.settings(), dock_model.media(), graphics)
    }

    pub(super) fn refresh_placement(
        &mut self,
        dock: &DockWindow,
        dock_model: &DockRuntime,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.monitors.mark_topology_dirty();
        self.sync_status(dock, dock_model, graphics)?;
        if self.launcher.is_visible() {
            self.launcher.sync_size(dock, graphics)?;
        }
        Ok(())
    }

    pub(super) fn sync_monitor_docks(
        &mut self,
        dock: &DockWindow,
        dock_model: &mut DockRuntime,
        graphics: &mut DeviceState,
        window_tracker: &WindowTracker,
    ) -> Result<(), AppError> {
        self.monitors
            .sync(dock, dock_model, graphics, window_tracker)
    }

    pub(super) fn drain_monitor_dock_events(
        &mut self,
        graphics: &mut DeviceState,
    ) -> Result<crate::app::monitors::MonitorDockEventDrain, AppError> {
        self.monitors.drain_events(graphics)
    }

    pub(super) fn has_pending_monitor_events(&self) -> bool {
        self.monitors.has_pending_events()
    }

    pub(super) fn toggle_launcher(
        &mut self,
        dock: &DockWindow,
        dock_model: &DockRuntime,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if !self.modules.is_enabled(ModuleId::Search) {
            return Ok(());
        }
        self.launcher
            .toggle(dock, dock_model, &self.applications, graphics)
    }

    pub(super) fn hide_launcher(&mut self) {
        self.launcher.hide();
    }

    pub(super) fn launcher_is_visible(&self) -> bool {
        self.launcher.is_visible()
    }

    pub(super) const fn monitor_topology_generation(&self) -> u64 {
        self.monitors.topology_generation()
    }

    pub(super) const fn monitor_integration_health(
        &self,
    ) -> crate::app::monitors::MonitorIntegrationHealth {
        self.monitors.health()
    }

    pub(super) fn monitor_replica_count(&self) -> usize {
        self.monitors.replica_count()
    }

    pub(super) fn has_visible_monitor_dock(&self) -> bool {
        self.monitors.has_visible_dock()
    }

    pub(super) fn diagnostic_surface_masks(&self) -> (u32, u32, u32) {
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

    pub(super) fn advance_launcher_animation(&mut self) {
        self.launcher.advance_animation();
    }

    pub(super) fn invalidate_launcher_surface(&mut self) {
        if let Some(surface) = &mut self.launcher.surface {
            surface.invalidate();
        }
    }

    pub(super) fn launcher_runtime(&mut self) -> &mut LauncherRuntime {
        &mut self.launcher
    }

    pub(super) fn drain_launcher_events(
        &mut self,
    ) -> Vec<lotus_windows::window::SearchEvent> {
        self.launcher.drain_events()
    }

    pub(super) fn refresh_catalog(
        &mut self,
        dock: &DockWindow,
        dock_model: &mut DockRuntime,
        graphics: &mut DeviceState,
    ) -> Result<bool, AppError> {
        self.launcher.refresh_catalog_if_ready(
            dock,
            dock_model,
            &self.applications,
            graphics,
        )
    }

    pub(super) fn application_snapshot(
        &self,
    ) -> std::sync::Arc<ApplicationCatalogSnapshot> {
        self.applications.snapshot()
    }

    pub(super) fn launcher_catalog_refresh_pending(&self) -> bool {
        self.applications
            .ready_generation()
            .is_some_and(|generation| {
                self.launcher.controller.catalog_generation() != Some(generation)
            })
    }

    pub(super) fn open_settings(
        &mut self,
        dock_model: &DockRuntime,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.settings.open(dock_model.settings(), graphics)?;
        self.refresh_open_application_manager(dock_model.items());
        Ok(())
    }

    pub(super) fn open_settings_without_refresh(
        &mut self,
        applied: &DockSettings,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.settings.open(applied, graphics)
    }

    pub(super) fn open_onboarding(
        &mut self,
        applied: &DockSettings,
        required: bool,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.settings.open_onboarding(applied, required, graphics)
    }

    pub(super) fn open_application_icon_manager(
        &mut self,
        dock_model: &mut DockRuntime,
        source_index: usize,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let Some(record) = application_icon_manager_record(dock_model, source_index) else {
            return Ok(());
        };
        let identity = record.id.clone();
        let mut applications = application_records(
            &self.applications,
            dock_model.items(),
            dock_model.settings(),
        );
        if !applications
            .iter()
            .any(|application| application.id == identity)
        {
            applications.push(record);
        }
        self.settings.open(dock_model.settings(), graphics)?;
        self.settings.set_applications(applications);
        self.settings.open_application_manager(&identity);
        self.hydrate_application_previews(dock_model.items());
        self.settings.invalidate();
        Ok(())
    }

    pub(super) fn hide_settings(&mut self) {
        self.settings.hide();
    }

    pub(super) fn settings_owner(&self) -> WindowHandle {
        self.settings.owner()
    }

    pub(super) fn settings_owns_window(&self, window: WindowHandle) -> bool {
        self.settings.owner() == window
    }

    pub(super) fn switcher_owns_window(&self, window: WindowHandle) -> bool {
        self.switcher.window.handle() == window
    }

    pub(super) fn monitor_docks_own_window(&self, window: WindowHandle) -> bool {
        self.monitors.owns_window(window)
    }

    pub(super) fn settings_scene(&mut self) -> &mut SettingsScene {
        self.settings.scene_mut()
    }

    pub(super) fn settings_on_apps_page(&self) -> bool {
        self.settings.page_is_apps()
    }

    pub(super) fn application_catalog_is_empty(&self) -> bool {
        self.settings.applications_are_empty()
    }

    pub(super) fn settings_applications_snapshot(&self) -> Vec<SettingsApplicationRecord> {
        self.settings.applications_snapshot()
    }

    pub(super) fn application_query(&self) -> &str {
        self.settings.application_query()
    }

    pub(super) fn set_application_query(&mut self, query: &str) -> bool {
        self.settings.set_application_query(query)
    }

    pub(super) fn reset_application_icon_override(&mut self, id: &str) {
        self.settings.reset_application_icon_override(id);
    }

    pub(super) fn merge_application_icon_overrides(
        &self,
        current: &DockSettings,
    ) -> Vec<lotus_core::settings::ApplicationIconOverride> {
        self.settings.merged_application_icon_overrides(current)
    }

    pub(super) fn mark_settings_applied(&mut self, applied: DockSettings) {
        self.settings.mark_applied(applied);
    }

    pub(super) fn apply_material_to_settings_window(&mut self, applied: &DockSettings) {
        self.settings.apply_material(applied);
    }

    pub(super) fn onboarding_required_for_close(&self) -> bool {
        self.settings.onboarding_active()
    }

    pub(super) fn end_onboarding(&mut self) {
        self.settings.end_onboarding();
    }

    pub(super) fn settings_runtime(&mut self) -> &mut SettingsRuntime {
        &mut self.settings
    }

    pub(super) fn clear_icon_caches(&mut self) {
        self.settings.clear_icon_caches();
    }

    pub(super) fn invalidate_settings(&mut self) {
        self.settings.invalidate();
    }

    pub(super) fn resize_settings(
        &mut self,
        graphics: &mut DeviceState,
        width: u32,
        height: u32,
    ) -> Result<(), AppError> {
        self.settings.resize(graphics, width, height)
    }

    pub(super) fn apply_settings_dpi(&mut self, dpi: u32) {
        self.settings.set_dpi(dpi);
        self.settings.invalidate();
    }

    pub(super) fn drain_settings_events(&mut self) -> Vec<SettingsEvent> {
        self.settings.drain_events()
    }

    pub(super) fn has_pending_settings_events(&self) -> bool {
        self.settings.has_pending_events()
    }

    pub(super) fn move_settings_pointer(
        &mut self,
        x: u32,
        y: u32,
    ) -> Option<lotus_settings::scene::SettingsAction> {
        self.settings.pointer_moved(x, y)
    }

    pub(super) fn settings_pointer_left(&mut self) {
        self.settings.pointer_left();
    }

    pub(super) fn press_settings_pointer(
        &mut self,
        x: u32,
        y: u32,
    ) -> Option<lotus_settings::scene::SettingsAction> {
        self.settings.pointer_pressed(x, y)
    }

    pub(super) fn release_settings_pointer(
        &mut self,
        x: i32,
        y: i32,
    ) -> Option<lotus_settings::scene::SettingsAction> {
        self.settings.pointer_released(x, y)
    }

    pub(super) fn cancel_settings_pointer(&mut self) {
        self.settings.pointer_cancelled();
    }

    pub(super) fn scroll_settings(&mut self, direction: i32) -> bool {
        self.settings.scrolled(direction)
    }

    pub(super) fn translate_settings_key(
        &mut self,
        key: lotus_windows::window::SettingsKey,
    ) -> lotus_settings::scene::SettingsAction {
        self.settings.translated_key(key)
    }

    pub(super) fn refresh_application_manager(&mut self, dock_items: &[DockItem]) {
        self.refresh_application_records(dock_items);
        self.settings.invalidate();
    }

    pub(super) fn refresh_open_application_manager(&mut self, dock_items: &[DockItem]) {
        let visible_on_apps_page =
            self.settings.is_visible() && self.settings.page_is_apps();

        if !visible_on_apps_page {
            return;
        }
        self.refresh_application_records(dock_items);
        self.settings.invalidate();
    }

    pub(super) fn hydrate_application_previews(&mut self, dock_items: &[DockItem]) {
        self.settings
            .hydrate_application_previews(&self.applications, dock_items);
    }

    fn refresh_application_records(&mut self, dock_items: &[DockItem]) {
        let selected = self.settings.selected_application_id().clone();
        let settings = self.settings.draft().clone();
        let applications = application_records(&self.applications, dock_items, &settings);
        self.settings.set_applications(applications);
        if let Some(selected) = selected {
            self.settings.open_application_manager(&selected);
        }

        self.settings
            .hydrate_application_previews(&self.applications, dock_items);
    }

    pub(super) fn drain_hydrated_icons(
        &mut self,
        dock_model: &mut DockRuntime,
    ) -> Result<(), AppError> {
        let mut launcher = Vec::new();
        let mut switcher = Vec::new();
        let mut dock = Vec::new();

        for result in self.icon_hydrator.drain() {
            match result {
                IconHydrationResult::Launcher(result) => launcher.push(result),
                IconHydrationResult::Switcher(result) => switcher.push(result),
                IconHydrationResult::Dock(result) => dock.push(result),
            }
        }

        let _changed = self.launcher.drain_hydrated_icons(launcher)?;
        self.switcher.drain_hydrated_icons(switcher);
        dock_model.drain_hydrated_window_icons(dock);
        Ok(())
    }

    pub(super) fn render_frames(
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

    pub(super) fn invalidate_surfaces(&mut self) {
        self.launcher.invalidate();
        self.settings.invalidate();
        self.context_menu.invalidate();
        self.switcher.invalidate();
        self.status.invalidate();
        self.monitors.invalidate();
    }

    pub(super) fn recover_surfaces(
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

    pub(super) fn record_switcher_foreground(
        &mut self,
        foreground: Option<lotus_core::window::WindowId>,
        windows: &[lotus_core::window::WindowInfo],
    ) {
        self.switcher.record_foreground(foreground.and_then(|id| {
            windows
                .iter()
                .find(|window| window.id == id)
                .map(lotus_core::window::WindowInfo::key)
        }));
    }

    pub(super) fn reconcile_switcher_windows(
        &mut self,
        windows: &[lotus_core::window::WindowInfo],
        application_catalog: std::sync::Arc<ApplicationCatalogSnapshot>,
        application_assignments: &lotus_core::application::WindowApplicationAssignments,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.switcher.reconcile_windows(
            windows,
            application_catalog,
            application_assignments,
            graphics,
        )
    }

    pub(super) fn drain_switcher_events(&mut self, graphics: &mut DeviceState) -> bool {
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

    pub(super) fn has_pending_switcher_events(&self) -> bool {
        self.switcher.window.has_pending_events()
    }

    pub(super) fn context_menu_runtime(&mut self) -> &mut ContextMenuRuntime {
        &mut self.context_menu
    }

    pub(super) fn drain_context_menu_events(&mut self) -> Vec<ContextMenuEvent> {
        self.context_menu.drain_events()
    }

    pub(super) fn hide_context_menu(&mut self) {
        self.context_menu.hide();
    }

    pub(super) fn open_context_menu(
        &mut self,
        anchor: SignedPoint,
        alignment: PopupAlignment,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.context_menu.open(anchor, alignment, graphics)
    }

    pub(super) fn open_application_context_menu(
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
            source_index,
            AppMenuOptions {
                identity: item.id.clone(),
                running_windows: item.windows.len(),
                pinned: item.is_pinned,
                shift_held,
            },
            graphics,
        )
    }

    pub(super) fn open_window_picker(
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
        self.context_menu.open_picker(
            anchor,
            source_index,
            identity,
            style,
            entries,
            graphics,
        )
    }

    pub(super) fn open_power_menu(
        &mut self,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.context_menu.open_power(graphics)
    }

    pub(super) fn reconcile_visible_window_picker(
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
            self.context_menu.hide();
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
        self.context_menu
            .replace_picker(source_index, style, windows, graphics)
    }

    pub(super) fn activate_media(
        &mut self,
        target: lotus_media::MediaHitTarget,
        dock_model: &mut DockRuntime,
        owner: WindowHandle,
    ) {
        self.media.activate(target, dock_model, owner);
    }

    pub(super) fn dismiss_popups_for_activation(&mut self) {
        self.launcher.hide();
    }

    pub(super) fn set_status_visible(&mut self, visible: bool) {
        self.status.set_visible(visible);
    }

    pub(super) fn set_status_fullscreen_occluded(
        &mut self,
        occluded: bool,
    ) -> Result<(), AppError> {
        self.status.set_fullscreen_occluded(occluded)
    }

    pub(super) fn refresh_status(&mut self, settings: &DockSettings) {
        self.status.refresh(settings);
    }

    pub(super) fn drain_status_events(&mut self) -> Vec<(usize, WindowEvent)> {
        self.status.drain_events()
    }

    pub(super) fn has_pending_window_events(&self) -> bool {
        self.launcher.has_pending_events()
            || self.context_menu.window.has_pending_events()
            || self.status.has_pending_events()
    }

    pub(super) fn handle_status_event(
        &mut self,
        zone_index: usize,
        event: WindowEvent,
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

    pub(super) fn hide_launcher_on_status_press(&mut self, event: &WindowEvent) {
        if matches!(
            event,
            WindowEvent::Pointer(PointerEvent::LeftButtonPressed { .. })
        ) {
            self.launcher.hide();
        }
    }
}

pub(super) struct StatusZoneActivation {
    pub(super) action: AuxiliaryZoneAction,
    pub(super) owner: WindowHandle,
    pub(super) anchor: Option<SignedPoint>,
}

fn application_icon_manager_record(
    dock_model: &mut DockRuntime,
    source_index: usize,
) -> Option<SettingsApplicationRecord> {
    let icon = dock_model.application_icon_preview(source_index);
    let item = dock_model.item(source_index)?;
    let custom = dock_model
        .settings()
        .application_icon_override_for(&item.application_identity());
    let id = custom.map_or_else(|| item.id.clone(), |override_| override_.id.clone());
    Some(SettingsApplicationRecord {
        id: id.clone(),
        name: item.display_name.clone(),
        icon,
        app_user_model_id: item.app_user_model_id.clone(),
        match_executables: std::path::Path::new(&item.executable_path)
            .file_name()
            .and_then(|name| name.to_str().map(str::to_owned))
            .into_iter()
            .collect(),
        customized: custom.is_some(),
        missing_icon: false,
    })
}

pub(super) struct ModuleRuntime {
    enabled: ModuleSet,
    input_config: Option<InputConfig>,
    input: Option<InputController>,
}

pub(super) struct ModuleResources<'a> {
    pub(super) dock: &'a DockWindow,
    pub(super) launcher: &'a mut LauncherRuntime,
    pub(super) switcher: &'a mut SwitcherRuntime,
    pub(super) media: &'a mut MediaRuntime,
    pub(super) status: &'a mut StatusRuntime,
}

impl ModuleRuntime {
    pub(super) fn new() -> Self {
        Self {
            enabled: ModuleSet::default(),
            input_config: None,
            input: None,
        }
    }

    pub(super) fn reconcile(
        &mut self,
        settings: &DockSettings,
        active: bool,
        resources: &mut ModuleResources<'_>,
    ) -> Result<(), AppError> {
        let next = if active {
            ModuleSet::from_settings(settings)
        } else {
            ModuleSet::default()
        };

        if self.was_disabled(ModuleId::Search, next) {
            resources.launcher.hide();
        }
        if self.was_disabled(ModuleId::AltTab, next) {
            resources.switcher.abandon();
        }
        if self.was_disabled(ModuleId::Status, next) {
            resources.status.set_visible(false);
        }

        resources.media.set_enabled(next.contains(ModuleId::Media));
        resources.dock.set_status_refresh_active(
            next.contains(ModuleId::Status) && settings.show_date_time_status,
        )?;
        self.reconcile_input(settings, next);
        self.enabled = next;
        Ok(())
    }

    pub(super) const fn is_enabled(&self, module: ModuleId) -> bool {
        self.enabled.contains(module)
    }

    pub(super) const fn input(&self) -> Option<&InputController> {
        self.input.as_ref()
    }

    fn was_disabled(&self, module: ModuleId, next: ModuleSet) -> bool {
        self.enabled.contains(module) && !next.contains(module)
    }

    fn reconcile_input(&mut self, settings: &DockSettings, modules: ModuleSet) {
        let next = InputConfig {
            windows_key_search: modules.contains(ModuleId::Search)
                && settings.search_open_with_windows_key,
            custom_alt_tab: modules.contains(ModuleId::AltTab),
        };
        let next = (next.windows_key_search || next.custom_alt_tab).then_some(next);
        if self.input_config == next {
            return;
        }

        self.input = None;
        self.input_config = None;
        let Some(config) = next else {
            return;
        };

        match InputController::start(config) {
            Ok(controller) => {
                self.input = Some(controller);
                self.input_config = Some(config);
            }
            Err(error) => lotus_windows::diagnostics::record_error("input.enable", &error),
        }
    }
}
