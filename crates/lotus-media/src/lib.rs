use lotus_ui::geometry::{DpiScale, PhysicalRect, physical_rect};

const ARTWORK_DIPS: u32 = 40;
const CONTROL_DIPS: u32 = 28;
const CONTROL_GAP_DIPS: u32 = 4;
const INNER_GAP_DIPS: u32 = 8;
const METADATA_DIPS: u32 = 120;
const PADDING_DIPS: u32 = 8;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PlaybackState {
    Playing,
    Paused,
    #[default]
    Stopped,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MediaControls {
    pub previous: bool,
    pub play: bool,
    pub pause: bool,
    pub next: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MediaSnapshot {
    pub source_id: String,
    pub title: String,
    pub artist: String,
    pub artwork: Option<Vec<u8>>,
    pub playback: PlaybackState,
    pub controls: MediaControls,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaWidgetAction {
    FocusSource,
    Previous,
    Play,
    Pause,
    Next,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaHitTarget {
    Metadata,
    Previous,
    PlayPause,
    Next,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaWidgetLayout {
    pub artwork: PhysicalRect,
    pub metadata: PhysicalRect,
    pub previous: PhysicalRect,
    pub play_pause: PhysicalRect,
    pub next: PhysicalRect,
    pub width: u32,
    pub height: u32,
}

impl MediaWidgetLayout {
    pub fn new(dpi: u32, height_dips: u32, show_metadata: bool) -> Option<Self> {
        let scale = DpiScale::new(dpi)?;
        let artwork_dips = ARTWORK_DIPS.min(height_dips.saturating_sub(PADDING_DIPS * 2));
        let height = scale.physical(height_dips);
        let padding = scale.physical(PADDING_DIPS);
        let artwork = scale.physical(artwork_dips);
        let gap = scale.physical(INNER_GAP_DIPS);
        let metadata = show_metadata.then(|| scale.physical(METADATA_DIPS));
        let control = scale.physical(CONTROL_DIPS);
        let control_gap = scale.physical(CONTROL_GAP_DIPS);
        let top = height.saturating_sub(artwork).saturating_div(2);

        let artwork_bounds = physical_rect(padding, top, artwork, artwork);
        let metadata_left = padding.saturating_add(artwork).saturating_add(gap);
        let metadata_bounds = physical_rect(
            metadata_left,
            padding,
            metadata.unwrap_or_default(),
            height - padding * 2,
        );
        let previous_left = metadata.map_or(metadata_left, |metadata| {
            metadata_left.saturating_add(metadata).saturating_add(gap)
        });
        let controls_top = height.saturating_sub(control).saturating_div(2);
        let previous = physical_rect(previous_left, controls_top, control, control);
        let play_left = previous_left
            .saturating_add(control)
            .saturating_add(control_gap);
        let play_pause = physical_rect(play_left, controls_top, control, control);
        let next_left = play_left
            .saturating_add(control)
            .saturating_add(control_gap);
        let next = physical_rect(next_left, controls_top, control, control);
        let width = next_left.saturating_add(control).saturating_add(padding);

        Some(Self {
            artwork: artwork_bounds,
            metadata: metadata_bounds,
            previous,
            play_pause,
            next,
            width,
            height,
        })
    }

    pub fn hit_test(self, x: u32, y: u32) -> Option<MediaHitTarget> {
        let point = lotus_ui::geometry::PhysicalUnsignedPoint::new(x, y);
        if self.artwork.contains(point) || self.metadata.contains(point) {
            Some(MediaHitTarget::Metadata)
        } else if self.previous.contains(point) {
            Some(MediaHitTarget::Previous)
        } else if self.play_pause.contains(point) {
            Some(MediaHitTarget::PlayPause)
        } else if self.next.contains(point) {
            Some(MediaHitTarget::Next)
        } else {
            None
        }
    }
}

#[derive(Default)]
pub struct MediaModel {
    snapshot: Option<MediaSnapshot>,
}

impl MediaModel {
    pub fn snapshot(&self) -> Option<&MediaSnapshot> {
        self.snapshot.as_ref()
    }

    pub fn replace(&mut self, snapshot: Option<MediaSnapshot>) -> bool {
        if self.snapshot == snapshot {
            return false;
        }
        self.snapshot = snapshot;
        true
    }

    pub fn action(&self, target: MediaHitTarget) -> Option<MediaWidgetAction> {
        let snapshot = self.snapshot.as_ref()?;
        match target {
            MediaHitTarget::Metadata if !snapshot.source_id.is_empty() => {
                Some(MediaWidgetAction::FocusSource)
            }
            MediaHitTarget::Previous if snapshot.controls.previous => {
                Some(MediaWidgetAction::Previous)
            }
            MediaHitTarget::PlayPause
                if snapshot.playback == PlaybackState::Playing
                    && snapshot.controls.pause =>
            {
                Some(MediaWidgetAction::Pause)
            }
            MediaHitTarget::PlayPause if snapshot.controls.play => {
                Some(MediaWidgetAction::Play)
            }
            MediaHitTarget::Next if snapshot.controls.next => Some(MediaWidgetAction::Next),
            _ => None,
        }
    }
}
