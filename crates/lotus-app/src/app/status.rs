mod zone;

use lotus_core::settings::{DockSettings, DockZone};
use lotus_media::MediaHitTarget;
use lotus_ui::frame::FramePass;
use lotus_windows::WindowHandle;
use lotus_windows::graphics::{DeviceState, GraphicsDevice};
use lotus_windows::window::{DockWindow, SignedPoint, StatusEvent, StatusWindow};

use self::zone::StatusZone;
use crate::app::AppError;
use crate::app::visuals::{MediaItem, SystemStatusKind};

pub(super) enum AuxiliaryZoneAction {
    Media(MediaHitTarget),
    Status(SystemStatusKind),
}

pub(super) struct StatusRuntime {
    zones: Vec<StatusZone>,
}

impl StatusRuntime {
    pub(super) fn diagnostic_surface_masks(&self) -> (bool, bool, bool) {
        self.zones
            .iter()
            .fold((false, false, false), |state, zone| {
                let masks = zone.diagnostic_state();
                (state.0 || masks.0, state.1 || masks.1, state.2 || masks.2)
            })
    }

    pub(super) fn new(
        windows: [StatusWindow; 2],
        settings: &DockSettings,
    ) -> Result<Self, AppError> {
        let zones = windows
            .into_iter()
            .map(|window| StatusZone::new(window, settings))
            .collect::<Result<Vec<_>, AppError>>()?;
        Ok(Self { zones })
    }

    pub(super) fn sync(
        &mut self,
        dock: &DockWindow,
        settings: &DockSettings,
        media: Option<&MediaItem>,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let occupied = occupied_external_zones(settings, media);
        for (index, zone) in self.zones.iter_mut().enumerate() {
            zone.sync(
                dock,
                occupied.get(index).copied(),
                settings,
                media,
                graphics,
            )?;
        }
        Ok(())
    }

    pub(super) fn set_visible(&self, dock_visible: bool) {
        for zone in &self.zones {
            zone.set_visible(dock_visible);
        }
    }

    pub(super) fn set_fullscreen_occluded(
        &mut self,
        occluded: bool,
    ) -> Result<(), AppError> {
        for zone in &mut self.zones {
            zone.set_fullscreen_occluded(occluded)?;
        }
        Ok(())
    }

    pub(super) fn refresh(&mut self, settings: &DockSettings) {
        for zone in &mut self.zones {
            zone.refresh(settings);
        }
    }

    pub(super) fn drain_events_up_to(&mut self, limit: usize) -> Vec<(usize, StatusEvent)> {
        let mut events = Vec::with_capacity(limit);
        for (index, zone) in self.zones.iter_mut().enumerate() {
            let remaining = limit.saturating_sub(events.len());
            if remaining == 0 {
                break;
            }
            events.extend(
                zone.drain_events_up_to(remaining)
                    .map(|event| (index, event)),
            );
        }
        events
    }

    pub(super) fn has_pending_events(&self) -> bool {
        self.zones.iter().any(StatusZone::has_pending_events)
    }

    pub(super) fn handle_event(
        &mut self,
        zone_index: usize,
        event: StatusEvent,
        graphics: &mut DeviceState,
    ) -> Result<Option<(AuxiliaryZoneAction, WindowHandle, Option<SignedPoint>)>, AppError>
    {
        let Some(zone) = self.zones.get_mut(zone_index) else {
            return Ok(None);
        };
        zone.handle_event(event, graphics)
    }

    pub(super) fn render_frame(
        &mut self,
        pass: &mut FramePass,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        for zone in &mut self.zones {
            zone.render_frame(pass, graphics)?;
        }
        Ok(())
    }

    pub(super) fn invalidate(&mut self) {
        for zone in &mut self.zones {
            zone.invalidate();
        }
    }

    pub(super) fn recover_surfaces(
        &mut self,
        device: &GraphicsDevice,
    ) -> Result<(), AppError> {
        for zone in &mut self.zones {
            zone.recover_surface(device)?;
        }
        Ok(())
    }
}

fn occupied_external_zones(
    settings: &DockSettings,
    media: Option<&MediaItem>,
) -> Vec<DockZone> {
    [DockZone::Left, DockZone::Center, DockZone::Right]
        .into_iter()
        .filter(|zone| *zone != settings.dock_zone)
        .filter(|zone| {
            (media.is_some()
                && settings.show_media_controls
                && *zone == settings.media_zone)
                || (settings.show_system_status
                    && *zone == settings.system_status_zone
                    && has_status_items(settings))
        })
        .collect()
}

fn has_status_items(settings: &DockSettings) -> bool {
    settings.show_volume_status
        || settings.show_hdr_status
        || settings.show_network_status
        || settings.show_background_apps_status
        || settings.show_date_time_status
}
