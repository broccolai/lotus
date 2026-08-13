use lotus_ui::geometry::DpiScale;
use windows::Win32::Foundation::{HINSTANCE, HWND, RECT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetClientRect, HWND_TOPMOST, IsWindow, IsWindowVisible,
    SET_WINDOW_POS_FLAGS, SW_HIDE, SW_SHOW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SetWindowPos, ShowWindow, WINDOW_EX_STYLE,
    WINDOW_STYLE,
};
use windows::core::PCWSTR;

use crate::NativeError;

type Result<T> = std::result::Result<T, NativeError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowHandle(HWND);

impl WindowHandle {
    pub(crate) const fn from_raw(hwnd: HWND) -> Self {
        Self(hwnd)
    }

    pub(crate) const fn raw(self) -> HWND {
        self.0
    }
}

pub fn current_instance() -> Result<HINSTANCE> {
    // SAFETY: Passing None requests the module handle for the current process.
    let module = unsafe { GetModuleHandleW(None)? };
    Ok(HINSTANCE(module.0))
}

#[derive(Clone, Copy)]
pub struct WindowCreation {
    pub instance: HINSTANCE,
    pub class_name: PCWSTR,
    pub title: PCWSTR,
    pub extended_style: WINDOW_EX_STYLE,
    pub style: WINDOW_STYLE,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub owner: Option<HWND>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Activation {
    Activate,
    KeepInactive,
}

pub struct NativeWindow<State> {
    hwnd: HWND,
    state: Box<State>,
}

impl<State> NativeWindow<State> {
    pub fn create(specification: WindowCreation, mut state: Box<State>) -> Result<Self> {
        let state_pointer =
            std::ptr::from_mut(state.as_mut()).cast::<std::ffi::c_void>().cast_const();
        // SAFETY: The caller supplies a registered class. `state_pointer` points into the Box this
        // guard takes ownership of, so it remains stable until after the owned HWND is destroyed.
        let hwnd = unsafe {
            CreateWindowExW(
                specification.extended_style,
                specification.class_name,
                specification.title,
                specification.style,
                specification.x,
                specification.y,
                specification.width,
                specification.height,
                specification.owner,
                None,
                Some(specification.instance),
                Some(state_pointer),
            )?
        };
        Ok(Self { hwnd, state })
    }

    pub(crate) const fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub const fn handle(&self) -> WindowHandle {
        WindowHandle(self.hwnd)
    }

    pub const fn state(&self) -> &State {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    pub fn dpi(&self) -> DpiScale {
        // SAFETY: Reading per-window DPI does not mutate the owned live HWND.
        DpiScale::from_system(unsafe { GetDpiForWindow(self.hwnd) })
    }

    pub fn client_size(&self) -> Result<(u32, u32)> {
        let mut bounds = RECT::default();
        // SAFETY: `bounds` is writable storage and this guard owns a live HWND.
        unsafe { GetClientRect(self.hwnd, &raw mut bounds)? };
        Ok((
            u32::try_from(bounds.right - bounds.left).unwrap_or_default(),
            u32::try_from(bounds.bottom - bounds.top).unwrap_or_default(),
        ))
    }

    pub fn place_topmost(
        &self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        activation: Activation,
        show: bool,
    ) -> Result<()> {
        let mut flags = if activation == Activation::KeepInactive {
            SWP_NOACTIVATE
        } else {
            SET_WINDOW_POS_FLAGS::default()
        };
        if show {
            flags |= SWP_SHOWWINDOW;
        }
        // SAFETY: This guard owns the HWND; geometry is already validated physical-pixel input.
        unsafe { SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, width, height, flags)? };
        Ok(())
    }

    pub fn reveal(&self, activation: Activation) {
        let command = match activation {
            Activation::Activate => SW_SHOW,
            Activation::KeepInactive => SW_SHOWNOACTIVATE,
        };
        // SAFETY: Showing this owned HWND transfers no ownership. The caller selects activation
        // explicitly so dock/search/settings semantics cannot be confused at call sites.
        let _ = unsafe { ShowWindow(self.hwnd, command) };
    }

    pub fn reveal_without_repositioning(&self) -> Result<()> {
        // SAFETY: The operation only changes visibility/z-order for this owned HWND.
        unsafe {
            SetWindowPos(
                self.hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_SHOWWINDOW,
            )?;
        }
        Ok(())
    }

    pub fn hide(&self) {
        // SAFETY: Hiding this owned HWND is reversible and transfers no ownership.
        let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
    }

    pub fn is_visible(&self) -> bool {
        // SAFETY: This is a read-only query against the owned HWND.
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }
}

impl<State> Drop for NativeWindow<State> {
    fn drop(&mut self) {
        // SAFETY: This guard owns the HWND and drops on its creating UI thread. The IsWindow check
        // prevents a second destruction after normal WM_DESTROY processing.
        unsafe {
            if IsWindow(Some(self.hwnd)).as_bool() {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}
