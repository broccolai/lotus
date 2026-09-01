use std::rc::Rc;

use lotus_ui::geometry::NonZeroPhysicalSize;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    WINDOW_EX_STYLE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::w;

use crate::NativeError;

type Result<T> = std::result::Result<T, NativeError>;

use crate::platform::windows::backdrop;
use crate::platform::windows::display::nearest_display_to_point;
use crate::platform::windows::native_window::{NativeWindow, WindowCreation, WindowHandle};
use crate::window::procedure::{ContextMenuEvent, SignedPoint, WindowClass, WindowState};
use crate::window::transient::TransientWindow;

pub struct ContextMenuWindow {
    window: TransientWindow,
    _class: Rc<WindowClass>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PopupAlignment {
    Start,
    #[default]
    Center,
    End,
}

impl ContextMenuWindow {
    pub(super) fn create(class: Rc<WindowClass>, owner: HWND) -> Result<Self> {
        let extended_style = WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0);
        let window = NativeWindow::create(
            WindowCreation {
                instance: class.instance(),
                class_name: WindowClass::NAME,
                title: w!("Lotus actions"),
                extended_style,
                style: WS_POPUP,
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                owner: Some(owner),
            },
            Box::new(WindowState::context_menu()),
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

    pub fn prepare_at(
        &mut self,
        anchor: SignedPoint,
        alignment: PopupAlignment,
        size: NonZeroPhysicalSize,
    ) -> Result<u32> {
        let display = nearest_display_to_point(anchor.x, anchor.y)?;
        let width = i32::try_from(size.width()).unwrap_or(i32::MAX);
        let height = i32::try_from(size.height()).unwrap_or(i32::MAX);
        let preferred_x = match alignment {
            PopupAlignment::Start => anchor.x,
            PopupAlignment::Center => anchor.x.saturating_sub(width / 2),
            PopupAlignment::End => anchor.x.saturating_sub(width),
        };
        let preferred_y = anchor.y.saturating_sub(height);
        let (x, y) = display.work_area.clamp_origin_for_size(
            preferred_x,
            preferred_y,
            width,
            height,
        );

        self.window.prepare_topmost(x, y, width, height)?;
        Ok(display.dpi()?.dpi())
    }

    pub fn show(&mut self) {
        self.window.show_and_focus();
    }

    pub fn hide(&mut self) {
        self.window.hide();
    }

    pub fn drain_events(&mut self) -> impl Iterator<Item = ContextMenuEvent> + '_ {
        self.window.state_mut().drain_events().into_iter()
    }

    pub fn has_pending_events(&self) -> bool {
        self.window.state().has_pending_events()
    }
}
