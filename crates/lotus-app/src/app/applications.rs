use std::sync::Arc;
use std::time::Duration;

use lotus_core::dock::DockItem;
use lotus_core::search::SearchCatalog;
use lotus_windows::icon_hydrator::{
    DockIconClient, HydratedDockIcon, HydratedLauncherIcon, HydratedSwitcherIcon,
    IconHydrationResult, IconHydrator, LauncherIconClient, SwitcherIconClient,
};
use lotus_windows::search_catalog::{ApplicationCatalogSnapshot, SearchCatalogCache};

pub(super) struct ApplicationServices {
    catalog: SearchCatalogCache,
    icon_hydrator: IconHydrator,
}

pub(super) struct PreparedLauncherCatalog {
    pub(super) generation: Option<u64>,
    pub(super) catalog: SearchCatalog,
}

pub(super) struct HydratedIconBatch {
    pub(super) launcher: Vec<HydratedLauncherIcon>,
    pub(super) switcher: Vec<HydratedSwitcherIcon>,
    pub(super) dock: Vec<HydratedDockIcon>,
}

impl ApplicationServices {
    pub(super) fn new() -> Result<Self, lotus_windows::icon_hydrator::IconHydratorError> {
        let icon_hydrator = IconHydrator::start()?;

        Ok(Self {
            catalog: SearchCatalogCache::new(),
            icon_hydrator,
        })
    }

    pub(super) fn dock_icon_client(&self) -> DockIconClient {
        self.icon_hydrator.dock_client()
    }

    pub(super) fn launcher_icon_client(&self) -> LauncherIconClient {
        self.icon_hydrator.launcher_client()
    }

    pub(super) fn switcher_icon_client(&self) -> SwitcherIconClient {
        self.icon_hydrator.switcher_client()
    }

    pub(super) fn refresh_launcher_catalog_if_stale(&self) {
        let _ = self.catalog.refresh_if_stale(Duration::from_mins(5));
    }

    pub(super) fn prepare_launcher_catalog(
        &self,
        dock_items: &[DockItem],
        hidden_executables: &[String],
    ) -> PreparedLauncherCatalog {
        if let Some(ready) = self.catalog.ready_catalog(dock_items, hidden_executables) {
            return PreparedLauncherCatalog {
                generation: Some(ready.generation),
                catalog: ready.catalog,
            };
        }

        PreparedLauncherCatalog {
            generation: None,
            catalog: self.catalog.catalog(dock_items, hidden_executables),
        }
    }

    pub(super) fn snapshot(&self) -> Arc<ApplicationCatalogSnapshot> {
        self.catalog.snapshot()
    }

    pub(super) fn launcher_catalog_refresh_pending(
        &self,
        launcher_generation: Option<u64>,
    ) -> bool {
        self.catalog
            .ready_generation()
            .is_some_and(|generation| launcher_generation != Some(generation))
    }

    pub(super) fn drain_hydrated_icons(&self) -> HydratedIconBatch {
        let mut batch = HydratedIconBatch {
            launcher: Vec::new(),
            switcher: Vec::new(),
            dock: Vec::new(),
        };

        for result in self.icon_hydrator.drain() {
            match result {
                IconHydrationResult::Launcher(result) => batch.launcher.push(result),
                IconHydrationResult::Switcher(result) => batch.switcher.push(result),
                IconHydrationResult::Dock(result) => batch.dock.push(result),
            }
        }

        batch
    }
}
