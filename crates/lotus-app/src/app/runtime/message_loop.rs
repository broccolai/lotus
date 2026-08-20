use lotus_ui::frame::{FrameOutcome, FramePass, FrameTrigger, ScheduledSurface};
use lotus_windows::alt_tab::is_alt_tab_wake;
use lotus_windows::appbar::fullscreen_notification;
use lotus_windows::graphics::{CompositionSurfaceState, DeviceState};
use lotus_windows::interaction::{NativeMessage, next_message};
use lotus_windows::media::is_media_wake;
use lotus_windows::search_catalog::is_search_catalog_wake;
use lotus_windows::taskbar_badges::is_taskbar_badge_wake;
use lotus_windows::update::is_update_wake;
use lotus_windows::window::DockWindow;
use lotus_windows::window_tracker::WindowTracker;
use lotus_windows::windows_key::is_windows_key_wake;

use super::{
    controllers, dock_events, presentation, search_events, settings_events, update_events,
    window_events,
};
use crate::app::switcher::AuxiliaryWindows;
use crate::app::{AppError, DockRuntime, RuntimePolicy};

pub(crate) fn run_message_loop(
    runtime: &RuntimePolicy<'_>,
    dock: &mut DockWindow,
    graphics: &mut DeviceState,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    window_tracker: &mut WindowTracker,
    dock_model: &mut DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    MessageLoop {
        runtime,
        dock,
        graphics,
        surface,
        window_tracker,
        dock_model,
        auxiliary,
    }
    .run()
}

pub(crate) fn flush_frame(
    dock: &mut DockWindow,
    graphics: &mut DeviceState,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    dock_model: &DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
    trigger: FrameTrigger,
) -> Result<(), AppError> {
    let mut pass = FramePass::new(trigger);
    let device_generation = graphics.generation();
    let animation_allowed = !dock.is_fullscreen_occluded();
    pass.render(surface, |surface| {
        presentation::render_surface(graphics, surface, dock_model.scene()).map(|outcome| {
            match outcome {
                FrameOutcome::Complete {
                    continues_animation,
                } => FrameOutcome::complete(continues_animation && animation_allowed),
                FrameOutcome::Retry => FrameOutcome::Retry,
            }
        })
    })?;
    auxiliary.launcher.render_frame(&mut pass, graphics)?;
    auxiliary.context_menu.render_frame(&mut pass, graphics)?;
    auxiliary.settings.render_frame(&mut pass, graphics)?;
    if let Err(error) = auxiliary.switcher.render_frame(&mut pass, graphics) {
        lotus_windows::diagnostics::record_error("alt_tab.render", &error);
        auxiliary.switcher.abandon();
    }
    auxiliary.status.render_frame(&mut pass, graphics)?;
    auxiliary.monitors.render_frame(&mut pass, graphics)?;

    if graphics.generation() != device_generation {
        surface.invalidate();
        auxiliary.invalidate_surfaces();
        pass.request_next_frame();
    }

    dock.set_animation_active(pass.animation_active())?;
    Ok(())
}

struct MessageLoop<'a, 'runtime> {
    runtime: &'a RuntimePolicy<'runtime>,
    dock: &'a mut DockWindow,
    graphics: &'a mut DeviceState,
    surface: &'a mut ScheduledSurface<CompositionSurfaceState>,
    window_tracker: &'a mut WindowTracker,
    dock_model: &'a mut DockRuntime,
    auxiliary: &'a mut AuxiliaryWindows,
}

impl MessageLoop<'_, '_> {
    fn run(&mut self) -> Result<(), AppError> {
        loop {
            let Some(message) = next_message().map_err(|_error| AppError::MessageLoop)?
            else {
                return Ok(());
            };

            self.process_message(&message)?;
        }
    }

    fn process_message(&mut self, message: &NativeMessage) -> Result<(), AppError> {
        let shell_fullscreen = fullscreen_notification(
            message.is_thread_message(),
            message.id(),
            message.parameter(),
        );
        if let Some(fullscreen) = shell_fullscreen {
            self.window_tracker.set_shell_fullscreen(fullscreen);
        }

        window_events::handle_tracker_message(
            message,
            &mut window_events::TrackerEventContext {
                runtime: self.runtime,
                dock: self.dock,
                graphics: self.graphics,
                surface: self.surface,
                window_tracker: self.window_tracker,
                dock_model: self.dock_model,
                auxiliary: self.auxiliary,
            },
        )?;
        if shell_fullscreen.is_some() && !self.runtime.onboarding_required {
            presentation::apply_fullscreen_visibility(
                self.dock,
                self.surface,
                self.window_tracker,
                self.dock_model,
                &mut self.auxiliary.launcher,
                &mut self.auxiliary.status,
            )?;
        }

        let wakes = WakeEvents::from_message(self.runtime, message.id());
        message.dispatch();
        let animation_tick = self.drain_events()?;
        self.process_wakes(wakes)?;
        presentation::sync_monitor_presentation(
            self.runtime,
            self.dock,
            self.surface,
            self.graphics,
            self.window_tracker,
            self.dock_model,
            self.auxiliary,
        )?;
        self.flush_frame(if animation_tick {
            FrameTrigger::AnimationTick
        } else {
            FrameTrigger::Changes
        })
    }

