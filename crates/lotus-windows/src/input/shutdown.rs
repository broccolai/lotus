use std::sync::atomic::Ordering;
use std::time::Duration;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};

use super::{InputController, health};
use crate::responsiveness::InputFailOpenReason;

pub(super) fn stop(controller: &mut InputController, timeout: Duration) {
    controller.shared.stopping.store(true, Ordering::Release);
    let _ = health::enter_fail_open(&controller.shared, InputFailOpenReason::Shutdown);
    let _ = health::request_cleanup(&controller.shared);
    request_stop(controller.thread_id);
    if let Some(thread) = controller.thread.take()
        && controller.completion.recv_timeout(timeout).is_ok()
    {
        let _ = thread.join();
    }
}

pub(super) fn request_stop(thread_id: u32) {
    if thread_id != 0 {
        let _ = unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
    }
}
