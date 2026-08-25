use windows::Win32::UI::WindowsAndMessaging::WM_APP;

pub(crate) const SEARCH_CATALOG_WAKE: u32 = WM_APP + 0x4C6;
pub(crate) const FULLSCREEN_NOTIFICATION: u32 = WM_APP + 0x4C8;
pub(crate) const MEDIA_WAKE: u32 = WM_APP + 0x4C9;
pub(crate) const UPDATE_WAKE: u32 = WM_APP + 0x4CA;
pub(crate) const TASKBAR_EVENT: u32 = WM_APP + 0x4CB;
pub(crate) const TASKBAR_BADGE_WAKE: u32 = WM_APP + 0x4CC;
pub(crate) const SEARCH_OUTSIDE_CLICK: u32 = WM_APP + 0x4CD;
pub(crate) const WINDOW_TRACKER_REFRESH: u32 = WM_APP + 0x4CE;
pub(crate) const INPUT_WAKE: u32 = WM_APP + 0x4CF;
pub(crate) const ALT_TAB_FALLBACK_REPLAY: u32 = WM_APP + 0x4D0;
pub(crate) const ICON_HYDRATION_WAKE: u32 = WM_APP + 0x4D1;
pub(crate) const SHELL_INTEGRATION_RECOVERY: u32 = WM_APP + 0x4D2;
pub(crate) const GRAPHICS_RECOVERY_WAKE: u32 = WM_APP + 0x4D3;
pub(crate) const INPUT_RESYNC: u32 = WM_APP + 0x4D4;
