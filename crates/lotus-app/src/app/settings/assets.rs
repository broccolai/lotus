use lotus_settings::scene::SettingsAssets;
use lotus_ui::embedded_icon::EmbeddedIcon;
use lotus_ui::frame::{FrameOutcome, FramePass};
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
            lotus: EmbeddedIcon::LotusPixel,
            search: EmbeddedIcon::FluentSearch,
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
        Err(SurfaceError::DeviceLost(loss)) => {
            graphics.mark_lost(loss);
            Ok(FrameOutcome::complete(false))
        }
        Err(error) => Err(error.into()),
    })
}
