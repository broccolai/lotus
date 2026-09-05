use lotus_core::settings::{DockSettings, WindowPickerStyle};
use lotus_dock::popup::PopupSymbol;
use lotus_settings::appearance::theme_for;
use lotus_ui::embedded_icon::EmbeddedIcon;
use lotus_ui::frame::{FrameOutcome, FramePass, ScheduledSurface};
use lotus_ui::geometry::NonZeroPhysicalSize;
use lotus_ui::theme::Theme;
use lotus_windows::dwm_thumbnail::DwmThumbnailHost;
use lotus_windows::graphics::context_menu_surface::ContextMenuCompositionSurfaceState;
use lotus_windows::graphics::surface::FrameResult;
use lotus_windows::graphics::{DeviceState, GraphicsDevice, SurfaceError};
use lotus_windows::window::{
    ContextMenuEvent, ContextMenuWindow, DismissReason, PopupAlignment, SelectionDirection,
    SignedPoint,
};

use crate::app::AppError;
use crate::app::visuals::{ContextMenuScene, NativePickerWindow};

pub(super) struct ContextMenuRuntime {
    pub(super) window: ContextMenuWindow,
    pub(super) scene: ContextMenuScene,
    pub(super) surface: Option<ScheduledSurface<ContextMenuCompositionSurfaceState>>,
    pub(super) visible: bool,
    thumbnails: DwmThumbnailHost,
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
    pub(super) fn diagnostic_surface_state(&self) -> (bool, bool, bool) {
        let surface = self.surface.as_ref();
        (
            surface.is_some_and(ScheduledSurface::is_dirty),
            surface.is_some_and(ScheduledSurface::is_animating),
            self.visible,
        )
    }

