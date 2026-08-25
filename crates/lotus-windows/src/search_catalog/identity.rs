use std::path::Path;

use lotus_core::application::RegisteredApplication;
use lotus_core::dock::DockItem;
use lotus_core::search::{ApplicationEntry, ApplicationSource, SearchCatalog};

const WINDOWS_SETTINGS_NAME: &str = "Windows Settings";
const WINDOWS_SETTINGS_TARGET: &str = "ms-settings:";

pub(super) fn compose_catalog(
    dock_items: &[DockItem],
    applications: &[RegisteredApplication],
    discovered_entries: &[ApplicationEntry],
    hidden_executables: &[String],
) -> SearchCatalog {
    let dock_entry = |item: &DockItem| {
        let entry = ApplicationEntry::new(
            item.display_name.clone(),
            item.launch_target.clone(),
            Some(item.executable_path.clone()),
        )
        .with_arguments(item.arguments.as_deref())
        .with_source(ApplicationSource::Pinned);
        if let Some(identity) = &item.app_user_model_id {
            entry.with_app_user_model_id(identity)
        } else {
            entry
        }
    };
    let mut entries = dock_items
        .iter()
        .filter(|item| item.is_pinned)
        .map(dock_entry)
        .collect::<Vec<_>>();

    entries.extend(applications.iter().zip(discovered_entries).filter_map(
        |(application, entry)| {
            let mut entry = entry.clone();
            if entry.icon_source.starts_with(r"shell:AppsFolder\")
                && let Some(item) = dock_items
                    .iter()
                    .find(|item| item.display_name.eq_ignore_ascii_case(&entry.name))
            {
                entry.icon_source.clone_from(&item.executable_path);
            }
            entry = if matches_hidden_executable(application, &entry, hidden_executables) {
                entry.hidden_until_search()
            } else {
                entry
            };
            (!has_pinned_alias(application, dock_items)).then_some(entry)
        },
    ));

    entries.push(ApplicationEntry::new(
        WINDOWS_SETTINGS_NAME,
        WINDOWS_SETTINGS_TARGET,
        Some(WINDOWS_SETTINGS_TARGET.into()),
    ));
    SearchCatalog::new(entries)
}

fn has_pinned_alias(application: &RegisteredApplication, dock_items: &[DockItem]) -> bool {
    dock_items
        .iter()
        .filter(|item| item.is_pinned)
        .any(|item| item.application_key == application.key)
}

fn matches_hidden_executable(
    application: &RegisteredApplication,
    entry: &ApplicationEntry,
    hidden_executables: &[String],
) -> bool {
    let identity = application.application_identity();
    hidden_executables.iter().any(|hidden| {
        let hidden = Path::new(hidden);
        let matches_path = [entry.launch_target.as_str(), entry.icon_source.as_str()]
            .into_iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(&hidden.to_string_lossy()));
        matches_path
            || identity.has_executable_alias(&hidden.to_string_lossy())
            || hidden
                .file_stem()
                .and_then(|name| name.to_str())
                .is_some_and(|name| entry.name.eq_ignore_ascii_case(name))
    })
}
