use lotus_settings::scene::SettingsAssets;
use lotus_ui::embedded_icon::EmbeddedIcon;
use lotus_ui::frame::FramePass;
use lotus_windows::graphics::DeviceState;

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

    runtime.surface.render_frame(pass, graphics, &presentation)
}
