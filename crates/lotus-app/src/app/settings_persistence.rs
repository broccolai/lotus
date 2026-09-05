use std::path::Path;

use lotus_core::settings::{DockSettings, SettingsReset, SettingsStore};

use crate::app::AppError;

/// Owns Lotus's compatible settings file and its file-level operations.
pub(super) struct SettingsPersistence {
    store: SettingsStore,
}

impl SettingsPersistence {
    pub(super) const fn new(store: SettingsStore) -> Self {
        Self { store }
    }

    pub(super) fn directory(&self) -> &Path {
        self.store.directory()
    }

    pub(super) fn save(&self, settings: &DockSettings) -> Result<(), AppError> {
        self.store.save(settings)?;
        Ok(())
    }

    pub(super) fn export(
        &self,
        settings: &DockSettings,
        destination: &Path,
    ) -> Result<(), AppError> {
        self.store.export(settings, destination)?;
        Ok(())
    }

    pub(super) fn validate_export_destination(
        &self,
        destination: &Path,
    ) -> Result<(), AppError> {
        self.store.validate_export_destination(destination)?;
        Ok(())
    }

    pub(super) fn reset(&self) -> Result<SettingsReset, AppError> {
        Ok(self.store.reset()?)
    }
}
