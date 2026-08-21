use std::path::Path;

use crate::dock::DockItem;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationSource {
    pub display_name: String,
    pub app_user_model_id: String,
    pub package_family_name: String,
    pub count: u32,
    pub count_is_lower_bound: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NotificationCount {
    pub value: u32,
    pub is_lower_bound: bool,
}

pub fn count_for_item(
    item: &DockItem,
    sources: &[NotificationSource],
    disabled_apps: &[String],
) -> NotificationCount {
    let candidates = item_candidates(item);
    if disabled_apps
        .iter()
        .map(|value| normalized(value))
        .any(|disabled| {
            !disabled.is_empty()
                && candidates.iter().any(|candidate| candidate == &disabled)
        })
    {
        return NotificationCount::default();
    }

    sources
        .iter()
        .filter(|source| source_matches(source, &candidates))
        .fold(NotificationCount::default(), |total, source| {
            NotificationCount {
                value: total.value.saturating_add(source.count),
                is_lower_bound: total.is_lower_bound || source.count_is_lower_bound,
            }
        })
}

fn source_matches(source: &NotificationSource, candidates: &[String]) -> bool {
    let display_name = normalized(&source.display_name);
    if !display_name.is_empty()
        && candidates
            .iter()
            .any(|candidate| candidate == &display_name)
    {
        return true;
    }

    let model_id = normalized(&source.app_user_model_id);
    let family = normalized(&source.package_family_name);
    candidates.iter().any(|candidate| {
        candidate.len() >= 4 && (model_id.contains(candidate) || family.contains(candidate))
    })
}

fn item_candidates(item: &DockItem) -> Vec<String> {
    let mut candidates = Vec::new();
    let identity = item.application_identity();
    for value in [
        item.display_name.as_str(),
        file_stem(&item.executable_path),
        file_stem(&item.launch_target),
    ]
    .into_iter()
    .chain(identity.identifiers())
    {
        let value = normalized(value);
        if !value.is_empty() && !candidates.contains(&value) {
            candidates.push(value);
        }
    }
    candidates
}

fn file_stem(value: &str) -> &str {
    Path::new(value)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(value)
}

fn normalized(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}
