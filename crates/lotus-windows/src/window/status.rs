use std::rc::Rc;

use lotus_core::settings::{DockSettings, DockZone};
use lotus_ui::geometry::NonZeroPhysicalSize;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, WINDOW_EX_STYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};
use windows::core::w;

use super::procedure::{WindowClass, WindowEvent, WindowState, apply_rounded_region};
use crate::NativeError;
use crate::platform::windows::backdrop;
use crate::platform::windows::display::primary_display;
use crate::platform::windows::native_window::{
    Activation, NativeWindow, WindowCreation, WindowHandle,
};

type Result<T> = std::result::Result<T, NativeError>;

pub struct StatusWindow {
    window: NativeWindow<WindowState>,
    _class: Rc<WindowClass>,
}

impl StatusWindow {
    pub(super) fn create(class: Rc<WindowClass>, owner: HWND) -> Result<Self> {
        let display = primary_display()?;
        let extended_style =
            WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0 | WS_EX_TOPMOST.0);
        let window = NativeWindow::create(
            WindowCreation {
                instance: class.instance(),
                class_name: WindowClass::NAME,
                title: w!("Lotus system status"),
                extended_style,
                style: WS_POPUP,
                x: display.bounds.right - 1,
                y: display.bounds.bottom - 1,
                width: 1,
                height: 1,
                owner: Some(owner),
            },
            Box::new(WindowState::status()),
        )?;
        backdrop::apply(window.hwnd());
        Ok(Self {
            window,
            _class: class,
        })
    }

    pub fn handle(&self) -> WindowHandle {
        self.window.handle()
    }

    pub fn dpi(&self) -> u32 {
        self.window.dpi().dpi()
    }

    pub(super) fn place_aligned(
        &self,
        dock: HWND,
        size: NonZeroPhysicalSize,
        zone: DockZone,
        settings: &DockSettings,
    ) -> Result<()> {
        let display = primary_display()?;
        let mut dock_bounds = RECT::default();
        // SAFETY: Both HWND and writable RECT belong to the current process.
        unsafe { GetWindowRect(dock, &raw mut dock_bounds)? };
        let width = i32::try_from(size.width()).unwrap_or(i32::MAX);
        let height = i32::try_from(size.height()).unwrap_or(i32::MAX);
        let edge_gap = display
            .bounds
            .bottom
            .saturating_sub(dock_bounds.bottom)
            .max(0);
        let x = match zone {
            DockZone::Left => display.bounds.left.saturating_add(edge_gap),
            DockZone::Center => display.bounds.left.saturating_add(
                display
                    .bounds
                    .right
                    .saturating_sub(display.bounds.left)
                    .saturating_sub(width)
                    / 2,
            ),
            DockZone::Right => display
                .bounds
                .right
                .saturating_sub(edge_gap)
                .saturating_sub(width),
        };
        let y = display
            .bounds
            .bottom
            .saturating_sub(edge_gap)
            .saturating_sub(height);

        self.window
            .state()
            .set_corner_radius(settings.corner_radius);
        self.window
            .place_topmost(x, y, width, height, Activation::KeepInactive, false)?;
        apply_rounded_region(self.window.hwnd(), settings.corner_radius);
        Ok(())
    }

    pub fn set_visible(&self, visible: bool) {
        if visible {
            self.window.reveal(Activation::KeepInactive);
        } else {
            self.window.hide();
        }
    }

    pub fn set_animation_active(&self, active: bool) -> Result<()> {
        self.window
            .state()
            .set_animation_active(self.window.hwnd(), active)
    }

    pub fn drain_events(&mut self) -> impl Iterator<Item = WindowEvent> + '_ {
        self.window.state_mut().drain()
    }
}
