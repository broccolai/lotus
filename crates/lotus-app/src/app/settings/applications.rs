use std::path::Path;
use std::time::Instant;

use lotus_core::application::{
    ApplicationIdentity, RegisteredApplication, is_shared_host_executable,
};
use lotus_core::dock::DockItem;
use lotus_core::settings::DockSettings;
use lotus_settings::scene::{SettingsApplicationRecord, SettingsControl};
use lotus_windows::custom_image::CustomImageCache;
use lotus_windows::native_icon::NativeIconCache;
use lotus_windows::responsiveness::{LayoutOperation, METRICS};
use lotus_windows::search_catalog::ApplicationCatalogSnapshot;

use super::SettingsRuntime;

const PREVIEW_ICON_PIXEL_SIZE: u32 = 96;

pub(in crate::app) fn application_records(
    snapshot: &ApplicationCatalogSnapshot,
    dock_items: &[DockItem],
    settings: &DockSettings,
) -> Vec<SettingsApplicationRecord> {
    let mut applications = snapshot
        .applications
        .iter()
        .map(|application| registered_record(application, settings))
        .collect::<Vec<_>>();
    applications.extend(
        dock_items
            .iter()
            .filter(|item| item.is_pinned)
            .filter(|item| {
                snapshot
                    .application_index_for_key(&item.application_key)
                    .is_none()
            })
            .map(|item| dock_record(item, settings)),
    );

    applications.sort_by_cached_key(|application| {
        (!application.customized, application.name.to_lowercase())
    });
    applications
}

pub(super) fn hydrate_previews(
    runtime: &mut SettingsRuntime,
    snapshot: &ApplicationCatalogSnapshot,
    dock_items: &[DockItem],
) {
    let ids = visible_application_ids(runtime);
    if ids.is_empty() {
        return;
    }

    let settings = runtime.scene.draft().clone();
    for id in ids {
        let source = snapshot
            .applications
            .iter()
            .find(|application| application.id.eq_ignore_ascii_case(&id))
            .map(|application| {
                (
                    application.application_identity(),
                    application.icon_source.as_str(),
                )
            })
            .or_else(|| {
                dock_items
                    .iter()
                    .find(|item| item.id.eq_ignore_ascii_case(&id))
                    .map(|item| (item.application_identity(), item.icon_source.as_str()))
            });
        let Some((identity, icon_source)) = source else {
            continue;
        };
        let Some(icon) = effective_application_icon(
            &identity,
            icon_source,
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

fn registered_record(
    application: &RegisteredApplication,
    settings: &DockSettings,
) -> SettingsApplicationRecord {
    settings_record(
        application.id.clone(),
        application.name.clone(),
        application.app_user_model_id.clone(),
        application.executable_aliases.clone(),
        &application.application_identity(),
        settings,
    )
}

fn dock_record(item: &DockItem, settings: &DockSettings) -> SettingsApplicationRecord {
    let match_executables = Path::new(&item.executable_path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !is_shared_host_executable(name))
        .map(str::to_owned)
        .into_iter()
        .collect();
    settings_record(
        item.id.clone(),
        item.display_name.clone(),
        item.app_user_model_id.clone(),
        match_executables,
        &item.application_identity(),
        settings,
    )
}

fn settings_record(
    id: String,
    name: String,
    app_user_model_id: Option<String>,
    match_executables: Vec<String>,
    identity: &ApplicationIdentity,
    settings: &DockSettings,
) -> SettingsApplicationRecord {
    let custom = settings.application_icon_override_for(identity);
    SettingsApplicationRecord {
        id,
        name,
        icon: None,
        app_user_model_id,
        match_executables,
        customized: custom.is_some(),
        missing_icon: custom
            .is_some_and(|override_| !Path::new(&override_.image_path).is_file()),
    }
}

fn effective_application_icon(
    identity: &ApplicationIdentity,
    icon_source: &str,
    settings: &DockSettings,
    native_icons: &mut NativeIconCache,
    custom_images: &mut CustomImageCache,
) -> Option<lotus_ui::icon::RasterIcon> {
    if let Some(override_) = settings.application_icon_override_for(identity)
        && let Ok(icon) = custom_images.image(Path::new(&override_.image_path))
    {
        return Some(icon);
    }

    native_icons
        .icon(Path::new(icon_source), PREVIEW_ICON_PIXEL_SIZE)
        .ok()
        .flatten()
}
