use crate::application::ApplicationKey;
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
    mut identifier_key: impl FnMut(&str) -> Option<ApplicationKey>,
) -> NotificationCount {
    if disabled_apps
        .iter()
        .filter_map(|disabled| identifier_key(disabled))
        .any(|disabled| disabled == item.application_key)
    {
        return NotificationCount::default();
    }

    sources
        .iter()
        .filter(|source| {
            identifier_key(&source.app_user_model_id)
                .or_else(|| identifier_key(&source.package_family_name))
                .as_ref()
                == Some(&item.application_key)
        })
        .fold(NotificationCount::default(), |total, source| {
            NotificationCount {
                value: total.value.saturating_add(source.count),
                is_lower_bound: total.is_lower_bound || source.count_is_lower_bound,
            }
        })
}
