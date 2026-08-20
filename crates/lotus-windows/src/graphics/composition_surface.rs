use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN;
use windows::Win32::Graphics::Dxgi::{
    DXGI_PRESENT, DXGI_SWAP_CHAIN_FLAG, IDXGIAdapter, IDXGIDevice, IDXGIFactory2,
    IDXGISwapChain1,
};
use windows::core::{Error as WindowsError, Interface};

use super::device::{DeviceLost, GraphicsDevice};
use super::surface::{SurfaceError, SurfaceSize, swap_chain_description};

pub(super) struct CompositionSurfaceCore {
    hwnd: HWND,
    size: SurfaceSize,
    d3d_device: ID3D11Device,
    swap_chain: IDXGISwapChain1,
    composition_device: IDCompositionDevice,
    _target: IDCompositionTarget,
    visual: IDCompositionVisual,
}

impl CompositionSurfaceCore {
    pub(super) fn create(
        graphics: &GraphicsDevice,
        hwnd: HWND,
        size: SurfaceSize,
    ) -> Result<Self, WindowsError> {
        let dxgi_device: IDXGIDevice = graphics.device().cast()?;
        let adapter: IDXGIAdapter = unsafe { dxgi_device.GetAdapter()? };
        let factory: IDXGIFactory2 = unsafe { adapter.GetParent()? };
        let description = swap_chain_description(size);
        let swap_chain = unsafe {
            factory.CreateSwapChainForComposition(
                graphics.device(),
                &raw const description,
                None,
            )?
        };
        let composition_device: IDCompositionDevice =
            unsafe { DCompositionCreateDevice(&dxgi_device)? };
        let target = unsafe { composition_device.CreateTargetForHwnd(hwnd, true)? };
        let visual = unsafe { composition_device.CreateVisual()? };

        unsafe {
            visual.SetContent(&swap_chain)?;
            target.SetRoot(&visual)?;
            composition_device.Commit()?;
        }

        Ok(Self {
            hwnd,
            size,
            d3d_device: graphics.device().clone(),
            swap_chain,
            composition_device,
            _target: target,
            visual,
        })
    }

    pub(super) const fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub(super) const fn size(&self) -> SurfaceSize {
        self.size
    }

    pub(super) const fn swap_chain(&self) -> &IDXGISwapChain1 {
        &self.swap_chain
    }

    pub(super) const fn composition_device(&self) -> &IDCompositionDevice {
        &self.composition_device
    }

    pub(super) const fn visual(&self) -> &IDCompositionVisual {
        &self.visual
    }

    pub(super) fn resize_buffers(&mut self, size: SurfaceSize) -> Result<(), WindowsError> {
        if size == self.size {
            return Ok(());
        }

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
        Ok(())
    }

    pub(super) fn present(&self) -> Result<(), WindowsError> {
        unsafe { self.swap_chain.Present(0, DXGI_PRESENT(0)).ok() }
    }

    pub(super) fn ensure_device_available(&self) -> Result<(), WindowsError> {
        unsafe { self.d3d_device.GetDeviceRemovedReason() }
    }

    pub(super) fn commit(&self) -> Result<(), WindowsError> {
        unsafe { self.composition_device.Commit() }
    }
}

pub(super) enum RecoverableSurface<Surface> {
    Ready(Box<Surface>),
    Lost {
        hwnd: HWND,
        size: SurfaceSize,
        reason: DeviceLost,
    },
}

impl<Surface> RecoverableSurface<Surface> {
    pub(super) fn ready(surface: Surface) -> Self {
        Self::Ready(Box::new(surface))
    }

    pub(super) const fn get(&self) -> Option<&Surface> {
        match self {
            Self::Ready(surface) => Some(surface),
            Self::Lost { .. } => None,
        }
    }

    pub(super) const fn get_mut(&mut self) -> Option<&mut Surface> {
        match self {
            Self::Ready(surface) => Some(surface),
            Self::Lost { .. } => None,
        }
    }

    pub(super) const fn loss(&self) -> Option<DeviceLost> {
        match self {
            Self::Ready(_) => None,
            Self::Lost { reason, .. } => Some(*reason),
        }
    }

    pub(super) fn remember_resize(&mut self, size: SurfaceSize) -> bool {
        let Self::Lost { size: pending, .. } = self else {
            return false;
        };

        *pending = size;
        true
    }

    pub(super) const fn recovery_target(&self) -> Option<(HWND, SurfaceSize)> {
        match self {
            Self::Ready(_) => None,
            Self::Lost { hwnd, size, .. } => Some((*hwnd, *size)),
        }
    }

    pub(super) fn replace(&mut self, surface: Surface) {
        *self = Self::ready(surface);
    }

    pub(super) fn fail<T>(
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
