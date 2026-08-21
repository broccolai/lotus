use std::path::{Path, PathBuf};

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
    executable: Option<PathBuf>,
    arguments: String,
}

impl ShortcutIdentity {
    pub(super) fn from_path(path: &Path) -> Option<Self> {
        is_shortcut(path).then(|| Self {
            app_user_model_id: shortcut_application_id(path)
                .filter(|id| is_reliable_registered_id(id)),
            executable: resolve_executable(&path.to_string_lossy())
                .as_deref()
                .map(normalize_path),
            arguments: shortcut_arguments(path)
                .map(|arguments| normalize_arguments(&arguments))
                .unwrap_or_default(),
        })
    }

    pub(super) fn equivalent_to(&self, other: &Self) -> bool {
        match (&self.app_user_model_id, &other.app_user_model_id) {
            (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
            _ => {
                self.executable.is_some()
                    && self.executable == other.executable
                    && self.arguments == other.arguments
            }
        }
    }

    pub(super) fn preferred_over(&self, other: &Self) -> bool {
        self.preference() < other.preference()
    }

    fn app_user_model_id(&self) -> Option<&str> {
        self.app_user_model_id.as_deref()
    }

    fn preference(&self) -> u8 {
        if process_start_executable(&self.arguments).is_some() {
            return 0;
        }
        if self
            .executable
            .as_deref()
            .is_some_and(is_versioned_app_path)
        {
            return 2;
        }

        1
    }
}

fn is_shortcut(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
}

fn normalize_path(path: &Path) -> PathBuf {
    PathBuf::from(
        path.to_string_lossy()
            .replace('/', "\\")
            .to_ascii_lowercase(),
    )
}

fn normalize_arguments(arguments: &str) -> String {
    arguments
        .split_ascii_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn shortcut_process_start_executable(path: &Path) -> Option<PathBuf> {
    let arguments = shortcut_arguments(path)?;
    process_start_executable(&arguments).map(PathBuf::from)
}

fn process_start_executable(arguments: &str) -> Option<&str> {
    let mut arguments = arguments.split_ascii_whitespace();
    while let Some(argument) = arguments.next() {
        if argument.eq_ignore_ascii_case("--processStart") {
            return arguments.next().map(|value| value.trim_matches('"'));
        }
    }

    None
}

fn is_versioned_app_path(path: &Path) -> bool {
    path.parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().starts_with("app-"))
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::chromium_web_app_identity;

    #[test]
    fn chromium_web_app_shortcuts_accept_launch_switches_and_browser_proxies() {
        let cases = [
            (
                Some("--profile-directory=Default --app-id=abcdefghijkl"),
                Some(Path::new("chrome.exe")),
                true,
            ),
            (
                Some("--app=https://mail.proton.me/"),
                Some(Path::new("chrome.exe")),
                true,
            ),
            (None, Some(Path::new("chrome_proxy.exe")), true),
            (None, Some(Path::new("msedge_proxy.exe")), true),
            (
                Some("--profile-directory=Default"),
                Some(Path::new("chrome.exe")),
                false,
            ),
            (None, Some(Path::new("ordinary.exe")), false),
        ];

        for (arguments, target, expected) in cases {
            assert_eq!(chromium_web_app_identity(arguments, target), expected);
        }
    }
}
