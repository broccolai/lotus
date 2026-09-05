use lotus_ui::embedded_icon::EmbeddedIcon;
use lotus_ui::presentation::Presentation;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::DirectComposition::{
    IDCompositionEffectGroup, IDCompositionScaleTransform,
};
use windows::core::Error as WindowsError;

use super::composition_surface::{CompositionSurfaceCore, RecoverableSurface};
use super::device::{DeviceLost, GraphicsDevice};
use super::presentation_renderer::{
    PresentationDrawResult, PresentationRenderer, PresentationRendererError,
};
use super::surface::{FrameResult, SurfaceError, SurfaceSize};
use crate::WindowHandle;

pub struct LauncherCompositionSurface {
    core: CompositionSurfaceCore,
    renderer: PresentationRenderer,
    scale: IDCompositionScaleTransform,
    effect: IDCompositionEffectGroup,
}

impl LauncherCompositionSurface {
    fn create(
        graphics: &GraphicsDevice,
        hwnd: HWND,
        size: SurfaceSize,
    ) -> Result<Self, SurfaceError> {
        let core = CompositionSurfaceCore::create(graphics, hwnd, size)?;
        let scale = unsafe { core.composition_device().CreateScaleTransform()? };
        let effect = unsafe { core.composition_device().CreateEffectGroup()? };
        unsafe {
            core.visual().SetTransform(&scale)?;
            core.visual().SetEffect(&effect)?;
        }
        core.commit()?;
        let renderer = PresentationRenderer::create(graphics, core.swap_chain())?;
        Ok(Self {
            core,
            renderer,
            scale,
            effect,
        })
    }

    fn resize(&mut self, size: SurfaceSize) -> Result<(), WindowsError> {
        if size == self.core.size() {
            return if self.renderer.is_target_attached() {
                Ok(())
            } else {
                self.renderer.attach_target(self.core.swap_chain())
            };
        }
        self.renderer.detach_target();
        self.core.resize_buffers(size)?;
        self.renderer.attach_target(self.core.swap_chain())
    }

    fn render(
        &mut self,
        presentation: &Presentation<EmbeddedIcon>,
        scale: f32,
        opacity: f32,
        needs_animation: bool,
    ) -> Result<FrameResult, PresentationRendererError> {
        if !self.renderer.is_target_attached() {
            self.renderer.attach_target(self.core.swap_chain())?;
        }
        let center_x = as_f32(self.core.size().width()) * 0.5;
        let center_y = as_f32(self.core.size().height()) * 0.08;
        unsafe {
            self.scale.SetCenterX2(center_x)?;
            self.scale.SetCenterY2(center_y)?;
            self.scale.SetScaleX2(scale)?;
            self.scale.SetScaleY2(scale)?;
            self.effect.SetOpacity2(opacity)?;
            self.core.composition_device().Commit()?;
        }
        match self.renderer.draw(presentation)? {
            PresentationDrawResult::Complete => {
                self.core.present()?;
                Ok(FrameResult::Presented { needs_animation })
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

#[allow(
    clippy::cast_precision_loss,
    reason = "surface dimensions stay below f32 exact range"
)]
const fn as_f32(value: u32) -> f32 {
    value as f32
}

pub struct LauncherCompositionSurfaceState(RecoverableSurface<LauncherCompositionSurface>);

impl LauncherCompositionSurfaceState {
    pub fn create(
        graphics: &GraphicsDevice,
        window: WindowHandle,
        size: SurfaceSize,
    ) -> Result<Self, SurfaceError> {
        let hwnd = window.raw();
        LauncherCompositionSurface::create(graphics, hwnd, size)
            .map(|surface| Self(RecoverableSurface::ready(surface)))
    }

    pub fn resize(&mut self, size: SurfaceSize) -> Result<(), SurfaceError> {
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
        scale: f32,
        opacity: f32,
        needs_animation: bool,
    ) -> Result<FrameResult, SurfaceError> {
        let Some(surface) = self.0.get_mut() else {
            return Err(SurfaceError::DeviceLost(
                self.loss().expect("launcher surface is known to be lost"),
            ));
        };
        let hwnd = surface.core.hwnd();
        let size = surface.core.size();
        match surface.render(presentation, scale, opacity, needs_animation) {
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
        let surface = LauncherCompositionSurface::create(graphics, hwnd, size)?;
        surface.commit()?;
        self.0.replace(surface);
        Ok(())
    }

    pub const fn loss(&self) -> Option<DeviceLost> {
        self.0.loss()
    }
}
