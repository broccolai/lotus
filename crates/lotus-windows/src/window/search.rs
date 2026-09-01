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
use crate::platform::windows::interaction::{OutsideClickObserver, claim_keyboard_focus};
use crate::platform::windows::native_window::{NativeWindow, WindowCreation, WindowHandle};
use crate::window::procedure::{
    SEARCH_OUTSIDE_CLICK_MESSAGE, SearchEvent, WindowClass, WindowState,
};
use crate::window::transient::TransientWindow;

const NORMAL_TOP_MINIMUM_DIPS: u32 = 52;

pub struct SearchWindow {
    window: TransientWindow,
    outside_click: Option<OutsideClickObserver>,
    interaction_suspended: bool,
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
            window: TransientWindow::new(window),
            outside_click: None,
            interaction_suspended: false,
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
        self.window.dpi()
    }

    pub fn open(&mut self, anchor: WindowHandle, width: u32, height: u32) -> Result<()> {
        let size = PopupSize::new(width, height)?;
        prepare_launcher(&mut self.window, anchor.raw(), size)?;
        super::procedure::apply_rounded_region(self.hwnd(), 0);
        super::procedure::start_search_clock_timer(self.hwnd())?;
        super::procedure::start_search_focus_timer(self.hwnd())?;
        if self.outside_click.is_none() {
            self.outside_click =
                OutsideClickObserver::start(self.hwnd(), SEARCH_OUTSIDE_CLICK_MESSAGE).ok();
        }

        self.interaction_suspended = false;
        self.window.show_and_focus();
        Ok(())
    }

    pub fn apply_geometry(
        &self,
        anchor: WindowHandle,
        width: u32,
        height: u32,
    ) -> Result<()> {
        let size = PopupSize::new(width, height)?;
        update_launcher_geometry(&self.window, anchor.raw(), size)?;
        super::procedure::apply_rounded_region(self.hwnd(), 0);
        Ok(())
    }

    pub fn focus(&self) -> bool {
        let focused = claim_keyboard_focus(self.hwnd()).is_owned();
        if focused {
            super::procedure::stop_search_focus_timer(self.hwnd());
        }
        focused
    }

    pub fn suspend_for_child_popup(&mut self) {
        super::procedure::stop_search_focus_timer(self.hwnd());
        self.outside_click = None;
        self.interaction_suspended = true;
    }

    pub fn resume_after_child_popup(&mut self) {
        self.interaction_suspended = false;
        let _ = super::procedure::start_search_focus_timer(self.hwnd());
        if self.outside_click.is_none() {
            self.outside_click =
                OutsideClickObserver::start(self.hwnd(), SEARCH_OUTSIDE_CLICK_MESSAGE).ok();
        }
    }

    pub fn hide(&mut self) {
        super::procedure::stop_search_clock_timer(self.hwnd());
        super::procedure::stop_search_focus_timer(self.hwnd());
        self.outside_click = None;
        self.window.hide();
        self.interaction_suspended = false;
    }

    pub fn drain_events(&mut self) -> impl Iterator<Item = SearchEvent> + '_ {
        let suspended = self.interaction_suspended;
        self.window
            .state_mut()
            .drain_events()
            .into_iter()
            .filter(move |event| {
                !suspended || !matches!(event, SearchEvent::FocusRefreshRequested)
            })
    }

    pub fn has_pending_events(&self) -> bool {
        self.window.state().has_pending_events()
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

fn prepare_launcher(
    window: &mut TransientWindow,
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
    window.prepare_topmost(x, y, size.width, size.height)?;
    Ok(())
}

fn update_launcher_geometry(
    window: &TransientWindow,
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
    window.update_topmost(x, y, size.width, size.height)?;
    Ok(())
}

fn normal_position(work_area: ScreenArea, size: PopupSize, dpi: DpiScale) -> (i32, i32) {
    let work_height = work_area.height().max(0);
    let proportional_top = work_height.saturating_mul(12).saturating_add(50) / 100;
    let minimum_top = dpi.physical_i32(NORMAL_TOP_MINIMUM_DIPS);
    (
        work_area.centered_origin(size.width, size.height).0,
        work_area
            .top
            .saturating_add(proportional_top.max(minimum_top)),
    )
}
