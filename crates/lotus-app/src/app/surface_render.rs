use lotus_ui::frame::FrameOutcome;
use lotus_windows::graphics::surface::FrameResult;
use lotus_windows::graphics::{DeviceState, SurfaceError};

use crate::app::AppError;

pub(super) fn frame_outcome(
    graphics: &mut DeviceState,
    result: Result<FrameResult, SurfaceError>,
) -> Result<FrameOutcome, AppError> {
    match result {
        Ok(FrameResult::Presented { needs_animation }) => {
            Ok(FrameOutcome::complete(needs_animation))
        }
        Ok(FrameResult::TargetRecreated) => Ok(FrameOutcome::Retry),
        Err(SurfaceError::DeviceLost(loss)) => {
            graphics.mark_lost(loss);
            Ok(FrameOutcome::complete(false))
        }
        Err(error) => Err(error.into()),
    }
}
