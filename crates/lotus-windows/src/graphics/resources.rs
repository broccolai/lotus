use std::ffi::c_void;

use lotus_ui::icon::{RasterIcon, RasterIconId};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_NONE,
    D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1, ID2D1Bitmap1, ID2D1DeviceContext,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::core::Error as WindowsError;

const TARGET_DPI: f32 = 96.0;

pub(super) fn target_bitmap_properties() -> D2D1_BITMAP_PROPERTIES1 {
    bitmap_properties(D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW)
}

pub(super) fn source_bitmap_properties() -> D2D1_BITMAP_PROPERTIES1 {
    bitmap_properties(D2D1_BITMAP_OPTIONS_NONE)
}

fn bitmap_properties(bitmap_options: D2D1_BITMAP_OPTIONS) -> D2D1_BITMAP_PROPERTIES1 {
    D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: TARGET_DPI,
        dpiY: TARGET_DPI,
        bitmapOptions: bitmap_options,
        ..D2D1_BITMAP_PROPERTIES1::default()
    }
}

pub(super) fn upload_bgra_pixels(
    context: &ID2D1DeviceContext,
    width: u32,
    height: u32,
    pixels: &[u8],
    stride: u32,
) -> Result<ID2D1Bitmap1, WindowsError> {
    let properties = source_bitmap_properties();
    unsafe {
        context.CreateBitmap(
            D2D_SIZE_U { width, height },
            Some(pixels.as_ptr().cast::<c_void>()),
            stride,
            &raw const properties,
        )
    }
}

pub(super) fn raster_key(raster: &RasterIcon) -> (RasterIconId, u32, u32) {
    (raster.id().clone(), raster.width(), raster.height())
}
