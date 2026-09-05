use std::marker::PhantomData;
use std::mem::size_of;
use std::rc::Rc;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DestroyWindow, IDC_ARROW, LoadCursorW,
    RegisterClassExW, SW_SHOW, ShowWindow, UnregisterClassW, WINDOW_EX_STYLE, WM_CLOSE,
    WM_DESTROY, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::NativeError;
use crate::launch::ComApartment;
use crate::platform::windows::backdrop;
use crate::platform::windows::interaction::request_exit;
use crate::platform::windows::native_window::{WindowHandle, current_instance};

type Result<T> = std::result::Result<T, NativeError>;

/// Keeps native icon extraction on an initialized, thread-affine COM apartment.
pub struct PhotoSession {
    _apartment: ComApartment,
    _thread: PhantomData<Rc<()>>,
}

impl PhotoSession {
    pub fn enter() -> Result<Self> {
        let apartment = ComApartment::enter().ok_or_else(|| {
            windows::core::Error::new(
                windows::Win32::Foundation::E_FAIL,
                "could not initialize photo mode COM apartment",
            )
        })?;
        Ok(Self {
            _apartment: apartment,
            _thread: PhantomData,
        })
    }
}

/// A self-contained window for deterministic renderer presentation capture.
///
/// This window deliberately has no product event routing, shell registration, or interaction
/// integration. Closing it ends only the photo-mode message loop.
pub struct PhotoWindow {
    hwnd: HWND,
    _class: PhotoWindowClass,
    thread_affinity: PhantomData<Rc<()>>,
}

impl PhotoWindow {
    pub fn create(width: u32, height: u32) -> Result<Self> {
        let width = i32::try_from(width).map_err(|_| invalid_size("width"))?;
        let height = i32::try_from(height).map_err(|_| invalid_size("height"))?;
        if width <= 0 || height <= 0 {
            return Err(invalid_size("dimensions"));
        }

        let class = PhotoWindowClass::register(current_instance()?)?;
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0),
                PhotoWindowClass::NAME,
                w!("Lotus Photo Mode"),
                WS_POPUP,
                64,
                64,
                width,
                height,
                None,
                None,
                Some(class.instance),
                None,
            )?
        };
        backdrop::apply(hwnd);
        Ok(Self {
            hwnd,
            _class: class,
            thread_affinity: PhantomData,
        })
    }

    pub const fn handle(&self) -> WindowHandle {
        WindowHandle::from_raw(self.hwnd)
    }

    pub fn show(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
        }
    }
}

impl Drop for PhotoWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

struct PhotoWindowClass {
    instance: HINSTANCE,
}

impl PhotoWindowClass {
    const NAME: PCWSTR = w!("Lotus.PhotoMode");

    fn register(instance: HINSTANCE) -> Result<Self> {
        let cursor = unsafe { LoadCursorW(None, IDC_ARROW)? };
        let class = WNDCLASSEXW {
            cbSize: size_u32::<WNDCLASSEXW>(),
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_procedure),
            hInstance: instance,
            hCursor: cursor,
            lpszClassName: Self::NAME,
            ..WNDCLASSEXW::default()
        };
        if unsafe { RegisterClassExW(&raw const class) } == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        Ok(Self { instance })
    }
}

impl Drop for PhotoWindowClass {
    fn drop(&mut self) {
        let _ = unsafe { UnregisterClassW(Self::NAME, Some(self.instance)) };
    }
}

unsafe extern "system" fn window_procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_CLOSE => {
            let _ = unsafe { DestroyWindow(hwnd) };
            LRESULT(0)
        }
        WM_DESTROY => {
            request_exit(0);
            LRESULT(0)
        }
        _ => unsafe {
            windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(
                hwnd, message, wparam, lparam,
            )
        },
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "WNDCLASSEXW is always smaller than u32::MAX"
)]
const fn size_u32<T>() -> u32 {
    size_of::<T>() as u32
}

fn invalid_size(subject: &str) -> NativeError {
    windows::core::Error::new(
        windows::core::HRESULT(0x8007_0057_u32.cast_signed()),
        format!("photo mode window {subject} is outside Win32 limits"),
    )
    .into()
}
