use std::rc::Rc;

use lotus_core::settings::DockSettings;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{WS_EX_APPWINDOW, WS_POPUP};
use windows::core::w;

use crate::NativeError;

type Result<T> = std::result::Result<T, NativeError>;

use crate::platform::windows::backdrop;
use crate::platform::windows::display::{fit_aspect_ratio, nearest_display};
use crate::platform::windows::interaction::{PointerCursor, claim_keyboard_focus};
use crate::platform::windows::native_window::{
    Activation, NativeWindow, WindowCreation, WindowHandle,
};
use crate::window::procedure::{SettingsEvent, WindowClass, WindowState};

const DEFAULT_WIDTH_DIPS: u32 = 900;
const DEFAULT_HEIGHT_DIPS: u32 = 730;
const SETTINGS_WORK_AREA_MARGIN_DIPS: u32 = 16;

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

    pub fn use_material(&self, settings: &DockSettings) {
        backdrop::apply_settings_material(self.hwnd(), settings);
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

    pub fn set_layout_dpi(&self, dpi: u32) {
        self.window.state().set_settings_layout_dpi(dpi);
    }

    pub fn drain_events(&mut self) -> impl Iterator<Item = SettingsEvent> + '_ {
        self.window.state_mut().drain_events().into_iter()
    }

    pub fn has_pending_events(&self) -> bool {
        self.window.state().has_pending_events()
    }
}

fn initial_bounds(anchor: HWND) -> Result<(i32, i32, i32, i32)> {
    let display = nearest_display(anchor)?;
    let dpi = display.dpi()?;
    let margin = dpi.physical_i32(SETTINGS_WORK_AREA_MARGIN_DIPS);
    let available_area = display.work_area.inset(margin);
    let (width, height) = fit_aspect_ratio(
        dpi.physical_i32(DEFAULT_WIDTH_DIPS),
        dpi.physical_i32(DEFAULT_HEIGHT_DIPS),
        available_area.width(),
        available_area.height(),
    );
    let (x, y) = display.work_area.centered_origin(width, height);
    Ok((x, y, width, height))
}

pub(super) fn fit_size_within(
    width: i32,
    height: i32,
    maximum_width: i32,
    maximum_height: i32,
) -> (i32, i32) {
    fit_aspect_ratio(width, height, maximum_width, maximum_height)
}
