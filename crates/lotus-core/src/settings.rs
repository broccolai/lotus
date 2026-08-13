use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub const CURRENT_APPEARANCE_VERSION: u32 = 3;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NotificationBadgeStyle {
    #[default]
    Off,
    Dot,
    Count,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("invalid Lotus settings: {0}")]
pub struct SettingsDecodeError(String);

impl From<serde_json::Error> for SettingsDecodeError {
    fn from(error: serde_json::Error) -> Self {
        Self(error.to_string())
    }
}

#[derive(Debug, Error)]
pub enum SettingsStoreError {
    #[error("could not {operation} `{path}`: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not encode Lotus settings: {0}")]
    Encode(#[from] serde_json::Error),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SettingsLoadSource {
    CreatedDefaults,
    Existing,
    Migrated,
    RecoveredInvalid { backup_path: PathBuf, error: SettingsDecodeError },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsLoad {
    pub settings: DockSettings,
    pub source: SettingsLoadSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsStore {
    directory: PathBuf,
}

impl SettingsStore {
    #[must_use]
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self { directory: directory.into() }
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    #[must_use]
    pub fn settings_path(&self) -> PathBuf {
        self.directory.join("settings.json")
    }

    pub fn load(&self) -> Result<SettingsLoad, SettingsStoreError> {
        self.ensure_directory()?;
        let path = self.settings_path();

        if !path.exists() {
            let settings = DockSettings::default().normalized();
            self.save(&settings)?;
            return Ok(SettingsLoad { settings, source: SettingsLoadSource::CreatedDefaults });
        }

        let source = fs::read_to_string(&path)
            .map_err(|error| store_io("read settings from", &path, error))?;
        let error = match decode_settings(&source) {
            Ok(mut settings) => {
                let migrated = apply_legacy_migrations(&source, &mut settings);
                if migrated {
                    self.save(&settings)?;
                }
                return Ok(SettingsLoad {
                    settings,
                    source: if migrated {
                        SettingsLoadSource::Migrated
                    } else {
                        SettingsLoadSource::Existing
                    },
                });
            }
            Err(error) => error,
        };

        let backup_path = self.invalid_backup_path();
        fs::copy(&path, &backup_path)
            .map_err(|error| store_io("back up invalid settings to", &backup_path, error))?;
        Ok(SettingsLoad {
            settings: DockSettings::default().normalized(),
            source: SettingsLoadSource::RecoveredInvalid { backup_path, error },
        })
    }

    pub fn save(&self, settings: &DockSettings) -> Result<(), SettingsStoreError> {
        self.ensure_directory()?;
        let path = self.settings_path();
        let settings = settings.clone().normalized();
        let mut json = serde_json::to_string_pretty(&settings)?;
        json.push('\n');

        let mut file = AtomicWriteFile::open(&path)
            .map_err(|error| store_io("open settings for atomic write at", &path, error))?;
        file.write_all(json.as_bytes())
            .map_err(|error| store_io("write settings to", &path, error))?;
        file.commit().map_err(|error| store_io("commit settings at", &path, error))
    }

    fn ensure_directory(&self) -> Result<(), SettingsStoreError> {
        fs::create_dir_all(&self.directory)
            .map_err(|error| store_io("create settings directory at", &self.directory, error))
    }

    fn invalid_backup_path(&self) -> PathBuf {
        let timestamp =
            SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |duration| duration.as_millis());
        let base = self.directory.join(format!("settings.json.invalid-{timestamp}"));
        unique_path(base)
    }
}

pub fn decode_settings(source: &str) -> Result<DockSettings, SettingsDecodeError> {
    let mut value = decode_compatible_value(source)?;
    repair_legacy_nulls(&mut value);
    repair_legacy_null_strings(&mut value);
    normalize_signed_integer_fields(&mut value)?;
    Ok(serde_json::from_value::<DockSettings>(value)?.normalized())
}

fn decode_compatible_value(source: &str) -> Result<Value, SettingsDecodeError> {
    let mut compatible_json = strip_json_comments_and_trailing_commas(source).into_bytes();
    canonicalize_property_names_in_json(&mut compatible_json);
    Ok(serde_json::from_slice(&compatible_json)?)
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent persisted preferences are not mutually exclusive state"
)]
pub struct DockSettings {
    pub icon_size: u32,
    pub item_spacing: u32,
    pub horizontal_padding: u32,
    pub vertical_padding: u32,
    pub bottom_offset: u32,
    pub corner_radius: u32,
    pub appearance_version: u32,
    pub background_opacity: f64,
    pub background_color: String,
    pub accent_color: String,
    pub mascot_image_path: Option<String>,
    pub show_unpinned_running_apps: bool,
    pub show_desktop_button: bool,
    pub start_with_windows: bool,
    pub hide_when_fullscreen: bool,
    pub replace_windows_taskbar: bool,
    pub exclusive_taskbar_replacement: bool,
    pub search_open_with_windows_key: bool,
    pub alt_tab_enabled: bool,
    pub notification_badge_style: NotificationBadgeStyle,
    pub notification_disabled_apps: Vec<String>,
    pub search_result_limit: u32,
    pub application_name_overrides: BTreeMap<String, String>,
    pub hidden_executables: Vec<String>,
    pub item_order: Vec<String>,
    pub pinned_apps: Vec<PinnedApp>,
}

impl Default for DockSettings {
    fn default() -> Self {
        Self {
            icon_size: 38,
            item_spacing: 8,
            horizontal_padding: 12,
            vertical_padding: 8,
            bottom_offset: 10,
            corner_radius: 8,
            appearance_version: CURRENT_APPEARANCE_VERSION,
            background_opacity: 0.56,
            background_color: "#11141A".into(),
            accent_color: "#F5A5A5".into(),
            mascot_image_path: None,
            show_unpinned_running_apps: true,
            show_desktop_button: false,
            start_with_windows: true,
            hide_when_fullscreen: true,
            replace_windows_taskbar: true,
            exclusive_taskbar_replacement: false,
            search_open_with_windows_key: true,
            alt_tab_enabled: false,
            notification_badge_style: NotificationBadgeStyle::Off,
            notification_disabled_apps: Vec::new(),
            search_result_limit: 5,
            application_name_overrides: BTreeMap::new(),
            hidden_executables: Vec::new(),
            item_order: Vec::new(),
            pinned_apps: Vec::new(),
        }
    }
}

impl DockSettings {
    #[must_use]
    pub fn normalized(mut self) -> Self {
        self.icon_size = self.icon_size.clamp(24, 72);
        self.item_spacing = self.item_spacing.clamp(2, 24);
        self.horizontal_padding = self.horizontal_padding.clamp(4, 48);
        self.vertical_padding = self.vertical_padding.clamp(4, 32);
        self.bottom_offset = self.bottom_offset.min(96);
        self.corner_radius = self.corner_radius.min(48);
        self.search_result_limit = self.search_result_limit.clamp(1, 8);
        self.background_opacity = self.background_opacity.clamp(0.08, 0.95);

        if !is_hex_color(&self.background_color) {
            self.background_color = "#11141A".into();
        }
        if !is_hex_color(&self.accent_color) {
            self.accent_color = "#F5A5A5".into();
        }
        self.mascot_image_path = self
            .mascot_image_path
            .and_then(|path| (!path.trim().is_empty()).then(|| path.trim().to_owned()));
        self.notification_disabled_apps =
            normalized_unique_strings(self.notification_disabled_apps);

        self.pinned_apps.retain(PinnedApp::is_launchable);
        for app in &mut self.pinned_apps {
            app.normalize();
        }

        self
    }

