use lotus_media::{MediaHitTarget, MediaModel, MediaWidgetAction};
use lotus_windows::dialog::show_error;
use lotus_windows::media::{MediaCommand, MediaController, MediaEvent};

use crate::app::activation;
use crate::app::dock::DockRuntime;

pub(super) struct MediaRuntime {
    controller: Option<MediaController>,
    model: MediaModel,
}

impl MediaRuntime {
    pub(super) fn new(enabled: bool) -> Self {
        Self {
            controller: enabled.then(MediaController::start).and_then(Result::ok),
            model: MediaModel::default(),
        }
    }

    pub(super) fn set_enabled(&mut self, enabled: bool) {
        match (enabled, self.controller.is_some()) {
            (true, false) => match MediaController::start() {
                Ok(controller) => self.controller = Some(controller),
                Err(error) => {
                    lotus_windows::diagnostics::record_error("media.enable", &error);
                    self.controller = None;
                }
            },
            (false, true) => {
                self.controller = None;
                let _ = self.model.replace(None);
            }
            (true, true) | (false, false) => {}
        }
    }

    pub(super) fn drain(&mut self, dock: &mut DockRuntime) -> bool {
        let Some(controller) = &self.controller else {
            return dock.replace_media(None);
        };
        let mut changed = false;
        for event in controller.drain_events() {
            match event {
                MediaEvent::Snapshot(snapshot) => {
                    changed |= self.model.replace(snapshot);
                }
                MediaEvent::Unavailable(_) => {
                    changed |= self.model.replace(None);
                }
            }
        }
        changed && dock.replace_media(self.model.snapshot())
    }

    pub(super) fn refresh(&self, dock: &mut DockRuntime) -> bool {
        dock.replace_media(self.model.snapshot())
    }

    pub(super) fn activate(
        &self,
        target: MediaHitTarget,
        dock: &mut DockRuntime,
        owner: lotus_windows::WindowHandle,
    ) {
        let Some(action) = self.model.action(target) else {
            return;
        };
        match action {
            MediaWidgetAction::FocusSource => {
                let Some(snapshot) = self.model.snapshot() else {
                    return;
                };
                let Some((source_index, item, preferred)) =
                    dock.media_application(&snapshot.source_id)
                else {
                    return;
                };
                match activation::focus_application(&item, preferred) {
                    Ok(outcome) => {
                        if let Some(window) = outcome.focused_key() {
                            dock.record_window_activation(source_index, window);
                        } else if matches!(
                            outcome,
                            activation::ActivationOutcome::ForegroundDenied
                        ) {
                            lotus_windows::diagnostics::record_diagnostic(
                                "activation.media_foreground_denied",
                                "Windows denied a media-source foreground request",
                            );
                        }
                    }
                    Err(error) => {
                        lotus_windows::diagnostics::record_error(
                            "activation.media_source",
                            &error,
                        );
                        show_error(
                            owner,
                            "Lotus media",
                            &format!(
                                "Lotus could not focus the media application.\n\n{error}"
                            ),
                        );
                    }
                }
            }
            MediaWidgetAction::Previous => self.execute(MediaCommand::Previous),
            MediaWidgetAction::Play => self.execute(MediaCommand::Play),
            MediaWidgetAction::Pause => self.execute(MediaCommand::Pause),
            MediaWidgetAction::Next => self.execute(MediaCommand::Next),
        }
    }

    fn execute(&self, command: MediaCommand) {
        if let Some(controller) = &self.controller {
            let _ = controller.execute(command);
        }
    }
}
