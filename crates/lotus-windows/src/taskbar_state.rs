use std::mem::size_of;

use thiserror::Error;
use windows::Win32::Foundation::LPARAM;
use windows::Win32::UI::Shell::{
    ABM_GETSTATE, ABM_SETSTATE, ABS_AUTOHIDE, APPBARDATA, SHAppBarMessage,
};
use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
use windows::core::{PCWSTR, w};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskbarState(u32);

impl TaskbarState {
    pub const fn bits(self) -> u32 {
        self.0
    }

    const fn has_autohide(self) -> bool {
        self.0 & ABS_AUTOHIDE != 0
    }

    const fn with_autohide(self) -> Self {
        Self(self.0 | ABS_AUTOHIDE)
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum TaskbarStateError {
    #[error("the Windows taskbar window could not be found")]
    MissingTaskbar,
    #[error("taskbar state value {0} does not fit the native state domain")]
    StateOutOfRange(usize),
    #[error("taskbar rejected state flags 0x{requested:08X}")]
    SetRejected { requested: u32 },
    #[error("taskbar state guard has already been restored")]
    AlreadyRestored,
}

pub struct TaskbarStateGuard {
    journal: StateJournal<ShellTaskbarState>,
}

impl TaskbarStateGuard {
    pub fn enable_autohide() -> Result<Self, TaskbarStateError> {
        StateJournal::enable(ShellTaskbarState).map(|journal| Self { journal })
    }

    pub fn ensure_autohide(&mut self) -> Result<bool, TaskbarStateError> {
        self.journal.ensure_autohide()
    }

    pub fn restore(&mut self) -> Result<bool, TaskbarStateError> {
        self.journal.restore()
    }

    pub const fn original_state(&self) -> TaskbarState {
        self.journal.original
    }
}

trait TaskbarStateApi {
    fn state(&mut self) -> Result<TaskbarState, TaskbarStateError>;
    fn set_state(&mut self, state: TaskbarState) -> Result<(), TaskbarStateError>;
}

struct StateJournal<B: TaskbarStateApi> {
    backend: B,
    original: TaskbarState,
    modified: bool,
    restored: bool,
}

impl<B: TaskbarStateApi> StateJournal<B> {
    fn enable(mut backend: B) -> Result<Self, TaskbarStateError> {
        let original = backend.state()?;
        let mut journal = Self {
            backend,
            original,
            modified: false,
            restored: false,
        };
        let _ = journal.ensure_autohide()?;
        Ok(journal)
    }

    fn ensure_autohide(&mut self) -> Result<bool, TaskbarStateError> {
        if self.restored {
            return Err(TaskbarStateError::AlreadyRestored);
        }
        let current = self.backend.state()?;
        if current.has_autohide() {
            return Ok(false);
        }
        self.backend.set_state(current.with_autohide())?;
        self.modified = true;
        Ok(true)
    }

    fn restore(&mut self) -> Result<bool, TaskbarStateError> {
        if self.restored {
            return Ok(false);
        }
        if self.modified {
            self.backend.set_state(self.original)?;
        }
        self.restored = true;
        Ok(self.modified)
    }
}

impl<B: TaskbarStateApi> Drop for StateJournal<B> {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

struct ShellTaskbarState;

impl TaskbarStateApi for ShellTaskbarState {
    fn state(&mut self) -> Result<TaskbarState, TaskbarStateError> {
        let mut data = appbar_data(None)?;
        // SAFETY: `data` has the correct ABI size and remains writable for the
        // synchronous AppBar query. ABM_GETSTATE returns the state flags.
        let state = unsafe { SHAppBarMessage(ABM_GETSTATE, &raw mut data) };
        u32::try_from(state)
            .map(TaskbarState)
            .map_err(|_| TaskbarStateError::StateOutOfRange(state))
    }

    fn set_state(&mut self, state: TaskbarState) -> Result<(), TaskbarStateError> {
        let parameter = isize::try_from(state.bits())
            .map_err(|_| TaskbarStateError::StateOutOfRange(state.bits() as usize))?;
        let mut data = appbar_data(Some(parameter))?;
        // SAFETY: `data` has the correct ABI size and lParam contains only the
        // captured state flags plus, when requested, ABS_AUTOHIDE.
        let accepted = unsafe { SHAppBarMessage(ABM_SETSTATE, &raw mut data) };
        if accepted == 0 {
            return Err(TaskbarStateError::SetRejected {
                requested: state.bits(),
            });
        }
        Ok(())
    }
}

fn appbar_data(parameter: Option<isize>) -> Result<APPBARDATA, TaskbarStateError> {
    let size = u32::try_from(size_of::<APPBARDATA>())
        .map_err(|_| TaskbarStateError::StateOutOfRange(size_of::<APPBARDATA>()))?;
    Ok(APPBARDATA {
        cbSize: size,
        hWnd: taskbar_window()?,
        lParam: LPARAM(parameter.unwrap_or_default()),
        ..APPBARDATA::default()
    })
}

fn taskbar_window() -> Result<windows::Win32::Foundation::HWND, TaskbarStateError> {
    // SAFETY: Both search strings are static and nul-terminated. A null title
    // requests the first top-level window with the primary taskbar class.
    unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }
        .map_err(|_| TaskbarStateError::MissingTaskbar)
}
