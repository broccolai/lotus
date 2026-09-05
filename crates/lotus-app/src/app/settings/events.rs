use lotus_settings::scene::SettingsAction;
use lotus_windows::graphics::DeviceState;
use lotus_windows::window::{SettingsEvent, SettingsKey};

use super::SettingsRuntime;
use crate::app::AppError;

#[derive(Default)]
pub(in crate::app) struct SettingsInteraction {
    pub(in crate::app) previews: ApplicationPreviewRefresh,
    pub(in crate::app) command: Option<SettingsCommand>,
}

#[derive(Default)]
pub(in crate::app) enum ApplicationPreviewRefresh {
    #[default]
    None,
    Reload,
    Hydrate,
}

pub(in crate::app) enum SettingsCommand {
    PasteQuery,
    Action(SettingsAction),
}

impl SettingsInteraction {
    fn action(action: SettingsAction) -> Self {
        let command = match action {
            SettingsAction::None => None,
            action => Some(SettingsCommand::Action(action)),
        };
        Self {
            command,
            ..Self::default()
        }
    }

    fn refresh(previews: ApplicationPreviewRefresh) -> Self {
        Self {
            previews,
            ..Self::default()
        }
    }
}

impl SettingsRuntime {
    pub(in crate::app) fn handle_event(
        &mut self,
        event: SettingsEvent,
        graphics: &mut DeviceState,
    ) -> Result<SettingsInteraction, AppError> {
        let action = match event {
            SettingsEvent::Resized { width, height } => {
                self.resize(graphics, width, height)?;
                return Ok(SettingsInteraction::default());
            }
            SettingsEvent::DpiChanged { dpi } => {
                self.set_dpi(dpi);
                self.invalidate();
                return Ok(SettingsInteraction::default());
            }
            SettingsEvent::RenderRequested => {
                self.invalidate();
                return Ok(SettingsInteraction::default());
            }
            SettingsEvent::PointerMoved { x, y } => {
                return Ok(self.pointer_moved(x, y).map_or_else(
                    SettingsInteraction::default,
                    SettingsInteraction::action,
                ));
            }
            SettingsEvent::PointerLeft => {
                self.pointer_left();
                return Ok(SettingsInteraction::default());
            }
            SettingsEvent::PointerPressed { x, y } => {
                return Ok(u32::try_from(x)
                    .ok()
                    .zip(u32::try_from(y).ok())
                    .and_then(|(x, y)| self.pointer_pressed(x, y))
                    .map_or_else(
                        SettingsInteraction::default,
                        SettingsInteraction::action,
                    ));
            }
            SettingsEvent::PointerReleased { x, y } => {
                return Ok(self.pointer_released(x, y).map_or_else(
                    SettingsInteraction::default,
                    SettingsInteraction::action,
                ));
            }
            SettingsEvent::PointerCancelled => {
                self.pointer_cancelled();
                return Ok(SettingsInteraction::default());
            }
            SettingsEvent::Scroll { direction } => {
                return Ok(if self.scrolled(direction) {
                    SettingsInteraction::refresh(ApplicationPreviewRefresh::Hydrate)
                } else {
                    SettingsInteraction::default()
                });
            }
            SettingsEvent::TextInput(character) => {
                if self.scene.update_prompt().is_some() {
                    return Ok(SettingsInteraction::default());
                }
                if self.append_query(character) {
                    return Ok(SettingsInteraction::refresh(
                        ApplicationPreviewRefresh::Hydrate,
                    ));
                }
                return Ok(SettingsInteraction::default());
            }
            SettingsEvent::KeyPressed(SettingsKey::Backspace) => {
                if self.scene.update_prompt().is_some() {
                    return Ok(SettingsInteraction::default());
                }
                if self.remove_query() {
                    return Ok(SettingsInteraction::refresh(
                        ApplicationPreviewRefresh::Hydrate,
                    ));
                }
                return Ok(SettingsInteraction::default());
            }
            SettingsEvent::KeyPressed(SettingsKey::Paste) => {
                return Ok(
                    if self.scene.update_prompt().is_none() && self.page_is_apps() {
                        SettingsInteraction {
                            command: Some(SettingsCommand::PasteQuery),
                            ..SettingsInteraction::default()
                        }
                    } else {
                        SettingsInteraction::default()
                    },
                );
            }
            SettingsEvent::CloseRequested => SettingsAction::Close,
            SettingsEvent::KeyPressed(key) => self.translated_key(key),
        };
        Ok(match action {
            SettingsAction::Changed
                if self.page_is_apps() && self.applications_are_empty() =>
            {
                SettingsInteraction::refresh(ApplicationPreviewRefresh::Reload)
            }
            SettingsAction::Changed if self.page_is_apps() => {
                SettingsInteraction::refresh(ApplicationPreviewRefresh::Hydrate)
            }
            SettingsAction::Reverted | SettingsAction::OpenApplications => {
                SettingsInteraction::refresh(ApplicationPreviewRefresh::Reload)
            }
            action => SettingsInteraction::action(action),
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
