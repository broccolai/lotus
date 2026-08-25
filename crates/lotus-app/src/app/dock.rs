mod interaction;
mod pinning;
mod projection;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lotus_core::application::{
    ApplicationKey, PinnedApplicationAssignment, WindowApplicationAssignments,
};
use lotus_core::dock::DockItem;
use lotus_core::notification::NotificationSource;
use lotus_core::settings::{DockSettings, SettingsStore};
use lotus_core::window::{TrackedWindowKey, WindowInfo};
use lotus_dock::interaction::DockInteraction;
use lotus_dock::model::{DockModel, SettingsImpact};
use lotus_dock::scene::DockPresenter;
use lotus_media::MediaSnapshot;
use lotus_settings::appearance::theme_for;
use lotus_windows::custom_image::{
    CustomImageCache, MascotAnimation, MascotLoopCount, load_mascot_image,
};
use lotus_windows::graphics::assets::SvgAsset;
use lotus_windows::icon_hydrator::{DockIconClient, DockIconRequest, HydratedDockIcon};
use lotus_windows::media::decode_artwork;
use lotus_windows::native_icon::NativeIconCache;
use lotus_windows::search_catalog::{
    ApplicationAssociations, ApplicationCatalogSnapshot, ApplicationResolver,
};
use projection::{
    departure_transition, docked_status_items, media_source_matches_item, projected_items,
};
pub(super) use projection::{
    dock_anchor, metrics, popup_overlap, status_items, status_popup_center,
};

use crate::app::AppError;
use crate::app::visuals::{
    DockIcon, DockItem as SceneDockItem, DockMetrics, DockScene, MediaItem, MediaSymbols,
};

const NATIVE_ICON_SAMPLE_SCALE: u32 = 2;
const EXIT_DURATION: Duration = Duration::from_millis(80);

pub(super) struct DockRuntime {
    model: DockModel,
    scene: DockScene,
    native_icons: NativeIconCache,
    icon_hydrator: Option<DockIconClient>,
    hydrated_window_icons: HashMap<String, HydratedDockIcon>,
    custom_images: CustomImageCache,
    notifications: Vec<NotificationSource>,
    interaction: DockInteraction,
    pending_items: Option<Vec<SceneDockItem>>,
    exit_deadline: Option<Instant>,
    media: Option<MediaItem>,
    recent_windows: HashMap<String, Vec<TrackedWindowKey>>,
    transient_unpinned: HashMap<ApplicationKey, (usize, DockItem)>,
    revision: u64,
    presenter: DockPresenter,
    mascot_animation: Option<MascotPlayback>,
    application_resolver: ApplicationResolver,
    application_catalog: Arc<ApplicationCatalogSnapshot>,
    application_assignments: WindowApplicationAssignments,
    adopted_catalog_generation: u64,
}

