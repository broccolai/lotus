use lotus_settings::scene::SettingsAssets;
use lotus_ui::frame::{FrameOutcome, FramePass};
use lotus_windows::graphics::assets::SvgAsset;
use lotus_windows::graphics::surface::FrameResult;
use lotus_windows::graphics::{DeviceState, SurfaceError};

use super::SettingsRuntime;
use crate::app::AppError;

pub(super) fn render_frame(
    runtime: &mut SettingsRuntime,
    pass: &mut FramePass,
    graphics: &mut DeviceState,
) -> Result<(), AppError> {
    if !runtime.is_visible() {
        return Ok(());
    }

    let presentation = runtime.scene.presentation(
        &SettingsAssets {
            lotus: SvgAsset::LotusPixel,
            search: SvgAsset::FluentSearch,
        },
        lotus_windows::backdrop::settings_uses_translucent_material(runtime.scene.draft()),
    );

    let Some(surface) = runtime.surface.as_mut() else {
        return Err(AppError::InvalidSettingsScene);
    };
    let render = |surface: &mut lotus_windows::graphics::settings_surface::SettingsCompositionSurfaceState| {
        surface.render_scene(&presentation)
    };

    pass.render(surface, |surface| match render(surface) {
        Ok(FrameResult::Presented { .. }) => Ok(FrameOutcome::complete(false)),
        Ok(FrameResult::TargetRecreated) => Ok(FrameOutcome::Retry),
        Err(SurfaceError::DeviceLost(_)) => {
            recover_device(graphics)?;

            let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
            surface.recover(device)?;

            match render(surface)? {
                FrameResult::Presented { .. } => Ok(FrameOutcome::complete(false)),
                FrameResult::TargetRecreated => Ok(FrameOutcome::Retry),
            }
        }
        Err(error) => Err(error.into()),
    })
}

pub(super) fn recover_device(graphics: &mut DeviceState) -> Result<(), AppError> {
    let _ = graphics.poll();
    graphics.recover()?;
    Ok(())
}