    pub(super) fn new(window: ContextMenuWindow, theme: &Theme) -> Result<Self, AppError> {
        let mut scene = ContextMenuScene::system(window.dpi())
            .ok_or(AppError::InvalidContextMenuScene)?;
        let _ = scene.set_theme(*theme);
        Ok(Self {
            thumbnails: DwmThumbnailHost::new(window.handle()),
            window,
            scene,
            surface: None,
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
        lotus_windows::backdrop::apply_context_menu_settings(
            self.window.handle(),
            settings,
        );
    }

    pub(super) fn open(
        &mut self,
        anchor: SignedPoint,
        alignment: PopupAlignment,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let mut scene = ContextMenuScene::system(self.window.dpi())
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
            self.window.dpi(),
            options.identity,
            options.running_windows,
            options.pinned,
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
        let mut scene = ContextMenuScene::file_location(self.window.dpi(), path)
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
        let mut scene = ContextMenuScene::power(self.window.dpi())
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
        let mut scene = ContextMenuScene::picker(self.window.dpi(), style, windows)
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
        let mut scene = ContextMenuScene::picker(self.window.dpi(), style, windows)
            .ok_or(AppError::InvalidContextMenuScene)?;
        let _ = scene.set_theme(self.theme);
        self.scene = scene;
        self.prepare_surface(anchor, graphics)?;
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
        self.prepare_surface(anchor, graphics)?;
        if let ContextMenuSessionTransition::Begin(owner) = session_transition {
            self.session.open(owner);
        }
        self.visible = true;
        self.invalidate();
        self.window.show();
        Ok(())
    }

    fn prepare_surface(
        &mut self,
        anchor: SignedPoint,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if self.surface.is_none() && graphics.ready().is_none() {
            return Err(AppError::GraphicsUnavailable);
        }
        let mut desired = self.scene.desired_size();
        let dpi = self.window.prepare_at(anchor, self.alignment, desired)?;
        if self.scene.set_dpi(dpi) {
            desired = self.scene.desired_size();
            let _dpi = self.window.prepare_at(anchor, self.alignment, desired)?;
        }
        if let Some(surface) = &mut self.surface {
            surface.value_mut().resize(desired)?;
        } else {
            let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
            self.surface = Some(ScheduledSurface::new(
                ContextMenuCompositionSurfaceState::create(
                    device,
                    self.window.handle(),
                    desired,
                )?,
            ));
        }
        Ok(())
    }

    pub(super) fn hide(&mut self) -> Option<PopupOwner> {
        if self.visible {
            self.window.hide();
            self.visible = false;
            self.thumbnails.clear();
            self.anchor = None;
            self.picker_identity = None;
            let _ = self.scene.pointer_left();
            if let Some(surface) = &mut self.surface {
                surface.stop_animation();
            }
        }
        self.session.close()
    }

    pub(super) fn invalidate(&mut self) {
        if let Some(surface) = &mut self.surface {
            surface.invalidate();
        }
    }

    pub(super) fn recover_surface(
        &mut self,
        device: &GraphicsDevice,
    ) -> Result<(), AppError> {
        if let Some(surface) = &mut self.surface {
            surface.value_mut().recover(device)?;
        }
        Ok(())
    }

    pub(super) fn render_frame(
        &mut self,
        pass: &mut FramePass,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if !self.visible {
            if let Some(surface) = &mut self.surface {
                surface.stop_animation();
            }
            return Ok(());
        }
        let surface = self
            .surface
            .as_mut()
            .ok_or(AppError::InvalidContextMenuScene)?;
        pass.render(surface, |surface| {
            let presentation = self.scene.presentation(popup_asset);
            match surface.render_scene(&presentation) {
                Ok(FrameResult::Presented { .. }) => {
                    self.thumbnails.reconcile(&self.scene.picker_previews());
                    Ok(FrameOutcome::complete(false))
                }
                Ok(FrameResult::TargetRecreated) => Ok(FrameOutcome::Retry),
                Err(SurfaceError::DeviceLost(loss)) => {
                    graphics.mark_lost(loss);
                    Ok(FrameOutcome::complete(false))
                }
                Err(error) => Err(error.into()),
            }
        })
    }

    pub(super) fn resize(&mut self, width: u32, height: u32) -> Result<(), AppError> {
        let Some(size) = NonZeroPhysicalSize::new(width, height) else {
            return Ok(());
        };
        if let Some(surface) = &mut self.surface {
            surface.value_mut().resize(size)?;
        }
        Ok(())
    }

    pub(super) fn drain_events(&mut self) -> Vec<PopupEvent> {
        let generation = self.window.interaction_generation();
        self.window
            .drain_events()
            .map(|event| PopupEvent { event, generation })
            .collect()
    }

    pub(super) fn handle_event(
        &mut self,
        event: PopupEvent,
    ) -> Result<ContextMenuEventOutcome, AppError> {
        if !self.visible || event.generation != self.window.interaction_generation() {
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
                if self.window.accepts_dismiss(request) {
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
                    if let Some(surface) = &mut self.surface {
                        surface.value_mut().resize(desired)?;
                    }
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

const fn popup_asset(symbol: PopupSymbol) -> EmbeddedIcon {
    match symbol {
        PopupSymbol::Power => EmbeddedIcon::FluentPower,
        PopupSymbol::Lock => EmbeddedIcon::FluentLock,
        PopupSymbol::Restart => EmbeddedIcon::FluentRestart,
        PopupSymbol::Settings => EmbeddedIcon::FluentSettings,
        PopupSymbol::Quit | PopupSymbol::Close => EmbeddedIcon::FluentDismiss,
        PopupSymbol::Open | PopupSymbol::Image => EmbeddedIcon::FluentOpen,
        PopupSymbol::Pin => EmbeddedIcon::FluentPin,
        PopupSymbol::Unpin => EmbeddedIcon::FluentPinOff,
        PopupSymbol::Previous => EmbeddedIcon::FluentPrevious,
        PopupSymbol::Next => EmbeddedIcon::FluentNext,
    }
}
