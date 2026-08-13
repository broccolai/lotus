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
use crate::platform::windows::interaction::claim_keyboard_focus;
use crate::platform::windows::native_window::{
    Activation, NativeWindow, WindowCreation, WindowHandle,
};
use crate::window::procedure::{
    ContextMenuEvent, PointerEvent, SignedPoint, WindowClass, WindowEvent, WindowState,
};

pub struct ContextMenuWindow {
    window: NativeWindow<WindowState>,
    _class: Rc<WindowClass>,
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
        Ok(Self { window, _class: class })
    }

    pub(crate) fn hwnd(&self) -> HWND {
        self.window.hwnd()
    }

    pub fn handle(&self) -> WindowHandle {
        self.window.handle()
    }

    pub fn dpi(&self) -> u32 {
        self.window.dpi().dpi()
    }

    pub fn prepare_at(&mut self, anchor: SignedPoint, size: NonZeroPhysicalSize) -> Result<u32> {
        let display = nearest_display_to_point(anchor.x, anchor.y)?;
        let width = i32::try_from(size.width()).unwrap_or(i32::MAX);
        let height = i32::try_from(size.height()).unwrap_or(i32::MAX);
        let maximum_x = display.work_area.right.saturating_sub(width);
        let maximum_y = display.work_area.bottom.saturating_sub(height);
        let x = anchor
            .x
            .saturating_sub(width / 2)
            .clamp(display.work_area.left, maximum_x.max(display.work_area.left));
        let y = anchor
            .y
            .saturating_sub(height)
            .clamp(display.work_area.top, maximum_y.max(display.work_area.top));

        self.window.state_mut().clear_events();
        self.window.place_topmost(x, y, width, height, Activation::KeepInactive, false)?;
        Ok(display.dpi()?.dpi())
    }

    pub fn show(&mut self) {
        self.window.state_mut().clear_events();
        self.window.reveal(Activation::Activate);
        let _ = claim_keyboard_focus(self.hwnd());
    }

    pub fn hide(&mut self) {
        self.window.hide();
        self.window.state_mut().clear_events();
    }

    pub fn drain_events(&mut self) -> impl Iterator<Item = ContextMenuEvent> + '_ {
        self.window.state_mut().drain().filter_map(context_event_from_window_event)
    }
}

fn context_event_from_window_event(event: WindowEvent) -> Option<ContextMenuEvent> {
    match event {
        WindowEvent::ContextMenu(event) => Some(event),
        WindowEvent::Resized { width, height } => Some(ContextMenuEvent::Resized { width, height }),
        WindowEvent::DpiChanged { dpi } => Some(ContextMenuEvent::DpiChanged { dpi }),
        WindowEvent::RenderRequested => Some(ContextMenuEvent::RenderRequested),
        WindowEvent::Pointer(PointerEvent::Moved { x, y }) => {
            Some(ContextMenuEvent::PointerMoved { x, y })
        }
        WindowEvent::Pointer(PointerEvent::Left) => Some(ContextMenuEvent::PointerLeft),
        WindowEvent::Pointer(PointerEvent::LeftButtonReleased { x, y }) => {
            Some(ContextMenuEvent::PointerReleased { x, y })
        }
        WindowEvent::Pointer(PointerEvent::Cancelled) => Some(ContextMenuEvent::DismissRequested),
        WindowEvent::Pointer(PointerEvent::LeftButtonPressed { .. })
        | WindowEvent::ContextMenuRequested(_)
        | WindowEvent::Search(_)
        | WindowEvent::Settings(_)
        | WindowEvent::Switcher(_)
        | WindowEvent::PlacementRefreshRequested
        | WindowEvent::AnimationFrame => None,
    }
}
