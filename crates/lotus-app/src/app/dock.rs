mod assets;
mod interaction;
mod mascot;
mod pinning;
mod projection;
mod status_observation;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use assets::DockAssets;
use lotus_core::application::{
    ApplicationKey, PinnedApplicationAssignment, WindowApplicationAssignments,
};
use lotus_core::dock::DockItem;
use lotus_core::notification::NotificationSource;
use lotus_core::settings::DockSettings;
use lotus_core::window::{TrackedWindowKey, WindowInfo};
use lotus_dock::interaction::DockInteraction;
use lotus_dock::model::{DockModel, DockReorderRequest, SettingsImpact};
use lotus_dock::scene::DockPresenter;
use lotus_media::MediaSnapshot;
use lotus_settings::appearance::theme_for;
use lotus_ui::embedded_icon::EmbeddedIcon;
use lotus_windows::WindowHandle;
use lotus_windows::icon_hydrator::{DockIconClient, HydratedDockIcon};
use lotus_windows::search_catalog::{
    ApplicationAssociations, ApplicationCatalogSnapshot, ApplicationResolver,
};
use mascot::Mascot;
use projection::{departure_transition, projected_items};
pub(super) use projection::{dock_anchor, metrics, popup_overlap, status_popup_center};
pub(super) use status_observation::{docked_status_items, status_items};

use crate::app::AppError;
use crate::app::monitors::{
    MonitorPresentationInput, MonitorReplicaInput, MonitorReplicaTarget,
};
use crate::app::settings_persistence::SettingsPersistence;
use crate::app::visuals::{
    DockIcon, DockItem as SceneDockItem, DockMetrics, DockScene, MediaItem,
};

const NATIVE_ICON_SAMPLE_SCALE: u32 = 2;
const EXIT_DURATION: Duration = Duration::from_millis(80);

pub(super) use lotus_dock::interaction::{DockInteractionIntent, DockInteractionOutcome};

pub(super) struct DockRuntime {
    status_owner: WindowHandle,
    model: DockModel,
    scene: DockScene,
    assets: DockAssets,
    notifications: Vec<NotificationSource>,
    interaction: DockInteraction,
    pending_items: Option<Vec<SceneDockItem>>,
    exit_deadline: Option<Instant>,
    media: Option<MediaItem>,
    recent_windows: HashMap<String, Vec<TrackedWindowKey>>,
    transient_unpinned: HashMap<ApplicationKey, (usize, DockItem)>,
    revision: u64,
    presenter: DockPresenter,
    mascot: Mascot,
    application_resolver: ApplicationResolver,
    application_catalog: Arc<ApplicationCatalogSnapshot>,
    application_assignments: WindowApplicationAssignments,
    adopted_catalog_generation: u64,
}

