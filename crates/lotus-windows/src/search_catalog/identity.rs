use std::path::Path;

use lotus_core::application::ApplicationIdentity;
use lotus_core::dock::DockItem;
use lotus_core::search::{ApplicationEntry, ApplicationSource, SearchCatalog};

use super::shortcuts::is_chromium_web_app_shortcut;
use crate::launch::{resolve_executable, shortcut_arguments};

const WINDOWS_SETTINGS_NAME: &str = "Windows Settings";
const WINDOWS_SETTINGS_TARGET: &str = "ms-settings:";

pub(super) fn compose_catalog(
    dock_items: &[DockItem],
    discovered_entries: impl IntoIterator<Item = ApplicationEntry>,
    hidden_executables: &[String],
) -> SearchCatalog {
    let dock_entry = |item: &DockItem| {
        let entry = ApplicationEntry::new(
            item.display_name.clone(),
            item.launch_target.clone(),
            Some(item.executable_path.clone()),
        )
        .with_source(if item.is_pinned {
            ApplicationSource::Pinned
        } else {
            ApplicationSource::Running
        });
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

    let discovered_entries = discovered_entries
        .into_iter()
        .map(|mut entry| {
            if entry.icon_source.starts_with(r"shell:AppsFolder\")
                && let Some(item) = dock_items
                    .iter()
                    .find(|item| item.display_name.eq_ignore_ascii_case(&entry.name))
            {
                entry.icon_source.clone_from(&item.executable_path);
            }
            if matches_hidden_executable(&entry, hidden_executables) {
                entry.hidden_until_search()
            } else {
                entry
            }
        })
        .filter(|entry| !has_pinned_alias(entry, dock_items))
        .collect::<Vec<_>>();

    entries.extend(discovered_entries.iter().cloned());
    entries.extend(
        dock_items
            .iter()
            .filter(|item| !item.is_pinned)
            .filter(|item| !has_installed_alias(item, &discovered_entries))
            .map(dock_entry),
    );

    entries.push(ApplicationEntry::new(
        WINDOWS_SETTINGS_NAME,
        WINDOWS_SETTINGS_TARGET,
        Some(WINDOWS_SETTINGS_TARGET.into()),
    ));
    SearchCatalog::new(entries)
}

fn has_installed_alias(item: &DockItem, discovered_entries: &[ApplicationEntry]) -> bool {
    let item_identity = executable_identity(Path::new(&item.executable_path));
    discovered_entries.iter().any(|entry| {
        plain_shortcut_executable(entry).is_some_and(|target| {
            let entry_identity = executable_identity(&target);
            entry_identity.match_strength(&item_identity).is_match()
        })
    })
}

fn has_pinned_alias(entry: &ApplicationEntry, dock_items: &[DockItem]) -> bool {
    plain_shortcut_executable(entry).is_some_and(|target| {
        let entry_identity = executable_identity(&target);
        dock_items.iter().filter(|item| item.is_pinned).any(|item| {
            entry_identity
                .match_strength(&executable_identity(Path::new(&item.executable_path)))
                .is_match()
        })
    })
}

fn executable_identity(path: &Path) -> ApplicationIdentity {
    ApplicationIdentity::from_path(None, None, Some(path), std::iter::empty())
}

fn plain_shortcut_executable(entry: &ApplicationEntry) -> Option<std::path::PathBuf> {
    let target = Path::new(&entry.launch_target);
    let is_shortcut = target
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"));
    if entry.app_user_model_id.is_some()
        || !is_shortcut
        || is_chromium_web_app_shortcut(target)
        || shortcut_arguments(target).is_some_and(|arguments| !arguments.trim().is_empty())
    {
        return None;
    }

    resolve_executable(&entry.launch_target)
}

fn matches_hidden_executable(
    entry: &ApplicationEntry,
    hidden_executables: &[String],
) -> bool {
    let executable = resolve_executable(&entry.launch_target);
    let identity = ApplicationIdentity::from_path(
        entry.app_user_model_id.as_deref(),
        Some(&entry.launch_target),
        executable.as_deref(),
        std::iter::empty(),
    );
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
