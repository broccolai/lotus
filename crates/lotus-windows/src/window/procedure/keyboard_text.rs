use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT,
    VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, WA_INACTIVE, WM_ACTIVATE, WM_CHAR, WM_KEYDOWN, WM_KEYUP, WM_MOUSEWHEEL,
};

use super::{
    ContextMenuEvent, CursorMove, DismissReason, SEARCH_OUTSIDE_CLICK_MESSAGE, SearchEdit,
    SearchEvent, SelectionDirection, SettingsEvent, SettingsKey, is_context_menu_window,
    is_search_window, is_settings_window, low_word, push_event, request_dismiss,
    with_window_state,
};
use crate::platform::windows::interaction::{
    OutsideClickObserver, claim_keyboard_focus, key_is_pressed,
};

pub(super) fn dispatch(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    if message == WM_MOUSEWHEEL {
        return dispatch_wheel(hwnd, wparam);
    }
    match message {
        WM_ACTIVATE if is_search_window(hwnd) => {
            Some(dispatch_search_activation(hwnd, wparam, lparam))
        }
        SEARCH_OUTSIDE_CLICK_MESSAGE if is_search_window(hwnd) => {
            if OutsideClickObserver::is_current_message(hwnd, wparam.0) {
                request_dismiss(hwnd, DismissReason::OutsideClick);
            }
            Some(LRESULT(0))
        }
        WM_ACTIVATE if is_context_menu_window(hwnd) => {
            Some(dispatch_context_menu_activation(hwnd, wparam, lparam))
        }
        WM_KEYDOWN if is_search_window(hwnd) => {
            Some(dispatch_search_key(hwnd, message, wparam, lparam))
        }
        WM_KEYDOWN if is_settings_window(hwnd) => {
            Some(dispatch_settings_key(hwnd, message, wparam, lparam))
        }
        WM_KEYDOWN | WM_KEYUP if is_context_menu_window(hwnd) => {
            Some(dispatch_context_menu_key(hwnd, message, wparam, lparam))
        }
        WM_CHAR if is_search_window(hwnd) => {
            push_search_text_unit(hwnd, wparam);
            Some(LRESULT(0))
        }
        WM_CHAR if is_settings_window(hwnd) => {
            if let Some(character) =
                char::from_u32(u32::try_from(wparam.0).unwrap_or_default())
            {
                push_event(hwnd, SettingsEvent::TextInput(character));
            }
            Some(LRESULT(0))
        }
        _ => None,
    }
}

fn dispatch_wheel(hwnd: HWND, wparam: WPARAM) -> Option<LRESULT> {
    let direction = wheel_selection_direction(wparam);
    if is_search_window(hwnd) {
        if let Some(direction) = direction {
            push_event(hwnd, SearchEvent::MoveSelection(direction));
        }
        return Some(LRESULT(0));
    }
    if is_context_menu_window(hwnd) {
        if let Some(direction) = direction {
            push_event(hwnd, ContextMenuEvent::Scroll(direction));
        }
        return Some(LRESULT(0));
    }
    if is_settings_window(hwnd) {
        if let Some(direction) = direction {
            let direction = match direction {
                SelectionDirection::Previous => -1,
                SelectionDirection::Next => 1,
            };
            push_event(hwnd, SettingsEvent::Scroll { direction });
        }
        return Some(LRESULT(0));
    }
    None
}

fn dispatch_search_activation(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let inactive = low_word(wparam.0) == WA_INACTIVE;
    let activated_window = HWND(lparam.0.cast_unsigned() as *mut _);
    let opening_context_menu = inactive && is_context_menu_window(activated_window);
    if inactive && !opening_context_menu {
        request_dismiss(hwnd, DismissReason::Deactivated);
    }
    let result = unsafe { DefWindowProcW(hwnd, WM_ACTIVATE, wparam, lparam) };
    if !inactive {
        let _ = claim_keyboard_focus(hwnd);
    }
    result
}

fn dispatch_context_menu_activation(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if low_word(wparam.0) == WA_INACTIVE {
        let activated_window = HWND(lparam.0.cast_unsigned() as *mut _);
        let reason = if is_search_window(activated_window) {
            DismissReason::OwnerActivated
        } else {
            DismissReason::Deactivated
        };
        request_dismiss(hwnd, reason);
    }
    unsafe { DefWindowProcW(hwnd, WM_ACTIVATE, wparam, lparam) }
}

fn dispatch_search_key(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if wparam.0 == usize::from(VK_ESCAPE.0) {
        request_dismiss(hwnd, DismissReason::Escape);
        return LRESULT(0);
    }
    if let Some(event) = search_key_event(wparam) {
        push_event(hwnd, event);
        LRESULT(0)
    } else {
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }
}

fn dispatch_settings_key(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if let Some(key) = settings_key(wparam) {
        push_event(hwnd, SettingsEvent::KeyPressed(key));
        LRESULT(0)
    } else {
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }
}

