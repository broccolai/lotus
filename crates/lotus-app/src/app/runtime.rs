use lotus_settings::scene::SettingsPointerStyle;
use lotus_windows::interaction::{NativeMessage, PointerCursor};

use super::{
    AppError, AppMenuAction, AuxiliaryWindows, AuxiliaryZoneAction, CommandId,
    CompositionSurfaceState, ContextMenuAction, ContextMenuEvent, DeviceState,
    DockContextRequest, DockHitTarget, DockRuntime, DockWindow, LauncherSubmission,
    MonitorDockAction, PointerEvent, PopupAction, PowerAction, RuntimePolicy,
    SceneSettingsKey, SelectionDirection, SettingsAction, SettingsEvent, SettingsRuntime,
    SettingsUpdateActivity, SurfaceSize, SystemStatusKind, UpdateResult, UpdateStatus,
    WindowEvent, WindowHandle, WindowSettingsKey, WindowTracker, WindowTrackerEvent,
    apply_fullscreen_visibility, confirm_install_update, confirm_restart, confirm_shutdown,
    fullscreen_notification, handle_alt_tab_events, handle_pointer_event,
    handle_search_event, handle_windows_key_events, is_alt_tab_wake, is_installed,
    is_media_wake, is_search_catalog_wake, is_taskbar_badge_wake, is_update_wake,
    is_windows_key_wake, launch_installer, launch_target, next_message,
    render_and_schedule, render_surface, request_exit, request_window_close, resize_dock,
    resize_surface, restart_current_process, show_error, show_information,
    startup_registration, write_text,
};

pub(super) fn run_message_loop(
    runtime: &RuntimePolicy<'_>,
    dock: &mut DockWindow,
    graphics: &mut DeviceState,
    surface: &mut CompositionSurfaceState,
    window_tracker: &mut WindowTracker,
    dock_model: &mut DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    loop {
        let Some(message) = next_message().map_err(|_error| AppError::MessageLoop)? else {
            return Ok(());
        };
        process_message(
            &message,
            runtime,
            dock,
            graphics,
            surface,
            window_tracker,
            dock_model,
            auxiliary,
        )?;
    }
}

