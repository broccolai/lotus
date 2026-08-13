use std::num::NonZeroU32;

use thiserror::Error;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIAdapter,
    IDXGIDevice, IDXGIFactory2, IDXGISwapChain1,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::core::{Error as WindowsError, Interface};

use super::assets::AssetError;
use super::device::{DeviceLost, GraphicsDevice};
use super::launcher_scene::LauncherSize;
use super::renderer::{Direct2DRenderer, DrawResult, RendererError};
use super::scene::{DockScene, DockSize};
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

impl From<DockSize> for SurfaceSize {
    fn from(size: DockSize) -> Self {
        Self::new(size.width(), size.height()).expect("DockSize dimensions are nonzero")
    }
}

impl From<LauncherSize> for SurfaceSize {
    fn from(size: LauncherSize) -> Self {
        Self::new(size.width(), size.height()).expect("launcher size is guaranteed nonzero")
    }
}

pub struct CompositionSurface {
    hwnd: HWND,
    size: SurfaceSize,
    d3d_device: ID3D11Device,
    swap_chain: IDXGISwapChain1,
    composition_device: IDCompositionDevice,
    renderer: Direct2DRenderer,
    _target: IDCompositionTarget,
    _root_visual: IDCompositionVisual,
}

impl CompositionSurface {
    fn create(
        graphics: &GraphicsDevice,
        hwnd: HWND,
        size: SurfaceSize,
    ) -> Result<Self, SurfaceError> {
        let dxgi_device: IDXGIDevice = graphics.device().cast()?;

        // SAFETY: `dxgi_device` is a live typed COM interface and GetAdapter
        // returns a separately owned COM reference.
        let adapter: IDXGIAdapter = unsafe { dxgi_device.GetAdapter()? };
        // SAFETY: `adapter` is live; `IDXGIFactory2` is the requested typed
        // parent interface and windows-rs validates the returned COM pointer.
        let factory: IDXGIFactory2 = unsafe { adapter.GetParent()? };

        let description = swap_chain_description(size);
        // SAFETY: The device and factory are live typed COM interfaces, and
        // `description` remains valid for the duration of the synchronous call.
        let swap_chain = unsafe {
            factory.CreateSwapChainForComposition(
                graphics.device(),
                &raw const description,
                None,
            )?
        };

        // SAFETY: The BGRA-capable DXGI device remains live for the call and
        // windows-rs creates an owned, typed DirectComposition interface.
        let composition_device: IDCompositionDevice =
            unsafe { DCompositionCreateDevice(&dxgi_device)? };
        // SAFETY: `hwnd` is supplied by the window owner and must outlive this
        // surface. DirectComposition returns independently owned COM objects.
        let target = unsafe { composition_device.CreateTargetForHwnd(hwnd, true)? };
        // SAFETY: The composition device is live and returns an owned visual.
        let root_visual = unsafe { composition_device.CreateVisual()? };

        // SAFETY: All interfaces are live. The swap chain is valid visual
        // content, the visual is valid for this target, and Commit is called
        // before any temporary setup references leave scope.
        unsafe {
            root_visual.SetContent(&swap_chain)?;
            target.SetRoot(&root_visual)?;
            composition_device.Commit()?;
        }

        let renderer = Direct2DRenderer::create(graphics, &swap_chain)?;

        Ok(Self {
            hwnd,
            size,
            d3d_device: graphics.device().clone(),
            swap_chain,
            composition_device,
            renderer,
            _target: target,
            _root_visual: root_visual,
        })
    }

    pub const fn size(&self) -> SurfaceSize {
        self.size
    }

    fn resize(&mut self, size: SurfaceSize) -> Result<(), WindowsError> {
        if size == self.size {
            return if self.renderer_is_attached() {
                Ok(())
            } else {
                self.renderer.attach_target(&self.swap_chain)
            };
        }

        self.renderer.detach_target();
        // SAFETY: The surface does not expose swap-chain buffers, so it owns no
        // outstanding buffer references. Zero-sized buffers are excluded by
        // `SurfaceSize`, and zero preserves the existing buffer count/flags.
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

    fn renderer_is_attached(&self) -> bool {
        self.renderer.is_target_attached()
    }

    fn render(&mut self, scene: &DockScene) -> Result<FrameResult, RendererError> {
        if !self.renderer.is_target_attached() {
            self.renderer.attach_target(&self.swap_chain)?;
        }
        match self.renderer.draw(self.size, scene)? {
            DrawResult::Complete { needs_animation } => {
                self.present()?;
                Ok(FrameResult::Presented { needs_animation })
            }
            DrawResult::RecreateTarget => {
                self.ensure_device_available()?;
                self.renderer.attach_target(&self.swap_chain)?;
                Ok(FrameResult::TargetRecreated)
            }
        }
    }

    fn ensure_device_available(&self) -> Result<(), WindowsError> {
        // SAFETY: `d3d_device` is a live typed COM interface retained for the
        // lifetime of this device-dependent surface.
        unsafe { self.d3d_device.GetDeviceRemovedReason() }
    }

    fn present(&self) -> Result<(), WindowsError> {
        // SAFETY: The swap chain is live and owned by this surface. A sync
        // interval of one and no presentation flags are valid for flip-model
        // composition swap chains.
        unsafe { self.swap_chain.Present(1, DXGI_PRESENT(0)).ok() }
    }

    fn commit(&self) -> Result<(), WindowsError> {
        // SAFETY: The composition device is live and all visual-tree objects
        // referenced by the pending transaction are owned by this surface.
        unsafe { self.composition_device.Commit() }
    }
}

pub enum CompositionSurfaceState {
    Ready(Box<CompositionSurface>),
    Lost {
        hwnd: HWND,
        size: SurfaceSize,
        reason: DeviceLost,
    },
}

impl CompositionSurfaceState {
    pub fn create(
        graphics: &GraphicsDevice,
        window: WindowHandle,
        size: SurfaceSize,
    ) -> Result<Self, SurfaceError> {
        let hwnd = window.raw();
        CompositionSurface::create(graphics, hwnd, size)
            .map(|surface| Self::Ready(Box::new(surface)))
    }

