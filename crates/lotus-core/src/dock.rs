use std::collections::HashMap;
use std::path::Path;

use crate::application::{
    ApplicationIdentity, ApplicationKey, ApplicationPresentationIcon,
    ApplicationResolution, LaunchSpec, PinnedApplicationAssignment, RegisteredApplication,
    WindowApplicationAssignments, is_reliable_application_identity, normalized_path,
};
use crate::settings::PinnedApp;
use crate::window::WindowInfo;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DockItem {
    pub application_key: ApplicationKey,
    pub id: String,
    pub display_name: String,
    pub launch_target: String,
    pub arguments: Option<String>,
    pub executable_path: String,
    pub icon_source: String,
    pub presentation_icon: ApplicationPresentationIcon,
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
    pub assignments: &'a WindowApplicationAssignments,
    pub applications: &'a [RegisteredApplication],
    pub pinned_applications: &'a [PinnedApplicationAssignment],
}

pub fn project_dock(windows: &[WindowInfo], settings: DockProjection<'_>) -> Vec<DockItem> {
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
        let key = settings
            .pinned_applications
            .iter()
            .find(|assignment| assignment.pin_id.eq_ignore_ascii_case(&pinned.id))
            .map_or_else(
                || {
                    LaunchSpec::new(&pinned.launch_target, pinned.arguments.as_deref())
                        .map_or_else(
                            || {
                                ApplicationKey::ExecutablePath(
                                    normalized_path(&pinned.launch_target).unwrap_or_else(
                                        || pinned.launch_target.to_lowercase(),
                                    ),
                                )
                            },
                            |launch| ApplicationKey::from_launch_fallback(&launch),
                        )
                },
                |assignment| assignment.key.clone(),
            );
        let registered = settings
            .pinned_applications
            .iter()
            .find(|assignment| assignment.pin_id.eq_ignore_ascii_case(&pinned.id))
            .and_then(|assignment| assignment.registered_index)
            .and_then(|index| settings.applications.get(index));
        let mut matches = Vec::new();
        for (index, window) in visible_windows.iter().enumerate() {
            if unmatched[index] && window_key(window, settings.assignments) == key {
                unmatched[index] = false;
                matches.push((*window).clone());
            }
        }
        let executable_path = matches.first().map_or_else(
            || {
                registered
                    .and_then(|application| application.canonical_executables.first())
                    .cloned()
                    .unwrap_or_else(|| pinned.launch_target.clone())
            },
            |window| path_text(&window.executable_path),
        );
        let icon_source = registered.map_or_else(
            || {
                pinned
                    .icon_source
                    .clone()
                    .unwrap_or_else(|| executable_path.clone())
            },
            |application| application.icon_source.clone(),
        );

        items.push(DockItem {
            application_key: key,
            id: pinned.id.clone(),
            display_name: registered.map_or_else(
                || pinned.name.clone(),
                |application| application.name.clone(),
            ),
            launch_target: pinned.launch_target.clone(),
            arguments: pinned.arguments.clone(),
            icon_source: icon_source.clone(),
            presentation_icon: ApplicationPresentationIcon::Source(icon_source),
            app_user_model_id: pinned
                .app_user_model_id
                .as_deref()
                .filter(|identity| is_reliable_application_identity(identity))
                .map(str::to_owned)
                .or_else(|| {
                    registered.and_then(|application| application.app_user_model_id.clone())
                })
                .or_else(|| {
                    matches.iter().find_map(|window| {
                        window
                            .application_facts
                            .reliable_id()
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
        append_unpinned(
            &mut items,
            &visible_windows,
            &unmatched,
            settings.assignments,
            settings.applications,
        );
    }

    apply_saved_order(&mut items, settings.item_order);
    items
}

fn append_unpinned(
    items: &mut Vec<DockItem>,
    visible_windows: &[&WindowInfo],
    unmatched: &[bool],
    assignments: &WindowApplicationAssignments,
    applications: &[RegisteredApplication],
) {
    let mut group_indices = HashMap::<ApplicationKey, usize>::new();
    let mut groups = Vec::<(ApplicationKey, Option<usize>, Vec<WindowInfo>)>::new();

    for (window, is_unmatched) in visible_windows.iter().zip(unmatched) {
        if !is_unmatched {
            continue;
        }

        let (key, registered_index) = window_assignment(window, assignments);
        let index = *group_indices.entry(key.clone()).or_insert_with(|| {
            groups.push((key.clone(), registered_index, Vec::new()));
            groups.len() - 1
        });
        groups[index].2.push((*window).clone());
    }

    groups.sort_by(|left, right| {
        group_name(left, assignments).cmp(&group_name(right, assignments))
    });

    items.extend(groups.into_iter().map(|(key, registered_index, windows)| {
        let registered = registered_index.and_then(|index| applications.get(index));
        let unregistered_launch = windows.iter().find_map(|window| {
            let ApplicationResolution::Unregistered { launch, .. } =
                assignments.by_window.get(&window.key())?
            else {
                return None;
            };
            launch.clone()
        });
        let executable_path = windows
            .first()
            .map_or_else(String::new, |window| path_text(&window.executable_path));
        let Some(presentation) = windows
            .iter()
            .find_map(|window| assignments.presentation_by_window.get(&window.key()))
        else {
            unreachable!("resolved dock groups always have centralized presentation")
        };
        let icon_source = presentation.icon.fallback_path().to_owned();
        DockItem {
            application_key: key.clone(),
            id: registered.map_or_else(
                || application_key_text(&key),
                |application| application.id.clone(),
            ),
            display_name: presentation.display_name.clone(),
            launch_target: registered.map_or_else(
                || {
                    unregistered_launch.as_ref().map_or_else(
                        || executable_path.clone(),
                        |launch| launch.target.clone(),
                    )
                },
                |application| application.launch.target.clone(),
            ),
            arguments: registered.map_or_else(
                || unregistered_launch.and_then(|launch| launch.arguments),
                |application| application.launch.arguments.clone(),
            ),
            icon_source,
            presentation_icon: presentation.icon.clone(),
            app_user_model_id: registered
                .and_then(|application| application.app_user_model_id.clone())
                .or_else(|| {
                    windows.iter().find_map(|window| {
                        window.application_facts.reliable_id().map(str::to_owned)
                    })
                }),
            executable_path,
            is_pinned: false,
            windows,
        }
    }));
}

fn window_key(
    window: &WindowInfo,
    assignments: &WindowApplicationAssignments,
) -> ApplicationKey {
    match assignments.by_window.get(&window.key()) {
        Some(
            ApplicationResolution::Resolved { key, .. }
            | ApplicationResolution::Associated { key }
            | ApplicationResolution::Unregistered { key, .. },
        ) => key.clone(),
        Some(
            ApplicationResolution::Prevented | ApplicationResolution::Ambiguous { .. },
        )
        | None => ApplicationKey::Ephemeral(window.key()),
    }
}

fn window_assignment(
    window: &WindowInfo,
    assignments: &WindowApplicationAssignments,
) -> (ApplicationKey, Option<usize>) {
    match assignments.by_window.get(&window.key()) {
        Some(ApplicationResolution::Resolved {
            key,
            registered_index,
            ..
        }) => (key.clone(), Some(*registered_index)),
        Some(
            ApplicationResolution::Associated { key }
            | ApplicationResolution::Unregistered { key, .. },
        ) => (key.clone(), None),
        Some(
            ApplicationResolution::Prevented | ApplicationResolution::Ambiguous { .. },
        )
        | None => (ApplicationKey::Ephemeral(window.key()), None),
    }
}

fn application_key_text(key: &ApplicationKey) -> String {
    match key {
        ApplicationKey::Registered(value)
        | ApplicationKey::LaunchSignature(value)
        | ApplicationKey::ExecutablePath(value) => value.clone(),
        ApplicationKey::Ephemeral(key) => {
            format!("window:{}:{}", key.id.get(), key.incarnation)
        }
    }
}

fn group_name(
    group: &(ApplicationKey, Option<usize>, Vec<WindowInfo>),
    assignments: &WindowApplicationAssignments,
) -> String {
    group
        .2
        .iter()
        .find_map(|window| assignments.presentation_by_window.get(&window.key()))
        .map_or_else(String::new, |presentation| {
            presentation.display_name.clone()
        })
        .to_lowercase()
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

fn case_key(value: &str) -> String {
    value.to_lowercase()
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
