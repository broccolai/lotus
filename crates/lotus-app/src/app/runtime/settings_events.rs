use std::path::Path;

use lotus_core::search::ApplicationEntry;
use lotus_core::settings::{ApplicationIconOverride, DockSettings};
use lotus_core::window::is_reliable_application_identity;
use lotus_settings::scene::{SettingsApplicationRecord, SettingsPointerStyle};
use lotus_windows::clipboard::read_text;
use lotus_windows::custom_image::CustomImageCache;
use lotus_windows::dialog::show_error;
use lotus_windows::graphics::{
    CompositionSurfaceState, DeviceState, SettingsAction, SettingsKey as SceneSettingsKey,
};
use lotus_windows::interaction::{PointerCursor, request_exit};
use lotus_windows::native_icon::NativeIconCache;
use lotus_windows::startup as startup_registration;
use lotus_windows::window::{DockWindow, SettingsEvent, SettingsKey as WindowSettingsKey};
use lotus_windows::window_tracker::WindowTracker;

use super::presentation::{apply_fullscreen_visibility, render_and_schedule, resize_dock};
use super::{controllers, update_events};
use crate::app::settings::SettingsRuntime;
use crate::app::switcher::AuxiliaryWindows;
use crate::app::{AppError, DockRuntime};

pub(super) struct SettingsEventContext<'a> {
    pub(super) dock: &'a DockWindow,
    pub(super) graphics: &'a mut DeviceState,
    pub(super) dock_surface: &'a mut CompositionSurfaceState,
    pub(super) window_tracker: &'a WindowTracker,
    pub(super) dock_model: &'a mut DockRuntime,
    pub(super) auxiliary: &'a mut AuxiliaryWindows,
}

pub(super) fn handle_settings_event(
    event: SettingsEvent,
    context: &mut SettingsEventContext<'_>,
) -> Result<(), AppError> {
    let action = match event {
        SettingsEvent::Resized { width, height } => {
            context
                .auxiliary
                .settings
                .resize(context.graphics, width, height)?;
            return Ok(());
        }
        SettingsEvent::DpiChanged { dpi } => {
            let _ = context.auxiliary.settings.scene.set_dpi(dpi);
            context.auxiliary.settings.render(context.graphics)?;
            return Ok(());
        }
        SettingsEvent::RenderRequested => {
            context.auxiliary.settings.render(context.graphics)?;
            return Ok(());
        }
        SettingsEvent::PointerMoved { x, y } => {
            let Some((x, y)) = u32::try_from(x).ok().zip(u32::try_from(y).ok()) else {
                return Ok(());
            };
            let cursor = if context.auxiliary.settings.dragging_slider.is_some() {
                PointerCursor::HorizontalResize
            } else {
                settings_pointer_cursor(
                    context.auxiliary.settings.scene.pointer_style(x, y),
                )
            };
            context.auxiliary.settings.window.set_pointer_cursor(cursor);
            if let Some(slider) = context.auxiliary.settings.dragging_slider {
                context.auxiliary.settings.scene.pointer_move(x, y);
                let action = context
                    .auxiliary
                    .settings
                    .scene
                    .set_slider_from_pointer(slider, x);
                return apply_settings_action(action, context);
            }
            if context.auxiliary.settings.scene.pointer_move(x, y) {
                context.auxiliary.settings.render(context.graphics)?;
            }
            return Ok(());
        }
        SettingsEvent::PointerLeft => {
            context
                .auxiliary
                .settings
                .window
                .set_pointer_cursor(PointerCursor::Arrow);
            if context.auxiliary.settings.scene.set_hovered(None) {
                context.auxiliary.settings.render(context.graphics)?;
            }
            return Ok(());
        }
        SettingsEvent::PointerPressed { x, y } => {
            let Some((x, y)) = u32::try_from(x).ok().zip(u32::try_from(y).ok()) else {
                return Ok(());
            };
            context.auxiliary.settings.scene.pointer_move(x, y);
            context.auxiliary.settings.dragging_slider =
                context.auxiliary.settings.scene.slider_at(x, y);
            if let Some(slider) = context.auxiliary.settings.dragging_slider {
                let action = context
                    .auxiliary
                    .settings
                    .scene
                    .set_slider_from_pointer(slider, x);
                return apply_settings_action(action, context);
            }
            return Ok(());
        }
        SettingsEvent::PointerReleased { x, y } => {
            if context.auxiliary.settings.dragging_slider.take().is_some() {
                let cursor = u32::try_from(x).ok().zip(u32::try_from(y).ok()).map_or(
                    PointerCursor::Arrow,
                    |(x, y)| {
                        settings_pointer_cursor(
                            context.auxiliary.settings.scene.pointer_style(x, y),
                        )
                    },
                );
                context.auxiliary.settings.window.set_pointer_cursor(cursor);
                return Ok(());
            }
            u32::try_from(x)
                .ok()
                .zip(u32::try_from(y).ok())
                .map_or(SettingsAction::None, |(x, y)| {
                    context.auxiliary.settings.scene.pointer_activate(x, y)
                })
        }
        SettingsEvent::Scroll { direction } => {
            if context.auxiliary.settings.scene.scroll(direction) {
                if context.auxiliary.settings.scene.page()
                    == lotus_settings::scene::SettingsPage::Apps
                {
                    hydrate_application_previews(
                        &context.auxiliary.applications,
                        context.dock_model.items(),
                        &mut context.auxiliary.settings,
                    );
                }
                context.auxiliary.settings.render(context.graphics)?;
            }
            return Ok(());
        }
        SettingsEvent::CloseRequested => SettingsAction::Close,
        SettingsEvent::TextInput(character) => {
            return append_application_query(character, context);
        }
        SettingsEvent::KeyPressed(key) => {
            match key {
                WindowSettingsKey::Backspace => return remove_application_query(context),
                WindowSettingsKey::Paste => return paste_application_query(context),
                _ => {}
            }
            settings_key_action(&mut context.auxiliary.settings, key)
        }
    };

    apply_settings_action(action, context)
}

