use std::num::NonZeroU32;

use lotus_ui::embedded_icon::EmbeddedIcon;
use lotus_ui::presentation::Presentation;
use thiserror::Error;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
    DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::core::Error as WindowsError;

use super::assets::AssetError;
use super::composition_surface::{CompositionSurfaceCore, RecoverableSurface};
use super::device::{DeviceLost, GraphicsDevice};
use super::presentation_renderer::{
    PresentationDrawResult, PresentationRenderer, PresentationRendererError,
};
use crate::{NativeError, WindowHandle};

const BUFFER_COUNT: u32 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SurfaceSize {
    width: NonZeroU32,
    height: NonZeroU32,
}

impl SurfaceSize {
    pub const fn new(width: u32, height: u32) -> Option<Self> {
        let Some(width) = NonZeroU32::new(width) else {
            return None;
        };
        let Some(height) = NonZeroU32::new(height) else {
            return None;
        };
        Some(Self { width, height })
    }

    pub const fn width(self) -> u32 {
        self.width.get()
    }

    pub const fn height(self) -> u32 {
        self.height.get()
    }
}

pub struct CompositionSurface {
    core: CompositionSurfaceCore,
    renderer: PresentationRenderer,
}

impl CompositionSurface {
    fn create(
        graphics: &GraphicsDevice,
        hwnd: HWND,
        size: SurfaceSize,
    ) -> Result<Self, SurfaceError> {
        let core = CompositionSurfaceCore::create(graphics, hwnd, size)?;
        let renderer = PresentationRenderer::create(graphics, core.swap_chain())?;
        Ok(Self { core, renderer })
    }

    pub const fn size(&self) -> SurfaceSize {
        self.core.size()
    }

    fn resize(&mut self, size: SurfaceSize) -> Result<(), WindowsError> {
        if size == self.core.size() {
            return if self.renderer_is_attached() {
                Ok(())
            } else {
                self.renderer.attach_target(self.core.swap_chain())
            };
        }

        self.renderer.detach_target();
        self.core.resize_buffers(size)?;
        self.renderer.attach_target(self.core.swap_chain())
    }

    fn renderer_is_attached(&self) -> bool {
        self.renderer.is_target_attached()
    }

    fn render(
        &mut self,
        presentation: &Presentation<EmbeddedIcon>,
        needs_animation: bool,
    ) -> Result<FrameResult, PresentationRendererError> {
        if !self.renderer.is_target_attached() {
            self.renderer.attach_target(self.core.swap_chain())?;
        }
        match self.renderer.draw(presentation)? {
            PresentationDrawResult::Complete => {
                self.present()?;
                Ok(FrameResult::Presented { needs_animation })
            }
            PresentationDrawResult::RecreateTarget => {
                self.ensure_device_available()?;
                self.renderer.attach_target(self.core.swap_chain())?;
                Ok(FrameResult::TargetRecreated)
            }
        }
    }

    fn ensure_device_available(&self) -> Result<(), WindowsError> {
        self.core.ensure_device_available()
    }

    fn present(&self) -> Result<(), WindowsError> {
        self.core.present()
    }

    fn commit(&self) -> Result<(), WindowsError> {
        self.core.commit()
    }
}

pub struct CompositionSurfaceState(RecoverableSurface<CompositionSurface>);

impl CompositionSurfaceState {
    pub fn create(
        graphics: &GraphicsDevice,
        window: WindowHandle,
        size: SurfaceSize,
    ) -> Result<Self, SurfaceError> {
        let hwnd = window.raw();
        CompositionSurface::create(graphics, hwnd, size)
            .map(|surface| Self(RecoverableSurface::ready(surface)))
    }

    pub const fn ready(&self) -> Option<&CompositionSurface> {
        self.0.get()
    }

    pub const fn loss(&self) -> Option<DeviceLost> {
        self.0.loss()
    }

    pub fn window_dpi(&self) -> Result<u32, SurfaceError> {
        let hwnd = self.0.get().map_or_else(
            || self.0.recovery_target().expect("surface exists").0,
            |surface| surface.core.hwnd(),
        );
        window_dpi(hwnd)
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

    pub fn present(&mut self) -> Result<(), SurfaceError> {
        let Some(surface) = self.0.get_mut() else {
            return Err(SurfaceError::DeviceLost(
                self.loss().expect("the surface is known to be lost"),
            ));
        };

        let hwnd = surface.core.hwnd();
        let size = surface.core.size();
        if let Err(error) = surface.present() {
            return self.0.fail(hwnd, size, error);
        }
        Ok(())
    }

    pub fn render_scene(
        &mut self,
        presentation: &Presentation<EmbeddedIcon>,
        needs_animation: bool,
    ) -> Result<FrameResult, SurfaceError> {
        let Some(surface) = self.0.get_mut() else {
            return Err(SurfaceError::DeviceLost(
                self.loss().expect("the surface is known to be lost"),
            ));
        };

        let hwnd = surface.core.hwnd();
        let size = surface.core.size();
        match surface.render(presentation, needs_animation) {
            Ok(result) => Ok(result),
            Err(PresentationRendererError::Windows(error)) => {
                self.0.fail(hwnd, size, error)
            }
            Err(error) => Err(SurfaceError::from(error)),
        }
    }

    pub fn commit(&mut self) -> Result<(), SurfaceError> {
        let Some(surface) = self.0.get_mut() else {
            return Err(SurfaceError::DeviceLost(
                self.loss().expect("the surface is known to be lost"),
            ));
        };

        let hwnd = surface.core.hwnd();
        let size = surface.core.size();
        if let Err(error) = surface.commit() {
            return self.0.fail(hwnd, size, error);
        }
        Ok(())
    }

    pub fn recover(&mut self, graphics: &GraphicsDevice) -> Result<(), SurfaceError> {
        let Some((hwnd, size)) = self.0.recovery_target() else {
            return Ok(());
        };
        let surface = CompositionSurface::create(graphics, hwnd, size)?;
        self.0.replace(surface);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameResult {
    Presented { needs_animation: bool },
    TargetRecreated,
}

impl FrameResult {
    pub const fn needs_animation(self) -> bool {
        matches!(
            self,
            Self::Presented {
                needs_animation: true
            }
        )
    }
}

#[derive(Debug, Error)]
pub enum SurfaceError {
    #[error(transparent)]
    Asset(#[from] AssetError),
    #[error("uploaded bitmap disappeared from the graphics cache")]
    BitmapCacheInvariant,
    #[error(transparent)]
    DeviceLost(DeviceLost),
    #[error("the dock window returned an invalid DPI")]
    InvalidWindowDpi,
    #[error("native graphics surface operation failed: {0}")]
    Native(NativeError),
}

impl From<WindowsError> for SurfaceError {
    fn from(error: WindowsError) -> Self {
        DeviceLost::from_hresult(error.code())
            .map_or_else(|| Self::Native(error.into()), Self::DeviceLost)
    }
}

fn window_dpi(hwnd: HWND) -> Result<u32, SurfaceError> {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    NonZeroU32::new(dpi)
        .map(NonZeroU32::get)
        .ok_or(SurfaceError::InvalidWindowDpi)
}

pub(super) fn swap_chain_description(size: SurfaceSize) -> DXGI_SWAP_CHAIN_DESC1 {
    DXGI_SWAP_CHAIN_DESC1 {
        Width: size.width(),
        Height: size.height(),
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: BUFFER_COUNT,
        Scaling: DXGI_SCALING_STRETCH,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
        AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
        ..DXGI_SWAP_CHAIN_DESC1::default()
    }
}
