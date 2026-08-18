use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::super::scene::{
    DockDragState, DockHitTarget, DockInteractionState, LaidOutItem,
};
use super::{
    CHROME_RESIZE_DISTANCE_DIP, CHROME_RESIZE_DURATION, EXIT_DURATION, HOVER_DURATION,
    PRESS_DURATION, REORDER_DURATION, TARGET_DPI,
};

#[derive(Default)]
pub(super) struct InteractionAnimator {
    items: HashMap<usize, ItemMotion>,
    jirachi: Option<ItemMotion>,
}

impl InteractionAnimator {
    pub(super) fn sample(
        &mut self,
        now: Instant,
        state: DockInteractionState,
        items: &[LaidOutItem],
    ) -> (Vec<ItemVisual>, ItemVisual, bool) {
        self.items.retain(|source_index, _| {
            items.iter().any(|item| item.source_index == *source_index)
        });
        let mut needs_animation = false;
        let visuals = items
            .iter()
            .map(|item| {
                let motion = self
                    .items
                    .entry(item.source_index)
                    .or_insert_with(|| ItemMotion::new(now));
                let target = DockHitTarget::Item(item.source_index);
                let (visual, animating) = motion.sample(
                    now,
                    state.hovered == Some(target),
                    state.pressed == Some(target),
                );
                needs_animation |= animating;
                visual
            })
            .collect();
        let jirachi = self.jirachi.get_or_insert_with(|| ItemMotion::new(now));
        let (jirachi_visual, jirachi_animating) = jirachi.sample(
            now,
            state.hovered == Some(DockHitTarget::Jirachi),
            state.pressed == Some(DockHitTarget::Jirachi),
        );
        needs_animation |= jirachi_animating;
        (visuals, jirachi_visual, needs_animation)
    }
}

#[derive(Default)]
pub(super) struct ReorderAnimator {
    items: HashMap<usize, OffsetMotion>,
    was_dragging: bool,
}

impl ReorderAnimator {
    pub(super) fn sample(
        &mut self,
        now: Instant,
        drag: Option<DockDragState>,
        items: &[LaidOutItem],
    ) -> (Vec<f32>, bool) {
        self.items.retain(|source_index, _| {
            items.iter().any(|item| item.source_index == *source_index)
        });
        let targets = drag.map_or_else(
            || vec![0.0; items.len()],
            |drag| reorder_targets(items, drag),
        );
        let released = self.was_dragging && drag.is_none();
        let mut animating = false;
        let offsets = items
            .iter()
            .zip(targets)
            .map(|(item, target)| {
                let motion = self
                    .items
                    .entry(item.source_index)
                    .or_insert_with(|| OffsetMotion::new(now));
                if released {
                    motion.snap(target, now);
                } else {
                    motion.retarget(target, now);
                }
                animating |= motion.is_animating(now);
                motion.sample(now)
            })
            .collect();
        self.was_dragging = drag.is_some();
        (offsets, animating)
    }
}

#[derive(Default)]
pub(super) struct ChromeAnimator {
    from: f32,
    target: f32,
    started: Option<Instant>,
}

impl ChromeAnimator {
    pub(super) fn sample(&mut self, now: Instant, width: u32, dpi: u32) -> (f32, bool) {
        let width = super::geometry::pixels_to_f32(width);
        if self.target == 0.0 {
            self.from = width;
            self.target = width;
            return (width, false);
        }
        if width > self.target {
            let scale = f32::from(u16::try_from(dpi).unwrap_or(u16::MAX)) / TARGET_DPI;
            self.from = (width - CHROME_RESIZE_DISTANCE_DIP * scale).max(self.target);
            self.target = width;
            self.started = Some(now);
        } else if width < self.target {
            self.from = width;
            self.target = width;
            self.started = None;
        }
        let Some(started) = self.started else {
            return (self.target, false);
        };
        let progress = (now.saturating_duration_since(started).as_secs_f32()
            / CHROME_RESIZE_DURATION.as_secs_f32())
        .clamp(0.0, 1.0);
        let width = self.from + (self.target - self.from) * ease_out_cubic(progress);
        let moving = progress < 1.0;
        if !moving {
            self.started = None;
        }
        (width, moving)
    }
}

#[derive(Default)]
pub(super) struct ExitAnimator {
    started: Option<Instant>,
}

impl ExitAnimator {
    pub(super) fn sample(&mut self, now: Instant, items: &[LaidOutItem]) -> (f32, bool) {
        if !items.iter().any(|item| item.exiting) {
            self.started = None;
            return (1.0, false);
        }
        let started = *self.started.get_or_insert(now);
        let progress = (now.saturating_duration_since(started).as_secs_f32()
            / EXIT_DURATION.as_secs_f32())
        .clamp(0.0, 1.0);
        (1.0 - progress, progress < 1.0)
    }
}