fn append_application_query(
    character: char,
    context: &mut SettingsEventContext<'_>,
) -> Result<(), AppError> {
    if context.auxiliary.settings.scene.page() != lotus_settings::scene::SettingsPage::Apps
        || character.is_control()
    {
        return Ok(());
    }
    let mut query = context
        .auxiliary
        .settings
        .scene
        .application_query()
        .to_owned();
    query.push(character);
    update_application_query(&query, context)
}

fn remove_application_query(
    context: &mut SettingsEventContext<'_>,
) -> Result<(), AppError> {
    if context.auxiliary.settings.scene.page() != lotus_settings::scene::SettingsPage::Apps
    {
        return Ok(());
    }
    let mut query = context
        .auxiliary
        .settings
        .scene
        .application_query()
        .to_owned();
    let _ = query.pop();
    update_application_query(&query, context)
}

fn paste_application_query(context: &mut SettingsEventContext<'_>) -> Result<(), AppError> {
    if context.auxiliary.settings.scene.page() != lotus_settings::scene::SettingsPage::Apps
    {
        return Ok(());
    }

    let Ok(clipboard) = read_text() else {
        return Ok(());
    };
    let pasted = clipboard
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>();
    if pasted.is_empty() {
        return Ok(());
    }

    let query = format!(
        "{}{}",
        context.auxiliary.settings.scene.application_query(),
        pasted
    );
    update_application_query(&query, context)
}

fn update_application_query(
    query: &str,
    context: &mut SettingsEventContext<'_>,
) -> Result<(), AppError> {
    if context
        .auxiliary
        .settings
        .scene
        .set_application_query(query)
    {
        hydrate_application_previews(
            &context.auxiliary.applications,
            context.dock_model.items(),
            &mut context.auxiliary.settings,
        );
        context.auxiliary.settings.render(context.graphics)?;
    }
    Ok(())
}

fn settings_pointer_cursor(style: SettingsPointerStyle) -> PointerCursor {
    match style {
        SettingsPointerStyle::Default => PointerCursor::Arrow,
        SettingsPointerStyle::Action => PointerCursor::Hand,
        SettingsPointerStyle::HorizontalAdjustment => PointerCursor::HorizontalResize,
    }
}

