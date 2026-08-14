use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use image::imageops::FilterType;
use lotus_media::{MediaControls, MediaSnapshot, PlaybackState};
use lotus_ui::icon::{RasterIcon, RasterIconError};
use thiserror::Error;
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession,
    GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};
use windows::Storage::Streams::DataReader;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_APP};
use windows::core::Error as WindowsError;

const MEDIA_WAKE_MESSAGE: u32 = WM_APP + 0x4C9;
const MAX_ARTWORK_BYTES: u64 = 8 * 1024 * 1024;
const ARTWORK_SAMPLE_SIZE: u32 = 160;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaCommand {
    Previous,
    Play,
    Pause,
    Next,
}

#[derive(Debug)]
pub enum MediaEvent {
    Snapshot(Option<MediaSnapshot>),
    Unavailable(String),
}

#[derive(Debug, Error)]
pub enum MediaControllerError {
    #[error("Lotus could not start its media worker: {0}")]
    Start(#[from] std::io::Error),
    #[error("the Lotus media worker is no longer available")]
    Disconnected,
}

pub struct MediaController {
    commands: Sender<WorkerCommand>,
    events: Receiver<MediaEvent>,
    worker: Option<JoinHandle<()>>,
}

impl MediaController {
    pub fn start() -> Result<Self, MediaControllerError> {
        let owner_thread = current_thread_id();
        let (command_sender, command_receiver) = mpsc::channel();
        let (event_sender, event_receiver) = mpsc::channel();
        let refresh_pending = Arc::new(AtomicBool::new(true));
        let callback_sender = command_sender.clone();
        let callback_pending = Arc::clone(&refresh_pending);
        let worker = thread::Builder::new()
            .name("lotus-media-controls".into())
            .spawn(move || {
                run_worker(
                    owner_thread,
                    &command_receiver,
                    &callback_sender,
                    &event_sender,
                    &callback_pending,
                );
            })?;

        Ok(Self {
            commands: command_sender,
            events: event_receiver,
            worker: Some(worker),
        })
    }

    pub fn execute(&self, command: MediaCommand) -> Result<(), MediaControllerError> {
        self.commands
            .send(WorkerCommand::Execute(command))
            .map_err(|_| MediaControllerError::Disconnected)
    }

    pub fn drain_events(&self) -> impl Iterator<Item = MediaEvent> + '_ {
        self.events.try_iter()
    }
}

