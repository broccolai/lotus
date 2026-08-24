mod enumeration;
mod events;
mod foreground;

use std::cmp::Ordering;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use lotus_core::window::{TrackedWindowKey, WindowId, WindowInfo};
use windows::Win32::Foundation::{E_FAIL, HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, GetWindowThreadProcessId, IsWindow, KillTimer, MSG,
    PM_NOREMOVE, PeekMessageW, PostThreadMessageW, SetTimer, TranslateMessage, WM_QUIT,
    WM_TIMER,
};
use windows::core::Error;

pub(crate) use self::enumeration::process_image_path;
use crate::responsiveness::METRICS;
use crate::{NativeError, WindowHandle};

const MAX_RECONCILE_INTERVAL_MS: u32 = 30_000;
const STALE_TARGET_TOMBSTONE_LIFETIME: Duration = Duration::from_secs(2);

static STALE_TARGETS: std::sync::LazyLock<Mutex<Vec<(TrackedWindowKey, Instant)>>> =
    std::sync::LazyLock::new(|| Mutex::new(Vec::new()));
static TRACKED_WINDOWS: std::sync::LazyLock<Mutex<TrackedWindowRegistry>> =
    std::sync::LazyLock::new(|| Mutex::new(TrackedWindowRegistry::default()));

#[derive(Default)]
struct TrackedWindowRegistry {
    next_incarnation: u64,
    windows: Vec<TrackedWindowKey>,
}

pub(crate) struct CurrentTrackedWindow {
    _registry: std::sync::MutexGuard<'static, TrackedWindowRegistry>,
}

pub(crate) fn hold_current_tracked_window(
    key: TrackedWindowKey,
) -> Option<CurrentTrackedWindow> {
    let registry = TRACKED_WINDOWS.lock().ok()?;
    registry.contains(key).then_some(CurrentTrackedWindow {
        _registry: registry,
    })
}

impl TrackedWindowRegistry {
    fn assign(&mut self, windows: &mut [WindowInfo]) {
        self.windows
            .retain(|key| windows.iter().any(|window| window.id == key.id));
        for window in windows {
            let key = self
                .windows
                .iter()
                .find(|key| key.id == window.id && key.process_id == window.process_id)
                .copied()
                .unwrap_or_else(|| {
                    self.next_incarnation = self.next_incarnation.wrapping_add(1).max(1);
                    let key = TrackedWindowKey {
                        id: window.id,
                        process_id: window.process_id,
                        incarnation: self.next_incarnation,
                    };
                    self.windows.retain(|candidate| candidate.id != key.id);
                    self.windows.push(key);
                    key
                });
            window.incarnation = key.incarnation;
        }
    }

    fn contains(&self, key: TrackedWindowKey) -> bool {
        self.windows.contains(&key)
    }

    fn retire(&mut self, key: TrackedWindowKey) {
        self.windows.retain(|candidate| *candidate != key);
    }
}

pub(crate) fn is_live_tracked_window(key: TrackedWindowKey) -> bool {
    with_live_tracked_window(key, |_| ()).is_some()
}

pub(crate) fn with_live_tracked_window<T>(
    key: TrackedWindowKey,
    operation: impl FnOnce(HWND) -> T,
) -> Option<T> {
    let Ok(address) = usize::try_from(key.id.get()) else {
        return None;
    };
    if address == 0 {
        return None;
    }
    let _current = hold_current_tracked_window(key)?;
    let hwnd = HWND(std::ptr::with_exposed_provenance_mut::<c_void>(address));
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return None;
    }
    let mut process_id = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut process_id)) };
    (process_id == key.process_id).then(|| operation(hwnd))
}

pub(crate) fn report_stale_target(key: TrackedWindowKey) {
    if let Ok(mut registry) = TRACKED_WINDOWS.lock() {
        registry.retire(key);
    }
    if let Ok(mut stale) = STALE_TARGETS.lock() {
        tombstone_target(&mut stale, key, Instant::now());
    }

    events::request_immediate_reconcile();
}

fn tombstone_target(
    stale: &mut Vec<(TrackedWindowKey, Instant)>,
    key: TrackedWindowKey,
    now: Instant,
) {
    stale.retain(|(_, expires)| *expires > now);
    if !stale.iter().any(|(candidate, _)| *candidate == key) {
        stale.push((key, now + STALE_TARGET_TOMBSTONE_LIFETIME));
    }
}

