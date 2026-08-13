use std::mem::size_of;

use thiserror::Error;
use windows::Win32::Foundation::HGLOBAL;
use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

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
    // SAFETY: A null owner opens the clipboard for this short synchronous read.
    unsafe { OpenClipboard(None).map_err(NativeError::from)? };
    let _clipboard = OpenClipboardGuard;
    // SAFETY: The clipboard remains open and the requested format is standard Unicode text.
    let handle = unsafe { GetClipboardData(CF_UNICODETEXT).map_err(NativeError::from)? };
    let global = HGLOBAL(handle.0);
    // SAFETY: The clipboard owns the handle and the lock is retained while copying.
    let pointer = unsafe { GlobalLock(global) }.cast::<u16>();
    if pointer.is_null() {
        return Err(NativeError::from(windows::core::Error::from_thread()).into());
    }
    let _lock = GlobalLockGuard(global);
    // SAFETY: The clipboard allocation remains locked during this size query.
    let bytes = unsafe { GlobalSize(global) };
    if bytes == 0 || !bytes.is_multiple_of(size_of::<u16>()) {
        return Err(ClipboardError::InvalidAllocation);
    }
    // SAFETY: GlobalSize bounds the locked allocation and the pointer is UTF-16 aligned.
    let units = unsafe { std::slice::from_raw_parts(pointer, bytes / size_of::<u16>()) };
    decode_unicode_text(units)
}

fn decode_unicode_text(units: &[u16]) -> Result<String, ClipboardError> {
    let end = units.iter().position(|unit| *unit == 0).unwrap_or(units.len());
    String::from_utf16(&units[..end]).map_err(|_| ClipboardError::InvalidUnicode)
}

struct OpenClipboardGuard;

impl Drop for OpenClipboardGuard {
    fn drop(&mut self) {
        // SAFETY: This guard exists only after OpenClipboard succeeds.
        let _ = unsafe { CloseClipboard() };
    }
}

struct GlobalLockGuard(HGLOBAL);

impl Drop for GlobalLockGuard {
    fn drop(&mut self) {
        // SAFETY: This guard owns one successful GlobalLock.
        let _ = unsafe { GlobalUnlock(self.0) };
    }
}
