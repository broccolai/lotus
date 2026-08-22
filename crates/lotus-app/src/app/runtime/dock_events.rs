use std::time::Instant;

use lotus_ui::frame::ScheduledSurface;
use lotus_windows::WindowHandle;
use lotus_windows::activation::launch_target;
use lotus_windows::dialog::show_error;
use lotus_windows::graphics::{CompositionSurfaceState, DeviceState, SurfaceSize};
use lotus_windows::window::{
    DockContextRequest, DockWindow, PointerEvent, PopupAlignment, SignedPoint, WindowEvent,
};

use super::presentation::{present_dock_change, resize_dock, resize_surface};
use crate::app::modules::ModuleHost;
use crate::app::monitors::MonitorDockAction;
use crate::app::visuals::{DockHitTarget, SystemStatusKind};
use crate::app::{AppError, DockRuntime};

pub(crate) fn handle_pointer_event(
    event: PointerEvent,
    model: &mut DockRuntime,
) -> Result<(bool, Option<DockHitTarget>), AppError> {
    Ok(match event {
        PointerEvent::Moved { x, y } => (model.pointer_moved(x, y), None),
        PointerEvent::Left => (model.pointer_left(), None),
        PointerEvent::LeftButtonPressed { x, y } => (model.pointer_pressed(x, y), None),
        PointerEvent::LeftButtonReleased { x, y } => return model.pointer_released(x, y),
        PointerEvent::Cancelled => (model.pointer_cancelled(), None),
    })
}

pub(super) fn handle_window_event(
    event: WindowEvent,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    match event {
        WindowEvent::Resized { width, height } => {
            if let Some(size) = SurfaceSize::new(width, height) {
                resize_surface(graphics, surface.value_mut(), size)?;
            }
        }
        WindowEvent::DpiChanged { dpi } => {
            dock_model.set_dpi(dpi)?;
            dock_model.set_drag_threshold(dock.drag_threshold());
            present_dock_change(dock, graphics, surface, auxiliary, dock_model)?;
        }
        WindowEvent::PlacementRefreshRequested => {
            dock.refresh_placement(dock_model.settings())?;
            auxiliary.refresh_placement(dock, dock_model, graphics)?;
        }
        WindowEvent::Pointer(event) => {
            handle_dock_pointer(event, dock, graphics, surface, dock_model, auxiliary)?;
        }
        WindowEvent::ContextMenuRequested(request) => {
            handle_context_menu(request, dock, graphics, surface, dock_model, auxiliary)?;
        }
        WindowEvent::Search(_)
        | WindowEvent::Settings(_)
        | WindowEvent::ContextMenu(_)
        | WindowEvent::Switcher(_) => {}
        WindowEvent::AnimationFrame => {
            auxiliary.advance_launcher_animation();
            if dock_model.advance_departure(Instant::now()) {
                resize_dock(dock, graphics, surface, dock_model)?;
            }
        }
        WindowEvent::StatusRefreshRequested => {
            if dock_model.refresh_status() {
                surface.invalidate();
            }
            auxiliary.refresh_status(dock_model.settings());
        }
        WindowEvent::RenderRequested => {
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
                .map(|screen| DockContextRequest::Pointer { screen, client })
        }
        PointerEvent::Moved { .. }
        | PointerEvent::Left
        | PointerEvent::LeftButtonPressed { .. }
        | PointerEvent::Cancelled => None,
    };
    let (changed, activation) = handle_pointer_event(event, dock_model)?;
    if changed {
        surface.invalidate();
    }
    let Some(target) = activation else {
        if matches!(event, PointerEvent::LeftButtonReleased { .. }) {
            auxiliary.hide_launcher();
        }
        return Ok(());
    };
    let activation_anchor = release_request
        .and_then(|request| dock_model.popup_target_anchor(request))
        .map(|(_, anchor, _)| anchor);
    match target {
        DockHitTarget::Item(source_index) => {
            let anchor = release_request
                .and_then(|request| dock_model.popup_target_anchor(request))
                .map(|(_, anchor, _)| anchor);
            activate_dock_item(
                source_index,
                target,
                anchor,
                dock.handle(),
                graphics,
                dock_model,
                auxiliary,
            )?;
        }
        DockHitTarget::Jirachi => {
            auxiliary.toggle_launcher(dock, dock_model, graphics)?;
        }
        DockHitTarget::Media(target) => {
            auxiliary.dismiss_popups_for_activation();
            auxiliary.activate_media(target, dock_model, dock.handle());
        }
        DockHitTarget::SystemStatus(kind) => {
            auxiliary.dismiss_popups_for_activation();
            activate_system_status(kind, dock.handle(), activation_anchor);
        }
        DockHitTarget::ShowDesktop => {
            auxiliary.dismiss_popups_for_activation();
            if let Err(error) = lotus_windows::desktop::toggle() {
                show_error(
                    dock.handle(),
                    "Lotus",
                    &format!("Lotus could not show the desktop.\n\n{error}"),
                );
            }
        }
    }
    Ok(())
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
) {
    let result = match kind {
        SystemStatusKind::Volume => native_panel_or_fallback(
            anchor.map_or_else(
                || lotus_windows::tray::open_quick_settings(owner),
                |point| lotus_windows::tray::open_quick_settings_at(owner, point.x),
            ),
            "sndvol.exe",
        ),
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
    }
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
    _dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    let Some((target, anchor, alignment)) = dock_model.popup_target_anchor(request) else {
        return Ok(());
    };
    auxiliary.hide_launcher();
    if dock_model.pointer_cancelled() {
        surface.invalidate();
    }
    open_context_target(target, anchor, alignment, graphics, dock_model, auxiliary)
}

fn open_context_target(
    target: DockHitTarget,
    anchor: SignedPoint,
    alignment: PopupAlignment,
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

pub(super) fn handle_monitor_dock_action(
    action: MonitorDockAction,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    match action {
        MonitorDockAction::Activate {
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
                activate_system_status(kind, owner, anchor);
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
        MonitorDockAction::Context {
            target,
            anchor,
            alignment,
        } => {
            auxiliary.hide_launcher();
            open_context_target(
                target, anchor, alignment, graphics, dock_model, auxiliary,
            )?;
        }
    }
    Ok(())
}
