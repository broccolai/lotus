use std::collections::HashMap;
use std::time::{Duration, Instant};

use lotus_ui::icon::Icon;
use lotus_ui::presentation::{
    FontFamily, FontWeight, HorizontalAlignment, ImageSampling, Presentation,
    PresentationPrimitive, PresentationRect, TextStyle, VerticalAlignment,
};
use lotus_ui::theme::Color;

use super::{
    DockAnchor, DockBadge, DockDragState, DockHitTarget, DockInteractionState, DockScene,
    LaidOutItem, LaidOutMedia, LaidOutStatusItem, PixelRect, SystemStatusKind,
};

const HOVER_DURATION: Duration = Duration::from_millis(145);
const PRESS_DURATION: Duration = Duration::from_millis(80);
const REORDER_DURATION: Duration = Duration::from_millis(180);
const CHROME_DURATION: Duration = Duration::from_millis(90);
const EXIT_DURATION: Duration = Duration::from_millis(80);

#[derive(Default)]
pub struct DockPresenter {
    interaction: InteractionAnimator,
    reorder: ReorderAnimator,
    chrome: ChromeAnimator,
    exit: ExitAnimator,
}

impl DockPresenter {
    pub fn present<Asset: Clone>(
        &mut self,
        scene: &DockScene<Asset>,
        width: u32,
        height: u32,
    ) -> (Presentation<Asset>, bool) {
        let layout = scene.layout(width, height);
        let now = Instant::now();
        let (visuals, mascot_visual, interaction_animating) =
            self.interaction
                .sample(now, scene.interaction(), &layout.items);
        let (offsets, reorder_animating) =
            self.reorder.sample(now, scene.drag(), &layout.items);
        let (chrome_width, chrome_animating) = self.chrome.sample(now, width, scene.dpi());
        let (exit_opacity, exit_animating) = self.exit.sample(now, &layout.items);
        let mut output = Presentation::new(scene.theme().canvas.with_alpha(0.0));

        output.push(fill(
            anchored_rect(width, height, chrome_width, scene.anchor()),
            scaled(scene.theme().radii.window, scene.dpi()),
            scene.theme().chrome_overlay,
        ));
        present_items(
            &mut output,
            scene,
            &layout.items,
            visuals,
            offsets,
            exit_opacity,
            width,
            height,
        );
        present_divider(
            &mut output,
            layout.launcher_button_visible,
            layout.divider,
            scene,
        );
        if let Some(bounds) = layout.media_divider {
            present_divider(&mut output, true, bounds, scene);
        }
        present_media(&mut output, layout.media.as_ref(), scene);
        if let Some(bounds) = layout.status_divider {
            present_divider(&mut output, true, bounds, scene);
        }
        present_status(&mut output, &layout.status_items, scene);
        present_show_desktop(&mut output, layout.show_desktop, scene);
        if layout.launcher_button_visible {
            let bounds = transformed(
                layout.jirachi,
                mascot_visual.scale,
                0.0,
                scaled(mascot_visual.translate_y, scene.dpi()),
            );
            output.push(PresentationPrimitive::Icon {
                icon: scene.mascot().clone(),
                bounds: fit_icon(scene.mascot(), bounds),
                tint: scene.theme().text,
                opacity: mascot_visual.opacity,
                sampling: icon_sampling(scene.mascot(), layout.jirachi.width),
                radius: 0.0,
            });
        }

        (
            output,
            interaction_animating
                || reorder_animating
                || chrome_animating
                || exit_animating,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn present_items<Asset: Clone>(
    output: &mut Presentation<Asset>,
    scene: &DockScene<Asset>,
    items: &[LaidOutItem<Asset>],
    visuals: Vec<ItemVisual>,
    offsets: Vec<f32>,
    exit_opacity: f32,
    width: u32,
    height: u32,
) {
    let mut draws = items
        .iter()
        .zip(visuals.into_iter().zip(offsets))
        .map(|(item, (mut visual, offset))| {
            let dragged = scene
                .drag()
                .is_some_and(|drag| drag.source_index == item.source_index);
            if scene.drag().is_some() {
                visual = ItemVisual::default();
            }
            if item.exiting {
                visual.opacity *= exit_opacity;
            }
            let bounds = scene.drag().filter(|_| dragged).map_or_else(
                || {
                    transformed(
                        item.bounds,
                        visual.scale,
                        if matches!(item.icon, Icon::Raster(_)) {
                            offset.round()
                        } else {
                            offset
                        },
                        scaled(visual.translate_y, scene.dpi()),
                    )
                },
                |drag| dragged_rect(drag, item.bounds.width, width, height),
            );
            (item, visual, bounds, dragged)
        })
        .collect::<Vec<_>>();
    draws.sort_by_key(|(_, _, _, dragged)| *dragged);

    for (item, visual, bounds, _) in &draws {
        output.push(PresentationPrimitive::Icon {
            icon: item.icon.clone(),
            bounds: *bounds,
            tint: scene.theme().text,
            opacity: visual.opacity,
            sampling: icon_sampling(&item.icon, item.bounds.width),
            radius: 0.0,
        });
    }
    for (item, visual, bounds, _) in &draws {
        if item.running {
            output.push(fill(
                running_indicator(*bounds, height, scene.dpi()),
                scaled(1.0, scene.dpi()),
                scene.theme().accent.with_alpha(visual.opacity * 0.72),
            ));
        }
    }
    for (item, visual, bounds, _) in draws {
        if let Some(badge) = item.badge {
            present_badge(output, badge, bounds, scene, visual.opacity);
        }
    }
}

fn present_badge<Asset: Clone>(
    output: &mut Presentation<Asset>,
    badge: DockBadge,
    icon: PresentationRect,
    scene: &DockScene<Asset>,
    opacity: f32,
) {
    let scale = scale_factor(scene.dpi());
    let (width, height) = match badge {
        DockBadge::Dot => (8.0 * scale, 8.0 * scale),
        DockBadge::Count(count) if count < 10 => (18.0 * scale, 18.0 * scale),
        DockBadge::Count(count) if count < 100 => (24.0 * scale, 18.0 * scale),
        DockBadge::Count(_) | DockBadge::AtLeast(_) => (30.0 * scale, 18.0 * scale),
    };
    let bounds = PresentationRect::new(
        icon.right - width + 3.0 * scale,
        icon.top - 3.0 * scale,
        icon.right + 3.0 * scale,
        icon.top + height - 3.0 * scale,
    );
    output.push(fill(
        bounds,
        height * 0.5,
        scene.theme().accent.with_alpha(opacity),
    ));
    if badge != DockBadge::Dot {
        output.push(PresentationPrimitive::Text {
            value: match badge {
                DockBadge::AtLeast(count) => format!("{count}+"),
                DockBadge::Count(count) if count > 99 => "99+".to_owned(),
                DockBadge::Count(count) => count.to_string(),
                DockBadge::Dot => String::new(),
            },
            bounds,
            style: text_style(10.5 * scale, FontWeight::Semibold),
            color: scene.theme().on_accent.with_alpha(opacity),
        });
    }
}

fn present_divider<Asset: Clone>(
    output: &mut Presentation<Asset>,
    visible: bool,
    bounds: PixelRect,
    scene: &DockScene<Asset>,
) {
    if visible {
        output.push(fill(
            rect(bounds),
            scaled(1.0, scene.dpi()),
            scene.theme().divider,
        ));
    }
}

fn present_media<Asset: Clone>(
    output: &mut Presentation<Asset>,
    media: Option<&LaidOutMedia<Asset>>,
    scene: &DockScene<Asset>,
) {
    let Some(media) = media else {
        return;
    };
    let metadata_opacity = target_opacity(
        scene.interaction(),
        DockHitTarget::Media(lotus_media::MediaHitTarget::Metadata),
    );
    output.push(PresentationPrimitive::Icon {
        icon: media.artwork.icon.clone(),
        bounds: rect(media.artwork.bounds),
        tint: scene.theme().text,
        opacity: metadata_opacity,
        sampling: ImageSampling::Smooth,
        radius: scaled(scene.theme().radii.control, scene.dpi()),
    });
    let midpoint = media
        .metadata
        .top
        .saturating_add(media.metadata.height.saturating_mul(11) / 20);
    let title = PixelRect {
        left: media.metadata.left,
        top: media.metadata.top,
        width: media.metadata.width,
        height: midpoint.saturating_sub(media.metadata.top),
    };
    let artist = PixelRect {
        left: media.metadata.left,
        top: midpoint,
        width: media.metadata.width,
        height: media
            .metadata
            .top
            .saturating_add(media.metadata.height)
            .saturating_sub(midpoint),
    };
    output.push(PresentationPrimitive::Text {
        value: media.title.clone(),
        bounds: rect(title),
        style: text_style(scaled(12.5, scene.dpi()), FontWeight::Normal),
        color: scene.theme().text.with_alpha(metadata_opacity),
    });
    output.push(PresentationPrimitive::Text {
        value: media.artist.clone(),
        bounds: rect(artist),
        style: text_style(scaled(10.5, scene.dpi()), FontWeight::Normal),
        color: scene.theme().text_muted.with_alpha(metadata_opacity),
    });
    for control in &media.controls {
        let opacity = if control.enabled {
            target_opacity(scene.interaction(), DockHitTarget::Media(control.target))
        } else {
            0.34
        };
        output.push(PresentationPrimitive::Icon {
            icon: control.icon.clone(),
            bounds: rect(inset(control.bounds, 5)),
            tint: scene.theme().text,
            opacity,
            sampling: ImageSampling::Smooth,
            radius: 0.0,
        });
    }
}

fn present_status<Asset: Clone>(
    output: &mut Presentation<Asset>,
    items: &[LaidOutStatusItem<Asset>],
    scene: &DockScene<Asset>,
) {
    for item in items {
        let opacity =
            target_opacity(scene.interaction(), DockHitTarget::SystemStatus(item.kind));
        if let Some(icon) = &item.icon {
            output.push(PresentationPrimitive::Icon {
                icon: icon.icon.clone(),
                bounds: rect(icon.bounds),
                tint: scene.theme().text,
                opacity,
                sampling: ImageSampling::Smooth,
                radius: 0.0,
            });
        } else if item.kind == SystemStatusKind::DateTime {
            present_clock(output, item, scene, opacity);
        } else {
            output.push(PresentationPrimitive::Text {
                value: item.primary_text.clone(),
                bounds: rect(item.hit_bounds),
                style: TextStyle {
                    size: scaled(18.0, scene.dpi()),
                    family: FontFamily::SystemSymbols,
                    weight: FontWeight::Normal,
                    horizontal: HorizontalAlignment::Center,
                    vertical: VerticalAlignment::Center,
                },
                color: scene.theme().text.with_alpha(opacity),
            });
        }
    }
}

fn present_clock<Asset: Clone>(
    output: &mut Presentation<Asset>,
    item: &LaidOutStatusItem<Asset>,
    scene: &DockScene<Asset>,
    opacity: f32,
) {
    if item.secondary_text.is_empty() {
        output.push(PresentationPrimitive::Text {
            value: item.primary_text.clone(),
            bounds: rect(item.hit_bounds),
            style: text_style(scaled(12.5, scene.dpi()), FontWeight::Normal),
            color: scene.theme().text.with_alpha(opacity),
        });
        return;
    }
    let stack_height = item.hit_bounds.height.saturating_mul(3) / 5;
    let top = item
        .hit_bounds
        .top
        .saturating_add(item.hit_bounds.height.saturating_sub(stack_height) / 2);
    let midpoint = top.saturating_add(stack_height / 2);
    output.push(PresentationPrimitive::Text {
        value: item.primary_text.clone(),
        bounds: rect(PixelRect {
            left: item.hit_bounds.left,
            top,
            width: item.hit_bounds.width,
            height: midpoint.saturating_sub(top),
        }),
        style: text_style(scaled(12.5, scene.dpi()), FontWeight::Normal),
        color: scene.theme().text.with_alpha(opacity),
    });
    output.push(PresentationPrimitive::Text {
        value: item.secondary_text.clone(),
        bounds: rect(PixelRect {
            left: item.hit_bounds.left,
            top: midpoint,
            width: item.hit_bounds.width,
            height: top.saturating_add(stack_height).saturating_sub(midpoint),
        }),
        style: text_style(scaled(10.5, scene.dpi()), FontWeight::Normal),
        color: scene.theme().text_muted.with_alpha(opacity),
    });
}

fn present_show_desktop<Asset: Clone>(
    output: &mut Presentation<Asset>,
    bounds: Option<PixelRect>,
    scene: &DockScene<Asset>,
) {
    let Some(bounds) = bounds else {
        return;
    };
    let opacity = if scene.interaction().pressed == Some(DockHitTarget::ShowDesktop) {
        1.0
    } else if scene.interaction().hovered == Some(DockHitTarget::ShowDesktop) {
        0.7
    } else {
        0.0
    };
    output.push(fill(
        rect(bounds),
        scaled(1.0, scene.dpi()),
        scene.theme().control_hover.with_alpha(opacity),
    ));
}

fn target_opacity(interaction: DockInteractionState, target: DockHitTarget) -> f32 {
    if interaction.pressed == Some(target) {
        0.62
    } else if interaction.hovered == Some(target) {
        1.0
    } else {
        0.78
    }
}

fn text_style(size: f32, weight: FontWeight) -> TextStyle {
    TextStyle {
        size,
        family: FontFamily::Interface,
        weight,
        horizontal: HorizontalAlignment::Center,
        vertical: VerticalAlignment::Center,
    }
}

fn fill<Asset>(
    bounds: PresentationRect,
    radius: f32,
    color: Color,
) -> PresentationPrimitive<Asset> {
    PresentationPrimitive::FillRoundedRect {
        bounds,
        radius,
        color,
    }
}

fn rect(value: PixelRect) -> PresentationRect {
    PresentationRect::new(
        as_f32(value.left),
        as_f32(value.top),
        as_f32(value.left.saturating_add(value.width)),
        as_f32(value.top.saturating_add(value.height)),
    )
}

fn inset(mut value: PixelRect, numerator: u32) -> PixelRect {
    let amount = value.width.saturating_mul(numerator) / 28;
    value.left = value.left.saturating_add(amount);
    value.top = value.top.saturating_add(amount);
    value.width = value.width.saturating_sub(amount.saturating_mul(2));
    value.height = value.height.saturating_sub(amount.saturating_mul(2));
    value
}

fn transformed(value: PixelRect, scale: f32, x: f32, y: f32) -> PresentationRect {
    let source = rect(value);
    let center_x = f32::midpoint(source.left, source.right);
    let center_y = f32::midpoint(source.top, source.bottom);
    let half_width = source.width() * 0.5 * scale;
    let half_height = source.height() * 0.5 * scale;
    PresentationRect::new(
        center_x - half_width + x,
        center_y - half_height + y,
        center_x + half_width + x,
        center_y + half_height + y,
    )
}

fn dragged_rect(
    drag: DockDragState,
    side: u32,
    width: u32,
    height: u32,
) -> PresentationRect {
    let half = as_f32(side) * 0.5;
    let center_x = clamped_drag_center(drag.pointer_x, width, half);
    let center_y = clamped_drag_center(drag.pointer_y, height, half);
    PresentationRect::new(
        center_x - half,
        center_y - half,
        center_x + half,
        center_y + half,
    )
}

#[allow(
    clippy::cast_precision_loss,
    reason = "captured dock pointer coordinates remain below f32 exact range"
)]
fn clamped_drag_center(pointer: i32, extent: u32, half: f32) -> f32 {
    let extent = as_f32(extent);
    if extent <= half * 2.0 {
        extent * 0.5
    } else {
        (pointer as f32).clamp(half, extent - half)
    }
}

fn running_indicator(icon: PresentationRect, height: u32, dpi: u32) -> PresentationRect {
    let width = scaled(8.0, dpi);
    let line_height = scaled(2.0, dpi);
    let center = f32::midpoint(icon.left, icon.right);
    let bottom = as_f32(height);
    PresentationRect::new(
        center - width * 0.5,
        bottom - line_height,
        center + width * 0.5,
        bottom,
    )
}

fn fit_icon<Asset>(icon: &Icon<Asset>, bounds: PresentationRect) -> PresentationRect {
    let Icon::Raster(raster) = icon else {
        return bounds;
    };
    let aspect = as_f32(raster.width()) / as_f32(raster.height());
    let (width, height) = if bounds.width() / bounds.height() > aspect {
        (bounds.height() * aspect, bounds.height())
    } else {
        (bounds.width(), bounds.width() / aspect)
    };
    let center_x = f32::midpoint(bounds.left, bounds.right);
    let center_y = f32::midpoint(bounds.top, bounds.bottom);
    PresentationRect::new(
        center_x - width * 0.5,
        center_y - height * 0.5,
        center_x + width * 0.5,
        center_y + height * 0.5,
    )
}

fn icon_sampling<Asset>(icon: &Icon<Asset>, target: u32) -> ImageSampling {
    match icon {
        Icon::Raster(raster) if raster.width() != target || raster.height() != target => {
            ImageSampling::Smooth
        }
        Icon::Embedded(_) | Icon::Raster(_) => ImageSampling::PixelAligned,
    }
}

fn anchored_rect(
    width: u32,
    height: u32,
    chrome: f32,
    anchor: DockAnchor,
) -> PresentationRect {
    let left = match anchor {
        DockAnchor::Left => 0.0,
        DockAnchor::Center => (as_f32(width) - chrome) * 0.5,
        DockAnchor::Right => as_f32(width) - chrome,
    };
    PresentationRect::new(left, 0.0, left + chrome, as_f32(height))
}

fn scaled(value: f32, dpi: u32) -> f32 {
    value * scale_factor(dpi)
}

fn scale_factor(dpi: u32) -> f32 {
    f32::from(u16::try_from(dpi).unwrap_or(u16::MAX)) / 96.0
}

#[allow(
    clippy::cast_precision_loss,
    reason = "dock dimensions and pointer coordinates remain below f32 exact range"
)]
fn as_f32(value: u32) -> f32 {
    value as f32
}

