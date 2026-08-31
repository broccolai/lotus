use super::ModuleHost;
use crate::app::AppError;
use crate::app::applications::HydratedIconBatch;
use crate::app::dock::DockRuntime;

pub(in crate::app) struct HydratedIconDrainOutcome {
    requests_frame: bool,
    dock_presentation_changed: bool,
}

impl HydratedIconDrainOutcome {
    pub(in crate::app) const fn requests_frame(&self) -> bool {
        self.requests_frame
    }

    pub(in crate::app) const fn dock_presentation_changed(&self) -> bool {
        self.dock_presentation_changed
    }
}

impl ModuleHost {
    pub(in crate::app) fn application_snapshot(
        &self,
    ) -> std::sync::Arc<lotus_windows::search_catalog::ApplicationCatalogSnapshot> {
        self.applications.snapshot()
    }

    pub(in crate::app) fn refresh_catalog(
        &mut self,
        dock: &lotus_windows::window::DockWindow,
        dock_model: &mut DockRuntime,
        graphics: &mut lotus_windows::graphics::DeviceState,
    ) -> Result<bool, AppError> {
        let catalog = self.applications.prepare_launcher_catalog(
            dock_model.items(),
            &dock_model.settings().hidden_executables,
        );

        self.launcher
            .refresh_catalog_if_ready(dock, catalog, graphics)
    }

    pub(in crate::app) fn launcher_catalog_refresh_pending(&self) -> bool {
        self.applications
            .launcher_catalog_refresh_pending(self.launcher.catalog_generation())
    }

    pub(in crate::app) fn drain_hydrated_icons(
        &mut self,
        dock_model: &mut DockRuntime,
    ) -> Result<HydratedIconDrainOutcome, AppError> {
        let HydratedIconBatch {
            launcher,
            switcher,
            dock,
        } = self.applications.drain_hydrated_icons();

        let launcher_changed = self.launcher.drain_hydrated_icons(launcher)?;
        let switcher_changed = self.switcher.drain_hydrated_icons(switcher);
        let dock_changed = dock_model.drain_hydrated_window_icons(dock);

        Ok(HydratedIconDrainOutcome {
            requests_frame: launcher_changed || switcher_changed || dock_changed,
            dock_presentation_changed: dock_changed,
        })
    }
}
