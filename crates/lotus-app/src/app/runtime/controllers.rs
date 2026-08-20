use lotus_windows::activation::launch_target;
use lotus_windows::alt_tab::{AltTabController, AltTabEvent};
use lotus_windows::graphics::DeviceState;
use lotus_windows::window_tracker::WindowTracker;

use crate::app::switcher::SwitcherRuntime;
use crate::app::{AppError, DockRuntime, RestartError};

pub(crate) fn restart_current_process() -> Result<(), RestartError> {
    let executable = std::env::current_exe()?;
    let arguments = format!("--restart-after {} --open-settings", std::process::id());
    launch_target(&executable.to_string_lossy(), Some(&arguments))?;
    Ok(())
}

pub(crate) fn enable_optional_windows_key<T, E>(
    enabled: bool,
    enable: impl FnOnce() -> Result<T, E>,
) -> Option<T> {
    enabled.then(enable).and_then(Result::ok)
}

pub(crate) fn enable_optional_alt_tab(enabled: bool) -> Option<AltTabController> {
    if !enabled {
        return None;
    }
    let mut controller = AltTabController::new();
    controller.enable().ok().map(|_| controller)
}

pub(super) fn handle_alt_tab_events(
    controller: &AltTabController,
    tracker: &WindowTracker,
    dock_model: &DockRuntime,
    graphics: &mut DeviceState,
    switcher: &mut SwitcherRuntime,
) -> Result<(), AppError> {
    for event in controller.drain_events() {
        match event {
            AltTabEvent::Begin {
                direction,
                foreground,
            } => switcher.begin(
                direction,
                foreground,
                tracker.current_windows(),
                dock_model.settings(),
                graphics,
            )?,
            AltTabEvent::Cycle(direction) => switcher.cycle(direction),
            AltTabEvent::Commit => switcher.commit(),
            AltTabEvent::Cancel => switcher.hide(),
        }
    }
    Ok(())
}
