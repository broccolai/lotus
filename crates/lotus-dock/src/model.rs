use lotus_core::application::{
    PinnedApplicationAssignment, RegisteredApplication, WindowApplicationAssignments,
    is_shared_host_executable,
};
use lotus_core::dock::DockItem;
use lotus_core::settings::{DockSettings, PinnedApp};
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

impl SettingsImpact {
    pub fn between(previous: &DockSettings, current: &DockSettings) -> Self {
        Self {
            changed: previous != current,
            restart_required: restart_required(previous, current),
        }
    }
}

pub struct DockModel {
    settings: DockSettings,
    items: Vec<DockItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DockSettingsChange {
    settings: DockSettings,
    items: Vec<DockItem>,
    impact: SettingsImpact,
}

impl DockSettingsChange {
    pub const fn impact(&self) -> SettingsImpact {
        self.impact
    }

    pub const fn settings(&self) -> &DockSettings {
        &self.settings
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DockReorderRequest {
    pub source_id: String,
    pub target_id: String,
    pub insert_after: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DockReorder {
    settings: DockSettings,
    items: Vec<DockItem>,
}

impl DockReorder {
    pub const fn settings(&self) -> &DockSettings {
        &self.settings
    }
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
    pub const fn new(settings: DockSettings, items: Vec<DockItem>) -> Self {
        Self { settings, items }
    }

    pub const fn settings(&self) -> &DockSettings {
        &self.settings
    }

    pub fn items(&self) -> &[DockItem] {
        &self.items
    }

    pub fn rebuild(&mut self, items: Vec<DockItem>) {
        self.items = items;
    }

    pub fn prepare_settings(
        &self,
        next: DockSettings,
        items: Vec<DockItem>,
    ) -> Option<DockSettingsChange> {
        let next = next.normalized();
        if self.settings == next {
            return None;
        }

        let previous = self.settings.clone();
        Some(DockSettingsChange {
            impact: SettingsImpact::between(&previous, &next),
            settings: next,
            items,
        })
    }

    pub fn commit_settings(&mut self, change: DockSettingsChange) -> SettingsImpact {
        self.settings = change.settings;
        self.items = change.items;
        change.impact
    }

    pub fn prepare_reorder(&self, request: &DockReorderRequest) -> Option<DockReorder> {
        let source_index = source_index_for_identity(&self.items, &request.source_id)?;
        let target_index = source_index_for_identity(&self.items, &request.target_id)?;
        let destination = lotus_core::reorder::destination_index(
            self.items.len(),
            source_index,
            target_index,
            request.insert_after,
        )?;
        if source_index == destination {
            return None;
        }

        let mut reordered = self.items.clone();
        let moved = reordered.remove(source_index);
        reordered.insert(destination, moved);
        let mut settings = self.settings.clone();
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
        let target_position = full_order
            .iter()
            .position(|id| id.eq_ignore_ascii_case(target_id))?;
        full_order.insert(
            target_position + usize::from(request.insert_after),
            source_id.clone(),
        );
        settings.item_order = full_order;
        Some(DockReorder {
            settings,
            items: reordered,
        })
    }

    pub fn prepare_pinned(
        &self,
        source_index: usize,
        pinned: bool,
        launch: Option<PinLaunch>,
    ) -> Option<DockSettings> {
        let item = self.items.get(source_index)?;
        if item.is_pinned == pinned {
            return None;
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
                pin.launch_target.eq_ignore_ascii_case(&launch.target)
                    && pin.arguments == launch.arguments
            }) {
                return None;
            }
            let pin_id = next_pin_id(&settings.pinned_apps, &launch.id);
            settings.pinned_apps.push(PinnedApp {
                id: pin_id.clone(),
                name: launch.name,
                launch_target: launch.target,
                arguments: launch.arguments,
                icon_source: launch.icon_source,
                app_user_model_id: launch.app_user_model_id,
                match_executables: launch.match_executables,
                ..Default::default()
            });
            insert_item_order(&mut settings.item_order, &self.items, source_index, &pin_id);
        } else {
            settings
                .pinned_apps
                .retain(|pin| !pin.id.eq_ignore_ascii_case(&item.id));
        }

        let settings = settings.normalized();
        Some(settings)
    }

    pub fn commit_settings_only(&mut self, settings: DockSettings) {
        self.settings = settings;
    }

    pub fn commit_reorder(&mut self, reorder: DockReorder) {
        self.settings = reorder.settings;
        self.items = reorder.items;
    }
}

fn executable_alias(path: &str) -> Option<String> {
    let executable = path.rsplit(['\\', '/']).next()?;
    (!is_shared_host_executable(executable) && !executable.is_empty())
        .then(|| executable.into())
}

fn insert_item_order(
    order: &mut Vec<String>,
    items: &[DockItem],
    source_index: usize,
    id: &str,
) {
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
    order.insert(next, id.to_owned());
}

fn next_pin_id(pins: &[PinnedApp], preferred: &str) -> String {
    if !pins
        .iter()
        .any(|pin| pin.id.eq_ignore_ascii_case(preferred))
    {
        return preferred.to_owned();
    }

    let mut suffix = 2_u32;
    loop {
        let candidate = format!("{preferred}#{suffix}");
        if !pins
            .iter()
            .any(|pin| pin.id.eq_ignore_ascii_case(&candidate))
        {
            return candidate;
        }
        suffix = suffix.saturating_add(1);
    }
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
