use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    IDYES, MB_DEFBUTTON2, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_YESNO,
    MessageBoxW,
};
use windows::core::HSTRING;

use crate::WindowHandle;

pub fn show_error(owner: WindowHandle, title: &str, message: &str) {
    show_error_for(owner.raw(), title, message);
}

pub fn show_unowned_error(title: &str, message: &str) {
    show_error_for(HWND::default(), title, message);
}

pub fn show_information(owner: WindowHandle, title: &str, message: &str) {
    let title = HSTRING::from(title);
    let message = HSTRING::from(message);
    // SAFETY: Strings remain live for the synchronous dialog and `owner` is a live Lotus HWND.
    unsafe {
        let _ = MessageBoxW(Some(owner.raw()), &message, &title, MB_OK | MB_ICONINFORMATION);
    }
}

pub fn confirm_install_update(owner: WindowHandle, version: &str, installed: bool) -> bool {
    let title = HSTRING::from(if installed { "Update Lotus" } else { "Install Lotus" });
    let action = if installed { "Download and install" } else { "Install" };
    let message =
        HSTRING::from(format!("{action} Lotus {version}?\n\nLotus will restart when it is ready."));
    // SAFETY: Strings remain live for the synchronous dialog and `owner` is a live Lotus HWND.
    unsafe {
        MessageBoxW(Some(owner.raw()), &message, &title, MB_YESNO | MB_ICONINFORMATION) == IDYES
    }
}

fn show_error_for(owner: HWND, title: &str, message: &str) {
    let title = HSTRING::from(title);
    let message = HSTRING::from(message);
    // SAFETY: Both strings remain alive for the synchronous call and `owner`
    // is either Lotus's live dock HWND or a null handle supplied by the caller.
    unsafe {
        let _ = MessageBoxW(Some(owner), &message, &title, MB_OK | MB_ICONERROR);
    }
}

pub fn confirm_shutdown(owner: WindowHandle) -> bool {
    let title = HSTRING::from("Lotus");
    let message = HSTRING::from("Shut down this PC now?");
    // SAFETY: Strings remain live for the synchronous dialog and `owner` is Lotus's dock HWND.
    unsafe {
        MessageBoxW(Some(owner.raw()), &message, &title, MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2)
            == IDYES
    }
}