pub struct WindowTracker {
    shared: Arc<SharedState>,
    worker: Option<JoinHandle<()>>,
    windows: Arc<[WindowInfo]>,
    own_process_id: u32,
    fullscreen_window: Option<WindowId>,
    window_revision: u64,
    fullscreen_revision: u64,
    presentation_revision: u64,
    shell_fullscreen_window: Option<WindowId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowTrackerEvent {
    SnapshotRefreshed,
    FullscreenRefreshed,
}

struct SharedState {
    latest: Mutex<PublishedSnapshot>,
    ui_thread: u32,
    worker_thread: std::sync::atomic::AtomicU32,
    ui_wake_queued: AtomicBool,
    running: AtomicBool,
}

#[derive(Clone)]
struct PublishedSnapshot {
    windows: Arc<[WindowInfo]>,
    fullscreen_window: Option<WindowId>,
    window_revision: u64,
    fullscreen_revision: u64,
}

impl Default for PublishedSnapshot {
    fn default() -> Self {
        Self {
            windows: Arc::from([]),
            fullscreen_window: None,
            window_revision: 0,
            fullscreen_revision: 0,
        }
    }
}

struct WorkerState {
    own_process_id: u32,
    shared: Arc<SharedState>,
    windows: Arc<[WindowInfo]>,
    fullscreen_window: Option<WindowId>,
    window_revision: u64,
    fullscreen_revision: u64,
    failures: u32,
    debounce_timer: Option<usize>,
    reconcile_timer: usize,
    process_cache: enumeration::ProcessMetadataCache,
}

impl WindowTracker {
    pub fn start() -> Result<Self, NativeError> {
        let (own_process_id, ui_thread) =
            unsafe { (GetCurrentProcessId(), GetCurrentThreadId()) };
        let shared = Arc::new(SharedState {
            latest: Mutex::new(PublishedSnapshot::default()),
            ui_thread,
            worker_thread: std::sync::atomic::AtomicU32::new(0),
            ui_wake_queued: AtomicBool::new(false),
            running: AtomicBool::new(true),
        });
        let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("lotus-window-tracker".to_owned())
            .spawn(move || run_worker(own_process_id, worker_shared, startup_sender))
            .map_err(|error| Error::new(E_FAIL, error.to_string()))?;

        match startup_receiver.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(_)) => {}
            Ok(Err(message)) => {
                shared.running.store(false, AtomicOrdering::Release);
                let _ = worker.join();
                return Err(Error::new(E_FAIL, message).into());
            }
            Err(_) => {
                shared.running.store(false, AtomicOrdering::Release);
                request_worker_stop(&shared);
                return Err(Error::new(E_FAIL, "Lotus window tracker did not start").into());
            }
        }
        let snapshot = {
            let latest = shared.latest.lock().map_err(|_| {
                Error::new(E_FAIL, "Lotus window tracker state is unavailable")
            })?;
            shared.ui_wake_queued.store(false, AtomicOrdering::Release);
            latest.clone()
        };

