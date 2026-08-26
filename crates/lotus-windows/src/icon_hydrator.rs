use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use lotus_core::application::{ApplicationPresentationIcon, is_shared_host_executable};
use lotus_core::window::{TrackedWindowKey, WindowId};
use lotus_ui::icon::RasterIcon;
use thiserror::Error;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;

use crate::custom_image::CustomImageCache;
use crate::launch::ComApartment;
use crate::messages::ICON_HYDRATION_WAKE;
use crate::native_icon::NativeIconCache;
use crate::responsiveness::METRICS;

#[derive(Clone, Debug)]
pub struct LauncherIconRequest {
    pub generation: u64,
    pub identity: String,
    pub icon_source: PathBuf,
    pub custom_image_path: Option<PathBuf>,
    pub pixel_size: u32,
    pub settings_revision: u64,
}

#[derive(Clone, Debug)]
pub struct SwitcherIconRequest {
    pub generation: u64,
    pub window: TrackedWindowKey,
    pub executable_path: PathBuf,
    pub presentation_icon: Option<ApplicationPresentationIcon>,
    pub custom_image_path: Option<PathBuf>,
    pub pixel_size: u32,
    pub settings_revision: u64,
}

#[derive(Clone, Debug)]
pub struct DockIconRequest {
    pub identity: String,
    pub window: TrackedWindowKey,
    pub executable_path: PathBuf,
    pub presentation_icon: ApplicationPresentationIcon,
    pub pixel_size: u32,
}

#[derive(Clone, Debug)]
pub struct HydratedLauncherIcon {
    pub generation: u64,
    pub identity: String,
    pub pixel_size: u32,
    pub settings_revision: u64,
    pub icon: Option<RasterIcon>,
}

#[derive(Clone, Debug)]
pub struct HydratedSwitcherIcon {
    pub generation: u64,
    pub window: WindowId,
    pub pixel_size: u32,
    pub settings_revision: u64,
    pub icon: Option<RasterIcon>,
}

#[derive(Clone, Debug)]
pub struct HydratedDockIcon {
    pub identity: String,
    pub window: TrackedWindowKey,
    pub pixel_size: u32,
    pub icon: Option<RasterIcon>,
}

#[derive(Debug)]
pub enum IconHydrationResult {
    Launcher(HydratedLauncherIcon),
    Switcher(HydratedSwitcherIcon),
    Dock(HydratedDockIcon),
}

#[derive(Debug, Error)]
pub enum IconHydratorError {
    #[error("Lotus could not create its icon worker: {0}")]
    Thread(#[from] std::io::Error),
}

#[derive(Clone)]
pub struct LauncherIconClient {
    shared: Arc<SharedState>,
}

#[derive(Clone, Copy)]
enum Consumer {
    Launcher,
    Switcher,
    Dock,
}

#[derive(Clone)]
pub struct SwitcherIconClient {
    shared: Arc<SharedState>,
}

#[derive(Clone)]
pub struct DockIconClient {
    shared: Arc<SharedState>,
}

pub struct IconHydrator {
    shared: Arc<SharedState>,
    worker: Option<JoinHandle<()>>,
}

pub const fn is_icon_hydration_wake(message: u32) -> bool {
    message == ICON_HYDRATION_WAKE
}

struct SharedState {
    state: Mutex<State>,
    wake: Condvar,
    owner_thread: u32,
    wake_queued: AtomicBool,
    stopping: AtomicBool,
}

struct State {
    launcher: Option<Vec<LauncherIconRequest>>,
    switcher: Option<Vec<SwitcherIconRequest>>,
    dock: Option<Vec<DockIconRequest>>,
    next: Consumer,
    results: Vec<IconHydrationResult>,
}

impl IconHydrator {
    pub fn start() -> Result<Self, IconHydratorError> {
        let shared = Arc::new(SharedState {
            state: Mutex::new(State {
                launcher: None,
                switcher: None,
                dock: None,
                next: Consumer::Launcher,
                results: Vec::new(),
            }),
            wake: Condvar::new(),
            owner_thread: unsafe { GetCurrentThreadId() },
            wake_queued: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
        });
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("lotus-icon-hydrator".to_owned())
            .spawn(move || hydrate_icons(&worker_shared))?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }

    pub fn launcher_client(&self) -> LauncherIconClient {
        LauncherIconClient {
            shared: Arc::clone(&self.shared),
        }
    }

    pub fn switcher_client(&self) -> SwitcherIconClient {
        SwitcherIconClient {
            shared: Arc::clone(&self.shared),
        }
    }

