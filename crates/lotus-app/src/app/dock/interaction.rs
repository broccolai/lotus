use std::time::Instant;

use lotus_core::dock::DockItem;
use lotus_core::window::{TrackedWindowKey, WindowId, WindowInfo};
use lotus_dock::popup::order_picker_windows;
use lotus_ui::embedded_icon::EmbeddedIcon;
use lotus_windows::WindowHandle;
use lotus_windows::activation::foreground_window;
use lotus_windows::dialog::show_error;
use lotus_windows::responsiveness::{LayoutOperation, METRICS};
use lotus_windows::window::{
    DockContextRequest, PointerEvent, PopupAlignment, SignedPoint,
};

use super::projection::{media_source_matches_item, popup_overlap, status_popup_center};
use super::{DockRuntime, NATIVE_ICON_SAMPLE_SCALE};
use crate::app::activation;
use crate::app::visuals::{DockAnchor, DockHitTarget, DockIcon, NativePickerWindow};

impl DockRuntime {
    pub(in crate::app) fn hit_test(&self, x: i32, y: i32) -> Option<DockHitTarget> {
        let x = u32::try_from(x).ok()?;
        let y = u32::try_from(y).ok()?;
        let size = self.scene.desired_size();
        let started = Instant::now();
        let target = self
            .scene
            .layout(size.width(), size.height())
            .hit_test(x, y);
        METRICS.record_layout(LayoutOperation::DockHitTest, started.elapsed());
        target
    }

