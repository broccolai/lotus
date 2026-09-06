use std::path::{Path, PathBuf};

use lotus_core::application::ApplicationIdentity;
use lotus_core::settings::DockSettings;
pub(super) fn application_icon_path(
    settings: &DockSettings,
    app_user_model_id: Option<&str>,
    stable_id: Option<&str>,
    executable_path: &Path,
) -> Option<PathBuf> {
    let identity = ApplicationIdentity::from_path(
        app_user_model_id,
        stable_id,
        Some(executable_path),
        std::iter::empty(),
    );
    application_icon_path_for_identity(settings, &identity)
}

pub(super) fn application_icon_path_for_identity(
    settings: &DockSettings,
    identity: &ApplicationIdentity,
) -> Option<PathBuf> {
    let custom = settings.application_icon_override_for(identity)?;
    Some(PathBuf::from(&custom.image_path))
}
