use std::path::Path;

use lotus_core::activation::{ActivationDecision, decide_activation};
use lotus_core::dock::DockItem;
use lotus_core::settings::{DockSettings, SettingsStore, SettingsStoreError};
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
            pinned_apps: &[],
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
}

fn restart_required(previous: &DockSettings, current: &DockSettings) -> bool {
    previous.replace_windows_taskbar != current.replace_windows_taskbar
        || previous.exclusive_taskbar_replacement != current.exclusive_taskbar_replacement
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
