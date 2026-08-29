pub use lotus_dock::action_menu::Action as ContextMenuAction;
pub use lotus_dock::popup::{AppMenuAction, PickerWindow, PopupAction, PowerAction};
pub use lotus_dock::scene::{
    DockAnchor, DockBadge, DockHitTarget, DockMetrics, DockSize, MediaSymbols,
    SystemStatusKind,
};
pub use lotus_switcher::scene::SwitcherHitTarget;
use lotus_ui::embedded_icon::EmbeddedIcon;
use lotus_ui::icon::Icon;

pub type DockIcon = Icon<EmbeddedIcon>;
pub type DockItem = lotus_dock::scene::DockItem<EmbeddedIcon>;
pub type MediaItem = lotus_dock::scene::MediaItem<EmbeddedIcon>;
pub type DockScene = lotus_dock::scene::DockScene<EmbeddedIcon>;
pub type SystemStatusItem = lotus_dock::scene::SystemStatusItem<EmbeddedIcon>;
pub type ContextMenuScene = lotus_dock::popup::DockPopup<EmbeddedIcon>;
pub type NativePickerWindow = PickerWindow<EmbeddedIcon>;
pub type SwitcherItem = lotus_switcher::scene::SwitcherItem<DockIcon>;
pub type SwitcherScene = lotus_switcher::scene::SwitcherScene<DockIcon>;

pub fn surface_size(size: DockSize) -> lotus_windows::graphics::SurfaceSize {
    lotus_windows::graphics::SurfaceSize::new(size.width(), size.height())
        .expect("dock scenes always have nonzero dimensions")
}