fn dispatch_context_menu_key(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let Ok(key) = u16::try_from(wparam.0) else {
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    };
    if key == VK_SHIFT.0 {
        push_event(hwnd, ContextMenuEvent::ShiftChanged(message == WM_KEYDOWN));
        return LRESULT(0);
    }
    if message == WM_KEYUP {
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    if key == VK_ESCAPE.0 {
        request_dismiss(hwnd, DismissReason::Escape);
        return LRESULT(0);
    }
    let event = match key {
        key if key == VK_RETURN.0 || key == VK_SPACE.0 => {
            ContextMenuEvent::SelectionRequested
        }
        key if key == VK_LEFT.0 || key == VK_UP.0 => {
            ContextMenuEvent::MoveSelection(SelectionDirection::Previous)
        }
        key if key == VK_RIGHT.0 || key == VK_DOWN.0 => {
            ContextMenuEvent::MoveSelection(SelectionDirection::Next)
        }
        _ => return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    };
    push_event(hwnd, event);
    LRESULT(0)
}

fn search_key_event(wparam: WPARAM) -> Option<SearchEvent> {
    search_key_event_for(u16::try_from(wparam.0).ok()?, key_is_pressed(VK_CONTROL))
}
fn search_key_event_for(key: u16, control_pressed: bool) -> Option<SearchEvent> {
    if control_pressed {
        return match key {
            0x41 => Some(SearchEvent::Edit(SearchEdit::SelectAll)),
            0x56 => Some(SearchEvent::PasteRequested),
            key if key == VK_BACK.0 => {
                Some(SearchEvent::Edit(SearchEdit::DeletePreviousWord))
            }
            _ => None,
        };
    }
    match key {
        key if key == VK_BACK.0 => Some(SearchEvent::Edit(SearchEdit::DeleteBackward)),
        key if key == VK_DELETE.0 => Some(SearchEvent::Edit(SearchEdit::DeleteForward)),
        key if key == VK_HOME.0 => {
            Some(SearchEvent::Edit(SearchEdit::MoveCursor(CursorMove::Home)))
        }
        key if key == VK_END.0 => {
            Some(SearchEvent::Edit(SearchEdit::MoveCursor(CursorMove::End)))
        }
        key if key == VK_LEFT.0 => Some(SearchEvent::Edit(SearchEdit::MoveCursor(
            CursorMove::Previous,
        ))),
        key if key == VK_RIGHT.0 => {
            Some(SearchEvent::Edit(SearchEdit::MoveCursor(CursorMove::Next)))
        }
        key if key == VK_UP.0 => {
            Some(SearchEvent::MoveSelection(SelectionDirection::Previous))
        }
        key if key == VK_DOWN.0 => {
            Some(SearchEvent::MoveSelection(SelectionDirection::Next))
        }
        key if key == VK_RETURN.0 => Some(SearchEvent::SubmitRequested),
        _ => None,
    }
}

fn settings_key(wparam: WPARAM) -> Option<SettingsKey> {
    settings_key_for(
        u16::try_from(wparam.0).ok()?,
        key_is_pressed(VK_CONTROL),
        key_is_pressed(VK_SHIFT),
    )
}
fn settings_key_for(
    key: u16,
    control_pressed: bool,
    shift_pressed: bool,
) -> Option<SettingsKey> {
    if control_pressed && key == 0x53 {
        return Some(SettingsKey::Save);
    }
    if control_pressed && key == 0x56 {
        return Some(SettingsKey::Paste);
    }
    match key {
        key if key == VK_ESCAPE.0 => Some(SettingsKey::Escape),
        key if key == VK_BACK.0 => Some(SettingsKey::Backspace),
        key if key == VK_RETURN.0 => Some(SettingsKey::Enter),
        key if key == VK_TAB.0 => Some(SettingsKey::Tab {
            reverse: shift_pressed,
        }),
        key if key == VK_LEFT.0 => Some(SettingsKey::Left),
        key if key == VK_RIGHT.0 => Some(SettingsKey::Right),
        key if key == VK_UP.0 => Some(SettingsKey::Up),
        key if key == VK_DOWN.0 => Some(SettingsKey::Down),
        key if key == VK_SPACE.0 => Some(SettingsKey::Space),
        _ => None,
    }
}

fn push_search_text_unit(hwnd: HWND, wparam: WPARAM) {
    let Ok(unit) = u16::try_from(wparam.0) else {
        return;
    };
    with_window_state(hwnd, |state| {
        let character = decode_text_unit(&state.pending_high_surrogate, unit);
        if let Some(character) = character {
            state.push_event(SearchEvent::TextInput(character));
        }
    });
}

fn decode_text_unit(
    pending_high_surrogate: &std::cell::Cell<Option<u16>>,
    unit: u16,
) -> Option<char> {
    if (0xD800..=0xDBFF).contains(&unit) {
        pending_high_surrogate.set(Some(unit));
        None
    } else if (0xDC00..=0xDFFF).contains(&unit) {
        pending_high_surrogate
            .replace(None)
            .and_then(|high| char::decode_utf16([high, unit]).next().and_then(Result::ok))
    } else {
        pending_high_surrogate.set(None);
        char::from_u32(u32::from(unit)).filter(|character| !character.is_control())
    }
}

fn wheel_selection_direction(wparam: WPARAM) -> Option<SelectionDirection> {
    let bits = u16::try_from((wparam.0 >> 16) & 0xFFFF).ok()?;
    match i16::from_ne_bytes(bits.to_ne_bytes()).cmp(&0) {
        std::cmp::Ordering::Greater => Some(SelectionDirection::Previous),
        std::cmp::Ordering::Less => Some(SelectionDirection::Next),
        std::cmp::Ordering::Equal => None,
    }
}
