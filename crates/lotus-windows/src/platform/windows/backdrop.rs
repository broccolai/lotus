use std::ffi::c_void;
use std::mem::{size_of, size_of_val};

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWMSBT_NONE, DWMSBT_TRANSIENTWINDOW, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
    DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_ROUND, DWMWCP_ROUNDSMALL, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::UI::Controls::MARGINS;
use windows::core::{BOOL, s, w};

use lotus_core::settings::DockSettings;

use crate::WindowHandle;

const WINDOW_COMPOSITION_ATTRIBUTE_ACCENT_POLICY: i32 = 19;
const ACCENT_ENABLE_ACRYLIC_BLUR_BEHIND: i32 = 4;
const DEFAULT_ACRYLIC_TINT: u32 = 0x8F1A_1411;
const STRONG_UI_ACRYLIC_ALPHA: u32 = 0xB3;
const POPUP_ELEVATION_ALPHA: u32 = 0x1F;

#[repr(C)]
struct AccentPolicy {
    state: i32,
    flags: i32,
    gradient_color: u32,
    animation_id: i32,
}

#[repr(C)]
struct CompositionAttributeData {
    attribute: i32,
    data: *mut c_void,
    size: usize,
}

type SetWindowCompositionAttribute =
    unsafe extern "system" fn(HWND, *mut CompositionAttributeData) -> BOOL;

pub(crate) fn apply(hwnd: HWND) {
    apply_common(hwnd, DWMWCP_ROUNDSMALL);

    let border_color = DWMWA_COLOR_NONE;

    // SAFETY: The attribute pointer references a correctly typed, initialized value and remains
    // valid for the duration of its synchronous DWM call.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            (&raw const border_color).cast::<c_void>(),
            value_size_u32(&border_color),
        );
    }
}

pub fn apply_dock_settings(window: WindowHandle, settings: &DockSettings) {
    let hwnd = window.raw();
    apply(hwnd);
    let _ = apply_explicit_acrylic(hwnd, acrylic_tint(settings));
}

pub(crate) fn apply_search_popup(hwnd: HWND) {
    apply_common(hwnd, DWMWCP_ROUND);
}

pub(crate) fn apply_context_menu(hwnd: HWND) {
    apply_common(hwnd, DWMWCP_ROUND);
    let tint = (DEFAULT_ACRYLIC_TINT & 0x00FF_FFFF) | (STRONG_UI_ACRYLIC_ALPHA << 24);
    let _ = apply_explicit_acrylic(hwnd, tint);
}

pub fn apply_search_settings(window: WindowHandle, settings: &DockSettings) {
    apply_popup_settings(window, settings);
}

pub fn apply_popup_settings(window: WindowHandle, settings: &DockSettings) {
    let hwnd = window.raw();
    apply_search_popup(hwnd);
    let tint = acrylic_tint(settings);
    let elevated_alpha = (tint >> 24).saturating_add(POPUP_ELEVATION_ALPHA).min(0xF2);
    let stronger_tint = (tint & 0x00FF_FFFF) | (elevated_alpha << 24);
    let _ = apply_explicit_acrylic(hwnd, stronger_tint);
}

pub(crate) fn apply_settings_window(hwnd: HWND) {
    let dark_mode = 1_i32;
    let corner = DWMWCP_ROUND;
    let backdrop = DWMSBT_NONE;
    let border_color = DWMWA_COLOR_NONE;
    // SAFETY: Every attribute pointer references a correctly typed local value that remains live
    // through its synchronous DWM call. No acrylic policy is installed for this window.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&raw const dark_mode).cast::<c_void>(),
            size_u32::<i32>(),
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const corner).cast::<c_void>(),
            value_size_u32(&corner),
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            (&raw const backdrop).cast::<c_void>(),
            value_size_u32(&backdrop),
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            (&raw const border_color).cast::<c_void>(),
            value_size_u32(&border_color),
        );
    }
}

fn apply_common(hwnd: HWND, corner: windows::Win32::Graphics::Dwm::DWM_WINDOW_CORNER_PREFERENCE) {
    let dark_mode = 1_i32;
    let backdrop = DWMSBT_TRANSIENTWINDOW;
    let margins =
        MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };

    // SAFETY: Every attribute pointer references a correctly typed, initialized value and
    // remains valid for the duration of its synchronous DWM call.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&raw const dark_mode).cast::<c_void>(),
            size_u32::<i32>(),
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const corner).cast::<c_void>(),
            value_size_u32(&corner),
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            (&raw const backdrop).cast::<c_void>(),
            value_size_u32(&backdrop),
        );
        let _ = DwmExtendFrameIntoClientArea(hwnd, &raw const margins);
    }

    let _ = apply_explicit_acrylic(hwnd, DEFAULT_ACRYLIC_TINT);
}

fn apply_explicit_acrylic(hwnd: HWND, tint: u32) -> bool {
    // SAFETY: user32.dll is loaded in every process that owns a Win32 window.
    let Ok(user32) = (unsafe { GetModuleHandleW(w!("user32.dll")) }) else {
        return false;
    };
    // SAFETY: The symbol is queried by its stable export name and checked for absence.
    let Some(procedure) = (unsafe { GetProcAddress(user32, s!("SetWindowCompositionAttribute")) })
    else {
        return false;
    };
    // SAFETY: This export has the SetWindowCompositionAttribute ABI represented by the alias.
    let set_attribute: SetWindowCompositionAttribute = unsafe { std::mem::transmute(procedure) };

    let mut policy = AccentPolicy {
        state: ACCENT_ENABLE_ACRYLIC_BLUR_BEHIND,
        flags: 2,
        gradient_color: tint,
        animation_id: 0,
    };
    let mut data = CompositionAttributeData {
        attribute: WINDOW_COMPOSITION_ATTRIBUTE_ACCENT_POLICY,
        data: (&raw mut policy).cast::<c_void>(),
        size: size_of::<AccentPolicy>(),
    };

    // SAFETY: The undocumented compatibility API is isolated here. Both ABI structures match
    // their Win32 layouts and remain valid for the duration of this synchronous call.
    unsafe { set_attribute(hwnd, &raw mut data) }.as_bool()
}

fn acrylic_tint(settings: &DockSettings) -> u32 {
    let color = settings.background_color.trim_start_matches('#');
    let rgb = u32::from_str_radix(&color[..color.len().min(6)], 16).unwrap_or(0x11_14_1A);
    let red = (rgb >> 16) & 0xFF;
    let green = (rgb >> 8) & 0xFF;
    let blue = rgb & 0xFF;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "normalized opacity is finite and clamped to 0.08..=0.95"
    )]
    let alpha = (settings.background_opacity * 255.0).round() as u32;
    (alpha << 24) | (blue << 16) | (green << 8) | red
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "Win32 structure sizes are fixed and far below u32::MAX"
)]
const fn size_u32<T>() -> u32 {
    size_of::<T>() as u32
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "Win32 attribute values are fixed-size ABI types far below u32::MAX"
)]
fn value_size_u32<T>(value: &T) -> u32 {
    size_of_val(value) as u32
}
