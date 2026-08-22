use lotus_ui::geometry::DpiScale;
use windows::Win32::Foundation::{HINSTANCE, HWND, RECT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetClientRect, HWND_BOTTOM, HWND_TOPMOST, IsWindow,
    IsWindowVisible, SET_WINDOW_POS_FLAGS, SW_HIDE, SW_SHOW, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SetWindowPos,
    ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE,
};
use windows::core::PCWSTR;

use crate::NativeError;

type Result<T> = std::result::Result<T, NativeError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowHandle(HWND);

impl WindowHandle {
    pub(crate) const fn from_raw(window: HWND) -> Self {
        Self(window)
    }

    pub(crate) const fn raw(self) -> HWND {
        self.0
    }
}

pub fn current_instance() -> Result<HINSTANCE> {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WindowLayer {
    Bottom,
    Topmost,
}

#[derive(Clone, Copy)]
pub(crate) struct WindowPlacement {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub activation: Activation,
    pub show: bool,
}

pub struct NativeWindow<State> {
    hwnd: HWND,
    state: Box<State>,
    thread_affinity: PhantomData<Rc<()>>,
}

impl<State> NativeWindow<State> {
    pub fn create(specification: WindowCreation, mut state: Box<State>) -> Result<Self> {
        let state_pointer = std::ptr::from_mut(state.as_mut())
            .cast::<std::ffi::c_void>()
            .cast_const();
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
        Ok(Self {
            hwnd,
            state,
            thread_affinity: PhantomData,
        })
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
        DpiScale::from_system(unsafe { GetDpiForWindow(self.hwnd) })
    }

    pub fn client_size(&self) -> Result<(u32, u32)> {
        let mut bounds = RECT::default();
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
        self.place_at_layer(
            WindowLayer::Topmost,
            WindowPlacement {
                x,
                y,
                width,
                height,
                activation,
                show,
            },
        )
    }

    pub(crate) fn place_at_layer(
        &self,
        layer: WindowLayer,
        placement: WindowPlacement,
    ) -> Result<()> {
        let mut flags = if placement.activation == Activation::KeepInactive {
            SWP_NOACTIVATE
        } else {
            SET_WINDOW_POS_FLAGS::default()
        };
        if placement.show {
            flags |= SWP_SHOWWINDOW;
        }
        let insert_after = match layer {
            WindowLayer::Bottom => HWND_BOTTOM,
            WindowLayer::Topmost => HWND_TOPMOST,
        };
        unsafe {
            SetWindowPos(
                self.hwnd,
                Some(insert_after),
                placement.x,
                placement.y,
                placement.width,
                placement.height,
                flags,
            )?;
        }
        Ok(())
    }

    pub fn reveal(&self, activation: Activation) {
        let command = match activation {
            Activation::Activate => SW_SHOW,
            Activation::KeepInactive => SW_SHOWNOACTIVATE,
        };
        let _ = unsafe { ShowWindow(self.hwnd, command) };
    }

    pub fn reveal_without_repositioning(&self) -> Result<()> {
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

    pub(crate) fn present_at_layer(&self, layer: WindowLayer) -> Result<()> {
        let insert_after = match layer {
            WindowLayer::Bottom => HWND_BOTTOM,
            WindowLayer::Topmost => HWND_TOPMOST,
        };
        unsafe {
            SetWindowPos(
                self.hwnd,
                Some(insert_after),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )?;
        }
        Ok(())
    }

    pub fn hide(&self) {
        let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
    }

    pub fn is_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }
}

impl<State> Drop for NativeWindow<State> {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(Some(self.hwnd)).as_bool() {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}
use std::marker::PhantomData;
use std::rc::Rc;
