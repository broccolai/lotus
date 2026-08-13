use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex};
use windows::core::w;

use crate::NativeError;

pub struct SingleInstance {
    mutex: HANDLE,
}

impl SingleInstance {
    pub fn acquire() -> Result<Option<Self>, NativeError> {
        // SAFETY: The static name is NUL-terminated and the returned handle is owned below.
        let mutex =
            unsafe { CreateMutexW(None, true, w!(r"Local\Lotus.Dock.SingleInstance"))? };
        // SAFETY: This immediately follows CreateMutexW without an intervening Win32 call.
        let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;

        if already_exists {
            // SAFETY: CreateMutexW returned this valid handle without granting mutex ownership.
            unsafe { CloseHandle(mutex)? };
            return Ok(None);
        }

        Ok(Some(Self { mutex }))
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        // SAFETY: The guard owns the initially acquired mutex handle exactly once.
        unsafe {
            let _ = ReleaseMutex(self.mutex);
            let _ = CloseHandle(self.mutex);
        }
    }
}
