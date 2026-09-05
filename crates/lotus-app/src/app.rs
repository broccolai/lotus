mod activation;
mod applications;
mod context_menu;
mod dock;
mod icon_override;
mod integration;
mod launcher;
mod media;
mod modules;
mod monitors;
mod primary_dock;
mod runtime;
mod search_usage;
mod settings;
mod settings_persistence;
mod status;
mod surface_render;
mod switcher;
mod system_actions;
mod visuals;

#[repr(u32)]
#[derive(Clone, Copy)]
pub(super) enum PresentationSurface {
    Dock = 0,
    Launcher = 1,
    ContextMenu = 2,
    Settings = 3,
    Switcher = 4,
    Status = 5,
    Monitors = 6,
}

impl PresentationSurface {
    pub(super) const fn bit(self) -> u32 {
        1 << self as u32
    }
}

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use dock::DockRuntime;
use lotus_core::search::SearchUsage;
use lotus_core::settings::{
    CURRENT_ONBOARDING_VERSION, DockSettings, NotificationBadgeStyle, SettingsDecodeError,
    SettingsLoadSource, SettingsStore, SettingsStoreError, decode_settings,
};
use lotus_windows::activation::ActivationError;
use lotus_windows::dpi::enable_per_monitor_v2;
use lotus_windows::graphics::{DeviceState, SurfaceError};
use lotus_windows::single_instance::SingleInstance;
use lotus_windows::startup::{
    self as startup_registration, RestartWaitError, StartupArgsError, StartupMode,
    parse_startup_args, wait_for_restart_source,
};
use lotus_windows::taskbar_badges::TaskbarBadgeController;
use lotus_windows::window_tracker::WindowTracker;
use modules::ModuleHost;
use primary_dock::PrimaryDock;
use runtime::{flush_frame, run_message_loop};
use search_usage::SearchUsageStore;
use settings_persistence::SettingsPersistence;
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
    settings_persistence: SettingsPersistence,
    taskbar_badges: Option<&'a TaskbarBadgeController>,
    onboarding_required: bool,
    startup_mode: StartupMode,
    startup_registration_allowed: bool,
    integration: &'a mut integration::IntegrationRecovery,
}

struct PreparedSettings {
    settings: DockSettings,
    store: SettingsStore,
    onboarding_required: bool,
}

#[derive(Clone, Copy)]
struct StartupEnvironment {
    mode: StartupMode,
    manages_installed_update_state: bool,
    cleans_requested_staging: bool,
    syncs_startup_registration: bool,
}

impl StartupEnvironment {
    fn detect(mode: StartupMode) -> Self {
        let installer_managed = if mode.allows_update_operations() {
            match lotus_windows::update::is_installer_managed_executable() {
                Ok(installed) => installed,
                Err(error) => {
                    lotus_windows::diagnostics::record_error(
                        "update.installer_managed_detection",
                        &error,
                    );
                    false
                }
            }
        } else {
            false
        };
        Self {
            mode,
            manages_installed_update_state: mode.allows_update_operations()
                && installer_managed,
            cleans_requested_staging: mode.allows_update_operations(),
            syncs_startup_registration: mode.allows_startup_registration()
                && installer_managed,
        }
    }

    const fn allows_shell_integration(self) -> bool {
        self.mode.allows_shell_integration()
    }
}

