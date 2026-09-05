use std::time::{Duration, Instant};

use lotus_ui::frame::ScheduledSurface;
use lotus_windows::appbar::{
    ShellIntegration, ShellIntegrationHealth, ShellRecoverySource,
};
use lotus_windows::graphics::{CompositionSurfaceState, DeviceState};
use lotus_windows::interaction::NativeMessage;
use lotus_windows::system_lifecycle::{
    SystemLifecycleEvent, SystemLifecycleHealth, SystemLifecycleObserver,
};
use lotus_windows::window::DockWindow;
use lotus_windows::window_tracker::WindowTracker;

use super::DockRuntime;
use super::modules::ModuleHost;
use super::runtime::apply_fullscreen_visibility;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum IntegrationRecoverySource {
    TaskbarCreated,
    Settings,
    DisplayChange,
    SystemResume,
    SessionUnlock,
    AppBarPositionChanged,
}

impl IntegrationRecoverySource {
    const fn diagnostic_name(self) -> &'static str {
        match self {
            Self::TaskbarCreated => "taskbar_created",
            Self::Settings => "settings",
            Self::DisplayChange => "display_change",
            Self::SystemResume => "system_resume",
            Self::SessionUnlock => "session_unlock",
            Self::AppBarPositionChanged => "appbar_position_changed",
        }
    }

    const fn shell_source(self) -> ShellRecoverySource {
        match self {
            Self::TaskbarCreated => ShellRecoverySource::TaskbarCreated,
            Self::Settings => ShellRecoverySource::Settings,
            Self::DisplayChange => ShellRecoverySource::DisplayChange,
            Self::SystemResume => ShellRecoverySource::SystemResume,
            Self::SessionUnlock => ShellRecoverySource::SessionUnlock,
            Self::AppBarPositionChanged => ShellRecoverySource::AppBarPositionChanged,
        }
    }
}

pub(super) struct IntegrationRecovery {
    shell: ShellIntegration,
    lifecycle: SystemLifecycleObserver,
    shell_effects_allowed: bool,
    last_maintenance: Option<Instant>,
}

pub(super) struct IntegrationRecoveryContext<'a> {
    pub(super) dock: &'a DockWindow,
    pub(super) graphics: &'a mut DeviceState,
    pub(super) dock_surface: &'a mut ScheduledSurface<CompositionSurfaceState>,
    pub(super) window_tracker: &'a mut WindowTracker,
    pub(super) dock_model: &'a mut DockRuntime,
    pub(super) auxiliary: &'a mut ModuleHost,
}

impl IntegrationRecovery {
    pub(super) fn new(
        settings: &lotus_core::settings::DockSettings,
        dock: &DockWindow,
        shell_effects_allowed: bool,
        integration_enabled: bool,
    ) -> Self {
        Self {
            shell: ShellIntegration::new(settings, dock, integration_enabled),
            lifecycle: SystemLifecycleObserver::register(dock.handle()),
            shell_effects_allowed,
            last_maintenance: None,
        }
    }

    pub(super) const fn shell_effects_allowed(&self) -> bool {
        self.shell_effects_allowed
    }

    pub(super) const fn requires_maintenance(&self) -> bool {
        self.shell_effects_allowed && self.shell.requires_maintenance()
    }

    pub(super) fn maintain(
        &mut self,
        settings: &lotus_core::settings::DockSettings,
        dock: &DockWindow,
    ) -> bool {
        if !self.requires_maintenance()
            || self
                .last_maintenance
                .is_some_and(|last| last.elapsed() < Duration::from_secs(1))
        {
            return false;
        }
        self.last_maintenance = Some(Instant::now());
        self.shell.maintain(settings, dock)
    }

    pub(super) fn recovery_source(
        &self,
        message: &NativeMessage,
        dock: &DockWindow,
    ) -> Option<IntegrationRecoverySource> {
        if !self.shell_effects_allowed {
            return None;
        }
        if let Some(source) = self
            .shell
            .take_recovery_request(message.is_thread_message(), message.id())
        {
            return Some(match source {
                ShellRecoverySource::TaskbarCreated => {
                    IntegrationRecoverySource::TaskbarCreated
                }
                ShellRecoverySource::AppBarPositionChanged => {
                    IntegrationRecoverySource::AppBarPositionChanged
                }
                _ => unreachable!("only queued shell recovery sources are returned"),
            });
        }
        if message.target_window() != Some(dock.handle()) {
            return None;
        }
        let event = self.lifecycle.classify(message.id(), message.parameter())?;
        lotus_windows::diagnostics::record_diagnostic(
            "system_lifecycle.event",
            event.diagnostic_name(),
        );
        match event {
            SystemLifecycleEvent::DisplayChanged
            | SystemLifecycleEvent::DpiChanged
            | SystemLifecycleEvent::WorkAreaChanged => {
                Some(IntegrationRecoverySource::DisplayChange)
            }
            SystemLifecycleEvent::Resumed => Some(IntegrationRecoverySource::SystemResume),
            SystemLifecycleEvent::SessionUnlocked => {
                Some(IntegrationRecoverySource::SessionUnlock)
            }
            SystemLifecycleEvent::Suspending | SystemLifecycleEvent::SessionLocked => None,
        }
    }

