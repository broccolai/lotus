use std::collections::HashMap;
use std::path::Path;

use crate::application::{ApplicationIdentity, is_reliable_application_identity};
use crate::settings::PinnedApp;
use crate::window::WindowInfo;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DockItem {
    pub id: String,
    pub display_name: String,
    pub launch_target: String,
    pub arguments: Option<String>,
    pub executable_path: String,
    pub icon_source: String,
    pub app_user_model_id: Option<String>,
    pub is_pinned: bool,
    pub windows: Vec<WindowInfo>,
}

impl DockItem {
    #[must_use]
    pub fn application_identity(&self) -> ApplicationIdentity {
        ApplicationIdentity::new(
            self.app_user_model_id.as_deref(),
            Some(&self.id),
            Some(&self.executable_path),
            std::iter::empty(),
        )
    }

    pub fn is_running(&self) -> bool {
        !self.windows.is_empty()
    }

    pub fn initial(&self) -> String {
        self.display_name.chars().next().map_or_else(
            || "?".into(),
            |character| character.to_uppercase().collect(),
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DockProjection<'a> {
    pub pinned_apps: &'a [PinnedApp],
    pub hidden_executables: &'a [String],
    pub item_order: &'a [String],
    pub show_unpinned_running_apps: bool,
}

pub fn project_dock<F>(
    windows: &[WindowInfo],
    settings: DockProjection<'_>,
    mut resolve_executable: F,
) -> Vec<DockItem>
where
    F: FnMut(&str) -> Option<String>,
{
    let visible_windows = windows
        .iter()
        .filter(|window| {
            !settings
                .hidden_executables
                .iter()
                .any(|hidden| window.application_identity().has_executable_alias(hidden))
        })
        .collect::<Vec<_>>();
    let mut unmatched = vec![true; visible_windows.len()];
    let mut items = Vec::new();

    for pinned in settings.pinned_apps {
        let resolved_launch = resolve_executable(&pinned.launch_target);
        let mut matches = Vec::new();
        for (index, window) in visible_windows.iter().enumerate() {
            if unmatched[index] && matches_pin(pinned, window, resolved_launch.as_deref()) {
                unmatched[index] = false;
                matches.push((*window).clone());
            }
        }
        let executable_path = matches.first().map_or_else(
            || {
                resolved_launch
                    .clone()
                    .unwrap_or_else(|| pinned.launch_target.clone())
            },
            |window| path_text(&window.executable_path),
        );

        items.push(DockItem {
            id: pinned.id.clone(),
            display_name: pinned.name.clone(),
            launch_target: pinned.launch_target.clone(),
            arguments: pinned.arguments.clone(),
            icon_source: pinned
                .icon_source
                .clone()
                .unwrap_or_else(|| executable_path.clone()),
            app_user_model_id: pinned
                .app_user_model_id
                .as_deref()
                .filter(|identity| is_reliable_application_identity(identity))
                .map(str::to_owned)
                .or_else(|| {
                    matches.iter().find_map(|window| {
                        window
                            .app_user_model_id
                            .as_deref()
                            .filter(|identity| is_reliable_application_identity(identity))
                            .map(str::to_owned)
                    })
                }),
            executable_path,
            is_pinned: true,
            windows: matches,
        });
    }

    if settings.show_unpinned_running_apps {
        append_unpinned(&mut items, &visible_windows, &unmatched);
    }

    apply_saved_order(&mut items, settings.item_order);
    items
}

fn matches_pin(
    pinned: &PinnedApp,
    window: &WindowInfo,
    resolved_launch: Option<&str>,
) -> bool {
    pinned
        .match_identity(&window.application_identity(), resolved_launch)
        .is_match()
}

fn append_unpinned(
    items: &mut Vec<DockItem>,
    visible_windows: &[&WindowInfo],
    unmatched: &[bool],
) {
    let mut group_indices = HashMap::<String, usize>::new();
    let mut groups = Vec::<(String, String, Vec<WindowInfo>)>::new();

    for (window, is_unmatched) in visible_windows.iter().zip(unmatched) {
        if !is_unmatched {
            continue;
        }

        let executable_path = path_text(&window.executable_path);
        let identity = window.application_identity();
        let key = identity
            .process_group_key()
            .unwrap_or_else(|| format!("window:{}", window.id.get()));
        let id = if identity.is_shared_host() {
            identity
                .reliable_registered_id()
                .or_else(|| identity.stable_id())
                .map_or_else(|| key.clone(), str::to_owned)
        } else {
            executable_path.clone()
        };
        let index = *group_indices.entry(key).or_insert_with(|| {
            groups.push((id, executable_path, Vec::new()));
            groups.len() - 1
        });
        groups[index].2.push((*window).clone());
    }

    groups.sort_by(|left, right| {
        case_key(&file_stem(&left.1))
            .cmp(&case_key(&file_stem(&right.1)))
            .then_with(|| case_key(&left.1).cmp(&case_key(&right.1)))
            .then_with(|| left.1.cmp(&right.1))
    });

    items.extend(groups.into_iter().map(|(id, executable_path, windows)| {
        DockItem {
            id,
            display_name: file_stem(&executable_path),
            launch_target: executable_path.clone(),
            arguments: None,
            icon_source: executable_path.clone(),
            app_user_model_id: windows
                .iter()
                .find_map(|window| window.app_user_model_id.clone()),
            executable_path,
            is_pinned: false,
            windows,
        }
    }));
}

fn apply_saved_order(items: &mut [DockItem], saved_order: &[String]) {
    let mut order = HashMap::new();
    for (index, id) in saved_order.iter().enumerate() {
        order.entry(case_key(id)).or_insert(index);
    }

    items.sort_by_key(|item| {
        order
            .get(&case_key(&item.id))
            .copied()
            .unwrap_or(usize::MAX)
    });
}

fn file_stem(path: &str) -> String {
    if path.eq_ignore_ascii_case(
        r"shell:AppsFolder\windows.immersivecontrolpanel_cw5n1h2txyewy!microsoft.windows.immersivecontrolpanel",
    ) {
        return "Settings".to_owned();
    }
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    name.rsplit_once('.')
        .map_or(name, |(stem, _)| stem)
        .to_owned()
}

fn case_key(value: &str) -> String {
    value.to_lowercase()
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::WindowId;

    #[test]
    fn stable_application_identity_matches_a_pin_across_helper_executable_changes() {
        let pin = PinnedApp {
            id: "discord".into(),
            name: "Discord".into(),
            launch_target: r"C:\ProgramData\Microsoft\Windows\Start Menu\Discord.lnk"
                .into(),
            arguments: None,
            icon_source: None,
            app_user_model_id: Some("com.squirrel.discord.discord".into()),
            match_executables: vec!["Discord.exe".into()],
        };
        let window = WindowInfo {
            id: WindowId::new(1),
            process_id: 1,
            title: "Discord".into(),
            executable_path: r"C:\Users\someone\AppData\Local\Discord\app-2.0\helper.exe"
                .into(),
            app_user_model_id: Some("COM.SQUIRREL.DISCORD.DISCORD".into()),
        };

        assert!(matches_pin(&pin, &window, None));
    }
}
