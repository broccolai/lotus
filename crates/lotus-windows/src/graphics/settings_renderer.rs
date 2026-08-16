use std::collections::HashMap;
use std::ffi::c_void;
use std::num::NonZeroU32;

use lotus_core::settings::{DockZone, NotificationBadgeStyle};
use lotus_settings::appearance::{AccentPreset, SurfacePreset};
use lotus_ui::theme::{Color, Theme};
use thiserror::Error;
use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, E_FAIL};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
    D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
    D2D1_ROUNDED_RECT, D2D1CreateFactory, ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext,
    ID2D1Factory1, ID2D1Image, ID2D1SolidColorBrush,
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
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::{IDXGISurface, IDXGISwapChain1};
use windows::core::{Error as WindowsError, Interface, w};

use super::assets::{AssetError, RasterSize, SvgAsset, SvgAssetCache};
use super::device::GraphicsDevice;
use super::settings_scene::{
    OnboardingModule, OnboardingStep, SettingsControl, SettingsLayout, SettingsPage,
    SettingsRect, SettingsScene, SettingsSlider, SettingsToggle, SettingsUpdateActivity,
};
use super::surface::SurfaceSize;
use super::theme;
use crate::font::BundledFontCollection;
use crate::platform::windows::backdrop::{self, SettingsMaterial};

const TARGET_DPI: f32 = 96.0;
const TRANSPARENT: D2D1_COLOR_F = color(0.0, 0.0, 0.0, 0.0);
const MATERIAL_CANVAS_ALPHA: f32 = 0.72;
const SETTINGS_SURFACE_ALPHA: f32 = 0.72;
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
    embedded: HashMap<(SvgAsset, NonZeroU32), ID2D1Bitmap1>,
}

