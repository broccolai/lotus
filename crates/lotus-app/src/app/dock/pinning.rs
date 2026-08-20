use lotus_core::dock::DockItem;
use lotus_core::window::WindowInfo;
use lotus_dock::model::{PinExecutableAlias, PinLaunch, PinUpgrade};

use super::DockRuntime;
use super::projection::{projected_items, window_matches_item};
use crate::app::AppError;

impl DockRuntime {
    pub(in crate::app) fn set_pinned(
        &mut self,
        source_index: usize,
        pinned: bool,
        windows: &[WindowInfo],
        registered: Option<lotus_windows::search_catalog::RegisteredApplication>,
    ) -> Result<bool, AppError> {
        let previous = self
            .model
            .items()
            .get(source_index)
            .cloned()
            .map(|item| (source_index, item));
        let launch = registered.map(|application| PinLaunch {
            id: application.id,
            name: application.name,
            target: application.launch_target,
            arguments: application.arguments,
            icon_source: Some(application.icon_source),
            app_user_model_id: application.app_user_model_id,
        });
        if !self.model.set_pinned(source_index, pinned, launch)? {
            return Ok(false);
        }
        if let Some((index, item)) = previous {
            if pinned {
                self.transient_unpinned.remove(&item.id);
            } else if item.is_running() {
                self.transient_unpinned
                    .insert(item.id.clone(), (index, item));
            }
        }
        let mut items = projected_items(self.model.settings(), windows);
        self.merge_transient_unpinned(&mut items, windows);
        self.model.rebuild(items);
        self.refresh_scene_items();
        Ok(true)
    }

    pub(in crate::app) fn reconcile_unpinned_pins(
        &mut self,
        windows: &[WindowInfo],
        catalog: &lotus_windows::search_catalog::SearchCatalogCache,
    ) -> Result<bool, AppError> {
        let aliases = projected_items(self.model.settings(), windows)
            .iter()
            .filter(|item| !item.is_pinned)
            .flat_map(|item| {
                item.windows.iter().filter_map(|window| {
                    let application =
                        catalog.registered_application(window, &item.display_name)?;
                    let executable_name = window
                        .executable_name()
                        .and_then(|name| name.to_str())?
                        .to_owned();
                    Some(PinExecutableAlias {
                        registered_id: application.id,
                        app_user_model_id: application.app_user_model_id,
                        executable_name,
                    })
                })
            })
            .collect();
        Ok(self.model.reconcile_pin_executables(aliases)?)
    }

    pub(in crate::app) fn upgrade_legacy_pins(
        &mut self,
        catalog: &lotus_windows::search_catalog::SearchCatalogCache,
    ) -> Result<bool, AppError> {
        let upgrades = self
            .model
            .items()
            .iter()
            .filter(|item| item.is_pinned)
            .filter_map(|item| {
                let window = item.windows.first()?;
                let application =
                    catalog.registered_application(window, &item.display_name)?;
                Some(PinUpgrade {
                    current_id: item.id.clone(),
                    launch: PinLaunch {
                        id: application.id,
                        name: application.name,
                        target: application.launch_target,
                        arguments: application.arguments,
                        icon_source: Some(application.icon_source),
                        app_user_model_id: application.app_user_model_id,
                    },
                })
            })
            .collect();
        Ok(self.model.upgrade_pins(upgrades)?)
    }

    pub(in crate::app) fn merge_transient_unpinned(
        &mut self,
        items: &mut Vec<DockItem>,
        windows: &[WindowInfo],
    ) {
        self.transient_unpinned.retain(|_, (_, item)| {
            item.windows = windows
                .iter()
                .filter(|window| window_matches_item(window, item))
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
            if items
                .iter()
                .any(|current| current.id.eq_ignore_ascii_case(&item.id))
            {
                continue;
            }
            items.insert(index.min(items.len()), item);
        }
    }
}
