use std::path::Path;

use lotus_core::settings::DockSettings;
use lotus_ui::icon::RasterIcon;
use lotus_windows::custom_image::CustomImageCache;

pub(super) fn resolve_application_icon(
    settings: &DockSettings,
    custom_images: &mut CustomImageCache,
    app_user_model_id: Option<&str>,
    stable_id: Option<&str>,
    executable_path: &Path,
) -> Option<RasterIcon> {
    let executable_name = executable_path.file_name().and_then(|name| name.to_str());
    let custom = settings.application_icon_override(
        app_user_model_id,
        stable_id,
        executable_name,
    )?;
    custom_images.image(Path::new(&custom.image_path)).ok()
}