#[derive(Clone, Copy)]
struct ItemVisual {
    scale: f32,
    translate_y: f32,
    opacity: f32,
}

impl Default for ItemVisual {
    fn default() -> Self {
        Self {
            scale: 1.0,
            translate_y: 0.0,
            opacity: 1.0,
        }
    }
}

#[derive(Default)]
struct InteractionAnimator {
    items: HashMap<usize, ItemMotion>,
    mascot: Option<ItemMotion>,
}

impl InteractionAnimator {
    fn sample<Asset>(
        &mut self,
        now: Instant,
        state: DockInteractionState,
        items: &[LaidOutItem<Asset>],
    ) -> (Vec<ItemVisual>, ItemVisual, bool) {
        self.items
            .retain(|source, _| items.iter().any(|item| item.source_index == *source));
        let mut animating = false;
        let visuals = items
            .iter()
            .map(|item| {
                let motion = self
                    .items
                    .entry(item.source_index)
                    .or_insert_with(|| ItemMotion::new(now));
                let target = DockHitTarget::Item(item.source_index);
                let (visual, moving) = motion.sample(
                    now,
                    state.hovered == Some(target),
                    state.pressed == Some(target),
                );
                animating |= moving;
                visual
            })
            .collect();
        let mascot = self.mascot.get_or_insert_with(|| ItemMotion::new(now));
        let (mascot, moving) = mascot.sample(
            now,
            state.hovered == Some(DockHitTarget::Jirachi),
            state.pressed == Some(DockHitTarget::Jirachi),
        );
        (visuals, mascot, animating || moving)
    }
}

