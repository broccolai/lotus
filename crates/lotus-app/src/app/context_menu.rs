use lotus_core::settings::{DockSettings, WindowPickerStyle};
use lotus_settings::appearance::theme_for;
use lotus_ui::frame::FramePass;
use lotus_ui::theme::Theme;
use lotus_windows::graphics::{DeviceState, GraphicsDevice};
use lotus_windows::window::{
    ContextMenuEvent, ContextMenuWindow, DismissReason, PopupAlignment, SelectionDirection,
    SignedPoint,
};

use crate::app::AppError;
use crate::app::visuals::{ContextMenuScene, NativePickerWindow};

mod surface;

use self::surface::ContextMenuSurface;

pub(super) struct ContextMenuRuntime {
    surface: ContextMenuSurface,
    scene: ContextMenuScene,
    visible: bool,
    theme: Theme,
    anchor: Option<SignedPoint>,
    alignment: PopupAlignment,
    picker_identity: Option<String>,
    session: ContextMenuSession,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PopupOwner {
    Dock,
    Search,
}

#[derive(Default)]
struct ContextMenuSession {
    owner: Option<PopupOwner>,
}

impl ContextMenuSession {
    fn open(&mut self, owner: PopupOwner) {
        self.owner = Some(owner);
    }

    fn close(&mut self) -> Option<PopupOwner> {
        self.owner.take()
    }

    fn owner(&self) -> Option<PopupOwner> {
        self.owner
    }
}

#[derive(Clone, Copy)]
enum ContextMenuSessionTransition {
    Begin(PopupOwner),
    Preserve,
}

pub(super) struct AppMenuOptions {
    pub(super) identity: String,
    pub(super) running_windows: usize,
    pub(super) pinned: bool,
    pub(super) pin_eligible: bool,
    pub(super) shift_held: bool,
}

pub(super) struct PopupInvocation {
    pub(super) action: crate::app::visuals::PopupAction,
}

pub(super) struct ContextMenuEventOutcome {
    pub(super) invocation: Option<PopupInvocation>,
    pub(super) closed_owner: Option<PopupOwner>,
    pub(super) dismissal_reason: Option<DismissReason>,
}

#[derive(Clone, Copy)]
pub(super) struct PopupEvent {
    event: ContextMenuEvent,
    generation: usize,
}

impl ContextMenuRuntime {
    pub(super) fn has_pending_events(&self) -> bool {
        self.surface.has_pending_events()
    }

    pub(super) fn diagnostic_surface_state(&self) -> (bool, bool, bool) {
        let (dirty, animating) = self.surface.diagnostic_state();
        (dirty, animating, self.visible)
    }

    pub(super) fn new(window: ContextMenuWindow, theme: &Theme) -> Result<Self, AppError> {
        let mut scene = ContextMenuScene::system(window.dpi())
            .ok_or(AppError::InvalidContextMenuScene)?;
        let _ = scene.set_theme(*theme);
        Ok(Self {
            surface: ContextMenuSurface::new(window),
            scene,
            visible: false,
            theme: *theme,
            anchor: None,
            alignment: PopupAlignment::Center,
            picker_identity: None,
            session: ContextMenuSession::default(),
        })
    }

    pub(super) fn apply_settings(&mut self, settings: &DockSettings) {
        let _ = self.scene.set_theme(theme_for(settings));
        self.theme = theme_for(settings);
        self.surface.apply_settings(settings);
    }

    pub(super) fn open(
        &mut self,
        anchor: SignedPoint,
        alignment: PopupAlignment,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let mut scene = ContextMenuScene::system(self.surface.dpi())
            .ok_or(AppError::InvalidContextMenuScene)?;
        let _ = scene.set_theme(self.theme);
        self.scene = scene;
        self.picker_identity = None;
        self.alignment = alignment;
        self.open_current(
            anchor,
            ContextMenuSessionTransition::Begin(PopupOwner::Dock),
            graphics,
        )
    }