    pub(super) fn diagnostic_snapshot(
        &self,
        graphics: &DeviceState,
        auxiliary: &ModuleHost,
    ) -> String {
        let lifecycle = match self.lifecycle.health() {
            SystemLifecycleHealth::Healthy => "healthy",
            SystemLifecycleHealth::Degraded => "degraded",
        };
        let input = if auxiliary.input_healthy() {
            "healthy"
        } else {
            "degraded"
        };
        let tray = lotus_windows::tray::current_health()
            .map_or("not_initialized".to_owned(), |health| format!("{health:?}"));

        format!(
            "shell={:?}\nlifecycle={}\ngraphics={:?}\ntray={}\nmonitors={:?}\nmonitor_replicas={}\nmonitor_topology_generation={}\ninput={}",
            self.shell.health(),
            lifecycle,
            graphics.health(),
            tray,
            auxiliary.monitor_integration_health(),
            auxiliary.monitor_replica_count(),
            auxiliary.monitor_topology_generation(),
            input,
        )
    }

    pub(super) fn recover(
        &mut self,
        source: IntegrationRecoverySource,
        context: &mut IntegrationRecoveryContext<'_>,
    ) {
        lotus_windows::diagnostics::record_diagnostic(
            "integration.recovery_requested",
            &format!("source={}", source.diagnostic_name()),
        );
        if !self.shell_effects_allowed {
            return;
        }
        if source == IntegrationRecoverySource::AppBarPositionChanged {
            self.shell.recover(
                context.dock_model.settings(),
                context.dock,
                source.shell_source(),
            );
            context.dock_surface.invalidate();
            lotus_windows::diagnostics::record_diagnostic(
                "integration.appbar_position_reconciled",
                &format!("shell={:?}", self.shell.health()),
            );
            return;
        }
        self.lifecycle.recover_registration();
        let mut degraded = false;
        if let Err(error) = context
            .dock
            .refresh_placement(context.dock_model.settings())
        {
            degraded = true;
            lotus_windows::diagnostics::record_error(
                "integration.primary_placement_failed",
                &error,
            );
        }
        self.shell.recover(
            context.dock_model.settings(),
            context.dock,
            source.shell_source(),
        );
        degraded |= self.shell.health() == ShellIntegrationHealth::Degraded;

        let tray_health = lotus_windows::tray::recover();
        degraded |= tray_health == lotus_windows::tray::TrayIntegrationHealth::Degraded;

        if context.graphics.poll() || context.graphics.lost().is_some() {
            if let Some(loss) = context.graphics.lost() {
                lotus_windows::diagnostics::record_diagnostic(
                    "graphics.loss_detected",
                    &loss.to_string(),
                );
            }
            degraded = true;
        }

        context.window_tracker.refresh_fullscreen();
        if let Err(error) = context.auxiliary.refresh_placement(
            context.dock,
            context.dock_model,
            context.graphics,
        ) {
            error.mark_graphics_lost(context.graphics);
            degraded = true;
            lotus_windows::diagnostics::record_error(
                "integration.auxiliary_placement_failed",
                &error,
            );
        }
        if let Err(error) = context.auxiliary.sync_monitor_docks(
            context.dock,
            context.dock_model,
            context.graphics,
            context.window_tracker,
        ) {
            error.mark_graphics_lost(context.graphics);
            degraded = true;
            lotus_windows::diagnostics::record_error(
                "integration.monitor_recovery_failed",
                &error,
            );
        }
        if let Err(error) = apply_fullscreen_visibility(
            context.dock,
            context.dock_surface,
            context.window_tracker,
            context.dock_model,
            context.auxiliary,
        ) {
            degraded = true;
            lotus_windows::diagnostics::record_error(
                "integration.presentation_recovery_failed",
                &error,
            );
        }

        context.dock_surface.invalidate();
        context.auxiliary.invalidate_surfaces();
        self.record_recovery_outcome(source, context, degraded, tray_health);
    }

    fn record_recovery_outcome(
        &self,
        source: IntegrationRecoverySource,
        context: &IntegrationRecoveryContext<'_>,
        mut degraded: bool,
        tray_health: lotus_windows::tray::TrayIntegrationHealth,
    ) {
        let lifecycle_health = match self.lifecycle.health() {
            SystemLifecycleHealth::Healthy => "healthy",
            SystemLifecycleHealth::Degraded => {
                degraded = true;
                "degraded"
            }
        };
        let input_health = if context.auxiliary.input_healthy() {
            "healthy"
        } else {
            degraded = true;
            "degraded"
        };
        lotus_windows::diagnostics::record_diagnostic(
            if degraded {
                "integration.recovery_degraded"
            } else {
                "integration.recovery_succeeded"
            },
            &format!(
                "source={} shell={:?} lifecycle={} graphics={:?} tray={:?} monitors={:?} replicas={} topology={} input={}",
                source.diagnostic_name(),
                self.shell.health(),
                lifecycle_health,
                context.graphics.health(),
                tray_health,
                context.auxiliary.monitor_integration_health(),
                context.auxiliary.monitor_replica_count(),
                context.auxiliary.monitor_topology_generation(),
                input_health,
            ),
        );
    }
}