    fn drain_events(&mut self) -> Result<bool, AppError> {
        let animation_tick = window_events::drain_window_events(
            self.dock,
            self.graphics,
            self.surface,
            self.window_tracker.current_windows(),
            self.dock_model,
            self.auxiliary,
        )?;
        self.drain_settings_events()?;
        self.drain_switcher_events();
        for action in self.auxiliary.monitors.drain_events(self.graphics)? {
            dock_events::handle_monitor_dock_action(
                action,
                self.dock,
                self.graphics,
                self.dock_model,
                self.auxiliary,
            )?;
        }

        Ok(animation_tick)
    }

    fn process_wakes(&mut self, wakes: WakeEvents) -> Result<(), AppError> {
        if wakes.update {
            update_events::handle_update_results(&mut self.auxiliary.settings);
        }
        if wakes.badges
            && let Some(controller) = self.runtime.taskbar_badges
            && let Ok(snapshot) = controller.snapshot()
        {
            self.dock_model.set_notifications(snapshot);
            self.render_dock();
        }
        if wakes.media && self.auxiliary.media.drain(self.dock_model) {
            presentation::resize_dock(
                self.dock,
                self.graphics,
                self.surface,
                self.dock_model,
            )?;
            self.auxiliary.status.sync(
                self.dock,
                self.dock_model.settings(),
                self.dock_model.media(),
                self.graphics,
            )?;
            self.render_dock();
        }
        if wakes.windows_key
            && let Some(controller) = self.runtime.windows_key
        {
            dock_events::handle_windows_key_events(
                controller,
                self.dock,
                self.graphics,
                self.dock_model,
                &self.auxiliary.applications,
                &mut self.auxiliary.launcher,
            )?;
        }
        if wakes.alt_tab
            && let Some(controller) = self.runtime.alt_tab
        {
            controllers::handle_alt_tab_events(
                controller,
                self.window_tracker,
                self.dock_model,
                self.graphics,
                &mut self.auxiliary.switcher,
            );
        }
        if wakes.search_catalog {
            search_events::refresh_catalog(
                self.dock,
                self.graphics,
                self.surface,
                self.window_tracker.current_windows(),
                self.dock_model,
                self.auxiliary,
            )?;
        }

        Ok(())
    }

    fn render_dock(&mut self) {
        self.surface.invalidate();
    }

    fn drain_settings_events(&mut self) -> Result<(), AppError> {
        let events = self.auxiliary.settings.drain_events();
        for event in events {
            settings_events::handle_settings_event(
                event,
                &mut settings_events::SettingsEventContext {
                    dock: self.dock,
                    graphics: self.graphics,
                    dock_surface: self.surface,
                    window_tracker: self.window_tracker,
                    dock_model: self.dock_model,
                    auxiliary: self.auxiliary,
                },
            )?;
        }
        Ok(())
    }

    fn drain_switcher_events(&mut self) {
        for event in self.auxiliary.switcher.drain_events() {
            if let Err(error) = self.auxiliary.switcher.handle_window_event(event) {
                lotus_windows::diagnostics::record_error("alt_tab.event", &error);
                self.auxiliary.switcher.abandon();
                break;
            }
        }
    }

    fn flush_frame(&mut self, trigger: FrameTrigger) -> Result<(), AppError> {
        flush_frame(
            self.dock,
            self.graphics,
            self.surface,
            self.dock_model,
            self.auxiliary,
            trigger,
        )
    }
}

#[derive(Clone, Copy)]
struct WakeEvents {
    windows_key: bool,
    alt_tab: bool,
    search_catalog: bool,
    update: bool,
    media: bool,
    badges: bool,
}

impl WakeEvents {
    fn from_message(runtime: &RuntimePolicy<'_>, message: u32) -> Self {
        Self {
            windows_key: runtime.windows_key.is_some() && is_windows_key_wake(message),
            alt_tab: runtime.alt_tab.is_some() && is_alt_tab_wake(message),
            search_catalog: is_search_catalog_wake(message),
            update: is_update_wake(message),
            media: is_media_wake(message),
            badges: runtime.taskbar_badges.is_some() && is_taskbar_badge_wake(message),
        }
    }
}