struct InitialWindows<'a> {
    primary_dock: &'a mut PrimaryDock,
    dock_model: &'a mut DockRuntime,
    graphics: &'a mut DeviceState,
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
    let (startup, environment, post_install_health) = prepare_startup()?;
    let Some(_instance) = SingleInstance::acquire()? else {
        return Ok(());
    };

    let Some(prepared) = prepare_settings(environment, post_install_health)? else {
        return Ok(());
    };
    let PreparedSettings {
        settings,
        store: settings_store,
        onboarding_required,
    } = prepared;
    let settings_persistence = SettingsPersistence::new(settings_store);
    let usage_store = SearchUsageStore::new(settings_persistence.directory());
    let usage = usage_store.load().unwrap_or_default();
    startup_phases.settings = startup_phases.complete();
    let mut graphics = DeviceState::create()?;
    let mut primary_dock = PrimaryDock::create(&graphics, &settings)?;
    startup_phases.graphics_window = startup_phases.complete();
    let mut window_tracker = WindowTracker::start(startup.mode)?;
    startup_phases.initial_window_tracking = startup_phases.complete();
    let mut dock_model = DockRuntime::new(
        primary_dock.window().handle(),
        settings,
        window_tracker.current_windows(),
        primary_dock.window().dpi(),
        primary_dock.window().drag_threshold(),
    )?;
    startup_phases.dock_model = startup_phases.complete();
    let shell_effects_allowed = environment.allows_shell_integration();
    let taskbar_badges = (shell_effects_allowed && !onboarding_required)
        .then(|| enable_notification_badges(&dock_model))
        .flatten();
    startup_phases.badge_worker_dispatch = startup_phases.complete();
    let mut auxiliary = create_auxiliary_windows(
        primary_dock.window(),
        &mut dock_model,
        usage,
        usage_store,
        !onboarding_required,
        shell_effects_allowed,
        environment.mode.allows_update_operations(),
    )?;
    startup_phases.auxiliary_windows = startup_phases.complete();
    primary_dock.resize_for_model(&mut graphics, &dock_model)?;
    let mut integration = integration::IntegrationRecovery::new(
        dock_model.settings(),
        primary_dock.window(),
        shell_effects_allowed,
        shell_effects_allowed && !onboarding_required,
    );
    window_tracker.refresh_fullscreen();
    let mut initial_windows = InitialWindows {
        primary_dock: &mut primary_dock,
        dock_model: &mut dock_model,
        graphics: &mut graphics,
        window_tracker: &window_tracker,
        auxiliary: &mut auxiliary,
    };
    prepare_initial_windows(
        startup.open_settings,
        onboarding_required,
        shell_effects_allowed,
        &mut initial_windows,
    )?;
    startup_phases.shell_integration_placement = startup_phases.complete();
    let first_frame_started = Instant::now();
    flush_frame(
        &mut primary_dock,
        &mut graphics,
        &mut dock_model,
        &mut auxiliary,
        lotus_ui::frame::FrameTrigger::Changes,
    )?;
    startup_phases.record_after_first_frame(first_frame_started.elapsed());
    let mut runtime = RuntimeServices {
        settings_persistence,
        taskbar_badges: taskbar_badges.as_ref(),
        onboarding_required,
        startup_mode: environment.mode,
        startup_registration_allowed: environment.syncs_startup_registration,
        integration: &mut integration,
    };
    let result = run_message_loop(
        &mut runtime,
        &mut primary_dock,
        &mut graphics,
        &mut window_tracker,
        &mut dock_model,
        &mut auxiliary,
    );
    lotus_windows::responsiveness::METRICS.capture_process_resources();
    result
}

fn prepare_startup() -> Result<
    (
        lotus_windows::startup::StartupOptions,
        StartupEnvironment,
        bool,
    ),
    AppError,
> {
    let startup = parse_startup_args(std::env::args_os().skip(1))?;
    let _restart_wait = wait_for_restart_source(startup.restart_after)?;
    let environment = StartupEnvironment::detect(startup.mode);
    lotus_windows::diagnostics::record_diagnostic(
        "startup.launch_context",
        &format!(
            "mode={:?} shell_effects_allowed={} startup_registration_allowed={} installer_managed={}",
            environment.mode,
            environment.allows_shell_integration(),
            environment.syncs_startup_registration,
            environment.manages_installed_update_state,
        ),
    );
    lotus_windows::diagnostics::record_state(
        "startup.launch_state",
        &[
            ("development", u64::from(environment.mode.is_development())),
            (
                "preview",
                u64::from(environment.mode.uses_isolated_settings()),
            ),
            ("debug_build", u64::from(cfg!(debug_assertions))),
            (
                "installer_managed",
                u64::from(environment.manages_installed_update_state),
            ),
            (
                "registration_allowed",
                u64::from(environment.syncs_startup_registration),
            ),
            (
                "updates_allowed",
                u64::from(environment.mode.allows_update_operations()),
            ),
        ],
    );
    let post_install_health = if environment.manages_installed_update_state {
        startup.post_install_health
            || lotus_windows::update::post_install_health_pending().unwrap_or(true)
            || lotus_windows::update::interrupted_install_health_pending().unwrap_or(true)
    } else {
        false
    };
    if environment.cleans_requested_staging
        && let Some(directory) = startup.cleanup_update.as_deref()
        && let Err(error) =
            lotus_windows::update::cleanup_requested_staging_directory(directory)
    {
        lotus_windows::diagnostics::record_error("update.cleanup_requested", &error);
    }
    Ok((startup, environment, post_install_health))
}

