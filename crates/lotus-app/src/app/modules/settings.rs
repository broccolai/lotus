use lotus_core::dock::DockItem;
use lotus_core::settings::{ApplicationIconOverride, DockSettings};
use lotus_settings::scene::SettingsApplicationRecord;
use lotus_windows::WindowHandle;
use lotus_windows::graphics::DeviceState;
use lotus_windows::window::SettingsEvent;

use super::{ModuleHost, SettingsIntent};
use crate::app::AppError;
use crate::app::dock::DockRuntime;
use crate::app::settings::{
    ApplicationIconOutcome, ColorOutcome, ColorTarget, SettingsEventOutcome,
    application_records,
};

impl ModuleHost {
    pub(in crate::app) fn open_settings(
        &mut self,
        dock_model: &DockRuntime,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.settings.open(dock_model.settings(), graphics)?;
        self.refresh_open_application_manager(dock_model.items());
        Ok(())
    }

    pub(in crate::app) fn open_settings_without_refresh(
        &mut self,
        applied: &DockSettings,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.settings.open(applied, graphics)
    }

    pub(in crate::app) fn open_onboarding(
        &mut self,
        applied: &DockSettings,
        required: bool,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.settings.open_onboarding(applied, required, graphics)
    }

    pub(in crate::app) fn open_application_icon_manager(
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

    pub(in crate::app) fn hide_settings(&mut self) {
        self.settings.hide();
    }

    pub(in crate::app) fn settings_owner(&self) -> WindowHandle {
        self.settings.owner()
    }

    pub(in crate::app) fn settings_owns_window(&self, window: WindowHandle) -> bool {
        self.settings.owner() == window
    }

    pub(in crate::app) fn settings_on_apps_page(&self) -> bool {
        self.settings.page_is_apps()
    }

    pub(in crate::app) fn application_catalog_is_empty(&self) -> bool {
        self.settings.applications_are_empty()
    }

    pub(in crate::app) fn reset_application_icon_override(&mut self, id: &str) {
        self.settings.reset_application_icon_override(id);
    }

    pub(in crate::app) fn merge_application_icon_overrides(
        &self,
        current: &DockSettings,
    ) -> Vec<ApplicationIconOverride> {
        self.settings.merged_application_icon_overrides(current)
    }

    pub(in crate::app) fn mark_settings_applied(&mut self, applied: DockSettings) {
        self.settings.mark_applied(applied);
    }

    pub(in crate::app) fn apply_material_to_settings_window(
        &mut self,
        applied: &DockSettings,
    ) {
        self.settings.apply_material(applied);
    }

    pub(in crate::app) fn onboarding_required_for_close(&self) -> bool {
        self.settings.onboarding_active()
    }

    pub(in crate::app) fn end_onboarding(&mut self) {
        self.settings.end_onboarding();
    }

    pub(in crate::app) fn clear_icon_caches(&mut self) {
        self.settings.clear_icon_caches();
    }

    pub(in crate::app) fn choose_settings_color(
        &mut self,
        target: ColorTarget,
    ) -> ColorOutcome {
        self.settings.choose_color(target)
    }

    pub(in crate::app) fn choose_settings_mascot_image(
        &mut self,
        settings_directory: &std::path::Path,
    ) {
        let _ = self.settings.choose_mascot_image(settings_directory);
    }

    pub(in crate::app) fn choose_settings_application_icon(
        &mut self,
        id: &str,
        settings_directory: &std::path::Path,
    ) -> ApplicationIconOutcome {
        self.settings
            .choose_application_icon(id, settings_directory)
    }

    pub(in crate::app) fn start_update_check(
        &mut self,
    ) -> Result<bool, lotus_windows::update::UpdateStartError> {
        self.settings.start_update_check()
    }

    pub(in crate::app) fn drain_update_results(
        &self,
    ) -> Vec<lotus_windows::update::UpdateResult> {
        self.settings.drain_update_results()
    }

    pub(in crate::app) fn start_update_download(
        &mut self,
        release: lotus_windows::update::Release,
    ) -> Result<bool, lotus_windows::update::UpdateStartError> {
        self.settings.start_update_download(release)
    }

    pub(in crate::app) fn reset_update_activity(&mut self) {
        self.settings
            .set_update_activity(lotus_settings::scene::SettingsUpdateActivity::Idle);
        self.settings.invalidate();
    }

    pub(in crate::app) fn invalidate_settings(&mut self) {
        self.settings.invalidate();
    }

    pub(in crate::app) fn drain_settings_events(&mut self) -> Vec<SettingsEvent> {
        self.settings.drain_events()
    }

    pub(in crate::app) fn handle_settings_event(
        &mut self,
        event: SettingsEvent,
        graphics: &mut DeviceState,
        dock_items: &[DockItem],
    ) -> Result<SettingsIntent, AppError> {
        let outcome = self.settings.handle_event(event, graphics)?;
        match outcome {
            SettingsEventOutcome::None => Ok(SettingsIntent::None),
            SettingsEventOutcome::RefreshApplications => {
                self.refresh_application_records(dock_items);
                self.settings.invalidate();
                Ok(SettingsIntent::None)
            }
            SettingsEventOutcome::HydrateApplicationPreviews => {
                self.settings
                    .hydrate_application_previews(&self.applications, dock_items);
                self.settings.invalidate();
                Ok(SettingsIntent::None)
            }
            SettingsEventOutcome::PasteQuery => Ok(SettingsIntent::PasteQuery),
            SettingsEventOutcome::Action(action) => Ok(SettingsIntent::Action(action)),
        }
    }

    pub(in crate::app) fn paste_settings_query(
        &mut self,
        clipboard: &str,
        dock_items: &[DockItem],
    ) {
        if self.settings.paste_query(clipboard) {
            self.settings
                .hydrate_application_previews(&self.applications, dock_items);
        }
    }

    pub(in crate::app) fn has_pending_settings_events(&self) -> bool {
        self.settings.has_pending_events()
    }

    pub(in crate::app) fn refresh_application_manager(&mut self, dock_items: &[DockItem]) {
        self.refresh_application_records(dock_items);
        self.settings.invalidate();
    }

    pub(in crate::app) fn refresh_open_application_manager(
        &mut self,
        dock_items: &[DockItem],
    ) {
        let visible_on_apps_page =
            self.settings.is_visible() && self.settings.page_is_apps();

        if !visible_on_apps_page {
            return;
        }
        self.refresh_application_records(dock_items);
        self.settings.invalidate();
    }

    pub(in crate::app) fn hydrate_application_previews(&mut self, dock_items: &[DockItem]) {
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