    pub(in crate::app) fn popup_target_anchor(
        &self,
        request: DockContextRequest,
    ) -> Option<(DockHitTarget, SignedPoint, PopupAlignment)> {
        let DockContextRequest::Pointer { screen, client, .. } = request else {
            return None;
        };
        let target = self.hit_test(client.x, client.y)?;
        let size = self.scene.desired_size();
        let started = Instant::now();
        let layout = self.scene.layout(size.width(), size.height());
        METRICS.record_layout(LayoutOperation::DockPopup, started.elapsed());
        let bounds = match target {
            DockHitTarget::Item(source_index) => layout
                .items
                .iter()
                .find(|item| item.source_index == source_index)
                .map(|item| item.bounds)?,
            DockHitTarget::Jirachi => layout.jirachi,
            DockHitTarget::SystemStatus(kind) => layout
                .status_items
                .iter()
                .find(|item| item.kind == kind)
                .map(|item| item.hit_bounds)?,
            DockHitTarget::Media(_) | DockHitTarget::ShowDesktop => return None,
        };
        let (anchor_x, alignment) = match (target, self.scene.anchor()) {
            (DockHitTarget::Jirachi, DockAnchor::Left) => (0, PopupAlignment::Start),
            (DockHitTarget::Jirachi, DockAnchor::Right) => {
                (size.width(), PopupAlignment::End)
            }
            (DockHitTarget::SystemStatus(_), _) => (
                status_popup_center(&layout.status_items)?,
                PopupAlignment::Center,
            ),
            _ => (
                bounds.left.saturating_add(bounds.width / 2),
                PopupAlignment::Center,
            ),
        };
        let anchor_x = i32::try_from(anchor_x).ok()?;
        let overlap = popup_overlap(self.scene.dpi());
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

    pub(in crate::app) fn item(&self, source_index: usize) -> Option<&DockItem> {
        self.model.items().get(source_index)
    }

    pub(in crate::app) fn source_index(&self, identity: &str) -> Option<usize> {
        lotus_dock::model::source_index_for_identity(self.model.items(), identity)
    }

    pub(in crate::app) fn source_index_for_key(
        &self,
        key: TrackedWindowKey,
    ) -> Option<usize> {
        self.model
            .items()
            .iter()
            .position(|item| item.windows.iter().any(|window| window.key() == key))
    }

    pub(in crate::app) fn tracked_key_for_window_id(
        &self,
        id: WindowId,
    ) -> Option<TrackedWindowKey> {
        self.model
            .items()
            .iter()
            .flat_map(|item| item.windows.iter())
            .find(|window| window.id == id)
            .map(WindowInfo::key)
    }

    pub(in crate::app) fn open_new(&self, source_index: usize, owner: WindowHandle) {
        let Some(item) = self.model.items().get(source_index) else {
            return;
        };
        if let Err(error) = activation::launch(item) {
            show_error(
                owner,
                "Lotus",
                &format!("Lotus could not open {}.\n\n{error}", item.display_name),
            );
        }
    }

    pub(in crate::app) fn picker_windows(
        &mut self,
        source_index: usize,
        foreground: Option<TrackedWindowKey>,
    ) -> Vec<NativePickerWindow> {
        let Some(item) = self.model.items().get(source_index) else {
            return Vec::new();
        };
        let identity = item.id.clone();
        let display_name = item.display_name.clone();
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
            .assets
            .picker_icon(self.model.settings(), item, size)
            .map_or(
                DockIcon::Embedded(EmbeddedIcon::FluentOpen),
                DockIcon::Raster,
            );
        ordered
            .into_iter()
            .map(|window| {
                let key = window.key();
                NativePickerWindow {
                    key,
                    title: if window.title.trim().is_empty() {
                        display_name.clone()
                    } else {
                        window.title
                    },
                    icon: icon.clone(),
                    active: Some(key) == foreground,
                }
            })
            .collect()
    }

    pub(in crate::app) fn application_icon_preview(
        &mut self,
        source_index: usize,
    ) -> Option<lotus_ui::icon::RasterIcon> {
        let item = self.model.items().get(source_index)?;
        let size = self
            .scene
            .icon_size_pixels()
            .saturating_mul(NATIVE_ICON_SAMPLE_SCALE);
        self.assets.preview_icon(self.model.settings(), item, size)
    }

    pub(in crate::app) fn record_window_activation(
        &mut self,
        source_index: usize,
        window: TrackedWindowKey,
    ) {
        let Some(item) = self.model.items().get(source_index) else {
            return;
        };
        let recent = self.recent_windows.entry(item.id.clone()).or_default();
        recent.retain(|candidate| *candidate != window);
        recent.insert(0, window);
    }

    pub(in crate::app) fn record_foreground(&mut self, window: Option<WindowId>) {
        let Some(window) = window else {
            return;
        };
        let key = self.model.items().iter().find_map(|item| {
            item.windows
                .iter()
                .find(|candidate| candidate.id == window)
                .map(WindowInfo::key)
        });
        if let Some(key) = key
            && let Some(source_index) = self.model.items().iter().position(|item| {
                item.windows.iter().any(|candidate| candidate.key() == key)
            })
        {
            self.record_window_activation(source_index, key);
        }
    }

    pub(in crate::app) fn prune_recent_windows(&mut self, windows: &[WindowInfo]) {
        self.recent_windows.retain(|_, recent| {
            recent.retain(|key| windows.iter().any(|window| window.key() == *key));
            !recent.is_empty()
        });
    }

    pub(in crate::app) fn media_application(
        &self,
        source_id: &str,
    ) -> Option<(usize, DockItem, Option<TrackedWindowKey>)> {
        self.model
            .items()
            .iter()
            .enumerate()
            .find(|(_, item)| {
                media_source_matches_item(source_id, item, &self.application_catalog)
            })
            .map(|(index, item)| {
                let preferred = self.recent_windows.get(&item.id).and_then(|recent| {
                    recent
                        .iter()
                        .copied()
                        .find(|key| item.windows.iter().any(|window| window.key() == *key))
                });
                (index, item.clone(), preferred)
            })
    }

    pub(in crate::app) fn pointer_moved(&mut self, x: i32, y: i32) -> bool {
        let target = self.hit_test(x, y);
        self.interaction
            .pointer_moved(&mut self.scene, target, x, y)
    }

    pub(in crate::app) fn handle_pointer_event(
        &mut self,
        event: PointerEvent,
    ) -> super::DockInteractionOutcome {
        let changed = match event {
            PointerEvent::Moved { x, y } => self.pointer_moved(x, y),
            PointerEvent::Left => self.pointer_left(),
            PointerEvent::LeftButtonPressed { x, y } => self.pointer_pressed(x, y),
            PointerEvent::LeftButtonReleased { x, y } => {
                return self.pointer_released(x, y);
            }
            PointerEvent::Cancelled => self.pointer_cancelled(),
        };
        super::DockInteractionOutcome {
            changed,
            intent: None,
        }
    }

    pub(in crate::app) fn pointer_left(&mut self) -> bool {
        self.scene.set_hovered(None)
    }

    pub(in crate::app) fn pointer_pressed(&mut self, x: i32, y: i32) -> bool {
        let target = self.hit_test(x, y);
        self.interaction
            .pointer_pressed(&mut self.scene, target, x, y)
    }

    pub(in crate::app) fn pointer_released(
        &mut self,
        x: i32,
        y: i32,
    ) -> super::DockInteractionOutcome {
        let had_drag = self.scene.drag().is_some();
        let started = Instant::now();
        let released_over = self.hit_test(x, y);
        let result = self.interaction.pointer_released(
            &mut self.scene,
            released_over,
            x,
            y,
            self.model.items(),
        );
        if had_drag {
            METRICS.record_layout(LayoutOperation::DockDrag, started.elapsed());
        }
        result
    }

    pub(in crate::app) fn pointer_cancelled(&mut self) -> bool {
        self.interaction.cancel(&mut self.scene)
    }

    pub(in crate::app) fn activate(&mut self, target: DockHitTarget, owner: WindowHandle) {
        let DockHitTarget::Item(source_index) = target else {
            return;
        };
        let foreground = foreground_window();
        let Some(item) = self.model.items().get(source_index).cloned() else {
            return;
        };
        let display_name = item.display_name.clone();
        match activation::activate_application(&item, foreground) {
            Ok(outcome) => {
                if let Some(window) = outcome.focused_key() {
                    self.record_window_activation(source_index, window);
                } else if matches!(outcome, activation::ActivationOutcome::ForegroundDenied)
                {
                    lotus_windows::diagnostics::record_diagnostic(
                        "activation.dock_foreground_denied",
                        "Windows denied a dock foreground request",
                    );
                }
            }
            Err(error) => {
                lotus_windows::diagnostics::record_error(
                    "activation.dock_application",
                    &error,
                );
                show_error(
                    owner,
                    "Lotus",
                    &format!("Lotus could not activate {display_name}.\n\n{error}"),
                );
            }
        }
    }
}
