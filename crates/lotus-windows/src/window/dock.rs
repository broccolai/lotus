use std::cell::Cell;
use std::rc::Rc;

use lotus_core::settings::DockSettings;
use lotus_ui::geometry::DpiScale;
use windows::Win32::Foundation::{E_INVALIDARG, HWND};
use windows::Win32::UI::WindowsAndMessaging::{
    WINDOW_EX_STYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{Error, w};

use crate::NativeError;

type Result<T> = std::result::Result<T, NativeError>;

use super::context_menu::ContextMenuWindow;
use crate::platform::windows::backdrop;
use crate::platform::windows::display::{ScreenArea, primary_display};
use crate::platform::windows::interaction::drag_threshold;
use crate::platform::windows::native_window::{
    Activation, NativeWindow, WindowCreation, WindowHandle, current_instance,
};
use crate::window::procedure::WindowClass;
use crate::window::procedure::{WindowEvent, WindowState};
use crate::window::search::SearchWindow;
use crate::window::settings::SettingsWindow;
use lotus_dock::appbar::AppBarLayout;

const PREVIEW_WIDTH: u32 = 118;

pub struct DockWindow {
    window: NativeWindow<WindowState>,
    class: Rc<WindowClass>,
    appbar_active: Cell<bool>,
}

impl DockWindow {
    pub fn create() -> Result<Self> {
        let instance = current_instance()?;
        let class = Rc::new(WindowClass::register(instance)?);
        let work_area = primary_work_area()?;

        let extended_style =
            WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0 | WS_EX_TOPMOST.0);
        let state = Box::<WindowState>::default();

        let window = NativeWindow::create(
            WindowCreation {
                instance,
                class_name: WindowClass::NAME,
                title: w!("Lotus"),
                extended_style,
                style: WS_POPUP,
                x: work_area.right - 1,
                y: work_area.top,
                width: 1,
                height: 1,
                owner: None,
            },
            state,
        )?;

        backdrop::apply(window.hwnd());
        Ok(Self { window, class, appbar_active: Cell::new(false) })
    }

    pub fn prepare(&self, settings: &DockSettings) -> Result<()> {
        let dpi = DpiScale::from_system(self.dpi());
        let width = dpi.physical(PREVIEW_WIDTH);
        let height = dpi.physical(settings.dock_height());
        self.resize_content(width, height, settings)
    }

    pub fn refresh_placement(&self, settings: &DockSettings) -> Result<()> {
        let (width, height) = self.client_size()?;
        self.resize_content(width, height, settings)
    }

    pub fn set_visible(&self, visible: bool) -> bool {
        self.apply_visibility(visible)
    }

    pub fn apply_appbar_layout(&self, layout: AppBarLayout, settings: &DockSettings) -> Result<()> {
        let rect = layout.content_rect();
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        self.window.state().set_corner_radius(settings.corner_radius);
        self.window.place_topmost(
            rect.left,
            rect.top,
            width,
            height,
            Activation::KeepInactive,
            false,
        )?;
        self.appbar_active.set(true);
        super::procedure::apply_rounded_region(self.hwnd(), settings.corner_radius);
        Ok(())
    }

    fn apply_visibility(&self, visible: bool) -> bool {
        let current = self.window.is_visible();
        let Some(action) = visibility_action(current, visible) else {
            return false;
        };
        match action {
            VisibilityAction::ShowNoActivate => self.window.reveal(Activation::KeepInactive),
            VisibilityAction::Hide => self.window.hide(),
        }
        true
    }

    pub fn resize_content(&self, width: u32, height: u32, settings: &DockSettings) -> Result<()> {
        let size = ContentSize::new(width, height)?;
        self.window.state().set_corner_radius(settings.corner_radius);
        let hwnd = self.window.hwnd();
        let dpi = DpiScale::from_system(self.dpi());
        let appbar_active = self.appbar_active.get();
        let reserved_gutter = dpi.physical_i32(settings.bottom_offset);
        let bottom_offset =
            if appbar_active { centered_bottom_offset(reserved_gutter) } else { reserved_gutter };
        let (x, y) = normal_position(primary_placement_area(appbar_active)?, size, bottom_offset);

        self.window.place_topmost(
            x,
            y,
            size.width,
            size.height,
            Activation::KeepInactive,
            false,
        )?;

        super::procedure::apply_rounded_region(hwnd, settings.corner_radius);
        Ok(())
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

    pub fn drag_threshold(&self) -> (u32, u32) {
        drag_threshold(self.hwnd())
    }

    pub fn drain_events(&mut self) -> impl Iterator<Item = WindowEvent> + '_ {
        self.window.state_mut().drain()
    }

    pub fn set_animation_active(&self, active: bool) -> Result<()> {
        self.window.state().set_animation_active(self.hwnd(), active)
    }

    pub fn create_search_window(&self) -> Result<SearchWindow> {
        SearchWindow::create(Rc::clone(&self.class))
    }

    pub fn create_settings_window(&self) -> Result<SettingsWindow> {
        SettingsWindow::create(Rc::clone(&self.class), self.hwnd())
    }

    pub fn create_context_menu_window(&self) -> Result<ContextMenuWindow> {
        ContextMenuWindow::create(Rc::clone(&self.class), self.hwnd())
    }

    pub fn create_switcher_window(&self) -> Result<super::SwitcherWindow> {
        super::SwitcherWindow::create(Rc::clone(&self.class))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VisibilityAction {
    ShowNoActivate,
    Hide,
}

const fn visibility_action(current: bool, requested: bool) -> Option<VisibilityAction> {
    match (current, requested) {
        (false, true) => Some(VisibilityAction::ShowNoActivate),
        (true, false) => Some(VisibilityAction::Hide),
        (false, false) | (true, true) => None,
    }
}

fn primary_work_area() -> Result<ScreenArea> {
    Ok(primary_display()?.work_area)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ContentSize {
    width: i32,
    height: i32,
}

impl ContentSize {
    fn new(width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(Error::new(E_INVALIDARG, "dock content dimensions must be nonzero").into());
        }
        let width = i32::try_from(width)
            .map_err(|_| Error::new(E_INVALIDARG, "dock content width exceeds Win32 limits"))?;
        let height = i32::try_from(height)
            .map_err(|_| Error::new(E_INVALIDARG, "dock content height exceeds Win32 limits"))?;
        Ok(Self { width, height })
    }
}

fn normal_position(work_area: ScreenArea, size: ContentSize, bottom_offset: i32) -> (i32, i32) {
    let available_width = work_area.right.saturating_sub(work_area.left);
    (
        work_area.left.saturating_add(available_width.saturating_sub(size.width) / 2),
        work_area.bottom.saturating_sub(size.height).saturating_sub(bottom_offset),
    )
}

const fn centered_bottom_offset(reserved_gutter: i32) -> i32 {
    reserved_gutter.saturating_add(1) / 2
}

fn primary_placement_area(appbar_active: bool) -> Result<ScreenArea> {
    let display = primary_display()?;
    Ok(if appbar_active { display.bounds } else { display.work_area })
}
