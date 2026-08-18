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

pub(crate) fn activate_window(hwnd: HWND) -> FocusClaim {
    if owns_foreground_application(hwnd) {
        return FocusClaim::Owned;
    }

    let foreground = unsafe { GetForegroundWindow() };
    let foreground_thread = window_thread(foreground);
    let target_thread = window_thread(hwnd);
    let current_thread = unsafe { GetCurrentThreadId() };
    let _foreground_attachment =
        InputQueueAttachment::new(current_thread, foreground_thread);
    let _target_attachment = InputQueueAttachment::new(current_thread, target_thread);

    if request_activation(hwnd) || owns_foreground_application(hwnd) {
        FocusClaim::Owned
    } else {
        FocusClaim::Denied
    }
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

fn owns_foreground_application(hwnd: HWND) -> bool {
    let foreground = unsafe { GetForegroundWindow() };
    if foreground == hwnd {
        return true;
    }
    let target_process = window_process(hwnd);
    target_process != 0 && target_process == window_process(foreground)
}

fn window_process(hwnd: HWND) -> u32 {
    if hwnd.is_invalid() {
        return 0;
    }
    let mut process_id = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut process_id)) };
    process_id
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
