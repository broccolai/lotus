use lotus_settings::scene::SettingsAction;
use lotus_windows::graphics::DeviceState;
use lotus_windows::window::{SettingsEvent, SettingsKey};

use super::SettingsRuntime;
use crate::app::AppError;

pub(in crate::app) enum SettingsEventOutcome {
    None,
    RefreshApplications,
    HydrateApplicationPreviews,
    PasteQuery,
    Action(SettingsAction),
}

impl SettingsRuntime {
    pub(in crate::app) fn handle_event(
        &mut self,
        event: SettingsEvent,
        graphics: &mut DeviceState,
    ) -> Result<SettingsEventOutcome, AppError> {
        let action = match event {
            SettingsEvent::Resized { width, height } => {
                self.resize(graphics, width, height)?;
                return Ok(SettingsEventOutcome::None);
            }
            SettingsEvent::DpiChanged { dpi } => {
                self.set_dpi(dpi);
                self.invalidate();
                return Ok(SettingsEventOutcome::None);
            }
            SettingsEvent::RenderRequested => {
                self.invalidate();
                return Ok(SettingsEventOutcome::None);
            }
            SettingsEvent::PointerMoved { x, y } => {
                return Ok(u32::try_from(x)
                    .ok()
                    .zip(u32::try_from(y).ok())
                    .and_then(|(x, y)| self.pointer_moved(x, y))
                    .map_or(SettingsEventOutcome::None, SettingsEventOutcome::Action));
            }
            SettingsEvent::PointerLeft => {
                self.pointer_left();
                return Ok(SettingsEventOutcome::None);
            }
            SettingsEvent::PointerPressed { x, y } => {
                return Ok(u32::try_from(x)
                    .ok()
                    .zip(u32::try_from(y).ok())
                    .and_then(|(x, y)| self.pointer_pressed(x, y))
                    .map_or(SettingsEventOutcome::None, SettingsEventOutcome::Action));
            }
            SettingsEvent::PointerReleased { x, y } => {
                return Ok(self
                    .pointer_released(x, y)
                    .map_or(SettingsEventOutcome::None, SettingsEventOutcome::Action));
            }
            SettingsEvent::PointerCancelled => {
                self.pointer_cancelled();
                return Ok(SettingsEventOutcome::None);
            }
            SettingsEvent::Scroll { direction } => {
                return Ok(if self.scrolled(direction) {
                    SettingsEventOutcome::HydrateApplicationPreviews
                } else {
                    SettingsEventOutcome::None
                });
            }
            SettingsEvent::TextInput(character) => {
                if self.append_query(character) {
                    return Ok(SettingsEventOutcome::HydrateApplicationPreviews);
                }
                return Ok(SettingsEventOutcome::None);
            }
            SettingsEvent::KeyPressed(SettingsKey::Backspace) => {
                if self.remove_query() {
                    return Ok(SettingsEventOutcome::HydrateApplicationPreviews);
                }
                return Ok(SettingsEventOutcome::None);
            }
            SettingsEvent::KeyPressed(SettingsKey::Paste) => {
                return Ok(if self.page_is_apps() {
                    SettingsEventOutcome::PasteQuery
                } else {
                    SettingsEventOutcome::None
                });
            }
            SettingsEvent::CloseRequested => SettingsAction::Close,
            SettingsEvent::KeyPressed(key) => self.translated_key(key),
        };
        Ok(match action {
            SettingsAction::Changed
                if self.page_is_apps() && self.applications_are_empty() =>
            {
                SettingsEventOutcome::RefreshApplications
            }
            SettingsAction::Changed if self.page_is_apps() => {
                SettingsEventOutcome::HydrateApplicationPreviews
            }
            SettingsAction::Reverted | SettingsAction::OpenApplications => {
                SettingsEventOutcome::RefreshApplications
            }
            action => SettingsEventOutcome::Action(action),
        })
    }

    pub(in crate::app) fn paste_query(&mut self, clipboard: &str) -> bool {
        if !self.page_is_apps() {
            return false;
        }
        let pasted = clipboard
            .chars()
            .filter(|character| !character.is_control())
            .collect::<String>();
        !pasted.is_empty()
            && self.update_query(&format!("{}{pasted}", self.application_query()))
    }

    fn append_query(&mut self, character: char) -> bool {
        !character.is_control()
            && self.page_is_apps()
            && self.update_query(&format!("{}{character}", self.application_query()))
    }

    fn remove_query(&mut self) -> bool {
        if !self.page_is_apps() {
            return false;
        }
        let mut query = self.application_query().to_owned();
        let _ = query.pop();
        self.update_query(&query)
    }

    fn update_query(&mut self, query: &str) -> bool {
        let changed = self.set_application_query(query);
        if changed {
            self.invalidate();
        }
        changed
    }
}
