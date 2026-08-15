pub use lotus_dock::action_menu::{
    Action as ContextMenuAction, Direction as MenuDirection,
};
use lotus_dock::popup::DockPopup;
pub use lotus_dock::popup::{
    AppMenuAction, PickerWindow, PopupAction, PopupEntry, PopupIcon, PopupSymbol,
    PowerAction,
};

use super::assets::SvgAsset;

pub type ContextMenuScene = DockPopup<SvgAsset>;
pub type NativePickerWindow = PickerWindow<SvgAsset>;
