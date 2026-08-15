mod context_menu;
mod dock;
mod launcher;
mod media;
mod monitors;
mod runtime;
mod runtime_helpers;
mod settings;
mod status;
mod switcher;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use context_menu::ContextMenuRuntime;
use dock::DockRuntime;
use launcher::{LauncherRuntime, LauncherSubmission};
use lotus_core::launcher_model::{CursorMove as ModelCursorMove, QueryEdit, SelectionMove};
use lotus_core::notification::NotificationSource;
use lotus_core::search::SearchUsage;
use lotus_core::settings::{
    CURRENT_ONBOARDING_VERSION, DockSettings, DockZone, NotificationBadgeStyle,
    SettingsDecodeError, SettingsStore, SettingsStoreError, WindowPickerStyle,
    decode_settings,
};
use lotus_core::window::{WindowId, WindowInfo};
use lotus_dock::popup::order_picker_windows;
use lotus_media::MediaSnapshot;
use lotus_search::command::CommandId;
use lotus_search::controller::{SearchController, SearchPresentation};
use lotus_search::usage::SearchUsageStore;
use lotus_settings::appearance::theme_for;
use lotus_switcher::model::{RecentOrder, SwitcherSession};
use lotus_ui::geometry::NonZeroPhysicalSize;
use lotus_windows::WindowHandle;
use lotus_windows::activation::{
    ActivationError, execute_activation, foreground_window, launch_target,
    request_window_close, switch_window,
};
use lotus_windows::alt_tab::{AltTabController, AltTabEvent, is_alt_tab_wake};
use lotus_windows::appbar::{ShellIntegration, fullscreen_notification};
use lotus_windows::clipboard::{read_text, write_text};
use lotus_windows::clock::{local_date, local_time_24h};
use lotus_windows::dialog::{
    confirm_install_update, confirm_restart, confirm_shutdown, show_error, show_information,
};
use lotus_windows::dpi::enable_per_monitor_v2;
use lotus_windows::dwm_thumbnail::DwmThumbnailHost;
use lotus_windows::interaction::{next_message, request_exit};
use lotus_windows::launch::resolve_executable;
use lotus_windows::media::{
    MediaCommand, MediaController, MediaEvent, decode_artwork, is_media_wake,
};
use lotus_windows::native_icon::NativeIconCache;
use lotus_windows::search_catalog::{SearchCatalogCache, is_search_catalog_wake};
use lotus_windows::single_instance::SingleInstance;
use lotus_windows::startup::{
    self as startup_registration, RestartWaitError, StartupArgsError, parse_startup_args,
    wait_for_restart_source,
};
use lotus_windows::taskbar_badges::{TaskbarBadgeController, is_taskbar_badge_wake};
use lotus_windows::update::{
    UpdateResult, UpdateStatus, is_installed, is_update_wake, launch_installer,
};
use lotus_windows::window_tracker::{WindowTracker, WindowTrackerEvent};
use lotus_windows::windows_key::{
    WindowsKeyController, WindowsKeyError, WindowsKeyEvent, is_windows_key_wake,
};
use media::MediaRuntime;
use monitors::{MonitorDockAction, MonitorDocks};
use runtime::run_message_loop;
use runtime_helpers::{
    apply_fullscreen_visibility, enable_optional_alt_tab, enable_optional_windows_key,
    handle_alt_tab_events, handle_pointer_event, handle_search_event,
    handle_windows_key_events, render_and_schedule, render_surface, resize_dock,
    resize_launcher_surface, resize_surface, restart_current_process,
};
use settings::SettingsRuntime;
use status::{AuxiliaryZoneAction, StatusRuntime};
use switcher::{AuxiliaryWindows, SwitcherRuntime};
use thiserror::Error;

