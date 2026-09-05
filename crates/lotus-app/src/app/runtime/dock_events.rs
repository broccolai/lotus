use std::time::Instant;

use lotus_windows::WindowHandle;
use lotus_windows::graphics::{DeviceState, SurfaceSize};
use lotus_windows::window::{
    DockContextRequest, DockEvent, DockWindow, PointerEvent, PopupAlignment, SignedPoint,
};

use super::presentation::present_dock_change;
use crate::app::dock::DockInteractionIntent;
use crate::app::modules::ModuleHost;
use crate::app::monitors::DockAction;
use crate::app::primary_dock::PrimaryDock;
use crate::app::settings_persistence::SettingsPersistence;
use crate::app::system_actions::{SystemAction, execute_system_action};
use crate::app::visuals::{DockHitTarget, SystemStatusKind};
use crate::app::{AppError, DockRuntime};

pub(super) fn handle_window_event(
    event: DockEvent,
    primary_dock: &mut PrimaryDock,
    graphics: &mut DeviceState,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
    settings_persistence: &SettingsPersistence,
) -> Result<(), AppError> {
    match event {
        DockEvent::Resized { width, height } => {
            if let Some(size) = SurfaceSize::new(width, height) {
                primary_dock.resize_surface(graphics, size)?;
            }
        }
        DockEvent::DpiChanged { dpi } => {
            dock_model.set_dpi(dpi)?;
            dock_model.set_drag_threshold(primary_dock.window().drag_threshold());
            present_dock_change(primary_dock, graphics, auxiliary, dock_model)?;
        }
        DockEvent::PlacementRefreshRequested => {
            primary_dock
                .window()
                .refresh_placement(dock_model.settings())?;
            auxiliary.refresh_placement(primary_dock.window(), dock_model, graphics)?;
        }
        DockEvent::Pointer(event) => {
            handle_dock_pointer(
                event,
                primary_dock,
                graphics,
                dock_model,
                auxiliary,
                settings_persistence,
            )?;
        }
        DockEvent::ContextMenuRequested(request) => {
            handle_context_menu(request, primary_dock, graphics, dock_model, auxiliary)?;
        }
        DockEvent::AnimationFrame => {
            auxiliary.advance_launcher_animation();
            if dock_model.advance_departure(Instant::now()) {
                primary_dock.resize_for_model(graphics, dock_model)?;
            }
        }
        DockEvent::MascotAnimationDeadline => {
            if dock_model.advance_mascot_animation()
                && primary_dock.window().is_visible()
                && !primary_dock.window().is_fullscreen_occluded()
            {
                primary_dock.invalidate();
            }
        }
        DockEvent::StatusRefreshRequested => {
            if dock_model.refresh_status() {
                primary_dock.invalidate();
            }
            auxiliary.refresh_status(dock_model.settings());
        }
        DockEvent::RenderRequested => {
            primary_dock.invalidate();
        }
    }
    Ok(())
}

fn handle_dock_pointer(
    event: PointerEvent,
    primary_dock: &mut PrimaryDock,
    graphics: &mut DeviceState,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
    settings_persistence: &SettingsPersistence,
) -> Result<(), AppError> {
    if matches!(event, PointerEvent::LeftButtonPressed { .. }) {
        auxiliary.hide_context_menu();
    }
    let release_request = match event {
        PointerEvent::LeftButtonReleased { x, y } => {
            let client = SignedPoint { x, y };
            primary_dock
                .window()
                .client_to_screen(client)
                .ok()
                .map(|screen| DockContextRequest::Pointer {
                    screen,
                    client,
                    shift_held: false,
                })
        }
        PointerEvent::Moved { .. }
        | PointerEvent::Left
        | PointerEvent::LeftButtonPressed { .. }
        | PointerEvent::Cancelled => None,
    };
    let interaction = dock_model.handle_pointer_event(event);
    if interaction.changed {
        primary_dock.invalidate();
    }
    let Some(intent) = interaction.intent else {
        if matches!(event, PointerEvent::LeftButtonReleased { .. }) {
            auxiliary.hide_launcher();
        }
        return Ok(());
    };
    match intent {
        DockInteractionIntent::Reorder(request) => {
            if dock_model.persist_reorder(&request, settings_persistence)? {
                primary_dock.invalidate();
            }
            auxiliary.hide_launcher();
            Ok(())
        }
        DockInteractionIntent::Activate(target) => execute_dock_activation(
            target,
            release_request,
            primary_dock,
            graphics,
            dock_model,
            auxiliary,
        ),
    }
}

