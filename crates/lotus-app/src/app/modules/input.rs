use lotus_windows::graphics::DeviceState;
use lotus_windows::input::{InputAction, capture_age};
use lotus_windows::window::DockWindow;
use lotus_windows::window_tracker::WindowTracker;

use super::ModuleHost;
use crate::app::DockRuntime;
use crate::app::switcher::SwitcherApplicationContext;

impl ModuleHost {
    pub(in crate::app) fn input_enabled(&self) -> bool {
        self.modules.input_enabled()
    }

    pub(in crate::app) fn input_healthy(&self) -> bool {
        self.modules.input_healthy()
    }

    pub(in crate::app) fn heartbeat_input(&self) {
        self.modules.heartbeat_input();
    }

    pub(in crate::app) fn handle_input_actions(
        &mut self,
        dock: &DockWindow,
        tracker: &WindowTracker,
        dock_model: &DockRuntime,
        graphics: &mut DeviceState,
    ) -> bool {
        let Some(controller) = self.modules.input.as_ref() else {
            return false;
        };

        let mut changed = false;
        if controller.take_cancelled_sequence().is_some() {
            self.switcher.hide();
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
                    lotus_windows::responsiveness::METRICS
                        .record_input_delivery(capture_age(captured_at));
                    let foreground = lotus_windows::activation::foreground_window();
                    if let Err(error) = self.switcher.begin(
                        direction,
                        foreground,
                        tracker.current_windows(),
                        dock_model.settings(),
                        SwitcherApplicationContext {
                            catalog: self.applications.snapshot(),
                            assignments: dock_model.application_assignments(),
                        },
                        graphics,
                    ) {
                        lotus_windows::diagnostics::record_error("alt_tab.begin", &error);
                        self.switcher.abandon();
                        controller.reject(sequence);
                    }
                    changed = true;
                }
                InputAction::AltTabCyclesPending { sequence } => {
                    if !controller.claim(sequence) {
                        continue;
                    }
                    let cycles = controller.take_alt_tab_cycles(sequence);
                    self.switcher.cycle_by(cycles);
                    changed |= cycles != 0;
                }
                InputAction::AltTabCommit {
                    sequence,
                    captured_at,
                } => {
                    if !controller.claim(sequence) {
                        continue;
                    }
                    lotus_windows::responsiveness::METRICS
                        .record_input_delivery(capture_age(captured_at));
                    self.switcher.commit();
                    changed = true;
                }
                InputAction::AltTabCancel { sequence } => {
                    if !controller.claim(sequence) {
                        continue;
                    }
                    self.switcher.hide();
                    changed = true;
                }
                InputAction::ToggleSearch {
                    sequence,
                    captured_at,
                } => {
                    if !controller.claim(sequence) {
                        continue;
                    }
                    lotus_windows::responsiveness::METRICS
                        .record_input_delivery(capture_age(captured_at));
                    if let Err(error) =
                        self.launcher
                            .toggle(dock, dock_model, &self.applications, graphics)
                    {
                        lotus_windows::diagnostics::record_error("input.search", &error);
                        controller.reject(sequence);
                    } else {
                        changed = true;
                    }
                }
            }
        }
        changed
    }
}
