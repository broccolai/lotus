use lotus_ui::geometry::NonZeroPhysicalSize;
use windows::Win32::Foundation::{E_FAIL, HWND};
use windows::core::Error as WindowsError;

use super::SwitcherScene;
use super::composition_surface::{CompositionSurfaceCore, RecoverableSurface};
use super::device::{DeviceLost, GraphicsDevice};
use super::surface::{FrameResult, SurfaceError, SurfaceSize};
use super::switcher_renderer::{DrawResult, RendererError, SwitcherRenderer};
use crate::WindowHandle;

impl From<RendererError> for SurfaceError {
    fn from(error: RendererError) -> Self {
        match error {
            RendererError::Windows(error) => Self::from(error),
            RendererError::Asset(error) => Self::from(error),
            RendererError::BitmapCacheInvariant => Self::from(WindowsError::new(
                E_FAIL,
                "switcher bitmap cache invariant failed",
            )),
        }
    }
}

pub struct SwitcherCompositionSurface {
    core: CompositionSurfaceCore,
    renderer: SwitcherRenderer,
}

impl SwitcherCompositionSurface {
    fn create(
        graphics: &GraphicsDevice,
        hwnd: HWND,
        size: SurfaceSize,
    ) -> Result<Self, SurfaceError> {
        let core = CompositionSurfaceCore::create(graphics, hwnd, size)?;
        let renderer = SwitcherRenderer::create(graphics, core.swap_chain())?;
        Ok(Self { core, renderer })
    }

    fn resize(&mut self, size: SurfaceSize) -> Result<(), WindowsError> {
        if self.core.size() == size {
            return Ok(());
        }
        self.renderer.detach_target();
        self.core.resize_buffers(size)?;
        self.renderer.attach_target(self.core.swap_chain())
    }

    fn render(&mut self, scene: &SwitcherScene) -> Result<FrameResult, RendererError> {
        match self.renderer.draw(self.core.size(), scene)? {
            DrawResult::Complete => {
                self.core.present()?;
                Ok(FrameResult::Presented {
                    needs_animation: false,
                })
            }
            DrawResult::RecreateTarget => {
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

pub struct SwitcherCompositionSurfaceState(RecoverableSurface<SwitcherCompositionSurface>);

impl SwitcherCompositionSurfaceState {
    pub fn create(
        graphics: &GraphicsDevice,
        window: WindowHandle,
        size: NonZeroPhysicalSize,
    ) -> Result<Self, SurfaceError> {
        SwitcherCompositionSurface::create(graphics, window.raw(), surface_size(size))
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
        scene: &SwitcherScene,
    ) -> Result<FrameResult, SurfaceError> {
        let Some(surface) = self.0.get_mut() else {
            return Err(SurfaceError::DeviceLost(
                self.loss().expect("switcher surface is lost"),
            ));
        };
        let hwnd = surface.core.hwnd();
        let size = surface.core.size();
        match surface.render(scene) {
            Ok(frame) => Ok(frame),
            Err(RendererError::Windows(error)) => self.0.fail(hwnd, size, error),
            Err(error) => Err(error.into()),
        }
    }

    pub fn recover(&mut self, graphics: &GraphicsDevice) -> Result<(), SurfaceError> {
        let Some((hwnd, size)) = self.0.recovery_target() else {
            return Ok(());
        };
        let surface = SwitcherCompositionSurface::create(graphics, hwnd, size)?;
        surface.commit()?;
        self.0.replace(surface);
        Ok(())
    }

    const fn loss(&self) -> Option<DeviceLost> {
        self.0.loss()
    }
}

fn surface_size(size: NonZeroPhysicalSize) -> SurfaceSize {
    SurfaceSize::new(size.width(), size.height()).expect("switcher size is nonzero")
}
