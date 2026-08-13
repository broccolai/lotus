use lotus_core::dock::DockItem as CoreDockItem;

use super::scene::{DockIcon, DockItem as SceneDockItem, RasterIcon};

pub fn adapt_dock_items_with_native<F>(
    items: &[CoreDockItem],
    mut native_icon: F,
) -> Vec<SceneDockItem>
where
    F: FnMut(usize, &CoreDockItem) -> Option<RasterIcon>,
{
    items
        .iter()
        .enumerate()
        .filter_map(|(source_index, item)| {
            native_icon(source_index, item).map(|icon| {
                SceneDockItem::with_source_index(source_index, DockIcon::Raster(icon))
            })
        })
        .collect()
}

pub fn resolve_icon_with_native<F>(native_icon: F) -> Option<DockIcon>
where
    F: FnOnce() -> Option<RasterIcon>,
{
    native_icon().map(DockIcon::Raster)
}
