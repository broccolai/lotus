use crate::scene::{DockHitTarget, DockScene};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DragThreshold {
    horizontal: u32,
    vertical: u32,
}

impl From<(u32, u32)> for DragThreshold {
    fn from((horizontal, vertical): (u32, u32)) -> Self {
        Self { horizontal: horizontal.max(1), vertical: vertical.max(1) }
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
        Self { threshold: threshold.into(), candidate: None }
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
            changed |=
                scene.begin_drag(candidate.source_index, candidate.origin_x, candidate.origin_y);
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
            Some(DockHitTarget::Item(source_index)) => {
                Some(DragCandidate { source_index, origin_x: x, origin_y: y })
            }
            Some(DockHitTarget::Jirachi | DockHitTarget::ShowDesktop) | None => None,
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
}

pub fn map_visual_insertion_slot(
    item_count: usize,
    visible_source_indices: &[usize],
    visual_slot: usize,
) -> Option<usize> {
    if visual_slot > visible_source_indices.len()
        || visible_source_indices.iter().any(|source| *source >= item_count)
    {
        return None;
    }
    if let Some(source) = visible_source_indices.get(visual_slot) {
        return Some(*source);
    }
    Some(visible_source_indices.last().map_or(0, |source| source.saturating_add(1)).min(item_count))
}

fn threshold_crossed(candidate: DragCandidate, x: i32, y: i32, threshold: DragThreshold) -> bool {
    candidate.origin_x.abs_diff(x) >= threshold.horizontal
        || candidate.origin_y.abs_diff(y) >= threshold.vertical
}
