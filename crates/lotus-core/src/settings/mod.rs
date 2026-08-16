mod codec;
mod model;
mod store;

pub use codec::{SettingsDecodeError, decode_settings};
pub use model::{
    CURRENT_APPEARANCE_VERSION, CURRENT_ONBOARDING_VERSION, DockSettings, DockZone,
    NotificationBadgeStyle, PinnedApp, WindowPickerStyle,
};
pub use store::{SettingsLoad, SettingsLoadSource, SettingsStore, SettingsStoreError};
