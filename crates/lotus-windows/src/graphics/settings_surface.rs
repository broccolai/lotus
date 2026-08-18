use windows::Win32::Foundation::HWND;
use windows::core::Error as WindowsError;

use super::composition_surface::{CompositionSurfaceCore, RecoverableSurface};
use super::device::{DeviceLost, GraphicsDevice};
use super::settings_renderer::{
    SettingsDrawResult, SettingsRenderer, SettingsRendererError,
};
use super::surface::{FrameResult, SurfaceError, SurfaceSize};
use super::{SettingsScene, SettingsSize};
use crate::WindowHandle;

pub struct SettingsCompositionSurface {
    core: CompositionSurfaceCore,
    renderer: SettingsRenderer,
}

impl SettingsCompositionSurface {
    fn create(
        graphics: &GraphicsDevice,
        hwnd: HWND,
        size: SurfaceSize,
    ) -> Result<Self, SurfaceError> {
        let core = CompositionSurfaceCore::create(graphics, hwnd, size)?;
        let renderer = SettingsRenderer::create(graphics, core.swap_chain())?;
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
        scene: &SettingsScene,
    ) -> Result<FrameResult, SettingsRendererError> {
        match self.renderer.draw(self.core.size(), scene)? {
            SettingsDrawResult::Complete => {
                self.core.present()?;
                Ok(FrameResult::Presented {
                    needs_animation: false,
                })
            }
            SettingsDrawResult::RecreateTarget => {
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

pub struct SettingsCompositionSurfaceState(RecoverableSurface<SettingsCompositionSurface>);

impl SettingsCompositionSurfaceState {
    pub fn create(
        graphics: &GraphicsDevice,
        window: WindowHandle,
        size: SettingsSize,
    ) -> Result<Self, SurfaceError> {
        let hwnd = window.raw();
        SettingsCompositionSurface::create(graphics, hwnd, surface_size(size))
            .map(|surface| Self(RecoverableSurface::ready(surface)))
    }

    pub fn resize(&mut self, size: SettingsSize) -> Result<(), SurfaceError> {
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
        scene: &SettingsScene,
    ) -> Result<FrameResult, SurfaceError> {
        let Some(surface) = self.0.get_mut() else {
            return Err(SurfaceError::DeviceLost(
                self.loss().expect("settings surface is known to be lost"),
            ));
        };
        let hwnd = surface.core.hwnd();
        let size = surface.core.size();
        match surface.render(scene) {
            Ok(frame) => Ok(frame),
            Err(SettingsRendererError::Windows(error)) => self.0.fail(hwnd, size, error),
        }
    }

    pub fn recover(&mut self, graphics: &GraphicsDevice) -> Result<(), SurfaceError> {
        let Some((hwnd, size)) = self.0.recovery_target() else {
            return Ok(());
        };
        let surface = SettingsCompositionSurface::create(graphics, hwnd, size)?;
        surface.commit()?;
        self.0.replace(surface);
        Ok(())
    }

    pub const fn loss(&self) -> Option<DeviceLost> {
        self.0.loss()
    }
}

impl From<SettingsRendererError> for SurfaceError {
    fn from(error: SettingsRendererError) -> Self {
        match error {
            SettingsRendererError::Windows(error) => Self::from(error),
        }
    }
}

fn surface_size(size: SettingsSize) -> SurfaceSize {
    SurfaceSize::new(size.width(), size.height())
        .expect("settings size is guaranteed nonzero")
}
