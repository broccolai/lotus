use std::collections::HashMap;
use std::path::Path;

use lotus_core::dock::DockItem;
use lotus_core::settings::DockSettings;
use lotus_media::MediaSnapshot;
use lotus_ui::icon::RasterIcon;
use lotus_windows::custom_image::CustomImageCache;
use lotus_windows::icon_hydrator::{DockIconClient, DockIconRequest, HydratedDockIcon};
use lotus_windows::native_icon::NativeIconCache;
use lotus_windows::search_catalog::ApplicationCatalogSnapshot;

use crate::app::icon_override::resolve_application_icon;
use crate::app::visuals::DockIcon;

#[derive(Default)]
pub(super) struct DockAssets {
    native_icons: NativeIconCache,
    icon_hydrator: Option<DockIconClient>,
    hydrated_window_icons: HashMap<String, HydratedDockIcon>,
    custom_images: CustomImageCache,
}

impl DockAssets {
    pub(super) fn media_artwork(
        &mut self,
        snapshot: &MediaSnapshot,
        items: &[DockItem],
        settings: &DockSettings,
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
        self.preview_icon(settings, item, pixel_size)
            .map(DockIcon::Raster)
    }

    pub(super) fn prepare_icons(
        &mut self,
        items: &[DockItem],
        settings: &DockSettings,
        pixel_size: u32,
    ) -> Vec<Option<RasterIcon>> {
        items
            .iter()
            .map(|item| self.icon_with_hydration(settings, item, pixel_size))
            .collect()
    }

    pub(super) fn picker_icon(
        &mut self,
        settings: &DockSettings,
        item: &DockItem,
        pixel_size: u32,
    ) -> Option<RasterIcon> {
        resolve_application_icon(
            settings,
            &mut self.custom_images,
            item.windows
                .first()
                .and_then(|window| window.application_facts.reliable_id()),
            Some(&item.id),
            Path::new(&item.icon_source),
        )
        .or_else(|| self.file_icon(item, pixel_size))
    }

    pub(super) fn preview_icon(
        &mut self,
        settings: &DockSettings,
        item: &DockItem,
        pixel_size: u32,
    ) -> Option<RasterIcon> {
        self.application_override(settings, item)
            .or_else(|| self.file_icon(item, pixel_size))
    }

    fn icon_with_hydration(
        &mut self,
        settings: &DockSettings,
        item: &DockItem,
        pixel_size: u32,
    ) -> Option<RasterIcon> {
        self.application_override(settings, item)
            .or_else(|| self.hydrated_icon(item, pixel_size))
            .or_else(|| self.file_icon(item, pixel_size))
    }

    fn application_override(
        &mut self,
        settings: &DockSettings,
        item: &DockItem,
    ) -> Option<RasterIcon> {
        resolve_application_icon(
            settings,
            &mut self.custom_images,
            item.app_user_model_id.as_deref(),
            Some(&item.id),
            Path::new(&item.executable_path),
        )
    }

    fn hydrated_icon(&self, item: &DockItem, pixel_size: u32) -> Option<RasterIcon> {
        let window = item.windows.first()?.key();
        let icon = self.hydrated_window_icons.get(&item.id)?;
        if icon.pixel_size != pixel_size || icon.window != window {
            return None;
        }
        icon.icon.clone()
    }

    fn file_icon(&mut self, item: &DockItem, pixel_size: u32) -> Option<RasterIcon> {
        self.native_icons
            .icon(Path::new(&item.icon_source), pixel_size)
            .ok()
            .flatten()
    }

    pub(super) fn drain(
        &mut self,
        items: &[DockItem],
        pixel_size: u32,
        results: impl IntoIterator<Item = HydratedDockIcon>,
    ) -> bool {
        let mut changed = false;
        for result in results {
            let current = items.iter().any(|item| {
                item.id == result.identity
                    && item
                        .windows
                        .first()
                        .is_some_and(|window| window.key() == result.window)
                    && result.pixel_size == pixel_size
            });
            let duplicate = self
                .hydrated_window_icons
                .get(&result.identity)
                .is_some_and(|existing| {
                    existing.window == result.window
                        && existing.pixel_size == result.pixel_size
                        && existing.icon == result.icon
                });
            if current && result.icon.is_some() && !duplicate {
                self.hydrated_window_icons
                    .insert(result.identity.clone(), result);
                changed = true;
            }
        }
        changed
    }

    pub(super) fn retain(&mut self, items: &[DockItem], pixel_size: u32) {
        let current = items
            .iter()
            .filter_map(|item| {
                item.windows
                    .first()
                    .map(|window| (item.id.clone(), window.key()))
            })
            .collect::<HashMap<_, _>>();
        self.hydrated_window_icons.retain(|identity, icon| {
            icon.pixel_size == pixel_size
                && current
                    .get(identity)
                    .is_some_and(|window| *window == icon.window)
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
                let window = item.windows.first()?.key();
                let missing = self.hydrated_window_icons.get(&item.id).is_none_or(|icon| {
                    icon.window != window || icon.pixel_size != pixel_size
                });
                (missing
                    && crate::app::icon_override::application_icon_path_for_identity(
                        settings,
                        &item.application_identity(),
                    )
                    .is_none())
                .then(|| DockIconRequest {
                    identity: item.id.clone(),
                    window,
                    executable_path: item.executable_path.clone().into(),
                    presentation_icon: item.presentation_icon.clone(),
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
        self.custom_images.clear();
    }
}
