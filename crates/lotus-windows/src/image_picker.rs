use std::ffi::c_void;
use std::path::PathBuf;
use std::string::FromUtf16Error;

use thiserror::Error;
use windows::Win32::Foundation::ERROR_CANCELLED;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FileOpenDialog,
    IFileOpenDialog, SIGDN_FILESYSPATH,
};
use windows::core::{Error as WindowsError, PCWSTR, w};

use crate::launch::ComApartment;
use crate::{NativeError, WindowHandle};

#[derive(Debug, Error)]
pub enum ImagePickerError {
    #[error("could not initialize the native image picker")]
    ComUnavailable,
    #[error(transparent)]
    Native(#[from] NativeError),
    #[error("the image picker returned an invalid path")]
    InvalidPath(#[from] FromUtf16Error),
}

impl From<WindowsError> for ImagePickerError {
    fn from(error: WindowsError) -> Self {
        Self::Native(error.into())
    }
}

pub fn choose_image(owner: WindowHandle) -> Result<Option<PathBuf>, ImagePickerError> {
    let _apartment = ComApartment::enter().ok_or(ImagePickerError::ComUnavailable)?;
    // SAFETY: FileOpenDialog is an in-process COM class. COM is initialized for
    // this thread and the returned interface owns its reference.
    let dialog: IFileOpenDialog =
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }?;
    let filters = [
        COMDLG_FILTERSPEC {
            pszName: w!("Image files"),
            pszSpec: w!("*.png;*.jpg;*.jpeg;*.webp;*.gif;*.bmp;*.ico"),
        },
        COMDLG_FILTERSPEC {
            pszName: w!("All files"),
            pszSpec: w!("*.*"),
        },
    ];
    // SAFETY: Filter labels are static NUL-terminated UTF-16 strings, and the
    // dialog copies its configuration during these synchronous calls.
    unsafe {
        dialog.SetFileTypes(&filters)?;
        dialog.SetOptions(FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST)?;
    }
    // SAFETY: `owner` is Lotus's live settings HWND. Cancellation is represented
    // by the documented ERROR_CANCELLED result and is not an application error.
    match unsafe { dialog.Show(Some(owner.raw())) } {
        Ok(()) => {}
        Err(error) if error.code() == ERROR_CANCELLED.into() => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    // SAFETY: The dialog completed successfully and returns an owned shell item.
    let item = unsafe { dialog.GetResult() }?;
    // SAFETY: SIGDN_FILESYSPATH asks the filesystem-backed item for a task-memory
    // UTF-16 path that stays live until the guard frees it below.
    let path = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }?;
    let path = TaskPath(path);
    // SAFETY: The shell returned a valid NUL-terminated string owned by `path`.
    let value = unsafe { PCWSTR(path.0.0).to_string() }?;
    Ok(Some(PathBuf::from(value)))
}

struct TaskPath(windows::core::PWSTR);

impl Drop for TaskPath {
    fn drop(&mut self) {
        // SAFETY: This allocation came from IShellItem::GetDisplayName and is
        // released once with the COM task allocator.
        unsafe { CoTaskMemFree(Some(self.0.0.cast::<c_void>())) };
    }
}