fn reorder_targets(items: &[LaidOutItem], drag: DockDragState) -> Vec<f32> {
    let Some(source_position) = items
        .iter()
        .position(|item| item.source_index == drag.source_index)
    else {
        return vec![0.0; items.len()];
    };
    let insertion_slot = items
        .iter()
        .position(|item| {
            i64::from(drag.pointer_x)
                < i64::from(item.bounds.left.saturating_add(item.bounds.width / 2))
        })
        .unwrap_or(items.len());
    let destination = if insertion_slot == items.len() {
        items.len().saturating_sub(1)
    } else if source_position < insertion_slot {
        insertion_slot.saturating_sub(1)
    } else {
        insertion_slot
    };
    let slot_width =
        super::geometry::pixels_to_f32(items[source_position].hit_bounds.width);
    (0..items.len())
        .map(|index| {
            if destination > source_position
                && index > source_position
                && index <= destination
            {
                -slot_width
            } else if destination < source_position
                && index >= destination
                && index < source_position
            {
                slot_width
            } else {
                0.0
            }
        })
        .collect()
}

struct OffsetMotion {
    from: f32,
    target: f32,
    started: Instant,
    moving: bool,
}
impl OffsetMotion {
    const fn new(now: Instant) -> Self {
        Self {
            from: 0.0,
            target: 0.0,
            started: now,
            moving: false,
        }
    }
    fn retarget(&mut self, target: f32, now: Instant) {
        if self.target.to_bits() == target.to_bits() {
            return;
        }
        self.from = self.sample(now);
        self.target = target;
        self.started = now;
        self.moving = true;
    }
    const fn snap(&mut self, target: f32, now: Instant) {
        self.from = target;
        self.target = target;
        self.started = now;
        self.moving = false;
    }
    fn sample(&self, now: Instant) -> f32 {
        if !self.moving {
            return self.target;
        }
        let progress = (now.saturating_duration_since(self.started).as_secs_f32()
            / REORDER_DURATION.as_secs_f32())
        .clamp(0.0, 1.0);
        self.from + (self.target - self.from) * ease_out_cubic(progress)
    }
    fn is_animating(&self, now: Instant) -> bool {
        self.moving && now.saturating_duration_since(self.started) < REORDER_DURATION
    }
}

struct ItemMotion {
    hover: AnimationTrack,
    press: AnimationTrack,
}
impl ItemMotion {
    fn new(now: Instant) -> Self {
        Self {
            hover: AnimationTrack::new(now, HOVER_DURATION),
            press: AnimationTrack::new(now, PRESS_DURATION),
        }
    }
    fn sample(&mut self, now: Instant, hovered: bool, pressed: bool) -> (ItemVisual, bool) {
        self.hover.retarget(hovered, now);
        self.press.retarget(pressed, now);
        let hover = self.hover.sample(now);
        let press = self.press.sample(now);
        let hover_translate_y = hover * -2.5;
        (
            ItemVisual {
                scale: 1.0 + (0.95 - 1.0) * press,
                translate_y: hover_translate_y + (1.0 - hover_translate_y) * press,
                icon_opacity: 1.0 - press * 0.10,
            },
            self.hover.is_animating(now) || self.press.is_animating(now),
        )
    }
}

struct AnimationTrack {
    from: f32,
    active: bool,
    moving: bool,
    started: Instant,
    duration: Duration,
}
impl AnimationTrack {
    const fn new(now: Instant, duration: Duration) -> Self {
        Self {
            from: 0.0,
            active: false,
            moving: false,
            started: now,
            duration,
        }
    }
    fn retarget(&mut self, active: bool, now: Instant) {
        if self.active == active {
            return;
        }
        self.from = self.sample(now);
        self.active = active;
        self.moving = true;
        self.started = now;
    }
    fn sample(&self, now: Instant) -> f32 {
        let target = if self.active {
            1.0
        } else {
            0.0
        };
        if !self.moving {
            return target;
        }
        let progress = (now.saturating_duration_since(self.started).as_secs_f32()
            / self.duration.as_secs_f32())
        .clamp(0.0, 1.0);
        self.from + (target - self.from) * ease_out_cubic(progress)
    }
    fn is_animating(&self, now: Instant) -> bool {
        self.moving && now.saturating_duration_since(self.started) < self.duration
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ItemVisual {
    pub(super) scale: f32,
    pub(super) translate_y: f32,
    pub(super) icon_opacity: f32,
}

fn ease_out_cubic(value: f32) -> f32 {
    1.0 - (1.0 - value).powi(3)
}
