use std::collections::HashMap;

use lotus_core::application::{
    PinnedApplicationAssignment, RegisteredApplication, WindowApplicationAssignments,
};
use lotus_core::dock::DockItem;
use lotus_core::notification::{NotificationSource, count_for_item};
use lotus_core::settings::{DockSettings, DockZone, NotificationBadgeStyle};
use lotus_core::window::WindowInfo;
use lotus_dock::model::project_snapshot;
use lotus_media::MediaSnapshot;
use lotus_ui::embedded_icon::EmbeddedIcon;
use lotus_ui::icon::RasterIcon;
use lotus_windows::search_catalog::ApplicationCatalogSnapshot;

use super::SceneDockItem;
use crate::app::AppError;
use crate::app::visuals::{
    DockAnchor, DockBadge, DockIcon, DockMetrics, MediaItem, MediaSymbols,
    SystemStatusItem, SystemStatusKind,
};

pub(in crate::app) struct StatusSnapshot {
    pub(super) advanced_color_label: String,
    pub(super) ethernet: bool,
    pub(super) date: String,
    pub(super) time: String,
}

pub(in crate::app) fn scene_items(
    items: &[DockItem],
    icons: &[Option<RasterIcon>],
    settings: &DockSettings,
    notifications: &[NotificationSource],
    catalog: &ApplicationCatalogSnapshot,
) -> Vec<SceneDockItem> {
    if !settings.show_app_dock {
        return Vec::new();
    }
    items
        .iter()
        .zip(icons)
        .enumerate()
        .filter_map(|(source_index, (source, icon))| {
            let mut item = SceneDockItem::with_source_index(
                source_index,
                DockIcon::Raster(icon.clone()?),
            );
            let notification_count = count_for_item(
                source,
                notifications,
                &settings.notification_disabled_apps,
                |identifier| catalog.key_for_external_identifier(identifier),
            );
            let count = notification_count.value;
            let badge = match (settings.notification_badge_style, count) {
                (_, 0) | (NotificationBadgeStyle::Off, _) => None,
                (NotificationBadgeStyle::Dot, _) => Some(DockBadge::Dot),
                (NotificationBadgeStyle::Count, count)
                    if notification_count.is_lower_bound =>
                {
                    Some(DockBadge::AtLeast(count))
                }
                (NotificationBadgeStyle::Count, count) => Some(DockBadge::Count(count)),
            };
            item.set_badge(badge);
            item.set_running(source.is_running() && settings.show_running_indicators);
            Some(item)
        })
        .collect()
}

pub(in crate::app) fn media_item(
    snapshot: &MediaSnapshot,
    artwork: DockIcon,
    show_metadata: bool,
) -> MediaItem {
    MediaItem {
        source_id: snapshot.source_id.clone(),
        title: snapshot.title.clone(),
        artist: snapshot.artist.clone(),
        show_metadata,
        artwork,
        controls: snapshot.controls,
        playback: snapshot.playback,
        symbols: MediaSymbols {
            previous: EmbeddedIcon::FluentPrevious,
            play: EmbeddedIcon::FluentPlay,
            pause: EmbeddedIcon::FluentPause,
            next: EmbeddedIcon::FluentNext,
        },
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
    let current_visual_positions = current_scene.iter().enumerate().fold(
        HashMap::<usize, Vec<usize>>::new(),
        |mut positions, (position, item)| {
            positions
                .entry(item.source_index())
                .or_default()
                .push(position);
            positions
        },
    );
    let mut next_visual_positions = HashMap::<usize, usize>::new();
    let mut transition = Vec::with_capacity(previous_scene.len().max(current_scene.len()));
    let mut departing = false;

    for previous in previous_scene {
        let Some(identity) = previous_model
            .get(previous.source_index())
            .map(|item| item.id.as_str())
        else {
            continue;
        };
        let position = current_indices.get(identity).and_then(|&current_index| {
            let positions = current_visual_positions.get(&current_index)?;
            let position_index = next_visual_positions.entry(current_index).or_default();
            let position = positions.get(*position_index).copied();
            *position_index = (*position_index).saturating_add(1);
            position
        });
        if let Some(position) = position
            && current_visuals[position].is_some()
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

pub(in crate::app) fn status_items(
    settings: &DockSettings,
    snapshot: &StatusSnapshot,
) -> Vec<SystemStatusItem> {
    if !settings.show_system_status {
        return Vec::new();
    }

    let mut items = Vec::with_capacity(5);
    if settings.show_volume_status {
        items.push(SystemStatusItem::icon(
            SystemStatusKind::Volume,
            EmbeddedIcon::FluentVolume,
        ));
    }
    if settings.show_hdr_status {
        items.push(SystemStatusItem::text(
            SystemStatusKind::AdvancedColor,
            &snapshot.advanced_color_label,
        ));
    }
    if settings.show_network_status {
        let item = if snapshot.ethernet {
            SystemStatusItem::symbol(SystemStatusKind::Network, '\u{E839}')
        } else {
            SystemStatusItem::icon(SystemStatusKind::Network, EmbeddedIcon::FluentNetwork)
        };
        items.push(item);
    }
    if settings.show_background_apps_status {
        items.push(SystemStatusItem::icon(
            SystemStatusKind::BackgroundApps,
            EmbeddedIcon::FluentTray,
        ));
    }
    if settings.show_date_time_status {
        items.push(SystemStatusItem::date_time(
            snapshot.time.clone(),
            snapshot.date.clone(),
        ));
    }
    items
}

pub(in crate::app) fn projected_items(
    settings: &DockSettings,
    windows: &[WindowInfo],
    assignments: &WindowApplicationAssignments,
    applications: &[RegisteredApplication],
    pinned_applications: &[PinnedApplicationAssignment],
) -> Vec<DockItem> {
    project_snapshot(
        settings,
        windows,
        assignments,
        applications,
        pinned_applications,
    )
}

pub(in crate::app) fn media_source_matches_item(
    source_id: &str,
    item: &DockItem,
    catalog: &ApplicationCatalogSnapshot,
) -> bool {
    catalog.key_for_external_identifier(source_id).as_ref() == Some(&item.application_key)
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