struct ItemMotion {
    hover: Track,
    press: Track,
}

impl ItemMotion {
    fn new(now: Instant) -> Self {
        Self {
            hover: Track::new(now, HOVER_DURATION),
            press: Track::new(now, PRESS_DURATION),
        }
    }

    fn sample(&mut self, now: Instant, hovered: bool, pressed: bool) -> (ItemVisual, bool) {
        self.hover.retarget(hovered, now);
        self.press.retarget(pressed, now);
        let hover = self.hover.sample(now);
        let press = self.press.sample(now);
        let hover_y = hover * -2.5;
        (
            ItemVisual {
                scale: 1.0 + (0.95 - 1.0) * press,
                translate_y: hover_y + (1.0 - hover_y) * press,
                opacity: 1.0 - press * 0.1,
            },
            self.hover.animating(now) || self.press.animating(now),
        )
    }
}

struct Track {
    from: f32,
    active: bool,
    moving: bool,
    started: Instant,
    duration: Duration,
}

impl Track {
    const fn new(started: Instant, duration: Duration) -> Self {
        Self {
            from: 0.0,
            active: false,
            moving: false,
            started,
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
        self.from + (target - self.from) * eased(progress(now, self.started, self.duration))
    }

    fn animating(&self, now: Instant) -> bool {
        self.moving && now.saturating_duration_since(self.started) < self.duration
    }
}

#[derive(Default)]
struct ReorderAnimator {
    items: HashMap<usize, OffsetMotion>,
    was_dragging: bool,
}

impl ReorderAnimator {
    fn sample<Asset>(
        &mut self,
        now: Instant,
        drag: Option<DockDragState>,
        items: &[LaidOutItem<Asset>],
    ) -> (Vec<f32>, bool) {
        self.items
            .retain(|source, _| items.iter().any(|item| item.source_index == *source));
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
                animating |= motion.animating(now);
                motion.sample(now)
            })
            .collect();
        self.was_dragging = drag.is_some();
        (offsets, animating)
    }
}

