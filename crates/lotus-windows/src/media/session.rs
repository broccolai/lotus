use std::sync::mpsc::Sender;

use lotus_media::{MediaControls, MediaSnapshot, PlaybackState};
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession,
    GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};
use windows::Storage::Streams::DataReader;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
use windows::core::Error as WindowsError;

use super::worker::{RefreshCallback, publish_event};
use super::{MediaCommand, MediaEvent};

const MAX_ARTWORK_BYTES: u64 = 8 * 1024 * 1024;

pub(super) struct MediaWorker {
    _apartment: WinRtApartment,
    manager: GlobalSystemMediaTransportControlsSessionManager,
    _manager_events: ManagerEvents,
    session_events: Option<SessionEvents>,
    callback: RefreshCallback,
}

impl MediaWorker {
    pub(super) fn start(callback: RefreshCallback) -> Result<Self, WindowsError> {
        let apartment = WinRtApartment::enter()?;
        let manager =
            GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.join()?;
        let manager_events = ManagerEvents::subscribe(manager.clone(), callback.clone())?;

        Ok(Self {
            _apartment: apartment,
            manager,
            _manager_events: manager_events,
            session_events: None,
            callback,
        })
    }

    pub(super) fn refresh(&mut self, owner_thread: u32, events: &Sender<MediaEvent>) {
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

    pub(super) fn execute(&self, command: MediaCommand) {
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

struct WinRtApartment;

impl WinRtApartment {
    fn enter() -> Result<Self, WindowsError> {
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }?;
        Ok(Self)
    }
}

impl Drop for WinRtApartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}
