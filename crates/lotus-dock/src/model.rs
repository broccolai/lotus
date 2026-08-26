use std::collections::HashMap;
use std::path::Path;

use lotus_core::application::{
    ApplicationIdentity, ApplicationKey, PinnedApplicationAssignment,
    RegisteredApplication, WindowApplicationAssignments, is_shared_host_executable,
};
use lotus_core::dock::DockItem;
use lotus_core::settings::{
    DockSettings, PinnedApp, SettingsReset, SettingsStore, SettingsStoreError,
};
use lotus_core::window::WindowInfo;

pub fn project_snapshot(
    settings: &DockSettings,
    windows: &[WindowInfo],
    assignments: &WindowApplicationAssignments,
    applications: &[RegisteredApplication],
    pinned_applications: &[PinnedApplicationAssignment],
) -> Vec<DockItem> {
    lotus_core::dock::project_dock(
        windows,
        lotus_core::dock::DockProjection {
            pinned_apps: &settings.pinned_apps,
            hidden_executables: &settings.hidden_executables,
            item_order: &settings.item_order,
            show_unpinned_running_apps: settings.show_unpinned_running_apps,
            assignments,
            applications,
            pinned_applications,
        },
    )
}

pub fn source_index_for_identity(items: &[DockItem], identity: &str) -> Option<usize> {
    items
        .iter()
        .position(|item| item.id.eq_ignore_ascii_case(identity))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SettingsImpact {
    pub changed: bool,
    pub restart_required: bool,
}

pub struct DockModel {
    settings: DockSettings,
    settings_store: SettingsStore,
    items: Vec<DockItem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PinLaunch {
    pub id: String,
    pub name: String,
    pub target: String,
    pub arguments: Option<String>,
    pub icon_source: Option<String>,
    pub app_user_model_id: Option<String>,
    pub match_executables: Vec<String>,
}

impl DockModel {
    pub const fn new(
        settings: DockSettings,
        settings_store: SettingsStore,
        items: Vec<DockItem>,
    ) -> Self {
        Self {
            settings,
            settings_store,
            items,
        }
    }

    pub const fn settings(&self) -> &DockSettings {
        &self.settings
    }

    pub fn settings_directory(&self) -> &Path {
        self.settings_store.directory()
    }

    pub fn export_settings(&self, destination: &Path) -> Result<(), SettingsStoreError> {
        self.settings_store.export(&self.settings, destination)
    }

    pub fn validate_export_destination(
        &self,
        destination: &Path,
    ) -> Result<(), SettingsStoreError> {
        self.settings_store.validate_export_destination(destination)
    }

    pub fn reset_settings(&self) -> Result<SettingsReset, SettingsStoreError> {
        self.settings_store.reset()
    }

    pub fn items(&self) -> &[DockItem] {
        &self.items
    }

    pub fn rebuild(&mut self, items: Vec<DockItem>) {
        self.items = items;
    }

    pub fn repair_catalogue_pins(
        &mut self,
        assignments: &[PinnedApplicationAssignment],
        applications: &[RegisteredApplication],
        safe_aliases: &[Vec<String>],
    ) -> Result<bool, SettingsStoreError> {
        let mut next = self.settings.clone();
        let mut retained = HashMap::<ApplicationKey, usize>::new();
        let mut removed = Vec::new();
        let mut aliases = HashMap::<usize, Vec<String>>::new();

        for (index, assignment) in assignments.iter().enumerate() {
            let strong = assignment.registered_index.is_some()
                || matches!(
                    assignment.key,
                    ApplicationKey::Registered(_) | ApplicationKey::LaunchSignature(_)
                );
            if !strong {
                continue;
            }
            if let Some(&first) = retained.get(&assignment.key) {
                removed.push((index, first));
                aliases
                    .entry(first)
                    .or_default()
                    .extend(safe_aliases.get(index).into_iter().flatten().cloned());
            } else {
                retained.insert(assignment.key.clone(), index);
            }
        }

        let mut renamed = HashMap::new();
        for (index, assignment) in assignments.iter().enumerate() {
            let Some(application) = assignment
                .registered_index
                .and_then(|index| applications.get(index))
            else {
                continue;
            };
            let Some(pin) = next.pinned_apps.get_mut(index) else {
                continue;
            };
            renamed
                .entry(pin.id.to_ascii_lowercase())
                .or_insert_with(|| application.id.clone());
            pin.id.clone_from(&application.id);
            pin.name.clone_from(&application.name);
            pin.launch_target.clone_from(&application.launch.target);
            pin.arguments.clone_from(&application.launch.arguments);
            pin.icon_source = Some(application.icon_source.clone());
            pin.app_user_model_id
                .clone_from(&application.app_user_model_id);
            let mut merged_aliases = safe_aliases.get(index).cloned().unwrap_or_default();
            merged_aliases.extend(aliases.remove(&index).unwrap_or_default());
            merged_aliases.sort();
            merged_aliases.dedup();
            pin.match_executables = merged_aliases;
        }
        for &(index, first) in &removed {
            let Some(duplicate) = self.settings.pinned_apps.get(index) else {
                continue;
            };
            let Some(retained) = next.pinned_apps.get(first) else {
                continue;
            };
            renamed
                .entry(duplicate.id.to_ascii_lowercase())
                .or_insert_with(|| retained.id.clone());
        }
        for &(index, _) in removed.iter().rev() {
            next.pinned_apps.remove(index);
        }
        next.item_order = next
            .item_order
            .into_iter()
            .map(|id| renamed.get(&id.to_ascii_lowercase()).cloned().unwrap_or(id))
            .fold(Vec::new(), |mut order, id| {
                if !order
                    .iter()
                    .any(|saved: &String| saved.eq_ignore_ascii_case(&id))
                {
                    order.push(id);
                }
                order
            });
        let next = next.normalized();
        if next == self.settings {
            return Ok(false);
        }
        self.settings_store.save(&next)?;
        self.settings = next;
        Ok(true)
    }

    pub fn apply_settings(
        &mut self,
        next: DockSettings,
        items: Vec<DockItem>,
    ) -> Result<SettingsImpact, SettingsStoreError> {
        let next = next.normalized();
        if self.settings == next {
            return Ok(SettingsImpact {
                changed: false,
                restart_required: false,
            });
        }

        let previous = self.settings.clone();
        self.settings_store.save(&next)?;
        self.settings = next;
        self.items = items;

        Ok(SettingsImpact {
            changed: true,
            restart_required: restart_required(&previous, &self.settings),
        })
    }

    pub fn persist_reorder(
        &mut self,
        source_index: usize,
        insertion_slot: usize,
    ) -> Result<bool, SettingsStoreError> {
        let Some(destination) =
            insertion_destination(self.items.len(), source_index, insertion_slot)
        else {
            return Ok(false);
        };
        if source_index == destination {
            return Ok(false);
        }

        let mut reordered = self.items.clone();
        let moved = reordered.remove(source_index);
        reordered.insert(destination, moved);
        let mut settings = self.settings.clone();
        let (target_index, insert_after) = if insertion_slot == self.items.len() {
            (self.items.len() - 1, true)
        } else {
            (insertion_slot, false)
        };
        let source_id = &self.items[source_index].id;
        let target_id = &self.items[target_index].id;
        let mut full_order =
            Vec::with_capacity(settings.item_order.len() + self.items.len());
        for id in settings
            .item_order
            .iter()
            .chain(self.items.iter().map(|item| &item.id))
        {
            if !full_order
                .iter()
                .any(|saved: &String| saved.eq_ignore_ascii_case(id))
            {
                full_order.push(id.clone());
            }
        }
        full_order.retain(|id| !id.eq_ignore_ascii_case(source_id));
        let Some(target_position) = full_order
            .iter()
            .position(|id| id.eq_ignore_ascii_case(target_id))
        else {
            return Ok(false);
        };
        full_order.insert(
            target_position + usize::from(insert_after),
            source_id.clone(),
        );
        settings.item_order = full_order;
        self.settings_store.save(&settings)?;

        self.settings = settings;
        self.items = reordered;
        Ok(true)
    }

    pub fn set_pinned(
        &mut self,
        source_index: usize,
        pinned: bool,
        launch: Option<PinLaunch>,
    ) -> Result<bool, SettingsStoreError> {
        let Some(item) = self.items.get(source_index) else {
            return Ok(false);
        };
        if item.is_pinned == pinned {
            return Ok(false);
        }

        let mut settings = self.settings.clone();
        if pinned {
            let launch = launch.unwrap_or_else(|| PinLaunch {
                id: item.id.clone(),
                name: item.display_name.clone(),
                target: item.launch_target.clone(),
                arguments: item.arguments.clone(),
                icon_source: Some(item.icon_source.clone()),
                app_user_model_id: item.windows.first().and_then(|window| {
                    window.application_facts.reliable_id().map(str::to_owned)
                }),
                match_executables: executable_alias(&item.executable_path)
                    .into_iter()
                    .collect(),
            });
            if settings.pinned_apps.iter().any(|pin| {
                pin.application_identity(None)
                    .match_strength(&launch.identity())
                    .is_match()
            }) {
                return Ok(false);
            }
            settings.pinned_apps.push(PinnedApp {
                id: launch.id,
                name: launch.name,
                launch_target: launch.target,
                arguments: launch.arguments,
                icon_source: launch.icon_source,
                app_user_model_id: launch.app_user_model_id,
                match_executables: launch.match_executables,
                ..Default::default()
            });
            insert_item_order(&mut settings.item_order, &self.items, source_index);
        } else {
            settings
                .pinned_apps
                .retain(|pin| !pin.id.eq_ignore_ascii_case(&item.id));
        }

        let settings = settings.normalized();
        self.settings_store.save(&settings)?;
        self.settings = settings;
        Ok(true)
    }
}

impl PinLaunch {
    fn identity(&self) -> ApplicationIdentity {
        ApplicationIdentity::new(
            self.app_user_model_id.as_deref(),
            Some(&self.id),
            Some(&self.target),
            std::iter::empty(),
        )
    }
}

fn executable_alias(path: &str) -> Option<String> {
    let executable = path.rsplit(['\\', '/']).next()?;
    (!is_shared_host_executable(executable) && !executable.is_empty())
        .then(|| executable.into())
}

fn insert_item_order(order: &mut Vec<String>, items: &[DockItem], source_index: usize) {
    let id = &items[source_index].id;
    if order.iter().any(|saved| saved.eq_ignore_ascii_case(id)) {
        return;
    }
    let next = items
        .iter()
        .skip(source_index + 1)
        .find_map(|item| {
            order
                .iter()
                .position(|saved| saved.eq_ignore_ascii_case(&item.id))
        })
        .unwrap_or(order.len());
    order.insert(next, id.clone());
}

fn restart_required(previous: &DockSettings, current: &DockSettings) -> bool {
    previous.replace_windows_taskbar != current.replace_windows_taskbar
        || previous.exclusive_taskbar_replacement != current.exclusive_taskbar_replacement
        || previous.notification_badge_style != current.notification_badge_style
        || (current.replace_windows_taskbar
            && (previous.icon_size != current.icon_size
                || previous.vertical_padding != current.vertical_padding
                || previous.bottom_offset != current.bottom_offset))
}

fn insertion_destination(
    item_count: usize,
    source_index: usize,
    insertion_slot: usize,
) -> Option<usize> {
    if insertion_slot > item_count || item_count == 0 {
        return None;
    }
    let (target_index, insert_after) = if insertion_slot == item_count {
        (item_count - 1, true)
    } else {
        (insertion_slot, false)
    };
    lotus_core::reorder::destination_index(
        item_count,
        source_index,
        target_index,
        insert_after,
    )
}
