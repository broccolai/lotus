use lotus_core::dock::DockItem;

use crate::model::DockReorderRequest;
use crate::scene::{DockHitTarget, DockScene};

#[derive(Debug)]
pub struct DockInteractionOutcome {
    pub changed: bool,
    pub intent: Option<DockInteractionIntent>,
}

#[derive(Debug)]
pub enum DockInteractionIntent {
    Activate(DockHitTarget),
    Reorder(DockReorderRequest),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DragThreshold {
    horizontal: u32,
    vertical: u32,
}

impl From<(u32, u32)> for DragThreshold {
    fn from((horizontal, vertical): (u32, u32)) -> Self {
        Self {
            horizontal: horizontal.max(1),
            vertical: vertical.max(1),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DragCandidate {
    source_index: usize,
    origin_x: i32,
    origin_y: i32,
}

pub struct DockInteraction {
    threshold: DragThreshold,
    candidate: Option<DragCandidate>,
}

impl DockInteraction {
    pub fn new(threshold: (u32, u32)) -> Self {
        Self {
            threshold: threshold.into(),
            candidate: None,
        }
    }

    pub fn set_drag_threshold(&mut self, threshold: (u32, u32)) {
        self.threshold = threshold.into();
    }

    pub fn pointer_moved<Asset: Clone>(
        &mut self,
        scene: &mut DockScene<Asset>,
        target: Option<DockHitTarget>,
        x: i32,
        y: i32,
    ) -> bool {
        let mut changed = scene.set_hovered(target);
        if scene.drag().is_some() {
            changed |= scene.update_drag(x, y);
        } else if let Some(candidate) = self.candidate
            && threshold_crossed(candidate, x, y, self.threshold)
        {
            changed |= scene.begin_drag(
                candidate.source_index,
                candidate.origin_x,
                candidate.origin_y,
            );
            changed |= scene.update_drag(x, y);
        }
        changed
    }

    pub fn pointer_pressed<Asset: Clone>(
        &mut self,
        scene: &mut DockScene<Asset>,
        target: Option<DockHitTarget>,
        x: i32,
        y: i32,
    ) -> bool {
        self.candidate = match target {
            Some(DockHitTarget::Item(source_index)) => Some(DragCandidate {
                source_index,
                origin_x: x,
                origin_y: y,
            }),
            Some(
                DockHitTarget::Jirachi
                | DockHitTarget::Media(_)
                | DockHitTarget::SystemStatus(_)
                | DockHitTarget::ShowDesktop,
            )
            | None => None,
        };
        scene.set_hovered(target) | scene.set_pressed(target)
    }

    pub fn release(&mut self) {
        self.candidate = None;
    }

    pub fn cancel<Asset: Clone>(&mut self, scene: &mut DockScene<Asset>) -> bool {
        self.candidate = None;
        scene.cancel_drag() | scene.set_pressed(None) | scene.set_hovered(None)
    }

    pub fn pointer_released<Asset: Clone>(
        &mut self,
        scene: &mut DockScene<Asset>,
        released_over: Option<DockHitTarget>,
        x: i32,
        y: i32,
        items: &[DockItem],
    ) -> DockInteractionOutcome {
        let pressed = scene.interaction().pressed;
        let mut changed = scene.set_pressed(None) | scene.set_hovered(released_over);
        self.candidate = None;

        if let Some(drag) = scene.drag() {
            changed |= scene.update_drag(x, y);
            let size = scene.desired_size();
            let insertion_slot = scene.drag_insertion_slot(size.width(), size.height());
            let source_index = drag.source_index;
            let visible_sources = scene
                .layout(size.width(), size.height())
                .items
                .iter()
                .map(|item| item.source_index)
                .collect::<Vec<_>>();
            changed |= scene.cancel_drag();
            let Some(insertion_slot) = insertion_slot.and_then(|slot| {
                map_visual_insertion_slot(items.len(), &visible_sources, slot)
            }) else {
                return DockInteractionOutcome {
                    changed,
                    intent: None,
                };
            };
            let Some(request) = reorder_request(items, source_index, insertion_slot) else {
                return DockInteractionOutcome {
                    changed,
                    intent: None,
                };
            };
            return DockInteractionOutcome {
                changed,
                intent: Some(DockInteractionIntent::Reorder(request)),
            };
        }
        let intent = if pressed == released_over {
            pressed.map(DockInteractionIntent::Activate)
        } else {
            None
        };
        DockInteractionOutcome { changed, intent }
    }
}

fn reorder_request(
    items: &[DockItem],
    source_index: usize,
    insertion_slot: usize,
) -> Option<DockReorderRequest> {
    let source_id = items.get(source_index)?.id.clone();
    let (target_index, insert_after) = if insertion_slot == items.len() {
        (items.len().checked_sub(1)?, true)
    } else {
        (insertion_slot, false)
    };
    Some(DockReorderRequest {
        source_id,
        target_id: items.get(target_index)?.id.clone(),
        insert_after,
    })
}

pub fn map_visual_insertion_slot(
    item_count: usize,
    visible_source_indices: &[usize],
    visual_slot: usize,
) -> Option<usize> {
    if visual_slot > visible_source_indices.len()
        || visible_source_indices
            .iter()
            .any(|source| *source >= item_count)
    {
        return None;
    }
    if let Some(source) = visible_source_indices.get(visual_slot) {
        return Some(*source);
    }
    Some(
        visible_source_indices
            .last()
            .map_or(0, |source| source.saturating_add(1))
            .min(item_count),
    )
}

fn threshold_crossed(
    candidate: DragCandidate,
    x: i32,
    y: i32,
    threshold: DragThreshold,
) -> bool {
    candidate.origin_x.abs_diff(x) >= threshold.horizontal
        || candidate.origin_y.abs_diff(y) >= threshold.vertical
}
