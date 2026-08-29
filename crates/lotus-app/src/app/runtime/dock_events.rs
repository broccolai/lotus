use std::time::Instant;

use lotus_ui::frame::ScheduledSurface;
use lotus_windows::WindowHandle;
use lotus_windows::activation::launch_target;
use lotus_windows::dialog::show_error;
use lotus_windows::graphics::{CompositionSurfaceState, DeviceState, SurfaceSize};
use lotus_windows::window::{
    DockContextRequest, DockEvent, DockWindow, PointerEvent, PopupAlignment, SignedPoint,
};

use super::presentation::{present_dock_change, resize_dock, resize_surface};
use crate::app::modules::ModuleHost;
use crate::app::monitors::DockAction;
use crate::app::visuals::{DockHitTarget, SystemStatusKind};
use crate::app::{AppError, DockRuntime};

pub(super) fn handle_window_event(
    event: DockEvent,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    match event {
        DockEvent::Resized { width, height } => {
            if let Some(size) = SurfaceSize::new(width, height) {
                resize_surface(graphics, surface.value_mut(), size)?;
            }
        }
        DockEvent::DpiChanged { dpi } => {
            dock_model.set_dpi(dpi)?;
            dock_model.set_drag_threshold(dock.drag_threshold());
            present_dock_change(dock, graphics, surface, auxiliary, dock_model)?;
        }
        DockEvent::PlacementRefreshRequested => {
            dock.refresh_placement(dock_model.settings())?;
            auxiliary.refresh_placement(dock, dock_model, graphics)?;
        }
        DockEvent::Pointer(event) => {
            handle_dock_pointer(event, dock, graphics, surface, dock_model, auxiliary)?;
        }
        DockEvent::ContextMenuRequested(request) => {
            handle_context_menu(request, dock, graphics, surface, dock_model, auxiliary)?;
        }
        DockEvent::AnimationFrame => {
            auxiliary.advance_launcher_animation();
            if dock_model.advance_departure(Instant::now()) {
                resize_dock(dock, graphics, surface, dock_model)?;
            }
        }
        DockEvent::MascotAnimationDeadline => {
            if dock_model.advance_mascot_animation()
                && dock.is_visible()
                && !dock.is_fullscreen_occluded()
            {
                surface.invalidate();
            }
        }
        DockEvent::StatusRefreshRequested => {
            if dock_model.refresh_status() {
                surface.invalidate();
            }
            auxiliary.refresh_status(dock_model.settings());
        }
        DockEvent::RenderRequested => {
            surface.invalidate();
        }
    }
    Ok(())
}

fn handle_dock_pointer(
    event: PointerEvent,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    if matches!(event, PointerEvent::LeftButtonPressed { .. }) {
        auxiliary.hide_context_menu();
    }
    let release_request = match event {
        PointerEvent::LeftButtonReleased { x, y } => {
            let client = SignedPoint { x, y };
            dock.client_to_screen(client)
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
    let interaction = dock_model.handle_pointer_event(event)?;
    if interaction.changed {
        surface.invalidate();
    }
    let Some(target) = interaction.effect else {
        if matches!(event, PointerEvent::LeftButtonReleased { .. }) {
            auxiliary.hide_launcher();
        }
        return Ok(());
    };
    let anchor = release_request
        .and_then(|request| dock_model.popup_target_anchor(request))
        .map(|(_, anchor, _)| anchor);
    execute_dock_action(
        DockAction::Activate {
            target,
            owner: dock.handle(),
            anchor,
        },
        dock,
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
    let result = match kind {
        SystemStatusKind::Volume => native_panel_or_fallback(
            anchor.map_or_else(
                || lotus_windows::tray::open_quick_settings(owner),
                |point| lotus_windows::tray::open_quick_settings_at(owner, point.x),
            ),
            "sndvol.exe",
        ),
        SystemStatusKind::AdvancedColor => lotus_windows::advanced_color::toggle(owner)
            .map(|_| ())
            .map_err(|error| error.to_string()),
        SystemStatusKind::Network => native_panel_or_fallback(
            anchor.map_or_else(
                || lotus_windows::tray::open_quick_settings(owner),
                |point| lotus_windows::tray::open_quick_settings_at(owner, point.x),
            ),
            "ms-settings:network",
        ),
        SystemStatusKind::BackgroundApps => anchor
            .map_or_else(
                || lotus_windows::tray::open_overflow(owner),
                |point| lotus_windows::tray::open_overflow_at(owner, point.x),
            )
            .map_err(|error| error.to_string()),
        SystemStatusKind::DateTime => native_panel_or_fallback(
            anchor.map_or_else(
                || lotus_windows::tray::open_calendar(owner),
                |point| lotus_windows::tray::open_calendar_at(owner, point.x),
            ),
            "ms-settings:dateandtime",
        ),
    };

    if let Err(error) = result {
        show_error(
            owner,
            "Lotus",
            &format!("Lotus could not open that system control.\n\n{error}"),
        );
        return false;
    }

    kind == SystemStatusKind::AdvancedColor
}

fn native_panel_or_fallback(
    native: Result<bool, lotus_windows::tray::TrayError>,
    fallback: &str,
) -> Result<(), String> {
    match native {
        Ok(true) => Ok(()),
        Ok(false) => launch_target(fallback, None).map_err(|error| error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

fn handle_context_menu(
    request: DockContextRequest,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    let Some((target, anchor, alignment)) = dock_model.popup_target_anchor(request) else {
        return Ok(());
    };
    if dock_model.pointer_cancelled() {
        surface.invalidate();
    }
    execute_dock_action(
        DockAction::Context {
            target,
            anchor,
            alignment,
            shift_held: request.shift_held(),
        },
        dock,
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
                if let Err(error) = lotus_windows::desktop::toggle() {
                    show_error(
                        owner,
                        "Lotus",
                        &format!("Lotus could not show the desktop.\n\n{error}"),
                    );
                }
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
