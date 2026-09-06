use lotus_core::application::{
    ApplicationKey, ApplicationResolution, is_shared_host_executable,
};
use lotus_core::dock::DockItem;
use lotus_core::window::WindowInfo;
use lotus_dock::model::PinLaunch;

use super::DockRuntime;

impl DockRuntime {
    pub(in crate::app) fn prepare_pinned(
        &self,
        source_index: usize,
        pinned: bool,
        registered: Option<lotus_core::application::RegisteredApplication>,
    ) -> Option<lotus_core::settings::DockSettings> {
        if pinned
            && !self
                .model
                .items()
                .get(source_index)
                .is_some_and(|item| item.pin_eligible)
        {
            return None;
        }
        let launch = registered
            .map(|application| {
                let match_executables = self
                    .applications
                    .catalog()
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
            })
            .or_else(|| {
                pinned
                    .then(|| self.unregistered_pin_launch(source_index))
                    .flatten()
            });
        if pinned && launch.is_none() {
            return None;
        }
        let settings = self.model.prepare_pinned(source_index, pinned, launch)?;
        Some(settings)
    }

    fn unregistered_pin_launch(&self, source_index: usize) -> Option<PinLaunch> {
        let item = self.model.items().get(source_index)?;
        let source = item.pin_source?;
        let assignments = self.applications.assignments();
        if !assignments.can_pin(source) {
            return None;
        }
        let window = item.windows.iter().find(|window| window.key() == source)?;
        let ApplicationResolution::Unregistered { launch, .. } =
            assignments.by_window.get(&source)?
        else {
            return None;
        };
        let executable_path = window.executable_path.to_string_lossy().into_owned();
        let (target, arguments) = launch.as_ref().map_or_else(
            || (executable_path.clone(), None),
            |launch| (launch.target.clone(), launch.arguments.clone()),
        );
        let presentation = assignments.presentation_by_window.get(&source)?;
        let match_executables = (!is_shared_host_executable(&executable_path))
            .then(|| {
                window
                    .executable_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .flatten()
            .into_iter()
            .collect();

        Some(PinLaunch {
            id: item.id.clone(),
            name: presentation.display_name.clone(),
            target,
            arguments,
            icon_source: Some(presentation.icon.fallback_path().to_owned()),
            app_user_model_id: window.application_facts.reliable_id().map(str::to_owned),
            match_executables,
        })
    }

    pub(in crate::app) fn merge_transient_unpinned(
        &mut self,
        items: &mut Vec<DockItem>,
        windows: &[WindowInfo],
    ) {
        let assignments = self.applications.assignments();
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
        Some(ApplicationResolution::Ambiguous { .. }) | None => {
            ApplicationKey::Ephemeral(window.key())
        }
    }
}