impl SettingsRenderer {
    pub(super) fn create(
        graphics: &GraphicsDevice,
        swap_chain: &IDXGISwapChain1,
    ) -> Result<Self, SettingsRendererError> {
        let dxgi = graphics.dxgi_device()?;
        // SAFETY: A supported typed factory is requested without retained options.
        let factory: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        // SAFETY: The live DXGI device is compatible with the Direct2D factory.
        let device = unsafe { factory.CreateDevice(&dxgi)? };
        // SAFETY: The live Direct2D device returns an owned context.
        let context =
            unsafe { device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)? };
        // SAFETY: DirectWrite returns an owned shared factory.
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
        // SAFETY: Each retained format is live and accepts these valid layout values.
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
            embedded: HashMap::new(),
        };
        renderer.attach_target(swap_chain)?;
        Ok(renderer)
    }

    pub(super) fn detach_target(&mut self) {
        // SAFETY: A null target releases the swap-chain buffer before resize.
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
        // SAFETY: Surface and properties live for the synchronous creation call.
        let target = unsafe {
            self.context
                .CreateBitmapFromDxgiSurface(&surface, Some(&raw const properties))?
        };
        // SAFETY: The target bitmap belongs to this context.
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
        let welcome_icon_size = if scene.onboarding_step() == Some(OnboardingStep::Welcome)
        {
            let size = NonZeroU32::new(scale(scene, 64))
                .expect("the scaled welcome icon is nonzero");
            self.ensure_embedded(SvgAsset::LotusPixel, size)?;
            Some(size)
        } else {
            None
        };
        let transparent = TRANSPARENT;
        // SAFETY: The target, brushes, formats and local geometry remain live through EndDraw.
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
        let onboarding = scene.onboarding_step().is_some();
        let acrylic = self.material == SettingsMaterial::Acrylic;
        let canvas = value.canvas.with_alpha(if acrylic {
            MATERIAL_CANVAS_ALPHA
        } else {
            1.0
        });
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
            if onboarding {
                value.surface
            } else {
                value.surface.with_alpha(SETTINGS_SURFACE_ALPHA)
            },
        );
        theme::set(&self.row, value.control);
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
        // SAFETY: The active context and retained brushes are live for these finite rectangles.
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

    fn draw_navigation(&self, scene: &SettingsScene, layout: &SettingsLayout) {
        self.draw_text(
            "lotus",
            SettingsRect {
                left: scale(scene, 34),
                top: scale(scene, 18),
                width: scale(scene, 160),
                height: scale(scene, 44),
            },
            &self.brand_format,
            &self.text,
            false,
        );
        for page in SettingsPage::ALL {
            let control = SettingsControl::Navigate(page);
            let Some(bounds) = layout.bounds(control) else {
                continue;
            };
            let selected = scene.page() == page;
            if selected {
                let surface =
                    rounded(rect(bounds), scale_f32(scene, scene.theme().radii.control));
                let marker = SettingsRect {
                    left: bounds.left + scale(scene, 3),
                    top: bounds.top + scale(scene, 12),
                    width: scale(scene, 3),
                    height: bounds.height.saturating_sub(scale(scene, 24)),
                };
                let marker = rounded(rect(marker), scale_f32(scene, 1.5));
                // SAFETY: Active context, local geometry, and retained brushes are live.
                unsafe {
                    self.context
                        .FillRoundedRectangle(&raw const surface, &self.sidebar_selected);
                    self.context
                        .FillRoundedRectangle(&raw const marker, &self.text);
                };
            }
            self.draw_text(
                page.title(),
                if page == SettingsPage::About {
                    bounds
                } else {
                    inset(bounds, scale(scene, 20), 0)
                },
                &self.body_format,
                &self.text,
                page == SettingsPage::About,
            );
        }

        let Some(appearance) =
            layout.bounds(SettingsControl::Navigate(SettingsPage::Appearance))
        else {
            return;
        };
        let Some(taskbar) = layout.bounds(SettingsControl::Navigate(SettingsPage::Taskbar))
        else {
            return;
        };
        let top = appearance
            .top
            .saturating_add(appearance.height)
            .saturating_add(
                taskbar
                    .top
                    .saturating_sub(appearance.top + appearance.height)
                    / 2,
            );
        let divider = D2D_RECT_F {
            left: as_f32(appearance.left.saturating_add(scale(scene, 20))),
            top: as_f32(top),
            right: as_f32(
                appearance
                    .left
                    .saturating_add(appearance.width)
                    .saturating_sub(scale(scene, 20)),
            ),
            bottom: as_f32(top.saturating_add(scale(scene, 1))),
        };
        // SAFETY: The active context, finite local rectangle, and retained brush are live.
        unsafe {
            self.context
                .FillRectangle(&raw const divider, &self.divider);
        }
    }

    fn draw_content(&self, scene: &SettingsScene, layout: &SettingsLayout) {
        let viewport = rect(layout.content_viewport);
        // SAFETY: The active context accepts this finite scene-owned clip for the balanced
        // PushAxisAlignedClip/PopAxisAlignedClip pair below.
        unsafe {
            self.context.PushAxisAlignedClip(
                &raw const viewport,
                D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            );
        }
        for entry in &layout.sections {
            self.draw_text(
                entry.section.title(),
                inset(entry.bounds, scale(scene, 16), 0),
                &self.small_format,
                &self.muted,
                false,
            );
        }
        self.draw_content_group(scene, layout);
        if scene.page() == SettingsPage::About {
            self.draw_about(scene);
        }
        for entry in layout
            .controls
            .iter()
            .filter(|entry| is_page_content(entry.control))
        {
            self.draw_settings_control(scene, entry.control, entry.bounds);
        }
        // SAFETY: This balances the clip pushed immediately above on the same active context.
        unsafe {
            self.context.PopAxisAlignedClip();
        }

        if let Some(thumb) = layout.scrollbar_thumb {
            let thumb = rounded(rect(thumb), scale_f32(scene, 1.5));
            // SAFETY: The active context, local geometry, and retained muted brush are live.
            unsafe {
                self.context
                    .FillRoundedRectangle(&raw const thumb, &self.muted);
            }
        }

        for entry in layout
            .controls
            .iter()
            .filter(|entry| !is_page_content(entry.control))
        {
            self.draw_settings_control(scene, entry.control, entry.bounds);
        }
    }

    fn draw_settings_control(
        &self,
        scene: &SettingsScene,
        control: SettingsControl,
        bounds: SettingsRect,
    ) {
        match control {
            SettingsControl::SurfacePreset => self.draw_surface_picker(scene, bounds),
            SettingsControl::AccentPreset => self.draw_accent_picker(scene, bounds),
            SettingsControl::NotificationBadgeStyle => {
                self.draw_notification_badge_style(scene, bounds);
            }
            SettingsControl::DockZone => self.draw_zone_picker(scene, bounds, false),
            SettingsControl::SystemStatusZone => self.draw_zone_picker(scene, bounds, true),
            SettingsControl::MediaZone => self.draw_media_zone_picker(scene, bounds),
            SettingsControl::Toggle(toggle) => self.draw_toggle(scene, bounds, toggle),
            SettingsControl::Slider(slider) => self.draw_slider(scene, bounds, slider),
            SettingsControl::ChooseMascotImage => self.draw_mascot_image(scene, bounds),
            SettingsControl::ResetMascotImage => self.draw_reset_mascot(scene, bounds),
            SettingsControl::CheckForUpdates => self.draw_check_for_updates(scene, bounds),
            SettingsControl::ReplaySetup => self.draw_button(
                scene,
                bounds,
                SettingsControl::ReplaySetup,
                "Run first setup again",
                true,
                ButtonEmphasis::Secondary,
            ),
            SettingsControl::Revert => self.draw_revert(scene, bounds),
            SettingsControl::Apply => self.draw_apply(scene, bounds),
            SettingsControl::Close => self.draw_close(scene, bounds),
            SettingsControl::Navigate(_)
            | SettingsControl::OnboardingModule(_)
            | SettingsControl::OnboardingZone(_)
            | SettingsControl::OnboardingBack
            | SettingsControl::OnboardingNext
            | SettingsControl::OnboardingFinish => {}
        }
    }

    fn draw_onboarding(
        &self,
        scene: &SettingsScene,
        layout: &SettingsLayout,
        step: OnboardingStep,
        welcome_icon_size: Option<NonZeroU32>,
    ) -> Result<(), SettingsRendererError> {
        let title = onboarding_title(step);
        let title_bounds = match step {
            OnboardingStep::Welcome => SettingsRect {
                left: scale(scene, 72),
                top: scale(scene, 220),
                width: scale(scene, 756),
                height: scale(scene, 130),
            },
            OnboardingStep::Ready => SettingsRect {
                left: scale(scene, 72),
                top: scale(scene, 136),
                width: scale(scene, 756),
                height: scale(scene, 72),
            },
            _ => SettingsRect {
                left: scale(scene, 72),
                top: scale(scene, 110),
                width: scale(scene, 756),
                height: scale(scene, 58),
            },
        };
        self.draw_text(
            title,
            title_bounds,
            if step == OnboardingStep::Welcome {
                &self.hero_format
            } else {
                &self.onboarding_format
            },
            if step == OnboardingStep::Welcome {
                &self.accent
            } else {
                &self.text
            },
            true,
        );
        if let Some(icon_size) = welcome_icon_size {
            self.draw_welcome_icon(scene, icon_size)?;
        }
        if step == OnboardingStep::Ready {
            self.draw_onboarding_ready(scene);
        }
        if step != OnboardingStep::Welcome {
            self.draw_onboarding_progress(scene, step);
        }

        for entry in &layout.controls {
            match entry.control {
                SettingsControl::OnboardingModule(module) => {
                    self.draw_onboarding_module(scene, entry.bounds, module);
                }
                SettingsControl::OnboardingZone(module) => {
                    self.draw_onboarding_zone(scene, entry.bounds, module);
                }
                SettingsControl::Toggle(toggle) => {
                    self.draw_toggle(scene, entry.bounds, toggle);
                }
                SettingsControl::OnboardingBack => self.draw_button(
                    scene,
                    entry.bounds,
                    entry.control,
                    "back",
                    true,
                    ButtonEmphasis::Outline,
                ),
                SettingsControl::OnboardingNext => self.draw_button(
                    scene,
                    entry.bounds,
                    entry.control,
                    if step == OnboardingStep::Welcome {
                        "begin"
                    } else {
                        "continue"
                    },
                    true,
                    ButtonEmphasis::Primary,
                ),
                SettingsControl::OnboardingFinish => self.draw_button(
                    scene,
                    entry.bounds,
                    entry.control,
                    "start lotus",
                    true,
                    ButtonEmphasis::Primary,
                ),
                SettingsControl::Close => self.draw_close(scene, entry.bounds),
                _ => {}
            }
        }
        Ok(())
    }

    fn draw_welcome_icon(
        &self,
        scene: &SettingsScene,
        size: NonZeroU32,
    ) -> Result<(), SettingsRendererError> {
        let bitmap = self.embedded_bitmap(SvgAsset::LotusPixel, size)?;
        let destination = rect(SettingsRect {
            left: scale(scene, 418),
            top: scale(scene, 180),
            width: size.get(),
            height: size.get(),
        });
        // SAFETY: The bitmap and destination remain live through the synchronous draw.
        unsafe {
            self.context.DrawBitmap(
                bitmap,
                Some(&raw const destination),
                1.0,
                D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                None,
                None,
            );
        }
        Ok(())
    }

    fn ensure_embedded(
        &mut self,
        asset: SvgAsset,
        size: NonZeroU32,
    ) -> Result<(), SettingsRendererError> {
        let key = (asset, size);
        if self.embedded.contains_key(&key) {
            return Ok(());
        }
        let raster = self
            .assets
            .rasterize(asset, RasterSize::square(size))
            .map_err(|error| asset_error(&error))?;
        let bitmap = upload_bitmap(
            &self.context,
            raster.size().width(),
            raster.size().height(),
            raster.pixels(),
            raster.stride().map_err(|error| asset_error(&error))?,
        )?;
        self.embedded.insert(key, bitmap);
        Ok(())
    }

    fn embedded_bitmap(
        &self,
        asset: SvgAsset,
        size: NonZeroU32,
    ) -> Result<&ID2D1Bitmap1, SettingsRendererError> {
        self.embedded.get(&(asset, size)).ok_or_else(|| {
            SettingsRendererError::Windows(WindowsError::new(
                E_FAIL,
                "uploaded onboarding artwork disappeared from the graphics cache",
            ))
        })
    }

    fn draw_onboarding_progress(&self, scene: &SettingsScene, step: OnboardingStep) {
        let track_bounds = SettingsRect {
            left: scale(scene, 390),
            top: scale(scene, 28),
            width: scale(scene, 120),
            height: scale(scene, 3),
        };
        let progress_bounds = SettingsRect {
            width: track_bounds.width.saturating_mul(step.number()) / 4,
            ..track_bounds
        };
        let track = rounded(rect(track_bounds), scale_f32(scene, 1.5));
        let progress = rounded(rect(progress_bounds), scale_f32(scene, 1.5));
        // SAFETY: The active context, local geometry, and retained brushes remain live.
        unsafe {
            self.context
                .FillRoundedRectangle(&raw const track, &self.divider);
            self.context
                .FillRoundedRectangle(&raw const progress, &self.accent);
        }
    }

    fn draw_onboarding_ready(&self, scene: &SettingsScene) {
        self.draw_text(
            "you can change these choices and much more in lotus settings.",
            SettingsRect {
                left: scale(scene, 170),
                top: scale(scene, 226),
                width: scale(scene, 560),
                height: scale(scene, 28),
            },
            &self.onboarding_body_format,
            &self.text,
            true,
        );
        self.draw_text(
            "right-click the lotus icon or search >settings.",
            SettingsRect {
                left: scale(scene, 210),
                top: scale(scene, 262),
                width: scale(scene, 480),
                height: scale(scene, 26),
            },
            &self.onboarding_body_format,
            &self.muted,
            true,
        );
    }

    fn draw_onboarding_module(
        &self,
        scene: &SettingsScene,
        bounds: SettingsRect,
        module: OnboardingModule,
    ) {
        let enabled = scene.onboarding_module_enabled(module);
        let surface = rounded(rect(bounds), scale_f32(scene, scene.theme().radii.panel));
        let brush = if enabled {
            &self.selected
        } else if scene.hovered() == Some(SettingsControl::OnboardingModule(module)) {
            &self.row
        } else {
            &self.group
        };
        // SAFETY: The active context, retained brushes, and local geometry remain live.
        unsafe {
            self.context.FillRoundedRectangle(&raw const surface, brush);
        }
        self.draw_text(
            module.title(),
            SettingsRect {
                left: bounds.left + scale(scene, 18),
                top: bounds.top + scale(scene, 7),
                width: bounds.width - scale(scene, 36),
                height: scale(scene, 28),
            },
            &self.onboarding_body_format,
            &self.text,
            false,
        );
        self.draw_text(
            module.description(),
            SettingsRect {
                left: bounds.left + scale(scene, 18),
                top: bounds.top + scale(scene, 34),
                width: bounds.width - scale(scene, 36),
                height: scale(scene, 24),
            },
            &self.onboarding_small_format,
            &self.muted,
            false,
        );
        self.draw_focus(scene, SettingsControl::OnboardingModule(module), bounds);
    }

    fn draw_onboarding_zone(
        &self,
        scene: &SettingsScene,
        bounds: SettingsRect,
        module: OnboardingModule,
    ) {
        self.draw_text(
            module.title(),
            SettingsRect {
                left: bounds.left.saturating_sub(scale(scene, 260)),
                top: bounds.top,
                width: scale(scene, 230),
                height: bounds.height,
            },
            &self.onboarding_body_format,
            &self.text,
            false,
        );
        let selector = rounded(rect(bounds), scale_f32(scene, scene.theme().radii.control));
        // SAFETY: The active context, local geometry, and retained brush remain live.
        unsafe {
            self.context
                .FillRoundedRectangle(&raw const selector, &self.group);
        };
        let selected = scene.onboarding_zone(module);
        let segment_width = bounds.width / 3;
        for (index, (label, zone)) in [
            ("LEFT", DockZone::Left),
            ("MIDDLE", DockZone::Center),
            ("RIGHT", DockZone::Right),
        ]
        .into_iter()
        .enumerate()
        {
            let index = u32::try_from(index).unwrap_or_default();
            let segment = SettingsRect {
                left: bounds.left + index * segment_width,
                top: bounds.top,
                width: if index == 2 {
                    bounds.width - segment_width * 2
                } else {
                    segment_width
                },
                height: bounds.height,
            };
            if selected == zone {
                let selected_surface = rounded(
                    rect(inset_all(segment, scale(scene, 3))),
                    scale_f32(scene, scene.theme().radii.compact),
                );
                // SAFETY: The active context, local geometry, and retained brush remain live.
                unsafe {
                    self.context
                        .FillRoundedRectangle(&raw const selected_surface, &self.selected);
                }
            }
            self.draw_text(
                label,
                segment,
                &self.onboarding_small_format,
                if selected == zone {
                    &self.accent
                } else {
                    &self.muted
                },
                true,
            );
        }
        self.draw_focus(scene, SettingsControl::OnboardingZone(module), bounds);
    }

    fn draw_content_group(&self, scene: &SettingsScene, layout: &SettingsLayout) {
        let controls: Vec<_> = layout
            .controls
            .iter()
            .filter(|entry| {
                matches!(
                    entry.control,
                    SettingsControl::SurfacePreset
                        | SettingsControl::AccentPreset
                        | SettingsControl::NotificationBadgeStyle
                        | SettingsControl::DockZone
                        | SettingsControl::SystemStatusZone
                        | SettingsControl::MediaZone
                        | SettingsControl::Toggle(_)
                        | SettingsControl::Slider(_)
                        | SettingsControl::ChooseMascotImage
                        | SettingsControl::ResetMascotImage
                )
            })
            .collect();
        if controls.is_empty() {
            return;
        }
        for pair in controls.windows(2) {
            let [entry, next] = pair else {
                continue;
            };
            let gap = next
                .bounds
                .top
                .saturating_sub(entry.bounds.top.saturating_add(entry.bounds.height));
            if gap > scale(scene, 8) {
                continue;
            }
            let y = entry
                .bounds
                .top
                .saturating_add(entry.bounds.height)
                .saturating_add(scale(scene, 2));
            let divider = D2D_RECT_F {
                left: as_f32(entry.bounds.left.saturating_add(scale(scene, 16))),
                top: as_f32(y),
                right: as_f32(
                    entry
                        .bounds
                        .left
                        .saturating_add(entry.bounds.width)
                        .saturating_sub(scale(scene, 16)),
                ),
                bottom: as_f32(y.saturating_add(scale(scene, 1))),
            };
            // SAFETY: Active context, local rectangle, and retained divider brush are live.
            unsafe {
                self.context
                    .FillRectangle(&raw const divider, &self.divider);
            };
        }
    }

    fn draw_surface_picker(&self, scene: &SettingsScene, bounds: SettingsRect) {
        self.draw_text(
            "Surface",
            inset(bounds, scale(scene, 16), 0),
            &self.body_format,
            &self.text,
            false,
        );
        let picker = picker_bounds(scene, bounds);
        let segment_width = picker.width / 4;
        let selected = SurfacePreset::selected(scene.draft());
        for index in 0_u32..4 {
            let segment = SettingsRect {
                left: picker
                    .left
                    .saturating_add(index.saturating_mul(segment_width)),
                top: picker.top,
                width: if index == 3 {
                    picker.width.saturating_sub(segment_width.saturating_mul(3))
                } else {
                    segment_width
                },
                height: picker.height,
            };
            let preset = usize::try_from(index)
                .ok()
                .and_then(|index| SurfacePreset::ALL.get(index));
            let color = preset.map_or_else(
                || {
                    Color::from_hex(&scene.draft().background_color)
                        .unwrap_or(scene.theme().canvas)
                },
                |preset| Color::from_hex(preset.color()).unwrap_or(scene.theme().canvas),
            );
            let label = preset.map_or("Custom", |preset| preset.name());
            let is_selected =
                preset.map_or(selected.is_none(), |preset| selected == Some(*preset));
            let surface = rounded(
                rect(inset_all(segment, scale(scene, 2))),
                scale_f32(scene, scene.theme().radii.compact),
            );
            theme::set(&self.row, color);
            // SAFETY: The active context, retained brush, and local geometry remain live.
            unsafe {
                self.context
                    .FillRoundedRectangle(&raw const surface, &self.row);
            };
            if is_selected {
                let outline = rounded(
                    inset_rect(surface.rect, scale_f32(scene, 0.5)),
                    scale_f32(scene, (scene.theme().radii.compact - 0.5).max(1.0)),
                );
                // SAFETY: The active context, retained brush, and local outline remain live.
                unsafe {
                    self.context.DrawRoundedRectangle(
                        &raw const outline,
                        &self.accent,
                        scale_f32(scene, 1.5),
                        None,
                    );
                }
            }
            self.draw_text(label, segment, &self.small_format, &self.text, true);
        }
        theme::set(&self.row, scene.theme().control);
        self.draw_focus(scene, SettingsControl::SurfacePreset, bounds);
    }

    fn draw_accent_picker(&self, scene: &SettingsScene, bounds: SettingsRect) {
        self.draw_text(
            "Accent",
            inset(bounds, scale(scene, 16), 0),
            &self.body_format,
            &self.text,
            false,
        );
        let picker = picker_bounds(scene, bounds);
        let segment_width = picker.width / 6;
        let selected = AccentPreset::selected(scene.draft());
        for index in 0_u32..6 {
            let diameter = scale(scene, 18);
            let left = picker
                .left
                .saturating_add(index.saturating_mul(segment_width))
                .saturating_add(segment_width.saturating_sub(diameter) / 2);
            let swatch_bounds = SettingsRect {
                left,
                top: bounds
                    .top
                    .saturating_add(bounds.height.saturating_sub(diameter) / 2),
                width: diameter,
                height: diameter,
            };
            let swatch = rounded(rect(swatch_bounds), as_f32(diameter) * 0.5);
            let preset = usize::try_from(index)
                .ok()
                .and_then(|index| AccentPreset::ALL.get(index));
            let color = preset.map_or_else(
                || {
                    Color::from_hex(&scene.draft().accent_color)
                        .unwrap_or(scene.theme().accent)
                },
                |preset| Color::from_hex(preset.color()).unwrap_or(scene.theme().accent),
            );
            theme::set(&self.row, color);
            // SAFETY: The active context, retained brush, and local geometry remain live.
            unsafe {
                self.context
                    .FillRoundedRectangle(&raw const swatch, &self.row);
            };
            let is_selected =
                preset.map_or(selected.is_none(), |preset| selected == Some(*preset));
            if is_selected {
                let outline = rounded(
                    outset_rect(swatch.rect, scale_f32(scene, 3.0)),
                    as_f32(diameter) * 0.5 + scale_f32(scene, 3.0),
                );
                // SAFETY: The active context, retained brush, and local outline remain live.
                unsafe {
                    self.context.DrawRoundedRectangle(
                        &raw const outline,
                        &self.text,
                        scale_f32(scene, 1.0),
                        None,
                    );
                }
            }
            if preset.is_none() {
                self.draw_text(
                    "+",
                    swatch_bounds,
                    &self.small_format,
                    &self.accent_dark,
                    true,
                );
            }
        }
        theme::set(&self.row, scene.theme().control);
        self.draw_focus(scene, SettingsControl::AccentPreset, bounds);
    }

    fn draw_notification_badge_style(&self, scene: &SettingsScene, bounds: SettingsRect) {
        let selected = scene.draft().notification_badge_style;
        self.draw_text_segments(
            scene,
            bounds,
            SettingsControl::NotificationBadgeStyle,
            "Notification badges",
            &[
                ("Off", selected == NotificationBadgeStyle::Off),
                ("Dot", selected == NotificationBadgeStyle::Dot),
                ("Number", selected == NotificationBadgeStyle::Count),
            ],
        );
    }

    fn draw_zone_picker(&self, scene: &SettingsScene, bounds: SettingsRect, status: bool) {
        let selected = if status {
            scene.draft().system_status_zone
        } else {
            scene.draft().dock_zone
        };
        self.draw_text_segments(
            scene,
            bounds,
            if status {
                SettingsControl::SystemStatusZone
            } else {
                SettingsControl::DockZone
            },
            if status {
                "System status position"
            } else {
                "Main dock position"
            },
            &[
                ("Left", selected == DockZone::Left),
                ("Centre", selected == DockZone::Center),
                ("Right", selected == DockZone::Right),
            ],
        );
    }

    fn draw_media_zone_picker(&self, scene: &SettingsScene, bounds: SettingsRect) {
        let selected = scene.draft().media_zone;
        self.draw_text_segments(
            scene,
            bounds,
            SettingsControl::MediaZone,
            "Media position",
            &[
                ("Left", selected == DockZone::Left),
                ("Centre", selected == DockZone::Center),
                ("Right", selected == DockZone::Right),
            ],
        );
    }

    fn draw_text_segments(
        &self,
        scene: &SettingsScene,
        bounds: SettingsRect,
        control: SettingsControl,
        label: &str,
        options: &[(&str, bool)],
    ) {
        self.draw_text(
            label,
            inset(bounds, scale(scene, 16), 0),
            &self.body_format,
            &self.text,
            false,
        );
        let picker = picker_bounds(scene, bounds);
        let count = u32::try_from(options.len()).unwrap_or(1).max(1);
        let segment_width = picker.width / count;
        for (index, (label, selected)) in options.iter().enumerate() {
            let index = u32::try_from(index).unwrap_or_default();
            let segment = SettingsRect {
                left: picker
                    .left
                    .saturating_add(index.saturating_mul(segment_width)),
                top: picker.top,
                width: if index + 1 == count {
                    picker
                        .width
                        .saturating_sub(segment_width.saturating_mul(index))
                } else {
                    segment_width
                },
                height: picker.height,
            };
            let surface = rounded(
                rect(inset_all(segment, scale(scene, 2))),
                scale_f32(scene, scene.theme().radii.compact),
            );
            // SAFETY: Active context, retained brushes, and local geometry remain live.
            unsafe {
                self.context.FillRoundedRectangle(
                    &raw const surface,
                    if *selected {
                        &self.selected
                    } else {
                        &self.row
                    },
                );
                if *selected {
                    self.context.DrawRoundedRectangle(
                        &raw const surface,
                        &self.accent,
                        scale_f32(scene, 1.0),
                        None,
                    );
                }
            }
            self.draw_text(label, segment, &self.small_format, &self.text, true);
        }
        self.draw_focus(scene, control, bounds);
    }

    fn draw_about(&self, scene: &SettingsScene) {
        self.draw_text(
            concat!("lotus ", env!("CARGO_PKG_VERSION")),
            SettingsRect {
                left: scale(scene, 260),
                top: scale(scene, 106),
                width: scale(scene, 600),
                height: scale(scene, 32),
            },
            &self.title_format,
            &self.text,
            false,
        );
        self.draw_text(
            "<3 broccoli",
            SettingsRect {
                left: scale(scene, 260),
                top: scale(scene, 148),
                width: scale(scene, 600),
                height: scale(scene, 32),
            },
            &self.body_format,
            &self.accent,
            false,
        );
    }

    fn draw_toggle(
        &self,
        scene: &SettingsScene,
        bounds: SettingsRect,
        toggle: SettingsToggle,
    ) {
        let label_format = if scene.onboarding_step().is_some() {
            &self.onboarding_body_format
        } else {
            &self.body_format
        };
        self.draw_text(
            toggle_label(toggle),
            inset(bounds, scale(scene, 16), 0),
            label_format,
            &self.text,
            false,
        );
        let switch = SettingsRect {
            left: bounds.left + bounds.width - scale(scene, 58),
            top: bounds.top + scale(scene, 11),
            width: scale(scene, 42),
            height: scale(scene, 24),
        };
        let switch_rect = rounded(rect(switch), as_f32(switch.height) * 0.5);
        let on = scene.toggle(toggle);
        let knob_size = scale(scene, 18);
        let knob = SettingsRect {
            left: if on {
                switch.left + switch.width - knob_size - scale(scene, 3)
            } else {
                switch.left + scale(scene, 3)
            },
            top: switch.top + scale(scene, 3),
            width: knob_size,
            height: knob_size,
        };
        let knob_rect = rounded(rect(knob), as_f32(knob_size) * 0.5);
        // SAFETY: Active context and geometry/brushes are live for synchronous drawing.
        unsafe {
            self.context.FillRoundedRectangle(
                &raw const switch_rect,
                if on {
                    &self.accent
                } else {
                    &self.track
                },
            );
            self.context.FillRoundedRectangle(
                &raw const knob_rect,
                if on {
                    &self.accent_dark
                } else {
                    &self.text
                },
            );
        }
    }

    fn draw_slider(
        &self,
        scene: &SettingsScene,
        bounds: SettingsRect,
        slider: SettingsSlider,
    ) {
        self.draw_text(
            slider_label(slider),
            inset(bounds, scale(scene, 16), 0),
            &self.body_format,
            &self.text,
            false,
        );
        let (track_left, track_width) = scene.slider_track(bounds);
        let track = SettingsRect {
            left: track_left,
            top: bounds.top + scale(scene, 21),
            width: track_width,
            height: scale(scene, 4),
        };
        let (minimum, maximum) = slider.range();
        let value = scene.slider_value(slider);
        let filled = track.width.saturating_mul(value - minimum) / (maximum - minimum);
        let fill = SettingsRect {
            width: filled,
            ..track
        };
        let knob = SettingsRect {
            left: track.left + filled.saturating_sub(scale(scene, 7)),
            top: track.top.saturating_sub(scale(scene, 5)),
            width: scale(scene, 14),
            height: scale(scene, 14),
        };
        let track_round = rounded(rect(track), as_f32(track.height) * 0.5);
        let fill_round = rounded(rect(fill), as_f32(fill.height) * 0.5);
        let knob_round = rounded(rect(knob), as_f32(knob.height) * 0.5);
        // SAFETY: Active context and local geometry remain live.
        unsafe {
            self.context
                .FillRoundedRectangle(&raw const track_round, &self.track);
            self.context
                .FillRoundedRectangle(&raw const fill_round, &self.accent);
            self.context
                .FillRoundedRectangle(&raw const knob_round, &self.accent);
        }
        let value_bounds = scene.slider_value_bounds(bounds);
        let value_surface = rounded(
            rect(value_bounds),
            scale_f32(scene, scene.theme().radii.compact),
        );
        // SAFETY: Active context, retained brushes, and local geometry remain live.
        unsafe {
            self.context
                .FillRoundedRectangle(&raw const value_surface, &self.row);
            self.context.DrawRoundedRectangle(
                &raw const value_surface,
                &self.divider,
                scale_f32(scene, 1.0),
                None,
            );
        }
        let value_text = slider_value_text(scene, slider);
        self.draw_text(
            &value_text,
            value_bounds,
            &self.small_format,
            &self.muted,
            true,
        );
    }

    fn draw_mascot_image(&self, scene: &SettingsScene, bounds: SettingsRect) {
        self.draw_text(
            "Dock image",
            inset(bounds, scale(scene, 16), 0),
            &self.body_format,
            &self.text,
            false,
        );
        let value = if scene.draft().mascot_image_path.is_some() {
            "Change image"
        } else {
            "Choose image"
        };
        self.draw_text(
            value,
            SettingsRect {
                left: bounds
                    .left
                    .saturating_add(bounds.width)
                    .saturating_sub(scale(scene, 142)),
                top: bounds.top,
                width: scale(scene, 126),
                height: bounds.height,
            },
            &self.small_format,
            &self.accent,
            true,
        );
    }

    fn draw_reset_mascot(&self, scene: &SettingsScene, bounds: SettingsRect) {
        self.draw_text(
            "Restore lotus icon",
            inset(bounds, scale(scene, 16), 0),
            &self.body_format,
            &self.muted,
            false,
        );
    }

    fn draw_check_for_updates(&self, scene: &SettingsScene, bounds: SettingsRect) {
        let enabled = scene.update_activity() == SettingsUpdateActivity::Idle;
        let label = match scene.update_activity() {
            SettingsUpdateActivity::Idle if scene.is_installed() => "Check for updates",
            SettingsUpdateActivity::Idle => "Install lotus",
            SettingsUpdateActivity::Checking => "Checking…",
            SettingsUpdateActivity::Installing => "Installing…",
        };
        self.draw_button(
            scene,
            bounds,
            SettingsControl::CheckForUpdates,
            label,
            enabled,
            ButtonEmphasis::Secondary,
        );
    }

    fn draw_apply(&self, scene: &SettingsScene, bounds: SettingsRect) {
        self.draw_button(
            scene,
            bounds,
            SettingsControl::Apply,
            "Apply",
            scene.is_dirty(),
            ButtonEmphasis::Primary,
        );
    }

    fn draw_revert(&self, scene: &SettingsScene, bounds: SettingsRect) {
        self.draw_button(
            scene,
            bounds,
            SettingsControl::Revert,
            "Revert",
            scene.is_dirty(),
            ButtonEmphasis::Outline,
        );
    }

    fn draw_button(
        &self,
        scene: &SettingsScene,
        bounds: SettingsRect,
        control: SettingsControl,
        label: &str,
        enabled: bool,
        emphasis: ButtonEmphasis,
    ) {
        let hovered = enabled && scene.hovered() == Some(control);
        let onboarding = matches!(
            control,
            SettingsControl::OnboardingBack
                | SettingsControl::OnboardingNext
                | SettingsControl::OnboardingFinish
        );
        let radius = if onboarding {
            10.0
        } else {
            scene.theme().radii.control
        };
        let surface = rounded(rect(bounds), scale_f32(scene, radius));
        let fill = match (emphasis, enabled, hovered) {
            (ButtonEmphasis::Primary, true, _) => Some(&self.accent),
            (ButtonEmphasis::Secondary | ButtonEmphasis::Outline, true, true) => {
                Some(&self.selected)
            }
            (ButtonEmphasis::Secondary, _, _) => Some(&self.row),
            (ButtonEmphasis::Primary | ButtonEmphasis::Outline, _, _) => None,
        };
        let border = if emphasis == ButtonEmphasis::Primary && enabled {
            &self.accent
        } else {
            &self.divider
        };
        // SAFETY: Active context, retained brushes, and local geometry remain live.
        unsafe {
            if let Some(fill) = fill {
                self.context.FillRoundedRectangle(&raw const surface, fill);
            }
            if !(onboarding && emphasis == ButtonEmphasis::Outline) {
                self.context.DrawRoundedRectangle(
                    &raw const surface,
                    border,
                    scale_f32(scene, 1.0),
                    None,
                );
            }
        }
        let text = match (emphasis, enabled) {
            (_, false) => &self.disabled,
            (ButtonEmphasis::Primary, true) => &self.accent_dark,
            (ButtonEmphasis::Secondary | ButtonEmphasis::Outline, true) => &self.text,
        };
        let format = if onboarding {
            &self.onboarding_button_format
        } else {
            &self.button_format
        };
        self.draw_text(label, bounds, format, text, true);
        self.draw_focus(scene, control, bounds);
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
        // SAFETY: Active context and retained brush remain live.
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
        // SAFETY: Text, format, bounds and brush remain live for the synchronous call.
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
    // SAFETY: Static family and locale strings are NUL terminated.
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
    // SAFETY: Static family and locale strings are NUL terminated.
    let format = unsafe {
        factory.CreateTextFormat(w!("Fraunces"), collection, &axes, size, w!("en-us"))
    }?;
    format.cast()
}

fn upload_bitmap(
    context: &ID2D1DeviceContext,
    width: u32,
    height: u32,
    pixels: &[u8],
    stride: u32,
) -> Result<ID2D1Bitmap1, WindowsError> {
    let properties = source_properties();
    // SAFETY: The source contains premultiplied BGRA bytes and remains live through the copy.
    unsafe {
        context.CreateBitmap(
            D2D_SIZE_U { width, height },
            Some(pixels.as_ptr().cast::<c_void>()),
            stride,
            &raw const properties,
        )
    }
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
    // SAFETY: Direct2D copies the color during this synchronous call.
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
            | SettingsControl::NotificationBadgeStyle
            | SettingsControl::DockZone
            | SettingsControl::SystemStatusZone
            | SettingsControl::MediaZone
            | SettingsControl::Toggle(_)
            | SettingsControl::Slider(_)
            | SettingsControl::ChooseMascotImage
            | SettingsControl::ResetMascotImage
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
