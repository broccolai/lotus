use std::num::NonZeroU32;

use lotus_core::settings::{DockZone, NotificationBadgeStyle};
use lotus_settings::appearance::{AccentPreset, ForegroundPreset, SurfacePreset};
use lotus_ui::icon::{RasterIcon, RasterIconId};
use lotus_ui::theme::{Color, Theme};
use thiserror::Error;
use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, E_FAIL};
use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_COLOR_F};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_ROUNDED_RECT, D2D1CreateFactory,
    ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext, ID2D1Factory1, ID2D1Image,
    ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_AXIS_TAG, DWRITE_FONT_AXIS_TAG_OPTICAL_SIZE,
    DWRITE_FONT_AXIS_TAG_WEIGHT, DWRITE_FONT_AXIS_VALUE, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory, IDWriteFactory6,
    IDWriteFontCollection1, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::{IDXGISurface, IDXGISwapChain1};
use windows::core::{Error as WindowsError, Interface, w};

use super::assets::{AssetError, IconTint, RasterSize, SvgAsset, SvgAssetCache};
use super::device::GraphicsDevice;
use super::resources::{raster_key, target_bitmap_properties, upload_bgra_pixels};
use super::surface::SurfaceSize;
use super::{
    OnboardingModule, OnboardingStep, SettingsControl, SettingsLayout, SettingsPage,
    SettingsRect, SettingsScene, SettingsSlider, SettingsToggle, SettingsUpdateActivity,
    theme,
};
use crate::font::BundledFontCollection;
use crate::platform::windows::backdrop::{self, SettingsMaterial};
use crate::resource_cache::BoundedResourceCache;

mod about_update;
mod controls;
mod navigation;
mod onboarding;

const TARGET_DPI: f32 = 96.0;
const TRANSPARENT: D2D1_COLOR_F = color(0.0, 0.0, 0.0, 0.0);
const FRAUNCES_SOFTNESS: DWRITE_FONT_AXIS_TAG =
    DWRITE_FONT_AXIS_TAG(u32::from_le_bytes(*b"SOFT"));
const FRAUNCES_WONK: DWRITE_FONT_AXIS_TAG =
    DWRITE_FONT_AXIS_TAG(u32::from_le_bytes(*b"WONK"));

