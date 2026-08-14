use std::collections::HashMap;
use std::time::{Duration, Instant};

use lotus_core::dock::DockItem;
use lotus_core::notification::count_for_item;
use lotus_dock::interaction::{DockInteraction, map_visual_insertion_slot};
use lotus_dock::model::{DockModel, SettingsImpact, project_snapshot};
use lotus_settings::appearance::theme_for;

use super::{
    AppError, DockAnchor, DockBadge, DockContextRequest, DockHitTarget, DockIcon,
    DockMetrics, DockScene, DockSettings, DockZone, MediaItem, MediaSnapshot, MediaSymbols,
    NativeIconCache, NativePickerWindow, NotificationBadgeStyle, NotificationSource, Path,
    PopupAlignment, SettingsStore, SignedPoint, SvgAsset, SystemStatusItem,
    SystemStatusKind, WindowHandle, WindowId, WindowInfo, adapt_dock_items_with_native,
    decode_artwork, execute_activation, foreground_window, local_date, local_time_24h,
    order_picker_windows, resolve_executable, show_error,
};
use crate::graphics::scene::DockItem as SceneDockItem;

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
        let mut scene = DockScene::new(dpi, metrics, mascot(&settings), Vec::new())
            .ok_or(AppError::InvalidScene)?;
        scene.set_anchor(dock_anchor(settings.dock_zone));
        let _ = scene.set_theme(theme_for(&settings));
        scene.set_show_desktop_button(settings.show_desktop_button);
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
        };
        runtime.refresh_scene_items();
        Ok(runtime)
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
    }

    pub(super) fn advance_departure(&mut self, now: Instant) -> bool {
        if self.exit_deadline.is_none_or(|deadline| now < deadline) {
            return false;
        }

        if let Some(items) = self.pending_items.take() {
            self.scene.replace_items(items);
        }
        self.exit_deadline = None;
        true
    }

    fn scene_items(&mut self) -> Vec<SceneDockItem> {
        let icon_size = self
            .scene
            .icon_size_pixels()
            .saturating_mul(NATIVE_ICON_SAMPLE_SCALE);
        let mut scene_items =
            adapt_dock_items_with_native(self.model.items(), |_, item| {
                self.native_icons
                    .icon(Path::new(&item.icon_source), icon_size)
                    .ok()
                    .flatten()
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
            let mut scene =
                DockScene::new(dpi, metrics, mascot(self.model.settings()), Vec::new())
                    .ok_or(AppError::InvalidScene)?;
            scene.set_anchor(dock_anchor(self.model.settings().dock_zone));
            let _ = scene.set_theme(theme_for(self.model.settings()));
            scene.set_show_desktop_button(self.model.settings().show_desktop_button);
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

    pub(super) const fn scene(&self) -> &DockScene {
        &self.scene
    }

    pub(super) fn hit_test(&self, x: i32, y: i32) -> Option<DockHitTarget> {
        let x = u32::try_from(x).ok()?;
        let y = u32::try_from(y).ok()?;
        let size = self.scene.desired_size();
        self.scene
            .layout(size.width(), size.height())
            .hit_test(x, y)
    }

    pub(super) fn popup_target_anchor(
        &self,
        request: DockContextRequest,
    ) -> Option<(DockHitTarget, SignedPoint, PopupAlignment)> {
        let DockContextRequest::Pointer { screen, client } = request else {
            return None;
        };
        let target = self.hit_test(client.x, client.y)?;
        let size = self.scene.desired_size();
        let layout = self.scene.layout(size.width(), size.height());
        let bounds = match target {
            DockHitTarget::Item(source_index) => layout
                .items
                .iter()
                .find(|item| item.source_index == source_index)
                .map(|item| item.bounds)?,
            DockHitTarget::Jirachi => layout.jirachi,
            DockHitTarget::Media(_)
            | DockHitTarget::SystemStatus(_)
            | DockHitTarget::ShowDesktop => return None,
        };
        let (anchor_x, alignment) = match (target, self.scene.anchor()) {
            (DockHitTarget::Jirachi, DockAnchor::Left) => (0, PopupAlignment::Start),
            (DockHitTarget::Jirachi, DockAnchor::Right) => {
                (size.width(), PopupAlignment::End)
            }
            _ => (
                bounds.left.saturating_add(bounds.width / 2),
                PopupAlignment::Center,
            ),
        };
        let anchor_x = i32::try_from(anchor_x).ok()?;
        let overlap = i32::try_from((u64::from(self.scene.dpi()) * 6 + 48) / 96).ok()?;
        let top = i32::try_from(bounds.top).ok()?;
        Some((
            target,
            SignedPoint {
                x: screen.x.saturating_sub(client.x).saturating_add(anchor_x),
                y: screen
                    .y
                    .saturating_sub(client.y)
                    .saturating_add(top)
                    .saturating_add(overlap),
            },
            alignment,
        ))
    }

    pub(super) fn item(&self, source_index: usize) -> Option<&DockItem> {
        self.model.items().get(source_index)
    }

    pub(super) fn source_index(&self, identity: &str) -> Option<usize> {
        self.model
            .items()
            .iter()
            .position(|item| item.id.eq_ignore_ascii_case(identity))
    }

    pub(super) fn open_new(&self, source_index: usize, owner: WindowHandle) {
        let Some(item) = self.model.items().get(source_index) else {
            return;
        };
        if let Err(error) =
            execute_activation(lotus_core::activation::ActivationDecision::Launch, item)
        {
            show_error(
                owner,
                "Lotus",
                &format!("Lotus could not open {}.\n\n{error}", item.display_name),
            );
        }
    }

    pub(super) fn set_pinned(
        &mut self,
        source_index: usize,
        pinned: bool,
        windows: &[WindowInfo],
    ) -> Result<bool, AppError> {
        let previous = self
            .model
            .items()
            .get(source_index)
            .cloned()
            .map(|item| (source_index, item));
        if !self.model.set_pinned(source_index, pinned)? {
            return Ok(false);
        }
        if let Some((index, item)) = previous {
            if pinned {
                self.transient_unpinned.remove(&item.id);
            } else if item.is_running() {
                self.transient_unpinned
                    .insert(item.id.clone(), (index, item));
            }
        }
        let mut items = projected_items(self.model.settings(), windows);
        self.merge_transient_unpinned(&mut items, windows);
        self.model.rebuild(items);
        self.refresh_scene_items();
        Ok(true)
    }

    fn merge_transient_unpinned(
        &mut self,
        items: &mut Vec<DockItem>,
        windows: &[WindowInfo],
    ) {
        self.transient_unpinned.retain(|_, (_, item)| {
            item.windows = windows
                .iter()
                .filter(|window| window_matches_item(window, item))
                .cloned()
                .collect();
            !item.windows.is_empty()
        });

        let mut retained = self
            .transient_unpinned
            .values()
            .cloned()
            .collect::<Vec<_>>();
        retained.sort_by_key(|(index, _)| *index);
        for (index, item) in retained {
            if items
                .iter()
                .any(|current| current.id.eq_ignore_ascii_case(&item.id))
            {
                continue;
            }
            items.insert(index.min(items.len()), item);
        }
    }

    pub(super) fn picker_windows(
        &mut self,
        source_index: usize,
        foreground: Option<WindowId>,
    ) -> Vec<NativePickerWindow> {
        let Some(item) = self.model.items().get(source_index) else {
            return Vec::new();
        };
        let identity = item.id.clone();
        let display_name = item.display_name.clone();
        let icon_source = item.icon_source.clone();
        let windows = item.windows.clone();
        let recent = self
            .recent_windows
            .get(&identity)
            .cloned()
            .unwrap_or_default();
        let ordered = order_picker_windows(&windows, foreground, &recent);

        let size = self
            .scene
            .icon_size_pixels()
            .saturating_mul(NATIVE_ICON_SAMPLE_SCALE);
        let icon = self
            .native_icons
            .icon(Path::new(&icon_source), size)
            .ok()
            .flatten()
            .map_or(DockIcon::Embedded(SvgAsset::FluentOpen), DockIcon::Raster);
        ordered
            .into_iter()
            .map(|window| NativePickerWindow {
                id: window.id,
                title: if window.title.trim().is_empty() {
                    display_name.clone()
                } else {
                    window.title
                },
                icon: icon.clone(),
                active: Some(window.id) == foreground,
            })
            .collect()
    }

    pub(super) fn record_window_activation(
        &mut self,
        source_index: usize,
        window: WindowId,
    ) {
        let Some(item) = self.model.items().get(source_index) else {
            return;
        };
        let recent = self.recent_windows.entry(item.id.clone()).or_default();
        recent.retain(|candidate| *candidate != window);
        recent.insert(0, window);
    }

    pub(super) fn record_foreground(&mut self, window: Option<WindowId>) {
        let Some(window) = window else {
            return;
        };
        let source_index =
            self.model.items().iter().position(|item| {
                item.windows.iter().any(|candidate| candidate.id == window)
            });
        if let Some(source_index) = source_index {
            self.record_window_activation(source_index, window);
        }
    }

    pub(super) fn media_window(&self, source_id: &str) -> Option<WindowId> {
        let item = self.model.items().iter().find(|item| {
            item.windows.iter().any(|window| {
                window
                    .app_user_model_id
                    .as_deref()
                    .is_some_and(|identity| identity.eq_ignore_ascii_case(source_id))
            }) || media_identity_matches(source_id, &item.executable_path)
                || media_identity_matches(source_id, &item.display_name)
        })?;
        self.recent_windows
            .get(&item.id)
            .and_then(|recent| {
                recent.iter().find(|window| {
                    item.windows
                        .iter()
                        .any(|candidate| candidate.id == **window)
                })
            })
            .copied()
            .or_else(|| item.windows.first().map(|window| window.id))
    }

    pub(super) fn pointer_moved(&mut self, x: i32, y: i32) -> bool {
        let target = self.hit_test(x, y);
        self.interaction
            .pointer_moved(&mut self.scene, target, x, y)
    }

    pub(super) fn pointer_left(&mut self) -> bool {
        self.scene.set_hovered(None)
    }

    pub(super) fn pointer_pressed(&mut self, x: i32, y: i32) -> bool {
        let target = self.hit_test(x, y);
        self.interaction
            .pointer_pressed(&mut self.scene, target, x, y)
    }

    pub(super) fn pointer_released(
        &mut self,
        x: i32,
        y: i32,
    ) -> Result<(bool, Option<DockHitTarget>), AppError> {
        let released_over = self.hit_test(x, y);
        let pressed = self.scene.interaction().pressed;
        let mut changed =
            self.scene.set_pressed(None) | self.scene.set_hovered(released_over);
        self.interaction.release();

        if let Some(drag) = self.scene.drag() {
            changed |= self.scene.update_drag(x, y);
            let size = self.scene.desired_size();
            let insertion_slot =
                self.scene.drag_insertion_slot(size.width(), size.height());
            let source_index = drag.source_index;
            let layout = self.scene.layout(size.width(), size.height());
            let visible_sources = layout
                .items
                .iter()
                .map(|item| item.source_index)
                .collect::<Vec<_>>();
            changed |= self.scene.cancel_drag();
            let Some(insertion_slot) = insertion_slot.and_then(|slot| {
                map_visual_insertion_slot(self.model.items().len(), &visible_sources, slot)
            }) else {
                return Ok((changed, None));
            };
            changed |= self.model.persist_reorder(source_index, insertion_slot)?;
            self.refresh_scene_items();
            return Ok((changed, None));
        }

        Ok((
            changed,
            (pressed == released_over).then_some(pressed).flatten(),
        ))
    }

    pub(super) fn pointer_cancelled(&mut self) -> bool {
        self.interaction.cancel(&mut self.scene)
    }

    pub(super) fn refresh_status(&mut self) -> bool {
        let next = docked_status_items(self.model.settings());
        if self.scene.status_items() == next {
            return false;
        }

        self.scene.replace_status_items(next);
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
            media_identity_matches(source_id, &item.executable_path)
                || media_identity_matches(source_id, &item.display_name)
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

    pub(super) fn activate(&mut self, target: DockHitTarget, owner: WindowHandle) {
        let DockHitTarget::Item(source_index) = target else {
            return;
        };
        let foreground = foreground_window();
        let Some((decision, item)) =
            self.model.activation(source_index, foreground.as_ref())
        else {
            return;
        };
        let display_name = item.display_name.clone();
        if let Err(error) = execute_activation(decision, item) {
            show_error(
                owner,
                "Lotus",
                &format!("Lotus could not activate {display_name}.\n\n{error}"),
            );
        } else if let lotus_core::activation::ActivationDecision::Focus(window) = decision {
            self.record_window_activation(source_index, window);
        }
    }
}

fn departure_transition(
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

pub(super) fn status_items(settings: &DockSettings) -> Vec<SystemStatusItem> {
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
        items.push(SystemStatusItem::icon(
            SystemStatusKind::Network,
            SvgAsset::FluentNetwork,
        ));
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
        items.push(SystemStatusItem::date_time(local_time_24h(), date));
    }
    items
}

fn docked_status_items(settings: &DockSettings) -> Vec<SystemStatusItem> {
    if settings.system_status_zone == settings.dock_zone {
        status_items(settings)
    } else {
        Vec::new()
    }
}

fn projected_items(settings: &DockSettings, windows: &[WindowInfo]) -> Vec<DockItem> {
    project_snapshot(settings, windows, |target| {
        resolve_executable(target).map(|path| path.to_string_lossy().into_owned())
    })
}

fn media_identity_matches(source_id: &str, candidate: &str) -> bool {
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

fn window_matches_item(window: &WindowInfo, item: &DockItem) -> bool {
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

pub(super) fn metrics(settings: &DockSettings) -> Result<DockMetrics, AppError> {
    DockMetrics::new(
        settings.icon_size,
        settings.item_spacing,
        settings.horizontal_padding,
        settings.vertical_padding,
    )
    .ok_or(AppError::InvalidScene)
}

fn mascot(settings: &DockSettings) -> DockIcon {
    settings
        .mascot_image_path
        .as_deref()
        .and_then(|path| {
            lotus_windows::custom_image::load_custom_image(Path::new(path)).ok()
        })
        .map_or(DockIcon::Embedded(SvgAsset::LotusPixel), DockIcon::Raster)
}

pub(super) const fn dock_anchor(zone: DockZone) -> DockAnchor {
    match zone {
        DockZone::Left => DockAnchor::Left,
        DockZone::Center => DockAnchor::Center,
        DockZone::Right => DockAnchor::Right,
    }
}
