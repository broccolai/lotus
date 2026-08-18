use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE, EVENT_OBJECT_LOCATIONCHANGE,
    EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_SHOW, EVENT_SYSTEM_FOREGROUND, KillTimer,
    OBJID_WINDOW, PostThreadMessageW, SetTimer, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS, WM_APP,
};
use windows::core::Error;

use crate::NativeError;
pub(super) const REFRESH_MESSAGE: u32 = WM_APP + 1;
pub(super) const REFRESH_DELAY_MS: u32 = 120;
pub(super) const RECONCILE_INTERVAL_MS: u32 = 1_000;
const TRACKED_EVENTS: [u32; 6] = [
    EVENT_SYSTEM_FOREGROUND,
    EVENT_OBJECT_DESTROY,
    EVENT_OBJECT_SHOW,
    EVENT_OBJECT_HIDE,
    EVENT_OBJECT_LOCATIONCHANGE,
    EVENT_OBJECT_NAMECHANGE,
];
static CALLBACK_THREAD: AtomicU32 = AtomicU32::new(0);
static DEFERRED_NOTIFICATION_QUEUED: AtomicBool = AtomicBool::new(false);
pub(super) fn claim_callback_thread(thread_id: u32) -> bool {
    CALLBACK_THREAD
        .compare_exchange(0, thread_id, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}
pub(super) fn release_callback_thread() {
    CALLBACK_THREAD.store(0, Ordering::Release);
    DEFERRED_NOTIFICATION_QUEUED.store(false, Ordering::Release);
}
pub(super) fn clear_deferred_notification() {
    DEFERRED_NOTIFICATION_QUEUED.store(false, Ordering::Release);
}
pub(super) fn create_thread_timer(interval_ms: u32) -> Result<usize, NativeError> {
    let id = unsafe { SetTimer(None, 0, interval_ms, None) };
    if id == 0 {
        Err(Error::from_thread().into())
    } else {
        Ok(id)
    }
}
pub(super) fn cancel_thread_timer(timer_id: usize) {
    let _ = unsafe { KillTimer(None, timer_id) };
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
    if hwnd.0.is_null() || (event != EVENT_SYSTEM_FOREGROUND && object_id != OBJID_WINDOW.0)
    {
        return;
    }
    let foreground = event == EVENT_SYSTEM_FOREGROUND;
    if !foreground && DEFERRED_NOTIFICATION_QUEUED.swap(true, Ordering::AcqRel) {
        return;
    }
    let thread_id = CALLBACK_THREAD.load(Ordering::Acquire);
    if thread_id == 0 {
        if !foreground {
            DEFERRED_NOTIFICATION_QUEUED.store(false, Ordering::Release);
        }
        return;
    }
    let parameter = usize::try_from(event).unwrap_or_default();
    if unsafe {
        PostThreadMessageW(thread_id, REFRESH_MESSAGE, WPARAM(parameter), LPARAM(0))
    }
    .is_err()
        && !foreground
    {
        DEFERRED_NOTIFICATION_QUEUED.store(false, Ordering::Release);
    }
}
