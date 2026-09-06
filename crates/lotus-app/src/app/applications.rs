use std::sync::Arc;
use std::time::Duration;

use lotus_core::application::WindowApplicationAssignments;
use lotus_core::dock::DockItem;
use lotus_core::search::SearchCatalog;
use lotus_core::settings::DockSettings;
use lotus_core::window::WindowInfo;
use lotus_windows::icon_hydrator::{
    DockIconClient, HydratedDockIcon, HydratedLauncherIcon, HydratedSettingsIcon,
    HydratedSwitcherIcon, IconHydrationResult, IconHydrator, LauncherIconClient,
    SettingsIconClient, SwitcherIconClient,
};
use lotus_windows::search_catalog::{
    ApplicationAssociations, ApplicationCatalogSnapshot, ApplicationResolver,
    SearchCatalogCache,
};

pub(super) struct ApplicationView {
    catalog: Arc<ApplicationCatalogSnapshot>,
    assignments: WindowApplicationAssignments,
    window_revision: u64,
    binding_revision: u64,
}

impl ApplicationView {
    fn new(
        catalog: Arc<ApplicationCatalogSnapshot>,
        assignments: WindowApplicationAssignments,
        window_revision: u64,
        binding_revision: u64,
    ) -> Self {
        Self {
            catalog,
            assignments,
            window_revision,
            binding_revision,
        }
    }

    pub(super) fn catalog(&self) -> &ApplicationCatalogSnapshot {
        &self.catalog
    }

    pub(super) fn assignments(&self) -> &WindowApplicationAssignments {
        &self.assignments
    }

    pub(super) const fn window_revision(&self) -> u64 {
        self.window_revision
    }

    pub(super) const fn binding_revision(&self) -> u64 {
        self.binding_revision
    }
}

pub(super) struct ApplicationServices {
    catalog: SearchCatalogCache,
    icon_hydrator: IconHydrator,
    resolver: ApplicationResolver,
    associations: ApplicationAssociations,
    binding_revision: u64,
    view: Arc<ApplicationView>,
}

pub(super) struct PreparedLauncherCatalog {
    pub(super) generation: Option<u64>,
    pub(super) catalog: SearchCatalog,
}

pub(super) struct HydratedIconBatch {
    pub(super) launcher: Vec<HydratedLauncherIcon>,
    pub(super) switcher: Vec<HydratedSwitcherIcon>,
    pub(super) dock: Vec<HydratedDockIcon>,
    pub(super) settings: Vec<HydratedSettingsIcon>,
}

impl ApplicationServices {
    pub(super) fn new(
        windows: &[WindowInfo],
        settings: &DockSettings,
        window_revision: u64,
    ) -> Result<Self, lotus_windows::icon_hydrator::IconHydratorError> {
        let icon_hydrator = IconHydrator::start()?;
        let catalog = SearchCatalogCache::new();
        let snapshot = catalog.snapshot();
        let associations =
            ApplicationAssociations::from_pins(&settings.pinned_apps, &snapshot);
        let mut resolver = ApplicationResolver::default();
        let assignments =
            resolver.resolve_all(windows, &snapshot, &associations, window_revision);
        let view = Arc::new(ApplicationView::new(
            snapshot,
            assignments,
            window_revision,
            0,
        ));

        Ok(Self {
            catalog,
            icon_hydrator,
            resolver,
            associations,
            binding_revision: 0,
            view,
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

    pub(super) fn settings_icon_client(&self) -> SettingsIconClient {
        self.icon_hydrator.settings_client()
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
        Arc::clone(&self.view.catalog)
    }

    pub(super) fn view(&self) -> Arc<ApplicationView> {
        Arc::clone(&self.view)
    }

    pub(super) fn reconcile_view(
        &mut self,
        windows: &[WindowInfo],
        settings: &DockSettings,
        window_revision: u64,
    ) -> Arc<ApplicationView> {
        let catalog = self.catalog.snapshot();
        let associations =
            ApplicationAssociations::from_pins(&settings.pinned_apps, &catalog);
        let bindings_changed = associations != self.associations;
        if bindings_changed {
            self.associations = associations;
            self.binding_revision = self.binding_revision.wrapping_add(1);
        }
        if Arc::ptr_eq(&catalog, &self.view.catalog)
            && self.view.window_revision() == window_revision
            && self.view.binding_revision() == self.binding_revision
        {
            return self.view();
        }
        let assignments = self.resolver.resolve_all(
            windows,
            &catalog,
            &self.associations,
            window_revision,
        );
        self.view = Arc::new(ApplicationView::new(
            catalog,
            assignments,
            window_revision,
            self.binding_revision,
        ));
        self.view()
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
            settings: Vec::new(),
        };

        for result in self.icon_hydrator.drain() {
            match result {
                IconHydrationResult::Launcher(result) => batch.launcher.push(result),
                IconHydrationResult::Switcher(result) => batch.switcher.push(result),
                IconHydrationResult::Dock(result) => batch.dock.push(result),
                IconHydrationResult::Settings(result) => batch.settings.push(result),
            }
        }

        batch
    }
}
