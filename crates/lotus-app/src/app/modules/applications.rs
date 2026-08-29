use lotus_windows::icon_hydrator::IconHydrationResult;

use super::ModuleHost;
use crate::app::AppError;
use crate::app::dock::DockRuntime;

impl ModuleHost {
    pub(in crate::app) fn drain_hydrated_icons(
        &mut self,
        dock_model: &mut DockRuntime,
    ) -> Result<(), AppError> {
        let mut launcher = Vec::new();
        let mut switcher = Vec::new();
        let mut dock = Vec::new();

        for result in self.icon_hydrator.drain() {
            match result {
                IconHydrationResult::Launcher(result) => launcher.push(result),
                IconHydrationResult::Switcher(result) => switcher.push(result),
                IconHydrationResult::Dock(result) => dock.push(result),
            }
        }

        let _changed = self.launcher.drain_hydrated_icons(launcher)?;
        self.switcher.drain_hydrated_icons(switcher);
        dock_model.drain_hydrated_window_icons(dock);
        Ok(())
    }
}
