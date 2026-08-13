use std::collections::HashMap;
use std::ffi::c_void;
use std::num::NonZeroU32;

use lotus_ui::theme::Theme;
use thiserror::Error;
use windows::Win32::Foundation::D2DERR_RECREATE_TARGET;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_OPTIONS_TARGET,
    D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_ROUNDED_RECT,
    D2D1CreateFactory, ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext, ID2D1Factory1,
    ID2D1Image, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::{IDXGISurface, IDXGISwapChain1};
use windows::core::{Error as WindowsError, PCWSTR, w};

use super::assets::{AssetError, RasterSize, SvgAsset, SvgAssetCache};
use super::device::GraphicsDevice;
use super::launcher_scene::{LauncherLayout, LauncherScene, PixelRect};
use super::scene::{DockIcon, RasterIcon, RasterIconId};
use super::surface::SurfaceSize;
use super::theme;

const TARGET_DPI: f32 = 96.0;
const TRANSPARENT: D2D1_COLOR_F = color(0.0, 0.0, 0.0, 0.0);

pub(super) enum LauncherDrawResult {
    Complete,
    RecreateTarget,
}

struct LauncherChrome {
    panel: D2D_RECT_F,
    query_panel: D2D1_ROUNDED_RECT,
    query_outline: D2D1_ROUNDED_RECT,
    search_bounds: D2D_RECT_F,
    query_text: Vec<u16>,
    query_text_bounds: D2D_RECT_F,
    caret: D2D_RECT_F,
}

pub(super) struct LauncherRenderer {
    _factory: ID2D1Factory1,
    _device: ID2D1Device,
    write_factory: IDWriteFactory,
    context: ID2D1DeviceContext,
    target: Option<ID2D1Bitmap1>,
    panel: ID2D1SolidColorBrush,
    field: ID2D1SolidColorBrush,
    field_border: ID2D1SolidColorBrush,
    hovered: ID2D1SolidColorBrush,
    selected: ID2D1SolidColorBrush,
    selected_border: ID2D1SolidColorBrush,
    query_text: ID2D1SolidColorBrush,
    placeholder_text: ID2D1SolidColorBrush,
    caret: ID2D1SolidColorBrush,
    search_glyph: ID2D1SolidColorBrush,
    result_text: ID2D1SolidColorBrush,
    initial_text: ID2D1SolidColorBrush,
    search_format: IDWriteTextFormat,
    query_format: IDWriteTextFormat,
    title_format: IDWriteTextFormat,
    initial_format: IDWriteTextFormat,
    empty_format: IDWriteTextFormat,
    footer_label_format: IDWriteTextFormat,
    footer_time_format: IDWriteTextFormat,
    assets: SvgAssetCache,
    embedded_icons: HashMap<(SvgAsset, NonZeroU32), ID2D1Bitmap1>,
    raster_icons: HashMap<(RasterIconId, u32, u32), ID2D1Bitmap1>,
}

