use std::collections::HashMap;
use std::path::Path;

use crate::window::TrackedWindowKey;

/// A normalized runtime application identity.  This deliberately keeps registered,
/// launch, executable, and ephemeral identities distinct.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ApplicationKey {
    Registered(String),
    LaunchSignature(String),
    ExecutablePath(String),
    Ephemeral(TrackedWindowKey),
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LaunchSpec {
    pub target: String,
    pub arguments: Option<String>,
}

impl LaunchSpec {
    #[must_use]
    pub fn new(target: impl AsRef<str>, arguments: Option<&str>) -> Option<Self> {
        let target = trimmed_value(target.as_ref())?;
        let arguments = arguments.and_then(trimmed_value);
        Some(Self { target, arguments })
    }

    #[must_use]
    pub fn signature(&self) -> String {
        format!(
            "{}\u{1f}{}",
            normalized_path(&self.target).unwrap_or_default(),
            self.arguments
                .as_deref()
                .and_then(trimmed_value)
                .unwrap_or_default()
        )
    }
}

impl ApplicationKey {
    #[must_use]
    pub fn from_launch_fallback(launch: &LaunchSpec) -> Self {
        let target = Path::new(&launch.target);
        let is_executable = target
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"));
        if launch.arguments.is_none()
            && is_executable
            && let Some(path) = normalized_path(&launch.target)
        {
            return Self::ExecutablePath(path);
        }
        Self::LaunchSignature(launch.signature())
    }
}

#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct WindowApplicationFacts {
    pub window_app_user_model_id: Option<String>,
    pub process_app_user_model_id: Option<String>,
    pub relaunch: Option<LaunchSpec>,
    pub display_name: Option<String>,
    pub icon_resource: Option<String>,
    pub prevent_pinning: bool,
}