    pub(super) fn open_app(
        &mut self,
        anchor: SignedPoint,
        options: AppMenuOptions,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let mut scene = ContextMenuScene::app(
            self.surface.dpi(),
            options.identity,
            options.running_windows,
            options.pinned,
            options.pin_eligible,
            options.shift_held,
        )
        .ok_or(AppError::InvalidContextMenuScene)?;
        let _ = scene.set_theme(self.theme);
        self.scene = scene;
        self.picker_identity = None;
        self.alignment = PopupAlignment::Center;
        self.open_current(
            anchor,
            ContextMenuSessionTransition::Begin(PopupOwner::Dock),
            graphics,
        )
    }

    pub(super) fn open_file_location(
        &mut self,
        anchor: SignedPoint,
        path: String,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let mut scene = ContextMenuScene::file_location(self.surface.dpi(), path)
            .ok_or(AppError::InvalidContextMenuScene)?;
        let _ = scene.set_theme(self.theme);
        self.scene = scene;
        self.picker_identity = None;
        self.alignment = PopupAlignment::Start;
        self.open_current(
            anchor,
            ContextMenuSessionTransition::Begin(PopupOwner::Search),
            graphics,
        )
    }

    pub(super) fn open_power(
        &mut self,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let anchor = self.anchor.ok_or(AppError::InvalidContextMenuScene)?;
        let mut scene = ContextMenuScene::power(self.surface.dpi())
            .ok_or(AppError::InvalidContextMenuScene)?;
        let _ = scene.set_theme(self.theme);
        self.scene = scene;
        self.picker_identity = None;
        self.open_current(anchor, ContextMenuSessionTransition::Preserve, graphics)
    }

    pub(super) fn open_picker(
        &mut self,
        anchor: SignedPoint,
        identity: String,
        style: WindowPickerStyle,
        windows: Vec<NativePickerWindow>,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let mut scene = ContextMenuScene::picker(self.surface.dpi(), style, windows)
            .ok_or(AppError::InvalidContextMenuScene)?;
        let _ = scene.set_theme(self.theme);
        self.scene = scene;
        self.picker_identity = Some(identity);
        self.alignment = PopupAlignment::Center;
        self.open_current(
            anchor,
            ContextMenuSessionTransition::Begin(PopupOwner::Dock),
            graphics,
        )
    }

    pub(super) fn picker_identity(&self) -> Option<&str> {
        self.picker_identity.as_deref()
    }

    pub(super) fn owner(&self) -> Option<PopupOwner> {
        self.visible.then(|| self.session.owner()).flatten()
    }

    pub(super) fn close_if_owned_by(&mut self, owner: PopupOwner) {
        if self.owner() == Some(owner) {
            let _ = self.hide();
        }
    }

    pub(super) fn replace_picker(
        &mut self,
        style: WindowPickerStyle,
        windows: Vec<NativePickerWindow>,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if windows.is_empty() {
            let _ = self.hide();
            return Ok(());
        }
        let Some(anchor) = self.anchor else {
            let _ = self.hide();
            return Ok(());
        };
        let mut scene = ContextMenuScene::picker(self.surface.dpi(), style, windows)
            .ok_or(AppError::InvalidContextMenuScene)?;
        let _ = scene.set_theme(self.theme);
        self.scene = scene;
        self.surface
            .prepare(anchor, self.alignment, &mut self.scene, graphics)?;
        self.invalidate();
        Ok(())
    }

    fn open_current(
        &mut self,
        anchor: SignedPoint,
        session_transition: ContextMenuSessionTransition,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.anchor = Some(anchor);
        self.surface
            .prepare(anchor, self.alignment, &mut self.scene, graphics)?;
        if let ContextMenuSessionTransition::Begin(owner) = session_transition {
            self.session.open(owner);
        }
        self.visible = true;
        self.invalidate();
        self.surface.show();
        Ok(())
    }

