mod interaction;
mod pinning;
mod projection;

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use lotus_core::dock::DockItem;
use lotus_core::notification::NotificationSource;
use lotus_core::settings::{DockSettings, SettingsStore};
use lotus_core::window::{WindowId, WindowInfo};
use lotus_dock::interaction::DockInteraction;
use lotus_dock::model::{DockModel, SettingsImpact};
use lotus_media::MediaSnapshot;
use lotus_settings::appearance::theme_for;
use lotus_windows::graphics::assets::SvgAsset;
use lotus_windows::graphics::scene::{
    DockIcon, DockItem as SceneDockItem, DockMetrics, DockScene, MediaItem, MediaSymbols,
};
use lotus_windows::media::decode_artwork;
use lotus_windows::native_icon::NativeIconCache;
use projection::{departure_transition, docked_status_items, mascot, projected_items};
pub(super) use projection::{
    dock_anchor, metrics, popup_overlap, status_items, status_popup_center,
};

use crate::app::AppError;

const NATIVE_ICON_SAMPLE_SCALE: u32 = 2;
const EXIT_DURATION: Duration = Duration::from_millis(80);

pub(super) struct DockRuntime {
    model: DockModel,
    scene: DockScene,
    native_icons: NativeIconCache,
    notifications: Vec<NotificationSource>,
    interaction: DockInteraction,
    pending_items: Option<Vec<SceneDockItem>>,
    exit_deadline: Option<Instant>,
    media: Option<MediaItem>,
    recent_windows: HashMap<String, Vec<WindowId>>,
    transient_unpinned: HashMap<String, (usize, DockItem)>,
    revision: u64,
}

impl DockRuntime {
    pub(super) fn new(
        settings: DockSettings,
        settings_store: SettingsStore,
        windows: &[WindowInfo],
        dpi: u32,
        drag_threshold: (u32, u32),
    ) -> Result<Self, AppError> {
        let metrics = metrics(&settings)?;
        let items = projected_items(&settings, windows);
        let mut scene = Self::configured_scene(dpi, &settings, metrics)
            .ok_or(AppError::InvalidScene)?;
        scene.replace_status_items(docked_status_items(&settings));
        let mut runtime = Self {
            model: DockModel::new(settings, settings_store, items),
            scene,
            native_icons: NativeIconCache::default(),
            notifications: Vec::new(),
            interaction: DockInteraction::new(drag_threshold),
            pending_items: None,
            exit_deadline: None,
            media: None,
            recent_windows: HashMap::new(),
            transient_unpinned: HashMap::new(),
            revision: 0,
        };
        runtime.refresh_scene_items();
        Ok(runtime)
    }

    fn configured_scene(
        dpi: u32,
        settings: &DockSettings,
        metrics: DockMetrics,
    ) -> Option<DockScene> {
        let mut scene = DockScene::new(dpi, metrics, mascot(settings), Vec::new())?;
        scene.set_anchor(dock_anchor(settings.dock_zone));
        scene.set_launcher_button_visible(settings.show_app_dock);
        let _ = scene.set_theme(theme_for(settings));
        scene.set_show_desktop_button(settings.show_desktop_button);
        Some(scene)
    }

    pub(super) fn rebuild(&mut self, windows: &[WindowInfo]) {
        let previous_model = self.model.items().to_vec();
        let previous_scene = self.scene.items().to_vec();
        let mut items = projected_items(self.model.settings(), windows);
        self.merge_transient_unpinned(&mut items, windows);
        self.model.rebuild(items);
        let final_items = self.scene_items();
        if let Some(transition) = departure_transition(
            &previous_model,
            &previous_scene,
            self.model.items(),
            &final_items,
        ) {
            self.scene.replace_items(transition);
            self.pending_items = Some(final_items);
            self.exit_deadline = Some(Instant::now() + EXIT_DURATION);
        } else {
            self.scene.replace_items(final_items);
            self.pending_items = None;
            self.exit_deadline = None;
        }
        self.mark_changed();
    }

    pub(super) fn set_dpi(&mut self, dpi: u32) -> Result<(), AppError> {
        self.scene
            .set_dpi(dpi)
            .then_some(())
            .ok_or(AppError::InvalidScene)?;
        self.refresh_scene_items();
        Ok(())
    }

    pub(super) fn set_drag_threshold(&mut self, threshold: (u32, u32)) {
        self.interaction.set_drag_threshold(threshold);
    }

    pub(super) fn refresh_scene_items(&mut self) {
        let scene_items = self.scene_items();
        self.scene.replace_items(scene_items);
        self.pending_items = None;
        self.exit_deadline = None;
        self.mark_changed();
    }

    pub(super) fn advance_departure(&mut self, now: Instant) -> bool {
        if self.exit_deadline.is_none_or(|deadline| now < deadline) {
            return false;
        }
        if let Some(items) = self.pending_items.take() {
            self.scene.replace_items(items);
        }
        self.exit_deadline = None;
        self.mark_changed();
        true
    }

