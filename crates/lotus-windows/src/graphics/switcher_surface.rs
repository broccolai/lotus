use lotus_ui::geometry::NonZeroPhysicalSize;
use windows::Win32::Foundation::{E_FAIL, HWND};
use windows::Win32::Graphics::Direct3D11::ID3D11Device;
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN;
use windows::Win32::Graphics::Dxgi::{
    DXGI_PRESENT, DXGI_SWAP_CHAIN_FLAG, IDXGIAdapter, IDXGIDevice, IDXGIFactory2, IDXGISwapChain1,
};
use windows::core::{Error as WindowsError, Interface};

use crate::WindowHandle;

use super::device::{DeviceLost, GraphicsDevice};
use super::surface::{FrameResult, SurfaceError, SurfaceSize, swap_chain_description};
use super::switcher_renderer::{DrawResult, RendererError, SwitcherRenderer};
use super::switcher_scene::SwitcherScene;

impl From<RendererError> for SurfaceError {
    fn from(error: RendererError) -> Self {
        match error {
            RendererError::Windows(error) => Self::from(error),
            RendererError::Asset(error) => Self::from(error),
            RendererError::BitmapCacheInvariant => {
                Self::from(WindowsError::new(E_FAIL, "switcher bitmap cache invariant failed"))
            }
        }
    }
}

pub struct SwitcherCompositionSurface {
    hwnd: HWND,
    size: SurfaceSize,
    d3d_device: ID3D11Device,
    swap_chain: IDXGISwapChain1,
    composition_device: IDCompositionDevice,
    renderer: SwitcherRenderer,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
}

impl SwitcherCompositionSurface {
    fn create(
        graphics: &GraphicsDevice,
        hwnd: HWND,
        size: SurfaceSize,
    ) -> Result<Self, SurfaceError> {
        let dxgi: IDXGIDevice = graphics.device().cast()?;
        // SAFETY: The live typed device returns an owned adapter.
        let adapter: IDXGIAdapter = unsafe { dxgi.GetAdapter()? };
        // SAFETY: The adapter returns its typed factory parent.
        let factory: IDXGIFactory2 = unsafe { adapter.GetParent()? };
        let description = swap_chain_description(size);
        // SAFETY: Device, factory, and description remain live through creation.
        let swap_chain = unsafe {
            factory.CreateSwapChainForComposition(
                graphics.device(),
                &raw const description,
                None,
            )?
        };
        // SAFETY: The live DXGI device supports DirectComposition.
        let composition_device: IDCompositionDevice = unsafe { DCompositionCreateDevice(&dxgi)? };
        // SAFETY: The owner retains the HWND for the surface lifetime.
        let target = unsafe { composition_device.CreateTargetForHwnd(hwnd, true)? };
        // SAFETY: The device returns an owned visual.
        let visual = unsafe { composition_device.CreateVisual()? };
        // SAFETY: All typed interfaces are retained by this surface.
        unsafe {
            visual.SetContent(&swap_chain)?;
            target.SetRoot(&visual)?;
            composition_device.Commit()?;
        }
        let renderer = SwitcherRenderer::create(graphics, &swap_chain)?;
        Ok(Self {
            hwnd,
            size,
            d3d_device: graphics.device().clone(),
            swap_chain,
            composition_device,
            renderer,
            _target: target,
            _visual: visual,
        })
    }

    fn resize(&mut self, size: SurfaceSize) -> Result<(), WindowsError> {
        if self.size == size {
            return Ok(());
        }
        self.renderer.detach_target();
        // SAFETY: The renderer released its buffer and dimensions are nonzero.
        unsafe {
            self.swap_chain.ResizeBuffers(
                0,
                size.width(),
                size.height(),
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            )?;
        };
        self.size = size;
        self.renderer.attach_target(&self.swap_chain)
    }

    fn render(&mut self, scene: &SwitcherScene) -> Result<FrameResult, RendererError> {
        match self.renderer.draw(self.size, scene)? {
            DrawResult::Complete => {
                // SAFETY: This surface exclusively owns the live swap chain.
                unsafe { self.swap_chain.Present(1, DXGI_PRESENT(0)).ok()? };
                Ok(FrameResult::Presented { needs_animation: false })
            }
            DrawResult::RecreateTarget => {
                // SAFETY: The retained D3D device is live; this checks removal.
                unsafe { self.d3d_device.GetDeviceRemovedReason()? };
                self.renderer.attach_target(&self.swap_chain)?;
                Ok(FrameResult::TargetRecreated)
            }
        }
    }

    fn commit(&self) -> Result<(), WindowsError> {
        // SAFETY: The retained composition tree remains live.
        unsafe { self.composition_device.Commit() }
    }
}

pub enum SwitcherCompositionSurfaceState {
    Ready(Box<SwitcherCompositionSurface>),
    Lost { hwnd: HWND, size: SurfaceSize, reason: DeviceLost },
}

impl SwitcherCompositionSurfaceState {
    pub fn create(
        graphics: &GraphicsDevice,
        window: WindowHandle,
        size: NonZeroPhysicalSize,
    ) -> Result<Self, SurfaceError> {
        SwitcherCompositionSurface::create(graphics, window.raw(), surface_size(size))
            .map(|surface| Self::Ready(Box::new(surface)))
    }

    pub fn resize(&mut self, size: NonZeroPhysicalSize) -> Result<(), SurfaceError> {
        let size = surface_size(size);
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

    pub fn render_scene(&mut self, scene: &SwitcherScene) -> Result<FrameResult, SurfaceError> {
        let Self::Ready(surface) = self else {
            return Err(SurfaceError::DeviceLost(self.loss().expect("switcher surface is lost")));
        };
        let hwnd = surface.hwnd;
        let size = surface.size;
        match surface.render(scene) {
            Ok(frame) => Ok(frame),
            Err(RendererError::Windows(error)) => self.handle_error(hwnd, size, error),
            Err(error) => Err(error.into()),
        }
    }

    pub fn recover(&mut self, graphics: &GraphicsDevice) -> Result<(), SurfaceError> {
        let Self::Lost { hwnd, size, .. } = self else { return Ok(()) };
        let (hwnd, size) = (*hwnd, *size);
        *self = SwitcherCompositionSurface::create(graphics, hwnd, size)
            .map(|surface| Self::Ready(Box::new(surface)))?;
        if let Self::Ready(surface) = self {
            surface.commit()?;
        }
        Ok(())
    }

    const fn loss(&self) -> Option<DeviceLost> {
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

fn surface_size(size: NonZeroPhysicalSize) -> SurfaceSize {
    SurfaceSize::new(size.width(), size.height()).expect("switcher size is nonzero")
}