        Ok(Self {
            shared,
            worker: Some(worker),
            windows: snapshot.windows,
            own_process_id,
            fullscreen_window: snapshot.fullscreen_window,
            window_revision: snapshot.window_revision,
            fullscreen_revision: snapshot.fullscreen_revision,
            presentation_revision: 0,
            shell_fullscreen_window: None,
        })
    }

    pub fn current_windows(&self) -> &[WindowInfo] {
        self.windows.as_ref()
    }

    pub const fn fullscreen_window(&self) -> Option<WindowId> {
        match self.shell_fullscreen_window {
            Some(window) => Some(window),
            None => self.fullscreen_window,
        }
    }

    pub fn fullscreen_on_same_monitor(&self, window: WindowHandle) -> bool {
        [self.shell_fullscreen_window, self.fullscreen_window]
            .into_iter()
            .flatten()
            .filter_map(foreground::hwnd_from_window_id)
            .any(|fullscreen| foreground::same_monitor(window.raw(), fullscreen))
    }

    pub fn set_shell_fullscreen(&mut self, fullscreen: bool) {
        if fullscreen {
            self.shell_fullscreen_window =
                foreground::observe_foreground_window(self.own_process_id);
            if self.shell_fullscreen_window.is_none() {
                self.fullscreen_window =
                    foreground::observe_fullscreen_window(self.own_process_id);
            }
            self.presentation_revision = self.presentation_revision.wrapping_add(1);
            return;
        }

        let ended_window = self.shell_fullscreen_window.take();
        let foreground = foreground::observe_foreground_window(self.own_process_id);
        self.fullscreen_window = if foreground == ended_window {
            None
        } else {
            foreground::observe_fullscreen_window(self.own_process_id)
        };
        self.presentation_revision = self.presentation_revision.wrapping_add(1);
    }

    pub const fn presentation_revision(&self) -> u64 {
        self.presentation_revision
    }

    pub fn handle_message(
        &mut self,
        is_thread_message: bool,
        message_id: u32,
        _parameter: usize,
    ) -> Result<Option<WindowTrackerEvent>, NativeError> {
        if !is_thread_message || message_id != events::REFRESH_MESSAGE {
            return Ok(None);
        }
        if !self
            .shared
            .ui_wake_queued
            .swap(false, AtomicOrdering::AcqRel)
        {
            return Ok(None);
        }

        let snapshot = self
            .shared
            .latest
            .lock()
            .map_err(|_| Error::new(E_FAIL, "Lotus window tracker state is unavailable"))?
            .clone();
        let event = observe_published_snapshot(
            &mut self.windows,
            &mut self.fullscreen_window,
            &mut self.window_revision,
            &mut self.fullscreen_revision,
            snapshot,
        );
        if event.is_some() {
            self.presentation_revision = self.presentation_revision.wrapping_add(1);
        }

        Ok(event)
    }

    pub fn refresh_fullscreen(&mut self) {
        self.validate_shell_fullscreen();
        self.fullscreen_window = foreground::observe_fullscreen_window(self.own_process_id);
    }

    fn validate_shell_fullscreen(&mut self) {
        if self
            .shell_fullscreen_window
            .is_some_and(|window| !foreground::is_fullscreen_window(window))
        {
            self.shell_fullscreen_window = None;
        }
    }
}

fn run_worker(
    own_process_id: u32,
    shared: Arc<SharedState>,
    startup: mpsc::SyncSender<Result<PublishedSnapshot, String>>,
) {
    let mut message = MSG::default();
    let _ = unsafe { PeekMessageW(&raw mut message, None, 0, 0, PM_NOREMOVE) };
    let thread_id = unsafe { GetCurrentThreadId() };
    if !events::claim_callback_thread(thread_id) {
        let _ = startup.send(Err("a Lotus window tracker is already active".to_owned()));
        drop(startup);
        return;
    }
    shared
        .worker_thread
        .store(thread_id, AtomicOrdering::Release);
    let hooks = match events::install_hooks() {
        Ok(hooks) => hooks,
        Err(error) => {
            events::release_callback_thread(thread_id);
            shared.worker_thread.store(0, AtomicOrdering::Release);
            let _ = startup.send(Err(error.to_string()));
            drop(startup);
            return;
        }
    };
    let reconcile_timer = create_thread_timer(events::RECONCILE_INTERVAL_MS);
    if reconcile_timer == 0 {
        events::release_callback_thread(thread_id);
        shared.worker_thread.store(0, AtomicOrdering::Release);
        drop(hooks);
        let _ = startup.send(Err(Error::from_thread().to_string()));
        drop(startup);
        return;
    }

    let mut state = WorkerState {
        own_process_id,
        shared,
        windows: Arc::from([]),
        fullscreen_window: None,
        window_revision: 0,
        fullscreen_revision: 0,
        failures: 0,
        debounce_timer: None,
        reconcile_timer,
        process_cache: enumeration::ProcessMetadataCache::default(),
    };
    state.refresh();
    if !state.shared.running.load(AtomicOrdering::Acquire) {
        state.cancel_timers();
        events::release_callback_thread(thread_id);
        state.shared.worker_thread.store(0, AtomicOrdering::Release);
        drop(hooks);
        let _ = startup.send(Err(
            "Lotus window tracker was stopped during startup".to_owned()
        ));
        drop(startup);
        return;
    }
    let _ = startup.send(Ok(state.snapshot()));
    drop(startup);
    events::activate_callbacks();

    worker_message_loop(&mut state);
    events::release_callback_thread(thread_id);
    state.shared.worker_thread.store(0, AtomicOrdering::Release);
    state.cancel_timers();
    drop(hooks);
}