fn prepare_settings(
    environment: StartupEnvironment,
    post_install_health: bool,
) -> Result<Option<PreparedSettings>, AppError> {
    let (settings, store) = load_settings(environment.mode)?;
    let recovery_notice = if environment.mode.allows_update_operations() {
        match lotus_windows::update::recover_failed_update_notice(environment.mode) {
            Ok(Some(notice)) => Some(notice),
            Ok(None) if environment.manages_installed_update_state => {
                match lotus_windows::update::recover_startup(post_install_health) {
                    Ok(notice) => notice,
                    Err(error) => {
                        lotus_windows::diagnostics::record_error("update.recovery", &error);
                        Some(format!(
                            "Lotus found an incomplete update, but could not clean it safely. Please re-run the Lotus installer to repair the installation.\n\n{error}"
                        ))
                    }
                }
            }
            Ok(None) => None,
            Err(error) => {
                lotus_windows::diagnostics::record_error("update.recovery", &error);
                Some(format!(
                    "Lotus found an incomplete update, but could not clean it safely. Please re-run the Lotus installer to repair the installation.\n\n{error}"
                ))
            }
        }
    } else {
        None
    };
    if environment.manages_installed_update_state {
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
                lotus_windows::dialog::show_unowned_error(
                    "Lotus repair required",
                    &message,
                );
                return Ok(None);
            }
            if let Err(error) =
                lotus_windows::update::complete_post_install_health(true, "")
            {
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
    }
    if let Some(notice) = recovery_notice {
        lotus_windows::diagnostics::record_message("update.recovered", &notice);
        lotus_windows::dialog::show_unowned_error("Lotus Update", &notice);
    }
    let onboarding_required = settings.onboarding_version < CURRENT_ONBOARDING_VERSION;
    if environment.syncs_startup_registration && !onboarding_required {
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
    shell_effects_allowed: bool,
    windows: &mut InitialWindows<'_>,
) -> Result<(), AppError> {
    if !onboarding_required {
        windows.auxiliary.sync_status(
            windows.primary_dock.window(),
            windows.dock_model,
            windows.graphics,
        )?;
        windows.auxiliary.sync_monitor_docks(
            windows.primary_dock.window(),
            windows.dock_model,
            windows.graphics,
            windows.window_tracker,
        )?;
    }
    if onboarding_required {
        let _changed = windows.primary_dock.window().set_visible(false);
        windows.auxiliary.set_status_visible(false);
        windows.auxiliary.open_onboarding(
            windows.dock_model.settings(),
            true,
            windows.graphics,
        )?;
    } else if shell_effects_allowed {
        runtime::apply_fullscreen_visibility(
            windows.primary_dock,
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
    dock: &lotus_windows::window::DockWindow,
    dock_model: &mut DockRuntime,
    usage: SearchUsage,
    usage_store: SearchUsageStore,
    modules_active: bool,
    shell_effects_allowed: bool,
    updates_allowed: bool,
) -> Result<ModuleHost, AppError> {
    ModuleHost::create(
        dock,
        dock_model,
        usage,
        usage_store,
        modules_active,
        shell_effects_allowed,
        updates_allowed,
    )
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

fn load_settings(mode: StartupMode) -> Result<(DockSettings, SettingsStore), AppError> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or(AppError::MissingLocalAppData)?;
    let live_settings_directory = local_app_data.join("Lotus");
    let settings_directory = if mode.uses_isolated_settings() {
        let preview_directory = live_settings_directory.join("preview");
        seed_preview_settings(&live_settings_directory, &preview_directory)?;
        preview_directory
    } else {
        let _ = fs::remove_file(live_settings_directory.join("lotus.log"));
        live_settings_directory
    };
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

fn seed_preview_settings(
    live: &std::path::Path,
    preview: &std::path::Path,
) -> Result<(), AppError> {
    let preview_settings = preview.join("settings.json");
    if preview_settings.exists() {
        return Ok(());
    }

    fs::create_dir_all(preview).map_err(|source| SettingsStoreError::Io {
        operation: "create preview settings directory at",
        path: preview.to_owned(),
        source,
    })?;
    let live_settings = live.join("settings.json");
    if live_settings.is_file() {
        fs::copy(&live_settings, &preview_settings).map_err(|source| {
            SettingsStoreError::Io {
                operation: "copy live settings to preview at",
                path: preview_settings,
                source,
            }
        })?;
    }
    Ok(())
}
