use lotus_core::settings::{DockZone, NotificationBadgeStyle};
use lotus_settings::appearance::{AccentPreset, SurfacePreset};
use lotus_ui::theme::{Color, Theme};
use thiserror::Error;
use windows::Win32::Foundation::D2DERR_RECREATE_TARGET;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_ROUNDED_RECT, D2D1CreateFactory, ID2D1Bitmap1,
    ID2D1Device, ID2D1DeviceContext, ID2D1Factory1, ID2D1Image, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory,
    IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::{IDXGISurface, IDXGISwapChain1};
use windows::core::{Error as WindowsError, w};

use super::device::GraphicsDevice;
use super::settings_scene::{
    SettingsControl, SettingsLayout, SettingsPage, SettingsRect, SettingsScene,
    SettingsSlider, SettingsToggle, SettingsUpdateActivity,
};
use super::surface::SurfaceSize;
use super::theme;

const TARGET_DPI: f32 = 96.0;
const TRANSPARENT: D2D1_COLOR_F = color(0.0, 0.0, 0.0, 0.0);

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
    body_format: IDWriteTextFormat,
    small_format: IDWriteTextFormat,
    button_format: IDWriteTextFormat,
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
        let write_factory: IDWriteFactory =
            unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let title_format = text_format(&write_factory, 18.0, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
        let body_format = text_format(&write_factory, 14.0, DWRITE_FONT_WEIGHT_NORMAL)?;
        let small_format = text_format(&write_factory, 12.5, DWRITE_FONT_WEIGHT_NORMAL)?;
        let button_format =
            text_format(&write_factory, 13.5, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
        // SAFETY: Each retained format is live and accepts these valid layout values.
        unsafe {
            title_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
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
            body_format,
            small_format,
            button_format,
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
        &self,
        size: SurfaceSize,
        scene: &SettingsScene,
    ) -> Result<SettingsDrawResult, SettingsRendererError> {
        debug_assert!(self.target.is_some());
        self.apply_theme(&scene.theme());
        let layout = scene.layout();
        let transparent = TRANSPARENT;
        // SAFETY: The target, brushes, formats and local geometry remain live through EndDraw.
        let result = unsafe {
            self.context.BeginDraw();
            self.context.Clear(Some(&raw const transparent));
            self.draw_background(size, scene);
            self.draw_navigation(scene, &layout);
            self.draw_content(scene, &layout);
            self.draw_footer(scene);
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

    fn apply_theme(&self, value: &Theme) {
        theme::set(&self.panel, value.canvas);
        theme::set(&self.group, value.surface);
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
        let divider = D2D_RECT_F {
            left: scale_f32(scene, 208.0),
            top: 0.0,
            right: scale_f32(scene, 209.0),
            bottom: as_f32(size.height()),
        };
        let footer = D2D_RECT_F {
            left: scale_f32(scene, 209.0),
            top: (as_f32(size.height()) - scale_f32(scene, 72.0)).max(0.0),
            right: as_f32(size.width()),
            bottom: as_f32(size.height()),
        };
        let footer_divider = D2D_RECT_F {
            bottom: footer.top + scale_f32(scene, 1.0),
            ..footer
        };
        // SAFETY: The active context and retained brushes are live. One continuous translucent
        // Lotus surface covers both areas, with only a quiet structural divider between them.
        unsafe {
            self.context.FillRectangle(&raw const surface, &self.panel);
            self.context.FillRectangle(&raw const footer, &self.group);
            self.context
                .FillRectangle(&raw const divider, &self.divider);
            self.context
                .FillRectangle(&raw const footer_divider, &self.divider);
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
            &self.title_format,
            &self.accent,
            false,
        );
        for page in SettingsPage::ALL {
            let control = SettingsControl::Navigate(page);
            let Some(bounds) = layout.bounds(control) else {
                continue;
            };
            let selected = scene.page() == page;
            self.draw_control_surface(scene, bounds, selected);
            if selected {
                let marker = SettingsRect {
                    left: bounds.left + scale(scene, 3),
                    top: bounds.top + scale(scene, 12),
                    width: scale(scene, 3),
                    height: bounds.height.saturating_sub(scale(scene, 24)),
                };
                let marker = rounded(rect(marker), scale_f32(scene, 1.5));
                // SAFETY: Active context, local geometry, and retained accent brush are live.
                unsafe {
                    self.context
                        .FillRoundedRectangle(&raw const marker, &self.accent);
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
    }

    fn draw_content(&self, scene: &SettingsScene, layout: &SettingsLayout) {
        self.draw_content_group(scene, layout);
        if scene.page() == SettingsPage::About {
            self.draw_about(scene);
        }
        for entry in &layout.controls {
            match entry.control {
                SettingsControl::SurfacePreset => {
                    self.draw_surface_picker(scene, entry.bounds);
                }
                SettingsControl::AccentPreset => {
                    self.draw_accent_picker(scene, entry.bounds);
                }
                SettingsControl::NotificationBadgeStyle => {
                    self.draw_notification_badge_style(scene, entry.bounds);
                }
                SettingsControl::DockZone => {
                    self.draw_zone_picker(scene, entry.bounds, false);
                }
                SettingsControl::SystemStatusZone => {
                    self.draw_zone_picker(scene, entry.bounds, true);
                }
                SettingsControl::Toggle(toggle) => {
                    self.draw_toggle(scene, entry.bounds, toggle);
                }
                SettingsControl::Slider(slider) => {
                    self.draw_slider(scene, entry.bounds, slider);
                }
                SettingsControl::ChooseMascotImage => {
                    self.draw_mascot_image(scene, entry.bounds);
                }
                SettingsControl::ResetMascotImage => {
                    self.draw_reset_mascot(scene, entry.bounds);
                }
                SettingsControl::CheckForUpdates => {
                    self.draw_check_for_updates(scene, entry.bounds);
                }
                SettingsControl::Revert => self.draw_revert(scene, entry.bounds),
                SettingsControl::Apply => self.draw_apply(scene, entry.bounds),
                SettingsControl::Close => self.draw_close(scene, entry.bounds),
                SettingsControl::Navigate(_) => {}
            }
        }
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
                        | SettingsControl::Toggle(_)
                        | SettingsControl::Slider(_)
                        | SettingsControl::ChooseMascotImage
                        | SettingsControl::ResetMascotImage
                )
            })
            .collect();
        let (Some(first), Some(last)) = (controls.first(), controls.last()) else {
            return;
        };
        let bottom = last.bounds.top.saturating_add(last.bounds.height);
        let bounds = SettingsRect {
            left: first.bounds.left,
            top: first.bounds.top,
            width: first.bounds.width,
            height: bottom.saturating_sub(first.bounds.top),
        };
        let card = rounded(rect(bounds), scale_f32(scene, scene.theme().radii.panel));
        // SAFETY: Active context, local geometry, and retained group brush are live.
        unsafe {
            self.context
                .FillRoundedRectangle(&raw const card, &self.group);
        };
        for entry in controls.iter().take(controls.len().saturating_sub(1)) {
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
            concat!("Lotus ", env!("CARGO_PKG_VERSION")),
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
        self.draw_control_surface(scene, bounds, false);
        self.draw_text(
            toggle_label(toggle),
            inset(bounds, scale(scene, 16), 0),
            &self.body_format,
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
        self.draw_control_surface(scene, bounds, false);
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
        self.draw_control_surface(scene, bounds, false);
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
        self.draw_control_surface(scene, bounds, false);
        self.draw_text(
            "Restore Lotus icon",
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
            SettingsUpdateActivity::Idle => "Install Lotus",
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
        let surface = rounded(rect(bounds), scale_f32(scene, scene.theme().radii.control));
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
            self.context.DrawRoundedRectangle(
                &raw const surface,
                border,
                scale_f32(scene, 1.0),
                None,
            );
        }
        let text = match (emphasis, enabled) {
            (_, false) => &self.disabled,
            (ButtonEmphasis::Primary, true) => &self.accent_dark,
            (ButtonEmphasis::Secondary | ButtonEmphasis::Outline, true) => &self.text,
        };
        self.draw_text(label, bounds, &self.button_format, text, true);
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

    fn draw_control_surface(
        &self,
        scene: &SettingsScene,
        bounds: SettingsRect,
        selected: bool,
    ) {
        let surface = rounded(rect(bounds), scale_f32(scene, scene.theme().radii.control));
        let brush = if selected {
            Some(&self.selected)
        } else {
            None
        };
        if let Some(brush) = brush {
            // SAFETY: Active context and retained brush remain live.
            unsafe { self.context.FillRoundedRectangle(&raw const surface, brush) };
        }
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

fn toggle_label(value: SettingsToggle) -> &'static str {
    match value {
        SettingsToggle::ShowUnpinnedRunningApps => "Show unpinned running applications",
        SettingsToggle::ShowDesktopButton => "Show a desktop button at the right edge",
        SettingsToggle::ShowSystemStatus => "Show system status",
        SettingsToggle::ShowVolumeStatus => "Show volume",
        SettingsToggle::ShowNetworkStatus => "Show network",
        SettingsToggle::ShowBackgroundAppsStatus => "Show background applications",
        SettingsToggle::ShowDateTimeStatus => "Show time",
        SettingsToggle::ShowDateInStatus => "Show date below the time",
        SettingsToggle::StartWithWindows => "Start Lotus when you sign in",
        SettingsToggle::ReplaceWindowsTaskbar => "Replace the Windows taskbar",
        SettingsToggle::ExclusiveTaskbarReplacement => {
            "Fully hide the native taskbar (experimental)"
        }
        SettingsToggle::HideWhenFullscreen => "Hide while an app is fullscreen",
        SettingsToggle::SearchOpenWithWindowsKey => "Open search with the Windows key",
        SettingsToggle::AltTabEnabled => "Replace Alt+Tab with Lotus",
    }
}

fn slider_label(value: SettingsSlider) -> &'static str {
    match value {
        SettingsSlider::IconSize => "Icon size",
        SettingsSlider::ItemSpacing => "Item spacing",
        SettingsSlider::HorizontalPadding => "Horizontal padding",
        SettingsSlider::VerticalPadding => "Vertical padding",
        SettingsSlider::BottomOffset => "Bottom offset",
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

fn rect(value: SettingsRect) -> D2D_RECT_F {
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
