use lotus_windows::graphics::DeviceState;
use lotus_windows::input::{InputAction, InputActionBatch, capture_age};
use lotus_windows::window::DockWindow;
use lotus_windows::window_tracker::WindowTracker;

use super::ModuleHost;
use crate::app::DockRuntime;
use crate::app::switcher::SwitcherApplicationContext;

pub(in crate::app) enum InputDrainOutcome {
    NoPresentationChange,
    RequestFrame,
}

impl InputDrainOutcome {
    pub(in crate::app) const fn requests_frame(&self) -> bool {
        matches!(self, Self::RequestFrame)
    }
}

impl ModuleHost {
    pub(in crate::app) fn input_enabled(&self) -> bool {
        self.lifecycle.input_enabled()
    }

    pub(in crate::app) fn input_healthy(&self) -> bool {
        self.lifecycle.input_healthy()
    }

    pub(in crate::app) fn heartbeat_input(&self) {
        self.lifecycle.heartbeat_input();
    }

    pub(in crate::app) fn handle_input_actions(
        &mut self,
        dock: &DockWindow,
        tracker: &WindowTracker,
        dock_model: &DockRuntime,
        graphics: &mut DeviceState,
    ) -> InputDrainOutcome {
        let Some(batch) = self
            .lifecycle
            .input_controller()
            .map(lotus_windows::input::InputController::drain_action_batch)
        else {
            return InputDrainOutcome::NoPresentationChange;
        };

        self.apply_input_batch(&batch, dock, tracker, dock_model, graphics)
    }

    fn apply_input_batch(
        &mut self,
        batch: &InputActionBatch,
        dock: &DockWindow,
        tracker: &WindowTracker,
        dock_model: &DockRuntime,
        graphics: &mut DeviceState,
    ) -> InputDrainOutcome {
        let mut changed = false;
        if batch.cancelled_sequence().is_some() {
            self.switcher.hide();
            changed = true;
        }

        for event in batch.actions() {
            match event {
                InputAction::AltTabBegin {
                    sequence,
                    direction,
                    captured_at,
                } => {
                    if !batch.claim(sequence) {
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
                        batch.reject(sequence);
                    }
                    changed = true;
                }
                InputAction::AltTabCyclesPending { sequence } => {
                    if !batch.claim(sequence) {
                        continue;
                    }
                    let cycles = batch.take_alt_tab_cycles(sequence);
                    self.switcher.cycle_by(cycles);
                    changed |= cycles != 0;
                }
                InputAction::AltTabCommit {
                    sequence,
                    captured_at,
                } => {
                    if !batch.claim(sequence) {
                        continue;
                    }
                    lotus_windows::responsiveness::METRICS
                        .record_input_delivery(capture_age(captured_at));
                    self.switcher.commit();
                    changed = true;
                }
                InputAction::AltTabCancel { sequence } => {
                    if !batch.claim(sequence) {
                        continue;
                    }
                    self.switcher.hide();
                    changed = true;
                }
                InputAction::ToggleSearch {
                    sequence,
                    captured_at,
                } => {
                    if !batch.claim(sequence) {
                        continue;
                    }

                    lotus_windows::responsiveness::METRICS
                        .record_input_delivery(capture_age(captured_at));

                    if self.launcher.is_visible() {
                        self.launcher.hide();
                        changed = true;
                        continue;
                    }

                    self.applications.refresh_launcher_catalog_if_stale();
                    let catalog = self.applications.prepare_launcher_catalog(
                        dock_model.items(),
                        &dock_model.settings().hidden_executables,
                    );

                    if let Err(error) =
                        self.launcher.open(dock, dock_model, catalog, graphics)
                    {
                        lotus_windows::diagnostics::record_error("input.search", &error);
                        batch.reject(sequence);
                    } else {
                        changed = true;
                    }
                }
            }
        }
        if changed {
            InputDrainOutcome::RequestFrame
        } else {
            InputDrainOutcome::NoPresentationChange
        }
    }
}
