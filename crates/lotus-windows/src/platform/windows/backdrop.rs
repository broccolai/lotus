use std::ffi::c_void;
use std::mem::{size_of, size_of_val};
use std::sync::OnceLock;

use lotus_core::settings::DockSettings;
use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWMSBT_NONE, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_SYSTEMBACKDROP_TYPE,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DWMWCP_ROUNDSMALL, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::Win32::UI::Controls::MARGINS;
use windows::core::{BOOL, s, w};

use crate::WindowHandle;

const WINDOW_COMPOSITION_ATTRIBUTE_ACCENT_POLICY: i32 = 19;
const ACCENT_DISABLED: i32 = 0;
const ACCENT_ENABLE_ACRYLIC_BLUR_BEHIND: i32 = 4;
const DEFAULT_ACRYLIC_TINT: u32 = 0x8F1A_1411;
const WINDOWS_11_22H2_BUILD: u32 = 22_621;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum SettingsMaterial {
    Acrylic,
    Opaque,
}

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
    apply_unified_material(hwnd, DWMWCP_ROUNDSMALL, settings);
}

pub(crate) fn apply_search_popup(hwnd: HWND) {
    apply_common(hwnd, DWMWCP_ROUND);
}

pub(crate) fn apply_context_menu(hwnd: HWND) {
    apply_common(hwnd, DWMWCP_ROUND);
}

pub fn apply_search_settings(window: WindowHandle, settings: &DockSettings) {
    apply_popup_settings(window, settings);
}

pub fn apply_popup_settings(window: WindowHandle, settings: &DockSettings) {
    let hwnd = window.raw();
    apply_unified_material(hwnd, DWMWCP_ROUND, settings);
}

pub fn apply_context_menu_settings(window: WindowHandle, settings: &DockSettings) {
    let hwnd = window.raw();
    apply_unified_material(hwnd, DWMWCP_ROUNDSMALL, settings);
}

pub(crate) fn apply_settings_window(hwnd: HWND) {
    let dark_mode = 1_i32;
    let corner = DWMWCP_ROUND;
    let backdrop = DWMSBT_NONE;
    let border_color = DWMWA_COLOR_NONE;
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

pub(crate) fn apply_settings_material(hwnd: HWND, settings: &DockSettings) {
    apply_window_backdrop(hwnd, DWMWCP_ROUND, DWMSBT_NONE, Some(DWMWA_COLOR_NONE));
    if settings_uses_translucent_material(settings) {
        let _ = apply_explicit_acrylic(hwnd, acrylic_tint(settings));
    } else {
        let _ = disable_explicit_acrylic(hwnd);
    }
}

pub fn settings_uses_translucent_material(settings: &DockSettings) -> bool {
    settings.use_acrylic && settings_material() == SettingsMaterial::Acrylic
}

pub(crate) fn settings_material() -> SettingsMaterial {
    static MATERIAL: OnceLock<SettingsMaterial> = OnceLock::new();
    *MATERIAL.get_or_init(detect_settings_material)
}

fn apply_common(
    hwnd: HWND,
    corner: windows::Win32::Graphics::Dwm::DWM_WINDOW_CORNER_PREFERENCE,
) {
    apply_window_backdrop(hwnd, corner, DWMSBT_NONE, Some(DWMWA_COLOR_NONE));
    let _ = apply_explicit_acrylic(hwnd, DEFAULT_ACRYLIC_TINT);
}

fn apply_unified_material(
    hwnd: HWND,
    corner: windows::Win32::Graphics::Dwm::DWM_WINDOW_CORNER_PREFERENCE,
    settings: &DockSettings,
) {
    apply_window_backdrop(hwnd, corner, DWMSBT_NONE, Some(DWMWA_COLOR_NONE));
    if settings.use_acrylic {
        let _ = apply_explicit_acrylic(hwnd, acrylic_tint(settings));
    } else {
        let _ = disable_explicit_acrylic(hwnd);
    }
}

fn apply_window_backdrop(
    hwnd: HWND,
    corner: windows::Win32::Graphics::Dwm::DWM_WINDOW_CORNER_PREFERENCE,
    backdrop: windows::Win32::Graphics::Dwm::DWM_SYSTEMBACKDROP_TYPE,
    border_color: Option<u32>,
) {
    let dark_mode = 1_i32;
    let margins = MARGINS {
        cxLeftWidth: -1,
        cxRightWidth: -1,
        cyTopHeight: -1,
        cyBottomHeight: -1,
    };

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
        if let Some(border_color) = border_color {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_BORDER_COLOR,
                (&raw const border_color).cast::<c_void>(),
                value_size_u32(&border_color),
            );
        }
        let _ = DwmExtendFrameIntoClientArea(hwnd, &raw const margins);
    }
}

fn detect_settings_material() -> SettingsMaterial {
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_u32::<OSVERSIONINFOW>(),
        ..OSVERSIONINFOW::default()
    };
    let status = unsafe { RtlGetVersion(&raw mut version) };
    if status.is_ok() && version.dwBuildNumber >= WINDOWS_11_22H2_BUILD {
        SettingsMaterial::Acrylic
    } else {
        SettingsMaterial::Opaque
    }
}

fn apply_explicit_acrylic(hwnd: HWND, tint: u32) -> bool {
    let Ok(user32) = (unsafe { GetModuleHandleW(w!("user32.dll")) }) else {
        return false;
    };
    let Some(procedure) =
        (unsafe { GetProcAddress(user32, s!("SetWindowCompositionAttribute")) })
    else {
        return false;
    };
    let set_attribute: SetWindowCompositionAttribute =
        unsafe { std::mem::transmute(procedure) };

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

    unsafe { set_attribute(hwnd, &raw mut data) }.as_bool()
}

fn disable_explicit_acrylic(hwnd: HWND) -> bool {
    let Ok(user32) = (unsafe { GetModuleHandleW(w!("user32.dll")) }) else {
        return false;
    };
    let Some(procedure) =
        (unsafe { GetProcAddress(user32, s!("SetWindowCompositionAttribute")) })
    else {
        return false;
    };
    let set_attribute: SetWindowCompositionAttribute =
        unsafe { std::mem::transmute(procedure) };
    let mut policy = AccentPolicy {
        state: ACCENT_DISABLED,
        flags: 0,
        gradient_color: 0,
        animation_id: 0,
    };
    let mut data = CompositionAttributeData {
        attribute: WINDOW_COMPOSITION_ATTRIBUTE_ACCENT_POLICY,
        data: (&raw mut policy).cast::<c_void>(),
        size: size_of::<AccentPolicy>(),
    };

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
