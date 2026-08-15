mod error;
mod platform;

pub mod activation;
pub mod alt_tab;
pub mod clipboard;
pub mod clock;
pub mod color_picker;
pub mod custom_image;
pub mod desktop;
pub mod dpi;
pub mod dwm_thumbnail;
pub mod exclusive_taskbar;
mod explorer_bridge;
mod font;
pub mod graphics;
pub mod image_picker;
pub mod launch;
pub mod media;
pub mod native_icon;
pub mod search_catalog;
mod shell_bridge;
pub mod single_instance;
pub mod startup;
pub mod taskbar_badges;
pub mod taskbar_state;
pub mod tray;
pub mod update;
pub mod window;
pub mod window_tracker;

pub use platform::windows::native_window::WindowHandle;
pub use platform::windows::{appbar, backdrop, dialog, interaction};
pub mod windows_key;

pub use error::NativeError;
