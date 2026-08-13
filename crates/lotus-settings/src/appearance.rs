use lotus_core::settings::DockSettings;
use lotus_ui::theme::Theme;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfacePreset {
    Lotus,
    Graphite,
    Midnight,
}

impl SurfacePreset {
    pub const ALL: [Self; 3] = [Self::Lotus, Self::Graphite, Self::Midnight];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Lotus => "Lotus",
            Self::Graphite => "Graphite",
            Self::Midnight => "Midnight",
        }
    }

    pub const fn color(self) -> &'static str {
        match self {
            Self::Lotus => "#11141A",
            Self::Graphite => "#19191B",
            Self::Midnight => "#0D1524",
        }
    }

    pub fn selected(settings: &DockSettings) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|preset| preset.color().eq_ignore_ascii_case(&settings.background_color))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccentPreset {
    Blossom,
    Peach,
    Lavender,
    Sky,
    Mint,
}

impl AccentPreset {
    pub const ALL: [Self; 5] = [Self::Blossom, Self::Peach, Self::Lavender, Self::Sky, Self::Mint];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Blossom => "Blossom",
            Self::Peach => "Peach",
            Self::Lavender => "Lavender",
            Self::Sky => "Sky",
            Self::Mint => "Mint",
        }
    }

    pub const fn color(self) -> &'static str {
        match self {
            Self::Blossom => "#F5A5A5",
            Self::Peach => "#F3B38C",
            Self::Lavender => "#B9A7F7",
            Self::Sky => "#91C7F4",
            Self::Mint => "#96D8B6",
        }
    }

    pub fn selected(settings: &DockSettings) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|preset| preset.color().eq_ignore_ascii_case(&settings.accent_color))
    }
}

pub fn theme_for(settings: &DockSettings) -> Theme {
    Theme::new(&settings.background_color, &settings.accent_color, settings.corner_radius)
}
