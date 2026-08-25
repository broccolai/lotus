mod codec;
mod model;
mod store;

pub use codec::{SettingsDecodeError, decode_settings};
pub use model::{
    ApplicationIconOverride, CURRENT_APPEARANCE_VERSION, CURRENT_ONBOARDING_VERSION,
    CURRENT_SETTINGS_SCHEMA_VERSION, DockSettings, DockZone, NotificationBadgeStyle,
    PinnedApp, UpdateChannel, WindowPickerStyle, merge_application_icon_overrides,
};
pub use store::{
    SettingsLoad, SettingsLoadSource, SettingsReset, SettingsStore, SettingsStoreError,
};
