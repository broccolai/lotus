use std::path::Path;

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

fn normalized_path(value: &str) -> Option<String> {
    normalized_value(value).map(|value| value.replace('/', "\\"))
}

fn normalized_executable_name(value: &str) -> Option<String> {
    let value = value.rsplit(['\\', '/']).next().unwrap_or(value);
    normalized_value(value)
}

fn normalized_value(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| normalized(value))
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
