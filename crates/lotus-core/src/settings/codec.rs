use serde_json::Value;
use thiserror::Error;

use super::model::{CURRENT_APPEARANCE_VERSION, DockSettings};

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("invalid Lotus settings: {0}")]
pub struct SettingsDecodeError(String);

impl From<serde_json::Error> for SettingsDecodeError {
    fn from(error: serde_json::Error) -> Self {
        Self(error.to_string())
    }
}

pub fn decode_settings(source: &str) -> Result<DockSettings, SettingsDecodeError> {
    let mut value = decode_compatible_value(source)?;

    repair_legacy_nulls(&mut value);
    repair_legacy_null_strings(&mut value);
    normalize_signed_integer_fields(&mut value)?;

    Ok(serde_json::from_value::<DockSettings>(value)?.normalized())
}

pub(super) fn apply_legacy_migrations(source: &str, settings: &mut DockSettings) -> bool {
    let Ok(value) = decode_compatible_value(source) else {
        return false;
    };
    let Some(object) = value.as_object() else {
        return false;
    };

    let has_appearance_version = object.contains_key("appearanceVersion");
    let needs_appearance_migration =
        !has_appearance_version || settings.appearance_version < 2;
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

fn decode_compatible_value(source: &str) -> Result<Value, SettingsDecodeError> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut compatible_json = strip_json_comments_and_trailing_commas(source).into_bytes();

    canonicalize_property_names_in_json(&mut compatible_json);

    Ok(serde_json::from_slice(&compatible_json)?)
}

fn strip_json_comments_and_trailing_commas(source: &str) -> String {
    let mut bytes = source.as_bytes().to_vec();
    strip_comments(&mut bytes);
    strip_trailing_commas(&mut bytes);

    String::from_utf8(bytes).expect("replacing ASCII JSON syntax preserves UTF-8")
}

fn strip_comments(bytes: &mut [u8]) {
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
                index = erase_line_comment(bytes, index);
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = erase_block_comment(bytes, index);
            }
            _ => index += 1,
        }
    }
}

fn erase_line_comment(bytes: &mut [u8], mut index: usize) -> usize {
    while index < bytes.len() && !matches!(bytes[index], b'\r' | b'\n') {
        bytes[index] = b' ';
        index += 1;
    }

    index
}

fn erase_block_comment(bytes: &mut [u8], mut index: usize) -> usize {
    bytes[index] = b' ';
    bytes[index + 1] = b' ';
    index += 2;

    while index < bytes.len() {
        if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
            bytes[index] = b' ';
            bytes[index + 1] = b' ';
            return index + 2;
        }
        if !matches!(bytes[index], b'\r' | b'\n') {
            bytes[index] = b' ';
        }
        index += 1;
    }

    index
}

fn strip_trailing_commas(bytes: &mut [u8]) {
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
            let next = bytes[index + 1..]
                .iter()
                .copied()
                .find(|byte| !byte.is_ascii_whitespace());
            if matches!(next, Some(b'}' | b']')) {
                bytes[index] = b' ';
            }
        }
    }
}

fn canonicalize_property_names_in_json(json: &mut [u8]) {
    let mut index = 0;

    while index < json.len() {
        if json[index] != b'"' {
            index += 1;
            continue;
        }

        let start = index + 1;
        let Some(end) = find_string_end(json, start) else {
            return;
        };

        if is_property_name(json, end) && !json[start..end].contains(&b'\\') {
            canonicalize_property_name(&mut json[start..end]);
        }

        index = end + 1;
    }
}

fn find_string_end(json: &[u8], start: usize) -> Option<usize> {
    let mut index = start;
    let mut escaped = false;

    while index < json.len() {
        match json[index] {
            b'\\' if !escaped => escaped = true,
            b'"' if !escaped => return Some(index),
            _ => escaped = false,
        }
        index += 1;
    }

    None
}

fn is_property_name(json: &[u8], string_end: usize) -> bool {
    json[string_end + 1..]
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())
        == Some(b':')
}

fn canonicalize_property_name(name: &mut [u8]) {
    for property in PROPERTY_NAMES {
        let property = property.as_bytes();
        if property.len() == name.len() && property.eq_ignore_ascii_case(name) {
            name.copy_from_slice(property);
            return;
        }
    }
}

fn repair_legacy_nulls(value: &mut Value) {
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

    replace_null_string(object, "backgroundColor", "#11141A");
    replace_null_string(object, "accentColor", "#F5A5A5");
    replace_null_string(object, "foregroundColor", "#F7F8FB");

    let Some(pinned_apps) = object.get_mut("pinnedApps").and_then(Value::as_array_mut)
    else {
        return;
    };

    for app in pinned_apps.iter_mut().filter_map(Value::as_object_mut) {
        for property in ["id", "name", "launchTarget"] {
            replace_null_string(app, property, "");
        }
    }
}

fn replace_null_string(
    object: &mut serde_json::Map<String, Value>,
    property: &str,
    replacement: &str,
) {
    if object.get(property).is_some_and(Value::is_null) {
        object.insert(property.to_owned(), Value::String(replacement.to_owned()));
    }
}

fn normalize_signed_integer_fields(value: &mut Value) -> Result<(), SettingsDecodeError> {
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
            return Err(integer_range_error(property));
        };
        if integer < i64::from(i32::MIN) || integer > i64::from(i32::MAX) {
            return Err(integer_range_error(property));
        }

        object.insert(
            property.to_owned(),
            Value::from(integer.clamp(minimum, maximum)),
        );
    }

    Ok(())
}

fn integer_range_error(property: &str) -> SettingsDecodeError {
    SettingsDecodeError(format!(
        "property `{property}` is outside the .NET Int32 range"
    ))
}

const COLLECTION_PROPERTIES: &[&str] = &[
    "hiddenExecutables",
    "notificationDisabledApps",
    "itemOrder",
    "pinnedApps",
    "matchExecutables",
];

const INTEGER_BOUNDS: &[(&str, i64, i64)] = &[
    ("iconSize", 24, 72),
    ("itemSpacing", 2, 24),
    ("horizontalPadding", 4, 48),
    ("verticalPadding", 4, 32),
    ("bottomOffset", 0, 96),
    ("screenEdgeInset", 0, 96),
    ("cornerRadius", 0, 48),
    ("appearanceVersion", 0, 2_147_483_647),
    ("searchResultLimit", 1, 8),
];

const PROPERTY_NAMES: &[&str] = &[
    "iconSize",
    "itemSpacing",
    "horizontalPadding",
    "verticalPadding",
    "bottomOffset",
    "screenEdgeInset",
    "cornerRadius",
    "appearanceVersion",
    "onboardingVersion",
    "backgroundOpacity",
    "backgroundColor",
    "accentColor",
    "foregroundColor",
    "mascotImagePath",
    "showAppDock",
    "showUnpinnedRunningApps",
    "showRunningIndicators",
    "showOnAllMonitors",
    "showDesktopButton",
    "showSystemStatus",
    "dockZone",
    "systemStatusZone",
    "showVolumeStatus",
    "showNetworkStatus",
    "showBackgroundAppsStatus",
    "showDateTimeStatus",
    "showDateInStatus",
    "use24HourTime",
    "showMediaControls",
    "showMediaMetadata",
    "mediaZone",
    "startWithWindows",
    "hideWhenFullscreen",
    "replaceWindowsTaskbar",
    "exclusiveTaskbarReplacement",
    "searchEnabled",
    "searchOpenWithWindowsKey",
    "altTabEnabled",
    "windowPickerStyle",
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
    "appUserModelId",
    "matchExecutables",
];