    pub const fn ready(&self) -> Option<&CompositionSurface> {
        match self {
            Self::Ready(surface) => Some(surface),
            Self::Lost { .. } => None,
        }
    }

    pub const fn loss(&self) -> Option<DeviceLost> {
        match self {
            Self::Ready(_) => None,
            Self::Lost { reason, .. } => Some(*reason),
        }
    }

    pub fn window_dpi(&self) -> Result<u32, SurfaceError> {
        let hwnd = match self {
            Self::Ready(surface) => surface.hwnd,
            Self::Lost { hwnd, .. } => *hwnd,
        };
        window_dpi(hwnd)
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
            return self.handle_operation_error(hwnd, size, error);
        }
        Ok(())
    }

    pub fn present(&mut self) -> Result<(), SurfaceError> {
        let Self::Ready(surface) = self else {
            return Err(SurfaceError::DeviceLost(
                self.loss().expect("the surface is known to be lost"),
            ));
        };

        let hwnd = surface.hwnd;
        let size = surface.size;
        if let Err(error) = surface.present() {
            return self.handle_operation_error(hwnd, size, error);
        }
        Ok(())
    }

    pub fn render(&mut self) -> Result<FrameResult, SurfaceError> {
        if let Self::Lost { reason, .. } = self {
            return Err(SurfaceError::DeviceLost(*reason));
        }
        let scene =
            DockScene::initial(self.window_dpi()?, super::assets::SvgAsset::LotusPixel)
                .ok_or(SurfaceError::InvalidWindowDpi)?;
        self.render_scene(&scene)
    }

    pub fn render_scene(&mut self, scene: &DockScene) -> Result<FrameResult, SurfaceError> {
        let Self::Ready(surface) = self else {
            return Err(SurfaceError::DeviceLost(
                self.loss().expect("the surface is known to be lost"),
            ));
        };

        let hwnd = surface.hwnd;
        let size = surface.size;
        match surface.render(scene) {
            Ok(result) => Ok(result),
            Err(RendererError::Windows(error)) => {
                self.handle_operation_error(hwnd, size, error)
            }
            Err(error) => Err(SurfaceError::from(error)),
        }
    }

    pub fn commit(&mut self) -> Result<(), SurfaceError> {
        let Self::Ready(surface) = self else {
            return Err(SurfaceError::DeviceLost(
                self.loss().expect("the surface is known to be lost"),
            ));
        };

        let hwnd = surface.hwnd;
        let size = surface.size;
        if let Err(error) = surface.commit() {
            return self.handle_operation_error(hwnd, size, error);
        }
        Ok(())
    }

    pub fn recover(&mut self, graphics: &GraphicsDevice) -> Result<(), SurfaceError> {
        let Self::Lost { hwnd, size, .. } = self else {
            return Ok(());
        };
        let hwnd = *hwnd;
        let size = *size;

        *self = Self::create(graphics, WindowHandle::from_raw(hwnd), size)?;
        Ok(())
    }

    fn handle_operation_error<T>(
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

impl From<RendererError> for SurfaceError {
    fn from(error: RendererError) -> Self {
        match error {
            RendererError::Asset(error) => Self::Asset(error),
            RendererError::BitmapCacheInvariant => Self::BitmapCacheInvariant,
            RendererError::Windows(error) => Self::from(error),
        }
    }
}

impl From<WindowsError> for SurfaceError {
    fn from(error: WindowsError) -> Self {
        DeviceLost::from_hresult(error.code())
            .map_or_else(|| Self::Native(error.into()), Self::DeviceLost)
    }
}

fn window_dpi(hwnd: HWND) -> Result<u32, SurfaceError> {
    // SAFETY: The composition surface retains the live HWND supplied by its
    // window owner. A zero result is handled as an explicit error.
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
