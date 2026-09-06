use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, mpsc};
use std::thread;
use std::time::Duration;

use lotus_core::application::{ApplicationPresentationIcon, is_shared_host_executable};
use lotus_core::window::TrackedWindowKey;
use lotus_ui::icon::RasterIcon;
use thiserror::Error;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;

use crate::background_worker::{BackgroundWorker, WorkerJoinPolicy};
use crate::custom_image::CustomImageCache;
use crate::launch::ComApartment;
use crate::messages::ICON_HYDRATION_WAKE;
use crate::native_icon::NativeIconCache;
use crate::responsiveness::METRICS;

const START_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_PENDING_RESULTS: usize = 128;

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
    pub window: Option<TrackedWindowKey>,
    pub executable_path: PathBuf,
    pub presentation_icon: ApplicationPresentationIcon,
    pub custom_image_path: Option<PathBuf>,
    pub pixel_size: u32,
}

#[derive(Clone, Debug)]
pub struct SettingsIconRequest {
    pub identity: String,
    pub icon_source: PathBuf,
    pub custom_image_path: Option<PathBuf>,
    pub pixel_size: u32,
    pub settings_revision: u64,
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
    pub window: TrackedWindowKey,
    pub pixel_size: u32,
    pub settings_revision: u64,
    pub presentation_icon: Option<ApplicationPresentationIcon>,
    pub custom_image_path: Option<PathBuf>,
    pub icon: Option<RasterIcon>,
}

#[derive(Clone, Debug)]
pub struct HydratedDockIcon {
    pub identity: String,
    pub window: Option<TrackedWindowKey>,
    pub executable_path: PathBuf,
    pub presentation_icon: ApplicationPresentationIcon,
    pub custom_image_path: Option<PathBuf>,
    pub pixel_size: u32,
    pub icon: Option<RasterIcon>,
}

#[derive(Clone, Debug)]
pub struct HydratedSettingsIcon {
    pub identity: String,
    pub icon_source: PathBuf,
    pub custom_image_path: Option<PathBuf>,
    pub pixel_size: u32,
    pub settings_revision: u64,
    pub icon: Option<RasterIcon>,
}

#[derive(Debug)]
pub enum IconHydrationResult {
    Launcher(HydratedLauncherIcon),
    Switcher(HydratedSwitcherIcon),
    Dock(HydratedDockIcon),
    Settings(HydratedSettingsIcon),
}

#[derive(Debug, Error)]
pub enum IconHydratorError {
    #[error("Lotus could not create its icon worker: {0}")]
    Thread(#[from] std::io::Error),
    #[error("Lotus could not initialize COM for its icon worker")]
    ComUnavailable,
    #[error("the icon worker stopped before it became ready")]
    WorkerExitedBeforeReady,
    #[error("the icon worker did not become ready within two seconds")]
    StartTimeout,
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
    Settings,
}

#[derive(Clone)]
pub struct SwitcherIconClient {
    shared: Arc<SharedState>,
}

#[derive(Clone)]
pub struct DockIconClient {
    shared: Arc<SharedState>,
}

#[derive(Clone)]
pub struct SettingsIconClient {
    shared: Arc<SharedState>,
}

pub struct IconHydrator {
    shared: Arc<SharedState>,
    worker: BackgroundWorker,
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
    settings: Option<Vec<SettingsIconRequest>>,
    epochs: [u64; 4],
    next: Consumer,
    results: VecDeque<IconHydrationResult>,
}

enum WorkerStartupError {
    ComUnavailable,
}

impl IconHydrator {
    pub fn start() -> Result<Self, IconHydratorError> {
        let shared = Arc::new(SharedState {
            state: Mutex::new(State {
                launcher: None,
                switcher: None,
                dock: None,
                settings: None,
                epochs: [0; 4],
                next: Consumer::Launcher,
                results: VecDeque::new(),
            }),
            wake: Condvar::new(),
            owner_thread: unsafe { GetCurrentThreadId() },
            wake_queued: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
        });
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("lotus-icon-hydrator".to_owned())
            .spawn(move || hydrate_icons(&worker_shared, &ready_sender))?;
        let stop_shared = Arc::clone(&shared);
        let mut worker =
            BackgroundWorker::new(worker, WorkerJoinPolicy::WhenFinished, move || {
                stop_icon_hydrator(&stop_shared);
            });

        match ready_receiver.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => Ok(Self { shared, worker }),
            Ok(Err(WorkerStartupError::ComUnavailable)) => {
                worker.shutdown();
                Err(IconHydratorError::ComUnavailable)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                worker.shutdown();
                Err(IconHydratorError::WorkerExitedBeforeReady)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                drop(ready_receiver);
                worker.shutdown();
                Err(IconHydratorError::StartTimeout)
            }
        }
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