fn reorder_targets<Asset>(items: &[LaidOutItem<Asset>], drag: DockDragState) -> Vec<f32> {
    let Some(source) = items
        .iter()
        .position(|item| item.source_index == drag.source_index)
    else {
        return vec![0.0; items.len()];
    };
    let insertion = items
        .iter()
        .position(|item| {
            i64::from(drag.pointer_x)
                < i64::from(item.bounds.left.saturating_add(item.bounds.width / 2))
        })
        .unwrap_or(items.len());
    let destination = if insertion == items.len() {
        items.len().saturating_sub(1)
    } else if source < insertion {
        insertion.saturating_sub(1)
    } else {
        insertion
    };
    let slot = as_f32(items[source].hit_bounds.width);
    (0..items.len())
        .map(|index| {
            if destination > source && index > source && index <= destination {
                -slot
            } else if destination < source && index >= destination && index < source {
                slot
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
    const fn new(started: Instant) -> Self {
        Self {
            from: 0.0,
            target: 0.0,
            started,
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
    fn snap(&mut self, target: f32, now: Instant) {
        self.from = target;
        self.target = target;
        self.started = now;
        self.moving = false;
    }
    fn sample(&self, now: Instant) -> f32 {
        if self.moving {
            self.from
                + (self.target - self.from)
                    * eased(progress(now, self.started, REORDER_DURATION))
        } else {
            self.target
        }
    }
    fn animating(&self, now: Instant) -> bool {
        self.moving && now.saturating_duration_since(self.started) < REORDER_DURATION
    }
}

#[derive(Default)]
struct ChromeAnimator {
    from: f32,
    target: f32,
    started: Option<Instant>,
}

impl ChromeAnimator {
    fn sample(&mut self, now: Instant, width: u32, dpi: u32) -> (f32, bool) {
        let width = as_f32(width);
        if self.target == 0.0 {
            self.from = width;
            self.target = width;
            return (width, false);
        }
        if width > self.target {
            self.from = (width - scaled(10.0, dpi)).max(self.target);
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
        let value = progress(now, started, CHROME_DURATION);
        if value >= 1.0 {
            self.started = None;
        }
        (
            self.from + (self.target - self.from) * eased(value),
            value < 1.0,
        )
    }
}

#[derive(Default)]
struct ExitAnimator {
    started: Option<Instant>,
}

impl ExitAnimator {
    fn sample<Asset>(&mut self, now: Instant, items: &[LaidOutItem<Asset>]) -> (f32, bool) {
        if !items.iter().any(|item| item.exiting) {
            self.started = None;
            return (1.0, false);
        }
        let started = *self.started.get_or_insert(now);
        let value = progress(now, started, EXIT_DURATION);
        (1.0 - value, value < 1.0)
    }
}

fn progress(now: Instant, started: Instant, duration: Duration) -> f32 {
    (now.saturating_duration_since(started).as_secs_f32() / duration.as_secs_f32())
        .clamp(0.0, 1.0)
}

fn eased(value: f32) -> f32 {
    1.0 - (1.0 - value).powi(3)
}
