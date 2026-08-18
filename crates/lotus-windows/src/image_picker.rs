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
    unsafe {
        dialog.SetFileTypes(&filters)?;
        dialog.SetOptions(FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST)?;
    }
    match unsafe { dialog.Show(Some(owner.raw())) } {
        Ok(()) => {}
        Err(error) if error.code() == ERROR_CANCELLED.into() => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let item = unsafe { dialog.GetResult() }?;
    let path = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }?;
    let path = TaskPath(path);
    let value = unsafe { PCWSTR(path.0.0).to_string() }?;
    Ok(Some(PathBuf::from(value)))
}

struct TaskPath(windows::core::PWSTR);

impl Drop for TaskPath {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0.0.cast::<c_void>())) };
    }
}
