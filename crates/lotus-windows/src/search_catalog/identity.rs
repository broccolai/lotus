use std::path::Path;

use lotus_core::application::ApplicationIdentity;
use lotus_core::dock::DockItem;
use lotus_core::search::{ApplicationEntry, ApplicationSource, SearchCatalog};

use super::shortcuts::{is_chromium_web_app_shortcut, shortcut_process_start_executable};
use crate::launch::resolve_executable;

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

    entries.extend(discovered_entries.into_iter().filter_map(|mut entry| {
        if entry.icon_source.starts_with(r"shell:AppsFolder\")
            && let Some(item) = dock_items
                .iter()
                .find(|item| item.display_name.eq_ignore_ascii_case(&entry.name))
        {
            entry.icon_source.clone_from(&item.executable_path);
        }
        entry = if matches_hidden_executable(&entry, hidden_executables) {
            entry.hidden_until_search()
        } else {
            entry
        };
        (!has_pinned_alias(&entry, dock_items)).then_some(entry)
    }));

    entries.push(ApplicationEntry::new(
        WINDOWS_SETTINGS_NAME,
        WINDOWS_SETTINGS_TARGET,
        Some(WINDOWS_SETTINGS_TARGET.into()),
    ));
    SearchCatalog::new(entries)
}

pub(super) fn application_entry_identity(entry: &ApplicationEntry) -> ApplicationIdentity {
    let executable = resolve_executable(&entry.launch_target);
    ApplicationIdentity::from_path(
        entry.app_user_model_id.as_deref(),
        Some(&entry.launch_target),
        executable.as_deref(),
        std::iter::empty(),
    )
}

fn has_pinned_alias(entry: &ApplicationEntry, dock_items: &[DockItem]) -> bool {
    dock_items
        .iter()
        .filter(|item| item.is_pinned)
        .any(|item| entry_matches_dock_item(entry, item))
}

fn executable_identity(path: &Path) -> ApplicationIdentity {
    ApplicationIdentity::from_path(None, None, Some(path), std::iter::empty())
}

fn entry_matches_dock_item(entry: &ApplicationEntry, item: &DockItem) -> bool {
    let entry_identity = application_entry_identity(entry);
    if entry_identity
        .match_strength(&item.application_identity())
        .is_match()
    {
        return true;
    }

    if squirrel_identity_matches_executable(entry, &item.executable_path) {
        return true;
    }

    entry_executable(entry).is_some_and(|target| {
        executable_identity(&target)
            .match_strength(&executable_identity(Path::new(&item.executable_path)))
            .is_match()
    })
}

fn squirrel_identity_matches_executable(
    entry: &ApplicationEntry,
    executable: &str,
) -> bool {
    let Some(identity) = entry
        .app_user_model_id
        .as_deref()
        .filter(|identity| identity.to_ascii_lowercase().starts_with("com.squirrel."))
    else {
        return false;
    };
    let Some(application) = identity.rsplit('.').next() else {
        return false;
    };
    Path::new(executable)
        .file_stem()
        .and_then(|name| name.to_str())
        .is_some_and(|executable| executable.eq_ignore_ascii_case(application))
}

fn entry_executable(entry: &ApplicationEntry) -> Option<std::path::PathBuf> {
    let target = Path::new(&entry.launch_target);
    let is_shortcut = target
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"));
    if is_shortcut && is_chromium_web_app_shortcut(target) {
        return None;
    }

    if is_shortcut && let Some(executable) = shortcut_process_start_executable(target) {
        return Some(executable);
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
