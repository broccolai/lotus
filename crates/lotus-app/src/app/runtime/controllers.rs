use lotus_windows::activation::launch_target;
use lotus_windows::graphics::DeviceState;
use lotus_windows::input::{InputAction, InputConfig, InputController, capture_age};
use lotus_windows::responsiveness::METRICS;
use lotus_windows::search_catalog::SearchCatalogCache;
use lotus_windows::window::DockWindow;
use lotus_windows::window_tracker::WindowTracker;

use crate::app::launcher::LauncherRuntime;
use crate::app::switcher::SwitcherRuntime;
use crate::app::{DockRuntime, RestartError};

pub(crate) fn restart_current_process() -> Result<(), RestartError> {
    let executable = std::env::current_exe()?;
    let arguments = format!("--restart-after {} --open-settings", std::process::id());
    launch_target(&executable.to_string_lossy(), Some(&arguments))?;
    Ok(())
}

pub(crate) fn enable_optional_input(
    enabled: bool,
    config: InputConfig,
) -> Option<InputController> {
    if !enabled || (!config.windows_key_search && !config.custom_alt_tab) {
        return None;
    }

    match InputController::start(config) {
        Ok(controller) => Some(controller),
        Err(error) => {
            lotus_windows::diagnostics::record_error("input.enable", &error);
            None
        }
    }
}

pub(super) struct InputEventContext<'a> {
    pub(super) controller: &'a InputController,
    pub(super) dock: &'a DockWindow,
    pub(super) tracker: &'a WindowTracker,
    pub(super) dock_model: &'a DockRuntime,
    pub(super) graphics: &'a mut DeviceState,
    pub(super) catalog: &'a SearchCatalogCache,
    pub(super) launcher: &'a mut LauncherRuntime,
    pub(super) switcher: &'a mut SwitcherRuntime,
}

pub(super) fn handle_input_actions(context: &mut InputEventContext<'_>) -> bool {
    let InputEventContext {
        controller,
        dock,
        tracker,
        dock_model,
        graphics,
        catalog,
        launcher,
        switcher,
    } = context;
    let mut changed = false;
    if controller.take_cancelled_sequence().is_some() {
        switcher.hide();
        changed = true;
    }
    for event in controller.drain_actions() {
        match event {
            InputAction::AltTabBegin {
                sequence,
                direction,
                captured_at,
            } => {
                if !controller.claim(sequence) {
                    continue;
                }
                METRICS.record_input_delivery(capture_age(captured_at));
                let foreground = lotus_windows::activation::foreground_window();
                if let Err(error) = switcher.begin(
                    direction,
                    foreground,
                    tracker.current_windows(),
                    dock_model.settings(),
                    graphics,
                ) {
                    lotus_windows::diagnostics::record_error("alt_tab.begin", &error);
                    switcher.abandon();
                    controller.reject(sequence);
                }
                changed = true;
            }
            InputAction::AltTabCyclesPending { sequence } => {
                if !controller.claim(sequence) {
                    continue;
                }
                let cycles = controller.take_alt_tab_cycles(sequence);
                switcher.cycle_by(cycles);
                changed |= cycles != 0;
            }
            InputAction::AltTabCommit {
                sequence,
                captured_at,
            } => {
                if !controller.claim(sequence) {
                    continue;
                }
                METRICS.record_input_delivery(capture_age(captured_at));
                switcher.commit();
                changed = true;
            }
            InputAction::AltTabCancel { sequence } => {
                if !controller.claim(sequence) {
                    continue;
                }
                switcher.hide();
                changed = true;
            }
            InputAction::ToggleSearch {
                sequence,
                captured_at,
            } => {
                if !controller.claim(sequence) {
                    continue;
                }
                METRICS.record_input_delivery(capture_age(captured_at));
                if let Err(error) = launcher.toggle(dock, dock_model, catalog, graphics) {
                    lotus_windows::diagnostics::record_error("input.search", &error);
                    controller.reject(sequence);
                } else {
                    changed = true;
                }
            }
            InputAction::ReplayIncomplete { .. } => {
                switcher.hide();
                changed = true;
            }
        }
    }
    changed
}