fn settings_key_action(
    runtime: &mut SettingsRuntime,
    key: WindowSettingsKey,
) -> SettingsAction {
    let key = match key {
        WindowSettingsKey::Escape => SceneSettingsKey::Escape,
        WindowSettingsKey::Enter | WindowSettingsKey::Space => SceneSettingsKey::Activate,
        WindowSettingsKey::Tab { reverse: false } => SceneSettingsKey::Tab,
        WindowSettingsKey::Tab { reverse: true } => SceneSettingsKey::ReverseTab,
        WindowSettingsKey::Left => SceneSettingsKey::Left,
        WindowSettingsKey::Right => SceneSettingsKey::Right,
        WindowSettingsKey::Up => SceneSettingsKey::Up,
        WindowSettingsKey::Down => SceneSettingsKey::Down,
        WindowSettingsKey::Save if runtime.scene.is_dirty() => {
            return SettingsAction::Apply(Box::new(
                runtime.scene.draft().clone().normalized(),
            ));
        }
        WindowSettingsKey::Backspace
        | WindowSettingsKey::Save
        | WindowSettingsKey::Paste => {
            return SettingsAction::None;
        }
    };
    runtime.scene.key(key)
}

fn apply_settings_action(
    action: SettingsAction,
    context: &mut SettingsEventContext<'_>,
) -> Result<(), AppError> {
    match action {
        SettingsAction::None => Ok(()),
        SettingsAction::Changed => {
            let needs_application_manager = context.auxiliary.settings.scene.page()
                == lotus_settings::scene::SettingsPage::Apps
                && context.auxiliary.settings.scene.applications().is_empty();
            if needs_application_manager {
                refresh_application_manager(context);
            } else if context.auxiliary.settings.scene.page()
                == lotus_settings::scene::SettingsPage::Apps
            {
                hydrate_application_previews(
                    &context.auxiliary.applications,
                    context.dock_model.items(),
                    &mut context.auxiliary.settings,
                );
            }
            context.auxiliary.settings.render(context.graphics)
        }
        SettingsAction::Reverted => {
            if context.auxiliary.settings.scene.page()
                == lotus_settings::scene::SettingsPage::Apps
            {
                refresh_application_manager(context);
            }
            context.auxiliary.settings.render(context.graphics)
        }
        SettingsAction::OpenApplications => {
            refresh_application_manager(context);
            context.auxiliary.settings.render(context.graphics)
        }
        SettingsAction::ChooseBackgroundColor => {
            choose_settings_color(context, SettingsColor::Background)
        }
        SettingsAction::ChooseAccentColor => {
            choose_settings_color(context, SettingsColor::Accent)
        }
        SettingsAction::ChooseForegroundColor => {
            choose_settings_color(context, SettingsColor::Foreground)
        }
        SettingsAction::ChooseMascotImage => {
            let owner = context.auxiliary.settings.window.handle();
            match lotus_windows::image_picker::choose_image(owner) {
                Ok(Some(path)) => match lotus_windows::custom_image::import_custom_image(
                    &path,
                    context.dock_model.settings_directory(),
                ) {
                    Ok(stored) => {
                        context.auxiliary.settings.scene.set_mascot_image_path(Some(
                            stored.to_string_lossy().into_owned(),
                        ));
                        context.auxiliary.settings.render(context.graphics)
                    }
                    Err(error) => {
                        show_error(
                            owner,
                            "Lotus Settings",
                            &format!("Lotus could not use that image.\n\n{error}"),
                        );
                        Ok(())
                    }
                },
                Ok(None) => Ok(()),
                Err(error) => {
                    show_error(
                        owner,
                        "Lotus Settings",
                        &format!("Lotus could not open the image picker.\n\n{error}"),
                    );
                    Ok(())
                }
            }
        }
        SettingsAction::ChooseApplicationIcon(id) => choose_application_icon(&id, context),
        SettingsAction::ResetApplicationIcon(id) => {
            context
                .auxiliary
                .settings
                .scene
                .reset_application_icon_override(&id);
            context.auxiliary.settings.custom_images.clear();
            refresh_application_manager(context);
            context.auxiliary.settings.render(context.graphics)
        }
        SettingsAction::CheckForUpdates => update_events::start_update_check(
            &mut context.auxiliary.settings,
            context.graphics,
        ),
        SettingsAction::ReplaySetup => context.auxiliary.settings.open_onboarding(
            context.dock_model.settings(),
            false,
            context.graphics,
        ),
        SettingsAction::Close => {
            if context.auxiliary.settings.scene.onboarding_required() {
                return Ok(());
            }
            context.auxiliary.settings.hide();
            Ok(())
        }
        SettingsAction::Apply(next) => apply_changed_settings(*next, context, false),
        SettingsAction::CompleteOnboarding(next) => {
            let initial_setup = context.auxiliary.settings.scene.onboarding_required();
            context.auxiliary.settings.scene.end_onboarding();
            apply_changed_settings(*next, context, initial_setup)?;
            if !initial_setup {
                context.auxiliary.settings.hide();
            }
            Ok(())
        }
    }
}

