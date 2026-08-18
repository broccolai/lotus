use std::mem::size_of;
use std::sync::{Mutex, PoisonError};

use thiserror::Error;
use windows::Win32::Foundation::COLORREF;
use windows::Win32::UI::Controls::Dialogs::{
    CC_ANYCOLOR, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW, ChooseColorW, CommDlgExtendedError,
};

use crate::WindowHandle;

static CUSTOM_COLORS: Mutex<[COLORREF; 16]> = Mutex::new([COLORREF(0); 16]);

#[derive(Debug, Error)]
pub enum ColorPickerError {
    #[error("the native color picker failed with code {0:#x}")]
    Native(u32),
}

pub fn choose_color(
    owner: WindowHandle,
    initial: &str,
) -> Result<Option<String>, ColorPickerError> {
    let mut custom_colors = CUSTOM_COLORS.lock().unwrap_or_else(PoisonError::into_inner);
    let mut choice = CHOOSECOLORW {
        lStructSize: u32::try_from(size_of::<CHOOSECOLORW>()).unwrap_or(u32::MAX),
        hwndOwner: owner.raw(),
        rgbResult: to_color_ref(initial),
        lpCustColors: custom_colors.as_mut_ptr(),
        Flags: CC_ANYCOLOR | CC_FULLOPEN | CC_RGBINIT,
        ..CHOOSECOLORW::default()
    };
    if unsafe { ChooseColorW(&raw mut choice) }.as_bool() {
        return Ok(Some(from_color_ref(choice.rgbResult)));
    }
    let error = unsafe { CommDlgExtendedError() }.0;
    if error == 0 {
        Ok(None)
    } else {
        Err(ColorPickerError::Native(error))
    }
}

fn to_color_ref(value: &str) -> COLORREF {
    let value = value.trim().strip_prefix('#').unwrap_or_default();
    let rgb = u32::from_str_radix(value, 16).unwrap_or(0x11_14_1A);
    COLORREF(((rgb >> 16) & 0xFF) | (rgb & 0x00_FF_00) | ((rgb & 0xFF) << 16))
}

fn from_color_ref(value: COLORREF) -> String {
    let red = value.0 & 0xFF;
    let green = (value.0 >> 8) & 0xFF;
    let blue = (value.0 >> 16) & 0xFF;
    format!("#{red:02X}{green:02X}{blue:02X}")
}
