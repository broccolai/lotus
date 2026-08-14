use lotus_core::settings::DockSettings;
use lotus_settings::appearance::theme_for;
use lotus_ui::geometry::NonZeroPhysicalSize;

use super::dock::{dock_anchor, metrics, status_items};
use super::{
    AppError, CompositionSurfaceState, DeviceState, DockHitTarget, DockIcon, DockScene,
    StatusWindow, SurfaceSize, SvgAsset, SystemStatusKind, WindowEvent, render_surface,
    resize_surface,
};

pub(super) struct StatusRuntime {
    pub(super) window: StatusWindow,
    surface: Option<CompositionSurfaceState>,
    scene: DockScene,
    external: bool,
}

impl StatusRuntime {
    pub(super) fn new(
        window: StatusWindow,
        settings: &DockSettings,
    ) -> Result<Self, AppError> {
        let mut scene = status_scene(window.dpi(), settings)?;
        scene.replace_status_items(Vec::new());
        Ok(Self {
            window,
            surface: None,
            scene,
            external: false,
        })
    }

    pub(super) fn sync(
        &mut self,
        dock: &super::DockWindow,
        settings: &DockSettings,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let items = status_items(settings);
        self.external =
            settings.system_status_zone != settings.dock_zone && !items.is_empty();
        if !self.external {
            self.window.set_visible(false);
            return Ok(());
        }

        self.scene = build_status_scene(self.window.dpi(), settings, items)?;
        let size = self.scene.desired_size();
        let physical = NonZeroPhysicalSize::new(size.width(), size.height())
            .ok_or(AppError::ZeroSizedSurface)?;
        dock.place_status_window(
            &self.window,
            physical,
            settings.system_status_zone,
            settings,
        )?;
        lotus_windows::backdrop::apply_dock_settings(self.window.handle(), settings);

        if let Some(surface) = &mut self.surface {
            resize_surface(graphics, surface, SurfaceSize::from(size))?;
        } else {
            let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
            self.surface = Some(CompositionSurfaceState::create(
                device,
                self.window.handle(),
                SurfaceSize::from(size),
            )?);
        }
        self.render(graphics)?;
        self.window.set_visible(dock.is_visible());
        Ok(())
    }

    pub(super) fn set_visible(&self, dock_visible: bool) {
        self.window.set_visible(self.external && dock_visible);
    }

    pub(super) fn refresh(
        &mut self,
        settings: &DockSettings,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if !self.external {
            return Ok(());
        }
        let next = status_items(settings);
        if self.scene.status_items() != next {
            self.scene.replace_status_items(next);
            self.render(graphics)?;
        }
        Ok(())
    }

    pub(super) fn drain_events(&mut self) -> Vec<WindowEvent> {
        self.window.drain_events().collect()
    }

    pub(super) fn handle_event(
        &mut self,
        event: WindowEvent,
        graphics: &mut DeviceState,
    ) -> Result<Option<SystemStatusKind>, AppError> {
        match event {
            WindowEvent::Pointer(pointer) => {
                let target = match pointer {
                    super::PointerEvent::Moved { x, y } => {
                        let target = self.hit_test(x, y);
                        let _ = self.scene.set_hovered(target);
                        None
                    }
                    super::PointerEvent::Left => {
                        let _ = self.scene.set_hovered(None);
                        None
                    }
                    super::PointerEvent::LeftButtonPressed { x, y } => {
                        let target = self.hit_test(x, y);
                        let _ = self.scene.set_pressed(target);
                        None
                    }
                    super::PointerEvent::LeftButtonReleased { x, y } => {
                        let target = self.hit_test(x, y);
                        let pressed = self.scene.interaction().pressed;
                        let _ = self.scene.set_pressed(None);
                        (pressed == target).then_some(target).flatten()
                    }
                    super::PointerEvent::Cancelled => {
                        let _ = self.scene.set_pressed(None);
                        None
                    }
                };
                self.render(graphics)?;
                Ok(target.and_then(system_status_kind))
            }
            WindowEvent::Resized { width, height } => {
                if let (Some(surface), Some(size)) =
                    (&mut self.surface, SurfaceSize::new(width, height))
                {
                    resize_surface(graphics, surface, size)?;
                    self.render(graphics)?;
                }
                Ok(None)
            }
            WindowEvent::RenderRequested | WindowEvent::AnimationFrame => {
                self.render(graphics)?;
                Ok(None)
            }
            WindowEvent::DpiChanged { .. }
            | WindowEvent::PlacementRefreshRequested
            | WindowEvent::ContextMenuRequested(_)
            | WindowEvent::Search(_)
            | WindowEvent::Settings(_)
            | WindowEvent::ContextMenu(_)
            | WindowEvent::Switcher(_)
            | WindowEvent::StatusRefreshRequested => Ok(None),
        }
    }

    fn hit_test(&self, x: i32, y: i32) -> Option<DockHitTarget> {
        let x = u32::try_from(x).ok()?;
        let y = u32::try_from(y).ok()?;
        let size = self.scene.desired_size();
        self.scene
            .layout(size.width(), size.height())
            .hit_test(x, y)
    }

    fn render(&mut self, graphics: &mut DeviceState) -> Result<(), AppError> {
        if let Some(surface) = &mut self.surface {
            let needs_animation = render_surface(graphics, surface, &self.scene)?;
            self.window.set_animation_active(needs_animation)?;
        }
        Ok(())
    }
}

fn status_scene(dpi: u32, settings: &DockSettings) -> Result<DockScene, AppError> {
    build_status_scene(dpi, settings, status_items(settings))
}

fn build_status_scene(
    dpi: u32,
    settings: &DockSettings,
    items: Vec<super::SystemStatusItem>,
) -> Result<DockScene, AppError> {
    let mut scene = DockScene::new(
        dpi,
        metrics(settings)?,
        DockIcon::Embedded(SvgAsset::LotusPixel),
        Vec::new(),
    )
    .ok_or(AppError::InvalidScene)?;
    scene.set_anchor(dock_anchor(settings.system_status_zone));
    scene.set_launcher_button_visible(false);
    scene.replace_status_items(items);
    let _ = scene.set_theme(theme_for(settings));
    Ok(scene)
}

fn system_status_kind(target: DockHitTarget) -> Option<SystemStatusKind> {
    match target {
        DockHitTarget::SystemStatus(kind) => Some(kind),
        DockHitTarget::Item(_) | DockHitTarget::Jirachi | DockHitTarget::ShowDesktop => {
            None
        }
    }
}
