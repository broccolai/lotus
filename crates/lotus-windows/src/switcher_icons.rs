use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread::{self, JoinHandle};

use lotus_core::window::WindowId;
use lotus_ui::icon::RasterIcon;
use thiserror::Error;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;

use crate::custom_image::CustomImageCache;
use crate::messages::SWITCHER_ICON_WAKE;
use crate::native_icon::NativeIconCache;
use crate::responsiveness::METRICS;

const REQUEST_QUEUE_CAPACITY: usize = 1;

#[derive(Clone, Debug)]
pub struct SwitcherIconRequest {
    pub generation: u64,
    pub window: WindowId,
    pub executable_path: PathBuf,
    pub custom_image_path: Option<PathBuf>,
    pub pixel_size: u32,
    pub settings_revision: u64,
}

#[derive(Clone, Debug)]
pub struct HydratedSwitcherIcon {
    pub generation: u64,
    pub window: WindowId,
    pub pixel_size: u32,
    pub settings_revision: u64,
    pub icon: Option<RasterIcon>,
}

#[derive(Debug, Error)]
pub enum SwitcherIconHydratorError {
    #[error("Lotus could not create its switcher icon worker: {0}")]
    Thread(#[from] std::io::Error),
}

pub struct SwitcherIconHydrator {
    sender: Option<mpsc::SyncSender<Vec<SwitcherIconRequest>>>,
    shared: Arc<SharedState>,
    worker: Option<JoinHandle<()>>,
}

struct SharedState {
    pending: Mutex<Option<Vec<SwitcherIconRequest>>>,
    results: Mutex<Vec<HydratedSwitcherIcon>>,
    wake_queued: AtomicBool,
    stopping: AtomicBool,
    owner_thread: u32,
}

impl SwitcherIconHydrator {
    pub fn start() -> Result<Self, SwitcherIconHydratorError> {
        let shared = Arc::new(SharedState {
            pending: Mutex::new(None),
            results: Mutex::new(Vec::new()),
            wake_queued: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            owner_thread: unsafe { GetCurrentThreadId() },
        });
        let (sender, receiver) = mpsc::sync_channel(REQUEST_QUEUE_CAPACITY);
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("lotus-switcher-icons".to_owned())
            .spawn(move || hydrate_icons(&receiver, &worker_shared))?;

        Ok(Self {
            sender: Some(sender),
            shared,
            worker: Some(worker),
        })
    }

    pub fn request(&self, requests: Vec<SwitcherIconRequest>) {
        if requests.is_empty() || self.shared.stopping.load(Ordering::Acquire) {
            return;
        }
        METRICS.record_switcher_requests(requests.len());
        let Some(sender) = &self.sender else {
            return;
        };

        match sender.try_send(requests) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(requests)) => {
                *lock(&self.shared.pending) = Some(requests);
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.shared.stopping.store(true, Ordering::Release);
            }
        }
    }

    pub fn drain(&self) -> Vec<HydratedSwitcherIcon> {
        self.shared.wake_queued.store(false, Ordering::Release);
        let mut results = lock(&self.shared.results);
        std::mem::take(&mut *results)
    }
}

impl Drop for SwitcherIconHydrator {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::Release);
        self.sender.take();

        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub const fn is_switcher_icon_wake(message: u32) -> bool {
    message == SWITCHER_ICON_WAKE
}

fn hydrate_icons(
    receiver: &mpsc::Receiver<Vec<SwitcherIconRequest>>,
    shared: &SharedState,
) {
    let mut native_icons = NativeIconCache::default();
    let mut custom_images = CustomImageCache::default();

    while !shared.stopping.load(Ordering::Acquire) {
        let Some(requests) = next_requests(receiver, shared) else {
            break;
        };
        let results = requests
            .into_iter()
            .map(|request| hydrate_icon(&request, &mut native_icons, &mut custom_images))
            .collect::<Vec<_>>();
        publish(results, shared);
    }
}

fn next_requests(
    receiver: &mpsc::Receiver<Vec<SwitcherIconRequest>>,
    shared: &SharedState,
) -> Option<Vec<SwitcherIconRequest>> {
    if let Some(pending) = lock(&shared.pending).take() {
        return Some(pending);
    }

    receiver.recv().ok()
}

fn hydrate_icon(
    request: &SwitcherIconRequest,
    native_icons: &mut NativeIconCache,
    custom_images: &mut CustomImageCache,
) -> HydratedSwitcherIcon {
    let icon = request
        .custom_image_path
        .as_deref()
        .and_then(|path| custom_images.image(path).ok())
        .or_else(|| {
            native_icons
                .icon(&request.executable_path, request.pixel_size)
                .ok()
                .flatten()
        });

    HydratedSwitcherIcon {
        generation: request.generation,
        window: request.window,
        pixel_size: request.pixel_size,
        settings_revision: request.settings_revision,
        icon,
    }
}

fn publish(results: Vec<HydratedSwitcherIcon>, shared: &SharedState) {
    if results.is_empty() || shared.stopping.load(Ordering::Acquire) {
        return;
    }
    METRICS.record_switcher_results(results.len());
    lock(&shared.results).extend(results);

    if !shared.wake_queued.swap(true, Ordering::AcqRel)
        && unsafe {
            PostThreadMessageW(
                shared.owner_thread,
                SWITCHER_ICON_WAKE,
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
