use std::collections::HashMap;
use std::path::Path;

use lotus_core::dock::DockItem;
use lotus_core::notification::count_for_item;
use lotus_core::settings::{DockSettings, DockZone, NotificationBadgeStyle};
use lotus_core::window::WindowInfo;
use lotus_dock::model::project_snapshot;
use lotus_windows::clock::{local_date, local_time};
use lotus_windows::graphics::assets::SvgAsset;
use lotus_windows::graphics::scene::{
    DockAnchor, DockBadge, DockIcon, DockMetrics, SystemStatusItem, SystemStatusKind,
};
use lotus_windows::graphics::scene_adapter::adapt_dock_items_with_native;
use lotus_windows::launch::resolve_executable;

use super::{DockRuntime, NATIVE_ICON_SAMPLE_SCALE, SceneDockItem};
use crate::app::AppError;

impl DockRuntime {
    pub(in crate::app) fn scene_items(&mut self) -> Vec<SceneDockItem> {
        let icon_size = self
            .scene
            .icon_size_pixels()
            .saturating_mul(NATIVE_ICON_SAMPLE_SCALE);
        self.scene_items_at_size(icon_size)
    }

    pub(in crate::app) fn scene_items_at_size(
        &mut self,
        icon_size: u32,
    ) -> Vec<SceneDockItem> {
        if !self.model.settings().show_app_dock {
            return Vec::new();
        }

        let settings = self.model.settings().clone();
        let mut scene_items =
            adapt_dock_items_with_native(self.model.items(), |_, item| {
                crate::app::icon_override::resolve_application_icon(
                    &settings,
                    &mut self.custom_images,
                    item.app_user_model_id.as_deref(),
                    Some(&item.id),
                    Path::new(&item.executable_path),
                )
                .or_else(|| {
                    self.native_icons
                        .icon(Path::new(&item.icon_source), icon_size)
                        .ok()
                        .flatten()
                })
            });
        for item in &mut scene_items {
            let Some(source) = self.model.items().get(item.source_index()) else {
                continue;
            };
            let notification_count = count_for_item(
                source,
                &self.notifications,
                &self.model.settings().notification_disabled_apps,
            );
            let count = notification_count.value;
            let badge = match (self.model.settings().notification_badge_style, count) {
                (_, 0) | (NotificationBadgeStyle::Off, _) => None,
                (NotificationBadgeStyle::Dot, _) => Some(DockBadge::Dot),
                (NotificationBadgeStyle::Count, count)
                    if notification_count.is_lower_bound
                        && count == notification_count.value =>
                {
                    Some(DockBadge::AtLeast(count))
                }
                (NotificationBadgeStyle::Count, count) => Some(DockBadge::Count(count)),
            };
            item.set_badge(badge);
            item.set_running(
                source.is_running() && self.model.settings().show_running_indicators,
            );
        }
        scene_items
    }
}

