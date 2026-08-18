pub mod assets;
mod composition_surface;
mod context_menu_renderer;
pub mod context_menu_surface;
mod device;
mod launcher_renderer;
pub mod launcher_surface;
mod renderer;
mod resources;
pub mod scene;
pub mod scene_adapter;
mod settings_renderer;
pub mod settings_surface;
pub mod surface;
mod switcher_renderer;
pub mod switcher_surface;
mod theme;

use assets::SvgAsset;
pub use device::{DeviceState, GraphicsDevice, GraphicsDeviceError};
pub use lotus_dock::action_menu::{
    Action as ContextMenuAction, Direction as MenuDirection,
};
pub use lotus_dock::popup::{
    AppMenuAction, PickerWindow, PopupAction, PopupEntry, PopupIcon, PopupSymbol,
    PowerAction,
};
pub use lotus_search::scene::{
    LauncherLayout, LauncherResultKind, LauncherSize, PixelRect,
};
pub use lotus_settings::scene::{
    OnboardingModule, OnboardingStep, SettingsAction, SettingsControl, SettingsKey,
    SettingsLayout, SettingsPage, SettingsRect, SettingsScene, SettingsSize,
    SettingsSlider, SettingsToggle, SettingsUpdateActivity,
};
pub use lotus_switcher::scene::{LaidOutItem, SwitcherHitTarget, SwitcherLayout};
use scene::DockIcon;
pub use surface::{CompositionSurfaceState, SurfaceError, SurfaceSize};

pub type ContextMenuScene = lotus_dock::popup::DockPopup<SvgAsset>;
pub type NativePickerWindow = PickerWindow<SvgAsset>;
pub type LauncherResult = lotus_search::scene::LauncherResult<SvgAsset>;
pub type LauncherScene = lotus_search::scene::LauncherScene<SvgAsset>;
pub type SwitcherItem = lotus_switcher::scene::SwitcherItem<DockIcon>;
pub type SwitcherScene = lotus_switcher::scene::SwitcherScene<DockIcon>;
