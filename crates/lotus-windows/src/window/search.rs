use std::rc::Rc;

use lotus_ui::geometry::DpiScale;
use windows::Win32::Foundation::{E_INVALIDARG, HWND};
use windows::Win32::UI::WindowsAndMessaging::{
    WINDOW_EX_STYLE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{Error, w};

use crate::NativeError;

type Result<T> = std::result::Result<T, NativeError>;

use crate::platform::windows::backdrop;
use crate::platform::windows::display::{ScreenArea, nearest_display};
use crate::platform::windows::interaction::claim_keyboard_focus;
use crate::platform::windows::native_window::{
    Activation, NativeWindow, WindowCreation, WindowHandle,
};
use crate::window::procedure::{
    PointerEvent, SearchEvent, WindowClass, WindowEvent, WindowState,
};

const NORMAL_TOP_MINIMUM_DIPS: u32 = 52;

pub struct SearchWindow {
    window: NativeWindow<WindowState>,
    _class: Rc<WindowClass>,
}

impl SearchWindow {
    pub(super) fn create(class: Rc<WindowClass>) -> Result<Self> {
        let state = Box::new(WindowState::search());
        let extended_style = WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0);

        let window = NativeWindow::create(
            WindowCreation {
                instance: class.instance(),
                class_name: WindowClass::NAME,
                title: w!("Lotus Search"),
                extended_style,
                style: WS_POPUP,
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                owner: None,
            },
            state,
        )?;
        backdrop::apply_search_popup(window.hwnd());
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

    pub fn show_sized(
        &mut self,
        anchor: WindowHandle,
        width: u32,
        height: u32,
    ) -> Result<()> {
        let size = PopupSize::new(width, height)?;
        self.window.state_mut().clear_events();
        position_launcher(&self.window, anchor.raw(), size)?;
        super::procedure::apply_rounded_region(self.hwnd(), 0);
        super::procedure::start_search_clock_timer(self.hwnd())?;
        super::procedure::start_search_focus_timer(self.hwnd())?;

        let _ = self.focus();
        Ok(())
    }

    pub fn focus(&self) -> bool {
        let focused = claim_keyboard_focus(self.hwnd()).is_owned();
        if focused {
            super::procedure::stop_search_focus_timer(self.hwnd());
        }
        focused
    }

    pub fn hide(&mut self) {
        super::procedure::stop_search_clock_timer(self.hwnd());
        super::procedure::stop_search_focus_timer(self.hwnd());
        self.window.hide();
        self.window.state_mut().clear_events();
    }

    pub fn drain_events(&mut self) -> impl Iterator<Item = SearchEvent> + '_ {
        self.window
            .state_mut()
            .drain()
            .filter_map(search_event_from_window_event)
    }
}

fn search_event_from_window_event(event: WindowEvent) -> Option<SearchEvent> {
    match event {
        WindowEvent::Search(event) => Some(event),
        WindowEvent::Resized { width, height } => {
            Some(SearchEvent::Resized { width, height })
        }
        WindowEvent::DpiChanged { dpi } => Some(SearchEvent::DpiChanged { dpi }),
        WindowEvent::RenderRequested => Some(SearchEvent::RenderRequested),
        WindowEvent::Pointer(PointerEvent::Moved { x, y }) => {
            Some(SearchEvent::PointerMoved { x, y })
        }
        WindowEvent::Pointer(PointerEvent::Left) => Some(SearchEvent::PointerLeft),
        WindowEvent::Pointer(PointerEvent::LeftButtonReleased { x, y }) => {
            Some(SearchEvent::PointerReleased { x, y })
        }
        WindowEvent::Pointer(
            PointerEvent::LeftButtonPressed { .. } | PointerEvent::Cancelled,
        )
        | WindowEvent::ContextMenuRequested(_)
        | WindowEvent::ContextMenu(_)
        | WindowEvent::Settings(_)
        | WindowEvent::Switcher(_)
        | WindowEvent::PlacementRefreshRequested
        | WindowEvent::AnimationFrame
        | WindowEvent::StatusRefreshRequested => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PopupSize {
    width: i32,
    height: i32,
}

impl PopupSize {
    fn new(width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(Error::new(
                E_INVALIDARG,
                "search popup dimensions must be nonzero",
            )
            .into());
        }
        let width = i32::try_from(width).map_err(|_| {
            Error::new(E_INVALIDARG, "search popup width exceeds Win32 limits")
        })?;
        let height = i32::try_from(height).map_err(|_| {
            Error::new(E_INVALIDARG, "search popup height exceeds Win32 limits")
        })?;
        Ok(Self { width, height })
    }
}

fn position_launcher(
    window: &NativeWindow<WindowState>,
    anchor: HWND,
    size: PopupSize,
) -> Result<()> {
    let anchor = if anchor.is_invalid() {
        window.hwnd()
    } else {
        anchor
    };
    let monitor = nearest_display(anchor)?;
    let dpi = monitor.dpi()?;
    let (x, y) = normal_position(monitor.work_area, size, dpi);
    window.place_topmost(x, y, size.width, size.height, Activation::Activate, true)?;
    Ok(())
}

fn normal_position(work_area: ScreenArea, size: PopupSize, dpi: DpiScale) -> (i32, i32) {
    let available_width = work_area.right.saturating_sub(work_area.left);
    let work_height = work_area.bottom.saturating_sub(work_area.top).max(0);
    let proportional_top = work_height.saturating_mul(12).saturating_add(50) / 100;
    let minimum_top = dpi.physical_i32(NORMAL_TOP_MINIMUM_DIPS);
    (
        work_area
            .left
            .saturating_add(available_width.saturating_sub(size.width) / 2),
        work_area
            .top
            .saturating_add(proportional_top.max(minimum_top)),
    )
}
