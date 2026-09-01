use lotus_windows::WindowHandle;
use lotus_windows::activation::launch_target;
use lotus_windows::dialog::{confirm_restart, confirm_shutdown, show_error};
use lotus_windows::interaction::request_exit;
use lotus_windows::window::SignedPoint;

use crate::app::visuals::SystemStatusKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SystemAction {
    OpenVolumeMixer,
    OpenNotificationArea {
        anchor: Option<SignedPoint>,
    },
    ShowDesktop,
    LockComputer,
    RestartComputer {
        confirmation: Confirmation,
    },
    ShutDownComputer {
        confirmation: Confirmation,
    },
    QuitLotus,
    ActivateStatus {
        kind: SystemStatusKind,
        anchor: Option<SignedPoint>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Confirmation {
    Required,
    AlreadyConfirmed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct SystemActionOutcome {
    pub(super) advanced_color_changed: bool,
}

pub(super) fn execute_system_action(
    action: SystemAction,
    owner: WindowHandle,
) -> SystemActionOutcome {
    match action {
        SystemAction::OpenVolumeMixer => {
            run_launch(owner, "open the Windows volume mixer", "sndvol.exe", None)
        }
        SystemAction::OpenNotificationArea { anchor } => {
            let result = anchor.map_or_else(
                || lotus_windows::tray::open_overflow(owner),
                |point| lotus_windows::tray::open_overflow_at(owner, point.x),
            );
            show_result(owner, result, "open the Windows notification area")
        }
        SystemAction::ShowDesktop => {
            show_result(owner, lotus_windows::desktop::toggle(), "show the desktop")
        }
        SystemAction::LockComputer => {
            show_result(owner, lotus_windows::desktop::lock(), "lock Windows")
        }
        SystemAction::RestartComputer { confirmation } => {
            if matches!(confirmation, Confirmation::Required) && !confirm_restart(owner) {
                return SystemActionOutcome::default();
            }
            run_launch(owner, "restart Windows", "shutdown.exe", Some("/r /t 0"))
        }
        SystemAction::ShutDownComputer { confirmation } => {
            if matches!(confirmation, Confirmation::Required) && !confirm_shutdown(owner) {
                return SystemActionOutcome::default();
            }
            run_launch(owner, "shut down Windows", "shutdown.exe", Some("/s /t 0"))
        }
        SystemAction::QuitLotus => {
            request_exit(0);
            SystemActionOutcome::default()
        }
        SystemAction::ActivateStatus { kind, anchor } => {
            execute_status(kind, anchor, owner)
        }
    }
}

fn execute_status(
    kind: SystemStatusKind,
    anchor: Option<SignedPoint>,
    owner: WindowHandle,
) -> SystemActionOutcome {
    let result = match kind {
        SystemStatusKind::Volume => native_panel_or_fallback(
            anchor.map_or_else(
                || lotus_windows::tray::open_quick_settings(owner),
                |point| lotus_windows::tray::open_quick_settings_at(owner, point.x),
            ),
            "sndvol.exe",
        ),
        SystemStatusKind::AdvancedColor => {
            return match lotus_windows::advanced_color::toggle(owner) {
                Ok(_) => SystemActionOutcome {
                    advanced_color_changed: true,
                },
                Err(error) => {
                    show_error(
                        owner,
                        "Lotus",
                        &format!("Lotus could not open that system control.\n\n{error}"),
                    );
                    SystemActionOutcome::default()
                }
            };
        }
        SystemStatusKind::Network => native_panel_or_fallback(
            anchor.map_or_else(
                || lotus_windows::tray::open_quick_settings(owner),
                |point| lotus_windows::tray::open_quick_settings_at(owner, point.x),
            ),
            "ms-settings:network",
        ),
        SystemStatusKind::BackgroundApps => anchor
            .map_or_else(
                || lotus_windows::tray::open_overflow(owner),
                |point| lotus_windows::tray::open_overflow_at(owner, point.x),
            )
            .map_err(|error| error.to_string()),
        SystemStatusKind::DateTime => native_panel_or_fallback(
            anchor.map_or_else(
                || lotus_windows::tray::open_calendar(owner),
                |point| lotus_windows::tray::open_calendar_at(owner, point.x),
            ),
            "ms-settings:dateandtime",
        ),
    };
    show_result(owner, result, "open that system control")
}

fn native_panel_or_fallback(
    native: Result<bool, lotus_windows::tray::TrayError>,
    fallback: &str,
) -> Result<(), String> {
    match native {
        Ok(true) => Ok(()),
        Ok(false) => launch_target(fallback, None).map_err(|error| error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

fn run_launch(
    owner: WindowHandle,
    operation: &str,
    target: &str,
    arguments: Option<&str>,
) -> SystemActionOutcome {
    show_result(owner, launch_target(target, arguments), operation)
}

fn show_result<T, E>(
    owner: WindowHandle,
    result: Result<T, E>,
    operation: &str,
) -> SystemActionOutcome
where
    E: std::fmt::Display,
{
    if let Err(error) = result {
        show_error(
            owner,
            "Lotus",
            &format!("Lotus could not {operation}.\n\n{error}"),
        );
    }
    SystemActionOutcome::default()
}
