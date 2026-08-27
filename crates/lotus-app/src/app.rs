mod activation;
mod context_menu;
mod dock;
mod icon_override;
mod integration;
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
use std::time::{Duration, Instant};

use dock::DockRuntime;
use lotus_core::search::SearchUsage;
use lotus_core::settings::{
    CURRENT_ONBOARDING_VERSION, DockSettings, NotificationBadgeStyle, SettingsDecodeError,
    SettingsLoadSource, SettingsStore, SettingsStoreError, decode_settings,
};
use lotus_search::usage::SearchUsageStore;
use lotus_ui::frame::ScheduledSurface;
use lotus_windows::activation::ActivationError;
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

impl AppError {
    pub(super) fn mark_graphics_lost(&self, graphics: &mut DeviceState) -> bool {
        match self {
            Self::Surface(SurfaceError::DeviceLost(loss)) => {
                graphics.mark_lost(*loss);
                true
            }
            _ => false,
        }
    }
}

#[derive(Debug, Error)]
enum RestartError {
    #[error("Lotus could not locate its current executable: {0}")]
    CurrentExecutable(#[from] std::io::Error),
    #[error(transparent)]
    Launch(#[from] ActivationError),
}

struct RuntimeServices<'a> {
    taskbar_badges: Option<&'a TaskbarBadgeController>,
    onboarding_required: bool,
    integration: &'a mut integration::IntegrationRecovery,
}

struct PreparedSettings {
    settings: DockSettings,
    store: SettingsStore,
    onboarding_required: bool,
}

struct InitialWindows<'a> {
    dock: &'a DockWindow,
    dock_model: &'a mut DockRuntime,
    graphics: &'a mut DeviceState,
    surface: &'a mut ScheduledSurface<CompositionSurfaceState>,
    window_tracker: &'a WindowTracker,
    auxiliary: &'a mut ModuleHost,
}

struct StartupPhases {
    started: Instant,
    checkpoint: Instant,
    settings: Duration,
    graphics_window: Duration,
    initial_window_tracking: Duration,
    dock_model: Duration,
    badge_worker_dispatch: Duration,
    auxiliary_windows: Duration,
    shell_integration_placement: Duration,
}

impl StartupPhases {
    fn start() -> Self {
        let started = Instant::now();
        Self {
            started,
            checkpoint: started,
            settings: Duration::ZERO,
            graphics_window: Duration::ZERO,
            initial_window_tracking: Duration::ZERO,
            dock_model: Duration::ZERO,
            badge_worker_dispatch: Duration::ZERO,
            auxiliary_windows: Duration::ZERO,
            shell_integration_placement: Duration::ZERO,
        }
    }

    fn complete(&mut self) -> Duration {
        let elapsed = self.checkpoint.elapsed();
        self.checkpoint = Instant::now();
        elapsed
    }

    fn record_after_first_frame(self, first_frame: Duration) {
        lotus_windows::diagnostics::record_diagnostic(
            "startup.cold_boot",
            &format!(
                "settings_ms={} graphics_window_ms={} initial_window_tracking_ms={} dock_model_ms={} badge_worker_dispatch_ms={} auxiliary_windows_ms={} shell_integration_placement_ms={} first_frame_ms={} total_ms={}",
                self.settings.as_millis(),
                self.graphics_window.as_millis(),
                self.initial_window_tracking.as_millis(),
                self.dock_model.as_millis(),
                self.badge_worker_dispatch.as_millis(),
                self.auxiliary_windows.as_millis(),
                self.shell_integration_placement.as_millis(),
                first_frame.as_millis(),
                self.started.elapsed().as_millis(),
            ),
        );
    }
}