    pub fn settings_client(&self) -> SettingsIconClient {
        SettingsIconClient {
            shared: Arc::clone(&self.shared),
        }
    }

    pub fn drain(&self) -> Vec<IconHydrationResult> {
        self.shared.wake_queued.store(false, Ordering::Release);
        let mut state = lock(&self.shared.state);
        state.results.drain(..).collect()
    }
}

impl Drop for IconHydrator {
    fn drop(&mut self) {
        self.worker.shutdown();
    }
}

fn stop_icon_hydrator(shared: &SharedState) {
    {
        let _state = lock(&shared.state);
        shared.stopping.store(true, Ordering::Release);
    }
    shared.wake.notify_all();
}

impl LauncherIconClient {
    pub fn request_launcher(&self, requests: Vec<LauncherIconRequest>) {
        self.request(Work::Launcher { requests, epoch: 0 });
    }

    fn request(&self, work: Work) {
        request(&self.shared, work);
    }
}

impl SwitcherIconClient {
    pub fn request_switcher(&self, requests: Vec<SwitcherIconRequest>) {
        self.request(Work::Switcher { requests, epoch: 0 });
    }

    fn request(&self, work: Work) {
        request(&self.shared, work);
    }
}

impl DockIconClient {
    pub fn request_dock(&self, requests: Vec<DockIconRequest>) {
        self.request(Work::Dock { requests, epoch: 0 });
    }