impl DockRuntime {
    pub(super) fn new(
        status_owner: WindowHandle,
        settings: DockSettings,
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
        scene.replace_status_items(docked_status_items(&settings, status_owner));
        let mut runtime = Self {
            status_owner,
            model: DockModel::new(settings, items),
            scene,
            assets: DockAssets::default(),
            notifications: Vec::new(),
            interaction: DockInteraction::new(drag_threshold),
            pending_items: None,
            exit_deadline: None,
            media: None,
            recent_windows: HashMap::new(),
            transient_unpinned: HashMap::new(),
            revision: 0,
            presenter: DockPresenter::default(),
            mascot: Mascot::default(),
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
            DockIcon::Embedded(EmbeddedIcon::LotusPixel),
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
        persistence: &SettingsPersistence,
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
        if let Some(settings) = self.model.prepared_catalogue_pin_repair(
            &assignments,
            &catalog.applications,
            &safe_aliases,
        ) {
            persistence.save(&settings)?;
            self.model.commit_settings_only(settings);
        }
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

    pub(in crate::app) fn scene_items(&mut self) -> Vec<SceneDockItem> {
        let pixel_size = self
            .scene
            .icon_size_pixels()
            .saturating_mul(NATIVE_ICON_SAMPLE_SCALE);
        self.scene_items_at_size(pixel_size)
    }

    pub(in crate::app) fn scene_items_at_size(
        &mut self,
        pixel_size: u32,
    ) -> Vec<SceneDockItem> {
        if !self.model.settings().show_app_dock {
            return Vec::new();
        }
        let icons = self.assets.prepare_icons(
            self.model.items(),
            self.model.settings(),
            pixel_size,
        );
        projection::scene_items(
            self.model.items(),
            &icons,
            self.model.settings(),
            &self.notifications,
            &self.application_catalog,
        )
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

    pub(super) fn prepare_monitor_presentation(
        &mut self,
        targets: Vec<MonitorReplicaTarget>,
    ) -> Result<MonitorPresentationInput, AppError> {
        let settings = self.model.settings().clone();
        let replicas = targets
            .into_iter()
            .map(|target| self.prepare_monitor_replica(target, &settings))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(MonitorPresentationInput {
            settings,
            revision: self.revision,
            replicas,
        })
    }

    fn prepare_monitor_replica(
        &mut self,
        target: MonitorReplicaTarget,
        settings: &DockSettings,
    ) -> Result<MonitorReplicaInput, AppError> {
        let mut scene = Self::configured_scene(target.dpi, settings, metrics(settings)?)
            .ok_or(AppError::InvalidScene)?;
        scene.replace_status_items(docked_status_items(settings, target.owner));
        if settings.show_media_controls && settings.media_zone == settings.dock_zone {
            scene.replace_media(self.media.clone());
        }
        let icon_size = scene
            .icon_size_pixels()
            .saturating_mul(NATIVE_ICON_SAMPLE_SCALE);
        scene.replace_items(self.scene_items_at_size(icon_size));
        Ok(MonitorReplicaInput {
            owner: target.owner,
            scene,
        })
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
    pub(super) fn items(&self) -> &[DockItem] {
        self.model.items()
    }

    pub(in crate::app) fn persist_reorder(
        &mut self,
        request: &DockReorderRequest,
        persistence: &SettingsPersistence,
    ) -> Result<bool, AppError> {
        let Some(reorder) = self.model.prepare_reorder(request) else {
            self.refresh_scene_items();
            return Ok(false);
        };
        persistence.save(reorder.settings())?;
        self.model.commit_reorder(reorder);
        self.refresh_scene_items();
        Ok(true)
    }
    pub(super) const fn scene(&self) -> &DockScene {
        &self.scene
    }

    pub(super) fn presentation(
        &mut self,
    ) -> (lotus_ui::presentation::Presentation<EmbeddedIcon>, bool) {
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
        persistence: &SettingsPersistence,
    ) -> Result<SettingsImpact, AppError> {
        let next = next.normalized();
        let metrics = metrics(&next)?;
        let dpi = self.scene.dpi();
        let retained_items = self.model.items().to_vec();
        let Some(change) = self.model.prepare_settings(next, retained_items) else {
            return Ok(SettingsImpact {
                changed: false,
                restart_required: false,
            });
        };
        persistence.save(change.settings())?;
        let impact = self.model.commit_settings(change);
        if impact.changed {
            self.resolve_current_applications(windows);
            let next_items = self.projected_items(windows);
            self.model.rebuild(next_items);
            self.assets.clear_custom_images();
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
            scene.replace_status_items(docked_status_items(
                self.model.settings(),
                self.status_owner,
            ));
            self.scene = scene;
            self.reset_mascot_animation();
            self.refresh_scene_items();
        }
        Ok(impact)
    }

    pub(super) fn refresh_status(&mut self) -> bool {
        let next = docked_status_items(self.model.settings(), self.status_owner);
        if self.scene.status_items() == next {
            return false;
        }
        self.scene.replace_status_items(next);
        self.mark_changed();
        true
    }

    pub(super) fn advanced_color_changed(&mut self) {
        let _ = self.refresh_status();
        self.mark_changed();
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
        self.mascot.delay()
    }

    pub(super) fn advance_mascot_animation(&mut self) -> bool {
        let Some(icon) = self.mascot.next_frame() else {
            return false;
        };
        self.scene.set_mascot(DockIcon::Raster(icon));
        self.mark_changed();
        true
    }

    fn reset_mascot_animation(&mut self) {
        self.mascot = Mascot::load(self.model.settings().mascot_image_path.as_deref());
        self.scene.set_mascot(self.mascot.initial_icon());
    }

    fn media_item(&mut self, snapshot: &MediaSnapshot) -> MediaItem {
        let artwork = self
            .assets
            .media_artwork(
                snapshot,
                self.model.items(),
                self.model.settings(),
                &self.application_catalog,
                self.scene
                    .icon_size_pixels()
                    .saturating_mul(NATIVE_ICON_SAMPLE_SCALE),
            )
            .unwrap_or(DockIcon::Embedded(EmbeddedIcon::FluentMusic));
        projection::media_item(snapshot, artwork, self.model.settings().show_media_metadata)
    }

    pub(in crate::app) fn drain_hydrated_window_icons(
        &mut self,
        results: impl IntoIterator<Item = HydratedDockIcon>,
    ) -> bool {
        let icon_size = self
            .scene
            .icon_size_pixels()
            .saturating_mul(NATIVE_ICON_SAMPLE_SCALE);
        let changed = self.assets.drain(self.model.items(), icon_size, results);
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
        self.assets
            .request(self.model.items(), self.model.settings(), pixel_size);
    }

    fn retain_current_window_icons(&mut self) {
        let pixel_size = self
            .scene
            .icon_size_pixels()
            .saturating_mul(NATIVE_ICON_SAMPLE_SCALE);
        self.assets.retain(self.model.items(), pixel_size);
    }

    pub(in crate::app) fn attach_icon_hydrator(&mut self, icon_hydrator: DockIconClient) {
        self.assets.attach(icon_hydrator);
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