    pub(super) fn hide(&mut self) -> Option<PopupOwner> {
        if self.visible {
            self.surface.hide();
            self.visible = false;
            self.anchor = None;
            self.picker_identity = None;
            let _ = self.scene.pointer_left();
        }
        self.session.close()
    }

    pub(super) fn invalidate(&mut self) {
        self.surface.invalidate();
    }

    pub(super) fn recover_surface(
        &mut self,
        device: &GraphicsDevice,
    ) -> Result<(), AppError> {
        self.surface.recover(device)
    }

    pub(super) fn render_frame(
        &mut self,
        pass: &mut FramePass,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.surface
            .render_frame(pass, graphics, &self.scene, self.visible)
    }

    pub(super) fn resize(&mut self, width: u32, height: u32) -> Result<(), AppError> {
        self.surface.resize(width, height)
    }

    pub(super) fn drain_events_up_to(&mut self, limit: usize) -> Vec<PopupEvent> {
        let generation = self.surface.interaction_generation();
        self.surface
            .drain_events_up_to(limit)
            .map(|event| PopupEvent { event, generation })
            .collect()
    }

    pub(super) fn handle_event(
        &mut self,
        event: PopupEvent,
    ) -> Result<ContextMenuEventOutcome, AppError> {
        if !self.visible || event.generation != self.surface.interaction_generation() {
            return Ok(ContextMenuEventOutcome {
                invocation: None,
                closed_owner: None,
                dismissal_reason: None,
            });
        }
        let owner_before_event = self.owner();
        let mut closed_owner = None;
        let mut dismissal_reason = None;
        let invocation = match event.event {
            ContextMenuEvent::PointerMoved { x, y } => {
                if self.scene.pointer_move(x, y) {
                    self.invalidate();
                }
                None
            }
            ContextMenuEvent::PointerLeft => {
                if self.scene.pointer_left() {
                    self.invalidate();
                }
                None
            }
            ContextMenuEvent::PointerReleased { x, y } => {
                self.take_action(self.scene.pointer_action(x, y))
            }
            ContextMenuEvent::SelectionRequested => {
                self.take_action(self.scene.selected_action())
            }
            ContextMenuEvent::MoveSelection(direction) => {
                if self
                    .scene
                    .move_selection(direction == SelectionDirection::Next)
                {
                    self.invalidate();
                }
                None
            }
            ContextMenuEvent::Scroll(direction) => {
                if self.scene.scroll(direction == SelectionDirection::Next) {
                    self.invalidate();
                }
                None
            }
            ContextMenuEvent::ShiftChanged(held) => {
                if self.scene.set_shift_held(held) {
                    self.invalidate();
                }
                None
            }
            ContextMenuEvent::DismissRequested(request) => {
                if self.surface.accepts_dismiss(request) {
                    dismissal_reason = Some(request.reason);
                    closed_owner = self.hide();
                }
                None
            }
            ContextMenuEvent::Resized { width, height } => {
                self.resize(width, height)?;
                self.invalidate();
                None
            }
            ContextMenuEvent::DpiChanged { dpi } => {
                if self.scene.set_dpi(dpi) {
                    let desired = self.scene.desired_size();
                    self.surface.resize_to_scene(desired)?;
                }
                self.invalidate();
                None
            }
            ContextMenuEvent::RenderRequested => {
                self.invalidate();
                None
            }
        };
        if !self.visible && closed_owner.is_none() {
            closed_owner = owner_before_event;
        }
        Ok(ContextMenuEventOutcome {
            invocation,
            closed_owner,
            dismissal_reason,
        })
    }

    fn take_action(
        &mut self,
        action: Option<crate::app::visuals::PopupAction>,
    ) -> Option<PopupInvocation> {
        let action = action?;
        if !matches!(
            action,
            crate::app::visuals::PopupAction::System(
                crate::app::visuals::ContextMenuAction::RequestShutdown
            )
        ) {
            let _ = self.hide();
        }
        Some(PopupInvocation { action })
    }
}