use crate::graphics::assets::SvgAsset;
use crate::graphics::context_menu_scene::{
    AppMenuAction, ContextMenuAction, ContextMenuScene, NativePickerWindow, PopupAction,
    PowerAction,
};
use crate::graphics::context_menu_surface::ContextMenuCompositionSurfaceState;
use crate::graphics::launcher_scene::{LauncherResult, LauncherScene};
use crate::graphics::launcher_surface::LauncherCompositionSurfaceState;
use crate::graphics::scene::{
    DockAnchor, DockBadge, DockHitTarget, DockIcon, DockMetrics, DockScene, MediaItem,
    MediaSymbols, SystemStatusItem, SystemStatusKind,
};
use crate::graphics::scene_adapter::{
    adapt_dock_items_with_native, resolve_icon_with_native,
};
use crate::graphics::settings_scene::{
    SettingsAction, SettingsKey as SceneSettingsKey, SettingsScene, SettingsSize,
    SettingsSlider, SettingsUpdateActivity,
};
use crate::graphics::settings_surface::SettingsCompositionSurfaceState;
use crate::graphics::switcher_scene::{SwitcherItem, SwitcherScene};
use crate::graphics::switcher_surface::SwitcherCompositionSurfaceState;
use crate::graphics::{CompositionSurfaceState, DeviceState, SurfaceError, SurfaceSize};
use crate::window::{
    ContextMenuEvent, ContextMenuWindow, CursorMove as WindowCursorMove,
    DockContextRequest, DockWindow, PointerEvent, PopupAlignment, SearchEdit, SearchEvent,
    SearchWindow, SelectionDirection, SettingsEvent, SettingsKey as WindowSettingsKey,
    SettingsWindow, SignedPoint, StatusWindow, SwitcherEvent, SwitcherWindow, WindowEvent,
};

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Graphics(#[from] crate::graphics::GraphicsDeviceError),
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
    windows_key: Option<&'a WindowsKeyController>,
    alt_tab: Option<&'a AltTabController>,
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
    let mut surface =
        CompositionSurfaceState::create(graphics_device, dock.handle(), surface_size)?;
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
    dock.set_status_refresh_active(
        dock_model.settings().show_system_status
            && dock_model.settings().show_date_time_status,
    )?;
    let mut auxiliary = create_auxiliary_windows(&dock, &dock_model, usage, usage_store)?;
    let windows_key = enable_optional_windows_key(
        !onboarding_required
            && dock_model.settings().search_enabled
            && dock_model.settings().search_open_with_windows_key,
        || {
            let mut controller = WindowsKeyController::new();
            let _enabled = controller.enable()?;
            Ok::<WindowsKeyController, WindowsKeyError>(controller)
        },
    );
    let alt_tab = enable_optional_alt_tab(
        !onboarding_required && dock_model.settings().alt_tab_enabled,
    );
    resize_dock(&dock, &mut graphics, &mut surface, &dock_model)?;
    let _shell_integration = if onboarding_required {
        None
    } else {
        ShellIntegration::setup(dock_model.settings(), &dock).unwrap_or(None)
    };
    window_tracker.refresh_fullscreen();
    if !onboarding_required {
        auxiliary.status.sync(
            &dock,
            dock_model.settings(),
            dock_model.media(),
            &mut graphics,
        )?;
        auxiliary
            .monitors
            .sync(&dock, &mut dock_model, &mut graphics, &window_tracker)?;
    }
    render_and_schedule(
        &dock,
        &mut graphics,
        &mut surface,
        dock_model.scene(),
        false,
    )?;
    if onboarding_required {
        let _changed = dock.set_visible(false);
        auxiliary.status.set_visible(false);
        auxiliary
            .settings
            .open_onboarding(dock_model.settings(), true, &mut graphics)?;
    } else {
        apply_fullscreen_visibility(
            &dock,
            &window_tracker,
            &dock_model,
            &mut auxiliary.launcher,
            &auxiliary.status,
        )?;
    }
    if startup.open_settings && !onboarding_required {
        auxiliary
            .settings
            .open(dock_model.settings(), &mut graphics)?;
    }
    let runtime = RuntimePolicy {
        windows_key: windows_key.as_ref(),
        alt_tab: alt_tab.as_ref(),
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
) -> Result<AuxiliaryWindows, AppError> {
    let search_window = dock.create_search_window()?;
    lotus_windows::backdrop::apply_search_settings(
        search_window.handle(),
        dock_model.settings(),
    );
    let launcher = LauncherRuntime::new(
        search_window,
        dock_model.settings().search_result_limit,
        &theme_for(dock_model.settings()),
        usage,
        usage_store,
    );
    let settings = SettingsRuntime::new(
        dock.create_settings_window()?,
        dock_model.settings().clone(),
        is_installed().unwrap_or(false),
    )?;
    let context_menu_window = dock.create_context_menu_window()?;
    lotus_windows::backdrop::apply_popup_settings(
        context_menu_window.handle(),
        dock_model.settings(),
    );
    let context_menu =
        ContextMenuRuntime::new(context_menu_window, &theme_for(dock_model.settings()))?;
    let switcher_window = dock.create_switcher_window()?;
    lotus_windows::backdrop::apply_popup_settings(
        switcher_window.handle(),
        dock_model.settings(),
    );
    let switcher = SwitcherRuntime::new(switcher_window, &theme_for(dock_model.settings()));
    let media = MediaRuntime::new(dock_model.settings().show_media_controls);
    let status = StatusRuntime::new(
        [dock.create_status_window()?, dock.create_status_window()?],
        dock_model.settings(),
    )?;

    Ok(AuxiliaryWindows {
        launcher,
        settings,
        context_menu,
        media,
        status,
        monitors: MonitorDocks::new(),
        switcher,
    })
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
    let legacy_without_onboarding = settings_existed
        && fs::read_to_string(store.settings_path()).is_ok_and(|contents| {
            !contents
                .to_ascii_lowercase()
                .contains("\"onboardingversion\"")
        });

    if !settings_existed {
        let shipped_defaults = decode_settings(include_str!(
            "../../lotus-core/assets/settings.default.json"
        ))?;
        store.save(&shipped_defaults)?;
    }

    let mut settings = store.load()?.settings;
    if legacy_without_onboarding {
        settings.onboarding_version = CURRENT_ONBOARDING_VERSION;
        store.save(&settings)?;
    }

    Ok((settings, store))
}
