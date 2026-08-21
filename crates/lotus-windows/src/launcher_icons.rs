use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread::{self, JoinHandle};

use lotus_ui::icon::RasterIcon;
use thiserror::Error;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;

use crate::custom_image::CustomImageCache;
use crate::launch::ComApartment;
use crate::messages::LAUNCHER_ICON_WAKE;
use crate::native_icon::NativeIconCache;

const REQUEST_QUEUE_CAPACITY: usize = 1;

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
pub struct HydratedLauncherIcon {
    pub generation: u64,
    pub identity: String,
    pub pixel_size: u32,
    pub settings_revision: u64,
    pub icon: Option<RasterIcon>,
}

#[derive(Debug, Error)]
pub enum LauncherIconHydratorError {
    #[error("Lotus could not create its launcher icon worker: {0}")]
    Thread(#[from] std::io::Error),
}

pub struct LauncherIconHydrator {
    sender: Option<mpsc::SyncSender<Vec<LauncherIconRequest>>>,
    shared: Arc<SharedState>,
    worker: Option<JoinHandle<()>>,
}

struct SharedState {
    pending: Mutex<Option<Vec<LauncherIconRequest>>>,
    results: Mutex<Vec<HydratedLauncherIcon>>,
    wake_queued: AtomicBool,
    stopping: AtomicBool,
    owner_thread: u32,
}

impl LauncherIconHydrator {
    pub fn start() -> Result<Self, LauncherIconHydratorError> {
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
            .name("lotus-launcher-icons".to_owned())
            .spawn(move || hydrate_icons(&receiver, &worker_shared))?;

        Ok(Self {
            sender: Some(sender),
            shared,
            worker: Some(worker),
        })
    }

    pub fn request(&self, requests: Vec<LauncherIconRequest>) {
        if requests.is_empty() || self.shared.stopping.load(Ordering::Acquire) {
            return;
        }
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

    pub fn drain(&self) -> Vec<HydratedLauncherIcon> {
        self.shared.wake_queued.store(false, Ordering::Release);
        let mut results = lock(&self.shared.results);
        std::mem::take(&mut *results)
    }
}

impl Drop for LauncherIconHydrator {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::Release);
        self.sender.take();

        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub const fn is_launcher_icon_wake(message: u32) -> bool {
    message == LAUNCHER_ICON_WAKE
}

fn hydrate_icons(
    receiver: &mpsc::Receiver<Vec<LauncherIconRequest>>,
    shared: &SharedState,
) {
    let _apartment = ComApartment::enter();
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
    receiver: &mpsc::Receiver<Vec<LauncherIconRequest>>,
    shared: &SharedState,
) -> Option<Vec<LauncherIconRequest>> {
    if let Some(pending) = lock(&shared.pending).take() {
        return Some(pending);
    }

    receiver.recv().ok()
}

fn hydrate_icon(
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

fn publish(results: Vec<HydratedLauncherIcon>, shared: &SharedState) {
    if results.is_empty() || shared.stopping.load(Ordering::Acquire) {
        return;
    }
    lock(&shared.results).extend(results);

    if !shared.wake_queued.swap(true, Ordering::AcqRel)
        && unsafe {
            PostThreadMessageW(
                shared.owner_thread,
                LAUNCHER_ICON_WAKE,
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
