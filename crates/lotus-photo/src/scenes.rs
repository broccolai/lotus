use lotus_core::window::{TrackedWindowKey, WindowId};
use lotus_dock::scene::{DockItem, DockMetrics, DockPresenter, DockScene};
use lotus_search::scene::{LauncherResult, LauncherScene};
use lotus_switcher::scene::{SwitcherItem, SwitcherScene};
use lotus_ui::embedded_icon::EmbeddedIcon;
use lotus_ui::icon::Icon;
use lotus_ui::presentation::{Presentation, PresentationPrimitive};
use lotus_windows::graphics::SurfaceSize;
use lotus_windows::native_icon::NativeIconCache;
use thiserror::Error;

use crate::config::{PhotoScene, SceneKind};
use crate::icons::{self, IconError};

pub struct RenderedScene {
    pub presentation: Presentation<EmbeddedIcon>,
    pub size: SurfaceSize,
}

pub fn build(photo: &PhotoScene) -> Result<RenderedScene, SceneError> {
    let mut cache = NativeIconCache::default();
    let mut output = match photo.kind {
        SceneKind::Dock => dock(photo, &mut cache),
        SceneKind::Search => search(photo, &mut cache),
        SceneKind::Switcher => switcher(photo, &mut cache),
    }?;
    scale_capture(
        &mut output.presentation,
        f32::from(u16::try_from(photo.dpi).unwrap()) / 96.0,
    );
    output.size = surface(
        output.size.width() * photo.dpi / 96,
        output.size.height() * photo.dpi / 96,
    )?;
    Ok(output)
}

fn app_icons(
    photo: &PhotoScene,
    cache: &mut NativeIconCache,
    size: u32,
) -> Result<Vec<Icon<EmbeddedIcon>>, SceneError> {
    photo
        .apps
        .iter()
        .map(|app| icons::load(cache, &app.name, &app.path, size).map_err(SceneError::Icon))
        .collect()
}

fn dock(
    photo: &PhotoScene,
    cache: &mut NativeIconCache,
) -> Result<RenderedScene, SceneError> {
    let items = app_icons(photo, cache, icon_size(photo.dpi, 38))?
        .into_iter()
        .enumerate()
        .map(|(index, icon)| {
            let mut item = DockItem::with_source_index(index, icon);
            item.set_running(true);
            item
        })
        .collect();
    let mascot = Icon::Embedded(EmbeddedIcon::LotusPixel);
    let scene = DockScene::new(96, DockMetrics::defaults(), mascot, items)
        .ok_or(SceneError::InvalidScene("dock DPI"))?;
    let size = scene.desired_size();
    let (presentation, _) =
        DockPresenter::default().present(&scene, size.width(), size.height());
    Ok(RenderedScene {
        presentation,
        size: surface(size.width(), size.height())?,
    })
}

fn search(
    photo: &PhotoScene,
    cache: &mut NativeIconCache,
) -> Result<RenderedScene, SceneError> {
    let results = app_icons(photo, cache, icon_size(photo.dpi, 26))?
        .into_iter()
        .zip(&photo.apps)
        .map(|(icon, app)| LauncherResult::with_icon(&app.name, icon))
        .collect();
    let mut scene = LauncherScene::new(
        96,
        &photo.query,
        lotus_search::controller::SearchMode::Applications,
        results,
        photo.selected,
    )
    .ok_or(SceneError::InvalidScene("search DPI"))?;
    scene.set_footer_time("9:41 AM");
    let size = scene.desired_size();
    Ok(RenderedScene {
        presentation: scene.render_presentation(EmbeddedIcon::FluentSearch),
        size: surface(size.width(), size.height())?,
    })
}

fn switcher(
    photo: &PhotoScene,
    cache: &mut NativeIconCache,
) -> Result<RenderedScene, SceneError> {
    let selected = photo.selected.unwrap_or(0);
    let items = app_icons(photo, cache, icon_size(photo.dpi, 38))?
        .into_iter()
        .zip(&photo.apps)
        .enumerate()
        .map(|(index, (icon, app))| SwitcherItem {
            key: TrackedWindowKey {
                id: WindowId::new(index as u64 + 1),
                process_id: 0,
                incarnation: 0,
            },
            title: app.name.clone(),
            icon: Some(icon),
        })
        .collect();
    let scene = SwitcherScene::new(96, items, selected)
        .ok_or(SceneError::InvalidScene("switcher selected index or DPI"))?;
    let size = scene.desired_size();
    Ok(RenderedScene {
        presentation: scene.presentation(EmbeddedIcon::FluentDismiss),
        size: surface(size.width(), size.height())?,
    })
}

fn surface(width: u32, height: u32) -> Result<SurfaceSize, SceneError> {
    SurfaceSize::new(width, height).ok_or(SceneError::InvalidScene("zero surface size"))
}

fn icon_size(dpi: u32, dips: u32) -> u32 {
    (dpi * dips * 2 / 96).clamp(1, 1024)
}

// Scale the completed 96-DPI scene uniformly so text and geometry stay in proportion.
fn scale_capture(presentation: &mut Presentation<EmbeddedIcon>, scale: f32) {
    for primitive in &mut presentation.primitives {
        let bounds = match primitive {
            PresentationPrimitive::PopClip => continue,
            PresentationPrimitive::PushClip { bounds } => bounds,
            PresentationPrimitive::FillRoundedRect { bounds, radius, .. }
            | PresentationPrimitive::Icon { bounds, radius, .. } => {
                *radius *= scale;
                bounds
            }
            PresentationPrimitive::StrokeRoundedRect {
                bounds,
                radius,
                width,
                ..
            } => {
                *radius *= scale;
                *width *= scale;
                bounds
            }
            PresentationPrimitive::Text { bounds, style, .. } => {
                style.size *= scale;
                bounds
            }
            PresentationPrimitive::TextCaret {
                bounds,
                style,
                top_inset,
                bottom_inset,
                width,
                ..
            } => {
                style.size *= scale;
                *top_inset *= scale;
                *bottom_inset *= scale;
                *width *= scale;
                bounds
            }
        };
        bounds.left *= scale;
        bounds.top *= scale;
        bounds.right *= scale;
        bounds.bottom *= scale;
    }
}

#[derive(Debug, Error)]
pub enum SceneError {
    #[error("{0} is invalid")]
    InvalidScene(&'static str),
    #[error(transparent)]
    Icon(#[from] IconError),
}
