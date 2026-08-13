use thiserror::Error;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL,
    D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_ERROR_DEVICE_HUNG, DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET,
    DXGI_ERROR_DRIVER_INTERNAL_ERROR, IDXGIDevice,
};
use windows::core::{Error as WindowsError, HRESULT, Interface};

use crate::NativeError;

const FEATURE_LEVELS: [D3D_FEATURE_LEVEL; 2] = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeviceDriver {
    Hardware,
    Warp,
}

impl DeviceDriver {
    const fn d3d_driver_type(self) -> D3D_DRIVER_TYPE {
        match self {
            Self::Hardware => D3D_DRIVER_TYPE_HARDWARE,
            Self::Warp => D3D_DRIVER_TYPE_WARP,
        }
    }
}

pub struct GraphicsDevice {
    device: ID3D11Device,
}

impl GraphicsDevice {
    pub fn create() -> Result<Self, GraphicsDeviceError> {
        match Self::create_with_driver(DeviceDriver::Hardware) {
            Ok(device) => Ok(device),
            Err(hardware) => Self::create_with_driver(DeviceDriver::Warp).map_err(|warp| {
                GraphicsDeviceError::CreationFailed { hardware: hardware.into(), warp: warp.into() }
            }),
        }
    }

    pub(crate) const fn device(&self) -> &ID3D11Device {
        &self.device
    }

    pub(crate) fn dxgi_device(&self) -> Result<IDXGIDevice, WindowsError> {
        self.device.cast()
    }

    pub fn loss(&self) -> Option<DeviceLost> {
        // SAFETY: `self.device` is a live, typed COM interface. The method has
        // no pointer arguments and `windows-rs` validates the returned HRESULT.
        unsafe { self.device.GetDeviceRemovedReason() }
            .err()
            .map(|error| DeviceLost::new(error.code()))
    }

    fn create_with_driver(driver: DeviceDriver) -> Result<Self, WindowsError> {
        let mut device = None;

        // SAFETY: All output pointers refer to initialized `Option` storage
        // that remains alive for the synchronous call. No adapter is supplied,
        // which is required when selecting either HARDWARE or WARP by type.
        unsafe {
            D3D11CreateDevice(
                None,
                driver.d3d_driver_type(),
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&FEATURE_LEVELS),
                D3D11_SDK_VERSION,
                Some(&raw mut device),
                None,
                None,
            )?;
        }

        Ok(Self { device: device.ok_or_else(unexpected_missing_interface)? })
    }
}

pub enum DeviceState {
    Ready(GraphicsDevice),
    Lost(DeviceLost),
}

impl DeviceState {
    pub fn create() -> Result<Self, GraphicsDeviceError> {
        GraphicsDevice::create().map(Self::Ready)
    }

    pub const fn ready(&self) -> Option<&GraphicsDevice> {
        match self {
            Self::Ready(device) => Some(device),
            Self::Lost(_) => None,
        }
    }

    pub const fn lost(&self) -> Option<DeviceLost> {
        match self {
            Self::Ready(_) => None,
            Self::Lost(loss) => Some(*loss),
        }
    }

    pub fn poll(&mut self) -> bool {
        let Self::Ready(device) = self else {
            return false;
        };
        let Some(loss) = device.loss() else {
            return false;
        };

        *self = Self::Lost(loss);
        true
    }

    pub fn recover(&mut self) -> Result<(), GraphicsDeviceError> {
        if matches!(self, Self::Ready(_)) {
            return Ok(());
        }

        *self = Self::create()?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("graphics device was lost ({reason})")]
pub struct DeviceLost {
    reason: HRESULT,
}

impl DeviceLost {
    const fn new(reason: HRESULT) -> Self {
        Self { reason }
    }

    pub(super) const fn from_hresult(reason: HRESULT) -> Option<Self> {
        match reason {
            DXGI_ERROR_DEVICE_HUNG
            | DXGI_ERROR_DEVICE_REMOVED
            | DXGI_ERROR_DEVICE_RESET
            | DXGI_ERROR_DRIVER_INTERNAL_ERROR => Some(Self::new(reason)),
            _ => None,
        }
    }
}

#[derive(Debug, Error)]
pub enum GraphicsDeviceError {
    #[error("hardware D3D11 creation failed ({hardware}); WARP creation also failed ({warp})")]
    CreationFailed { hardware: NativeError, warp: NativeError },
}

fn unexpected_missing_interface() -> WindowsError {
    WindowsError::new(
        HRESULT(0x8000_FFFF_u32.cast_signed()),
        "D3D11 reported success without returning a required interface",
    )
}
