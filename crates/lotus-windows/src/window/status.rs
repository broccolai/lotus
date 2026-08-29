use std::cell::Cell;
use std::rc::Rc;

use lotus_core::settings::{DockSettings, DockZone};
use lotus_ui::geometry::{DpiScale, NonZeroPhysicalSize};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, WINDOW_EX_STYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};
use windows::core::w;

use super::procedure::{
    DockEvent, StatusEvent, WindowClass, WindowState, apply_rounded_region,
};
use crate::NativeError;
use crate::platform::windows::backdrop;
use crate::platform::windows::display::{Display, primary_display, secondary_displays};
use crate::platform::windows::native_window::{
    Activation, NativeWindow, WindowCreation, WindowHandle, WindowLayer, WindowPlacement,
};

type Result<T> = std::result::Result<T, NativeError>;

pub struct StatusWindow {
    window: NativeWindow<WindowState>,
    _class: Rc<WindowClass>,
    display: Option<Display>,
    fullscreen_occluded: Cell<bool>,
}

impl StatusWindow {
    pub(super) fn create(class: Rc<WindowClass>, owner: HWND) -> Result<Self> {
        let display = primary_display()?;
        Self::create_on_display(class, owner, display, None)
    }

    pub(super) fn create_secondary_displays(
        class: &Rc<WindowClass>,
        owner: HWND,
    ) -> Result<Vec<DockReplicaWindow>> {
        secondary_displays()?
            .into_iter()
            .map(|display| {
                Self::create_on_display(Rc::clone(class), owner, display, Some(display))
                    .map(DockReplicaWindow)
            })
            .collect()
    }