pub fn run() -> Result<(), AppError> {
    let mut startup_phases = StartupPhases::start();
    enable_per_monitor_v2()?;
    let startup = parse_startup_args(std::env::args_os().skip(1))?;
    let _restart_wait = wait_for_restart_source(startup.restart_after)?;
    let post_install_health = startup.post_install_health
        || lotus_windows::update::post_install_health_pending().unwrap_or(true)
        || lotus_windows::update::interrupted_install_health_pending().unwrap_or(true);
    if let Some(directory) = startup.cleanup_update.as_deref()
        && let Err(error) =
            lotus_windows::update::cleanup_requested_staging_directory(directory)
    {
        lotus_windows::diagnostics::record_error("update.cleanup_requested", &error);
    }
    let Some(_instance) = SingleInstance::acquire()? else {
        return Ok(());
    };

    let Some(prepared) = prepare_settings(post_install_health)? else {
        return Ok(());
    };
    let PreparedSettings {
        settings,
        store: settings_store,
        onboarding_required,
    } = prepared;
    let usage_store = SearchUsageStore::new(settings_store.directory());
    let usage = usage_store.load().unwrap_or_default();
    startup_phases.settings = startup_phases.complete();
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
    startup_phases.graphics_window = startup_phases.complete();
    let mut window_tracker = WindowTracker::start()?;
    startup_phases.initial_window_tracking = startup_phases.complete();
    let mut dock_model = DockRuntime::new(
        dock.handle(),
        settings,
        settings_store,
        window_tracker.current_windows(),
        dock.dpi(),
        dock.drag_threshold(),
    )?;
    startup_phases.dock_model = startup_phases.complete();
    let taskbar_badges = (!onboarding_required)
        .then(|| enable_notification_badges(&dock_model))
        .flatten();
    startup_phases.badge_worker_dispatch = startup_phases.complete();
    let mut auxiliary = create_auxiliary_windows(
        &dock,
        &mut dock_model,
        usage,
        usage_store,
        !onboarding_required,
    )?;
    startup_phases.auxiliary_windows = startup_phases.complete();
    resize_dock(&dock, &mut graphics, &mut surface, &dock_model)?;
    let mut integration = integration::IntegrationRecovery::new(
        dock_model.settings(),
        &dock,
        !onboarding_required,
    );
    window_tracker.refresh_fullscreen();
    let mut initial_windows = InitialWindows {
        dock: &dock,
        dock_model: &mut dock_model,
        graphics: &mut graphics,
        surface: &mut surface,
        window_tracker: &window_tracker,
        auxiliary: &mut auxiliary,
    };
    prepare_initial_windows(
        startup.open_settings,
        onboarding_required,
        &mut initial_windows,
    )?;
    startup_phases.shell_integration_placement = startup_phases.complete();
    let first_frame_started = Instant::now();
    flush_frame(
        &mut dock,
        &mut graphics,
        &mut surface,
        &mut dock_model,
        &mut auxiliary,
        lotus_ui::frame::FrameTrigger::Changes,
    )?;
    startup_phases.record_after_first_frame(first_frame_started.elapsed());
    let mut runtime = RuntimeServices {
        taskbar_badges: taskbar_badges.as_ref(),
        onboarding_required,
        integration: &mut integration,
    };
    let result = run_message_loop(
        &mut runtime,
        &mut dock,
        &mut graphics,
        &mut surface,
        &mut window_tracker,
        &mut dock_model,
        &mut auxiliary,
    );
    lotus_windows::responsiveness::METRICS.capture_process_resources();
    result
}

fn prepare_settings(
    post_install_health: bool,
) -> Result<Option<PreparedSettings>, AppError> {
    let (settings, store) = load_settings()?;
    let recovery_notice = match lotus_windows::update::recover_startup(post_install_health)
    {
        Ok(notice) => notice,
        Err(error) => {
            lotus_windows::diagnostics::record_error("update.recovery", &error);
            Some(format!(
                "Lotus found an incomplete update, but could not clean it safely. Please re-run the Lotus installer to repair the installation.\n\n{error}"
            ))
        }
    };
    std::thread::spawn(|| {
        for error in lotus_windows::update::cleanup_stale_staging() {
            lotus_windows::diagnostics::record_error("update.cleanup_stale", &error);
        }
    });
    if post_install_health {
        if let Err(error) =
            validate_post_install_health(&store, settings.start_with_windows)
        {
            lotus_windows::diagnostics::record_message(
                "update.post_install_health",
                &error,
            );
            let message = format!(
                "Lotus could not complete its post-install health check. Native shell integration was not started.\n\n{error}\n\nPlease re-run the Lotus installer and choose Repair."
            );
            if let Err(journal_error) =
                lotus_windows::update::complete_post_install_health(false, &message)
            {
                lotus_windows::diagnostics::record_error(
                    "update.post_install_health_journal",
                    &journal_error,
                );
            }
            lotus_windows::dialog::show_unowned_error("Lotus repair required", &message);
            return Ok(None);
        }
        if let Err(error) = lotus_windows::update::complete_post_install_health(true, "") {
            lotus_windows::diagnostics::record_error(
                "update.post_install_health_journal",
                &error,
            );
        }
        lotus_windows::diagnostics::record_message(
            "update.post_install_health",
            "installed executable, bridge DLLs, settings, and startup registration are healthy",
        );
    }
    if let Some(notice) = recovery_notice {
        lotus_windows::diagnostics::record_message("update.recovered", &notice);
        lotus_windows::dialog::show_unowned_error("Lotus Update", &notice);
    }
    let onboarding_required = settings.onboarding_version < CURRENT_ONBOARDING_VERSION;
    if !onboarding_required {
        let _ = sync_startup_preference(settings.start_with_windows);
    }
    Ok(Some(PreparedSettings {
        settings,
        store,
        onboarding_required,
    }))
}

