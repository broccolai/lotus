use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_APP};

use super::session::MediaWorker;
use super::{MediaCommand, MediaEvent};

const MEDIA_WAKE_MESSAGE: u32 = WM_APP + 0x4C9;

pub(super) enum WorkerCommand {
    Refresh,
    Execute(MediaCommand),
    Shutdown,
}

pub(super) fn run_worker(
    owner_thread: u32,
    commands: &Receiver<WorkerCommand>,
    callback_sender: Sender<WorkerCommand>,
    events: &Sender<MediaEvent>,
) {
    let refresh_pending = Arc::new(AtomicBool::new(true));
    let callback = RefreshCallback::new(callback_sender, Arc::clone(&refresh_pending));
    let mut media = match MediaWorker::start(callback) {
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

pub const fn is_media_wake(message: u32) -> bool {
    message == MEDIA_WAKE_MESSAGE
}

pub(super) fn current_thread_id() -> u32 {
    unsafe { GetCurrentThreadId() }
}

#[derive(Clone)]
pub(super) struct RefreshCallback {
    sender: Sender<WorkerCommand>,
    pending: Arc<AtomicBool>,
}

impl RefreshCallback {
    pub(super) fn new(sender: Sender<WorkerCommand>, pending: Arc<AtomicBool>) -> Self {
        Self { sender, pending }
    }

    pub(super) fn request(&self) {
        if self.pending.swap(true, Ordering::AcqRel) {
            return;
        }
        if self.sender.send(WorkerCommand::Refresh).is_err() {
            self.pending.store(false, Ordering::Release);
        }
    }
}

pub(super) fn publish_event(
    owner_thread: u32,
    events: &Sender<MediaEvent>,
    event: MediaEvent,
) {
    if events.send(event).is_err() {
        return;
    }

    let _ = unsafe {
        PostThreadMessageW(owner_thread, MEDIA_WAKE_MESSAGE, WPARAM(0), LPARAM(0))
    };
}