fn worker_message_loop(state: &mut WorkerState) {
    let mut message = MSG::default();
    while unsafe { GetMessageW(&raw mut message, None, 0, 0) }.as_bool() {
        match message.message {
            events::REFRESH_MESSAGE => state.restart_debounce_timer(),
            WM_TIMER if state.debounce_timer == Some(message.wParam.0) => {
                state.cancel_debounce_timer();
                events::clear_refresh_notification();
                state.refresh();
            }
            WM_TIMER if message.wParam.0 == state.reconcile_timer => state.refresh(),
            events::IMMEDIATE_RECONCILE_MESSAGE => state.refresh(),
            events::SHUTDOWN_MESSAGE | WM_QUIT => break,
            _ => unsafe {
                let _ = TranslateMessage(&raw const message);
                DispatchMessageW(&raw const message);
            },
        }
    }
}

impl WorkerState {
    fn refresh(&mut self) {
        let enumeration_started = Instant::now();
        let Ok(mut windows) =
            enumeration::enumerate_windows(self.own_process_id, &mut self.process_cache)
        else {
            METRICS.record_window_enumeration(enumeration_started.elapsed());
            self.failures = self.failures.saturating_add(1);
            self.reschedule_reconcile();
            return;
        };
        assign_window_incarnations(&mut windows);
        suppress_stale_targets(&mut windows, Instant::now());
        METRICS.record_window_enumeration(enumeration_started.elapsed());

        self.failures = 0;
        self.reschedule_reconcile();
        let fullscreen_window = foreground::observe_fullscreen_window(self.own_process_id);
        let windows_changed = !same_window_snapshot(&self.windows, &windows);
        let fullscreen_changed = self.fullscreen_window != fullscreen_window;
        if !windows_changed && !fullscreen_changed {
            METRICS.record_window_unchanged();
            return;
        }

        if windows_changed {
            self.windows = Arc::from(windows);
            self.window_revision = self.window_revision.wrapping_add(1);
        }
        self.fullscreen_window = fullscreen_window;
        if fullscreen_changed {
            self.fullscreen_revision = self.fullscreen_revision.wrapping_add(1);
        }
        self.publish();
    }

    fn snapshot(&self) -> PublishedSnapshot {
        PublishedSnapshot {
            windows: Arc::clone(&self.windows),
            fullscreen_window: self.fullscreen_window,
            window_revision: self.window_revision,
            fullscreen_revision: self.fullscreen_revision,
        }
    }

    fn publish(&mut self) {
        METRICS.record_window_publish();
        let Ok(mut stale) = STALE_TARGETS.lock() else {
            return;
        };
        stale.retain(|(_, expires)| *expires > Instant::now());
        let snapshot = publish_pending_snapshot(
            &mut self.windows,
            self.fullscreen_window,
            &mut self.window_revision,
            self.fullscreen_revision,
            &stale,
        );
        let Ok(mut latest) = self.shared.latest.lock() else {
            return;
        };
        *latest = snapshot;
        drop(latest);
        drop(stale);

        if self.shared.running.load(AtomicOrdering::Acquire)
            && !self
                .shared
                .ui_wake_queued
                .swap(true, AtomicOrdering::AcqRel)
            && unsafe {
                PostThreadMessageW(
                    self.shared.ui_thread,
                    events::REFRESH_MESSAGE,
                    WPARAM(0),
                    LPARAM(0),
                )
            }
            .is_err()
        {
            self.shared
                .ui_wake_queued
                .store(false, AtomicOrdering::Release);
        }
    }

    fn restart_debounce_timer(&mut self) {
        let timer = create_thread_timer(events::REFRESH_DELAY_MS);
        if timer == 0 {
            return;
        }
        self.cancel_debounce_timer();
        self.debounce_timer = Some(timer);
    }

    fn cancel_debounce_timer(&mut self) {
        if let Some(timer) = self.debounce_timer.take() {
            cancel_timer(timer);
        }
    }

    fn reschedule_reconcile(&mut self) {
        let multiplier = 1_u32.checked_shl(self.failures.min(4)).unwrap_or(u32::MAX);
        let interval = events::RECONCILE_INTERVAL_MS
            .saturating_mul(multiplier)
            .min(MAX_RECONCILE_INTERVAL_MS);
        let timer = create_thread_timer(interval);
        if timer == 0 {
            return;
        }
        cancel_timer(self.reconcile_timer);
        self.reconcile_timer = timer;
    }

    fn cancel_timers(&mut self) {
        self.cancel_debounce_timer();
        cancel_timer(self.reconcile_timer);
    }
}

fn assign_window_incarnations(windows: &mut [WindowInfo]) {
    if let Ok(mut registry) = TRACKED_WINDOWS.lock() {
        registry.assign(windows);
    }
}

