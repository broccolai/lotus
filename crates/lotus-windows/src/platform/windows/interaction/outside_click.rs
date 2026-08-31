use std::hint::spin_loop;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GA_ROOT, GetAncestor, HHOOK, MSLLHOOKSTRUCT, PostMessageW,
    SetWindowsHookExW, UnhookWindowsHookEx, WH_MOUSE_LL, WM_LBUTTONDOWN, WM_MBUTTONDOWN,
    WM_RBUTTONDOWN, WM_XBUTTONDOWN, WindowFromPoint,
};

use crate::NativeError;

static OUTSIDE_CLICK_TARGET: AtomicIsize = AtomicIsize::new(0);
static OUTSIDE_CLICK_MESSAGE: AtomicIsize = AtomicIsize::new(0);
// Even values are stable generations. The low bit is a short-lived writer marker while a
// start or drop updates the accompanying target and message.
static OUTSIDE_CLICK_GENERATION: AtomicUsize = AtomicUsize::new(2);

pub(crate) struct OutsideClickObserver {
    hook: HHOOK,
    generation: usize,
}

impl OutsideClickObserver {
    pub(crate) fn start(target: HWND, message: u32) -> Result<Self, NativeError> {
        let module = unsafe { GetModuleHandleW(None) }?;
        let hook = unsafe {
            SetWindowsHookExW(
                WH_MOUSE_LL,
                Some(outside_click_hook),
                Some(HINSTANCE(module.0)),
                0,
            )
        }?;

        let generation = publish_outside_click_target(target, message);
        Ok(Self { hook, generation })
    }
}

impl Drop for OutsideClickObserver {
    fn drop(&mut self) {
        clear_outside_click_target(self.generation);
        let _ = unsafe { UnhookWindowsHookEx(self.hook) };
    }
}

fn publish_outside_click_target(target: HWND, message: u32) -> usize {
    let generation = lock_outside_click_state();
    OUTSIDE_CLICK_TARGET.store(target.0.addr().cast_signed(), Ordering::Relaxed);
    OUTSIDE_CLICK_MESSAGE.store(
        isize::try_from(message).unwrap_or_default(),
        Ordering::Relaxed,
    );
    unlock_outside_click_state(generation)
}

fn clear_outside_click_target(owner_generation: usize) {
    let generation = lock_outside_click_state();
    if generation == owner_generation {
        OUTSIDE_CLICK_TARGET.store(0, Ordering::Relaxed);
        OUTSIDE_CLICK_MESSAGE.store(0, Ordering::Relaxed);
        let _ = unlock_outside_click_state(generation);
    } else {
        OUTSIDE_CLICK_GENERATION.store(generation, Ordering::Release);
    }
}

fn lock_outside_click_state() -> usize {
    loop {
        let generation = OUTSIDE_CLICK_GENERATION.load(Ordering::Acquire);
        if generation & 1 != 0 {
            spin_loop();
            continue;
        }

        if OUTSIDE_CLICK_GENERATION
            .compare_exchange_weak(
                generation,
                generation | 1,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            return generation;
        }
    }
}

fn unlock_outside_click_state(generation: usize) -> usize {
    let next_generation = generation.wrapping_add(2);
    let next_generation = if next_generation == 0 {
        2
    } else {
        next_generation
    };
    OUTSIDE_CLICK_GENERATION.store(next_generation, Ordering::Release);
    next_generation
}

unsafe extern "system" fn outside_click_hook(
    code: i32,
    message: WPARAM,
    data: LPARAM,
) -> LRESULT {
    if code >= 0 && is_pointer_press(message) && data.0 != 0 {
        let generation = OUTSIDE_CLICK_GENERATION.load(Ordering::Acquire);
        let raw_target = OUTSIDE_CLICK_TARGET.load(Ordering::Relaxed);
        let raw_message = OUTSIDE_CLICK_MESSAGE.load(Ordering::Relaxed);
        let is_current = OUTSIDE_CLICK_GENERATION.load(Ordering::Acquire) == generation;
        if generation & 1 == 0 && is_current && raw_target != 0 && raw_message > 0 {
            let target = HWND(raw_target.cast_unsigned() as *mut _);
            let pointer = unsafe { &*(data.0 as *const MSLLHOOKSTRUCT) };
            let clicked_root = unsafe { GetAncestor(WindowFromPoint(pointer.pt), GA_ROOT) };
            if clicked_root != target {
                let _ = unsafe {
                    PostMessageW(
                        Some(target),
                        u32::try_from(raw_message).unwrap_or_default(),
                        WPARAM(0),
                        LPARAM(0),
                    )
                };
            }
        }
    }

    unsafe { CallNextHookEx(None, code, message, data) }
}

fn is_pointer_press(message: WPARAM) -> bool {
    u32::try_from(message.0).is_ok_and(|message| {
        matches!(
            message,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
        )
    })
}