impl LauncherRenderer {
    pub(super) fn create(
        graphics: &GraphicsDevice,
        swap_chain: &IDXGISwapChain1,
    ) -> Result<Self, LauncherRendererError> {
        let dxgi = graphics.dxgi_device()?;
        // SAFETY: A supported typed Direct2D factory is requested with no retained options.
        let factory: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        // SAFETY: Both typed device creation calls use live COM interfaces.
        let device = unsafe { factory.CreateDevice(&dxgi)? };
        // SAFETY: The device is live and returns an owned context.
        let context =
            unsafe { device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)? };
        // SAFETY: DirectWrite returns an owned typed shared factory.
        let write_factory: IDWriteFactory =
            unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        // SAFETY: Static color values remain alive for each synchronous brush creation.
        let theme = Theme::default();
        let panel = brush(&context, &theme::d2d(theme.chrome_overlay))?;
        let field = brush(&context, &theme::d2d(theme.control))?;
        let field_border = brush(&context, &theme::d2d(theme.border))?;
        let hovered = brush(&context, &theme::d2d(theme.control_hover))?;
        let selected = brush(&context, &theme::d2d(theme.control_selected))?;
        let selected_border = brush(&context, &theme::d2d(theme.border_strong))?;
        let query_text = brush(&context, &theme::d2d(theme.text))?;
        let placeholder_text = brush(&context, &theme::d2d(theme.text_muted))?;
        let caret = brush(&context, &theme::d2d(theme.accent))?;
        let search_glyph = brush(&context, &theme::d2d(theme.text_muted))?;
        let result_text = brush(&context, &theme::d2d(theme.text))?;
        let initial_text = brush(&context, &theme::d2d(theme.text))?;
        let search_format = text_format_family(
            &write_factory,
            w!("Segoe Fluent Icons"),
            17.0,
            DWRITE_FONT_WEIGHT_NORMAL,
        )?;
        let query_format = text_format(&write_factory, 18.0, DWRITE_FONT_WEIGHT_NORMAL)?;
        let title_format = text_format(&write_factory, 14.5, DWRITE_FONT_WEIGHT_NORMAL)?;
        let initial_format =
            text_format(&write_factory, 15.0, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
        let empty_format = text_format(&write_factory, 14.0, DWRITE_FONT_WEIGHT_NORMAL)?;
        let footer_label_format =
            text_format(&write_factory, 12.5, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
        let footer_time_format =
            text_format(&write_factory, 12.5, DWRITE_FONT_WEIGHT_NORMAL)?;
        // SAFETY: The owned format is live and alignment values are valid.
        unsafe {
            title_format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            search_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            search_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            query_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            title_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            initial_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            initial_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            empty_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            empty_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            footer_label_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            footer_time_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
            footer_time_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        }
        let mut result = Self {
            _factory: factory,
            _device: device,
            write_factory,
            context,
            target: None,
            panel,
            field,
            field_border,
            hovered,
            selected,
            selected_border,
            query_text,
            placeholder_text,
            caret,
            search_glyph,
            result_text,
            initial_text,
            search_format,
            query_format,
            title_format,
            initial_format,
            empty_format,
            footer_label_format,
            footer_time_format,
            assets: SvgAssetCache::create()?,
            embedded_icons: HashMap::new(),
            raster_icons: HashMap::new(),
        };
        result.attach_target(swap_chain)?;
        Ok(result)
    }

    pub(super) fn detach_target(&mut self) {
        // SAFETY: A null target releases the current buffer reference before resize.
        unsafe { self.context.SetTarget(None::<&ID2D1Image>) };
        self.target = None;
    }

    pub(super) fn attach_target(
        &mut self,
        chain: &IDXGISwapChain1,
    ) -> Result<(), WindowsError> {
        self.detach_target();
        // SAFETY: Buffer zero exists on the initialized composition swap chain.
        let surface: IDXGISurface = unsafe { chain.GetBuffer(0)? };
        let properties = target_properties();
        // SAFETY: Surface and properties are live through the synchronous call.
        let target = unsafe {
            self.context
                .CreateBitmapFromDxgiSurface(&surface, Some(&raw const properties))?
        };
        // SAFETY: The bitmap belongs to this context and has TARGET enabled.
        unsafe { self.context.SetTarget(&target) };
        self.target = Some(target);
        Ok(())
    }

    pub(super) fn draw(
        &mut self,
        size: SurfaceSize,
        scene: &LauncherScene,
    ) -> Result<LauncherDrawResult, LauncherRendererError> {
        debug_assert!(self.target.is_some());
        self.apply_theme(&scene.theme());
        let layout = scene.layout();
        let chrome = self.prepare_chrome(size, scene, &layout)?;
        for entry in scene.results() {
            if let Some(icon) = &entry.icon {
                self.ensure_icon(icon, scene.result_icon_size())?;
            }
        }
        let icon_draws = scene
            .results()
            .iter()
            .zip(&layout.row_icons)
            .filter_map(|(entry, bounds)| entry.icon.as_ref().zip(*bounds))
            .map(|(icon, bounds)| {
                self.icon(icon, scene.result_icon_size())
                    .map(|bitmap| (bitmap, rect(bounds)))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let transparent = TRANSPARENT;
        // SAFETY: Target, brushes, formats, text and geometry live through EndDraw.
        let result = unsafe {
            self.context.BeginDraw();
            self.context.Clear(Some(&raw const transparent));
            self.draw_chrome(&chrome, scene, &layout);
            self.draw_results(scene, &layout, &icon_draws);
            self.draw_footer(scene, &layout);
            self.context.EndDraw(None, None)
        };
        match result {
            Ok(()) => Ok(LauncherDrawResult::Complete),
            Err(error) if error.code() == D2DERR_RECREATE_TARGET => {
                Ok(LauncherDrawResult::RecreateTarget)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn apply_theme(&self, value: &Theme) {
        theme::set(&self.panel, value.chrome_overlay);
        theme::set(&self.field, value.control);
        theme::set(&self.field_border, value.border);
        theme::set(&self.hovered, value.control_hover);
        theme::set(&self.selected, value.control_selected);
        theme::set(&self.selected_border, value.border_strong);
        theme::set(&self.query_text, value.text);
        theme::set(&self.placeholder_text, value.text_muted);
        theme::set(&self.caret, value.accent);
        theme::set(&self.search_glyph, value.text_muted);
        theme::set(&self.result_text, value.text);
        theme::set(&self.initial_text, value.text);
    }

    fn prepare_chrome(
        &self,
        size: SurfaceSize,
        scene: &LauncherScene,
        layout: &LauncherLayout,
    ) -> Result<LauncherChrome, WindowsError> {
        let panel_rect = surface_rect(size);
        let query_rect = rect(layout.query);
        let control_radius = control_radius(scene);
        let query_text_bounds = search_text_rect(query_rect);
        Ok(LauncherChrome {
            panel: panel_rect,
            query_panel: rounded(query_rect, control_radius),
            query_outline: rounded(inset_all(query_rect, 0.5), control_radius - 0.5),
            search_bounds: search_glyph_rect(query_rect),
            query_text: utf16(if scene.query().is_empty() {
                "Search applications…"
            } else {
                scene.query()
            }),
            query_text_bounds,
            caret: caret_rect(
                &self.write_factory,
                &self.query_format,
                query_text_bounds,
                scene.query_before_cursor(),
            )?,
        })
    }

    fn draw_chrome(
        &self,
        chrome: &LauncherChrome,
        scene: &LauncherScene,
        layout: &LauncherLayout,
    ) {
        // SAFETY: The active draw target, retained brushes, formats, and geometry remain live.
        unsafe {
            self.context
                .FillRectangle(&raw const chrome.panel, &self.panel);
            self.context
                .FillRoundedRectangle(&raw const chrome.query_panel, &self.field);
            self.context.DrawRoundedRectangle(
                &raw const chrome.query_outline,
                &self.field_border,
                1.0,
                None,
            );
            self.context.DrawText(
                &utf16("\u{E721}"),
                &self.search_format,
                &raw const chrome.search_bounds,
                &self.search_glyph,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
            let query_brush = if scene.query().is_empty() {
                &self.placeholder_text
            } else {
                &self.query_text
            };
            self.context.DrawText(
                &chrome.query_text,
                &self.query_format,
                &raw const chrome.query_text_bounds,
                query_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
            self.context
                .FillRectangle(&raw const chrome.caret, &self.caret);
            self.draw_row_states(layout, control_radius(scene));
        }
    }

    fn draw_row_states(&self, layout: &LauncherLayout, radius: f32) {
        // SAFETY: The active draw target, retained brushes, and projected geometry remain live.
        unsafe {
            if let Some(hovered) = layout
                .hovered
                .filter(|hovered| Some(*hovered) != layout.selected)
                .and_then(|index| layout.row_surfaces.get(index))
            {
                let highlight = rounded(rect(*hovered), radius);
                self.context
                    .FillRoundedRectangle(&raw const highlight, &self.hovered);
            }
            if let Some(selected) = layout
                .selected
                .and_then(|index| layout.row_surfaces.get(index))
            {
                let highlight = rounded(rect(*selected), radius);
                let outline = rounded(inset_all(rect(*selected), 0.5), radius - 0.5);
                self.context
                    .FillRoundedRectangle(&raw const highlight, &self.selected);
                self.context.DrawRoundedRectangle(
                    &raw const outline,
                    &self.selected_border,
                    1.0,
                    None,
                );
            }
        }
    }

    fn draw_results(
        &self,
        scene: &LauncherScene,
        layout: &LauncherLayout,
        icon_draws: &[(&ID2D1Bitmap1, D2D_RECT_F)],
    ) {
        // SAFETY: The active draw target and all cached bitmaps, text, formats, and brushes live.
        unsafe {
            for (bitmap, bounds) in icon_draws {
                self.context.DrawBitmap(
                    *bitmap,
                    Some(bounds),
                    1.0,
                    windows::Win32::Graphics::Direct2D::D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                    None,
                    None,
                );
            }
            for (index, (entry, bounds)) in
                scene.results().iter().zip(&layout.row_texts).enumerate()
            {
                if entry.icon.is_none() {
                    let initial_bounds = rect(layout.row_icon_cells[index]);
                    self.context.DrawText(
                        &utf16(&entry.initial()),
                        &self.initial_format,
                        &raw const initial_bounds,
                        &self.initial_text,
                        D2D1_DRAW_TEXT_OPTIONS_CLIP,
                        DWRITE_MEASURING_MODE_NATURAL,
                    );
                }
                let title = rect(*bounds);
                self.context.DrawText(
                    &utf16(&entry.title),
                    &self.title_format,
                    &raw const title,
                    &self.result_text,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
            }
            if let Some(empty_state) = layout.empty_state {
                let bounds = rect(empty_state);
                self.context.DrawText(
                    &utf16("No applications found"),
                    &self.empty_format,
                    &raw const bounds,
                    &self.placeholder_text,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
            }
        }
    }

    fn draw_footer(&self, scene: &LauncherScene, layout: &LauncherLayout) {
        let separator = rect(layout.footer_separator);
        let label = rect(layout.footer_label);
        let time = rect(layout.footer_time);
        // SAFETY: The active target, retained brushes/formats, and local UTF-16 buffers remain
        // live for all synchronous Direct2D/DirectWrite calls.
        unsafe {
            if let Some(thumb) = layout.scrollbar_thumb {
                let radius =
                    f32::from(u16::try_from(thumb.width).unwrap_or(u16::MAX)) / 2.0;
                let thumb = rounded(rect(thumb), radius);
                self.context
                    .FillRoundedRectangle(&raw const thumb, &self.placeholder_text);
            }
            self.context
                .FillRectangle(&raw const separator, &self.field_border);
            self.context.DrawText(
                &utf16("Lotus"),
                &self.footer_label_format,
                &raw const label,
                &self.caret,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
            self.context.DrawText(
                &utf16(scene.footer_time()),
                &self.footer_time_format,
                &raw const time,
                &self.placeholder_text,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    fn ensure_icon(
        &mut self,
        icon: &DockIcon,
        size: NonZeroU32,
    ) -> Result<(), LauncherRendererError> {
        match icon {
            DockIcon::Embedded(asset) => {
                let key = (*asset, size);
                if !self.embedded_icons.contains_key(&key) {
                    let raster = self.assets.rasterize(*asset, RasterSize::square(size))?;
                    let bitmap = upload_pixels(
                        &self.context,
                        raster.size().width(),
                        raster.size().height(),
                        raster.pixels(),
                        raster.stride()?,
                    )?;
                    self.embedded_icons.insert(key, bitmap);
                }
            }
            DockIcon::Raster(raster) => {
                let key = raster_key(raster);
                if !self.raster_icons.contains_key(&key) {
                    let bitmap = upload_pixels(
                        &self.context,
                        raster.width(),
                        raster.height(),
                        raster.pixels(),
                        raster.stride(),
                    )?;
                    self.raster_icons.insert(key, bitmap);
                }
            }
        }
        Ok(())
    }

    fn icon(
        &self,
        icon: &DockIcon,
        size: NonZeroU32,
    ) -> Result<&ID2D1Bitmap1, LauncherRendererError> {
        match icon {
            DockIcon::Embedded(asset) => self.embedded_icons.get(&(*asset, size)),
            DockIcon::Raster(raster) => self.raster_icons.get(&raster_key(raster)),
        }
        .ok_or(LauncherRendererError::BitmapCacheInvariant)
    }
}

#[derive(Debug, Error)]
pub(super) enum LauncherRendererError {
    #[error(transparent)]
    Asset(#[from] AssetError),
    #[error("uploaded launcher icon disappeared from the graphics cache")]
    BitmapCacheInvariant,
    #[error(transparent)]
    Windows(#[from] WindowsError),
}

fn text_format(
    factory: &IDWriteFactory,
    size: f32,
    weight: DWRITE_FONT_WEIGHT,
) -> Result<IDWriteTextFormat, WindowsError> {
    text_format_family(factory, w!("Segoe UI Variable Text"), size, weight)
}

fn text_format_family(
    factory: &IDWriteFactory,
    family: PCWSTR,
    size: f32,
    weight: DWRITE_FONT_WEIGHT,
) -> Result<IDWriteTextFormat, WindowsError> {
    // SAFETY: Static family and locale strings are NUL terminated.
    unsafe {
        factory.CreateTextFormat(
            family,
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-us"),
        )
    }
}

fn brush(
    context: &ID2D1DeviceContext,
    value: &D2D1_COLOR_F,
) -> Result<ID2D1SolidColorBrush, WindowsError> {
    // SAFETY: The color remains live for Direct2D's synchronous copy.
    unsafe { context.CreateSolidColorBrush(value, None) }
}

fn target_properties() -> D2D1_BITMAP_PROPERTIES1 {
    D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: TARGET_DPI,
        dpiY: TARGET_DPI,
        bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
        ..D2D1_BITMAP_PROPERTIES1::default()
    }
}

fn source_properties() -> D2D1_BITMAP_PROPERTIES1 {
    D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: TARGET_DPI,
        dpiY: TARGET_DPI,
        bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
        ..D2D1_BITMAP_PROPERTIES1::default()
    }
}

fn upload_pixels(
    context: &ID2D1DeviceContext,
    width: u32,
    height: u32,
    pixels: &[u8],
    stride: u32,
) -> Result<ID2D1Bitmap1, WindowsError> {
    let properties = source_properties();
    // SAFETY: Both icon sources validate tightly packed premultiplied BGRA8;
    // the slice remains live for Direct2D's synchronous copy.
    unsafe {
        context.CreateBitmap(
            D2D_SIZE_U { width, height },
            Some(pixels.as_ptr().cast::<c_void>()),
            stride,
            &raw const properties,
        )
    }
}

fn raster_key(raster: &RasterIcon) -> (RasterIconId, u32, u32) {
    (raster.id().clone(), raster.width(), raster.height())
}

const fn color(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}

fn surface_rect(size: SurfaceSize) -> D2D_RECT_F {
    D2D_RECT_F {
        left: 0.0,
        top: 0.0,
        right: as_f32(size.width()),
        bottom: as_f32(size.height()),
    }
}
fn rect(value: PixelRect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: as_f32(value.left),
        top: as_f32(value.top),
        right: as_f32(value.left.saturating_add(value.width)),
        bottom: as_f32(value.top.saturating_add(value.height)),
    }
}
fn rounded(rect: D2D_RECT_F, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect,
        radiusX: radius,
        radiusY: radius,
    }
}
fn inset_all(mut rect: D2D_RECT_F, amount: f32) -> D2D_RECT_F {
    rect.left += amount;
    rect.top += amount;
    rect.right -= amount;
    rect.bottom -= amount;
    rect
}

fn search_text_rect(query: D2D_RECT_F) -> D2D_RECT_F {
    let scale = (query.bottom - query.top) / 50.0;
    D2D_RECT_F {
        left: query.left + 44.0 * scale,
        top: query.top,
        right: query.right - 14.0 * scale,
        bottom: query.bottom,
    }
}

fn control_radius(scene: &LauncherScene) -> f32 {
    as_f32(scene.dpi()) * scene.theme().radii.control / TARGET_DPI
}

fn search_glyph_rect(query: D2D_RECT_F) -> D2D_RECT_F {
    let scale = (query.bottom - query.top) / 50.0;
    D2D_RECT_F {
        left: query.left + 14.0 * scale,
        top: query.top,
        right: query.left + 31.0 * scale,
        bottom: query.bottom,
    }
}

fn caret_rect(
    factory: &IDWriteFactory,
    format: &IDWriteTextFormat,
    query: D2D_RECT_F,
    text: &str,
) -> Result<D2D_RECT_F, WindowsError> {
    let text = utf16(text);
    // SAFETY: The text slice and format are live for layout creation, and the
    // metrics pointer is valid for the synchronous read.
    let metrics = unsafe {
        let layout = factory.CreateTextLayout(
            &text,
            format,
            query.right - query.left,
            query.bottom - query.top,
        )?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        layout.GetMetrics(&raw mut metrics)?;
        metrics
    };
    let left =
        (query.left + metrics.widthIncludingTrailingWhitespace + 1.0).min(query.right);
    Ok(D2D_RECT_F {
        left,
        top: query.top + 13.0,
        right: left + 1.0,
        bottom: query.bottom - 13.0,
    })
}
fn utf16(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}

#[allow(
    clippy::cast_precision_loss,
    reason = "launcher dimensions remain below f32 exact range"
)]
const fn as_f32(value: u32) -> f32 {
    value as f32
}