    pub fn dock_client(&self) -> DockIconClient {
        DockIconClient {
            shared: Arc::clone(&self.shared),
        }
    }

    pub fn drain(&self) -> Vec<IconHydrationResult> {
        self.shared.wake_queued.store(false, Ordering::Release);
        let mut state = lock(&self.shared.state);
        std::mem::take(&mut state.results)
    }
}

impl Drop for IconHydrator {
    fn drop(&mut self) {
        {
            let _state = lock(&self.shared.state);
            self.shared.stopping.store(true, Ordering::Release);
        }
        self.shared.wake.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl LauncherIconClient {
    pub fn request_launcher(&self, requests: Vec<LauncherIconRequest>) {
        self.request(Work::Launcher(requests));
    }

    fn request(&self, work: Work) {
        request(&self.shared, work);
    }
}

impl SwitcherIconClient {
    pub fn request_switcher(&self, requests: Vec<SwitcherIconRequest>) {
        self.request(Work::Switcher(requests));
    }

    fn request(&self, work: Work) {
        request(&self.shared, work);
    }
}

impl DockIconClient {
    pub fn request_dock(&self, requests: Vec<DockIconRequest>) {
        self.request(Work::Dock(requests));
    }

    fn request(&self, work: Work) {
        request(&self.shared, work);
    }
}

fn request(shared: &SharedState, work: Work) {
    let mut state = lock(&shared.state);
    if shared.stopping.load(Ordering::Acquire) {
        return;
    }
    match work {
        Work::Launcher(requests) => {
            state.launcher = (!requests.is_empty()).then_some(requests);
        }
        Work::Switcher(requests) => {
            if !requests.is_empty() {
                METRICS.record_switcher_requests(requests.len());
            }
            state.switcher = (!requests.is_empty()).then_some(requests);
        }
        Work::Dock(requests) => state.dock = (!requests.is_empty()).then_some(requests),
    }
    drop(state);
    shared.wake.notify_one();
}

enum Work {
    Launcher(Vec<LauncherIconRequest>),
    Switcher(Vec<SwitcherIconRequest>),
    Dock(Vec<DockIconRequest>),
}

fn hydrate_icons(shared: &SharedState) {
    let _apartment = ComApartment::enter();
    let mut native_icons = NativeIconCache::default();
    let mut custom_images = CustomImageCache::default();

    while let Some(work) = next_work(shared) {
        let results = match work {
            Work::Launcher(requests) => requests
                .iter()
                .map(|request| {
                    IconHydrationResult::Launcher(hydrate_launcher_icon(
                        request,
                        &mut native_icons,
                        &mut custom_images,
                    ))
                })
                .collect(),
            Work::Switcher(requests) => requests
                .iter()
                .map(|request| {
                    IconHydrationResult::Switcher(hydrate_switcher_icon(
                        request,
                        &mut native_icons,
                        &mut custom_images,
                    ))
                })
                .collect(),
            Work::Dock(requests) => requests
                .iter()
                .map(|request| {
                    IconHydrationResult::Dock(hydrate_dock_icon(request, &mut native_icons))
                })
                .collect(),
        };
        publish(results, shared);
    }
}

fn next_work(shared: &SharedState) -> Option<Work> {
    let mut state = lock(&shared.state);
    loop {
        if shared.stopping.load(Ordering::Acquire) {
            return None;
        }
        let work = match state.next {
            Consumer::Launcher => state
                .launcher
                .take()
                .map(Work::Launcher)
                .or_else(|| state.switcher.take().map(Work::Switcher))
                .or_else(|| take_dock_quantum(&mut state)),
            Consumer::Switcher => state
                .switcher
                .take()
                .map(Work::Switcher)
                .or_else(|| take_dock_quantum(&mut state))
                .or_else(|| state.launcher.take().map(Work::Launcher)),
            Consumer::Dock => take_dock_quantum(&mut state)
                .or_else(|| state.launcher.take().map(Work::Launcher))
                .or_else(|| state.switcher.take().map(Work::Switcher)),
        };
        if let Some(work) = work {
            state.next = match work {
                Work::Launcher(_) => Consumer::Switcher,
                Work::Switcher(_) => Consumer::Dock,
                Work::Dock(_) => Consumer::Launcher,
            };
            return Some(work);
        }
        state = shared
            .wake
            .wait(state)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }
}

fn take_dock_quantum(state: &mut State) -> Option<Work> {
    let mut requests = state.dock.take()?;
    let remainder = requests.split_off(1);
    state.dock = (!remainder.is_empty()).then_some(remainder);
    Some(Work::Dock(requests))
}

fn hydrate_dock_icon(
    request: &DockIconRequest,
    native_icons: &mut NativeIconCache,
) -> HydratedDockIcon {
    let icon = hydrate_presentation_icon(
        request.window,
        &request.executable_path,
        Some(&request.presentation_icon),
        request.pixel_size,
        native_icons,
    );
    HydratedDockIcon {
        identity: request.identity.clone(),
        window: request.window,
        pixel_size: request.pixel_size,
        icon,
    }
}

fn hydrate_launcher_icon(
    request: &LauncherIconRequest,
    native_icons: &mut NativeIconCache,
    custom_images: &mut CustomImageCache,
) -> HydratedLauncherIcon {
    let icon = request
        .custom_image_path
        .as_deref()
        .and_then(|path| custom_images.image(path).ok())
        .or_else(|| {
            native_icons
                .icon(&request.icon_source, request.pixel_size)
                .ok()
                .flatten()
        });
    HydratedLauncherIcon {
        generation: request.generation,
        identity: request.identity.clone(),
        pixel_size: request.pixel_size,
        settings_revision: request.settings_revision,
        icon,
    }
}

fn hydrate_switcher_icon(
    request: &SwitcherIconRequest,
    native_icons: &mut NativeIconCache,
    custom_images: &mut CustomImageCache,
) -> HydratedSwitcherIcon {
    let icon = request
        .custom_image_path
        .as_deref()
        .and_then(|path| custom_images.image(path).ok())
        .or_else(|| {
            hydrate_presentation_icon(
                request.window,
                &request.executable_path,
                request.presentation_icon.as_ref(),
                request.pixel_size,
                native_icons,
            )
        });
    HydratedSwitcherIcon {
        generation: request.generation,
        window: request.window.id,
        pixel_size: request.pixel_size,
        settings_revision: request.settings_revision,
        icon,
    }
}

fn hydrate_presentation_icon(
    window: TrackedWindowKey,
    executable_path: &std::path::Path,
    presentation_icon: Option<&ApplicationPresentationIcon>,
    pixel_size: u32,
    native_icons: &mut NativeIconCache,
) -> Option<RasterIcon> {
    let presentation_icon = presentation_icon?;
    match presentation_icon {
        ApplicationPresentationIcon::NativeWindow { fallback_path, .. } => {
            window_icon(window, pixel_size)
                .or_else(|| source_icon(native_icons, fallback_path.as_ref(), pixel_size))
        }
        ApplicationPresentationIcon::Source(source)
            if crate::native_icon::is_shell_namespace_path(source.as_ref()) =>
        {
            if is_shared_host_executable(&executable_path.to_string_lossy()) {
                window_icon(window, pixel_size)
                    .or_else(|| source_icon(native_icons, source.as_ref(), pixel_size))
                    .or_else(|| source_icon(native_icons, executable_path, pixel_size))
            } else {
                source_icon(native_icons, executable_path, pixel_size)
                    .or_else(|| window_icon(window, pixel_size))
                    .or_else(|| source_icon(native_icons, source.as_ref(), pixel_size))
            }
        }
        ApplicationPresentationIcon::Source(source) => {
            source_icon(native_icons, source.as_ref(), pixel_size)
                .or_else(|| window_icon(window, pixel_size))
                .or_else(|| source_icon(native_icons, executable_path, pixel_size))
        }
    }
}

fn source_icon(
    native_icons: &mut NativeIconCache,
    source: &std::path::Path,
    pixel_size: u32,
) -> Option<RasterIcon> {
    native_icons.icon(source, pixel_size).ok().flatten()
}

fn window_icon(window: TrackedWindowKey, pixel_size: u32) -> Option<RasterIcon> {
    crate::native_icon::window_icon(window, pixel_size)
        .ok()
        .flatten()
}

fn publish(results: Vec<IconHydrationResult>, shared: &SharedState) {
    if results.is_empty() || shared.stopping.load(Ordering::Acquire) {
        return;
    }
    let switcher_results = results
        .iter()
        .filter(|result| matches!(result, IconHydrationResult::Switcher(_)))
        .count();
    if switcher_results != 0 {
        METRICS.record_switcher_results(switcher_results);
    }
    lock(&shared.state).results.extend(results);
    if !shared.wake_queued.swap(true, Ordering::AcqRel)
        && unsafe {
            PostThreadMessageW(
                shared.owner_thread,
                ICON_HYDRATION_WAKE,
                WPARAM(0),
                LPARAM(0),
            )
        }
        .is_err()
    {
        shared.wake_queued.store(false, Ordering::Release);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
