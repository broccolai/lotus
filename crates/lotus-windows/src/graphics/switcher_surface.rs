use lotus_ui::embedded_icon::EmbeddedIcon;
use lotus_ui::geometry::NonZeroPhysicalSize;
use lotus_ui::presentation::Presentation;
use windows::Win32::Foundation::HWND;
use windows::core::Error as WindowsError;

use super::composition_surface::{CompositionSurfaceCore, RecoverableSurface};
use super::device::{DeviceLost, GraphicsDevice};
use super::presentation_renderer::{
    PresentationDrawResult, PresentationRenderer, PresentationRendererError,
};
use super::surface::{FrameResult, SurfaceError, SurfaceSize};
use crate::WindowHandle;

pub struct SwitcherCompositionSurface {
    core: CompositionSurfaceCore,
    renderer: PresentationRenderer,
}

impl SwitcherCompositionSurface {
    fn create(
        graphics: &GraphicsDevice,
        hwnd: HWND,
        size: SurfaceSize,
    ) -> Result<Self, SurfaceError> {
        let core = CompositionSurfaceCore::create(graphics, hwnd, size)?;
        let renderer = PresentationRenderer::create(graphics, core.swap_chain())?;
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

    fn render(
        &mut self,
        presentation: &Presentation<EmbeddedIcon>,
    ) -> Result<FrameResult, PresentationRendererError> {
        match self.renderer.draw(presentation)? {
            PresentationDrawResult::Complete => {
                self.core.present()?;
                Ok(FrameResult::Presented {
                    needs_animation: false,
                })
            }
            PresentationDrawResult::RecreateTarget => {
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
        presentation: &Presentation<EmbeddedIcon>,
    ) -> Result<FrameResult, SurfaceError> {
        let Some(surface) = self.0.get_mut() else {
            return Err(SurfaceError::DeviceLost(
                self.loss().expect("switcher surface is lost"),
            ));
        };
        let hwnd = surface.core.hwnd();
        let size = surface.core.size();
        match surface.render(presentation) {
            Ok(frame) => Ok(frame),
            Err(PresentationRendererError::Windows(error)) => {
                self.0.fail(hwnd, size, error)
            }
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