fn execute_dock_activation(
    target: DockHitTarget,
    release_request: Option<DockContextRequest>,
    primary_dock: &PrimaryDock,
    graphics: &mut DeviceState,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    let anchor = release_request
        .and_then(|request| dock_model.popup_target_anchor(request))
        .map(|(_, anchor, _)| anchor);
    execute_dock_action(
        DockAction::Activate {
            target,
            owner: primary_dock.window().handle(),
            anchor,
        },
        primary_dock.window(),
        graphics,
        dock_model,
        auxiliary,
    )
}

fn activate_dock_item(
    source_index: usize,
    target: DockHitTarget,
    anchor: Option<SignedPoint>,
    owner: WindowHandle,
    graphics: &mut DeviceState,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    auxiliary.hide_launcher();
    let window_count = dock_model
        .item(source_index)
        .map_or(0, |item| item.windows.len());
    if window_count <= 1 {
        dock_model.activate(target, owner);
        return Ok(());
    }
    let Some(anchor) = anchor else {
        dock_model.activate(target, owner);
        return Ok(());
    };

    auxiliary.open_window_picker(anchor, source_index, dock_model, graphics)
}

pub(super) fn activate_system_status(
    kind: SystemStatusKind,
    owner: WindowHandle,
    anchor: Option<SignedPoint>,
) -> bool {
    execute_system_action(SystemAction::ActivateStatus { kind, anchor }, owner)
        .advanced_color_changed
}

fn handle_context_menu(
    request: DockContextRequest,
    primary_dock: &mut PrimaryDock,
    graphics: &mut DeviceState,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    let Some((target, anchor, alignment)) = dock_model.popup_target_anchor(request) else {
        return Ok(());
    };
    if dock_model.pointer_cancelled() {
        primary_dock.invalidate();
    }
    execute_dock_action(
        DockAction::Context {
            target,
            anchor,
            alignment,
            shift_held: request.shift_held(),
        },
        primary_dock.window(),
        graphics,
        dock_model,
        auxiliary,
    )
}

fn open_context_target(
    target: DockHitTarget,
    anchor: SignedPoint,
    alignment: PopupAlignment,
    shift_held: bool,
    graphics: &mut DeviceState,
    dock_model: &DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    match target {
        DockHitTarget::Jirachi => {
            auxiliary.open_context_menu(anchor, alignment, graphics)?;
        }
        DockHitTarget::Item(source_index) => {
            auxiliary.open_application_context_menu(
                anchor,
                source_index,
                shift_held,
                dock_model,
                graphics,
            )?;
        }
        DockHitTarget::Media(_)
        | DockHitTarget::SystemStatus(_)
        | DockHitTarget::ShowDesktop => {}
    }
    Ok(())
}

pub(super) fn execute_dock_action(
    action: DockAction,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    match action {
        DockAction::Activate {
            target,
            owner,
            anchor,
        } => match target {
            DockHitTarget::Item(source_index) => activate_dock_item(
                source_index,
                target,
                anchor,
                owner,
                graphics,
                dock_model,
                auxiliary,
            )?,
            DockHitTarget::Jirachi => {
                auxiliary.toggle_launcher(dock, dock_model, graphics)?;
            }
            DockHitTarget::Media(target) => {
                auxiliary.dismiss_popups_for_activation();
                auxiliary.activate_media(target, dock_model, owner);
            }
            DockHitTarget::SystemStatus(kind) => {
                auxiliary.dismiss_popups_for_activation();
                if activate_system_status(kind, owner, anchor) {
                    dock_model.advanced_color_changed();
                    auxiliary.refresh_status(dock_model.settings());
                }
            }
            DockHitTarget::ShowDesktop => {
                auxiliary.dismiss_popups_for_activation();
                execute_system_action(SystemAction::ShowDesktop, owner);
            }
        },
        DockAction::Context {
            target,
            anchor,
            alignment,
            shift_held,
        } => {
            auxiliary.hide_launcher();
            open_context_target(
                target, anchor, alignment, shift_held, graphics, dock_model, auxiliary,
            )?;
        }
    }
    Ok(())
}
