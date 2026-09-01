mod artwork;
mod session;
mod worker;

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use lotus_media::MediaSnapshot;
use thiserror::Error;

pub use self::artwork::{MediaArtworkError, decode_artwork};
pub use self::worker::is_media_wake;
use self::worker::{WorkerCommand, current_thread_id, run_worker};
use crate::background_worker::{BackgroundWorker, WorkerJoinPolicy};

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
    worker: BackgroundWorker,
}

impl MediaController {
    pub fn start() -> Result<Self, MediaControllerError> {
        let owner_thread = current_thread_id();
        let (command_sender, command_receiver) = mpsc::channel();
        let (event_sender, event_receiver) = mpsc::channel();
        let callback_sender = command_sender.clone();
        let worker = thread::Builder::new()
            .name("lotus-media-controls".into())
            .spawn(move || {
                run_worker(
                    owner_thread,
                    &command_receiver,
                    callback_sender,
                    &event_sender,
                );
            })?;

        let stop_sender = command_sender.clone();
        Ok(Self {
            commands: command_sender,
            events: event_receiver,
            worker: BackgroundWorker::new(worker, WorkerJoinPolicy::Always, move || {
                let _ = stop_sender.send(WorkerCommand::Shutdown);
            }),
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
        self.worker.shutdown();
    }
}
