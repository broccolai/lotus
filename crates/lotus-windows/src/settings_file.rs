use std::ffi::c_void;
use std::path::PathBuf;
use std::string::FromUtf16Error;

use thiserror::Error;
use windows::Win32::Foundation::ERROR_CANCELLED;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST, FileSaveDialog,
    IFileSaveDialog, SIGDN_FILESYSPATH,
};
use windows::core::{Error as WindowsError, PCWSTR, w};

use crate::launch::ComApartment;
use crate::{NativeError, WindowHandle};

#[derive(Debug, Error)]
pub enum SettingsFileError {
    #[error("could not initialize the native settings export dialog")]
    ComUnavailable,
    #[error(transparent)]
    Native(#[from] NativeError),
    #[error("the settings export dialog returned an invalid path")]
    InvalidPath(#[from] FromUtf16Error),
}

impl From<WindowsError> for SettingsFileError {
    fn from(error: WindowsError) -> Self {
        Self::Native(error.into())
    }
}

pub fn choose_export_path(
    owner: WindowHandle,
) -> Result<Option<PathBuf>, SettingsFileError> {
    choose_path(owner, ExportKind::Settings)
}

pub fn choose_diagnostics_export_path(
    owner: WindowHandle,
    file_name: &str,
) -> Result<Option<PathBuf>, SettingsFileError> {
    choose_path(owner, ExportKind::Diagnostics(file_name))
}

#[derive(Clone, Copy)]
enum ExportKind<'a> {
    Settings,
    Diagnostics(&'a str),
}

fn choose_path(
    owner: WindowHandle,
    kind: ExportKind<'_>,
) -> Result<Option<PathBuf>, SettingsFileError> {
    let (filter_name, filter_spec, file_name, extension) = match kind {
        ExportKind::Settings => (
            w!("JSON settings"),
            w!("*.json"),
            "lotus-settings.json",
            w!("json"),
        ),
        ExportKind::Diagnostics(file_name) => {
            (w!("Text diagnostics"), w!("*.txt"), file_name, w!("txt"))
        }
    };
    let file_name = windows::core::HSTRING::from(file_name);
    let _apartment = ComApartment::enter().ok_or(SettingsFileError::ComUnavailable)?;
    let dialog: IFileSaveDialog =
        unsafe { CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER) }?;
    let filters = [
        COMDLG_FILTERSPEC {
            pszName: filter_name,
            pszSpec: filter_spec,
        },
        COMDLG_FILTERSPEC {
            pszName: w!("All files"),
            pszSpec: w!("*.*"),
        },
    ];
    unsafe {
        dialog.SetFileTypes(&filters)?;
        dialog.SetFileName(&file_name)?;
        dialog.SetDefaultExtension(extension)?;
        dialog.SetOptions(FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_OVERWRITEPROMPT)?;
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