    pub(super) fn replica_scene(&mut self, dpi: u32) -> Result<DockScene, AppError> {
        let settings = self.model.settings().clone();
        let mut scene = Self::configured_scene(dpi, &settings, metrics(&settings)?)
            .ok_or(AppError::InvalidScene)?;
        scene.replace_status_items(docked_status_items(&settings));
        if settings.show_media_controls && settings.media_zone == settings.dock_zone {
            scene.replace_media(self.media.clone());
        }
        let icon_size = scene
            .icon_size_pixels()
            .saturating_mul(NATIVE_ICON_SAMPLE_SCALE);
        scene.replace_items(self.scene_items_at_size(icon_size));
        Ok(scene)
    }

    pub(super) const fn revision(&self) -> u64 {
        self.revision
    }
    fn mark_changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
    pub(super) fn set_notifications(&mut self, notifications: Vec<NotificationSource>) {
        self.notifications = notifications;
        self.refresh_scene_items();
    }
    pub(super) const fn settings(&self) -> &DockSettings {
        self.model.settings()
    }
    pub(super) fn settings_directory(&self) -> &Path {
        self.model.settings_directory()
    }
    pub(super) fn items(&self) -> &[DockItem] {
        self.model.items()
    }
    pub(super) const fn scene(&self) -> &DockScene {
        &self.scene
    }

    pub(super) fn apply_settings(
        &mut self,
        next: DockSettings,
        windows: &[WindowInfo],
    ) -> Result<SettingsImpact, AppError> {
        let next = next.normalized();
        let next_items = projected_items(&next, windows);
        let metrics = metrics(&next)?;
        let dpi = self.scene.dpi();
        let impact = self.model.apply_settings(next, next_items)?;
        if impact.changed {
            if let Some(media) = &mut self.media {
                media.show_metadata = self.model.settings().show_media_metadata;
            }
            let mut scene = Self::configured_scene(dpi, self.model.settings(), metrics)
                .ok_or(AppError::InvalidScene)?;
            if self.model.settings().show_media_controls
                && self.model.settings().media_zone == self.model.settings().dock_zone
            {
                scene.replace_media(self.media.clone());
            }
            scene.replace_status_items(docked_status_items(self.model.settings()));
            self.scene = scene;
            self.refresh_scene_items();
        }
        Ok(impact)
    }

    pub(super) fn refresh_status(&mut self) -> bool {
        let next = docked_status_items(self.model.settings());
        if self.scene.status_items() == next {
            return false;
        }
        self.scene.replace_status_items(next);
        self.mark_changed();
        true
    }

    pub(super) fn replace_media(&mut self, snapshot: Option<&MediaSnapshot>) -> bool {
        let media = snapshot.map(|snapshot| self.media_item(snapshot));
        if self.media == media {
            return false;
        }
        self.media = media;
        let docked = self.model.settings().show_media_controls
            && self.model.settings().media_zone == self.model.settings().dock_zone;
        self.scene
            .replace_media(docked.then(|| self.media.clone()).flatten());
        self.mark_changed();
        true
    }

    pub(super) fn media(&self) -> Option<&MediaItem> {
        self.media.as_ref()
    }

    fn media_item(&mut self, snapshot: &MediaSnapshot) -> MediaItem {
        let artwork = snapshot
            .artwork
            .as_deref()
            .and_then(|artwork| decode_artwork(&snapshot.source_id, artwork).ok())
            .map(DockIcon::Raster)
            .or_else(|| self.media_source_icon(&snapshot.source_id))
            .unwrap_or(DockIcon::Embedded(SvgAsset::FluentMusic));
        MediaItem {
            source_id: snapshot.source_id.clone(),
            title: snapshot.title.clone(),
            artist: snapshot.artist.clone(),
            show_metadata: self.model.settings().show_media_metadata,
            artwork,
            controls: snapshot.controls,
            playback: snapshot.playback,
            symbols: MediaSymbols {
                previous: SvgAsset::FluentPrevious,
                play: SvgAsset::FluentPlay,
                pause: SvgAsset::FluentPause,
                next: SvgAsset::FluentNext,
            },
        }
    }

    fn media_source_icon(&mut self, source_id: &str) -> Option<DockIcon> {
        let item = self.model.items().iter().find(|item| {
            projection::media_identity_matches(source_id, &item.executable_path)
                || projection::media_identity_matches(source_id, &item.display_name)
        })?;
        let size = self
            .scene
            .icon_size_pixels()
            .saturating_mul(NATIVE_ICON_SAMPLE_SCALE);
        self.native_icons
            .icon(Path::new(&item.icon_source), size)
            .ok()
            .flatten()
            .map(DockIcon::Raster)
    }
}
