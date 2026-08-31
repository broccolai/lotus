use lotus_windows::dialog::{show_error, show_information};
use lotus_windows::interaction::request_exit;
use lotus_windows::update::{UpdateResult, UpdateStatus, is_installed, launch_installer};

use crate::app::modules::ModuleHost;

pub(super) fn start_update_check(auxiliary: &mut ModuleHost) {
    let owner = auxiliary.settings_owner();
    match auxiliary.start_update_check() {
        Ok(true) => {
            auxiliary.invalidate_settings();
        }
        Ok(false) => {}
        Err(error) => {
            show_error(owner, "Lotus Update", &error.to_string());
        }
    }
}

pub(super) fn handle_update_results(auxiliary: &mut ModuleHost) {
    for result in auxiliary.drain_update_results() {
        match result {
            UpdateResult::Checked(result) => {
                handle_update_check(result, auxiliary);
            }
            UpdateResult::Staged(result) => {
                handle_staged_update(result, auxiliary);
            }
        }
    }
}

fn handle_update_check(
    result: Result<UpdateStatus, lotus_windows::update::UpdateError>,
    auxiliary: &mut ModuleHost,
) {
    let owner = auxiliary.settings_owner();
    let installed = match is_installed() {
        Ok(installed) => installed,
        Err(error) => {
            reset_update_activity(auxiliary);
            show_error(owner, "Lotus Update", &error.to_string());
            return;
        }
    };

    match result {
        Ok(UpdateStatus::Current { release }) if installed => {
            reset_update_activity(auxiliary);
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
            offer_update(auxiliary, release, false);
        }
        Ok(UpdateStatus::Ahead { current, release }) => {
            reset_update_activity(auxiliary);
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
            offer_update(auxiliary, release, installed);
        }
        Err(error) => {
            reset_update_activity(auxiliary);
            show_error(
                owner,
                "Lotus Update",
                &format!("Lotus could not check for updates.\n\n{error}"),
            );
        }
    }
}

fn offer_update(
    auxiliary: &mut ModuleHost,
    release: lotus_windows::update::Release,
    installed: bool,
) {
    if !auxiliary.offer_update(release, installed) {
        reset_update_activity(auxiliary);
    }
}

pub(super) fn accept_update(auxiliary: &mut ModuleHost) {
    let owner = auxiliary.settings_owner();
    let Some(release) = auxiliary.take_update_offer() else {
        reset_update_activity(auxiliary);
        return;
    };
    match auxiliary.start_update_download(release) {
        Ok(true) => {
            auxiliary.invalidate_settings();
        }
        Ok(false) => reset_update_activity(auxiliary),
        Err(error) => {
            reset_update_activity(auxiliary);
            show_error(owner, "Lotus Update", &error.to_string());
        }
    }
}

pub(super) fn cancel_update(auxiliary: &mut ModuleHost) {
    auxiliary.cancel_update_offer();
}

fn handle_staged_update(
    result: Result<lotus_windows::update::StagedUpdate, lotus_windows::update::UpdateError>,
    auxiliary: &mut ModuleHost,
) {
    let owner = auxiliary.settings_owner();
    match result {
        Ok(staged) => match launch_installer(&staged) {
            Ok(()) => request_exit(0),
            Err(error) => {
                reset_update_activity(auxiliary);
                show_error(owner, "Lotus Update", &error.to_string());
            }
        },
        Err(error) => {
            reset_update_activity(auxiliary);
            show_error(
                owner,
                "Lotus Update",
                &format!("Lotus could not prepare the update.\n\n{error}"),
            );
        }
    }
}

fn reset_update_activity(auxiliary: &mut ModuleHost) {
    auxiliary.reset_update_activity();
}