fn apply_changed_settings(
    mut next: DockSettings,
    context: &mut SettingsEventContext<'_>,
    restart_after_apply: bool,
) -> Result<(), AppError> {
    next.application_icon_overrides = context
        .auxiliary
        .settings
        .scene
        .merge_application_icon_overrides(context.dock_model.settings());
    preserve_externally_managed_settings(&mut next, context.dock_model.settings());

    let start_with_windows = next.start_with_windows;
    let impact = context
        .dock_model
        .apply_settings(next, context.window_tracker.current_windows())?;
    context
        .auxiliary
        .settings
        .window
        .use_material(context.dock_model.settings());
    if restart_after_apply {
        context
            .auxiliary
            .settings
            .scene
            .mark_applied(context.dock_model.settings().clone());
        context.auxiliary.settings.hide();
        if let Err(error) = controllers::restart_current_process() {
            context
                .auxiliary
                .settings
                .open(context.dock_model.settings(), context.graphics)?;
            show_error(
                context.auxiliary.settings.window.handle(),
                "Lotus Settings",
                &format!("Lotus saved your settings but could not restart.\n\n{error}"),
            );
        } else {
            request_exit(0);
        }
        return Ok(());
    }
    if let Err(error) = startup_registration::sync(start_with_windows) {
        show_error(
            context.auxiliary.settings.window.handle(),
            "Lotus Settings",
            &format!(
                "Lotus saved your preference but could not update Windows startup.\n\n{error}"
            ),
        );
    }
    if !impact.changed {
        return Ok(());
    }

    context.dock.set_status_refresh_active(
        context.dock_model.settings().show_system_status
            && context.dock_model.settings().show_date_time_status,
    )?;
    context
        .auxiliary
        .media
        .set_enabled(context.dock_model.settings().show_media_controls);
    if context.dock_model.settings().show_media_controls {
        let _changed = context.auxiliary.media.refresh(context.dock_model);
    } else {
        let _changed = context.auxiliary.media.drain(context.dock_model);
    }

    lotus_windows::backdrop::apply_dock_settings(
        context.dock.handle(),
        context.dock_model.settings(),
    );
    apply_auxiliary_settings(context)?;
    resize_dock(
        context.dock,
        context.graphics,
        context.dock_surface,
        context.dock_model,
    )?;
    context.auxiliary.status.sync(
        context.dock,
        context.dock_model.settings(),
        context.dock_model.media(),
        context.graphics,
    )?;
    render_and_schedule(
        context.dock,
        context.graphics,
        context.dock_surface,
        context.dock_model.scene(),
        context.auxiliary.launcher.needs_animation(),
    )?;
    apply_fullscreen_visibility(
        context.dock,
        context.window_tracker,
        context.dock_model,
        &mut context.auxiliary.launcher,
        &context.auxiliary.status,
    )?;
    context
        .auxiliary
        .settings
        .scene
        .mark_applied(context.dock_model.settings().clone());
    context.auxiliary.settings.custom_images.clear();
    if context.auxiliary.settings.scene.page() == lotus_settings::scene::SettingsPage::Apps
    {
        refresh_application_manager(context);
    }
    context.auxiliary.settings.render(context.graphics)?;

    if impact.restart_required {
        if let Err(error) = controllers::restart_current_process() {
            show_error(
                context.auxiliary.settings.window.handle(),
                "Lotus Settings",
                &format!("Lotus saved your settings but could not restart.\n\n{error}"),
            );
        } else {
            request_exit(0);
        }
    }
    Ok(())
}