struct MascotPlayback {
    animation: MascotAnimation,
    frame_index: usize,
    completed_loops: u32,
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
        let application_catalog = Arc::new(ApplicationCatalogSnapshot::new(0, Vec::new()));
        let mut application_resolver = ApplicationResolver::default();
        let associations =
            ApplicationAssociations::from_pins(&settings.pinned_apps, &application_catalog);
        let application_assignments = application_resolver.resolve_all(
            windows,
            &application_catalog,
            &associations,
            0,
        );
        let pinned_applications =
            pinned_application_assignments(&settings, &application_catalog);
        let items = projected_items(
            &settings,
            windows,
            &application_assignments,
            &application_catalog.applications,
            &pinned_applications,
        );
        let mut scene = Self::configured_scene(dpi, &settings, metrics)
            .ok_or(AppError::InvalidScene)?;
        scene.replace_status_items(docked_status_items(&settings));
        let mut runtime = Self {
            model: DockModel::new(settings, settings_store, items),
            scene,
            native_icons: NativeIconCache::default(),
            icon_hydrator: None,
            hydrated_window_icons: HashMap::new(),
            custom_images: CustomImageCache::default(),
            notifications: Vec::new(),
            interaction: DockInteraction::new(drag_threshold),
            pending_items: None,
            exit_deadline: None,
            media: None,
            recent_windows: HashMap::new(),
            transient_unpinned: HashMap::new(),
            revision: 0,
            presenter: DockPresenter::default(),
            mascot_animation: None,
            application_resolver,
            application_catalog,
            application_assignments,
            adopted_catalog_generation: 0,
        };
        runtime.reset_mascot_animation();
        runtime.refresh_scene_items();
        Ok(runtime)
    }

    fn configured_scene(
        dpi: u32,
        settings: &DockSettings,
        metrics: DockMetrics,
    ) -> Option<DockScene> {
        let mut scene = DockScene::new(
            dpi,
            metrics,
            DockIcon::Embedded(SvgAsset::LotusPixel),
            Vec::new(),
        )?;
        scene.set_anchor(dock_anchor(settings.dock_zone));
        scene.set_launcher_button_visible(settings.show_app_dock);
        let _ = scene.set_theme(theme_for(settings));
        scene.set_show_desktop_button(settings.show_desktop_button);
        Some(scene)
    }

    pub(super) fn rebuild(
        &mut self,
        windows: &[WindowInfo],
        catalog: Arc<ApplicationCatalogSnapshot>,
    ) {
        let settings = self.model.settings().clone();
        self.resolve_application_assignments(&settings, windows, &catalog);
        self.application_catalog = catalog;
        let previous_model = self.model.items().to_vec();
        let previous_scene = self.scene.items().to_vec();
        let mut items = self.projected_items(windows);
        self.merge_transient_unpinned(&mut items, windows);
        self.model.rebuild(items);
        self.retain_current_window_icons();
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
        self.request_native_window_icons();
    }

    pub(in crate::app) fn adopt_catalogue_pins(
        &mut self,
        catalog: &ApplicationCatalogSnapshot,
    ) -> Result<(), AppError> {
        if catalog.generation == 0 || self.adopted_catalog_generation == catalog.generation
        {
            return Ok(());
        }
        let assignments = pinned_application_assignments(self.model.settings(), catalog);
        let safe_aliases = assignments
            .iter()
            .zip(&self.model.settings().pinned_apps)
            .map(|(assignment, pin)| {
                let mut aliases = assignment
                    .registered_index
                    .and_then(|index| catalog.application(index))
                    .map(|application| catalog.safe_executable_aliases(application))
                    .unwrap_or_default();
                aliases.extend(
                    pin.match_executables
                        .iter()
                        .filter(|alias| catalog.is_safe_executable_alias(alias))
                        .cloned(),
                );
                aliases.sort();
                aliases.dedup();
                aliases
            })
            .collect::<Vec<_>>();
        let _ = self.model.repair_catalogue_pins(
            &assignments,
            &catalog.applications,
            &safe_aliases,
        )?;
        self.adopted_catalog_generation = catalog.generation;
        Ok(())
    }

    pub(in crate::app) fn registered_application_for_item(
        &self,
        item: &DockItem,
    ) -> Option<lotus_core::application::RegisteredApplication> {
        self.application_catalog
            .application_index_for_key(&item.application_key)
            .and_then(|index| self.application_catalog.application(index))
            .cloned()
    }

    fn projected_items(&self, windows: &[WindowInfo]) -> Vec<DockItem> {
        self.projected_items_for(self.model.settings(), windows)
    }

    fn resolve_application_assignments(
        &mut self,
        settings: &DockSettings,
        windows: &[WindowInfo],
        catalog: &ApplicationCatalogSnapshot,
    ) {
        let associations =
            ApplicationAssociations::from_pins(&settings.pinned_apps, catalog);
        self.application_assignments = self.application_resolver.resolve_all(
            windows,
            catalog,
            &associations,
            self.revision.saturating_add(1),
        );
    }

    pub(in crate::app) fn resolve_current_applications(&mut self, windows: &[WindowInfo]) {
        let settings = self.model.settings().clone();
        let catalog = Arc::clone(&self.application_catalog);
        self.resolve_application_assignments(&settings, windows, &catalog);
    }

    pub(in crate::app) fn application_assignments(&self) -> &WindowApplicationAssignments {
        &self.application_assignments
    }

    fn projected_items_for(
        &self,
        settings: &DockSettings,
        windows: &[WindowInfo],
    ) -> Vec<DockItem> {
        let pinned_applications =
            pinned_application_assignments(settings, &self.application_catalog);
        let started = Instant::now();
        let items = projected_items(
            settings,
            windows,
            &self.application_assignments,
            &self.application_catalog.applications,
            &pinned_applications,
        );
        lotus_windows::responsiveness::METRICS.record_dock_projection(started.elapsed());
        items
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
        self.retain_current_window_icons();
        let scene_items = self.scene_items();
        self.scene.replace_items(scene_items);
        self.pending_items = None;
        self.exit_deadline = None;
        self.mark_changed();
        self.request_native_window_icons();
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

    pub(super) fn presentation(
        &mut self,
    ) -> (lotus_ui::presentation::Presentation<SvgAsset>, bool) {
        let size = self.scene.desired_size();
        let departure_pending = self.exit_deadline.is_some();
        let (presentation, needs_animation) =
            self.presenter
                .present(&self.scene, size.width(), size.height());
        (presentation, needs_animation || departure_pending)
    }

    pub(super) fn apply_settings(
        &mut self,
        next: DockSettings,
        windows: &[WindowInfo],
    ) -> Result<SettingsImpact, AppError> {
        let next = next.normalized();
        let metrics = metrics(&next)?;
        let dpi = self.scene.dpi();
        let retained_items = self.model.items().to_vec();
        let impact = self.model.apply_settings(next, retained_items)?;
        if impact.changed {
            self.resolve_current_applications(windows);
            let next_items = self.projected_items(windows);
            self.model.rebuild(next_items);
            self.custom_images.clear();
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
            self.reset_mascot_animation();
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

    pub(super) fn mascot_animation_delay(&self) -> Option<Duration> {
        self.mascot_animation
            .as_ref()
            .map(|playback| playback.animation.frames[playback.frame_index].delay)
    }

    pub(super) fn advance_mascot_animation(&mut self) -> bool {
        let Some(playback) = &mut self.mascot_animation else {
            return false;
        };
        let Some(frame_index) = advance_mascot_playback(playback) else {
            self.mascot_animation = None;
            return false;
        };
        self.scene.set_mascot(DockIcon::Raster(
            playback.animation.frames[frame_index].icon.clone(),
        ));
        self.mark_changed();
        true
    }

    fn reset_mascot_animation(&mut self) {
        let mascot = self
            .model
            .settings()
            .mascot_image_path
            .as_deref()
            .and_then(|path| load_mascot_image(Path::new(path)).ok());
        if let Some(mascot) = mascot {
            self.scene.set_mascot(DockIcon::Raster(mascot.icon));
            self.mascot_animation = mascot.animation.map(|animation| MascotPlayback {
                animation,
                frame_index: 0,
                completed_loops: 0,
            });
        } else {
            self.scene
                .set_mascot(DockIcon::Embedded(SvgAsset::LotusPixel));
            self.mascot_animation = None;
        }
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
            media_source_matches_item(source_id, item, &self.application_catalog)
        })?;
        let size = self
            .scene
            .icon_size_pixels()
            .saturating_mul(NATIVE_ICON_SAMPLE_SCALE);
        crate::app::icon_override::resolve_application_icon(
            self.model.settings(),
            &mut self.custom_images,
            item.app_user_model_id.as_deref(),
            Some(&item.id),
            Path::new(&item.executable_path),
        )
        .map(DockIcon::Raster)
        .or_else(|| {
            self.native_icons
                .icon(Path::new(&item.icon_source), size)
                .ok()
                .flatten()
                .map(DockIcon::Raster)
        })
    }

    pub(in crate::app) fn drain_hydrated_window_icons(
        &mut self,
        results: impl IntoIterator<Item = HydratedDockIcon>,
    ) -> bool {
        let icon_size = self
            .scene
            .icon_size_pixels()
            .saturating_mul(NATIVE_ICON_SAMPLE_SCALE);
        let mut changed = false;

        for result in results {
            let current = self.model.items().iter().any(|item| {
                item.id == result.identity
                    && item.presentation_icon.native_window() == Some(result.window)
                    && result.pixel_size == icon_size
            });
            if current && result.icon.is_some() {
                self.hydrated_window_icons
                    .insert(result.identity.clone(), result);
                changed = true;
            }
        }
        if changed {
            self.refresh_scene_items();
        }
        changed
    }

    fn request_native_window_icons(&self) {
        let pixel_size = self
            .scene
            .icon_size_pixels()
            .saturating_mul(NATIVE_ICON_SAMPLE_SCALE);
        let requests = self
            .model
            .items()
            .iter()
            .filter_map(|item| {
                let window = item.presentation_icon.native_window()?;
                if self
                    .hydrated_window_icons
                    .get(&item.id)
                    .is_some_and(|icon| {
                        icon.window == window && icon.pixel_size == pixel_size
                    })
                {
                    return None;
                }
                let identity = item.application_identity();
                crate::app::icon_override::application_icon_path_for_identity(
                    self.model.settings(),
                    &identity,
                )
                .is_none()
                .then(|| DockIconRequest {
                    identity: item.id.clone(),
                    window,
                    fallback_path: item.icon_source.clone().into(),
                    pixel_size,
                })
            })
            .collect();
        if let Some(icon_hydrator) = &self.icon_hydrator {
            icon_hydrator.request_dock(requests);
        }
    }

    fn retain_current_window_icons(&mut self) {
        let pixel_size = self
            .scene
            .icon_size_pixels()
            .saturating_mul(NATIVE_ICON_SAMPLE_SCALE);
        let current = self
            .model
            .items()
            .iter()
            .filter_map(|item| {
                item.presentation_icon
                    .native_window()
                    .map(|window| (item.id.clone(), window))
            })
            .collect::<HashMap<_, _>>();
        self.hydrated_window_icons.retain(|identity, icon| {
            icon.pixel_size == pixel_size
                && current
                    .get(identity)
                    .is_some_and(|window| *window == icon.window)
        });
    }

    pub(in crate::app) fn attach_icon_hydrator(&mut self, icon_hydrator: DockIconClient) {
        self.icon_hydrator = Some(icon_hydrator);
        self.request_native_window_icons();
    }
}

