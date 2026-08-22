mod context_menu;
mod dock;
mod icon_override;
mod launcher;
mod media;
mod modules;
mod monitors;
mod runtime;
mod settings;
mod status;
mod switcher;
mod visuals;

use std::fs;
use std::path::PathBuf;

use dock::DockRuntime;
use lotus_core::search::SearchUsage;
use lotus_core::settings::{
    CURRENT_ONBOARDING_VERSION, DockSettings, NotificationBadgeStyle, SettingsDecodeError,
    SettingsLoadSource, SettingsStore, SettingsStoreError, decode_settings,
};
use lotus_search::usage::SearchUsageStore;
use lotus_ui::frame::ScheduledSurface;
use lotus_windows::activation::ActivationError;
use lotus_windows::appbar::ShellIntegration;
use lotus_windows::dpi::enable_per_monitor_v2;
use lotus_windows::graphics::{
    CompositionSurfaceState, DeviceState, SurfaceError, SurfaceSize,
};
use lotus_windows::single_instance::SingleInstance;
use lotus_windows::startup::{
    self as startup_registration, RestartWaitError, StartupArgsError, parse_startup_args,
    wait_for_restart_source,
};
use lotus_windows::taskbar_badges::TaskbarBadgeController;
use lotus_windows::window::DockWindow;
use lotus_windows::window_tracker::WindowTracker;
use modules::ModuleHost;
use runtime::{apply_fullscreen_visibility, flush_frame, resize_dock, run_message_loop};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Graphics(#[from] lotus_windows::graphics::GraphicsDeviceError),
    #[error("the newly created graphics device was unexpectedly unavailable")]
    GraphicsUnavailable,
    #[error(transparent)]
    Surface(#[from] SurfaceError),
    #[error("the dock cannot create a zero-sized composition surface")]
    ZeroSizedSurface,
    #[error("normalized dock settings could not produce a render scene")]
    InvalidScene,
    #[error("the application switcher could not produce a valid render scene")]
    InvalidSwitcherScene,
    #[error(transparent)]
    IconHydrator(#[from] lotus_windows::icon_hydrator::IconHydratorError),
    #[error("the native launcher could not produce a valid render scene")]
    InvalidLauncherScene,
    #[error("the native settings window could not produce a valid render scene")]
    InvalidSettingsScene,
    #[error("the native context menu could not produce a valid render scene")]
    InvalidContextMenuScene,
    #[error(transparent)]
    Native(#[from] lotus_windows::NativeError),
    #[error("GetMessageW failed")]
    MessageLoop,
    #[error("LOCALAPPDATA is unavailable; Lotus cannot locate its settings directory")]
    MissingLocalAppData,
    #[error(transparent)]
    SettingsDecode(#[from] SettingsDecodeError),
    #[error(transparent)]
    SettingsStore(#[from] SettingsStoreError),
    #[error(transparent)]
    StartupArgs(#[from] StartupArgsError),
    #[error(transparent)]
    RestartWait(#[from] RestartWaitError),
}

#[derive(Debug, Error)]
enum RestartError {
    #[error("Lotus could not locate its current executable: {0}")]
    CurrentExecutable(#[from] std::io::Error),
    #[error(transparent)]
    Launch(#[from] ActivationError),
}

struct RuntimePolicy<'a> {
    taskbar_badges: Option<&'a TaskbarBadgeController>,
    onboarding_required: bool,
}

pub fn run() -> Result<(), AppError> {
    enable_per_monitor_v2()?;
    let startup = parse_startup_args(std::env::args_os().skip(1))?;
    let _restart_wait = wait_for_restart_source(startup.restart_after)?;
    if let Some(directory) = startup.cleanup_update.as_deref() {
        let _ = lotus_windows::update::cleanup_staging_directory(directory);
    }
    let Some(_instance) = SingleInstance::acquire()? else {
        return Ok(());
    };

    let (settings, settings_store) = load_settings()?;
    let onboarding_required = settings.onboarding_version < CURRENT_ONBOARDING_VERSION;
    if !onboarding_required {
        sync_startup_preference(settings.start_with_windows);
    }
    let usage_store = SearchUsageStore::new(settings_store.directory());
    let usage = usage_store.load().unwrap_or_default();
    let mut graphics = DeviceState::create()?;
    let mut dock = DockWindow::create()?;
    lotus_windows::backdrop::apply_dock_settings(dock.handle(), &settings);
    dock.prepare(&settings)?;
    let (width, height) = dock.client_size()?;
    let surface_size = SurfaceSize::new(width, height).ok_or(AppError::ZeroSizedSurface)?;
    let graphics_device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
    let mut surface = ScheduledSurface::new(CompositionSurfaceState::create(
        graphics_device,
        dock.handle(),
        surface_size,
    )?);
    let mut window_tracker = WindowTracker::start()?;
    let mut dock_model = DockRuntime::new(
        settings,
        settings_store,
        window_tracker.current_windows(),
        dock.dpi(),
        dock.drag_threshold(),
    )?;
    let taskbar_badges = (!onboarding_required)
        .then(|| enable_notification_badges(&mut dock_model))
        .flatten();
    let mut auxiliary = create_auxiliary_windows(
        &dock,
        &dock_model,
        usage,
        usage_store,
        !onboarding_required,
    )?;
    resize_dock(&dock, &mut graphics, &mut surface, &dock_model)?;
    let _shell_integration = if onboarding_required {
        None
    } else {
        ShellIntegration::setup(dock_model.settings(), &dock).unwrap_or(None)
    };
    window_tracker.refresh_fullscreen();
    if !onboarding_required {
        auxiliary.sync_status(&dock, &dock_model, &mut graphics)?;
        auxiliary.sync_monitor_docks(
            &dock,
            &mut dock_model,
            &mut graphics,
            &window_tracker,
        )?;
    }
    if onboarding_required {
        let _changed = dock.set_visible(false);
        auxiliary.set_status_visible(false);
        auxiliary.open_onboarding(dock_model.settings(), true, &mut graphics)?;
    } else {
        apply_fullscreen_visibility(
            &dock,
            &mut surface,
            &window_tracker,
            &dock_model,
            &mut auxiliary,
        )?;
    }
    if startup.open_settings && !onboarding_required {
        auxiliary.open_settings_without_refresh(dock_model.settings(), &mut graphics)?;
    }
    flush_frame(
        &mut dock,
        &mut graphics,
        &mut surface,
        &mut dock_model,
        &mut auxiliary,
        lotus_ui::frame::FrameTrigger::Changes,
    )?;
    let runtime = RuntimePolicy {
        taskbar_badges: taskbar_badges.as_ref(),
        onboarding_required,
    };
    run_message_loop(
        &runtime,
        &mut dock,
        &mut graphics,
        &mut surface,
        &mut window_tracker,
        &mut dock_model,
        &mut auxiliary,
    )
}

fn create_auxiliary_windows(
    dock: &DockWindow,
    dock_model: &DockRuntime,
    usage: SearchUsage,
    usage_store: SearchUsageStore,
    modules_active: bool,
) -> Result<ModuleHost, AppError> {
    ModuleHost::create(dock, dock_model, usage, usage_store, modules_active)
}

fn sync_startup_preference(enabled: bool) {
    let _ = startup_registration::sync(enabled);
}

fn enable_notification_badges(model: &mut DockRuntime) -> Option<TaskbarBadgeController> {
    if model.settings().notification_badge_style == NotificationBadgeStyle::Off {
        return None;
    }
    let controller = TaskbarBadgeController::start().ok()?;
    if let Ok(snapshot) = controller.snapshot() {
        model.set_notifications(snapshot);
    }
    Some(controller)
}

fn load_settings() -> Result<(DockSettings, SettingsStore), AppError> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or(AppError::MissingLocalAppData)?;
    let settings_directory = local_app_data.join("Lotus");
    let _ = fs::remove_file(settings_directory.join("lotus.log"));
    let store = SettingsStore::new(settings_directory);

    let settings_existed = store.settings_path().exists();

    if !settings_existed {
        let shipped_defaults = decode_settings(include_str!(
            "../../lotus-core/assets/settings.default.json"
        ))?;
        store.save(&shipped_defaults)?;
    }

    let load = store.load()?;
    if let SettingsLoadSource::RecoveredInvalid { backup_path, error } = &load.source {
        lotus_windows::diagnostics::record_message(
            "settings.recovered_invalid",
            &format!(
                "Lotus restored default settings after `{error}`. The original file is at `{}`.",
                backup_path.display()
            ),
        );
    }

    Ok((load.settings, store))
}
