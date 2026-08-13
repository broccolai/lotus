use std::num::NonZeroU32;
use std::sync::Arc;

use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Icon<Asset> {
    Embedded(Asset),
    Raster(RasterIcon),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RasterIconId(Arc<str>);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RasterIcon {
    id: RasterIconId,
    width: NonZeroU32,
    height: NonZeroU32,
    stride: u32,
    pixels: Arc<[u8]>,
}

impl RasterIcon {
    pub fn new(
        identity: String,
        width: u32,
        height: u32,
        premultiplied_bgra: Vec<u8>,
    ) -> Result<Self, RasterIconError> {
        if identity.trim().is_empty() {
            return Err(RasterIconError::EmptyIdentity);
        }
        let width = NonZeroU32::new(width).ok_or(RasterIconError::ZeroDimensions)?;
        let height = NonZeroU32::new(height).ok_or(RasterIconError::ZeroDimensions)?;
        let stride = width.get().checked_mul(4).ok_or(RasterIconError::DimensionsTooLarge)?;
        let expected = u64::from(width.get())
            .checked_mul(u64::from(height.get()))
            .and_then(|pixels| pixels.checked_mul(4))
            .and_then(|bytes| usize::try_from(bytes).ok())
            .ok_or(RasterIconError::DimensionsTooLarge)?;
        if premultiplied_bgra.len() != expected {
            return Err(RasterIconError::InvalidPixelLength {
                expected,
                actual: premultiplied_bgra.len(),
            });
        }
        if let Some((pixel_index, _)) = premultiplied_bgra
            .chunks_exact(4)
            .enumerate()
            .find(|(_, pixel)| pixel[0] > pixel[3] || pixel[1] > pixel[3] || pixel[2] > pixel[3])
        {
            return Err(RasterIconError::NotPremultiplied { pixel_index });
        }

        Ok(Self {
            id: RasterIconId(Arc::from(identity)),
            width,
            height,
            stride,
            pixels: Arc::from(premultiplied_bgra),
        })
    }

    pub const fn id(&self) -> &RasterIconId {
        &self.id
    }

    pub const fn width(&self) -> u32 {
        self.width.get()
    }

    pub const fn height(&self) -> u32 {
        self.height.get()
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub const fn stride(&self) -> u32 {
        self.stride
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum RasterIconError {
    #[error("raster icon identity cannot be empty")]
    EmptyIdentity,
    #[error("raster icon dimensions must be nonzero")]
    ZeroDimensions,
    #[error("raster icon dimensions are too large")]
    DimensionsTooLarge,
    #[error("raster icon requires {expected} bytes but received {actual}")]
    InvalidPixelLength { expected: usize, actual: usize },
    #[error("raster icon pixel {pixel_index} is not premultiplied BGRA")]
    NotPremultiplied { pixel_index: usize },
}
