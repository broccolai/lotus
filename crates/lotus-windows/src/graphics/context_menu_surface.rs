use lotus_ui::geometry::NonZeroPhysicalSize;
use windows::Win32::Foundation::HWND;
use windows::core::Error as WindowsError;

use super::ContextMenuScene;
use super::composition_surface::{CompositionSurfaceCore, RecoverableSurface};
use super::context_menu_renderer::{
    ContextMenuDrawResult, ContextMenuRenderer, ContextMenuRendererError,
};
use super::device::{DeviceLost, GraphicsDevice};
use super::surface::{FrameResult, SurfaceError, SurfaceSize};
use crate::WindowHandle;

pub struct ContextMenuCompositionSurface {
    core: CompositionSurfaceCore,
    renderer: ContextMenuRenderer,
}

impl ContextMenuCompositionSurface {
    fn create(
        graphics: &GraphicsDevice,
        hwnd: HWND,
        size: SurfaceSize,
    ) -> Result<Self, SurfaceError> {
        let core = CompositionSurfaceCore::create(graphics, hwnd, size)?;
        let renderer = ContextMenuRenderer::create(graphics, core.swap_chain())?;
        Ok(Self { core, renderer })
    }

    fn resize(&mut self, size: SurfaceSize) -> Result<(), WindowsError> {
        if size == self.core.size() {
            return Ok(());
        }
        self.renderer.detach_target();
        self.core.resize_buffers(size)?;
        self.renderer.attach_target(self.core.swap_chain())
    }

    fn render(
        &mut self,
        scene: &ContextMenuScene,
    ) -> Result<FrameResult, ContextMenuRendererError> {
        match self.renderer.draw(self.core.size(), scene)? {
            ContextMenuDrawResult::Complete => {
                self.core.present()?;
                Ok(FrameResult::Presented {
                    needs_animation: false,
                })
            }
            ContextMenuDrawResult::RecreateTarget => {
                self.core.ensure_device_available()?;
                self.renderer.attach_target(self.core.swap_chain())?;
                Ok(FrameResult::TargetRecreated)
            }
        }
    }

    fn commit(&self) -> Result<(), WindowsError> {
        self.core.commit()
    }
}

pub struct ContextMenuCompositionSurfaceState(
    RecoverableSurface<ContextMenuCompositionSurface>,
);

impl ContextMenuCompositionSurfaceState {
    pub fn create(
        graphics: &GraphicsDevice,
        window: WindowHandle,
        size: NonZeroPhysicalSize,
    ) -> Result<Self, SurfaceError> {
        let hwnd = window.raw();
        ContextMenuCompositionSurface::create(graphics, hwnd, surface_size(size))
            .map(|surface| Self(RecoverableSurface::ready(surface)))
    }

    pub fn resize(&mut self, size: NonZeroPhysicalSize) -> Result<(), SurfaceError> {
        let size = surface_size(size);
        if self.0.remember_resize(size) {
            return Ok(());
        }
        let surface = self.0.get_mut().expect("surface is ready");
        let hwnd = surface.core.hwnd();
        if let Err(error) = surface.resize(size) {
            return self.0.fail(hwnd, size, error);
        }
        Ok(())
    }

    pub fn render_scene(
        &mut self,
        scene: &ContextMenuScene,
    ) -> Result<FrameResult, SurfaceError> {
        let Some(surface) = self.0.get_mut() else {
            return Err(SurfaceError::DeviceLost(
                self.loss()
                    .expect("context menu surface is known to be lost"),
            ));
        };
        let hwnd = surface.core.hwnd();
        let size = surface.core.size();
        match surface.render(scene) {
            Ok(frame) => Ok(frame),
            Err(ContextMenuRendererError::Windows(error)) => self.0.fail(hwnd, size, error),
            Err(error) => Err(error.into()),
        }
    }

    pub fn recover(&mut self, graphics: &GraphicsDevice) -> Result<(), SurfaceError> {
        let Some((hwnd, size)) = self.0.recovery_target() else {
            return Ok(());
        };
        let surface = ContextMenuCompositionSurface::create(graphics, hwnd, size)?;
        surface.commit()?;
        self.0.replace(surface);
        Ok(())
    }

    const fn loss(&self) -> Option<DeviceLost> {
        self.0.loss()
    }
}

impl From<ContextMenuRendererError> for SurfaceError {
    fn from(error: ContextMenuRendererError) -> Self {
        match error {
            ContextMenuRendererError::Asset(error) => Self::Asset(error),
            ContextMenuRendererError::BitmapCacheInvariant => Self::BitmapCacheInvariant,
            ContextMenuRendererError::Windows(error) => Self::from(error),
        }
    }
}

fn surface_size(size: NonZeroPhysicalSize) -> SurfaceSize {
    SurfaceSize::new(size.width(), size.height()).expect("context menu size is nonzero")
}
