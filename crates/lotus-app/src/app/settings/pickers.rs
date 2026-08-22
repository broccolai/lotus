use lotus_core::settings::ApplicationIconOverride;
use lotus_settings::scene::SettingsScene;
use lotus_windows::WindowHandle;
use lotus_windows::dialog::show_error;

pub(in crate::app) enum MascotImageOutcome {
    Updated,
    Unchanged,
}

pub(in crate::app) enum ApplicationIconOutcome {
    Updated,
    Unchanged,
}

pub(in crate::app) enum ColorOutcome {
    Changed,
    Unchanged,
}

pub(in crate::app) fn choose_mascot_image(
    owner: WindowHandle,
    settings_directory: &std::path::Path,
    scene: &mut SettingsScene,
) -> MascotImageOutcome {
    match lotus_windows::image_picker::choose_image(owner) {
        Ok(Some(path)) => match lotus_windows::custom_image::import_custom_image(
            &path,
            settings_directory,
        ) {
            Ok(stored) => {
                scene.set_mascot_image_path(Some(stored.to_string_lossy().into_owned()));
                MascotImageOutcome::Updated
            }
            Err(error) => {
                show_error(
                    owner,
                    "Lotus Settings",
                    &format!("Lotus could not use that image.\n\n{error}"),
                );
                MascotImageOutcome::Unchanged
            }
        },
        Ok(None) | Err(_) => MascotImageOutcome::Unchanged,
    }
}

pub(in crate::app) fn choose_application_icon(
    id: &str,
    owner: WindowHandle,
    settings_directory: &std::path::Path,
    scene: &mut SettingsScene,
    applications: &[lotus_settings::scene::SettingsApplicationRecord],
) -> ApplicationIconOutcome {
    let Some(record) = applications
        .iter()
        .find(|record| record.id.eq_ignore_ascii_case(id))
        .cloned()
    else {
        return ApplicationIconOutcome::Unchanged;
    };

    match lotus_windows::image_picker::choose_image(owner) {
        Ok(Some(path)) => {
            match lotus_windows::custom_image::import_application_icon(
                &path,
                settings_directory,
            ) {
                Ok(stored) => {
                    scene.set_application_icon_override(ApplicationIconOverride {
                        id: record.id,
                        image_path: stored.to_string_lossy().into_owned(),
                        app_user_model_id: record.app_user_model_id,
                        match_executables: record.match_executables,
                    });
                    ApplicationIconOutcome::Updated
                }
                Err(error) => {
                    show_error(
                        owner,
                        "Lotus Settings",
                        &format!("Lotus could not use that image.\n\n{error}"),
                    );
                    ApplicationIconOutcome::Unchanged
                }
            }
        }
        Ok(None) | Err(_) => ApplicationIconOutcome::Unchanged,
    }
}

#[derive(Clone, Copy)]
pub(in crate::app) enum ColorTarget {
    Background,
    Accent,
    Foreground,
}

pub(in crate::app) fn choose_color(
    scene: &mut SettingsScene,
    owner: WindowHandle,
    target: ColorTarget,
) -> ColorOutcome {
    let initial = match target {
        ColorTarget::Background => &scene.draft().background_color,
        ColorTarget::Accent => &scene.draft().accent_color,
        ColorTarget::Foreground => &scene.draft().foreground_color,
    };

    match lotus_windows::color_picker::choose_color(owner, initial) {
        Ok(Some(color)) => {
            match target {
                ColorTarget::Background => scene.set_background_color(color),
                ColorTarget::Accent => scene.set_accent_color(color),
                ColorTarget::Foreground => scene.set_foreground_color(color),
            }
            ColorOutcome::Changed
        }
        Ok(None) | Err(_) => ColorOutcome::Unchanged,
    }
}
