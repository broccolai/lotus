use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionEffectGroup,
    IDCompositionScaleTransform, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN;
use windows::Win32::Graphics::Dxgi::{
    DXGI_PRESENT, DXGI_SWAP_CHAIN_FLAG, IDXGIAdapter, IDXGIDevice, IDXGIFactory2, IDXGISwapChain1,
};
use windows::core::{Error as WindowsError, Interface};

use crate::WindowHandle;

use super::device::{DeviceLost, GraphicsDevice};
use super::launcher_renderer::{LauncherDrawResult, LauncherRenderer, LauncherRendererError};
use super::launcher_scene::LauncherScene;
use super::surface::{FrameResult, SurfaceError, SurfaceSize, swap_chain_description};

pub struct LauncherCompositionSurface {
    hwnd: HWND,
    size: SurfaceSize,
    d3d_device: ID3D11Device,
    swap_chain: IDXGISwapChain1,
    composition_device: IDCompositionDevice,
    renderer: LauncherRenderer,
    scale: IDCompositionScaleTransform,
    effect: IDCompositionEffectGroup,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
}

impl LauncherCompositionSurface {
    fn create(
        graphics: &GraphicsDevice,
        hwnd: HWND,
        size: SurfaceSize,
    ) -> Result<Self, SurfaceError> {
        let dxgi: IDXGIDevice = graphics.device().cast()?;
        // SAFETY: The typed DXGI device is live and returns an owned adapter.
        let adapter: IDXGIAdapter = unsafe { dxgi.GetAdapter()? };
        // SAFETY: The adapter is live and returns its typed factory parent.
        let factory: IDXGIFactory2 = unsafe { adapter.GetParent()? };
        let description = swap_chain_description(size);
        // SAFETY: Device, factory and description remain valid through creation.
        let swap_chain = unsafe {
            factory.CreateSwapChainForComposition(
                graphics.device(),
                &raw const description,
                None,
            )?
        };
        // SAFETY: The live DXGI device supports DirectComposition ownership.
        let composition_device: IDCompositionDevice = unsafe { DCompositionCreateDevice(&dxgi)? };
        // SAFETY: HWND ownership remains with the caller and outlives this surface.
        let target = unsafe { composition_device.CreateTargetForHwnd(hwnd, true)? };
        // SAFETY: The live composition device returns an owned visual.
        let visual = unsafe { composition_device.CreateVisual()? };
        // SAFETY: The live composition device returns an owned transform.
        let scale = unsafe { composition_device.CreateScaleTransform()? };
        // SAFETY: The live composition device returns an owned effect group.
        let effect = unsafe { composition_device.CreateEffectGroup()? };
        // SAFETY: All typed interfaces are live and retained by the result.
        unsafe {
            visual.SetContent(&swap_chain)?;
            visual.SetTransform(&scale)?;
            visual.SetEffect(&effect)?;
            target.SetRoot(&visual)?;
            composition_device.Commit()?;
        }
        let renderer = LauncherRenderer::create(graphics, &swap_chain)?;
        Ok(Self {
            hwnd,
            size,
            d3d_device: graphics.device().clone(),
            swap_chain,
            composition_device,
            renderer,
            scale,
            effect,
            _target: target,
            _visual: visual,
        })
    }

    fn resize(&mut self, size: SurfaceSize) -> Result<(), WindowsError> {
        if size == self.size {
            return Ok(());
        }
        self.renderer.detach_target();
        // SAFETY: The renderer released its buffer reference; dimensions are nonzero.
        unsafe {
            self.swap_chain.ResizeBuffers(
                0,
                size.width(),
                size.height(),
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            )?;
        }
        self.size = size;
        self.renderer.attach_target(&self.swap_chain)
    }

