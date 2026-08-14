use super::{
    AltTabController, AltTabEvent, AppError, CompositionSurfaceState, DeviceState,
    DockHitTarget, DockRuntime, DockScene, DockWindow, LauncherCompositionSurfaceState,
    LauncherRuntime, LauncherSubmission, ModelCursorMove, PointerEvent, QueryEdit,
    RestartError, SearchEdit, SearchEvent, StatusRuntime, SurfaceError, SurfaceSize,
    SwitcherRuntime, WindowCursorMove, WindowTracker, WindowsKeyController,
    WindowsKeyEvent, launch_target, local_time_24h, read_text,
};

pub(super) fn restart_current_process() -> Result<(), RestartError> {
    let executable = std::env::current_exe()?;
    let arguments = restart_arguments(std::process::id());
    launch_target(&executable.to_string_lossy(), Some(&arguments))?;
    Ok(())
}

pub(super) fn restart_arguments(process_id: u32) -> String {
    format!("--restart-after {process_id} --open-settings")
}

pub(super) fn enable_optional_windows_key<T, E>(
    enabled: bool,
    enable: impl FnOnce() -> Result<T, E>,
) -> Option<T> {
    if !enabled {
        return None;
    }
    enable().ok()
}

pub(super) fn enable_optional_alt_tab(enabled: bool) -> Option<AltTabController> {
    if !enabled {
        return None;
    }
    let mut controller = AltTabController::new();
    controller.enable().ok().map(|_| controller)
}

pub(super) fn handle_windows_key_events(
    controller: &WindowsKeyController,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    dock_model: &DockRuntime,
    launcher: &mut LauncherRuntime,
) -> Result<(), AppError> {
    for event in controller.drain_events() {
        match event {
            WindowsKeyEvent::ToggleRequested => {
                let needs_animation = launcher.toggle(dock, dock_model, graphics)?;
                dock.set_animation_active(needs_animation)?;
            }
            WindowsKeyEvent::ReplayIncomplete { .. } => {}
        }
    }
    Ok(())
}

pub(super) fn handle_alt_tab_events(
    controller: &AltTabController,
    tracker: &WindowTracker,
    dock_model: &DockRuntime,
    graphics: &mut DeviceState,
    switcher: &mut SwitcherRuntime,
) -> Result<(), AppError> {
    for event in controller.drain_events() {
        match event {
            AltTabEvent::Begin {
                direction,
                foreground,
            } => {
                switcher.begin(
                    direction,
                    foreground,
                    tracker.current_windows(),
                    dock_model.settings(),
                    graphics,
                )?;
            }
            AltTabEvent::Cycle(direction) => switcher.cycle(direction, graphics)?,
            AltTabEvent::Commit => switcher.commit(),
            AltTabEvent::Cancel => switcher.hide(),
        }
    }
    Ok(())
}

pub(super) fn apply_fullscreen_visibility(
    dock: &DockWindow,
    tracker: &WindowTracker,
    model: &DockRuntime,
    launcher: &mut LauncherRuntime,
    status: &StatusRuntime,
) -> Result<(), AppError> {
    let visible = dock_visible(
        model.settings().hide_when_fullscreen,
        tracker.fullscreen_on_same_monitor(dock.handle()),
    );
    if !visible {
        launcher.hide();
        dock.set_animation_active(false)?;
    }
    let _changed = dock.set_visible(visible);
    status.set_visible(visible);
    Ok(())
}

const fn dock_visible(hide_when_fullscreen: bool, fullscreen_present: bool) -> bool {
    !hide_when_fullscreen || !fullscreen_present
}

