use std::rc::Rc;

use lotus_core::settings::DockSettings;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{WS_EX_APPWINDOW, WS_POPUP};
use windows::core::w;

use crate::NativeError;

type Result<T> = std::result::Result<T, NativeError>;

use crate::platform::windows::backdrop;
use crate::platform::windows::display::nearest_display;
use crate::platform::windows::interaction::{PointerCursor, claim_keyboard_focus};
use crate::platform::windows::native_window::{
    Activation, NativeWindow, WindowCreation, WindowHandle,
};
use crate::window::procedure::{
    PointerEvent, SettingsEvent, WindowClass, WindowEvent, WindowState,
};

const DEFAULT_WIDTH_DIPS: u32 = 900;
const DEFAULT_HEIGHT_DIPS: u32 = 730;

pub struct SettingsWindow {
    window: NativeWindow<WindowState>,
    _class: Rc<WindowClass>,
}

impl SettingsWindow {
    pub(super) fn create(class: Rc<WindowClass>, dock: HWND) -> Result<Self> {
        let state = Box::new(WindowState::settings());
        let (x, y, width, height) = initial_bounds(dock)?;

        let window = NativeWindow::create(
            WindowCreation {
                instance: class.instance(),
                class_name: WindowClass::NAME,
                title: w!("lotus settings"),
                extended_style: WS_EX_APPWINDOW,
                style: WS_POPUP,
                x,
                y,
                width,
                height,
                owner: None,
            },
            state,
        )?;
        backdrop::apply_settings_window(window.hwnd());
        Ok(Self {
            window,
            _class: class,
        })
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

    pub fn client_size(&self) -> Result<(u32, u32)> {
        self.window.client_size()
    }

    pub fn show(&mut self) -> Result<()> {
        self.window.state_mut().clear_events();
        self.window.reveal(Activation::Activate);
        self.window.reveal_without_repositioning()?;
        self.focus();
        Ok(())
    }

    pub fn use_settings_material(&self) {
        backdrop::apply_settings_window(self.hwnd());
    }

    pub fn use_onboarding_material(&self, settings: &DockSettings) {
        backdrop::apply_onboarding_window(self.hwnd(), settings);
    }

    pub fn focus(&self) {
        let _ = claim_keyboard_focus(self.hwnd());
    }

    pub fn hide(&mut self) {
        self.window.hide();
        self.window.state_mut().clear_events();
    }

    pub fn set_pointer_cursor(&self, cursor: PointerCursor) {
        self.window.state().set_pointer_cursor(cursor);
    }

    pub fn drain_events(&mut self) -> impl Iterator<Item = SettingsEvent> + '_ {
        self.window
            .state_mut()
            .drain()
            .filter_map(settings_event_from_window_event)
    }
}

fn settings_event_from_window_event(event: WindowEvent) -> Option<SettingsEvent> {
    match event {
        WindowEvent::Settings(event) => Some(event),
        WindowEvent::Resized { width, height } => {
            Some(SettingsEvent::Resized { width, height })
        }
        WindowEvent::DpiChanged { dpi } => Some(SettingsEvent::DpiChanged { dpi }),
        WindowEvent::RenderRequested => Some(SettingsEvent::RenderRequested),
        WindowEvent::Pointer(PointerEvent::Moved { x, y }) => {
            Some(SettingsEvent::PointerMoved { x, y })
        }
        WindowEvent::Pointer(PointerEvent::Left) => Some(SettingsEvent::PointerLeft),
        WindowEvent::Pointer(PointerEvent::LeftButtonPressed { x, y }) => {
            Some(SettingsEvent::PointerPressed { x, y })
        }
        WindowEvent::Pointer(PointerEvent::LeftButtonReleased { x, y }) => {
            Some(SettingsEvent::PointerReleased { x, y })
        }
        WindowEvent::Pointer(PointerEvent::Cancelled)
        | WindowEvent::ContextMenuRequested(_)
        | WindowEvent::ContextMenu(_)
        | WindowEvent::Switcher(_)
        | WindowEvent::Search(_)
        | WindowEvent::PlacementRefreshRequested
        | WindowEvent::AnimationFrame
        | WindowEvent::StatusRefreshRequested => None,
    }
}

fn initial_bounds(anchor: HWND) -> Result<(i32, i32, i32, i32)> {
    let display = nearest_display(anchor)?;
    let dpi = display.dpi()?;
    let width = dpi.physical_i32(DEFAULT_WIDTH_DIPS);
    let height = dpi.physical_i32(DEFAULT_HEIGHT_DIPS);
    let work_width = display
        .work_area
        .right
        .saturating_sub(display.work_area.left);
    let work_height = display
        .work_area
        .bottom
        .saturating_sub(display.work_area.top);
    let x = display
        .work_area
        .left
        .saturating_add(work_width.saturating_sub(width) / 2);
    let y = display
        .work_area
        .top
        .saturating_add(work_height.saturating_sub(height) / 2);
    Ok((x, y, width, height))
}