fn pinned_application_assignments(
    settings: &DockSettings,
    catalog: &ApplicationCatalogSnapshot,
) -> Vec<PinnedApplicationAssignment> {
    settings
        .pinned_apps
        .iter()
        .map(|pin| {
            lotus_core::application::LaunchSpec::new(
                &pin.launch_target,
                pin.arguments.as_deref(),
            )
            .and_then(|launch| {
                catalog
                    .key_for_pin(
                        &pin.id,
                        pin.app_user_model_id.as_deref(),
                        &launch,
                        &pin.match_executables,
                    )
                    .map(|key| PinnedApplicationAssignment {
                        registered_index: catalog.application_index_for_key(&key),
                        key,
                    })
            })
            .unwrap_or_else(|| PinnedApplicationAssignment {
                key: ApplicationKey::from_launch_fallback(
                    &lotus_core::application::LaunchSpec::new(
                        &pin.launch_target,
                        pin.arguments.as_deref(),
                    )
                    .unwrap_or_else(|| {
                        lotus_core::application::LaunchSpec {
                            target: pin.launch_target.clone(),
                            arguments: pin.arguments.clone(),
                        }
                    }),
                ),
                registered_index: None,
            })
        })
        .collect()
}

fn advance_mascot_playback(playback: &mut MascotPlayback) -> Option<usize> {
    let next = playback.frame_index + 1;
    if next < playback.animation.frames.len() {
        playback.frame_index = next;
        return Some(next);
    }
    if matches!(playback.animation.loop_count, MascotLoopCount::Finite(count) if playback.completed_loops + 1 >= count)
    {
        return None;
    }
    playback.frame_index = 0;
    playback.completed_loops = playback.completed_loops.saturating_add(1);
    Some(0)
}
