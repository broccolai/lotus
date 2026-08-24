use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetActiveWindow, GetFocus, SetActiveWindow, SetFocus,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, IDC_ARROW, IDC_HAND,
    IDC_SIZEWE, LoadCursorW, SetCursor, SetForegroundWindow,
};

use crate::NativeError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FocusClaim {
    Owned,
    Denied,
}

impl FocusClaim {
    pub(crate) const fn is_owned(self) -> bool {
        matches!(self, Self::Owned)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PointerCursor {
    #[default]
    Arrow,
    Hand,
    HorizontalResize,
}

impl PointerCursor {
    pub(crate) fn apply(self) -> Result<(), NativeError> {
        let resource = match self {
            Self::Arrow => IDC_ARROW,
            Self::Hand => IDC_HAND,
            Self::HorizontalResize => IDC_SIZEWE,
        };
        let cursor = unsafe { LoadCursorW(None, resource)? };
        unsafe { SetCursor(Some(cursor)) };
        Ok(())
    }
}

pub(crate) fn claim_keyboard_focus(hwnd: HWND) -> FocusClaim {
    if owns_keyboard_focus(hwnd) || focus_once(hwnd) {
        return FocusClaim::Owned;
    }

    let foreground = unsafe { GetForegroundWindow() };
    let foreground_thread = window_thread(foreground);
    let current_thread = unsafe { GetCurrentThreadId() };
    let _attachment = InputQueueAttachment::new(current_thread, foreground_thread);

    if focus_once(hwnd) {
        FocusClaim::Owned
    } else {
        FocusClaim::Denied
    }
}

/// Claims a particular top-level window for delayed exact-window actions.
///
/// This deliberately does not treat a sibling owned by the same process as success:
/// picker and Alt+Tab must claim their presented HWND.
pub(crate) fn activate_exact_window(hwnd: HWND) -> FocusClaim {
    if owns_exact_foreground(hwnd) {
        return FocusClaim::Owned;
    }

    let foreground = unsafe { GetForegroundWindow() };
    let foreground_thread = window_thread(foreground);
    let target_thread = window_thread(hwnd);
    let current_thread = unsafe { GetCurrentThreadId() };
    let _foreground_attachment =
        InputQueueAttachment::new(current_thread, foreground_thread);
    let _target_attachment = InputQueueAttachment::new(current_thread, target_thread);

    let _requested = request_activation(hwnd);
    exact_foreground_claim(hwnd, unsafe { GetForegroundWindow() })
}

fn window_thread(hwnd: HWND) -> u32 {
    if hwnd.is_invalid() {
        return 0;
    }
    unsafe { GetWindowThreadProcessId(hwnd, None) }
}

fn focus_once(hwnd: HWND) -> bool {
    unsafe {
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetActiveWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
    }
    owns_keyboard_focus(hwnd)
}

fn request_activation(hwnd: HWND) -> bool {
    unsafe {
        let _ = BringWindowToTop(hwnd);
        let requested = SetForegroundWindow(hwnd).as_bool();
        let _ = SetActiveWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        requested
    }
}

fn owns_exact_foreground(hwnd: HWND) -> bool {
    unsafe { GetForegroundWindow() == hwnd }
}

fn exact_foreground_claim(requested: HWND, foreground: HWND) -> FocusClaim {
    if requested == foreground {
        FocusClaim::Owned
    } else {
        FocusClaim::Denied
    }
}

fn owns_keyboard_focus(hwnd: HWND) -> bool {
    unsafe {
        GetForegroundWindow() == hwnd && GetActiveWindow() == hwnd && GetFocus() == hwnd
    }
}

struct InputQueueAttachment {
    source: u32,
    target: u32,
    attached: bool,
}

impl InputQueueAttachment {
    fn new(source: u32, target: u32) -> Self {
        let attached = source != 0
            && target != 0
            && source != target
            && unsafe { AttachThreadInput(source, target, true) }.as_bool();
        Self {
            source,
            target,
            attached,
        }
    }
}

impl Drop for InputQueueAttachment {
    fn drop(&mut self) {
        if self.attached {
            let _ = unsafe { AttachThreadInput(self.source, self.target, false) };
        }
    }
}
