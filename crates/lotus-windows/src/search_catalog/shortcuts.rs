use std::path::Path;

use lotus_core::application::is_reliable_registered_id;
use lotus_core::search::ApplicationEntry;

use super::super::application_identity::shortcut_application_id;
use super::super::launch::{resolve_executable, shortcut_arguments};

pub(super) fn shortcut_entry(
    name: String,
    path: &Path,
    identity: Option<&ShortcutIdentity>,
) -> ApplicationEntry {
    let target = path.to_string_lossy().into_owned();
    let mut entry = ApplicationEntry::new(name, target.clone(), Some(target));
    if let Some(identity) = identity.and_then(ShortcutIdentity::app_user_model_id) {
        entry = entry.with_app_user_model_id(identity);
    }
    entry
}

pub(super) struct ShortcutIdentity {
    app_user_model_id: Option<String>,
}

impl ShortcutIdentity {
    pub(super) fn from_path(path: &Path) -> Option<Self> {
        is_shortcut(path).then(|| Self {
            app_user_model_id: shortcut_application_id(path)
                .filter(|id| is_reliable_registered_id(id)),
        })
    }

    fn app_user_model_id(&self) -> Option<&str> {
        self.app_user_model_id.as_deref()
    }
}

fn is_shortcut(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
}

pub(super) fn is_chromium_web_app_shortcut(path: &Path) -> bool {
    if !is_shortcut(path) {
        return false;
    }

    let arguments = shortcut_arguments(path);
    let target = resolve_executable(&path.to_string_lossy());
    chromium_web_app_identity(arguments.as_deref(), target.as_deref())
}

fn chromium_web_app_identity(arguments: Option<&str>, target: Option<&Path>) -> bool {
    arguments.is_some_and(chromium_web_app_arguments)
        || target
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.to_ascii_lowercase().ends_with("_proxy.exe"))
}

fn chromium_web_app_arguments(arguments: &str) -> bool {
    arguments.split_ascii_whitespace().any(|argument| {
        let argument = argument.trim_matches('"').to_ascii_lowercase();
        argument.starts_with("--app-id=") || argument.starts_with("--app=")
    })
}