impl Drop for MediaController {
    fn drop(&mut self) {
        let _ = self.commands.send(WorkerCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub const fn is_media_wake(message: u32) -> bool {
    message == MEDIA_WAKE_MESSAGE
}

pub fn decode_artwork(
    source_id: &str,
    encoded: &[u8],
) -> Result<RasterIcon, MediaArtworkError> {
    let image = image::load_from_memory(encoded)?
        .resize_to_fill(
            ARTWORK_SAMPLE_SIZE,
            ARTWORK_SAMPLE_SIZE,
            FilterType::Lanczos3,
        )
        .to_rgba8();
    let mut pixels = image.into_raw();
    premultiply_rgba_to_bgra(&mut pixels);
    let mut hasher = DefaultHasher::new();
    source_id.hash(&mut hasher);
    encoded.hash(&mut hasher);
    let identity = format!("media:{:016x}", hasher.finish());
    RasterIcon::new(identity, ARTWORK_SAMPLE_SIZE, ARTWORK_SAMPLE_SIZE, pixels)
        .map_err(Into::into)
}

#[derive(Debug, Error)]
pub enum MediaArtworkError {
    #[error("the media artwork could not be decoded: {0}")]
    Image(#[from] image::ImageError),
    #[error(transparent)]
    Raster(#[from] RasterIconError),
}

enum WorkerCommand {
    Refresh,
    Execute(MediaCommand),
    Shutdown,
}

fn run_worker(
    owner_thread: u32,
    commands: &Receiver<WorkerCommand>,
    callback_sender: &Sender<WorkerCommand>,
    events: &Sender<MediaEvent>,
    refresh_pending: &Arc<AtomicBool>,
) {
    let mut media =
        match MediaWorker::start(callback_sender.clone(), Arc::clone(refresh_pending)) {
            Ok(media) => media,
            Err(error) => {
                publish_event(
                    owner_thread,
                    events,
                    MediaEvent::Unavailable(error.to_string()),
                );
                return;
            }
        };

    refresh_pending.store(false, Ordering::Release);
    media.refresh(owner_thread, events);

    while let Ok(command) = commands.recv() {
        match command {
            WorkerCommand::Refresh => {
                refresh_pending.store(false, Ordering::Release);
                media.refresh(owner_thread, events);
            }
            WorkerCommand::Execute(command) => media.execute(command),
            WorkerCommand::Shutdown => break,
        }
    }
}

fn publish_event(owner_thread: u32, events: &Sender<MediaEvent>, event: MediaEvent) {
    if events.send(event).is_err() {
        return;
    }

    // SAFETY: The captured id belongs to the Lotus UI thread. This message carries no pointers.
    let _ = unsafe {
        PostThreadMessageW(owner_thread, MEDIA_WAKE_MESSAGE, WPARAM(0), LPARAM(0))
    };
}

struct MediaWorker {
    _apartment: WinRtApartment,
    manager: GlobalSystemMediaTransportControlsSessionManager,
    _manager_events: ManagerEvents,
    session_events: Option<SessionEvents>,
    callback: RefreshCallback,
}

impl MediaWorker {
    fn start(
        callback_sender: Sender<WorkerCommand>,
        refresh_pending: Arc<AtomicBool>,
    ) -> Result<Self, WindowsError> {
        let apartment = WinRtApartment::enter()?;
        let manager =
            GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.join()?;
        let callback = RefreshCallback {
            sender: callback_sender,
            pending: refresh_pending,
        };
        let manager_events = ManagerEvents::subscribe(manager.clone(), callback.clone())?;

        Ok(Self {
            _apartment: apartment,
            manager,
            _manager_events: manager_events,
            session_events: None,
            callback,
        })
    }

    fn refresh(&mut self, owner_thread: u32, events: &Sender<MediaEvent>) {
        let session = self.manager.GetCurrentSession().ok();
        let session_changed = self
            .session_events
            .as_ref()
            .is_none_or(|bound| Some(&bound.session) != session.as_ref());

        if session_changed {
            self.session_events = session.as_ref().and_then(|session| {
                SessionEvents::subscribe(session.clone(), self.callback.clone()).ok()
            });
        }

        let snapshot = session.as_ref().and_then(read_snapshot);
        publish_event(owner_thread, events, MediaEvent::Snapshot(snapshot));
    }

    fn execute(&self, command: MediaCommand) {
        let Ok(session) = self.manager.GetCurrentSession() else {
            return;
        };

        let result = match command {
            MediaCommand::Previous => session.TrySkipPreviousAsync(),
            MediaCommand::Play => session.TryPlayAsync(),
            MediaCommand::Pause => session.TryPauseAsync(),
            MediaCommand::Next => session.TrySkipNextAsync(),
        };
        if let Ok(operation) = result {
            let _ = operation.join();
        }
    }
}

#[derive(Clone)]
struct RefreshCallback {
    sender: Sender<WorkerCommand>,
    pending: Arc<AtomicBool>,
}

impl RefreshCallback {
    fn request(&self) {
        if self.pending.swap(true, Ordering::AcqRel) {
            return;
        }
        if self.sender.send(WorkerCommand::Refresh).is_err() {
            self.pending.store(false, Ordering::Release);
        }
    }
}

struct ManagerEvents {
    manager: GlobalSystemMediaTransportControlsSessionManager,
    current_session: i64,
    sessions: i64,
}

impl ManagerEvents {
    fn subscribe(
        manager: GlobalSystemMediaTransportControlsSessionManager,
        callback: RefreshCallback,
    ) -> Result<Self, WindowsError> {
        let current_callback = callback.clone();
        let current_handler = TypedEventHandler::new(move |_, _| {
            current_callback.request();
            Ok(())
        });
        let current_session = manager.CurrentSessionChanged(&current_handler)?;

        let sessions_handler = TypedEventHandler::new(move |_, _| {
            callback.request();
            Ok(())
        });
        let sessions = match manager.SessionsChanged(&sessions_handler) {
            Ok(token) => token,
            Err(error) => {
                let _ = manager.RemoveCurrentSessionChanged(current_session);
                return Err(error);
            }
        };

        Ok(Self {
            manager,
            current_session,
            sessions,
        })
    }
}

impl Drop for ManagerEvents {
    fn drop(&mut self) {
        let _ = self.manager.RemoveSessionsChanged(self.sessions);
        let _ = self
            .manager
            .RemoveCurrentSessionChanged(self.current_session);
    }
}

struct SessionEvents {
    session: GlobalSystemMediaTransportControlsSession,
    media_properties: i64,
    playback_info: i64,
}

impl SessionEvents {
    fn subscribe(
        session: GlobalSystemMediaTransportControlsSession,
        callback: RefreshCallback,
    ) -> Result<Self, WindowsError> {
        let media_callback = callback.clone();
        let media_handler = TypedEventHandler::new(move |_, _| {
            media_callback.request();
            Ok(())
        });
        let media_properties = session.MediaPropertiesChanged(&media_handler)?;

        let playback_handler = TypedEventHandler::new(move |_, _| {
            callback.request();
            Ok(())
        });
        let playback_info = match session.PlaybackInfoChanged(&playback_handler) {
            Ok(token) => token,
            Err(error) => {
                let _ = session.RemoveMediaPropertiesChanged(media_properties);
                return Err(error);
            }
        };

        Ok(Self {
            session,
            media_properties,
            playback_info,
        })
    }
}

impl Drop for SessionEvents {
    fn drop(&mut self) {
        let _ = self.session.RemovePlaybackInfoChanged(self.playback_info);
        let _ = self
            .session
            .RemoveMediaPropertiesChanged(self.media_properties);
    }
}

fn read_snapshot(
    session: &GlobalSystemMediaTransportControlsSession,
) -> Option<MediaSnapshot> {
    let properties = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    let playback = session.GetPlaybackInfo().ok()?;
    let controls = playback.Controls().ok()?;

    Some(MediaSnapshot {
        source_id: session.SourceAppUserModelId().ok()?.to_string_lossy(),
        title: properties.Title().ok()?.to_string_lossy(),
        artist: properties.Artist().ok()?.to_string_lossy(),
        artwork: read_artwork(&properties),
        playback: playback
            .PlaybackStatus()
            .map_or(PlaybackState::Stopped, playback_state),
        controls: MediaControls {
            previous: controls.IsPreviousEnabled().unwrap_or(false),
            play: controls.IsPlayEnabled().unwrap_or(false),
            pause: controls.IsPauseEnabled().unwrap_or(false),
            next: controls.IsNextEnabled().unwrap_or(false),
        },
    })
}

fn read_artwork(
    properties: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties,
) -> Option<Vec<u8>> {
    let stream = properties
        .Thumbnail()
        .ok()?
        .OpenReadAsync()
        .ok()?
        .join()
        .ok()?;
    let size = stream.Size().ok()?;
    if size == 0 || size > MAX_ARTWORK_BYTES {
        return None;
    }

    let count = u32::try_from(size).ok()?;
    let input = stream.GetInputStreamAt(0).ok()?;
    let reader = DataReader::CreateDataReader(&input).ok()?;
    let loaded = reader.LoadAsync(count).ok()?.join().ok()?;
    if loaded != count {
        let _ = reader.Close();
        let _ = stream.Close();
        return None;
    }

    let mut artwork = vec![0; count as usize];
    let read = reader.ReadBytes(&mut artwork).is_ok();
    let _ = reader.Close();
    let _ = stream.Close();
    read.then_some(artwork)
}

fn playback_state(
    status: GlobalSystemMediaTransportControlsSessionPlaybackStatus,
) -> PlaybackState {
    if status == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing {
        PlaybackState::Playing
    } else if status == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Paused {
        PlaybackState::Paused
    } else {
        PlaybackState::Stopped
    }
}

fn current_thread_id() -> u32 {
    // SAFETY: Reading the calling thread's id has no preconditions.
    unsafe { GetCurrentThreadId() }
}

struct WinRtApartment;

impl WinRtApartment {
    fn enter() -> Result<Self, WindowsError> {
        // SAFETY: The media worker is a fresh thread with no existing apartment initialization.
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }?;
        Ok(Self)
    }
}

impl Drop for WinRtApartment {
    fn drop(&mut self) {
        // SAFETY: This runs on the same worker thread after a successful RoInitialize call.
        unsafe { RoUninitialize() };
    }
}

fn premultiply_rgba_to_bgra(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        let red = u16::from(pixel[0]);
        let green = u16::from(pixel[1]);
        let blue = u16::from(pixel[2]);
        pixel[0] = u8::try_from(blue.saturating_mul(alpha) / 255).unwrap_or(u8::MAX);
        pixel[1] = u8::try_from(green.saturating_mul(alpha) / 255).unwrap_or(u8::MAX);
        pixel[2] = u8::try_from(red.saturating_mul(alpha) / 255).unwrap_or(u8::MAX);
    }
}
