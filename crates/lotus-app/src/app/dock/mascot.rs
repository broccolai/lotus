use std::path::Path;
use std::time::Duration;

use lotus_ui::embedded_icon::EmbeddedIcon;
use lotus_ui::icon::RasterIcon;
use lotus_windows::custom_image::{MascotAnimation, MascotLoopCount, load_mascot_image};

use crate::app::visuals::DockIcon;

struct MascotPlayback {
    animation: MascotAnimation,
    frame_index: usize,
    completed_loops: u32,
}

pub(super) struct Mascot {
    initial_icon: DockIcon,
    playback: Option<MascotPlayback>,
}

impl Default for Mascot {
    fn default() -> Self {
        Self {
            initial_icon: DockIcon::Embedded(EmbeddedIcon::LotusPixel),
            playback: None,
        }
    }
}

impl Mascot {
    pub(super) fn load(path: Option<&str>) -> Self {
        let Some(mascot) = path
            .map(Path::new)
            .and_then(|path| load_mascot_image(path).ok())
        else {
            return Self::default();
        };
        Self {
            initial_icon: DockIcon::Raster(mascot.icon),
            playback: mascot.animation.map(|animation| MascotPlayback {
                animation,
                frame_index: 0,
                completed_loops: 0,
            }),
        }
    }

    pub(super) fn initial_icon(&self) -> DockIcon {
        self.initial_icon.clone()
    }

    pub(super) fn delay(&self) -> Option<Duration> {
        self.playback
            .as_ref()
            .map(|playback| playback.animation.frames[playback.frame_index].delay)
    }

    pub(super) fn next_frame(&mut self) -> Option<RasterIcon> {
        let playback = self.playback.as_mut()?;
        let next = playback.frame_index + 1;
        if next < playback.animation.frames.len() {
            playback.frame_index = next;
        } else {
            let completed = playback.completed_loops.saturating_add(1);
            if matches!(playback.animation.loop_count, MascotLoopCount::Finite(count) if completed >= count)
            {
                self.playback = None;
                return None;
            }
            playback.frame_index = 0;
            playback.completed_loops = completed;
        }

        Some(playback.animation.frames[playback.frame_index].icon.clone())
    }
}
