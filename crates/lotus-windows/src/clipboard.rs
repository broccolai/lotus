use std::mem::size_of;

use thiserror::Error;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};

use crate::NativeError;

const CF_UNICODETEXT: u32 = 13;

#[derive(Debug, Error)]
pub enum ClipboardError {
    #[error(transparent)]
    Native(#[from] NativeError),
    #[error("clipboard Unicode text is not valid UTF-16")]
    InvalidUnicode,
    #[error("clipboard Unicode text allocation has an invalid byte length")]
    InvalidAllocation,
}

pub fn read_text() -> Result<String, ClipboardError> {
    unsafe { OpenClipboard(None).map_err(NativeError::from)? };
    let _clipboard = OpenClipboardGuard;
    let handle = unsafe { GetClipboardData(CF_UNICODETEXT).map_err(NativeError::from)? };
    let global = HGLOBAL(handle.0);
    let pointer = unsafe { GlobalLock(global) }.cast::<u16>();
    if pointer.is_null() {
        return Err(NativeError::from(windows::core::Error::from_thread()).into());
    }
    let _lock = GlobalLockGuard(global);
    let bytes = unsafe { GlobalSize(global) };
    if bytes == 0 || !bytes.is_multiple_of(size_of::<u16>()) {
        return Err(ClipboardError::InvalidAllocation);
    }
    let units = unsafe { std::slice::from_raw_parts(pointer, bytes / size_of::<u16>()) };
    decode_unicode_text(units)
}

pub fn write_text(text: &str) -> Result<(), ClipboardError> {
    let units = text.encode_utf16().chain([0]).collect::<Vec<_>>();
    let bytes = units
        .len()
        .checked_mul(size_of::<u16>())
        .ok_or(ClipboardError::InvalidAllocation)?;
    let global = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes).map_err(NativeError::from)? };
    let mut allocation = GlobalAllocationGuard(Some(global));
    let pointer = unsafe { GlobalLock(global) }.cast::<u16>();
    if pointer.is_null() {
        return Err(NativeError::from(windows::core::Error::from_thread()).into());
    }
    unsafe { pointer.copy_from_nonoverlapping(units.as_ptr(), units.len()) };
    let _ = unsafe { GlobalUnlock(global) };

    unsafe { OpenClipboard(None).map_err(NativeError::from)? };
    let _clipboard = OpenClipboardGuard;
    unsafe { EmptyClipboard().map_err(NativeError::from)? };
    unsafe {
        SetClipboardData(CF_UNICODETEXT, Some(HANDLE(global.0)))
            .map_err(NativeError::from)?;
    }
    allocation.0 = None;
    Ok(())
}

fn decode_unicode_text(units: &[u16]) -> Result<String, ClipboardError> {
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len());
    String::from_utf16(&units[..end]).map_err(|_| ClipboardError::InvalidUnicode)
}

struct OpenClipboardGuard;

impl Drop for OpenClipboardGuard {
    fn drop(&mut self) {
        let _ = unsafe { CloseClipboard() };
    }
}

struct GlobalLockGuard(HGLOBAL);

impl Drop for GlobalLockGuard {
    fn drop(&mut self) {
        let _ = unsafe { GlobalUnlock(self.0) };
    }
}

struct GlobalAllocationGuard(Option<HGLOBAL>);

impl Drop for GlobalAllocationGuard {
    fn drop(&mut self) {
        if let Some(global) = self.0.take() {
            let _ = unsafe { GlobalFree(Some(global)) };
        }
    }
}