    fn request(&self, work: Work) {
        request(&self.shared, work);
    }
}

impl SettingsIconClient {
    pub fn request_settings(&self, requests: Vec<SettingsIconRequest>) {
        request(&self.shared, Work::Settings { requests, epoch: 0 });
    }
}

fn request(shared: &SharedState, work: Work) {
    let mut state = lock(&shared.state);
    if shared.stopping.load(Ordering::Acquire) {
        return;
    }
    match work {
        Work::Launcher { requests, .. } => {
            state.epochs[0] = state.epochs[0].wrapping_add(1);
            state.launcher = (!requests.is_empty()).then_some(requests);
        }
        Work::Switcher { requests, .. } => {
            state.epochs[1] = state.epochs[1].wrapping_add(1);
            if !requests.is_empty() {
                METRICS.record_switcher_requests(requests.len());
            }
            state.switcher = (!requests.is_empty()).then_some(requests);
        }
        Work::Dock { requests, .. } => {
            state.epochs[2] = state.epochs[2].wrapping_add(1);
            state.dock = (!requests.is_empty()).then_some(requests);
        }
        Work::Settings { requests, .. } => {
            state.epochs[3] = state.epochs[3].wrapping_add(1);
            state.settings = (!requests.is_empty()).then_some(requests);
        }
    }
    drop(state);
    shared.wake.notify_one();
}

enum Work {
    Launcher {
        requests: Vec<LauncherIconRequest>,
        epoch: u64,
    },
    Switcher {
        requests: Vec<SwitcherIconRequest>,
        epoch: u64,
    },
    Dock {
        requests: Vec<DockIconRequest>,
        epoch: u64,
    },
    Settings {
        requests: Vec<SettingsIconRequest>,
        epoch: u64,
    },
}

fn hydrate_icons(
    shared: &SharedState,
    ready: &mpsc::SyncSender<Result<(), WorkerStartupError>>,
) {
    let Some(_apartment) = ComApartment::enter() else {
        let _ = ready.send(Err(WorkerStartupError::ComUnavailable));
        return;
    };
    if ready.send(Ok(())).is_err() {
        return;
    }

    let mut native_icons = NativeIconCache::default();
    let mut custom_images = CustomImageCache::default();

    while let Some(work) = next_work(shared) {
        let (results, consumer, epoch) = match work {
            Work::Launcher { requests, epoch } => (
                requests
                    .iter()
                    .filter(|_| current_epoch(shared, Consumer::Launcher) == Some(epoch))
                    .map(|request| {
                        IconHydrationResult::Launcher(hydrate_launcher_icon(
                            request,
                            &mut native_icons,
                            &mut custom_images,
                        ))
                    })
                    .collect(),
                Consumer::Launcher,
                epoch,
            ),
            Work::Switcher { requests, epoch } => (
                requests
                    .iter()
                    .filter(|_| current_epoch(shared, Consumer::Switcher) == Some(epoch))
                    .map(|request| {
                        IconHydrationResult::Switcher(hydrate_switcher_icon(
                            request,
                            &mut native_icons,
                            &mut custom_images,
                        ))
                    })
                    .collect(),
                Consumer::Switcher,
                epoch,
            ),
            Work::Dock { requests, epoch } => (
                requests
                    .iter()
                    .filter(|_| current_epoch(shared, Consumer::Dock) == Some(epoch))
                    .map(|request| {
                        IconHydrationResult::Dock(hydrate_dock_icon(
                            request,
                            &mut native_icons,
                            &mut custom_images,
                        ))
                    })
                    .collect(),
                Consumer::Dock,
                epoch,
            ),
            Work::Settings { requests, epoch } => (
                requests
                    .iter()
                    .filter(|_| current_epoch(shared, Consumer::Settings) == Some(epoch))
                    .map(|request| {
                        IconHydrationResult::Settings(hydrate_settings_icon(
                            request,
                            &mut native_icons,
                            &mut custom_images,
                        ))
                    })
                    .collect(),
                Consumer::Settings,
                epoch,
            ),
        };
        publish(results, shared, consumer, epoch);
    }
}

fn next_work(shared: &SharedState) -> Option<Work> {
    let mut state = lock(&shared.state);
    loop {
        if shared.stopping.load(Ordering::Acquire) {
            return None;
        }
        let epochs = state.epochs;
        let work = match state.next {
            Consumer::Launcher => take_quantum(&mut state.launcher, epochs[0])
                .map(|(requests, epoch)| Work::Launcher { requests, epoch })
                .or_else(|| {
                    take_quantum(&mut state.switcher, epochs[1])
                        .map(|(requests, epoch)| Work::Switcher { requests, epoch })
                })
                .or_else(|| take_dock_quantum(&mut state))
                .or_else(|| {
                    take_quantum(&mut state.settings, epochs[3])
                        .map(|(requests, epoch)| Work::Settings { requests, epoch })
                }),
            Consumer::Switcher => take_quantum(&mut state.switcher, epochs[1])
                .map(|(requests, epoch)| Work::Switcher { requests, epoch })
                .or_else(|| take_dock_quantum(&mut state))
                .or_else(|| {
                    take_quantum(&mut state.launcher, epochs[0])
                        .map(|(requests, epoch)| Work::Launcher { requests, epoch })
                })
                .or_else(|| {
                    take_quantum(&mut state.settings, epochs[3])
                        .map(|(requests, epoch)| Work::Settings { requests, epoch })
                }),
            Consumer::Dock => take_dock_quantum(&mut state)
                .or_else(|| {
                    take_quantum(&mut state.launcher, epochs[0])
                        .map(|(requests, epoch)| Work::Launcher { requests, epoch })
                })
                .or_else(|| {
                    take_quantum(&mut state.switcher, epochs[1])
                        .map(|(requests, epoch)| Work::Switcher { requests, epoch })
                })
                .or_else(|| {
                    take_quantum(&mut state.settings, epochs[3])
                        .map(|(requests, epoch)| Work::Settings { requests, epoch })
                }),
            Consumer::Settings => take_quantum(&mut state.settings, epochs[3])
                .map(|(requests, epoch)| Work::Settings { requests, epoch })
                .or_else(|| {
                    take_quantum(&mut state.launcher, epochs[0])
                        .map(|(requests, epoch)| Work::Launcher { requests, epoch })
                })
                .or_else(|| {
                    take_quantum(&mut state.switcher, epochs[1])
                        .map(|(requests, epoch)| Work::Switcher { requests, epoch })
                })
                .or_else(|| take_dock_quantum(&mut state)),
        };
        if let Some(work) = work {
            state.next = match work {
                Work::Launcher { .. } => Consumer::Switcher,
                Work::Switcher { .. } => Consumer::Dock,
                Work::Dock { .. } => Consumer::Settings,
                Work::Settings { .. } => Consumer::Launcher,
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
    take_quantum(&mut state.dock, state.epochs[2])
        .map(|(requests, epoch)| Work::Dock { requests, epoch })
}

fn take_quantum<T>(slot: &mut Option<Vec<T>>, epoch: u64) -> Option<(Vec<T>, u64)> {
    let mut requests = slot.take()?;
    let remainder = requests.split_off(1);
    *slot = (!remainder.is_empty()).then_some(remainder);
    Some((requests, epoch))
}

fn current_epoch(shared: &SharedState, consumer: Consumer) -> Option<u64> {
    if shared.stopping.load(Ordering::Acquire) {
        return None;
    }
    let state = lock(&shared.state);
    Some(state.epochs[consumer as usize])
}

fn hydrate_dock_icon(
    request: &DockIconRequest,
    native_icons: &mut NativeIconCache,
    custom_images: &mut CustomImageCache,
) -> HydratedDockIcon {
    let icon = request
        .custom_image_path
        .as_deref()
        .and_then(|path| custom_images.image(path).ok())
        .or_else(|| match request.window {
            Some(window) => hydrate_presentation_icon(
                window,
                &request.executable_path,
                Some(&request.presentation_icon),
                request.pixel_size,
                native_icons,
            ),
            None => source_icon(
                native_icons,
                request.presentation_icon.fallback_path().as_ref(),
                request.pixel_size,
            ),
        });
    HydratedDockIcon {
        identity: request.identity.clone(),
        window: request.window,
        executable_path: request.executable_path.clone(),
        presentation_icon: request.presentation_icon.clone(),
        custom_image_path: request.custom_image_path.clone(),
        pixel_size: request.pixel_size,
        icon,
    }
}

fn hydrate_settings_icon(
    request: &SettingsIconRequest,
    native_icons: &mut NativeIconCache,
    custom_images: &mut CustomImageCache,
) -> HydratedSettingsIcon {
    let icon = request
        .custom_image_path
        .as_deref()
        .and_then(|path| custom_images.image(path).ok())
        .or_else(|| source_icon(native_icons, &request.icon_source, request.pixel_size));
    HydratedSettingsIcon {
        identity: request.identity.clone(),
        icon_source: request.icon_source.clone(),
        custom_image_path: request.custom_image_path.clone(),
        pixel_size: request.pixel_size,
        settings_revision: request.settings_revision,
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
        window: request.window,
        pixel_size: request.pixel_size,
        settings_revision: request.settings_revision,
        presentation_icon: request.presentation_icon.clone(),
        custom_image_path: request.custom_image_path.clone(),
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
            if is_shared_host_executable(&executable_path.to_string_lossy()) {
                window_icon(window, pixel_size).or_else(|| {
                    source_icon(native_icons, fallback_path.as_ref(), pixel_size)
                })
            } else {
                source_icon(native_icons, fallback_path.as_ref(), pixel_size)
                    .or_else(|| window_icon(window, pixel_size))
            }
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

fn publish(
    results: Vec<IconHydrationResult>,
    shared: &SharedState,
    consumer: Consumer,
    epoch: u64,
) {
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
    let mut state = lock(&shared.state);
    if state.epochs[consumer as usize] != epoch {
        return;
    }
    for result in results {
        state
            .results
            .retain(|existing| !same_request(existing, &result));
        if state.results.len() == MAX_PENDING_RESULTS {
            let _discarded = state.results.pop_front();
        }
        state.results.push_back(result);
    }
    drop(state);
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

fn same_request(left: &IconHydrationResult, right: &IconHydrationResult) -> bool {
    match (left, right) {
        (IconHydrationResult::Launcher(left), IconHydrationResult::Launcher(right)) => {
            left.generation == right.generation
                && left.identity == right.identity
                && left.pixel_size == right.pixel_size
                && left.settings_revision == right.settings_revision
        }
        (IconHydrationResult::Switcher(left), IconHydrationResult::Switcher(right)) => {
            left.generation == right.generation
                && left.window == right.window
                && left.pixel_size == right.pixel_size
                && left.settings_revision == right.settings_revision
        }
        (IconHydrationResult::Dock(left), IconHydrationResult::Dock(right)) => {
            left.identity == right.identity
                && left.window == right.window
                && left.executable_path == right.executable_path
                && left.presentation_icon == right.presentation_icon
                && left.custom_image_path == right.custom_image_path
                && left.pixel_size == right.pixel_size
        }
        (IconHydrationResult::Settings(left), IconHydrationResult::Settings(right)) => {
            left.identity == right.identity
                && left.icon_source == right.icon_source
                && left.custom_image_path == right.custom_image_path
                && left.pixel_size == right.pixel_size
                && left.settings_revision == right.settings_revision
        }
        _ => false,
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
