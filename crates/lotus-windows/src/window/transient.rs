use windows::Win32::Foundation::HWND;

use super::procedure::WindowState;
use crate::NativeError;
use crate::platform::windows::interaction::claim_keyboard_focus;
use crate::platform::windows::native_window::{Activation, NativeWindow, WindowHandle};

type Result<T> = std::result::Result<T, NativeError>;

/// Shared lifecycle mechanics for short-lived, topmost Lotus windows.
///
/// Feature windows retain their own placement policy, scene state, and input model. This type
/// only owns the repeated native transition mechanics: clearing stale events before an open,
/// preparing a hidden window, revealing it, and clearing events on hide.
pub(super) struct TransientWindow {
    native: NativeWindow<WindowState>,
}

impl TransientWindow {
    pub(super) const fn new(native: NativeWindow<WindowState>) -> Self {
        Self { native }
    }

    pub(super) const fn hwnd(&self) -> HWND {
        self.native.hwnd()
    }

    pub(super) const fn handle(&self) -> WindowHandle {
        self.native.handle()
    }

    pub(super) fn dpi(&self) -> u32 {
        self.native.dpi().dpi()
    }

    pub(super) fn prepare_topmost(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<()> {
        self.clear_events();
        self.native
            .place_topmost(x, y, width, height, Activation::KeepInactive, false)
    }

    pub(super) fn update_topmost(
        &self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<()> {
        self.native
            .place_topmost(x, y, width, height, Activation::KeepInactive, false)
    }

    pub(super) fn prepare_and_show_topmost_inactive(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<()> {
        self.clear_events();
        self.native
            .place_topmost(x, y, width, height, Activation::KeepInactive, true)
    }

    pub(super) fn show_and_focus(&mut self) {
        self.state().advance_interaction();
        self.native.reveal(Activation::Activate);
        let _ = claim_keyboard_focus(self.hwnd());
    }

    pub(super) fn hide(&mut self) {
        self.native.hide();
        self.state().advance_interaction();
    }

    pub(super) fn state(&self) -> &WindowState {
        self.native.state()
    }

    pub(super) fn state_mut(&mut self) -> &mut WindowState {
        self.native.state_mut()
    }

    fn clear_events(&mut self) {
        self.native.state_mut().clear_events();
    }
}