impl WindowApplicationFacts {
    #[must_use]
    pub fn reliable_id(&self) -> Option<&str> {
        self.window_app_user_model_id
            .as_deref()
            .filter(|id| is_reliable_registered_id(id))
            .or_else(|| {
                self.process_app_user_model_id
                    .as_deref()
                    .filter(|id| is_reliable_registered_id(id))
            })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolutionEvidence {
    ExplicitAssociation,
    ExactRegisteredId,
    ExactRelaunch,
    ExactProviderKey,
    ExactExecutablePath,
    UniqueExecutableAlias,
    NoMatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationResolution {
    Resolved {
        key: ApplicationKey,
        registered_index: usize,
        evidence: ResolutionEvidence,
    },
    Associated {
        key: ApplicationKey,
    },
    Prevented,
    Ambiguous {
        evidence: ResolutionEvidence,
        candidate_count: usize,
    },
    Unregistered {
        key: ApplicationKey,
        launch: Option<LaunchSpec>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationPresentation {
    pub display_name: String,
    pub icon: ApplicationPresentationIcon,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationPresentationIcon {
    Source(String),
    NativeWindow {
        key: TrackedWindowKey,
        fallback_path: String,
    },
}

impl ApplicationPresentationIcon {
    #[must_use]
    pub fn fallback_path(&self) -> &str {
        match self {
            Self::Source(path)
            | Self::NativeWindow {
                fallback_path: path,
                ..
            } => path,
        }
    }

    #[must_use]
    pub const fn native_window(&self) -> Option<TrackedWindowKey> {
        match self {
            Self::Source(_) => None,
            Self::NativeWindow { key, .. } => Some(*key),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct WindowApplicationAssignments {
    pub catalog_generation: u64,
    pub window_revision: u64,
    pub by_window: HashMap<TrackedWindowKey, ApplicationResolution>,
    pub presentation_by_window: HashMap<TrackedWindowKey, ApplicationPresentation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PinnedApplicationAssignment {
    pub pin_id: String,
    pub key: ApplicationKey,
    pub registered_index: Option<usize>,
}

/// A fully materialized installed application record. Windows-specific discovery owns
/// constructing these records; all consumers only read their already-normalized facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisteredApplication {
    pub key: ApplicationKey,
    pub id: String,
    pub name: String,
    pub launch: LaunchSpec,
    pub launch_aliases: Vec<LaunchSpec>,
    pub icon_source: String,
    pub app_user_model_id: Option<String>,
    pub canonical_executables: Vec<String>,
    pub executable_aliases: Vec<String>,
    pub provider_keys: Vec<String>,
}

impl RegisteredApplication {
    #[must_use]
    pub fn application_identity(&self) -> ApplicationIdentity {
        ApplicationIdentity::new(
            self.app_user_model_id.as_deref(),
            Some(&self.id),
            self.canonical_executables.first().map(String::as_str),
            self.executable_aliases.iter().map(String::as_str),
        )
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub enum ApplicationMatchStrength {
    #[default]
    None,
    ExecutableAlias,
    ExecutablePath,
    StableId,
    RegisteredId,
}

impl ApplicationMatchStrength {
    #[must_use]
    pub const fn is_match(self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ApplicationIdentity {
    registered_id: Option<String>,
    stable_id: Option<String>,
    executable_path: Option<String>,
    executable_aliases: Vec<String>,
}

impl ApplicationIdentity {
    #[must_use]
    pub fn new<'a>(
        registered_id: Option<&str>,
        stable_id: Option<&str>,
        executable_path: Option<&str>,
        executable_aliases: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        let executable_path = executable_path.and_then(normalized_path);
        let mut aliases = executable_aliases
            .into_iter()
            .filter_map(normalized_executable_name)
            .collect::<Vec<_>>();
        if let Some(path) = executable_path.as_deref()
            && let Some(name) = executable_name(path)
        {
            push_unique(&mut aliases, name);
        }

        Self {
            registered_id: registered_id
                .filter(|value| is_reliable_registered_id(value))
                .map(normalized),
            stable_id: stable_id.and_then(normalized_value),
            executable_path,
            executable_aliases: aliases,
        }
    }

    #[must_use]
    pub fn from_path<'a>(
        registered_id: Option<&str>,
        stable_id: Option<&str>,
        executable_path: Option<&Path>,
        executable_aliases: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        let executable_path = executable_path.map(|path| path.to_string_lossy());
        Self::new(
            registered_id,
            stable_id,
            executable_path.as_deref(),
            executable_aliases,
        )
    }

    #[must_use]
    pub fn match_strength(&self, other: &Self) -> ApplicationMatchStrength {
        if let (Some(left), Some(right)) = (&self.registered_id, &other.registered_id) {
            return if left == right {
                ApplicationMatchStrength::RegisteredId
            } else {
                ApplicationMatchStrength::None
            };
        }

        if let (Some(left), Some(right)) = (&self.stable_id, &other.stable_id)
            && left == right
        {
            return ApplicationMatchStrength::StableId;
        }

        if self.registered_id.is_some() || other.registered_id.is_some() {
            return ApplicationMatchStrength::None;
        }

        if self.is_shared_host() || other.is_shared_host() {
            return ApplicationMatchStrength::None;
        }

        if let (Some(left), Some(right)) = (&self.executable_path, &other.executable_path)
            && left == right
        {
            return ApplicationMatchStrength::ExecutablePath;
        }

        if self
            .executable_aliases
            .iter()
            .any(|left| other.executable_aliases.iter().any(|right| left == right))
        {
            ApplicationMatchStrength::ExecutableAlias
        } else {
            ApplicationMatchStrength::None
        }
    }

    #[must_use]
    pub fn deduplication_key(&self) -> Option<String> {
        self.registered_id
            .as_ref()
            .map(|value| format!("registered:{value}"))
            .or_else(|| {
                self.stable_id
                    .as_ref()
                    .map(|value| format!("stable:{value}"))
            })
            .or_else(|| {
                self.executable_path
                    .as_ref()
                    .map(|value| format!("path:{value}"))
            })
    }

    #[must_use]
    pub fn process_group_key(&self) -> Option<String> {
        if self.is_shared_host() {
            return self
                .registered_id
                .as_ref()
                .map(|value| format!("registered:{value}"))
                .or_else(|| {
                    self.stable_id
                        .as_ref()
                        .map(|value| format!("stable:{value}"))
                });
        }

        self.executable_path
            .as_ref()
            .map(|value| format!("path:{value}"))
            .or_else(|| {
                self.registered_id
                    .as_ref()
                    .map(|value| format!("registered:{value}"))
            })
            .or_else(|| {
                self.stable_id
                    .as_ref()
                    .map(|value| format!("stable:{value}"))
            })
    }

    #[must_use]
    pub fn has_executable_alias(&self, executable: &str) -> bool {
        normalized_executable_name(executable).is_some_and(|candidate| {
            self.executable_aliases
                .iter()
                .any(|alias| alias == &candidate)
        })
    }

    #[must_use]
    pub fn reliable_registered_id(&self) -> Option<&str> {
        self.registered_id.as_deref()
    }

    #[must_use]
    pub fn stable_id(&self) -> Option<&str> {
        self.stable_id.as_deref()
    }

    pub fn identifiers(&self) -> impl Iterator<Item = &str> {
        self.registered_id
            .iter()
            .chain(self.stable_id.iter())
            .map(String::as_str)
    }

    #[must_use]
    pub fn is_shared_host(&self) -> bool {
        self.executable_aliases
            .iter()
            .any(|alias| is_shared_host_executable(alias))
    }
}

#[must_use]
pub fn is_reliable_application_identity(value: &str) -> bool {
    is_reliable_registered_id(value)
}

#[must_use]
pub fn is_reliable_registered_id(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && !value.eq_ignore_ascii_case("com.electron.app")
        && !value.contains(['\\', '/'])
        && !value.contains("://")
        && !Path::new(value).extension().is_some_and(|extension| {
            extension.eq_ignore_ascii_case("exe") || extension.eq_ignore_ascii_case("lnk")
        })
}

#[must_use]
pub fn is_shared_host_executable(value: &str) -> bool {
    normalized_executable_name(value).is_some_and(|executable| {
        matches!(
            executable.as_str(),
            "chrome.exe" | "msedge.exe" | "applicationframehost.exe"
        )
    })
}

#[must_use]
pub fn application_provider_keys(
    registered_id: Option<&str>,
    arguments: Option<&str>,
) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(id) = registered_id
        .filter(|id| is_reliable_registered_id(id))
        .filter(|id| id.to_ascii_lowercase().starts_with("com.squirrel."))
    {
        keys.push(format!("squirrel:registered:{}", id.to_lowercase()));
    }
    let mut arguments = arguments.unwrap_or_default().split_ascii_whitespace();
    while let Some(argument) = arguments.next() {
        let (name, inline_value) = argument
            .split_once('=')
            .map_or((argument, None), |(name, value)| (name, Some(value)));
        let value = inline_value.or_else(|| {
            (name.eq_ignore_ascii_case("--app-id") || name.eq_ignore_ascii_case("--app"))
                .then(|| arguments.next())
                .flatten()
        });
        let Some(value) = value.map(|value| value.trim_matches('"')) else {
            continue;
        };
        if name.eq_ignore_ascii_case("--app-id") {
            keys.push(format!("chromium-id:{}", value.to_lowercase()));
        } else if name.eq_ignore_ascii_case("--app") {
            keys.push(format!("chromium-url:{value}"));
        }
    }
    keys.sort();
    keys.dedup();
    keys
}

#[must_use]
pub fn normalized_path(value: &str) -> Option<String> {
    normalized_value(value).map(|value| value.replace('/', "\\"))
}

#[must_use]
pub fn normalized_executable_name(value: &str) -> Option<String> {
    let value = value.rsplit(['\\', '/']).next().unwrap_or(value);
    normalized_value(value)
}

#[must_use]
pub fn normalized_value(value: &str) -> Option<String> {
    trimmed_value(value).map(|value| normalized(&value))
}

fn trimmed_value(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn normalized(value: &str) -> String {
    value.to_lowercase()
}

fn executable_name(path: &str) -> Option<&str> {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    (!name.is_empty()).then_some(name)
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    let value = normalized(value);
    if !values.iter().any(|saved| saved == &value) {
        values.push(value);
    }
}
