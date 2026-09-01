use std::rc::Rc;

use lotus_core::window::WindowId;
use lotus_ui::geometry::NonZeroPhysicalSize;
use windows::Win32::UI::WindowsAndMessaging::{
    WINDOW_EX_STYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::w;

use super::procedure::{SwitcherEvent, WindowClass};
use crate::NativeError;
use crate::platform::windows::backdrop;
use crate::platform::windows::display::primary_display;
use crate::platform::windows::interaction::PointerCursor;
use crate::platform::windows::native_window::{NativeWindow, WindowCreation, WindowHandle};
use crate::window::procedure::WindowState;
use crate::window::transient::TransientWindow;

type Result<T> = std::result::Result<T, NativeError>;

pub struct SwitcherWindow {
    window: TransientWindow,
    _class: Rc<WindowClass>,
}

impl SwitcherWindow {
    pub(super) fn create(class: Rc<WindowClass>) -> Result<Self> {
        let window = NativeWindow::create(
            WindowCreation {
                instance: class.instance(),
                class_name: WindowClass::NAME,
                title: w!("Lotus Application Switcher"),
                extended_style: WINDOW_EX_STYLE(
                    WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0 | WS_EX_NOACTIVATE.0,
                ),
                style: WS_POPUP,
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                owner: None,
            },
            Box::new(WindowState::switcher()),
        )?;
        backdrop::apply_context_menu(window.hwnd());
        Ok(Self {
            window: TransientWindow::new(window),
            _class: class,
        })
    }

    pub fn handle(&self) -> WindowHandle {
        self.window.handle()
    }

    pub fn dpi(&self) -> u32 {
        self.window.dpi()
    }

    pub fn show_centered(
        &mut self,
        _anchor: Option<WindowId>,
        size: NonZeroPhysicalSize,
    ) -> Result<u32> {
        let display = primary_display()?;
        let width = i32::try_from(size.width()).unwrap_or(i32::MAX);
        let height = i32::try_from(size.height()).unwrap_or(i32::MAX);
        let (x, y) = display.work_area.centered_origin(width, height);
        self.window
            .prepare_and_show_topmost_inactive(x, y, width, height)?;
        super::procedure::apply_rounded_region(self.window.hwnd(), 0);
        Ok(display.dpi()?.dpi())
    }

    pub fn hide(&mut self) {
        self.window.hide();
    }

    pub fn set_pointer_cursor(&self, cursor: PointerCursor) {
        self.window.state().set_pointer_cursor(cursor);
    }

    pub fn drain_events(&mut self) -> impl Iterator<Item = SwitcherEvent> + '_ {
        self.window.state_mut().drain_events().into_iter()
    }

    pub fn has_pending_events(&self) -> bool {
        self.window.state().has_pending_events()
    }
}
