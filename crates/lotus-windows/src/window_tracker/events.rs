use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE, EVENT_OBJECT_LOCATIONCHANGE,
    EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_SHOW, EVENT_SYSTEM_FOREGROUND, OBJID_WINDOW,
    PostThreadMessageW, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};
use windows::core::Error;

use crate::NativeError;
pub(super) use crate::messages::WINDOW_TRACKER_REFRESH as REFRESH_MESSAGE;

pub(super) const REFRESH_DELAY_MS: u32 = 120;
pub(super) const RECONCILE_INTERVAL_MS: u32 = 1_000;
pub(super) const SHUTDOWN_MESSAGE: u32 = REFRESH_MESSAGE + 1;
pub(super) const IMMEDIATE_RECONCILE_MESSAGE: u32 = REFRESH_MESSAGE + 2;

const TRACKED_EVENTS: [u32; 6] = [
    EVENT_SYSTEM_FOREGROUND,
    EVENT_OBJECT_DESTROY,
    EVENT_OBJECT_SHOW,
    EVENT_OBJECT_HIDE,
    EVENT_OBJECT_LOCATIONCHANGE,
    EVENT_OBJECT_NAMECHANGE,
];

static CALLBACK_THREAD: AtomicU32 = AtomicU32::new(0);
static REFRESH_QUEUED: AtomicBool = AtomicBool::new(false);
static CALLBACKS_ACTIVE: AtomicBool = AtomicBool::new(false);

pub(super) fn claim_callback_thread(thread_id: u32) -> bool {
    CALLBACK_THREAD
        .compare_exchange(0, thread_id, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

pub(super) fn callback_thread() -> u32 {
    CALLBACK_THREAD.load(Ordering::Acquire)
}

pub(super) fn activate_callbacks() {
    CALLBACKS_ACTIVE.store(true, Ordering::Release);
}

pub(super) fn release_callback_thread(thread_id: u32) {
    if CALLBACK_THREAD
        .compare_exchange(thread_id, 0, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        CALLBACKS_ACTIVE.store(false, Ordering::Release);
        REFRESH_QUEUED.store(false, Ordering::Release);
    }
}

pub(super) fn clear_refresh_notification() {
    REFRESH_QUEUED.store(false, Ordering::Release);
}

pub(super) fn request_immediate_reconcile() {
    let thread_id = callback_thread();
    if thread_id != 0 {
        let _ = unsafe {
            PostThreadMessageW(thread_id, IMMEDIATE_RECONCILE_MESSAGE, WPARAM(0), LPARAM(0))
        };
    }
}

pub(super) fn install_hooks() -> Result<Vec<OwnedWinEventHook>, NativeError> {
    TRACKED_EVENTS
        .into_iter()
        .map(OwnedWinEventHook::install)
        .collect()
}

pub(super) struct OwnedWinEventHook(HWINEVENTHOOK);

impl OwnedWinEventHook {
    fn install(event: u32) -> Result<Self, NativeError> {
        let hook = unsafe {
            SetWinEventHook(
                event,
                event,
                None,
                Some(win_event_callback),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };

        if hook.is_invalid() {
            Err(Error::from_thread().into())
        } else {
            Ok(Self(hook))
        }
    }
}

impl Drop for OwnedWinEventHook {
    fn drop(&mut self) {
        let _ = unsafe { UnhookWinEvent(self.0) };
    }
}

unsafe extern "system" fn win_event_callback(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object_id: i32,
    _child_id: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if !CALLBACKS_ACTIVE.load(Ordering::Acquire)
        || hwnd.0.is_null()
        || (event != EVENT_SYSTEM_FOREGROUND && object_id != OBJID_WINDOW.0)
        || REFRESH_QUEUED.swap(true, Ordering::AcqRel)
    {
        return;
    }

    let thread_id = callback_thread();
    if thread_id == 0
        || unsafe { PostThreadMessageW(thread_id, REFRESH_MESSAGE, WPARAM(0), LPARAM(0)) }
            .is_err()
    {
        REFRESH_QUEUED.store(false, Ordering::Release);
    }
}
