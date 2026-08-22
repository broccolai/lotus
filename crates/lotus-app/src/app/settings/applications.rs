use std::path::Path;
use std::time::Instant;

use lotus_core::application::{ApplicationIdentity, is_shared_host_executable};
use lotus_core::dock::DockItem;
use lotus_core::search::ApplicationEntry;
use lotus_core::settings::DockSettings;
use lotus_settings::scene::{SettingsApplicationRecord, SettingsControl};
use lotus_windows::custom_image::CustomImageCache;
use lotus_windows::launch::resolve_executable;
use lotus_windows::native_icon::NativeIconCache;
use lotus_windows::responsiveness::{LayoutOperation, METRICS};
use lotus_windows::search_catalog::SearchCatalogCache;

use super::SettingsRuntime;

const PREVIEW_ICON_PIXEL_SIZE: u32 = 96;

pub(in crate::app) fn application_records(
    cache: &SearchCatalogCache,
    dock_items: &[DockItem],
    settings: &DockSettings,
) -> Vec<SettingsApplicationRecord> {
    let catalog = cache.catalog(dock_items, &[]);
    let mut applications = catalog
        .entries_for_management()
        .map(|entry| {
            let id = application_record_id(entry);
            let executable = resolve_executable(&entry.launch_target);
            let executable_name = executable
                .as_deref()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str());
            let identity = ApplicationIdentity::from_path(
                entry.app_user_model_id.as_deref(),
                Some(&id),
                executable.as_deref(),
                std::iter::empty(),
            );
            let custom = settings.application_icon_override_for(&identity);
            SettingsApplicationRecord {
                id,
                name: entry.name.clone(),
                icon: None,
                app_user_model_id: entry.app_user_model_id.clone(),
                match_executables: executable_name
                    .filter(|name| !is_shared_host_executable(name))
                    .map(str::to_owned)
                    .into_iter()
                    .collect(),
                customized: custom.is_some(),
                missing_icon: custom
                    .is_some_and(|override_| !Path::new(&override_.image_path).is_file()),
            }
        })
        .collect::<Vec<_>>();

    applications.sort_by_cached_key(|application| {
        (!application.customized, application.name.to_lowercase())
    });
    applications
}

pub(super) fn hydrate_previews(
    runtime: &mut SettingsRuntime,
    cache: &SearchCatalogCache,
    dock_items: &[DockItem],
) {
    let ids = visible_application_ids(runtime);
    if ids.is_empty() {
        return;
    }

    let settings = runtime.scene.draft().clone();
    let catalog = cache.catalog(dock_items, &[]);

    for id in ids {
        let Some(entry) = catalog
            .entries_for_management()
            .find(|entry| application_record_id(entry).eq_ignore_ascii_case(&id))
        else {
            continue;
        };
        let Some(icon) = effective_application_icon(
            entry,
            &settings,
            &mut runtime.native_icons,
            &mut runtime.custom_images,
        ) else {
            continue;
        };
        let _ = runtime.scene.set_application_icon(&id, icon);
    }
}

fn visible_application_ids(runtime: &SettingsRuntime) -> Vec<String> {
    let started = Instant::now();
    let layout = runtime.scene.layout();
    METRICS.record_layout(LayoutOperation::SettingsVisibleRows, started.elapsed());
    let mut ids = layout
        .controls
        .iter()
        .filter(|entry| layout.content_intersects_viewport(entry.bounds))
        .filter_map(|entry| match entry.control {
            SettingsControl::ApplicationRow(index) => {
                runtime.scene.applications().get(index)
            }
            _ => None,
        })
        .map(|application| application.id.clone())
        .collect::<Vec<_>>();

    if let Some(selected) = runtime.scene.selected_application()
        && !ids.iter().any(|id| id.eq_ignore_ascii_case(&selected.id))
    {
        ids.push(selected.id.clone());
    }

    ids
}

fn application_record_id(entry: &ApplicationEntry) -> String {
    let identity = entry.application_identity();

    identity
        .reliable_registered_id()
        .or_else(|| identity.stable_id())
        .unwrap_or(&entry.launch_target)
        .to_owned()
}

fn effective_application_icon(
    entry: &ApplicationEntry,
    settings: &DockSettings,
    native_icons: &mut NativeIconCache,
    custom_images: &mut CustomImageCache,
) -> Option<lotus_ui::icon::RasterIcon> {
    let executable = resolve_executable(&entry.launch_target)
        .unwrap_or_else(|| Path::new(&entry.icon_source).to_path_buf());
    let identity = ApplicationIdentity::from_path(
        entry.app_user_model_id.as_deref(),
        Some(&application_record_id(entry)),
        Some(&executable),
        std::iter::empty(),
    );

    if let Some(override_) = settings.application_icon_override_for(&identity)
        && let Ok(icon) = custom_images.image(Path::new(&override_.image_path))
    {
        return Some(icon);
    }

    native_icons
        .icon(Path::new(&entry.icon_source), PREVIEW_ICON_PIXEL_SIZE)
        .ok()
        .flatten()
}
