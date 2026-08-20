use lotus_windows::dialog::{confirm_install_update, show_error, show_information};
use lotus_windows::graphics::SettingsUpdateActivity;
use lotus_windows::interaction::request_exit;
use lotus_windows::update::{UpdateResult, UpdateStatus, is_installed, launch_installer};

use crate::app::settings::SettingsRuntime;

pub(super) fn start_update_check(settings: &mut SettingsRuntime) {
    let owner = settings.window.handle();
    match settings.start_update_check() {
        Ok(true) => {
            settings.invalidate();
        }
        Ok(false) => {}
        Err(error) => {
            show_error(owner, "Lotus Update", &error.to_string());
        }
    }
}

pub(super) fn handle_update_results(settings: &mut SettingsRuntime) {
    for result in settings.drain_update_results() {
        match result {
            UpdateResult::Checked(result) => {
                handle_update_check(result, settings);
            }
            UpdateResult::Staged(result) => {
                handle_staged_update(result, settings);
            }
        }
    }
}

fn handle_update_check(
    result: Result<UpdateStatus, lotus_windows::update::UpdateError>,
    settings: &mut SettingsRuntime,
) {
    let owner = settings.window.handle();
    let installed = match is_installed() {
        Ok(installed) => installed,
        Err(error) => {
            reset_update_activity(settings);
            show_error(owner, "Lotus Update", &error.to_string());
            return;
        }
    };

    match result {
        Ok(UpdateStatus::Current { release }) if installed => {
            reset_update_activity(settings);
            show_information(
                owner,
                "Lotus is up to date",
                &format!(
                    "You are running the latest Lotus release ({}).",
                    release.version
                ),
            );
        }
        Ok(UpdateStatus::Current { release }) => {
            offer_update(settings, release, false);
        }
        Ok(UpdateStatus::Ahead { current, release }) => {
            reset_update_activity(settings);
            show_information(
                owner,
                "Lotus is ahead of the latest release",
                &format!(
                    "You are running Lotus {current}; the latest published release is {}.",
                    release.version
                ),
            );
        }
        Ok(UpdateStatus::Available { release, .. }) => {
            offer_update(settings, release, installed);
        }
        Err(error) => {
            reset_update_activity(settings);
            show_error(
                owner,
                "Lotus Update",
                &format!("Lotus could not check for updates.\n\n{error}"),
            );
        }
    }
}

fn offer_update(
    settings: &mut SettingsRuntime,
    release: lotus_windows::update::Release,
    installed: bool,
) {
    let owner = settings.window.handle();
    if !confirm_install_update(owner, &release.version, installed) {
        reset_update_activity(settings);
        return;
    }

    match settings.start_update_download(release) {
        Ok(true) => {
            settings.invalidate();
        }
        Ok(false) => {}
        Err(error) => {
            reset_update_activity(settings);
            show_error(owner, "Lotus Update", &error.to_string());
        }
    }
}

fn handle_staged_update(
    result: Result<lotus_windows::update::StagedUpdate, lotus_windows::update::UpdateError>,
    settings: &mut SettingsRuntime,
) {
    let owner = settings.window.handle();
    match result {
        Ok(staged) => match launch_installer(&staged) {
            Ok(()) => request_exit(0),
            Err(error) => {
                reset_update_activity(settings);
                show_error(owner, "Lotus Update", &error.to_string());
            }
        },
        Err(error) => {
            reset_update_activity(settings);
            show_error(
                owner,
                "Lotus Update",
                &format!("Lotus could not prepare the update.\n\n{error}"),
            );
        }
    }
}

fn reset_update_activity(settings: &mut SettingsRuntime) {
    let _ = settings
        .scene
        .set_update_activity(SettingsUpdateActivity::Idle);
    settings.invalidate();
}
