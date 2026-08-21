use std::path::Path;

use lotus_core::activation::ActivationDecision;
use lotus_core::window::WindowId;
use lotus_dock::interaction::map_visual_insertion_slot;
use lotus_dock::popup::order_picker_windows;
use lotus_windows::WindowHandle;
use lotus_windows::activation::{ActivationError, execute_activation, foreground_window};
use lotus_windows::dialog::show_error;
use lotus_windows::graphics::assets::SvgAsset;
use lotus_windows::window::{DockContextRequest, PopupAlignment, SignedPoint};

use super::projection::{media_source_matches_item, popup_overlap, status_popup_center};
use super::{DockRuntime, NATIVE_ICON_SAMPLE_SCALE};
use crate::app::AppError;
use crate::app::visuals::{DockAnchor, DockHitTarget, DockIcon, NativePickerWindow};

impl DockRuntime {
    pub(in crate::app) fn hit_test(&self, x: i32, y: i32) -> Option<DockHitTarget> {
        let x = u32::try_from(x).ok()?;
        let y = u32::try_from(y).ok()?;
        let size = self.scene.desired_size();
        self.scene
            .layout(size.width(), size.height())
            .hit_test(x, y)
    }

    pub(in crate::app) fn popup_target_anchor(
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

    pub(in crate::app) fn item(
        &self,
        source_index: usize,
    ) -> Option<&lotus_core::dock::DockItem> {
        self.model.items().get(source_index)
    }

    pub(in crate::app) fn source_index(&self, identity: &str) -> Option<usize> {
        self.model
            .items()
            .iter()
            .position(|item| item.id.eq_ignore_ascii_case(identity))
    }

    pub(in crate::app) fn open_new(&self, source_index: usize, owner: WindowHandle) {
        let Some(item) = self.model.items().get(source_index) else {
            return;
        };
        if let Err(error) = execute_activation(ActivationDecision::Launch, item) {
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
        let icon = crate::app::icon_override::resolve_application_icon(
            self.model.settings(),
            &mut self.custom_images,
            windows
                .first()
                .and_then(|window| window.app_user_model_id.as_deref()),
            Some(&identity),
            Path::new(&icon_source),
        )
        .or_else(|| {
            self.native_icons
                .icon(Path::new(&icon_source), size)
                .ok()
                .flatten()
        })
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

    pub(in crate::app) fn application_icon_preview(
        &mut self,
        source_index: usize,
    ) -> Option<lotus_ui::icon::RasterIcon> {
        let item = self.model.items().get(source_index)?;
        let app_user_model_id = item.app_user_model_id.clone();
        let id = item.id.clone();
        let executable_path = item.executable_path.clone();
        let icon_source = item.icon_source.clone();
        let size = self
            .scene
            .icon_size_pixels()
            .saturating_mul(NATIVE_ICON_SAMPLE_SCALE);
        crate::app::icon_override::resolve_application_icon(
            self.model.settings(),
            &mut self.custom_images,
            app_user_model_id.as_deref(),
            Some(&id),
            Path::new(&executable_path),
        )
        .or_else(|| {
            self.native_icons
                .icon(Path::new(&icon_source), size)
                .ok()
                .flatten()
        })
    }

    pub(in crate::app) fn record_window_activation(
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

    pub(in crate::app) fn record_foreground(&mut self, window: Option<WindowId>) {
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

    pub(in crate::app) fn media_window(&self, source_id: &str) -> Option<WindowId> {
        let item = self
            .model
            .items()
            .iter()
            .find(|item| media_source_matches_item(source_id, item))?;
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

    pub(in crate::app) fn pointer_moved(&mut self, x: i32, y: i32) -> bool {
        let target = self.hit_test(x, y);
        self.interaction
            .pointer_moved(&mut self.scene, target, x, y)
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

    pub(in crate::app) fn pointer_cancelled(&mut self) -> bool {
        self.interaction.cancel(&mut self.scene)
    }

    pub(in crate::app) fn activate(&mut self, target: DockHitTarget, owner: WindowHandle) {
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
        match execute_activation(decision, item) {
            Ok(()) => {
                if let ActivationDecision::Focus(window) = decision {
                    self.record_window_activation(source_index, window);
                }
            }
            Err(ActivationError::ForegroundDenied(_)) => {}
            Err(error) => show_error(
                owner,
                "Lotus",
                &format!("Lotus could not activate {display_name}.\n\n{error}"),
            ),
        }
    }
}
