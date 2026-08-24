use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;

use crate::messages::GRAPHICS_RECOVERY_WAKE;

const RETRY_DELAY: Duration = Duration::from_millis(250);
const MAX_RETRIES: u8 = 3;

pub enum GraphicsRecoverySchedule {
    Scheduled { attempt: u8 },
    Pending,
    Exhausted,
    AlreadyExhausted,
    Unavailable,
}

pub struct GraphicsRecoveryScheduler {
    thread_id: u32,
    pending: Arc<AtomicBool>,
    attempts: u8,
    exhausted: bool,
    pending_wake: Option<usize>,
    next_wake: usize,
}

impl GraphicsRecoveryScheduler {
    pub fn new() -> Self {
        Self {
            thread_id: unsafe { GetCurrentThreadId() },
            pending: Arc::new(AtomicBool::new(false)),
            attempts: 0,
            exhausted: false,
            pending_wake: None,
            next_wake: 0,
        }
    }

    pub const fn is_wake(&self, message: u32) -> bool {
        message == GRAPHICS_RECOVERY_WAKE
    }

    pub fn take_wake(&mut self, wake: usize) -> bool {
        if self.pending_wake != Some(wake) {
            return false;
        }
        self.pending_wake = None;
        self.pending.store(false, Ordering::Release);
        true
    }

    pub fn schedule(&mut self) -> GraphicsRecoverySchedule {
        if self.pending.load(Ordering::Acquire) {
            return GraphicsRecoverySchedule::Pending;
        }
        if self.exhausted {
            return GraphicsRecoverySchedule::AlreadyExhausted;
        }
        if self.attempts >= MAX_RETRIES {
            self.exhausted = true;
            return GraphicsRecoverySchedule::Exhausted;
        }
        self.pending.store(true, Ordering::Release);
        self.attempts = self.attempts.saturating_add(1);
        let attempt = self.attempts;
        let wake = self.next_wake;
        self.next_wake = self.next_wake.wrapping_add(1);
        self.pending_wake = Some(wake);
        let pending = Arc::clone(&self.pending);
        let thread_id = self.thread_id;
        if std::thread::Builder::new()
            .name("lotus-graphics-recovery".to_owned())
            .spawn(move || {
                std::thread::sleep(RETRY_DELAY);
                if unsafe {
                    PostThreadMessageW(
                        thread_id,
                        GRAPHICS_RECOVERY_WAKE,
                        WPARAM(wake),
                        LPARAM(0),
                    )
                }
                .is_err()
                {
                    pending.store(false, Ordering::Release);
                }
            })
            .is_err()
        {
            self.pending.store(false, Ordering::Release);
            return GraphicsRecoverySchedule::Unavailable;
        }
        GraphicsRecoverySchedule::Scheduled { attempt }
    }

    pub fn reset(&mut self) {
        self.attempts = 0;
        self.exhausted = false;
        self.pending_wake = None;
        self.pending.store(false, Ordering::Release);
    }
}

impl Default for GraphicsRecoveryScheduler {
    fn default() -> Self {
        Self::new()
    }
}
