use lotus_core::settings::DockSettings;
use lotus_windows::WindowHandle;

pub(in crate::app) fn status_items(
    settings: &DockSettings,
    owner: WindowHandle,
) -> Vec<crate::app::visuals::SystemStatusItem> {
    if !settings.show_system_status {
        return Vec::new();
    }
    let snapshot = super::projection::StatusSnapshot {
        advanced_color_label: if settings.show_hdr_status {
            lotus_windows::advanced_color::state(owner)
                .unwrap_or(lotus_windows::advanced_color::AdvancedColorState::Sdr)
                .label()
                .to_owned()
        } else {
            String::new()
        },
        ethernet: settings.show_network_status
            && matches!(
                lotus_windows::network::connection_kind(),
                lotus_windows::network::NetworkConnectionKind::Ethernet
            ),
        date: (settings.show_date_time_status && settings.show_date_in_status)
            .then(lotus_windows::clock::local_date)
            .unwrap_or_default(),
        time: if settings.show_date_time_status {
            lotus_windows::clock::local_time(settings.use_24_hour_time)
        } else {
            String::new()
        },
    };
    super::projection::status_items(settings, &snapshot)
}

pub(in crate::app) fn docked_status_items(
    settings: &DockSettings,
    owner: WindowHandle,
) -> Vec<crate::app::visuals::SystemStatusItem> {
    if settings.system_status_zone == settings.dock_zone {
        status_items(settings, owner)
    } else {
        Vec::new()
    }
}