fn suppress_stale_targets(windows: &mut Vec<WindowInfo>, now: Instant) {
    let Ok(mut stale) = STALE_TARGETS.lock() else {
        return;
    };
    stale.retain(|(_, expires)| *expires > now);
    filter_stale_windows(windows, &stale);
}

fn filter_stale_windows(
    windows: &mut Vec<WindowInfo>,
    stale: &[(TrackedWindowKey, Instant)],
) {
    windows.retain(|window| !stale.iter().any(|(key, _)| *key == window.key()));
}

fn snapshot_for_publication(
    windows: &Arc<[WindowInfo]>,
    fullscreen_window: Option<WindowId>,
    window_revision: u64,
    fullscreen_revision: u64,
    stale: &[(TrackedWindowKey, Instant)],
) -> PublishedSnapshot {
    let mut published_windows = windows.as_ref().to_vec();
    filter_stale_windows(&mut published_windows, stale);
    let filtered = published_windows.as_slice() != windows.as_ref();
    PublishedSnapshot {
        windows: Arc::from(published_windows),
        fullscreen_window,
        window_revision: window_revision.wrapping_add(u64::from(filtered)),
        fullscreen_revision,
    }
}

fn publish_pending_snapshot(
    windows: &mut Arc<[WindowInfo]>,
    fullscreen_window: Option<WindowId>,
    window_revision: &mut u64,
    fullscreen_revision: u64,
    stale: &[(TrackedWindowKey, Instant)],
) -> PublishedSnapshot {
    let snapshot = snapshot_for_publication(
        windows,
        fullscreen_window,
        *window_revision,
        fullscreen_revision,
        stale,
    );
    if snapshot.windows != *windows {
        *windows = Arc::clone(&snapshot.windows);
        *window_revision = snapshot.window_revision;
    }
    snapshot
}

fn observe_published_snapshot(
    windows: &mut Arc<[WindowInfo]>,
    fullscreen_window: &mut Option<WindowId>,
    window_revision: &mut u64,
    fullscreen_revision: &mut u64,
    snapshot: PublishedSnapshot,
) -> Option<WindowTrackerEvent> {
    let windows_changed = *window_revision != snapshot.window_revision;
    let fullscreen_changed = *fullscreen_revision != snapshot.fullscreen_revision;
    *windows = snapshot.windows;
    *fullscreen_window = snapshot.fullscreen_window;
    *window_revision = snapshot.window_revision;
    *fullscreen_revision = snapshot.fullscreen_revision;

    windows_changed
        .then_some(WindowTrackerEvent::SnapshotRefreshed)
        .or_else(|| fullscreen_changed.then_some(WindowTrackerEvent::FullscreenRefreshed))
}

fn create_thread_timer(interval: u32) -> usize {
    unsafe { SetTimer(None, 0, interval, None) }
}

fn cancel_timer(timer_id: usize) {
    let _ = unsafe { KillTimer(None, timer_id) };
}

fn request_worker_stop(shared: &SharedState) {
    let thread_id = shared.worker_thread.load(AtomicOrdering::Acquire);
    if thread_id != 0 {
        let _ = unsafe {
            PostThreadMessageW(thread_id, events::SHUTDOWN_MESSAGE, WPARAM(0), LPARAM(0))
        };
    }
}

fn same_window_snapshot(previous: &[WindowInfo], current: &[WindowInfo]) -> bool {
    if previous.len() != current.len() {
        return false;
    }

    let mut sorted_current = current.iter().collect::<Vec<_>>();
    sorted_current.sort_unstable_by(|left, right| compare_window_info(left, right));

    previous.iter().all(|window| {
        sorted_current
            .binary_search_by(|candidate| compare_window_info(candidate, window))
            .is_ok()
    })
}

fn compare_window_info(left: &WindowInfo, right: &WindowInfo) -> Ordering {
    left.id
        .cmp(&right.id)
        .then_with(|| left.process_id.cmp(&right.process_id))
        .then_with(|| left.incarnation.cmp(&right.incarnation))
        .then_with(|| left.title.cmp(&right.title))
        .then_with(|| left.executable_path.cmp(&right.executable_path))
        .then_with(|| left.app_user_model_id.cmp(&right.app_user_model_id))
}

impl Drop for WindowTracker {
    fn drop(&mut self) {
        self.shared.running.store(false, AtomicOrdering::Release);
        request_worker_stop(&self.shared);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