fn choose_application_icon(
    id: &str,
    context: &mut SettingsEventContext<'_>,
) -> Result<(), AppError> {
    let owner = context.auxiliary.settings.window.handle();
    let Some(record) = context
        .auxiliary
        .settings
        .scene
        .applications()
        .iter()
        .find(|record| record.id.eq_ignore_ascii_case(id))
        .cloned()
    else {
        return Ok(());
    };
    match lotus_windows::image_picker::choose_image(owner) {
        Ok(Some(path)) => match lotus_windows::custom_image::import_application_icon(
            &path,
            context.dock_model.settings_directory(),
        ) {
            Ok(stored) => {
                context
                    .auxiliary
                    .settings
                    .scene
                    .set_application_icon_override(ApplicationIconOverride {
                        id: record.id,
                        image_path: stored.to_string_lossy().into_owned(),
                        app_user_model_id: record.app_user_model_id,
                        match_executables: record.match_executables,
                    });
                context.auxiliary.settings.custom_images.clear();
                refresh_application_manager(context);
                context.auxiliary.settings.render(context.graphics)
            }
            Err(error) => {
                show_error(
                    owner,
                    "Lotus Settings",
                    &format!("Lotus could not use that image.\n\n{error}"),
                );
                Ok(())
            }
        },
        Ok(None) => Ok(()),
        Err(error) => {
            show_error(
                owner,
                "Lotus Settings",
                &format!("Lotus could not open the image picker.\n\n{error}"),
            );
            Ok(())
        }
    }
}

pub(super) fn refresh_application_manager(context: &mut SettingsEventContext<'_>) {
    let selected = context
        .auxiliary
        .settings
        .scene
        .selected_application()
        .map(|application| application.id.clone());
    let settings = context.auxiliary.settings.scene.draft().clone();
    let applications = application_records(
        &context.auxiliary.applications,
        context.dock_model.items(),
        &settings,
    );
    let _ = context
        .auxiliary
        .settings
        .scene
        .set_applications(applications);
    if let Some(selected) = selected {
        let _ = context
            .auxiliary
            .settings
            .scene
            .open_application_manager(&selected);
    }
    hydrate_application_previews(
        &context.auxiliary.applications,
        context.dock_model.items(),
        &mut context.auxiliary.settings,
    );
}

