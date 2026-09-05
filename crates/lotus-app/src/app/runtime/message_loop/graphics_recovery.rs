use lotus_windows::graphics::recovery::{
    GraphicsRecoverySchedule, GraphicsRecoveryScheduler as NativeGraphicsRecoveryScheduler,
};
use lotus_windows::graphics::{GraphicsDeviceHealth, SurfaceError};
use lotus_windows::interaction::NativeMessage;

use super::MessageLoop;
use crate::app::AppError;

pub(super) type GraphicsRecoveryScheduler = NativeGraphicsRecoveryScheduler;

impl MessageLoop<'_, '_> {
    pub(super) fn handle_graphics_recovery_wake(
        &mut self,
        message: &NativeMessage,
    ) -> Result<bool, AppError> {
        if !self.graphics_recovery.is_wake(message.id()) {
            return Ok(false);
        }
        if !self.graphics_recovery.take_wake(message.parameter()) {
            return Ok(true);
        }
        self.retry_graphics_recovery()?;
        Ok(true)
    }

    pub(super) fn schedule_graphics_recovery(&mut self) {
        if self.graphics.health() != GraphicsDeviceHealth::Lost {
            return;
        }
        match self.graphics_recovery.schedule() {
            GraphicsRecoverySchedule::Scheduled { attempt } => {
                lotus_windows::diagnostics::record_diagnostic(
                    "graphics.recovery_scheduled",
                    &format!("attempt={attempt}"),
                );
            }
            GraphicsRecoverySchedule::Exhausted => {
                lotus_windows::diagnostics::record_diagnostic(
                    "graphics.recovery_exhausted",
                    "attempts=3",
                );
            }
            GraphicsRecoverySchedule::Pending
            | GraphicsRecoverySchedule::AlreadyExhausted
            | GraphicsRecoverySchedule::Unavailable => {}
        }
    }

    fn retry_graphics_recovery(&mut self) -> Result<(), AppError> {
        if self.graphics.health() != GraphicsDeviceHealth::Lost {
            self.graphics_recovery.reset();
            return Ok(());
        }
        match self.graphics.recover() {
            Ok(()) => match self.recover_surfaces() {
                Ok(()) => {
                    self.graphics_recovery.reset();
                    self.primary_dock.invalidate();
                    self.auxiliary.invalidate_surfaces();
                    self.last_monitor_key = None;
                    lotus_windows::diagnostics::record_diagnostic(
                        "graphics.recovered",
                        &format!("generation={}", self.graphics.generation()),
                    );
                }
                Err(AppError::Surface(SurfaceError::DeviceLost(loss))) => {
                    self.graphics.mark_lost(loss);
                    self.schedule_graphics_recovery();
                }
                Err(error) => return Err(error),
            },
            Err(error) => {
                lotus_windows::diagnostics::record_error(
                    "graphics.recovery_failed",
                    &error,
                );
                self.schedule_graphics_recovery();
            }
        }
        Ok(())
    }

    fn recover_surfaces(&mut self) -> Result<(), AppError> {
        let device = self.graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
        self.primary_dock.recover_surface(device)?;
        self.auxiliary.recover_surfaces(device)
    }
}
