use lotus_core::application::{ApplicationKey, ApplicationResolution};
use lotus_core::dock::DockItem;
use lotus_core::window::WindowInfo;
use lotus_dock::model::PinLaunch;

use super::DockRuntime;
use crate::app::AppError;
use crate::app::settings_persistence::SettingsPersistence;

impl DockRuntime {
    pub(in crate::app) fn set_pinned(
        &mut self,
        source_index: usize,
        pinned: bool,
        windows: &[WindowInfo],
        registered: Option<lotus_core::application::RegisteredApplication>,
        persistence: &SettingsPersistence,
    ) -> Result<bool, AppError> {
        let previous = self
            .model
            .items()
            .get(source_index)
            .cloned()
            .map(|item| (source_index, item));
        if pinned
            && previous.as_ref().is_some_and(|(_, item)| {
                super::pinned_application_assignments(
                    self.model.settings(),
                    &self.application_catalog,
                )
                .iter()
                .any(|assignment| assignment.key == item.application_key)
            })
        {
            return Ok(false);
        }
        let launch = registered.map(|application| {
            let match_executables = self
                .application_catalog
                .safe_executable_aliases(&application);
            PinLaunch {
                id: application.id,
                name: application.name,
                target: application.launch.target,
                arguments: application.launch.arguments,
                icon_source: Some(application.icon_source),
                app_user_model_id: application.app_user_model_id,
                match_executables,
            }
        });
        let Some(settings) = self.model.prepare_pinned(source_index, pinned, launch) else {
            return Ok(false);
        };
        persistence.save(&settings)?;
        self.model.commit_settings_only(settings);
        self.resolve_current_applications(windows);
        if let Some((index, item)) = previous {
            if pinned {
                self.transient_unpinned.remove(&item.application_key);
            } else if item.is_running() {
                self.transient_unpinned
                    .insert(item.application_key.clone(), (index, item));
            }
        }
        let mut items = self.projected_items(windows);
        self.merge_transient_unpinned(&mut items, windows);
        self.model.rebuild(items);
        self.refresh_scene_items();
        Ok(true)
    }

    pub(in crate::app) fn merge_transient_unpinned(
        &mut self,
        items: &mut Vec<DockItem>,
        windows: &[WindowInfo],
    ) {
        let assignments = &self.application_assignments;
        self.transient_unpinned.retain(|key, (_, item)| {
            item.windows = windows
                .iter()
                .filter(|window| window_application_key(window, assignments) == *key)
                .cloned()
                .collect();
            !item.windows.is_empty()
        });
        let mut retained = self
            .transient_unpinned
            .values()
            .cloned()
            .collect::<Vec<_>>();
        retained.sort_by_key(|(index, _)| *index);
        for (index, item) in retained {
            if let Some(current_index) = items
                .iter()
                .position(|current| current.application_key == item.application_key)
            {
                let current = items.remove(current_index);
                items.insert(index.min(items.len()), current);
                self.transient_unpinned.remove(&item.application_key);
                continue;
            }
            items.insert(index.min(items.len()), item);
        }
    }
}

fn window_application_key(
    window: &WindowInfo,
    assignments: &lotus_core::application::WindowApplicationAssignments,
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