pub(super) fn application_records(
    cache: &lotus_windows::search_catalog::SearchCatalogCache,
    dock_items: &[lotus_core::dock::DockItem],
    settings: &DockSettings,
) -> Vec<SettingsApplicationRecord> {
    let catalog = cache.catalog(dock_items, &[]);
    let mut applications = catalog
        .entries_for_management()
        .map(|entry| {
            let id = application_record_id(entry);
            let executable =
                lotus_windows::launch::resolve_executable(&entry.launch_target);
            let executable_name = executable
                .as_deref()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str());
            let custom = settings.application_icon_override(
                entry.app_user_model_id.as_deref(),
                Some(&id),
                executable_name,
            );
            SettingsApplicationRecord {
                id,
                name: entry.name.clone(),
                icon: None,
                app_user_model_id: entry.app_user_model_id.clone(),
                match_executables: executable_name
                    .filter(|name| !is_shared_host_executable(name))
                    .map(str::to_owned)
                    .into_iter()
                    .collect(),
                customized: custom.is_some(),
                missing_icon: custom
                    .is_some_and(|override_| !Path::new(&override_.image_path).is_file()),
            }
        })
        .collect::<Vec<_>>();
    applications.sort_by(|left, right| {
        right
            .customized
            .cmp(&left.customized)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    applications
}

pub(super) fn hydrate_application_previews(
    cache: &lotus_windows::search_catalog::SearchCatalogCache,
    dock_items: &[lotus_core::dock::DockItem],
    settings_runtime: &mut SettingsRuntime,
) {
    let layout = settings_runtime.scene.layout();
    let mut ids = layout
        .controls
        .iter()
        .filter(|entry| rects_intersect(entry.bounds, layout.content_viewport))
        .filter_map(|entry| match entry.control {
            lotus_settings::scene::SettingsControl::ApplicationRow(index) => {
                settings_runtime.scene.applications().get(index)
            }
            _ => None,
        })
        .map(|application| application.id.clone())
        .collect::<Vec<_>>();
    if let Some(selected) = settings_runtime.scene.selected_application()
        && !ids.iter().any(|id| id.eq_ignore_ascii_case(&selected.id))
    {
        ids.push(selected.id.clone());
    }

    let settings = settings_runtime.scene.draft().clone();
    let catalog = cache.catalog(dock_items, &[]);
    for id in ids {
        let Some(entry) = catalog
            .entries_for_management()
            .find(|entry| application_record_id(entry).eq_ignore_ascii_case(&id))
        else {
            continue;
        };
        let Some(icon) = effective_application_icon(
            entry,
            &settings,
            &mut settings_runtime.native_icons,
            &mut settings_runtime.custom_images,
        ) else {
            continue;
        };
        let _ = settings_runtime.scene.set_application_icon(&id, icon);
    }
}

fn rects_intersect(
    left: lotus_settings::scene::SettingsRect,
    right: lotus_settings::scene::SettingsRect,
) -> bool {
    left.left < right.left.saturating_add(right.width)
        && right.left < left.left.saturating_add(left.width)
        && left.top < right.top.saturating_add(right.height)
        && right.top < left.top.saturating_add(left.height)
}

fn application_record_id(entry: &ApplicationEntry) -> String {
    entry
        .app_user_model_id
        .as_deref()
        .filter(|identity| is_reliable_application_identity(identity))
        .unwrap_or(&entry.launch_target)
        .to_owned()
}

fn is_shared_host_executable(executable: &str) -> bool {
    ["chrome.exe", "msedge.exe", "applicationframehost.exe"]
        .iter()
        .any(|host| executable.eq_ignore_ascii_case(host))
}

fn effective_application_icon(
    entry: &ApplicationEntry,
    settings: &DockSettings,
    native_icons: &mut NativeIconCache,
    custom_images: &mut CustomImageCache,
) -> Option<lotus_ui::icon::RasterIcon> {
    let executable = lotus_windows::launch::resolve_executable(&entry.launch_target)
        .unwrap_or_else(|| Path::new(&entry.icon_source).to_path_buf());
    let executable_name = executable.file_name().and_then(|name| name.to_str());
    if let Some(override_) = settings.application_icon_override(
        entry.app_user_model_id.as_deref(),
        entry
            .app_user_model_id
            .as_deref()
            .or(Some(&entry.launch_target)),
        executable_name,
    ) && let Ok(icon) = custom_images.image(Path::new(&override_.image_path))
    {
        return Some(icon);
    }
    native_icons
        .icon(Path::new(&entry.icon_source), 96)
        .ok()
        .flatten()
}

fn preserve_externally_managed_settings(next: &mut DockSettings, current: &DockSettings) {
    next.notification_disabled_apps
        .clone_from(&current.notification_disabled_apps);
    next.application_name_overrides
        .clone_from(&current.application_name_overrides);
    next.hidden_executables
        .clone_from(&current.hidden_executables);
    next.item_order.clone_from(&current.item_order);
    next.pinned_apps.clone_from(&current.pinned_apps);
}

#[derive(Clone, Copy)]
enum SettingsColor {
    Background,
    Accent,
    Foreground,
}

fn choose_settings_color(
    context: &mut SettingsEventContext<'_>,
    target: SettingsColor,
) -> Result<(), AppError> {
    let owner = context.auxiliary.settings.window.handle();
    let initial = match target {
        SettingsColor::Background => {
            &context.auxiliary.settings.scene.draft().background_color
        }
        SettingsColor::Accent => &context.auxiliary.settings.scene.draft().accent_color,
        SettingsColor::Foreground => {
            &context.auxiliary.settings.scene.draft().foreground_color
        }
    };
    match lotus_windows::color_picker::choose_color(owner, initial) {
        Ok(Some(color)) => {
            match target {
                SettingsColor::Background => {
                    context.auxiliary.settings.scene.set_background_color(color);
                }
                SettingsColor::Accent => {
                    context.auxiliary.settings.scene.set_accent_color(color);
                }
                SettingsColor::Foreground => {
                    context.auxiliary.settings.scene.set_foreground_color(color);
                }
            }
            context.auxiliary.settings.render(context.graphics)
        }
        Ok(None) => Ok(()),
        Err(error) => {
            show_error(
                owner,
                "Lotus Settings",
                &format!("Lotus could not open the color picker.\n\n{error}"),
            );
            Ok(())
        }
    }
}

fn apply_auxiliary_settings(
    context: &mut SettingsEventContext<'_>,
) -> Result<(), AppError> {
    let settings = context.dock_model.settings();
    context
        .auxiliary
        .launcher
        .apply_settings(settings, context.dock, context.graphics)?;
    context.auxiliary.context_menu.apply_settings(settings);
    context.auxiliary.switcher.apply_settings(settings);
    Ok(())
}