pub(super) enum SettingsDrawResult {
    Complete,
    RecreateTarget,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ButtonEmphasis {
    Primary,
    Secondary,
    Outline,
}

pub(super) struct SettingsRenderer {
    _factory: ID2D1Factory1,
    _device: ID2D1Device,
    context: ID2D1DeviceContext,
    target: Option<ID2D1Bitmap1>,
    panel: ID2D1SolidColorBrush,
    sidebar: ID2D1SolidColorBrush,
    sidebar_selected: ID2D1SolidColorBrush,
    group: ID2D1SolidColorBrush,
    row: ID2D1SolidColorBrush,
    hover: ID2D1SolidColorBrush,
    selected: ID2D1SolidColorBrush,
    accent: ID2D1SolidColorBrush,
    accent_dark: ID2D1SolidColorBrush,
    text: ID2D1SolidColorBrush,
    muted: ID2D1SolidColorBrush,
    disabled: ID2D1SolidColorBrush,
    track: ID2D1SolidColorBrush,
    focus: ID2D1SolidColorBrush,
    divider: ID2D1SolidColorBrush,
    title_format: IDWriteTextFormat,
    _bundled_fonts: BundledFontCollection,
    brand_format: IDWriteTextFormat,
    hero_format: IDWriteTextFormat,
    onboarding_format: IDWriteTextFormat,
    onboarding_body_format: IDWriteTextFormat,
    onboarding_small_format: IDWriteTextFormat,
    onboarding_button_format: IDWriteTextFormat,
    body_format: IDWriteTextFormat,
    small_format: IDWriteTextFormat,
    button_format: IDWriteTextFormat,
    material: SettingsMaterial,
    assets: SvgAssetCache,
    embedded: BoundedResourceCache<(SvgAsset, NonZeroU32), ID2D1Bitmap1>,
    rasters: BoundedResourceCache<(RasterIconId, u32, u32), ID2D1Bitmap1>,
}

impl SettingsRenderer {
    pub(super) fn create(
        graphics: &GraphicsDevice,
        swap_chain: &IDXGISwapChain1,
    ) -> Result<Self, SettingsRendererError> {
        let dxgi = graphics.dxgi_device()?;
        let factory: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let device = unsafe { factory.CreateDevice(&dxgi)? };
        let context =
            unsafe { device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)? };
        let write_factory: IDWriteFactory6 =
            unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let bundled_fonts = BundledFontCollection::create(&write_factory)?;
        let title_format = text_format(&write_factory, 18.0, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
        let brand_format =
            fraunces_format(&write_factory, bundled_fonts.collection(), 22.0, 600.0)?;
        let hero_format =
            fraunces_format(&write_factory, bundled_fonts.collection(), 88.0, 600.0)?;
        let onboarding_format =
            fraunces_format(&write_factory, bundled_fonts.collection(), 30.0, 400.0)?;
        let onboarding_body_format =
            text_format(&write_factory, 15.0, DWRITE_FONT_WEIGHT_NORMAL)?;
        let onboarding_small_format =
            text_format(&write_factory, 13.5, DWRITE_FONT_WEIGHT_NORMAL)?;
        let onboarding_button_format =
            text_format(&write_factory, 16.5, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
        let body_format = text_format(&write_factory, 14.0, DWRITE_FONT_WEIGHT_NORMAL)?;
        let small_format = text_format(&write_factory, 12.5, DWRITE_FONT_WEIGHT_NORMAL)?;
        let button_format =
            text_format(&write_factory, 13.5, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
        unsafe {
            title_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            brand_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            hero_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            onboarding_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            onboarding_body_format
                .SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            onboarding_body_format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            onboarding_small_format
                .SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            onboarding_small_format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            onboarding_button_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            onboarding_button_format
                .SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            body_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            body_format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            small_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            small_format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            button_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            button_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        }
        let theme = Theme::default();
        let mut renderer = Self {
            _factory: factory,
            _device: device,
            context: context.clone(),
            target: None,
            panel: brush(&context, &theme::d2d(theme.canvas))?,
            sidebar: brush(&context, &theme::d2d(theme.accent_soft))?,
            sidebar_selected: brush(&context, &theme::d2d(theme.border_strong))?,
            group: brush(&context, &theme::d2d(theme.surface))?,
            row: brush(&context, &theme::d2d(theme.control))?,
            hover: brush(&context, &theme::d2d(theme.control_hover))?,
            selected: brush(&context, &theme::d2d(theme.control_selected))?,
            accent: brush(&context, &theme::d2d(theme.accent))?,
            accent_dark: brush(&context, &theme::d2d(theme.on_accent))?,
            text: brush(&context, &theme::d2d(theme.text))?,
            muted: brush(&context, &theme::d2d(theme.text_muted))?,
            disabled: brush(&context, &theme::d2d(theme.text_disabled))?,
            track: brush(&context, &theme::d2d(theme.border_strong))?,
            focus: brush(&context, &theme::d2d(theme.accent_soft))?,
            divider: brush(&context, &theme::d2d(theme.divider))?,
            title_format,
            _bundled_fonts: bundled_fonts,
            brand_format,
            hero_format,
            onboarding_format,
            onboarding_body_format,
            onboarding_small_format,
            onboarding_button_format,
            body_format,
            small_format,
            button_format,
            material: backdrop::settings_material(),
            assets: SvgAssetCache::create().map_err(|error| asset_error(&error))?,
            embedded: BoundedResourceCache::new(16 * 1024 * 1024),
            rasters: BoundedResourceCache::new(16 * 1024 * 1024),
        };
        renderer.attach_target(swap_chain)?;
        Ok(renderer)
    }

    pub(super) fn detach_target(&mut self) {
        unsafe { self.context.SetTarget(None::<&ID2D1Image>) };
        self.target = None;
    }

    pub(super) fn attach_target(
        &mut self,
        chain: &IDXGISwapChain1,
    ) -> Result<(), WindowsError> {
        self.detach_target();
        let surface: IDXGISurface = unsafe { chain.GetBuffer(0)? };
        let properties = target_bitmap_properties();
        let target = unsafe {
            self.context
                .CreateBitmapFromDxgiSurface(&surface, Some(&raw const properties))?
        };
        unsafe { self.context.SetTarget(&target) };
        self.target = Some(target);
        Ok(())
    }

    pub(super) fn draw(
        &mut self,
        size: SurfaceSize,
        scene: &SettingsScene,
    ) -> Result<SettingsDrawResult, SettingsRendererError> {
        debug_assert!(self.target.is_some());
        self.apply_theme(scene);
        let layout = scene.layout();
        self.ensure_application_rasters(scene)?;
        let welcome_icon_size = if scene.onboarding_step() == Some(OnboardingStep::Welcome)
        {
            let size = NonZeroU32::new(scale(scene, 64))
                .expect("the scaled welcome icon is nonzero");
            self.ensure_embedded(
                SvgAsset::LotusPixel,
                size,
                IconTint::from_color(scene.theme().text),
            )?;
            Some(size)
        } else {
            None
        };
        if scene.page() == SettingsPage::Apps {
            let size = NonZeroU32::new(scale(scene, 18))
                .expect("the scaled application search icon is nonzero");
            self.ensure_embedded(
                SvgAsset::FluentSearch,
                size,
                IconTint::from_color(scene.theme().text_muted),
            )?;
        }
        let transparent = TRANSPARENT;
        let result = unsafe {
            self.context.BeginDraw();
            self.context.Clear(Some(&raw const transparent));
            self.draw_background(size, scene);
            if let Some(step) = scene.onboarding_step() {
                self.draw_onboarding(scene, &layout, step, welcome_icon_size)?;
            } else {
                self.draw_navigation(scene, &layout);
                self.draw_content(scene, &layout);
                self.draw_footer(scene);
            }
            self.context.EndDraw(None, None)
        };
        match result {
            Ok(()) => Ok(SettingsDrawResult::Complete),
            Err(error) if error.code() == D2DERR_RECREATE_TARGET => {
                Ok(SettingsDrawResult::RecreateTarget)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn apply_theme(&self, scene: &SettingsScene) {
        let value = scene.theme();
        let acrylic =
            self.material == SettingsMaterial::Acrylic && scene.draft().use_acrylic;
        let canvas = if acrylic {
            value.chrome_overlay
        } else {
            value.canvas
        };
        let sidebar = if acrylic {
            value.accent.with_alpha(0.38)
        } else {
            value.canvas.blend(value.accent, 0.18)
        };
        theme::set(&self.panel, canvas);
        theme::set(&self.sidebar, sidebar);
        theme::set(&self.sidebar_selected, value.text.with_alpha(0.14));
        theme::set(
            &self.group,
            if acrylic {
                value.control
            } else {
                value.surface
            },
        );
        theme::set(&self.row, value.control);
        theme::set(&self.hover, value.control_hover);
        theme::set(&self.selected, value.control_selected);
        theme::set(&self.accent, value.accent);
        theme::set(&self.accent_dark, value.on_accent);
        theme::set(&self.text, value.text);
        theme::set(&self.muted, value.text_muted);
        theme::set(&self.disabled, value.text_disabled);
        theme::set(&self.track, value.border_strong);
        theme::set(&self.focus, value.accent_soft);
        theme::set(&self.divider, value.divider);
    }

    fn draw_background(&self, size: SurfaceSize, scene: &SettingsScene) {
        let surface = D2D_RECT_F {
            left: 0.0,
            top: 0.0,
            right: as_f32(size.width()),
            bottom: as_f32(size.height()),
        };
        let sidebar = D2D_RECT_F {
            left: 0.0,
            top: 0.0,
            right: scale_f32(scene, 209.0),
            bottom: as_f32(size.height()),
        };
        let main = D2D_RECT_F {
            left: sidebar.right,
            ..surface
        };
        unsafe {
            if scene.onboarding_step().is_some() {
                self.context.FillRectangle(&raw const surface, &self.panel);
                return;
            }
            self.context.FillRectangle(&raw const main, &self.panel);
            self.context
                .FillRectangle(&raw const sidebar, &self.sidebar);
        }
    }

    fn ensure_embedded(
        &mut self,
        asset: SvgAsset,
        size: NonZeroU32,
        tint: IconTint,
    ) -> Result<(), SettingsRendererError> {
        let key = (asset, size);
        if self.embedded.get(&key).is_some() {
            return Ok(());
        }
        let raster = self
            .assets
            .rasterize(asset, RasterSize::square(size), tint)
            .map_err(|error| asset_error(&error))?;
        let bitmap = upload_bgra_pixels(
            &self.context,
            raster.size().width(),
            raster.size().height(),
            raster.pixels(),
            raster.stride().map_err(|error| asset_error(&error))?,
        )?;
        let bytes = usize::try_from(size.get())
            .unwrap_or(usize::MAX)
            .saturating_mul(usize::try_from(size.get()).unwrap_or(usize::MAX))
            .saturating_mul(4);
        self.embedded.insert(key, bitmap, bytes);
        Ok(())
    }

    fn embedded_bitmap(
        &self,
        asset: SvgAsset,
        size: NonZeroU32,
    ) -> Result<&ID2D1Bitmap1, SettingsRendererError> {
        self.embedded.peek(&(asset, size)).ok_or_else(|| {
            SettingsRendererError::Windows(WindowsError::new(
                E_FAIL,
                "uploaded onboarding artwork disappeared from the graphics cache",
            ))
        })
    }

    fn ensure_application_rasters(
        &mut self,
        scene: &SettingsScene,
    ) -> Result<(), SettingsRendererError> {
        for application in scene.applications() {
            if let Some(icon) = &application.icon {
                self.ensure_raster(icon)?;
            }
        }
        Ok(())
    }

    fn ensure_raster(&mut self, raster: &RasterIcon) -> Result<(), SettingsRendererError> {
        let key = raster_key(raster);
        if self.rasters.get(&key).is_some() {
            return Ok(());
        }
        let bitmap = upload_bgra_pixels(
            &self.context,
            raster.width(),
            raster.height(),
            raster.pixels(),
            raster.stride(),
        )?;
        let bytes = usize::try_from(raster.width())
            .unwrap_or(usize::MAX)
            .saturating_mul(usize::try_from(raster.height()).unwrap_or(usize::MAX))
            .saturating_mul(4);
        self.rasters.insert(key, bitmap, bytes);
        Ok(())
    }

    fn raster_bitmap(&self, raster: &RasterIcon) -> Option<&ID2D1Bitmap1> {
        self.rasters.peek(&raster_key(raster))
    }

    fn draw_footer(&self, scene: &SettingsScene) {
        if !scene.is_dirty() {
            return;
        }
        self.draw_text(
            "Unsaved changes",
            SettingsRect {
                left: scale(scene, 244),
                top: scene
                    .desired_size()
                    .height()
                    .saturating_sub(scale(scene, 72)),
                width: scale(scene, 240),
                height: scale(scene, 72),
            },
            &self.small_format,
            &self.muted,
            false,
        );
    }

    fn draw_close(&self, _scene: &SettingsScene, bounds: SettingsRect) {
        self.draw_text("×", bounds, &self.title_format, &self.muted, true);
    }

    fn draw_focus(
        &self,
        scene: &SettingsScene,
        control: SettingsControl,
        bounds: SettingsRect,
    ) {
        if !scene.focus_visible() || scene.focused() != Some(control) {
            return;
        }
        let outline = rounded(
            inset_rect(rect(bounds), scale_f32(scene, 2.0)),
            scale_f32(scene, scene.theme().radii.compact),
        );
        unsafe {
            self.context.DrawRoundedRectangle(
                &raw const outline,
                &self.focus,
                scale_f32(scene, 1.0),
                None,
            );
        }
    }

    fn draw_text(
        &self,
        value: &str,
        bounds: SettingsRect,
        format: &IDWriteTextFormat,
        brush: &ID2D1SolidColorBrush,
        centered: bool,
    ) {
        let text = utf16(value);
        let bounds = rect(bounds);
        unsafe {
            if centered {
                format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER).ok();
            }
            self.context.DrawText(
                &text,
                format,
                &raw const bounds,
                brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
            if centered {
                format
                    .SetTextAlignment(
                        windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT_LEADING,
                    )
                    .ok();
            }
        }
    }
}

#[derive(Debug, Error)]
pub(super) enum SettingsRendererError {
    #[error(transparent)]
    Windows(#[from] WindowsError),
}

fn asset_error(error: &AssetError) -> SettingsRendererError {
    SettingsRendererError::Windows(WindowsError::new(E_FAIL, error.to_string()))
}

fn toggle_label(value: SettingsToggle) -> &'static str {
    match value {
        SettingsToggle::UseAcrylic => "Acrylic",
        SettingsToggle::ShowAppDock => "Application dock",
        SettingsToggle::ShowUnpinnedRunningApps => "Show unpinned running applications",
        SettingsToggle::ShowRunningIndicators => "Show indicators for open applications",
        SettingsToggle::ShowOnAllMonitors => "Show lotus on every monitor",
        SettingsToggle::ShowDesktopButton => "Show a desktop button at the right edge",
        SettingsToggle::ShowSystemStatus => "Show system status",
        SettingsToggle::ShowVolumeStatus => "Show volume",
        SettingsToggle::ShowNetworkStatus => "Show network",
        SettingsToggle::ShowBackgroundAppsStatus => "Show background applications",
        SettingsToggle::ShowDateTimeStatus => "Show time",
        SettingsToggle::ShowDateInStatus => "Show date below the time",
        SettingsToggle::Use24HourTime => "Use 24-hour time",
        SettingsToggle::ShowMediaControls => "Show media controls",
        SettingsToggle::ShowMediaMetadata => "Show track information",
        SettingsToggle::StartWithWindows => "Start lotus when you sign in",
        SettingsToggle::ReplaceWindowsTaskbar => "Replace the Windows taskbar",
        SettingsToggle::HideWhenFullscreen => "Hide while an app is fullscreen",
        SettingsToggle::SearchEnabled => "Application search",
        SettingsToggle::SearchOpenWithWindowsKey => "Open search with the Windows key",
        SettingsToggle::AltTabEnabled => "Replace Alt+Tab with lotus",
    }
}

fn slider_label(value: SettingsSlider) -> &'static str {
    match value {
        SettingsSlider::IconSize => "Icon size",
        SettingsSlider::ItemSpacing => "Item spacing",
        SettingsSlider::HorizontalPadding => "Horizontal padding",
        SettingsSlider::VerticalPadding => "Vertical padding",
        SettingsSlider::BottomOffset => "Bottom offset",
        SettingsSlider::ScreenEdgeInset => "Screen edge inset",
        SettingsSlider::CornerRadius => "Corner radius",
        SettingsSlider::BackgroundOpacity => "Material opacity",
        SettingsSlider::SearchResultLimit => "Number of search results",
    }
}

fn slider_value_text(scene: &SettingsScene, slider: SettingsSlider) -> String {
    let value = scene.slider_value(slider);
    if slider == SettingsSlider::BackgroundOpacity {
        format!("{value}%")
    } else {
        value.to_string()
    }
}

fn text_format(
    factory: &IDWriteFactory,
    size: f32,
    weight: DWRITE_FONT_WEIGHT,
) -> Result<IDWriteTextFormat, WindowsError> {
    unsafe {
        factory.CreateTextFormat(
            w!("Segoe UI Variable Text"),
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-us"),
        )
    }
}

fn fraunces_format(
    factory: &IDWriteFactory6,
    collection: &IDWriteFontCollection1,
    size: f32,
    weight: f32,
) -> Result<IDWriteTextFormat, WindowsError> {
    let axes = [
        DWRITE_FONT_AXIS_VALUE {
            axisTag: DWRITE_FONT_AXIS_TAG_OPTICAL_SIZE,
            value: size.clamp(9.0, 144.0),
        },
        DWRITE_FONT_AXIS_VALUE {
            axisTag: DWRITE_FONT_AXIS_TAG_WEIGHT,
            value: weight,
        },
        DWRITE_FONT_AXIS_VALUE {
            axisTag: FRAUNCES_SOFTNESS,
            value: 0.0,
        },
        DWRITE_FONT_AXIS_VALUE {
            axisTag: FRAUNCES_WONK,
            value: 1.0,
        },
    ];
    let format = unsafe {
        factory.CreateTextFormat(w!("Fraunces"), collection, &axes, size, w!("en-us"))
    }?;
    format.cast()
}

const fn onboarding_title(step: OnboardingStep) -> &'static str {
    match step {
        OnboardingStep::Welcome => "lotus",
        OnboardingStep::Modules => "choose your lotus",
        OnboardingStep::Layout => "arrange your lotus",
        OnboardingStep::Integration => "integrate with windows",
        OnboardingStep::Ready => "thank you!",
    }
}

fn brush(
    context: &ID2D1DeviceContext,
    value: &D2D1_COLOR_F,
) -> Result<ID2D1SolidColorBrush, WindowsError> {
    unsafe { context.CreateSolidColorBrush(value, None) }
}

fn rect(value: SettingsRect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: as_f32(value.left),
        top: as_f32(value.top),
        right: as_f32(value.left.saturating_add(value.width)),
        bottom: as_f32(value.top.saturating_add(value.height)),
    }
}

fn is_page_content(control: SettingsControl) -> bool {
    matches!(
        control,
        SettingsControl::SurfacePreset
            | SettingsControl::AccentPreset
            | SettingsControl::ForegroundPreset
            | SettingsControl::NotificationBadgeStyle
            | SettingsControl::DockZone
            | SettingsControl::SystemStatusZone
            | SettingsControl::MediaZone
            | SettingsControl::Toggle(_)
            | SettingsControl::Slider(_)
            | SettingsControl::ChooseMascotImage
            | SettingsControl::ResetMascotImage
            | SettingsControl::ApplicationSearch
            | SettingsControl::ApplicationRow(_)
            | SettingsControl::ChooseApplicationIcon(_)
            | SettingsControl::ResetApplicationIcon(_)
            | SettingsControl::ReplaySetup
    )
}
fn rounded(rect: D2D_RECT_F, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect,
        radiusX: radius,
        radiusY: radius,
    }
}
fn inset(bounds: SettingsRect, horizontal: u32, vertical: u32) -> SettingsRect {
    SettingsRect {
        left: bounds.left.saturating_add(horizontal),
        top: bounds.top.saturating_add(vertical),
        width: bounds.width.saturating_sub(horizontal.saturating_mul(2)),
        height: bounds.height.saturating_sub(vertical.saturating_mul(2)),
    }
}
fn inset_all(bounds: SettingsRect, amount: u32) -> SettingsRect {
    inset(bounds, amount, amount)
}
fn picker_bounds(scene: &SettingsScene, bounds: SettingsRect) -> SettingsRect {
    scene.control_column(bounds)
}
fn inset_rect(mut rect: D2D_RECT_F, amount: f32) -> D2D_RECT_F {
    rect.left += amount;
    rect.top += amount;
    rect.right -= amount;
    rect.bottom -= amount;
    rect
}
fn outset_rect(mut rect: D2D_RECT_F, amount: f32) -> D2D_RECT_F {
    rect.left -= amount;
    rect.top -= amount;
    rect.right += amount;
    rect.bottom += amount;
    rect
}
fn utf16(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}
fn scale(scene: &SettingsScene, dips: u32) -> u32 {
    u32::try_from((u64::from(dips) * u64::from(scene.dpi()) + 48) / 96).unwrap_or(u32::MAX)
}
fn scale_f32(scene: &SettingsScene, dips: f32) -> f32 {
    as_f32(scene.dpi()) * dips / TARGET_DPI
}

const fn color(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}
#[allow(
    clippy::cast_precision_loss,
    reason = "settings dimensions remain below f32 exact range"
)]
const fn as_f32(value: u32) -> f32 {
    value as f32
}
