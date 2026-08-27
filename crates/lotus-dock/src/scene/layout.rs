use lotus_media::{MediaWidgetLayout, PlaybackState};

use super::{
    DockIcon, DockLayout, DockScene, DockSize, LaidOutItem, LaidOutMedia,
    LaidOutMediaControl, LaidOutStatusIcon, LaidOutStatusItem, PixelRect, SystemStatusItem,
    SystemStatusKind, nonzero_or_one, scale_dips,
};

const DIVIDER_WIDTH_DIP: u32 = 2;
const DIVIDER_HEIGHT_DIP: u32 = 18;
const SHOW_DESKTOP_WIDTH_DIP: u32 = 10;
const STATUS_ICON_SIZE_DIP: u32 = 18;
const STATUS_ICON_SLOT_WIDTH_DIP: u32 = 32;
const STATUS_CLOCK_WIDTH_DIP: u32 = 72;

impl<Asset: Clone> DockScene<Asset> {
    pub fn icon_size_pixels(&self) -> u32 {
        nonzero_or_one(self.scaled_metrics().icon_size).get()
    }

    pub fn desired_size(&self) -> DockSize {
        let metrics = self.scaled_metrics();
        let item_count = u32::try_from(self.items.len()).unwrap_or(u32::MAX);
        let slot_width = metrics
            .icon_size
            .saturating_add(metrics.spacing.saturating_mul(2));
        let item_strip_width = item_count.saturating_mul(slot_width);
        let media_width = self
            .media
            .as_ref()
            .and_then(|media| {
                MediaWidgetLayout::new(
                    self.dpi(),
                    self.desired_height_dips(),
                    media.show_metadata,
                )
            })
            .map_or(0, |layout| layout.width);
        let show_desktop_width = self
            .show_desktop_button
            .then_some(metrics.show_desktop_width);
        let status_width = self.status_items.iter().fold(0_u32, |width, item| {
            width.saturating_add(match item.kind {
                SystemStatusKind::DateTime => metrics.status_clock_width,
                SystemStatusKind::Volume
                | SystemStatusKind::AdvancedColor
                | SystemStatusKind::Network
                | SystemStatusKind::BackgroundApps => metrics.status_icon_slot_width,
            })
        });
        let media_chrome_width = self.segment_chrome_width(self.media_separator_visible());
        let status_chrome_width =
            self.segment_chrome_width(self.status_separator_visible());
        let launcher_width = self.launcher_button_visible.then_some(
            slot_width
                .saturating_add(metrics.spacing)
                .saturating_add(metrics.divider_width)
                .saturating_add(metrics.spacing),
        );
        let width = metrics
            .horizontal_padding
            .saturating_mul(2)
            .saturating_add(item_strip_width)
            .saturating_add(launcher_width.unwrap_or_default())
            .saturating_add(media_chrome_width)
            .saturating_add(media_width)
            .saturating_add(status_chrome_width)
            .saturating_add(status_width)
            .saturating_add(show_desktop_width.unwrap_or_default());
        let height = metrics
            .icon_size
            .saturating_add(metrics.vertical_padding.saturating_mul(2));

        DockSize {
            width: nonzero_or_one(width),
            height: nonzero_or_one(height),
        }
    }