fn prepare_initial_windows(
    open_settings: bool,
    onboarding_required: bool,
    windows: &mut InitialWindows<'_>,
) -> Result<(), AppError> {
    if !onboarding_required {
        windows.auxiliary.sync_status(
            windows.dock,
            windows.dock_model,
            windows.graphics,
        )?;
        windows.auxiliary.sync_monitor_docks(
            windows.dock,
            windows.dock_model,
            windows.graphics,
            windows.window_tracker,
        )?;
    }
    if onboarding_required {
        let _changed = windows.dock.set_visible(false);
        windows.auxiliary.set_status_visible(false);
        windows.auxiliary.open_onboarding(
            windows.dock_model.settings(),
            true,
            windows.graphics,
        )?;
    } else {
        apply_fullscreen_visibility(
            windows.dock,
            windows.surface,
            windows.window_tracker,
            windows.dock_model,
            windows.auxiliary,
        )?;
    }
    if open_settings && !onboarding_required {
        windows.auxiliary.open_settings_without_refresh(
            windows.dock_model.settings(),
            windows.graphics,
        )?;
    }
    Ok(())
}

fn create_auxiliary_windows(
    dock: &DockWindow,
    dock_model: &mut DockRuntime,
    usage: SearchUsage,
    usage_store: SearchUsageStore,
    modules_active: bool,
) -> Result<ModuleHost, AppError> {
    ModuleHost::create(dock, dock_model, usage, usage_store, modules_active)
}

fn sync_startup_preference(
    enabled: bool,
) -> Result<(), startup_registration::StartupRegistrationError> {
    if let Err(error) = startup_registration::sync(enabled) {
        lotus_windows::diagnostics::record_error("startup.registration", &error);
        return Err(error);
    }
    Ok(())
}

fn validate_post_install_health(
    settings_store: &SettingsStore,
    start_with_windows: bool,
) -> Result<(), String> {
    lotus_windows::update::verify_post_install_target()
        .map_err(|error| error.to_string())?;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    if !executable.is_file() {
        return Err("the installed lotus.exe is missing".to_owned());
    }
    let directory = executable
        .parent()
        .ok_or("the installed Lotus directory is invalid")?;
    if !directory.join("unins000.exe").is_file() {
        return Err("the Lotus uninstaller is missing".to_owned());
    }
    for bridge in ["lotus_shell_bridge.dll", "lotus_explorer_bridge.dll"] {
        if !directory.join(bridge).is_file() {
            return Err(format!("the installed {bridge} is missing"));
        }
    }
    fs::File::open(settings_store.settings_path())
        .map_err(|error| format!("Lotus settings could not be read: {error}"))?;
    sync_startup_preference(start_with_windows).map_err(|error| error.to_string())
}

fn enable_notification_badges(model: &DockRuntime) -> Option<TaskbarBadgeController> {
    if model.settings().notification_badge_style == NotificationBadgeStyle::Off {
        return None;
    }
    match TaskbarBadgeController::start() {
        Ok(controller) => Some(controller),
        Err(error) => {
            lotus_windows::diagnostics::record_error(
                "taskbar_badges.worker_dispatch",
                &error,
            );
            None
        }
    }
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
    match &load.source {
        SettingsLoadSource::Migrated {
            backup_path,
            from_version,
            to_version,
        } => lotus_windows::diagnostics::record_message(
            "settings.migrated",
            &format!(
                "Lotus migrated settings schema {from_version} to {to_version}. The original file is at `{}`.",
                backup_path.display()
            ),
        ),
        SettingsLoadSource::RecoveredInvalid { backup_path, error } => {
            lotus_windows::diagnostics::record_message(
                "settings.recovered_invalid",
                &format!(
                    "Lotus restored default settings after `{error}`. The original file is at `{}`.",
                    backup_path.display()
                ),
            );
        }
        SettingsLoadSource::CreatedDefaults | SettingsLoadSource::Existing => {}
    }

    Ok((load.settings, store))
}