pub(super) fn resize_surface(
    graphics: &mut DeviceState,
    surface: &mut CompositionSurfaceState,
    size: SurfaceSize,
) -> Result<(), AppError> {
    match surface.resize(size) {
        Ok(()) => Ok(()),
        Err(SurfaceError::DeviceLost(_)) => recover_graphics(graphics, surface),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn resize_launcher_surface(
    graphics: &mut DeviceState,
    surface: &mut LauncherCompositionSurfaceState,
    size: SurfaceSize,
) -> Result<(), AppError> {
    match surface.resize(size) {
        Ok(()) => Ok(()),
        Err(SurfaceError::DeviceLost(_)) => {
            let _ = graphics.poll();
            graphics.recover()?;
            let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
            surface.recover(device)?;
            surface.resize(size)?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

pub(super) fn render_surface(
    graphics: &mut DeviceState,
    surface: &mut CompositionSurfaceState,
    scene: &DockScene,
) -> Result<bool, AppError> {
    match surface.render_scene(scene) {
        Ok(frame) => Ok(frame.needs_animation()),
        Err(SurfaceError::DeviceLost(_)) => {
            recover_graphics(graphics, surface)?;
            Ok(surface.render_scene(scene)?.needs_animation())
        }
        Err(error) => Err(error.into()),
    }
}

pub(super) fn render_and_schedule(
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut CompositionSurfaceState,
    scene: &DockScene,
    launcher_needs_animation: bool,
) -> Result<(), AppError> {
    let needs_animation =
        render_surface(graphics, surface, scene)? || launcher_needs_animation;
    dock.set_animation_active(needs_animation)?;
    Ok(())
}

pub(super) fn handle_pointer_event(
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

pub(super) fn handle_search_event(
    event: SearchEvent,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    dock_surface: &mut CompositionSurfaceState,
    dock_model: &DockRuntime,
    launcher: &mut LauncherRuntime,
) -> Result<Option<LauncherSubmission>, AppError> {
    let mut scene_changed = false;
    let mut command = None;
    match event {
        SearchEvent::TextInput(character) => {
            launcher.controller.push_character(character);
            launcher.rebuild_scene(launcher.window.dpi())?;
            scene_changed = true;
        }
        SearchEvent::Edit(edit) => {
            if launcher.controller.edit_query(model_query_edit(edit)) {
                launcher.rebuild_scene(launcher.window.dpi())?;
                scene_changed = true;
            }
        }
        SearchEvent::PasteRequested => {
            if let Ok(text) = read_text()
                && launcher.controller.insert_text(&text)
            {
                launcher.rebuild_scene(launcher.window.dpi())?;
                scene_changed = true;
            }
        }
        SearchEvent::MoveSelection(direction) => {
            launcher.move_selection(direction)?;
            scene_changed = true;
        }
        SearchEvent::DismissRequested => launcher.hide(),
        SearchEvent::SubmitRequested => command = launcher.submit(dock.handle()),
        SearchEvent::Resized { width, height } => {
            if let (Some(size), Some(surface)) =
                (SurfaceSize::new(width, height), launcher.surface.as_mut())
            {
                resize_launcher_surface(graphics, surface, size)?;
                scene_changed = true;
            }
        }
        SearchEvent::DpiChanged { dpi } => {
            launcher.rebuild_scene(dpi)?;
            scene_changed = true;
        }
        SearchEvent::ClockRefreshRequested => {
            scene_changed = launcher
                .scene
                .as_mut()
                .is_some_and(|scene| scene.set_footer_time(local_time_24h()));
        }
        SearchEvent::FocusRefreshRequested => {
            let _ = launcher.window.focus();
        }
        SearchEvent::RenderRequested => scene_changed = true,
        SearchEvent::PointerMoved { x, y } => {
            let hovered = launcher.result_at(x, y);
            scene_changed = launcher.set_hovered_result(hovered);
        }
        SearchEvent::PointerLeft => {
            scene_changed = launcher.set_hovered_result(None);
        }
        pointer_event @ SearchEvent::PointerReleased { .. } => {
            if let Some((x, y)) = launcher_submission_coordinates(&pointer_event)
                && let Some(index) = launcher.result_at(x, y)
            {
                let _ = launcher.select_result(index)?;
                command = launcher.submit(dock.handle());
            }
        }
    }

    if scene_changed && launcher.is_visible() {
        launcher.sync_size(dock, graphics)?;
    }
    let dock_animation = render_surface(graphics, dock_surface, dock_model.scene())?;
    let launcher_animation = launcher.render(graphics)?;
    dock.set_animation_active(dock_animation || launcher_animation)?;
    Ok(command)
}

pub(super) fn launcher_submission_coordinates(event: &SearchEvent) -> Option<(i32, i32)> {
    match event {
        SearchEvent::PointerReleased { x, y } => Some((*x, *y)),
        SearchEvent::PointerMoved { .. }
        | SearchEvent::PointerLeft
        | SearchEvent::TextInput(_)
        | SearchEvent::Edit(_)
        | SearchEvent::PasteRequested
        | SearchEvent::MoveSelection(_)
        | SearchEvent::DismissRequested
        | SearchEvent::SubmitRequested
        | SearchEvent::Resized { .. }
        | SearchEvent::DpiChanged { .. }
        | SearchEvent::ClockRefreshRequested
        | SearchEvent::FocusRefreshRequested
        | SearchEvent::RenderRequested => None,
    }
}

const fn model_query_edit(edit: SearchEdit) -> QueryEdit {
    match edit {
        SearchEdit::DeleteBackward => QueryEdit::DeleteBackward,
        SearchEdit::DeleteForward => QueryEdit::DeleteForward,
        SearchEdit::MoveCursor(movement) => QueryEdit::MoveCursor(match movement {
            WindowCursorMove::Home => ModelCursorMove::Home,
            WindowCursorMove::End => ModelCursorMove::End,
            WindowCursorMove::Previous => ModelCursorMove::Previous,
            WindowCursorMove::Next => ModelCursorMove::Next,
        }),
        SearchEdit::SelectAll => QueryEdit::SelectAll,
    }
}

pub(super) fn resize_dock(
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut CompositionSurfaceState,
    model: &DockRuntime,
) -> Result<(), AppError> {
    let size = model.scene().desired_size();
    dock.resize_content(size.width(), size.height(), model.settings())?;
    resize_surface(graphics, surface, SurfaceSize::from(size))
}

pub(super) fn recover_graphics(
    graphics: &mut DeviceState,
    surface: &mut CompositionSurfaceState,
) -> Result<(), AppError> {
    let _ = graphics.poll();
    graphics.recover()?;
    let graphics_device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
    surface.recover(graphics_device)?;
    Ok(())
}
