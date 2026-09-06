use std::collections::HashMap;

use lotus_core::dock::DockItem;
use lotus_core::settings::DockSettings;
use lotus_media::MediaSnapshot;
use lotus_ui::icon::RasterIcon;
use lotus_windows::icon_hydrator::{DockIconClient, DockIconRequest, HydratedDockIcon};
use lotus_windows::search_catalog::ApplicationCatalogSnapshot;

use crate::app::visuals::DockIcon;

#[derive(Default)]
pub(super) struct DockAssets {
    icon_hydrator: Option<DockIconClient>,
    hydrated_icons: HashMap<String, HydratedDockIcon>,
    settings: Option<DockSettings>,
}

impl DockAssets {
    pub(super) fn media_artwork(
        &mut self,
        snapshot: &MediaSnapshot,
        items: &[DockItem],
        catalog: &ApplicationCatalogSnapshot,
        pixel_size: u32,
    ) -> Option<DockIcon> {
        let artwork = snapshot.artwork.as_deref().and_then(|artwork| {
            lotus_windows::media::decode_artwork(&snapshot.source_id, artwork).ok()
        });
        if let Some(artwork) = artwork {
            return Some(DockIcon::Raster(artwork));
        }

        let source = catalog.key_for_external_identifier(&snapshot.source_id)?;
        let item = items.iter().find(|item| item.application_key == source)?;
        self.preview_icon(item, pixel_size).map(DockIcon::Raster)
    }

    pub(super) fn prepare_icons(
        &mut self,
        items: &[DockItem],
        settings: &DockSettings,
        pixel_size: u32,
    ) -> Vec<Option<RasterIcon>> {
        self.settings = Some(settings.clone());
        items
            .iter()
            .map(|item| self.hydrated_icon(item, settings, pixel_size))
            .collect()
    }

    pub(super) fn picker_icon(
        &mut self,
        item: &DockItem,
        pixel_size: u32,
    ) -> Option<RasterIcon> {
        self.settings
            .as_ref()
            .and_then(|settings| self.hydrated_icon(item, settings, pixel_size))
    }

    pub(super) fn preview_icon(
        &mut self,
        item: &DockItem,
        pixel_size: u32,
    ) -> Option<RasterIcon> {
        self.settings
            .as_ref()
            .and_then(|settings| self.hydrated_icon(item, settings, pixel_size))
    }

    fn hydrated_icon(
        &self,
        item: &DockItem,
        settings: &DockSettings,
        pixel_size: u32,
    ) -> Option<RasterIcon> {
        let window = item
            .windows
            .first()
            .map(lotus_core::window::WindowInfo::key);
        let icon = self.hydrated_icons.get(&item.id)?;
        if !same_subject(item, settings, icon, pixel_size, window) {
            return None;
        }
        icon.icon.clone()
    }

    pub(super) fn drain(
        &mut self,
        items: &[DockItem],
        settings: &DockSettings,
        pixel_size: u32,
        results: impl IntoIterator<Item = HydratedDockIcon>,
    ) -> bool {
        let mut changed = false;
        for result in results {
            let current = items.iter().any(|item| {
                item.id == result.identity
                    && same_subject(item, settings, &result, pixel_size, result.window)
            });
            let duplicate =
                self.hydrated_icons
                    .get(&result.identity)
                    .is_some_and(|existing| {
                        existing.executable_path == result.executable_path
                            && existing.presentation_icon == result.presentation_icon
                            && existing.custom_image_path == result.custom_image_path
                            && existing.window == result.window
                            && existing.pixel_size == result.pixel_size
                            && existing.icon == result.icon
                    });
            if current && result.icon.is_some() && !duplicate {
                self.hydrated_icons.insert(result.identity.clone(), result);
                changed = true;
            }
        }
        changed
    }

    pub(super) fn retain(
        &mut self,
        items: &[DockItem],
        settings: &DockSettings,
        pixel_size: u32,
    ) {
        self.hydrated_icons.retain(|identity, icon| {
            items.iter().any(|item| {
                item.id == *identity
                    && same_subject(item, settings, icon, pixel_size, icon.window)
            })
        });
    }

    pub(super) fn request(
        &self,
        items: &[DockItem],
        settings: &DockSettings,
        pixel_size: u32,
    ) {
        let requests = items
            .iter()
            .filter_map(|item| {
                let window = item
                    .windows
                    .first()
                    .map(lotus_core::window::WindowInfo::key);
                let missing = self.hydrated_icons.get(&item.id).is_none_or(|icon| {
                    !same_subject(item, settings, icon, pixel_size, window)
                });
                missing.then(|| DockIconRequest {
                    identity: item.id.clone(),
                    window,
                    executable_path: item.executable_path.clone().into(),
                    presentation_icon: item.presentation_icon.clone(),
                    custom_image_path:
                        crate::app::icon_override::application_icon_path_for_identity(
                            settings,
                            &item.application_identity(),
                        ),
                    pixel_size,
                })
            })
            .collect();
        if let Some(client) = &self.icon_hydrator {
            client.request_dock(requests);
        }
    }

    pub(super) fn attach(&mut self, client: DockIconClient) {
        self.icon_hydrator = Some(client);
    }

    pub(super) fn clear_custom_images(&mut self) {
        self.hydrated_icons.clear();
    }
}

fn same_subject(
    item: &DockItem,
    settings: &DockSettings,
    icon: &HydratedDockIcon,
    pixel_size: u32,
    window: Option<lotus_core::window::TrackedWindowKey>,
) -> bool {
    icon.identity == item.id
        && icon.window == window
        && icon.executable_path == item.executable_path
        && icon.presentation_icon == item.presentation_icon
        && icon.custom_image_path
            == crate::app::icon_override::application_icon_path_for_identity(
                settings,
                &item.application_identity(),
            )
        && icon.pixel_size == pixel_size
}