    pub fn dock_height(&self) -> u32 {
        self.icon_size + self.vertical_padding * 2
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PinnedApp {
    pub id: String,
    pub name: String,
    pub launch_target: String,
    pub arguments: Option<String>,
    pub icon_source: Option<String>,
    pub match_executables: Vec<String>,
}

impl PinnedApp {
    fn is_launchable(&self) -> bool {
        !self.id.trim().is_empty() && !self.launch_target.trim().is_empty()
    }

    fn normalize(&mut self) {
        self.id = self.id.trim().into();
        self.name = match self.name.trim() {
            "" => "Application".into(),
            name => name.into(),
        };
        self.launch_target = self.launch_target.trim().into();
    }
}

fn is_hex_color(value: &str) -> bool {
    let Some(hex) = value.strip_prefix('#') else {
        return false;
    };

    matches!(hex.len(), 6 | 8) && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn normalized_unique_strings(values: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let value = value.trim();
        if !value.is_empty()
            && !normalized.iter().any(|saved: &String| saved.eq_ignore_ascii_case(value))
        {
            normalized.push(value.to_owned());
        }
    }
    normalized
}

fn strip_json_comments_and_trailing_commas(source: &str) -> String {
    let mut bytes = source.as_bytes().to_vec();
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;

    while index < bytes.len() {
        if in_string {
            match bytes[index] {
                b'\\' if !escaped => escaped = true,
                b'"' if !escaped => in_string = false,
                _ => escaped = false,
            }
            index += 1;
            continue;
        }

        match bytes[index] {
            b'"' => {
                in_string = true;
                index += 1;
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                bytes[index] = b' ';
                bytes[index + 1] = b' ';
                index += 2;
                while index < bytes.len() && !matches!(bytes[index], b'\r' | b'\n') {
                    bytes[index] = b' ';
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                bytes[index] = b' ';
                bytes[index + 1] = b' ';
                index += 2;
                while index < bytes.len() {
                    if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
                        bytes[index] = b' ';
                        bytes[index + 1] = b' ';
                        index += 2;
                        break;
                    }
                    if !matches!(bytes[index], b'\r' | b'\n') {
                        bytes[index] = b' ';
                    }
                    index += 1;
                }
            }
            _ => index += 1,
        }
    }

    let mut in_string = false;
    let mut escaped = false;
    for index in 0..bytes.len() {
        if in_string {
            match bytes[index] {
                b'\\' if !escaped => escaped = true,
                b'"' if !escaped => in_string = false,
                _ => escaped = false,
            }
        } else if bytes[index] == b'"' {
            in_string = true;
        } else if bytes[index] == b',' {
            let next = bytes[index + 1..].iter().copied().find(|byte| !byte.is_ascii_whitespace());
            if matches!(next, Some(b'}' | b']')) {
                bytes[index] = b' ';
            }
        }
    }

    String::from_utf8(bytes).expect("replacing ASCII JSON syntax preserves UTF-8")
}

fn canonicalize_property_names_in_json(json: &mut [u8]) {
    const PROPERTY_NAMES: &[&str] = &[
        "iconSize",
        "itemSpacing",
        "horizontalPadding",
        "verticalPadding",
        "bottomOffset",
        "cornerRadius",
        "appearanceVersion",
        "backgroundOpacity",
        "backgroundColor",
        "accentColor",
        "mascotImagePath",
        "showUnpinnedRunningApps",
        "showDesktopButton",
        "startWithWindows",
        "hideWhenFullscreen",
        "replaceWindowsTaskbar",
        "exclusiveTaskbarReplacement",
        "searchOpenWithWindowsKey",
        "altTabEnabled",
        "notificationBadgeStyle",
        "notificationDisabledApps",
        "searchResultLimit",
        "applicationNameOverrides",
        "hiddenExecutables",
        "itemOrder",
        "pinnedApps",
        "id",
        "name",
        "launchTarget",
        "arguments",
        "iconSource",
        "matchExecutables",
    ];

    let mut index = 0;
    while index < json.len() {
        if json[index] != b'"' {
            index += 1;
            continue;
        }

        let start = index + 1;
        index = start;
        let mut escaped = false;
        while index < json.len() {
            match json[index] {
                b'\\' if !escaped => escaped = true,
                b'"' if !escaped => break,
                _ => escaped = false,
            }
            index += 1;
        }
        if index == json.len() {
            return;
        }

        let end = index;
        let mut after = end + 1;
        while json.get(after).is_some_and(u8::is_ascii_whitespace) {
            after += 1;
        }
        if json.get(after) == Some(&b':') && !json[start..end].contains(&b'\\') {
            for property in PROPERTY_NAMES {
                let property = property.as_bytes();
                if property.len() == end - start && property.eq_ignore_ascii_case(&json[start..end])
                {
                    json[start..end].copy_from_slice(property);
                    break;
                }
            }
        }
        index = end + 1;
    }
}

fn repair_legacy_nulls(value: &mut Value) {
    const COLLECTION_PROPERTIES: &[&str] = &[
        "hiddenExecutables",
        "notificationDisabledApps",
        "itemOrder",
        "pinnedApps",
        "matchExecutables",
    ];

    match value {
        Value::Object(object) => {
            for property in COLLECTION_PROPERTIES {
                if object.get(*property).is_some_and(Value::is_null) {
                    object.insert((*property).to_owned(), Value::Array(Vec::new()));
                }
            }
            object.values_mut().for_each(repair_legacy_nulls);
        }
        Value::Array(values) => values.iter_mut().for_each(repair_legacy_nulls),
        _ => {}
    }
}

fn repair_legacy_null_strings(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };

    if object.get("backgroundColor").is_some_and(Value::is_null) {
        object.insert("backgroundColor".to_owned(), Value::String("#11141A".to_owned()));
    }
    if object.get("accentColor").is_some_and(Value::is_null) {
        object.insert("accentColor".to_owned(), Value::String("#F5A5A5".to_owned()));
    }

    let Some(pinned_apps) = object.get_mut("pinnedApps").and_then(Value::as_array_mut) else {
        return;
    };
    for app in pinned_apps.iter_mut().filter_map(Value::as_object_mut) {
        for property in ["id", "name", "launchTarget"] {
            if app.get(property).is_some_and(Value::is_null) {
                app.insert(property.to_owned(), Value::String(String::new()));
            }
        }
    }
}

fn normalize_signed_integer_fields(value: &mut Value) -> Result<(), SettingsDecodeError> {
    const INTEGER_BOUNDS: &[(&str, i64, i64)] = &[
        ("iconSize", 24, 72),
        ("itemSpacing", 2, 24),
        ("horizontalPadding", 4, 48),
        ("verticalPadding", 4, 32),
        ("bottomOffset", 0, 96),
        ("cornerRadius", 0, 48),
        ("appearanceVersion", 0, 2_147_483_647),
        ("searchResultLimit", 1, 8),
    ];

    let Some(object) = value.as_object_mut() else {
        return Ok(());
    };
    for &(property, minimum, maximum) in INTEGER_BOUNDS {
        let Some(number) = object.get(property).and_then(Value::as_number) else {
            continue;
        };
        if number.is_f64() {
            continue;
        }
        let Some(integer) = number.as_i64() else {
            return Err(SettingsDecodeError(format!(
                "property `{property}` is outside the .NET Int32 range"
            )));
        };
        if integer < i64::from(i32::MIN) || integer > i64::from(i32::MAX) {
            return Err(SettingsDecodeError(format!(
                "property `{property}` is outside the .NET Int32 range"
            )));
        }
        object.insert(property.to_owned(), Value::from(integer.clamp(minimum, maximum)));
    }
    Ok(())
}

fn apply_legacy_migrations(source: &str, settings: &mut DockSettings) -> bool {
    let Ok(value) = decode_compatible_value(source) else {
        return false;
    };
    let Some(object) = value.as_object() else {
        return false;
    };

    let has_appearance_version = object.contains_key("appearanceVersion");
    let needs_appearance_migration = !has_appearance_version || settings.appearance_version < 2;
    let needs_frosted_material_migration =
        !has_appearance_version || settings.appearance_version < CURRENT_APPEARANCE_VERSION;

    if needs_appearance_migration {
        settings.corner_radius = 8;
        settings.icon_size = 38;
        settings.item_spacing = 8;
        settings.horizontal_padding = 12;
        settings.vertical_padding = 8;
    }
    if needs_frosted_material_migration {
        settings.background_opacity = settings.background_opacity.max(0.56);
        settings.appearance_version = CURRENT_APPEARANCE_VERSION;
    }

    let changed = needs_appearance_migration || needs_frosted_material_migration;
    if changed {
        *settings = settings.clone().normalized();
    }
    changed
}

fn unique_path(base: PathBuf) -> PathBuf {
    if !base.exists() {
        return base;
    }

    for suffix in 1_u32.. {
        let candidate = base.with_extension(format!(
            "{}-{suffix}",
            base.extension().and_then(|extension| extension.to_str()).unwrap_or("invalid")
        ));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("u32 path suffixes cannot be exhausted in practice")
}

fn store_io(operation: &'static str, path: &Path, source: io::Error) -> SettingsStoreError {
    SettingsStoreError::Io { operation, path: path.to_owned(), source }
}