    fn render(&mut self, scene: &LauncherScene) -> Result<FrameResult, LauncherRendererError> {
        let presentation = scene.presentation();
        let center_x = as_f32(self.size.width()) * 0.5;
        let center_y = as_f32(self.size.height()) * 0.08;
        // SAFETY: The visual and transform are live and accept finite values
        // derived from bounded scene progress and nonzero surface dimensions.
        unsafe {
            self.scale.SetCenterX2(center_x)?;
            self.scale.SetCenterY2(center_y)?;
            self.scale.SetScaleX2(presentation.scale)?;
            self.scale.SetScaleY2(presentation.scale)?;
            self.effect.SetOpacity2(presentation.opacity)?;
            self.composition_device.Commit()?;
        }
        match self.renderer.draw(self.size, scene)? {
            LauncherDrawResult::Complete => {
                // SAFETY: The live swap chain is owned by this surface.
                unsafe { self.swap_chain.Present(1, DXGI_PRESENT(0)).ok()? };
                Ok(FrameResult::Presented { needs_animation: scene.needs_animation() })
            }
            LauncherDrawResult::RecreateTarget => {
                // SAFETY: The retained D3D device is live; this checks removal.
                unsafe { self.d3d_device.GetDeviceRemovedReason()? };
                self.renderer.attach_target(&self.swap_chain)?;
                Ok(FrameResult::TargetRecreated)
            }
        }
    }

    fn commit(&self) -> Result<(), WindowsError> {
        // SAFETY: The composition device and its retained visual tree are live.
        unsafe { self.composition_device.Commit() }
    }
}

#[allow(clippy::cast_precision_loss, reason = "surface dimensions stay below f32 exact range")]
const fn as_f32(value: u32) -> f32 {
    value as f32
}

pub enum LauncherCompositionSurfaceState {
    Ready(Box<LauncherCompositionSurface>),
    Lost { hwnd: HWND, size: SurfaceSize, reason: DeviceLost },
}

impl LauncherCompositionSurfaceState {
    pub fn create(
        graphics: &GraphicsDevice,
        window: WindowHandle,
        size: SurfaceSize,
    ) -> Result<Self, SurfaceError> {
        let hwnd = window.raw();
        LauncherCompositionSurface::create(graphics, hwnd, size)
            .map(|surface| Self::Ready(Box::new(surface)))
    }

    pub fn resize(&mut self, size: SurfaceSize) -> Result<(), SurfaceError> {
        let Self::Ready(surface) = self else {
            if let Self::Lost { size: pending, .. } = self {
                *pending = size;
            }
            return Ok(());
        };
        let hwnd = surface.hwnd;
        if let Err(error) = surface.resize(size) {
            return self.handle_error(hwnd, size, error);
        }
        Ok(())
    }

    pub fn render_scene(&mut self, scene: &LauncherScene) -> Result<FrameResult, SurfaceError> {
        let Self::Ready(surface) = self else {
            return Err(SurfaceError::DeviceLost(
                self.loss().expect("launcher surface is known to be lost"),
            ));
        };
        let hwnd = surface.hwnd;
        let size = surface.size;
        match surface.render(scene) {
            Ok(frame) => Ok(frame),
            Err(LauncherRendererError::Windows(error)) => self.handle_error(hwnd, size, error),
            Err(error) => Err(error.into()),
        }
    }

    pub fn recover(&mut self, graphics: &GraphicsDevice) -> Result<(), SurfaceError> {
        let Self::Lost { hwnd, size, .. } = self else {
            return Ok(());
        };
        let hwnd = *hwnd;
        let size = *size;
        *self = Self::create(graphics, WindowHandle::from_raw(hwnd), size)?;
        if let Self::Ready(surface) = self {
            surface.commit()?;
        }
        Ok(())
    }

    pub const fn loss(&self) -> Option<DeviceLost> {
        match self {
            Self::Ready(_) => None,
            Self::Lost { reason, .. } => Some(*reason),
        }
    }

    fn handle_error<T>(
        &mut self,
        hwnd: HWND,
        size: SurfaceSize,
        error: WindowsError,
    ) -> Result<T, SurfaceError> {
        let Some(reason) = DeviceLost::from_hresult(error.code()) else {
            return Err(SurfaceError::from(error));
        };
        *self = Self::Lost { hwnd, size, reason };
        Err(SurfaceError::DeviceLost(reason))
    }
}

impl From<LauncherRendererError> for SurfaceError {
    fn from(error: LauncherRendererError) -> Self {
        match error {
            LauncherRendererError::Asset(error) => Self::Asset(error),
            LauncherRendererError::BitmapCacheInvariant => Self::BitmapCacheInvariant,
            LauncherRendererError::Windows(error) => Self::from(error),
        }
    }
}
