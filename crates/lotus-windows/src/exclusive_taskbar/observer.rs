use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_CREATE, EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_SHOW, GetMessageW, MSG,
    OBJID_WINDOW, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS, WM_QUIT,
};
use windows::core::Error;

use super::taskbar_windows::is_taskbar_window;
use super::visibility_transaction::TaskbarVisibilityTransaction;
use crate::exclusive_taskbar::ExclusiveTaskbarError;
use crate::messages::TASKBAR_EVENT as TASKBAR_EVENT_MESSAGE;
const START_TIMEOUT: Duration = Duration::from_secs(5);
static EVENT_THREAD_ID: AtomicU32 = AtomicU32::new(0);

pub(super) struct TaskbarEventObserver {
    thread_id: u32,
    thread: Option<thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

impl TaskbarEventObserver {
    pub(super) fn start() -> Result<Self, ExclusiveTaskbarError> {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("lotus-taskbar-events".into())
            .spawn(move || taskbar_event_loop(&ready_tx, &thread_stop))?;

        match ready_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                thread: Some(thread),
                stop,
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(ExclusiveTaskbarError::EventObserver(error.into()))
            }
            Err(_) => {
                stop.store(true, Ordering::Release);
                post_quit(EVENT_THREAD_ID.load(Ordering::Acquire));
                let _ = thread.join();
                Err(ExclusiveTaskbarError::EventObserverStopped)
            }
        }
    }

    pub(super) fn is_finished(&self) -> bool {
        self.thread
            .as_ref()
            .is_none_or(thread::JoinHandle::is_finished)
    }
}

impl Drop for TaskbarEventObserver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        post_quit(self.thread_id);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn taskbar_event_loop(ready: &mpsc::SyncSender<Result<u32, Error>>, stop: &AtomicBool) {
    let mut message = MSG::default();
    // SAFETY: A no-remove peek is the documented way to establish this thread's message queue.
    let _ = unsafe { PeekMessageW(&raw mut message, None, 0, 0, PM_NOREMOVE) };
    // SAFETY: This call only identifies the current event-loop thread.
    let thread_id = unsafe { GetCurrentThreadId() };
    EVENT_THREAD_ID.store(thread_id, Ordering::Release);

    let hooks = install_hooks();
    let hooks = match hooks {
        Ok(hooks) => hooks,
        Err(error) => {
            clear_event_thread(thread_id);
            let _ = ready.send(Err(error));
            return;
        }
    };

    let mut windows = TaskbarVisibilityTransaction::start();
    if stop.load(Ordering::Acquire) {
        clear_event_thread(thread_id);
        return;
    }
    windows.hide_existing();
    if ready.send(Ok(thread_id)).is_err() {
        clear_event_thread(thread_id);
        return;
    }

    loop {
        // SAFETY: `message` remains writable and this thread owns its message queue.
        let result = unsafe { GetMessageW(&raw mut message, None, 0, 0) }.0;
        if result <= 0 {
            break;
        }
        if message.message == TASKBAR_EVENT_MESSAGE {
            let hwnd = HWND(std::ptr::with_exposed_provenance_mut(message.wParam.0));
            windows.hide(hwnd);
        }
    }

    drop(hooks);
    clear_event_thread(thread_id);
}

fn install_hooks() -> Result<Vec<OwnedWinEventHook>, Error> {
    [
        EVENT_OBJECT_CREATE,
        EVENT_OBJECT_SHOW,
        EVENT_OBJECT_LOCATIONCHANGE,
    ]
    .into_iter()
    .map(OwnedWinEventHook::install)
    .collect()
}

fn post_quit(thread_id: u32) {
    if thread_id != 0 {
        // SAFETY: A nonzero published ID belongs to the event thread's live queue.
        let _ = unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
    }
}

fn clear_event_thread(thread_id: u32) {
    let _ =
        EVENT_THREAD_ID.compare_exchange(thread_id, 0, Ordering::AcqRel, Ordering::Acquire);
}

struct OwnedWinEventHook(HWINEVENTHOOK);

impl OwnedWinEventHook {
    fn install(event: u32) -> Result<Self, Error> {
        // SAFETY: The static callback contains no borrowed state. Out-of-context delivery keeps
        // Lotus code outside Explorer, and this process owns a pumping message queue.
        let hook = unsafe {
            SetWinEventHook(
                event,
                event,
                None,
                Some(taskbar_event_callback),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };
        if hook.is_invalid() {
            Err(Error::from_thread())
        } else {
            Ok(Self(hook))
        }
    }
}

impl Drop for OwnedWinEventHook {
    fn drop(&mut self) {
        // SAFETY: This guard owns the successful event-hook registration exactly once.
        let _ = unsafe { UnhookWinEvent(self.0) };
    }
}

unsafe extern "system" fn taskbar_event_callback(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    object_id: i32,
    _child_id: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if hwnd.0.is_null() || object_id != OBJID_WINDOW.0 || !is_taskbar_window(hwnd) {
        return;
    }

    let thread_id = EVENT_THREAD_ID.load(Ordering::Acquire);
    if thread_id != 0 {
        // SAFETY: The observer thread publishes its ID after creating a message queue. The HWND is
        // an opaque integer value and contains no borrowed process memory.
        let _ = unsafe {
            PostThreadMessageW(
                thread_id,
                TASKBAR_EVENT_MESSAGE,
                WPARAM(hwnd.0.addr()),
                LPARAM(0),
            )
        };
    }
}