    pub fn layout(&self, surface_width: u32, surface_height: u32) -> DockLayout<Asset> {
        let metrics = self.scaled_metrics();
        let desired = self.desired_size();
        let content_left = surface_width.saturating_sub(desired.width()) / 2;
        let content_top = surface_height.saturating_sub(desired.height()) / 2;
        let icon_top = content_top.saturating_add(metrics.vertical_padding);
        let mut cursor = content_left.saturating_add(metrics.horizontal_padding);
        let slot_width = metrics
            .icon_size
            .saturating_add(metrics.spacing.saturating_mul(2));
        let jirachi_hit_bounds = PixelRect {
            left: cursor,
            top: icon_top,
            width: slot_width,
            height: metrics.icon_size,
        };
        let jirachi = PixelRect::square(
            cursor.saturating_add(metrics.spacing),
            icon_top,
            metrics.icon_size,
        );
        let divider = PixelRect {
            left: cursor
                .saturating_add(slot_width)
                .saturating_add(metrics.spacing),
            top: surface_height.saturating_sub(metrics.divider_height) / 2,
            width: metrics.divider_width,
            height: metrics.divider_height,
        };
        if self.launcher_button_visible {
            cursor = divider
                .left
                .saturating_add(metrics.divider_width)
                .saturating_add(metrics.spacing);
        }
        let items = self
            .items
            .iter()
            .map(|item| {
                let hit_bounds = PixelRect {
                    left: cursor,
                    top: icon_top,
                    width: slot_width,
                    height: metrics.icon_size,
                };
                let laid_out = LaidOutItem {
                    source_index: item.source_index,
                    icon: item.icon.clone(),
                    badge: item.badge,
                    running: item.running,
                    exiting: item.exiting,
                    bounds: PixelRect::square(
                        cursor.saturating_add(metrics.spacing),
                        icon_top,
                        metrics.icon_size,
                    ),
                    hit_bounds,
                };
                cursor = cursor.saturating_add(slot_width);
                laid_out
            })
            .collect();
        let (media_divider, media) =
            self.layout_media(&mut cursor, content_top, surface_height, desired, &metrics);
        let (status_divider, status_items) = self.layout_status_items(
            &mut cursor,
            content_top,
            surface_height,
            desired,
            &metrics,
        );
        let show_desktop = self.show_desktop_button.then_some(PixelRect {
            left: cursor,
            top: content_top,
            width: metrics.show_desktop_width,
            height: desired.height(),
        });

        DockLayout {
            items,
            launcher_button_visible: self.launcher_button_visible,
            divider,
            media_divider,
            media,
            status_divider,
            jirachi,
            jirachi_hit_bounds,
            status_items,
            show_desktop,
            icon_size: nonzero_or_one(metrics.icon_size),
        }
    }

    fn layout_media(
        &self,
        cursor: &mut u32,
        content_top: u32,
        surface_height: u32,
        desired: DockSize,
        metrics: &ScaledMetrics,
    ) -> (Option<PixelRect>, Option<LaidOutMedia<Asset>>) {
        let Some(media) = &self.media else {
            return (None, None);
        };

        let divider = self.media_separator_visible().then(|| {
            let divider = segment_divider(*cursor, surface_height, metrics);
            *cursor = divider
                .left
                .saturating_add(metrics.divider_width)
                .saturating_add(metrics.spacing);
            divider
        });
        let widget = MediaWidgetLayout::new(
            self.dpi(),
            self.desired_height_dips(),
            media.show_metadata,
        )
        .expect("the scene has a nonzero DPI");
        let translated = |bounds: lotus_ui::geometry::PhysicalRect| PixelRect {
            left: cursor.saturating_add(bounds.origin.x),
            top: content_top.saturating_add(bounds.origin.y),
            width: bounds.size.width,
            height: bounds.size.height,
        };
        let play_pause = if media.playback == PlaybackState::Playing {
            (&media.symbols.pause, media.controls.pause)
        } else {
            (&media.symbols.play, media.controls.play)
        };
        let laid_out = LaidOutMedia {
            source_id: media.source_id.clone(),
            artwork: LaidOutStatusIcon {
                icon: media.artwork.clone(),
                bounds: translated(widget.artwork),
            },
            metadata: translated(widget.metadata),
            title: media.title.clone(),
            artist: media.artist.clone(),
            controls: vec![
                LaidOutMediaControl {
                    target: lotus_media::MediaHitTarget::Previous,
                    icon: DockIcon::Embedded(media.symbols.previous.clone()),
                    bounds: translated(widget.previous),
                    enabled: media.controls.previous,
                },
                LaidOutMediaControl {
                    target: lotus_media::MediaHitTarget::PlayPause,
                    icon: DockIcon::Embedded(play_pause.0.clone()),
                    bounds: translated(widget.play_pause),
                    enabled: play_pause.1,
                },
                LaidOutMediaControl {
                    target: lotus_media::MediaHitTarget::Next,
                    icon: DockIcon::Embedded(media.symbols.next.clone()),
                    bounds: translated(widget.next),
                    enabled: media.controls.next,
                },
            ],
        };
        *cursor = cursor.saturating_add(widget.width);
        debug_assert_eq!(desired.height(), widget.height);
        (divider, Some(laid_out))
    }