    fn create_on_display(
        class: Rc<WindowClass>,
        owner: HWND,
        initial_display: Display,
        display: Option<Display>,
    ) -> Result<Self> {
        let extended_style =
            WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0 | WS_EX_TOPMOST.0);
        let window = NativeWindow::create(
            WindowCreation {
                instance: class.instance(),
                class_name: WindowClass::NAME,
                title: w!("Lotus system status"),
                extended_style,
                style: WS_POPUP,
                x: initial_display.bounds.right - 1,
                y: initial_display.bounds.bottom - 1,
                width: 1,
                height: 1,
                owner: Some(owner),
            },
            Box::new(if display.is_some() {
                WindowState::dock_replica()
            } else {
                WindowState::status()
            }),
        )?;
        backdrop::apply(window.hwnd());
        Ok(Self {
            window,
            _class: class,
            display,
            fullscreen_occluded: Cell::new(false),
        })
    }

    pub fn handle(&self) -> WindowHandle {
        self.window.handle()
    }

    pub fn dpi(&self) -> u32 {
        self.window.dpi().dpi()
    }

    pub fn client_to_screen(
        &self,
        point: super::SignedPoint,
    ) -> Result<super::SignedPoint> {
        let mut native = POINT {
            x: point.x,
            y: point.y,
        };
        unsafe { ClientToScreen(self.window.hwnd(), &raw mut native) }.ok()?;
        Ok(super::SignedPoint {
            x: native.x,
            y: native.y,
        })
    }

    pub(super) fn place_aligned(
        &self,
        dock: HWND,
        size: NonZeroPhysicalSize,
        zone: DockZone,
        settings: &DockSettings,
    ) -> Result<()> {
        let display = self.display.map_or_else(primary_display, Ok)?;
        let mut dock_bounds = RECT::default();
        unsafe { GetWindowRect(dock, &raw mut dock_bounds)? };
        let width = i32::try_from(size.width()).unwrap_or(i32::MAX);
        let height = i32::try_from(size.height()).unwrap_or(i32::MAX);
        let edge_gap = display
            .bounds
            .bottom
            .saturating_sub(dock_bounds.bottom)
            .max(0);
        let edge_inset =
            DpiScale::from_system(self.dpi()).physical_i32(settings.screen_edge_inset);
        let x = match zone {
            DockZone::Left => display.bounds.left.saturating_add(edge_inset),
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
                .saturating_sub(edge_inset)
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
        self.window.place_at_layer(
            self.presentation_layer(),
            WindowPlacement {
                x,
                y,
                width,
                height,
                activation: Activation::KeepInactive,
                show: false,
            },
        )?;
        apply_rounded_region(self.window.hwnd(), settings.corner_radius);
        Ok(())
    }

    pub fn place_replica(
        &self,
        dock: HWND,
        size: NonZeroPhysicalSize,
        settings: &DockSettings,
    ) -> Result<()> {
        let display = self.display.map_or_else(primary_display, Ok)?;
        let mut dock_bounds = RECT::default();
        unsafe { GetWindowRect(dock, &raw mut dock_bounds)? };
        let primary = primary_display()?;
        let source_gap = primary.bounds.bottom.saturating_sub(dock_bounds.bottom);
        let source_dpi = DpiScale::from_system(unsafe {
            windows::Win32::UI::HiDpi::GetDpiForWindow(dock)
        });
        let target_dpi = DpiScale::from_system(self.dpi());
        let edge_gap = scale_physical_gap(source_gap, source_dpi.dpi(), target_dpi.dpi());
        let width = i32::try_from(size.width()).unwrap_or(i32::MAX);
        let height = i32::try_from(size.height()).unwrap_or(i32::MAX);
        let inset = target_dpi.physical_i32(settings.screen_edge_inset);
        let x = match settings.dock_zone {
            DockZone::Left => display.bounds.left.saturating_add(inset),
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
                .saturating_sub(inset)
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
        self.window.place_at_layer(
            self.presentation_layer(),
            WindowPlacement {
                x,
                y,
                width,
                height,
                activation: Activation::KeepInactive,
                show: false,
            },
        )?;
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

    pub fn set_fullscreen_occluded(&self, occluded: bool) -> Result<()> {
        let changed = self.fullscreen_occluded.replace(occluded) != occluded;
        if changed || !self.window.is_visible() {
            self.window.present_at_layer(self.presentation_layer())?;
        }
        Ok(())
    }

    pub fn is_fullscreen_occluded(&self) -> bool {
        self.fullscreen_occluded.get()
    }

    pub fn is_visible(&self) -> bool {
        self.window.is_visible()
    }

    pub fn drain_events(&mut self) -> impl Iterator<Item = StatusEvent> + '_ {
        self.window.state_mut().drain_status().into_iter()
    }

    pub fn has_pending_events(&self) -> bool {
        self.window.state().has_pending_events()
    }

    fn presentation_layer(&self) -> WindowLayer {
        if self.fullscreen_occluded.get() {
            WindowLayer::Bottom
        } else {
            WindowLayer::Topmost
        }
    }
}

pub struct DockReplicaWindow(StatusWindow);

impl DockReplicaWindow {
    pub fn handle(&self) -> WindowHandle {
        self.0.handle()
    }
    pub fn dpi(&self) -> u32 {
        self.0.dpi()
    }
    pub fn client_to_screen(
        &self,
        point: super::SignedPoint,
    ) -> Result<super::SignedPoint> {
        self.0.client_to_screen(point)
    }
    pub fn place_replica(
        &self,
        dock: HWND,
        size: NonZeroPhysicalSize,
        settings: &DockSettings,
    ) -> Result<()> {
        self.0.place_replica(dock, size, settings)
    }
    pub fn set_visible(&self, visible: bool) {
        self.0.set_visible(visible);
    }
    pub fn set_fullscreen_occluded(&self, occluded: bool) -> Result<()> {
        self.0.set_fullscreen_occluded(occluded)
    }
    pub fn is_fullscreen_occluded(&self) -> bool {
        self.0.is_fullscreen_occluded()
    }
    pub fn is_visible(&self) -> bool {
        self.0.is_visible()
    }
    pub fn drain_events(&mut self) -> impl Iterator<Item = DockEvent> + '_ {
        self.0.window.state_mut().drain_dock().into_iter()
    }
    pub fn has_pending_events(&self) -> bool {
        self.0.has_pending_events()
    }
}

fn scale_physical_gap(gap: i32, source_dpi: u32, target_dpi: u32) -> i32 {
    let scaled = i64::from(gap)
        .saturating_mul(i64::from(target_dpi))
        .checked_div(i64::from(source_dpi.max(1)))
        .unwrap_or_default();
    i32::try_from(scaled).unwrap_or_else(|_| {
        if scaled.is_negative() {
            i32::MIN
        } else {
            i32::MAX
        }
    })
}
