use std::sync::atomic::{AtomicIsize, Ordering};

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

pub(crate) struct OutsideClickObserver {
    hook: HHOOK,
    target: HWND,
}

impl OutsideClickObserver {
    pub(crate) fn start(target: HWND, message: u32) -> Result<Self, NativeError> {
        let module = unsafe { GetModuleHandleW(None) }?;
        OUTSIDE_CLICK_TARGET.store(target.0.addr().cast_signed(), Ordering::Release);
        OUTSIDE_CLICK_MESSAGE.store(
            isize::try_from(message).unwrap_or_default(),
            Ordering::Release,
        );

        let hook = unsafe {
            SetWindowsHookExW(
                WH_MOUSE_LL,
                Some(outside_click_hook),
                Some(HINSTANCE(module.0)),
                0,
            )
        }
        .inspect_err(|_| clear_outside_click_target(target))?;
        Ok(Self { hook, target })
    }
}

impl Drop for OutsideClickObserver {
    fn drop(&mut self) {
        clear_outside_click_target(self.target);
        let _ = unsafe { UnhookWindowsHookEx(self.hook) };
    }
}

fn clear_outside_click_target(target: HWND) {
    let target = target.0.addr().cast_signed();
    let _ = OUTSIDE_CLICK_TARGET.compare_exchange(
        target,
        0,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
    OUTSIDE_CLICK_MESSAGE.store(0, Ordering::Release);
}

unsafe extern "system" fn outside_click_hook(
    code: i32,
    message: WPARAM,
    data: LPARAM,
) -> LRESULT {
    if code >= 0 && is_pointer_press(message) && data.0 != 0 {
        let raw_target = OUTSIDE_CLICK_TARGET.load(Ordering::Acquire);
        let raw_message = OUTSIDE_CLICK_MESSAGE.load(Ordering::Acquire);
        if raw_target != 0 && raw_message > 0 {
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