    fn layout_status_items(
        &self,
        cursor: &mut u32,
        content_top: u32,
        surface_height: u32,
        desired: DockSize,
        metrics: &ScaledMetrics,
    ) -> (Option<PixelRect>, Vec<LaidOutStatusItem<Asset>>) {
        let divider = self.status_separator_visible().then(|| {
            let divider = segment_divider(*cursor, surface_height, metrics);
            *cursor = cursor
                .saturating_add(metrics.spacing)
                .saturating_add(metrics.divider_width)
                .saturating_add(metrics.spacing);
            divider
        });
        let items = self
            .status_items
            .iter()
            .map(|item| layout_status_item(item, cursor, content_top, desired, metrics))
            .collect();

        (divider, items)
    }

    fn segment_chrome_width(&self, visible: bool) -> u32 {
        let metrics = self.scaled_metrics();
        if visible {
            metrics
                .spacing
                .saturating_add(metrics.divider_width)
                .saturating_add(metrics.spacing)
        } else {
            0
        }
    }

    fn status_separator_visible(&self) -> bool {
        (self.app_segment_visible() || self.media.is_some())
            && !self.status_items.is_empty()
    }

    fn media_separator_visible(&self) -> bool {
        self.app_segment_visible() && self.media.is_some()
    }

    fn app_segment_visible(&self) -> bool {
        self.launcher_button_visible || !self.items.is_empty()
    }

    fn desired_height_dips(&self) -> u32 {
        self.metrics
            .icon_size
            .get()
            .saturating_add(self.metrics.vertical_padding.saturating_mul(2))
    }

    fn scaled_metrics(&self) -> ScaledMetrics {
        ScaledMetrics {
            icon_size: scale_dips(self.metrics.icon_size.get(), self.dpi),
            spacing: scale_dips(self.metrics.item_spacing, self.dpi),
            horizontal_padding: scale_dips(self.metrics.horizontal_padding, self.dpi),
            vertical_padding: scale_dips(self.metrics.vertical_padding, self.dpi),
            divider_width: scale_dips(DIVIDER_WIDTH_DIP, self.dpi),
            divider_height: scale_dips(DIVIDER_HEIGHT_DIP, self.dpi),
            show_desktop_width: scale_dips(SHOW_DESKTOP_WIDTH_DIP, self.dpi),
            status_icon_size: scale_dips(STATUS_ICON_SIZE_DIP, self.dpi),
            status_icon_slot_width: scale_dips(STATUS_ICON_SLOT_WIDTH_DIP, self.dpi),
            status_clock_width: scale_dips(STATUS_CLOCK_WIDTH_DIP, self.dpi),
        }
    }
}

fn layout_status_item<Asset: Clone>(
    item: &SystemStatusItem<Asset>,
    cursor: &mut u32,
    content_top: u32,
    desired: DockSize,
    metrics: &ScaledMetrics,
) -> LaidOutStatusItem<Asset> {
    let width = match item.kind {
        SystemStatusKind::DateTime => metrics.status_clock_width,
        SystemStatusKind::Volume
        | SystemStatusKind::AdvancedColor
        | SystemStatusKind::Network
        | SystemStatusKind::BackgroundApps => metrics.status_icon_slot_width,
    };
    let hit_bounds = PixelRect {
        left: *cursor,
        top: content_top,
        width,
        height: desired.height(),
    };
    let icon = item.icon.as_ref().map(|icon| LaidOutStatusIcon {
        icon: icon.clone(),
        bounds: PixelRect::square(
            cursor.saturating_add(width.saturating_sub(metrics.status_icon_size) / 2),
            content_top.saturating_add(
                desired.height().saturating_sub(metrics.status_icon_size) / 2,
            ),
            metrics.status_icon_size,
        ),
    });
    let laid_out = LaidOutStatusItem {
        kind: item.kind,
        hit_bounds,
        icon,
        primary_text: item.primary_text.clone(),
        secondary_text: item.secondary_text.clone(),
    };
    *cursor = cursor.saturating_add(width);
    laid_out
}

struct ScaledMetrics {
    icon_size: u32,
    spacing: u32,
    horizontal_padding: u32,
    vertical_padding: u32,
    divider_width: u32,
    divider_height: u32,
    show_desktop_width: u32,
    status_icon_size: u32,
    status_icon_slot_width: u32,
    status_clock_width: u32,
}

fn segment_divider(cursor: u32, surface_height: u32, metrics: &ScaledMetrics) -> PixelRect {
    PixelRect {
        left: cursor.saturating_add(metrics.spacing),
        top: surface_height.saturating_sub(metrics.divider_height) / 2,
        width: metrics.divider_width,
        height: metrics.divider_height,
    }
}
