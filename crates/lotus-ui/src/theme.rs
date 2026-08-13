#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
}

impl Color {
    #[must_use]
    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self::rgba(red, green, blue, u8::MAX)
    }

    #[must_use]
    pub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        const MAX: f32 = u8::MAX as f32;
        Self {
            red: red as f32 / MAX,
            green: green as f32 / MAX,
            blue: blue as f32 / MAX,
            alpha: alpha as f32 / MAX,
        }
    }

    #[must_use]
    pub fn from_hex(value: &str) -> Option<Self> {
        let hex = value.trim().strip_prefix('#')?;
        if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return None;
        }
        let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
        let alpha = hex
            .get(6..8)
            .map_or(Some(u8::MAX), |alpha| u8::from_str_radix(alpha, 16).ok())?;
        Some(Self::rgba(red, green, blue, alpha))
    }

    #[must_use]
    pub const fn with_alpha(self, alpha: f32) -> Self {
        Self { alpha, ..self }
    }

    #[must_use]
    pub fn blend(self, overlay: Self, amount: f32) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        let inverse = 1.0 - amount;
        Self {
            red: self.red * inverse + overlay.red * amount,
            green: self.green * inverse + overlay.green * amount,
            blue: self.blue * inverse + overlay.blue * amount,
            alpha: self.alpha * inverse + overlay.alpha * amount,
        }
    }

    #[must_use]
    pub fn relative_luminance(self) -> f32 {
        0.2126 * self.red + 0.7152 * self.green + 0.0722 * self.blue
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CornerRadii {
    pub window: f32,
    pub panel: f32,
    pub control: f32,
    pub compact: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub canvas: Color,
    pub acrylic_tint: Color,
    pub chrome_overlay: Color,
    pub surface: Color,
    pub elevated_surface: Color,
    pub control: Color,
    pub control_hover: Color,
    pub control_selected: Color,
    pub border: Color,
    pub border_strong: Color,
    pub divider: Color,
    pub text: Color,
    pub text_muted: Color,
    pub text_disabled: Color,
    pub accent: Color,
    pub accent_soft: Color,
    pub accent_subtle: Color,
    pub on_accent: Color,
    pub radii: CornerRadii,
}

impl Theme {
    #[must_use]
    pub fn new(background: &str, accent: &str, window_radius: u32) -> Self {
        let canvas =
            Color::from_hex(background).unwrap_or_else(|| Color::rgb(0x11, 0x14, 0x1A));
        let accent =
            Color::from_hex(accent).unwrap_or_else(|| Color::rgb(0xF5, 0xA5, 0xA5));
        let white = Color::rgb(u8::MAX, u8::MAX, u8::MAX);
        let window = f32::from(u16::try_from(window_radius).unwrap_or(u16::MAX));
        Self {
            canvas,
            acrylic_tint: canvas,
            chrome_overlay: white.with_alpha(0.035),
            surface: canvas.blend(white, 0.035),
            elevated_surface: canvas.blend(white, 0.065),
            control: white.with_alpha(0.055),
            control_hover: white.with_alpha(0.085),
            control_selected: accent.with_alpha(0.14),
            border: white.with_alpha(0.085),
            border_strong: white.with_alpha(0.14),
            divider: white.with_alpha(0.105),
            text: Color::rgba(0xF7, 0xF8, 0xFB, 0xF5),
            text_muted: Color::rgba(0xF7, 0xF8, 0xFB, 0x9E),
            text_disabled: Color::rgba(0xF7, 0xF8, 0xFB, 0x4D),
            accent,
            accent_soft: accent.with_alpha(0.22),
            accent_subtle: accent.with_alpha(0.12),
            on_accent: if accent.relative_luminance() >= 0.62 {
                Color::rgb(0x22, 0x1B, 0x20)
            } else {
                Color::rgb(0xFF, 0xFF, 0xFF)
            },
            radii: CornerRadii {
                window,
                panel: window.clamp(8.0, 12.0),
                control: (window * 0.62).clamp(5.0, 8.0),
                compact: (window * 0.45).clamp(4.0, 6.0),
            },
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::new("#11141A", "#F5A5A5", 8)
    }
}