pub(in crate::app) fn departure_transition(
    previous_model: &[DockItem],
    previous_scene: &[SceneDockItem],
    current_model: &[DockItem],
    current_scene: &[SceneDockItem],
) -> Option<Vec<SceneDockItem>> {
    let current_indices = current_model
        .iter()
        .enumerate()
        .map(|(index, item)| (item.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut current_visuals = current_scene.iter().cloned().map(Some).collect::<Vec<_>>();
    let mut transition = Vec::with_capacity(previous_scene.len().max(current_scene.len()));
    let mut departing = false;

    for previous in previous_scene {
        let Some(identity) = previous_model
            .get(previous.source_index())
            .map(|item| item.id.as_str())
        else {
            continue;
        };
        if let Some(&current_index) = current_indices.get(identity)
            && let Some(position) = current_visuals.iter().position(|item| {
                item.as_ref()
                    .is_some_and(|item| item.source_index() == current_index)
            })
        {
            transition.push(current_visuals[position].take().expect("item is present"));
        } else {
            let mut exiting = previous.clone();
            exiting.set_source_index(usize::MAX);
            exiting.set_exiting(true);
            transition.push(exiting);
            departing = true;
        }
    }

    transition.extend(current_visuals.into_iter().flatten());
    departing.then_some(transition)
}

pub(in crate::app) fn status_items(settings: &DockSettings) -> Vec<SystemStatusItem> {
    if !settings.show_system_status {
        return Vec::new();
    }

    let mut items = Vec::with_capacity(4);
    if settings.show_volume_status {
        items.push(SystemStatusItem::icon(
            SystemStatusKind::Volume,
            SvgAsset::FluentVolume,
        ));
    }
    if settings.show_network_status {
        let item = match lotus_windows::network::connection_kind() {
            lotus_windows::network::NetworkConnectionKind::Ethernet => {
                SystemStatusItem::symbol(SystemStatusKind::Network, '\u{E839}')
            }
            lotus_windows::network::NetworkConnectionKind::Wifi
            | lotus_windows::network::NetworkConnectionKind::Other => {
                SystemStatusItem::icon(SystemStatusKind::Network, SvgAsset::FluentNetwork)
            }
        };
        items.push(item);
    }
    if settings.show_background_apps_status {
        items.push(SystemStatusItem::icon(
            SystemStatusKind::BackgroundApps,
            SvgAsset::FluentTray,
        ));
    }
    if settings.show_date_time_status {
        let date = if settings.show_date_in_status {
            local_date()
        } else {
            String::new()
        };
        items.push(SystemStatusItem::date_time(
            local_time(settings.use_24_hour_time),
            date,
        ));
    }
    items
}

pub(in crate::app) fn docked_status_items(
    settings: &DockSettings,
) -> Vec<SystemStatusItem> {
    if settings.system_status_zone == settings.dock_zone {
        status_items(settings)
    } else {
        Vec::new()
    }
}

pub(in crate::app) fn projected_items(
    settings: &DockSettings,
    windows: &[WindowInfo],
) -> Vec<DockItem> {
    project_snapshot(settings, windows, |target| {
        resolve_executable(target).map(|path| path.to_string_lossy().into_owned())
    })
}

pub(in crate::app) fn media_identity_matches(source_id: &str, candidate: &str) -> bool {
    let source = source_id.to_ascii_lowercase();
    let candidate = candidate.to_ascii_lowercase();
    let candidate = Path::new(&candidate)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(&candidate);
    !candidate.is_empty()
        && (source == candidate
            || source.ends_with(&format!("\\{candidate}.exe"))
            || source.contains(candidate))
}

pub(in crate::app) fn window_matches_item(window: &WindowInfo, item: &DockItem) -> bool {
    if window
        .executable_path
        .to_string_lossy()
        .eq_ignore_ascii_case(&item.executable_path)
    {
        return true;
    }

    let window_name = window
        .executable_path
        .file_name()
        .and_then(|name| name.to_str());
    let item_name = Path::new(&item.executable_path)
        .file_name()
        .and_then(|name| name.to_str());
    window_name
        .zip(item_name)
        .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
}

pub(in crate::app) fn metrics(settings: &DockSettings) -> Result<DockMetrics, AppError> {
    DockMetrics::new(
        settings.icon_size,
        settings.item_spacing,
        settings.horizontal_padding,
        settings.vertical_padding,
    )
    .ok_or(AppError::InvalidScene)
}

pub(in crate::app) fn mascot(settings: &DockSettings) -> DockIcon {
    settings
        .mascot_image_path
        .as_deref()
        .and_then(|path| {
            lotus_windows::custom_image::load_custom_image(Path::new(path)).ok()
        })
        .map_or(DockIcon::Embedded(SvgAsset::LotusPixel), DockIcon::Raster)
}

pub(in crate::app) const fn dock_anchor(zone: DockZone) -> DockAnchor {
    match zone {
        DockZone::Left => DockAnchor::Left,
        DockZone::Center => DockAnchor::Center,
        DockZone::Right => DockAnchor::Right,
    }
}

pub(in crate::app) fn popup_overlap(dpi: u32) -> i32 {
    i32::try_from((u64::from(dpi) * 6 + 48) / 96).unwrap_or(6)
}

pub(in crate::app) fn status_popup_center<Asset>(
    items: &[lotus_dock::scene::LaidOutStatusItem<Asset>],
) -> Option<u32> {
    if let Some(item) = items
        .iter()
        .find(|item| item.kind == SystemStatusKind::BackgroundApps)
    {
        return Some(
            item.hit_bounds
                .left
                .saturating_add(item.hit_bounds.width / 2),
        );
    }
    let left = items.iter().map(|item| item.hit_bounds.left).min()?;
    let right = items
        .iter()
        .map(|item| item.hit_bounds.left.saturating_add(item.hit_bounds.width))
        .max()?;
    Some(left.saturating_add(right.saturating_sub(left) / 2))
}
