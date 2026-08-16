use std::path::Path;

use lotus_core::activation::{ActivationDecision, decide_activation};
use lotus_core::dock::DockItem;
use lotus_core::settings::{DockSettings, PinnedApp, SettingsStore, SettingsStoreError};
use lotus_core::window::{WindowId, WindowInfo};

pub fn project_snapshot<F>(
    settings: &DockSettings,
    windows: &[WindowInfo],
    resolve_executable: F,
) -> Vec<DockItem>
where
    F: FnMut(&str) -> Option<String>,
{
    lotus_core::dock::project_dock(
        windows,
        lotus_core::dock::DockProjection {
            pinned_apps: &settings.pinned_apps,
            hidden_executables: &settings.hidden_executables,
            item_order: &settings.item_order,
            show_unpinned_running_apps: settings.show_unpinned_running_apps,
        },
        resolve_executable,
    )
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
}

pub struct PinUpgrade {
    pub current_id: String,
    pub launch: PinLaunch,
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

    pub fn items(&self) -> &[DockItem] {
        &self.items
    }

    pub fn rebuild(&mut self, items: Vec<DockItem>) {
        self.items = items;
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

    pub fn activation(
        &self,
        source_index: usize,
        foreground: Option<&WindowId>,
    ) -> Option<(ActivationDecision<WindowId>, &DockItem)> {
        let item = self.items.get(source_index)?;
        let windows = item
            .windows
            .iter()
            .map(|window| window.id)
            .collect::<Vec<_>>();
        Some((decide_activation(&windows, foreground), item))
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
            if settings
                .pinned_apps
                .iter()
                .any(|pin| pin.id.eq_ignore_ascii_case(&item.id))
            {
                return Ok(false);
            }
            let executable = Path::new(&item.executable_path)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
                .into_iter()
                .collect();
            let launch = launch.unwrap_or_else(|| PinLaunch {
                id: item.id.clone(),
                name: item.display_name.clone(),
                target: item.launch_target.clone(),
                arguments: item.arguments.clone(),
                icon_source: Some(item.icon_source.clone()),
                app_user_model_id: item
                    .windows
                    .first()
                    .and_then(|window| window.app_user_model_id.clone()),
            });
            if settings.pinned_apps.iter().any(|pin| {
                pin.id.eq_ignore_ascii_case(&launch.id)
                    || pin.app_user_model_id.as_deref().is_some_and(|identity| {
                        launch
                            .app_user_model_id
                            .as_deref()
                            .is_some_and(|candidate| {
                                candidate.eq_ignore_ascii_case(identity)
                            })
                    })
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
                match_executables: executable,
            });
            insert_item_order(&mut settings.item_order, &self.items, source_index);
        } else {
            settings
                .pinned_apps
                .retain(|pin| !pin.id.eq_ignore_ascii_case(&item.id));
        }

        self.settings_store.save(&settings)?;
        self.settings = settings;
        Ok(true)
    }

    pub fn upgrade_pins(
        &mut self,
        upgrades: Vec<PinUpgrade>,
    ) -> Result<bool, SettingsStoreError> {
        if upgrades.is_empty() {
            return Ok(false);
        }

        let mut settings = self.settings.clone();
        let mut changed = false;
        for upgrade in upgrades {
            let Some(pin) = settings
                .pinned_apps
                .iter_mut()
                .find(|pin| pin.id.eq_ignore_ascii_case(&upgrade.current_id))
            else {
                continue;
            };
            if pin.id.eq_ignore_ascii_case(&upgrade.launch.id)
                && pin.name == upgrade.launch.name
                && pin.launch_target == upgrade.launch.target
                && pin.arguments == upgrade.launch.arguments
                && pin.icon_source == upgrade.launch.icon_source
            {
                continue;
            }

            replace_order_identity(&mut settings.item_order, &pin.id, &upgrade.launch.id);
            pin.id = upgrade.launch.id;
            pin.name = upgrade.launch.name;
            pin.launch_target = upgrade.launch.target;
            pin.arguments = upgrade.launch.arguments;
            pin.icon_source = upgrade.launch.icon_source;
            pin.app_user_model_id = upgrade.launch.app_user_model_id;
            changed = true;
        }
        if !changed {
            return Ok(false);
        }

        self.settings_store.save(&settings)?;
        self.settings = settings;
        Ok(true)
    }
}

fn replace_order_identity(order: &mut [String], previous: &str, current: &str) {
    for identity in order {
        if identity.eq_ignore_ascii_case(previous) {
            current.clone_into(identity);
        }
    }
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
        || previous.search_enabled != current.search_enabled
        || previous.search_open_with_windows_key != current.search_open_with_windows_key
        || previous.alt_tab_enabled != current.alt_tab_enabled
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
