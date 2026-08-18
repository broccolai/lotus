use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex};
use windows::core::w;

use crate::NativeError;

pub struct SingleInstance {
    mutex: HANDLE,
}

impl SingleInstance {
    pub fn acquire() -> Result<Option<Self>, NativeError> {
        let mutex =
            unsafe { CreateMutexW(None, true, w!(r"Local\Lotus.Dock.SingleInstance"))? };
        let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;

        if already_exists {
            unsafe { CloseHandle(mutex)? };
            return Ok(None);
        }

        Ok(Some(Self { mutex }))
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.mutex);
            let _ = CloseHandle(self.mutex);
        }
    }
}
