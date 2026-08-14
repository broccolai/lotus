use std::collections::HashMap;
use std::time::{Duration, Instant};

use lotus_core::dock::DockItem;
use lotus_core::notification::count_for_item;
use lotus_dock::interaction::{DockInteraction, map_visual_insertion_slot};
use lotus_dock::model::{DockModel, SettingsImpact, project_snapshot};
use lotus_settings::appearance::theme_for;

use super::{
    AppError, DockAnchor, DockBadge, DockContextRequest, DockHitTarget, DockIcon,
    DockMetrics, DockScene, DockSettings, DockZone, NativeIconCache,
    NotificationBadgeStyle, NotificationSource, Path, SettingsStore, SignedPoint, SvgAsset,
    SystemStatusItem, SystemStatusKind, WindowHandle, WindowInfo,
    adapt_dock_items_with_native, execute_activation, foreground_window, local_date,
    local_time_24h, resolve_executable, show_error,
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
        };
        runtime.refresh_scene_items();
        Ok(runtime)
    }

    pub(super) fn rebuild(&mut self, windows: &[WindowInfo]) {
        let previous_model = self.model.items().to_vec();
        let previous_scene = self.scene.items().to_vec();
        self.model
            .rebuild(projected_items(self.model.settings(), windows));
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
            let mut scene =
                DockScene::new(dpi, metrics, mascot(self.model.settings()), Vec::new())
                    .ok_or(AppError::InvalidScene)?;
            scene.set_anchor(dock_anchor(self.model.settings().dock_zone));
            let _ = scene.set_theme(theme_for(self.model.settings()));
            scene.set_show_desktop_button(self.model.settings().show_desktop_button);
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

    pub(super) fn jirachi_menu_anchor(
        &self,
        request: DockContextRequest,
    ) -> Option<SignedPoint> {
        let DockContextRequest::Pointer { screen, client } = request else {
            return None;
        };
        if self.hit_test(client.x, client.y) != Some(DockHitTarget::Jirachi) {
            return None;
        }
        let size = self.scene.desired_size();
        let jirachi = self.scene.layout(size.width(), size.height()).jirachi;
        let center_x =
            i32::try_from(jirachi.left.saturating_add(jirachi.width / 2)).ok()?;
        let overlap = i32::try_from((u64::from(self.scene.dpi()) * 6 + 48) / 96).ok()?;
        let top = i32::try_from(jirachi.top).ok()?;
        Some(SignedPoint {
            x: screen.x.saturating_sub(client.x).saturating_add(center_x),
            y: screen
                .y
                .saturating_sub(client.y)
                .saturating_add(top)
                .saturating_add(overlap),
        })
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

    pub(super) fn activate(&self, target: DockHitTarget, owner: WindowHandle) {
        let DockHitTarget::Item(source_index) = target else {
            return;
        };
        let foreground = foreground_window();
        let Some((decision, item)) =
            self.model.activation(source_index, foreground.as_ref())
        else {
            return;
        };
        if let Err(error) = execute_activation(decision, item) {
            show_error(
                owner,
                "Lotus",
                &format!("Lotus could not activate {}.\n\n{error}", item.display_name),
            );
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
