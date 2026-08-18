use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::D2D1_ROUNDED_RECT;

use super::super::scene::{DockAnchor, DockIcon, PixelRect};
use super::super::surface::SurfaceSize;
use super::TARGET_DPI;

pub(super) fn dock_rectangle(
    size: SurfaceSize,
    radius: f32,
    width: f32,
    anchor: DockAnchor,
) -> D2D1_ROUNDED_RECT {
    let surface_width = pixels_to_f32(size.width());
    let left = match anchor {
        DockAnchor::Left => 0.0,
        DockAnchor::Center => (surface_width - width) * 0.5,
        DockAnchor::Right => surface_width - width,
    };
    D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left,
            top: 0.0,
            right: left + width,
            bottom: pixels_to_f32(size.height()),
        },
        radiusX: radius,
        radiusY: radius,
    }
}
pub(super) fn pixel_rectangle(rectangle: PixelRect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: pixels_to_f32(rectangle.left),
        top: pixels_to_f32(rectangle.top),
        right: pixels_to_f32(rectangle.left.saturating_add(rectangle.width)),
        bottom: pixels_to_f32(rectangle.top.saturating_add(rectangle.height)),
    }
}

pub(super) fn fitted_mascot_bounds(icon: &DockIcon, bounds: D2D_RECT_F) -> D2D_RECT_F {
    let DockIcon::Raster(raster) = icon else {
        return bounds;
    };
    let available_width = bounds.right - bounds.left;
    let available_height = bounds.bottom - bounds.top;
    let aspect = pixels_to_f32(raster.width()) / pixels_to_f32(raster.height());
    let (width, height) = if available_width / available_height > aspect {
        (available_height * aspect, available_height)
    } else {
        (available_width, available_width / aspect)
    };
    let center_x = f32::midpoint(bounds.left, bounds.right);
    let center_y = f32::midpoint(bounds.top, bounds.bottom);
    D2D_RECT_F {
        left: center_x - width * 0.5,
        top: center_y - height * 0.5,
        right: center_x + width * 0.5,
        bottom: center_y + height * 0.5,
    }
}
pub(super) fn inset_rectangle(rectangle: PixelRect, numerator: u32) -> PixelRect {
    let inset = rectangle.width.saturating_mul(numerator) / 28;
    PixelRect {
        left: rectangle.left.saturating_add(inset),
        top: rectangle.top.saturating_add(inset),
        width: rectangle.width.saturating_sub(inset.saturating_mul(2)),
        height: rectangle.height.saturating_sub(inset.saturating_mul(2)),
    }
}
pub(super) fn translated_scaled_pixel_rectangle(
    rectangle: PixelRect,
    scale: f32,
    offset_x: f32,
    offset_y: f32,
) -> D2D_RECT_F {
    let original = pixel_rectangle(rectangle);
    let center_x = f32::midpoint(original.left, original.right);
    let center_y = f32::midpoint(original.top, original.bottom);
    let half_width = (original.right - original.left) * 0.5 * scale;
    let half_height = (original.bottom - original.top) * 0.5 * scale;
    D2D_RECT_F {
        left: center_x - half_width + offset_x,
        top: center_y - half_height + offset_y,
        right: center_x + half_width + offset_x,
        bottom: center_y + half_height + offset_y,
    }
}
pub(super) fn scale_dip_offset(offset: f32, dpi: u32) -> f32 {
    offset * f32::from(u16::try_from(dpi).unwrap_or(u16::MAX)) / TARGET_DPI
}
pub(super) fn dragged_rectangle(
    pointer_x: i32,
    pointer_y: i32,
    side: u32,
    scale: f32,
    surface: SurfaceSize,
) -> D2D_RECT_F {
    let half = pixels_to_f32(side) * scale * 0.5;
    let center_x = clamped_drag_center(pointer_x, surface.width(), half);
    let center_y = clamped_drag_center(pointer_y, surface.height(), half);
    D2D_RECT_F {
        left: center_x - half,
        top: center_y - half,
        right: center_x + half,
        bottom: center_y + half,
    }
}
fn clamped_drag_center(pointer: i32, extent: u32, half_side: f32) -> f32 {
    let extent = pixels_to_f32(extent);
    if extent <= half_side * 2.0 {
        extent * 0.5
    } else {
        signed_pixels_to_f32(pointer).clamp(half_side, extent - half_side)
    }
}
#[allow(
    clippy::cast_precision_loss,
    reason = "captured pointer coordinates remain below f32 exact range"
)]
const fn signed_pixels_to_f32(value: i32) -> f32 {
    value as f32
}
pub(super) fn rounded_pixel_rectangle(
    rectangle: PixelRect,
    radius: f32,
) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect: pixel_rectangle(rectangle),
        radiusX: radius,
        radiusY: radius,
    }
}
pub(super) fn running_indicator(
    bounds: D2D_RECT_F,
    surface_height: u32,
    dpi: u32,
) -> D2D1_ROUNDED_RECT {
    let scale = f32::from(u16::try_from(dpi).unwrap_or(u16::MAX)) / TARGET_DPI;
    let width = 8.0 * scale;
    let height = 2.0 * scale;
    let center = f32::midpoint(bounds.left, bounds.right);
    let bottom = pixels_to_f32(surface_height);
    D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: center - width * 0.5,
            top: bottom - height,
            right: center + width * 0.5,
            bottom,
        },
        radiusX: height * 0.5,
        radiusY: height * 0.5,
    }
}
#[allow(
    clippy::cast_precision_loss,
    reason = "window dimensions are far below f32's exact integer range"
)]
pub(super) const fn pixels_to_f32(value: u32) -> f32 {
    value as f32
}
