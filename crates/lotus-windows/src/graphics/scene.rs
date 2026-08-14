pub use lotus_dock::scene::{
    DockBadge, DockDragState, DockHitTarget, DockInteractionState, DockMetrics, DockSize,
    PixelRect, RasterIcon, RasterIconId, SystemStatusKind,
};

use super::assets::SvgAsset;

pub type DockIcon = lotus_dock::scene::DockIcon<SvgAsset>;
pub type DockItem = lotus_dock::scene::DockItem<SvgAsset>;
pub type DockScene = lotus_dock::scene::DockScene<SvgAsset>;
pub type LaidOutItem = lotus_dock::scene::LaidOutItem<SvgAsset>;
pub type LaidOutStatusItem = lotus_dock::scene::LaidOutStatusItem<SvgAsset>;
pub type SystemStatusItem = lotus_dock::scene::SystemStatusItem<SvgAsset>;
pub type DockLayout = lotus_dock::scene::DockLayout<SvgAsset>;
