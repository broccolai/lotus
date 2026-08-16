use std::rc::Rc;

use lotus_core::window::WindowId;
use lotus_ui::geometry::NonZeroPhysicalSize;
use windows::Win32::UI::WindowsAndMessaging::{
    WINDOW_EX_STYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::w;

use super::procedure::{PointerEvent, SwitcherEvent, WindowClass, WindowEvent};
use crate::NativeError;
use crate::platform::windows::backdrop;
use crate::platform::windows::display::primary_display;
use crate::platform::windows::interaction::PointerCursor;
use crate::platform::windows::native_window::{
    Activation, NativeWindow, WindowCreation, WindowHandle,
};
use crate::window::procedure::WindowState;

type Result<T> = std::result::Result<T, NativeError>;

pub struct SwitcherWindow {
    window: NativeWindow<WindowState>,
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

    pub fn show_centered(
        &mut self,
        _anchor: Option<WindowId>,
        size: NonZeroPhysicalSize,
    ) -> Result<u32> {
        let display = primary_display()?;
        let width = i32::try_from(size.width()).unwrap_or(i32::MAX);
        let height = i32::try_from(size.height()).unwrap_or(i32::MAX);
        let x = display.work_area.left.saturating_add(
            display
                .work_area
                .right
                .saturating_sub(display.work_area.left)
                .saturating_sub(width)
                / 2,
        );
        let y = display.work_area.top.saturating_add(
            display
                .work_area
                .bottom
                .saturating_sub(display.work_area.top)
                .saturating_sub(height)
                / 2,
        );
        self.window.state_mut().clear_events();
        self.window
            .place_topmost(x, y, width, height, Activation::KeepInactive, true)?;
        super::procedure::apply_rounded_region(self.window.hwnd(), 0);
        Ok(display.dpi()?.dpi())
    }

    pub fn hide(&mut self) {
        self.window.hide();
        self.window.state_mut().clear_events();
    }

    pub fn set_pointer_cursor(&self, cursor: PointerCursor) {
        self.window.state().set_pointer_cursor(cursor);
    }

    pub fn drain_events(&mut self) -> impl Iterator<Item = SwitcherEvent> + '_ {
        self.window.state_mut().drain().filter_map(switcher_event)
    }
}

fn switcher_event(event: WindowEvent) -> Option<SwitcherEvent> {
    match event {
        WindowEvent::Switcher(event) => Some(event),
        WindowEvent::Resized { width, height } => {
            Some(SwitcherEvent::Resized { width, height })
        }
        WindowEvent::DpiChanged { dpi } => Some(SwitcherEvent::DpiChanged { dpi }),
        WindowEvent::RenderRequested => Some(SwitcherEvent::RenderRequested),
        WindowEvent::Pointer(PointerEvent::Moved { x, y }) => {
            Some(SwitcherEvent::PointerMoved { x, y })
        }
        WindowEvent::Pointer(PointerEvent::Left) => Some(SwitcherEvent::PointerLeft),
        WindowEvent::Pointer(PointerEvent::LeftButtonReleased { x, y }) => {
            Some(SwitcherEvent::PointerReleased { x, y })
        }
        WindowEvent::PlacementRefreshRequested
        | WindowEvent::Pointer(
            PointerEvent::LeftButtonPressed { .. } | PointerEvent::Cancelled,
        )
        | WindowEvent::ContextMenuRequested(_)
        | WindowEvent::Search(_)
        | WindowEvent::Settings(_)
        | WindowEvent::ContextMenu(_)
        | WindowEvent::AnimationFrame
        | WindowEvent::StatusRefreshRequested => None,
    }
}