#[allow(clippy::too_many_arguments)]
fn process_message(
    message: &NativeMessage,
    runtime: &RuntimePolicy<'_>,
    dock: &mut DockWindow,
    graphics: &mut DeviceState,
    surface: &mut CompositionSurfaceState,
    window_tracker: &mut WindowTracker,
    dock_model: &mut DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    let shell_fullscreen = fullscreen_notification(
        message.is_thread_message(),
        message.id(),
        message.parameter(),
    );
    if let Some(fullscreen) = shell_fullscreen {
        window_tracker.set_shell_fullscreen(fullscreen);
    }
    handle_tracker_message(
        message,
        runtime,
        dock,
        graphics,
        surface,
        window_tracker,
        dock_model,
        auxiliary,
    )?;
    if shell_fullscreen.is_some() && !runtime.onboarding_required {
        apply_fullscreen_visibility(
            dock,
            window_tracker,
            dock_model,
            &mut auxiliary.launcher,
            &auxiliary.status,
        )?;
    }
    let windows_key_wake = runtime
        .windows_key
        .is_some_and(|_| is_windows_key_wake(message.id()));
    let alt_tab_wake = runtime
        .alt_tab
        .is_some_and(|_| is_alt_tab_wake(message.id()));
    let search_catalog_wake = is_search_catalog_wake(message.id());
    let update_wake = is_update_wake(message.id());
    let media_wake = is_media_wake(message.id());
    let badge_wake =
        runtime.taskbar_badges.is_some() && is_taskbar_badge_wake(message.id());
    message.dispatch();

    drain_window_events(
        dock,
        graphics,
        surface,
        window_tracker.current_windows(),
        dock_model,
        auxiliary,
    )?;
    drain_settings_and_switcher_events(&mut SettingsEventContext {
        dock,
        graphics,
        dock_surface: surface,
        window_tracker,
        dock_model,
        auxiliary,
    })?;
    for action in auxiliary.monitors.drain_events(graphics)? {
        handle_monitor_dock_action(action, dock, graphics, dock_model, auxiliary)?;
    }

    if update_wake {
        handle_update_results(&mut auxiliary.settings, graphics)?;
    }
    if badge_wake
        && let Some(controller) = runtime.taskbar_badges
        && let Ok(snapshot) = controller.snapshot()
    {
        dock_model.set_notifications(snapshot);
        render_and_schedule(
            dock,
            graphics,
            surface,
            dock_model.scene(),
            auxiliary.launcher.needs_animation(),
        )?;
    }
    if media_wake && auxiliary.media.drain(dock_model) {
        resize_dock(dock, graphics, surface, dock_model)?;
        auxiliary
            .status
            .sync(dock, dock_model.settings(), dock_model.media(), graphics)?;
        render_and_schedule(
            dock,
            graphics,
            surface,
            dock_model.scene(),
            auxiliary.launcher.needs_animation(),
        )?;
    }
    if windows_key_wake && let Some(controller) = runtime.windows_key {
        handle_windows_key_events(
            controller,
            dock,
            graphics,
            dock_model,
            &mut auxiliary.launcher,
        )?;
    }
    if alt_tab_wake && let Some(controller) = runtime.alt_tab {
        handle_alt_tab_events(
            controller,
            window_tracker,
            dock_model,
            graphics,
            &mut auxiliary.switcher,
        )?;
    }
    if search_catalog_wake
        && auxiliary
            .launcher
            .refresh_catalog_if_ready(dock, dock_model, graphics)?
    {
        let dock_animation = render_surface(graphics, surface, dock_model.scene())?;
        let launcher_animation = auxiliary.launcher.render(graphics)?;
        dock.set_animation_active(dock_animation || launcher_animation)?;
    }
    auxiliary
        .monitors
        .sync(dock, dock_model, graphics, window_tracker)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn drain_window_events(
    dock: &mut DockWindow,
    graphics: &mut DeviceState,
    surface: &mut CompositionSurfaceState,
    windows: &[lotus_core::window::WindowInfo],
    dock_model: &mut DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    let events = dock.drain_events().collect::<Vec<_>>();
    for event in events {
        handle_window_event(event, dock, graphics, surface, dock_model, auxiliary)?;
    }
    for event in auxiliary.launcher.drain_events() {
        if let Some(submission) = handle_search_event(
            event,
            dock,
            graphics,
            surface,
            dock_model,
            &mut auxiliary.launcher,
        )? {
            execute_search_submission(submission, dock, graphics, dock_model, auxiliary)?;
        }
    }
    for event in auxiliary.context_menu.drain_events() {
        handle_context_menu_event(
            event, dock, graphics, surface, windows, dock_model, auxiliary,
        )?;
    }
    for (zone, event) in auxiliary.status.drain_events() {
        if let Some((action, owner, anchor)) =
            auxiliary.status.handle_event(zone, event, graphics)?
        {
            match action {
                AuxiliaryZoneAction::Media(target) => {
                    auxiliary.media.activate(target, dock_model, owner);
                }
                AuxiliaryZoneAction::Status(kind) => {
                    activate_system_status(kind, owner, anchor);
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_tracker_message(
    message: &NativeMessage,
    runtime: &RuntimePolicy<'_>,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut CompositionSurfaceState,
    window_tracker: &mut WindowTracker,
    dock_model: &mut DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    let Some(event) = window_tracker.handle_message(
        message.is_thread_message(),
        message.id(),
        message.parameter(),
    )?
    else {
        return Ok(());
    };
    if event == WindowTrackerEvent::SnapshotRefreshed {
        dock_model.rebuild(window_tracker.current_windows());
        dock_model.record_foreground(lotus_windows::activation::foreground_window());
        reconcile_visible_picker(dock_model, &mut auxiliary.context_menu, graphics)?;
        resize_dock(dock, graphics, surface, dock_model)?;
        auxiliary
            .status
            .sync(dock, dock_model.settings(), dock_model.media(), graphics)?;
        render_and_schedule(
            dock,
            graphics,
            surface,
            dock_model.scene(),
            auxiliary.launcher.needs_animation(),
        )?;
    }
    if runtime.onboarding_required {
        Ok(())
    } else {
        apply_fullscreen_visibility(
            dock,
            window_tracker,
            dock_model,
            &mut auxiliary.launcher,
            &auxiliary.status,
        )
    }
}

fn reconcile_visible_picker(
    dock_model: &mut DockRuntime,
    popup: &mut super::ContextMenuRuntime,
    graphics: &mut DeviceState,
) -> Result<(), AppError> {
    let Some(identity) = popup.picker_identity().map(str::to_owned) else {
        return Ok(());
    };
    let Some(source_index) = dock_model.source_index(&identity) else {
        popup.hide();
        return Ok(());
    };
    let windows = dock_model
        .picker_windows(source_index, lotus_windows::activation::foreground_window());
    let style = dock_model.settings().window_picker_style;
    popup.replace_picker(source_index, style, windows, graphics)
}

fn execute_search_submission(
    submission: LauncherSubmission,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    dock_model: &DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    match submission {
        LauncherSubmission::Command(command) => {
            execute_search_command(command, dock, graphics, dock_model, auxiliary)
        }
        LauncherSubmission::Calculation(value) => {
            if let Err(error) = write_text(&value) {
                show_error(
                    dock.handle(),
                    "Lotus Calculator",
                    &format!("Lotus could not copy the result.\n\n{error}"),
                );
            }
            Ok(())
        }
    }
}

fn execute_search_command(
    command: CommandId,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    dock_model: &DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    match command {
        CommandId::OpenSettings => {
            auxiliary.settings.open(dock_model.settings(), graphics)?;
        }
        CommandId::OpenVolumeMixer => {
            if let Err(error) = launch_target("sndvol.exe", None) {
                show_error(
                    dock.handle(),
                    "Lotus",
                    &format!("Lotus could not open the Windows volume mixer.\n\n{error}"),
                );
            }
        }
        CommandId::OpenNotificationArea => {
            if let Err(error) = lotus_windows::tray::open_overflow(dock.handle()) {
                show_error(
                    dock.handle(),
                    "Lotus",
                    &format!(
                        "Lotus could not open the Windows notification area.\n\n{error}"
                    ),
                );
            }
        }
        CommandId::ShowDesktop => {
            if let Err(error) = lotus_windows::desktop::toggle() {
                show_error(
                    dock.handle(),
                    "Lotus",
                    &format!("Lotus could not show the desktop.\n\n{error}"),
                );
            }
        }
        CommandId::LockComputer => {
            if let Err(error) = lotus_windows::desktop::lock() {
                show_error(
                    dock.handle(),
                    "Lotus",
                    &format!("Lotus could not lock Windows.\n\n{error}"),
                );
            }
        }
        CommandId::RestartComputer => {
            if confirm_restart(dock.handle())
                && let Err(error) = launch_target("shutdown.exe", Some("/r /t 0"))
            {
                show_error(
                    dock.handle(),
                    "Lotus",
                    &format!("Lotus could not restart Windows.\n\n{error}"),
                );
            }
        }
        CommandId::ShutDownComputer => {
            if confirm_shutdown(dock.handle())
                && let Err(error) = launch_target("shutdown.exe", Some("/s /t 0"))
            {
                show_error(
                    dock.handle(),
                    "Lotus",
                    &format!("Lotus could not shut down Windows.\n\n{error}"),
                );
            }
        }
        CommandId::QuitLotus => request_exit(0),
    }
    Ok(())
}

fn drain_settings_and_switcher_events(
    context: &mut SettingsEventContext<'_>,
) -> Result<(), AppError> {
    let settings_events = context.auxiliary.settings.drain_events();
    for event in settings_events {
        handle_settings_event(event, context)?;
    }
    let switcher_events = context.auxiliary.switcher.drain_events();
    for event in switcher_events {
        context
            .auxiliary
            .switcher
            .handle_window_event(event, context.graphics)?;
    }
    Ok(())
}

fn handle_context_menu_event(
    event: ContextMenuEvent,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut CompositionSurfaceState,
    windows: &[lotus_core::window::WindowInfo],
    dock_model: &mut DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    match event {
        ContextMenuEvent::PointerMoved { x, y } => {
            if auxiliary.context_menu.scene.pointer_move(x, y) {
                auxiliary.context_menu.render(graphics)?;
            }
        }
        ContextMenuEvent::PointerLeft => {
            if auxiliary.context_menu.scene.pointer_left() {
                auxiliary.context_menu.render(graphics)?;
            }
        }
        ContextMenuEvent::PointerReleased { x, y } => {
            let action = auxiliary.context_menu.scene.pointer_action(x, y);
            let source_index = auxiliary.context_menu.scene.source_index();
            if !action.is_some_and(opens_power_menu) {
                auxiliary.context_menu.hide();
            }
            if let Some(action) = action {
                execute_popup_action(
                    action,
                    source_index,
                    dock,
                    graphics,
                    surface,
                    windows,
                    dock_model,
                    auxiliary,
                )?;
            }
        }
        ContextMenuEvent::SelectionRequested => {
            let action = auxiliary.context_menu.scene.selected_action();
            let source_index = auxiliary.context_menu.scene.source_index();
            if !action.is_some_and(opens_power_menu) {
                auxiliary.context_menu.hide();
            }
            if let Some(action) = action {
                execute_popup_action(
                    action,
                    source_index,
                    dock,
                    graphics,
                    surface,
                    windows,
                    dock_model,
                    auxiliary,
                )?;
            }
        }
        ContextMenuEvent::MoveSelection(direction) => {
            let next = direction == SelectionDirection::Next;
            if auxiliary.context_menu.scene.move_selection(next) {
                auxiliary.context_menu.render(graphics)?;
            }
        }
        ContextMenuEvent::Scroll(direction) => {
            let next = direction == SelectionDirection::Next;
            if auxiliary.context_menu.scene.scroll(next) {
                auxiliary.context_menu.render(graphics)?;
            }
        }
        ContextMenuEvent::DismissRequested => auxiliary.context_menu.hide(),
        ContextMenuEvent::Resized { width, height } => {
            auxiliary.context_menu.resize(width, height)?;
            auxiliary.context_menu.render(graphics)?;
        }
        ContextMenuEvent::DpiChanged { dpi } => {
            if auxiliary.context_menu.scene.set_dpi(dpi) {
                let desired = auxiliary.context_menu.scene.desired_size();
                if let Some(surface) = &mut auxiliary.context_menu.surface {
                    surface.resize(desired)?;
                }
            }
            auxiliary.context_menu.render(graphics)?;
        }
        ContextMenuEvent::RenderRequested => auxiliary.context_menu.render(graphics)?,
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn execute_popup_action(
    action: PopupAction,
    source_index: Option<usize>,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut CompositionSurfaceState,
    windows: &[lotus_core::window::WindowInfo],
    dock_model: &mut DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    match action {
        PopupAction::System(action) => {
            execute_context_menu_action(action, graphics, dock_model, auxiliary)?;
        }
        PopupAction::Power(action) => execute_power_action(action, dock.handle()),
        PopupAction::App(action) => {
            let Some(source_index) = source_index else {
                return Ok(());
            };
            execute_app_menu_action(
                action,
                source_index,
                dock,
                graphics,
                surface,
                windows,
                dock_model,
                auxiliary,
            )?;
        }
        PopupAction::Activate(window) => {
            if let Some(source_index) = source_index {
                dock_model.record_window_activation(source_index, window);
            }
            if let Err(error) = lotus_windows::activation::switch_window(window) {
                show_error(
                    dock.handle(),
                    "Lotus",
                    &format!("Lotus could not activate that window.\n\n{error}"),
                );
            }
        }
        PopupAction::CloseWindow(window) => {
            if let Err(error) = request_window_close(window) {
                show_error(
                    dock.handle(),
                    "Lotus",
                    &format!("Lotus could not close that window.\n\n{error}"),
                );
            }
        }
    }
    Ok(())
}

const fn opens_power_menu(action: PopupAction) -> bool {
    matches!(
        action,
        PopupAction::System(ContextMenuAction::RequestShutdown)
    )
}

fn execute_power_action(action: PowerAction, owner: WindowHandle) {
    let result = match action {
        PowerAction::Lock => {
            lotus_windows::desktop::lock().map_err(|error| error.to_string())
        }
        PowerAction::Restart => launch_target("shutdown.exe", Some("/r /t 0"))
            .map_err(|error| error.to_string()),
        PowerAction::ShutDown => launch_target("shutdown.exe", Some("/s /t 0"))
            .map_err(|error| error.to_string()),
        PowerAction::Cancel => return,
    };
    if let Err(error) = result {
        show_error(
            owner,
            "Lotus",
            &format!("Lotus could not complete that power action.\n\n{error}"),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_app_menu_action(
    action: AppMenuAction,
    source_index: usize,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut CompositionSurfaceState,
    windows: &[lotus_core::window::WindowInfo],
    dock_model: &mut DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    match action {
        AppMenuAction::Open => dock_model.open_new(source_index, dock.handle()),
        AppMenuAction::TogglePin => {
            let pinned = dock_model
                .item(source_index)
                .is_some_and(|item| item.is_pinned);
            let changed = match dock_model.set_pinned(source_index, !pinned, windows) {
                Ok(changed) => changed,
                Err(error) => {
                    show_error(
                        dock.handle(),
                        "Lotus",
                        &format!("Lotus could not save that pin.\n\n{error}"),
                    );
                    false
                }
            };
            if changed {
                resize_dock(dock, graphics, surface, dock_model)?;
                auxiliary.status.sync(
                    dock,
                    dock_model.settings(),
                    dock_model.media(),
                    graphics,
                )?;
                render_and_schedule(
                    dock,
                    graphics,
                    surface,
                    dock_model.scene(),
                    auxiliary.launcher.needs_animation(),
                )?;
            }
        }
        AppMenuAction::Close => {
            let window_ids = dock_model
                .item(source_index)
                .map(|item| {
                    item.windows
                        .iter()
                        .map(|window| window.id)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for window in window_ids {
                if let Err(error) = request_window_close(window) {
                    show_error(
                        dock.handle(),
                        "Lotus",
                        &format!("Lotus could not close that window.\n\n{error}"),
                    );
                    break;
                }
            }
        }
    }
    Ok(())
}

fn execute_context_menu_action(
    action: ContextMenuAction,
    graphics: &mut DeviceState,
    dock_model: &DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    match action {
        ContextMenuAction::OpenSettings => {
            auxiliary.settings.open(dock_model.settings(), graphics)?;
        }
        ContextMenuAction::RequestShutdown => {
            auxiliary.context_menu.open_power(graphics)?;
        }
        ContextMenuAction::QuitLotus => request_exit(0),
    }
    Ok(())
}

struct SettingsEventContext<'a> {
    dock: &'a DockWindow,
    graphics: &'a mut DeviceState,
    dock_surface: &'a mut CompositionSurfaceState,
    window_tracker: &'a WindowTracker,
    dock_model: &'a mut DockRuntime,
    auxiliary: &'a mut AuxiliaryWindows,
}

fn handle_settings_event(
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
                context.auxiliary.settings.render(context.graphics)?;
            }
            return Ok(());
        }
        SettingsEvent::CloseRequested => SettingsAction::Close,
        SettingsEvent::KeyPressed(key) => {
            settings_key_action(&mut context.auxiliary.settings, key)
        }
    };

    apply_settings_action(action, context)
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
        WindowSettingsKey::Save => return SettingsAction::None,
    };
    runtime.scene.key(key)
}

fn apply_settings_action(
    action: SettingsAction,
    context: &mut SettingsEventContext<'_>,
) -> Result<(), AppError> {
    match action {
        SettingsAction::None => Ok(()),
        SettingsAction::Changed => context.auxiliary.settings.render(context.graphics),
        SettingsAction::ChooseBackgroundColor => choose_settings_color(context, true),
        SettingsAction::ChooseAccentColor => choose_settings_color(context, false),
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
        SettingsAction::CheckForUpdates => start_update_check(context),
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
    next: lotus_core::settings::DockSettings,
    context: &mut SettingsEventContext<'_>,
    restart_after_apply: bool,
) -> Result<(), AppError> {
    let start_with_windows = next.start_with_windows;
    let impact = context
        .dock_model
        .apply_settings(next, context.window_tracker.current_windows())?;
    if restart_after_apply {
        context
            .auxiliary
            .settings
            .scene
            .mark_applied(context.dock_model.settings().clone());
        context.auxiliary.settings.hide();
        if let Err(error) = restart_current_process() {
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
    if !context.dock_model.settings().show_media_controls {
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
    context.auxiliary.settings.render(context.graphics)?;

    if impact.restart_required {
        if let Err(error) = restart_current_process() {
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

fn choose_settings_color(
    context: &mut SettingsEventContext<'_>,
    background: bool,
) -> Result<(), AppError> {
    let owner = context.auxiliary.settings.window.handle();
    let initial = if background {
        &context.auxiliary.settings.scene.draft().background_color
    } else {
        &context.auxiliary.settings.scene.draft().accent_color
    };
    match lotus_windows::color_picker::choose_color(owner, initial) {
        Ok(Some(color)) => {
            if background {
                context.auxiliary.settings.scene.set_background_color(color);
            } else {
                context.auxiliary.settings.scene.set_accent_color(color);
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

fn start_update_check(context: &mut SettingsEventContext<'_>) -> Result<(), AppError> {
    let owner = context.auxiliary.settings.window.handle();
    match context.auxiliary.settings.start_update_check() {
        Ok(true) => context.auxiliary.settings.render(context.graphics),
        Ok(false) => Ok(()),
        Err(error) => {
            show_error(owner, "Lotus Update", &error.to_string());
            Ok(())
        }
    }
}

fn handle_update_results(
    settings: &mut SettingsRuntime,
    graphics: &mut DeviceState,
) -> Result<(), AppError> {
    let results = settings.drain_update_results();
    for result in results {
        match result {
            UpdateResult::Checked(result) => {
                handle_update_check(result, settings, graphics)?;
            }
            UpdateResult::Staged(result) => {
                handle_staged_update(result, settings, graphics)?;
            }
        }
    }
    Ok(())
}

fn handle_update_check(
    result: Result<UpdateStatus, lotus_windows::update::UpdateError>,
    settings: &mut SettingsRuntime,
    graphics: &mut DeviceState,
) -> Result<(), AppError> {
    let owner = settings.window.handle();
    let installed = match is_installed() {
        Ok(installed) => installed,
        Err(error) => {
            let _ = settings
                .scene
                .set_update_activity(SettingsUpdateActivity::Idle);
            settings.render(graphics)?;
            show_error(owner, "Lotus Update", &error.to_string());
            return Ok(());
        }
    };
    match result {
        Ok(UpdateStatus::Current { release }) if installed => {
            let _ = settings
                .scene
                .set_update_activity(SettingsUpdateActivity::Idle);
            settings.render(graphics)?;
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
            if confirm_install_update(owner, &release.version, false) {
                match settings.start_update_download(release) {
                    Ok(true) => settings.render(graphics)?,
                    Ok(false) => {}
                    Err(error) => {
                        let _ = settings
                            .scene
                            .set_update_activity(SettingsUpdateActivity::Idle);
                        settings.render(graphics)?;
                        show_error(owner, "Lotus Update", &error.to_string());
                    }
                }
            } else {
                let _ = settings
                    .scene
                    .set_update_activity(SettingsUpdateActivity::Idle);
                settings.render(graphics)?;
            }
        }
        Ok(UpdateStatus::Ahead { current, release }) => {
            let _ = settings
                .scene
                .set_update_activity(SettingsUpdateActivity::Idle);
            settings.render(graphics)?;
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
            if confirm_install_update(owner, &release.version, installed) {
                match settings.start_update_download(release) {
                    Ok(true) => settings.render(graphics)?,
                    Ok(false) => {}
                    Err(error) => {
                        let _ = settings
                            .scene
                            .set_update_activity(SettingsUpdateActivity::Idle);
                        settings.render(graphics)?;
                        show_error(owner, "Lotus Update", &error.to_string());
                    }
                }
            } else {
                let _ = settings
                    .scene
                    .set_update_activity(SettingsUpdateActivity::Idle);
                settings.render(graphics)?;
            }
        }
        Err(error) => {
            let _ = settings
                .scene
                .set_update_activity(SettingsUpdateActivity::Idle);
            settings.render(graphics)?;
            show_error(
                owner,
                "Lotus Update",
                &format!("Lotus could not check for updates.\n\n{error}"),
            );
        }
    }
    Ok(())
}

fn handle_staged_update(
    result: Result<lotus_windows::update::StagedUpdate, lotus_windows::update::UpdateError>,
    settings: &mut SettingsRuntime,
    graphics: &mut DeviceState,
) -> Result<(), AppError> {
    let owner = settings.window.handle();
    match result {
        Ok(staged) => match launch_installer(&staged) {
            Ok(()) => request_exit(0),
            Err(error) => {
                let _ = settings
                    .scene
                    .set_update_activity(SettingsUpdateActivity::Idle);
                settings.render(graphics)?;
                show_error(owner, "Lotus Update", &error.to_string());
            }
        },
        Err(error) => {
            let _ = settings
                .scene
                .set_update_activity(SettingsUpdateActivity::Idle);
            settings.render(graphics)?;
            show_error(
                owner,
                "Lotus Update",
                &format!("Lotus could not prepare the update.\n\n{error}"),
            );
        }
    }
    Ok(())
}

fn handle_window_event(
    event: WindowEvent,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut CompositionSurfaceState,
    dock_model: &mut DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    match event {
        WindowEvent::Resized { width, height } => {
            if let Some(size) = SurfaceSize::new(width, height) {
                resize_surface(graphics, surface, size)?;
                render_and_schedule(
                    dock,
                    graphics,
                    surface,
                    dock_model.scene(),
                    auxiliary.launcher.needs_animation(),
                )?;
            }
        }
        WindowEvent::DpiChanged { dpi } => {
            dock_model.set_dpi(dpi)?;
            dock_model.set_drag_threshold(dock.drag_threshold());
            resize_dock(dock, graphics, surface, dock_model)?;
            auxiliary.status.sync(
                dock,
                dock_model.settings(),
                dock_model.media(),
                graphics,
            )?;
            render_and_schedule(
                dock,
                graphics,
                surface,
                dock_model.scene(),
                auxiliary.launcher.needs_animation(),
            )?;
        }
        WindowEvent::PlacementRefreshRequested => {
            dock.refresh_placement(dock_model.settings())?;
            auxiliary.monitors.mark_topology_dirty();
            auxiliary.status.sync(
                dock,
                dock_model.settings(),
                dock_model.media(),
                graphics,
            )?;
            if auxiliary.launcher.is_visible() {
                auxiliary.launcher.sync_size(dock, graphics)?;
            }
        }
        WindowEvent::Pointer(event) => {
            handle_dock_pointer(event, dock, graphics, surface, dock_model, auxiliary)?;
        }
        WindowEvent::ContextMenuRequested(request) => {
            handle_context_menu(request, dock, graphics, surface, dock_model, auxiliary)?;
        }
        WindowEvent::Search(_)
        | WindowEvent::Settings(_)
        | WindowEvent::ContextMenu(_)
        | WindowEvent::Switcher(_) => {}
        WindowEvent::AnimationFrame => {
            auxiliary.launcher.advance_animation();
            if dock_model.advance_departure(Instant::now()) {
                resize_dock(dock, graphics, surface, dock_model)?;
            }
            let dock_animation = render_surface(graphics, surface, dock_model.scene())?;
            let launcher_animation = auxiliary.launcher.render(graphics)?;
            dock.set_animation_active(dock_animation || launcher_animation)?;
        }
        WindowEvent::StatusRefreshRequested => {
            if dock_model.refresh_status() {
                render_and_schedule(
                    dock,
                    graphics,
                    surface,
                    dock_model.scene(),
                    auxiliary.launcher.needs_animation(),
                )?;
            }
            auxiliary.status.refresh(dock_model.settings(), graphics)?;
        }
        WindowEvent::RenderRequested => {
            render_and_schedule(
                dock,
                graphics,
                surface,
                dock_model.scene(),
                auxiliary.launcher.needs_animation(),
            )?;
        }
    }
    Ok(())
}

fn handle_dock_pointer(
    event: PointerEvent,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut CompositionSurfaceState,
    dock_model: &mut DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    if matches!(event, PointerEvent::LeftButtonPressed { .. }) {
        auxiliary.context_menu.hide();
    }
    let release_request = match event {
        PointerEvent::LeftButtonReleased { x, y } => {
            let client = super::SignedPoint { x, y };
            dock.client_to_screen(client)
                .ok()
                .map(|screen| DockContextRequest::Pointer { screen, client })
        }
        PointerEvent::Moved { .. }
        | PointerEvent::Left
        | PointerEvent::LeftButtonPressed { .. }
        | PointerEvent::Cancelled => None,
    };
    let (changed, activation) = handle_pointer_event(event, dock_model)?;
    if changed {
        render_and_schedule(
            dock,
            graphics,
            surface,
            dock_model.scene(),
            auxiliary.launcher.needs_animation(),
        )?;
    }
    let Some(target) = activation else {
        return Ok(());
    };
    let activation_anchor = release_request
        .and_then(|request| dock_model.popup_target_anchor(request))
        .map(|(_, anchor, _)| anchor);
    match target {
        DockHitTarget::Item(source_index) => {
            let anchor = release_request
                .and_then(|request| dock_model.popup_target_anchor(request))
                .map(|(_, anchor, _)| anchor);
            activate_dock_item(
                source_index,
                target,
                anchor,
                dock.handle(),
                graphics,
                dock_model,
                auxiliary,
            )?;
        }
        DockHitTarget::Jirachi => {
            if dock_model.settings().search_enabled {
                let needs_animation =
                    auxiliary.launcher.toggle(dock, dock_model, graphics)?;
                dock.set_animation_active(needs_animation)?;
            }
        }
        DockHitTarget::Media(target) => {
            auxiliary.launcher.hide();
            auxiliary.media.activate(target, dock_model, dock.handle());
        }
        DockHitTarget::SystemStatus(kind) => {
            auxiliary.launcher.hide();
            activate_system_status(kind, dock.handle(), activation_anchor);
        }
        DockHitTarget::ShowDesktop => {
            auxiliary.launcher.hide();
            if let Err(error) = lotus_windows::desktop::toggle() {
                show_error(
                    dock.handle(),
                    "Lotus",
                    &format!("Lotus could not show the desktop.\n\n{error}"),
                );
            }
        }
    }
    Ok(())
}

fn activate_dock_item(
    source_index: usize,
    target: DockHitTarget,
    anchor: Option<super::SignedPoint>,
    owner: WindowHandle,
    graphics: &mut DeviceState,
    dock_model: &mut DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    auxiliary.launcher.hide();
    let window_count = dock_model
        .item(source_index)
        .map_or(0, |item| item.windows.len());
    if window_count <= 1 {
        dock_model.activate(target, owner);
        return Ok(());
    }
    let Some(anchor) = anchor else {
        dock_model.activate(target, owner);
        return Ok(());
    };

    let entries = dock_model
        .picker_windows(source_index, lotus_windows::activation::foreground_window());
    let identity = dock_model
        .item(source_index)
        .map(|item| item.id.clone())
        .unwrap_or_default();
    let style = dock_model.settings().window_picker_style;
    auxiliary.context_menu.open_picker(
        anchor,
        source_index,
        identity,
        style,
        entries,
        graphics,
    )
}

fn activate_system_status(
    kind: SystemStatusKind,
    owner: WindowHandle,
    anchor: Option<super::SignedPoint>,
) {
    let result = match kind {
        SystemStatusKind::Volume => native_panel_or_fallback(
            anchor.map_or_else(
                || lotus_windows::tray::open_quick_settings(owner),
                |point| lotus_windows::tray::open_quick_settings_at(owner, point.x),
            ),
            "sndvol.exe",
        ),
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

    if let Err(error) = result {
        show_error(
            owner,
            "Lotus",
            &format!("Lotus could not open that system control.\n\n{error}"),
        );
    }
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

fn handle_context_menu(
    request: DockContextRequest,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut CompositionSurfaceState,
    dock_model: &mut DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    let Some((target, anchor, alignment)) = dock_model.popup_target_anchor(request) else {
        return Ok(());
    };
    auxiliary.launcher.hide();
    if dock_model.pointer_cancelled() {
        render_and_schedule(dock, graphics, surface, dock_model.scene(), false)?;
    }
    open_context_target(target, anchor, alignment, graphics, dock_model, auxiliary)
}

fn open_context_target(
    target: DockHitTarget,
    anchor: super::SignedPoint,
    alignment: super::PopupAlignment,
    graphics: &mut DeviceState,
    dock_model: &DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    match target {
        DockHitTarget::Jirachi => {
            auxiliary.context_menu.open(anchor, alignment, graphics)?;
        }
        DockHitTarget::Item(source_index) => {
            let Some(item) = dock_model.item(source_index) else {
                return Ok(());
            };
            auxiliary.context_menu.open_app(
                anchor,
                source_index,
                item.windows.len(),
                item.is_pinned,
                graphics,
            )?;
        }
        DockHitTarget::Media(_)
        | DockHitTarget::SystemStatus(_)
        | DockHitTarget::ShowDesktop => {}
    }
    Ok(())
}

fn handle_monitor_dock_action(
    action: MonitorDockAction,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    dock_model: &mut DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    match action {
        MonitorDockAction::Activate {
            target,
            owner,
            anchor,
        } => match target {
            DockHitTarget::Item(source_index) => activate_dock_item(
                source_index,
                target,
                anchor,
                owner,
                graphics,
                dock_model,
                auxiliary,
            )?,
            DockHitTarget::Jirachi => {
                if dock_model.settings().search_enabled {
                    let needs_animation =
                        auxiliary.launcher.toggle(dock, dock_model, graphics)?;
                    dock.set_animation_active(needs_animation)?;
                }
            }
            DockHitTarget::Media(target) => {
                auxiliary.launcher.hide();
                auxiliary.media.activate(target, dock_model, owner);
            }
            DockHitTarget::SystemStatus(kind) => {
                auxiliary.launcher.hide();
                activate_system_status(kind, owner, anchor);
            }
            DockHitTarget::ShowDesktop => {
                auxiliary.launcher.hide();
                if let Err(error) = lotus_windows::desktop::toggle() {
                    show_error(
                        owner,
                        "Lotus",
                        &format!("Lotus could not show the desktop.\n\n{error}"),
                    );
                }
            }
        },
        MonitorDockAction::Context {
            target,
            anchor,
            alignment,
        } => {
            auxiliary.launcher.hide();
            open_context_target(
                target, anchor, alignment, graphics, dock_model, auxiliary,
            )?;
        }
    }
    Ok(())
}
use std::time::Instant;
