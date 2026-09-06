use crate::icon::Icon;
use crate::theme::{Color, InterfaceFont};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresentationRect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl PresentationRect {
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub const fn width(self) -> f32 {
        self.right - self.left
    }

    pub const fn height(self) -> f32 {
        self.bottom - self.top
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HorizontalAlignment {
    Leading,
    Center,
    Trailing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerticalAlignment {
    Top,
    Center,
    Bottom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontWeight {
    Normal,
    Semibold,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum FontFamily {
    #[default]
    Interface,
    SystemSymbols,
    Brand,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageSampling {
    Smooth,
    PixelAligned,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub family: FontFamily,
    pub weight: FontWeight,
    pub horizontal: HorizontalAlignment,
    pub vertical: VerticalAlignment,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PresentationPrimitive<Asset> {
    PushClip {
        bounds: PresentationRect,
    },
    PopClip,
    FillRoundedRect {
        bounds: PresentationRect,
        radius: f32,
        color: Color,
    },
    StrokeRoundedRect {
        bounds: PresentationRect,
        radius: f32,
        width: f32,
        color: Color,
    },
    Text {
        value: String,
        bounds: PresentationRect,
        style: TextStyle,
        color: Color,
    },
    TextCaret {
        before: String,
        bounds: PresentationRect,
        style: TextStyle,
        top_inset: f32,
        bottom_inset: f32,
        width: f32,
        color: Color,
    },
    Icon {
        icon: Icon<Asset>,
        bounds: PresentationRect,
        tint: Color,
        opacity: f32,
        sampling: ImageSampling,
        radius: f32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Presentation<Asset> {
    pub clear: Color,
    pub primitives: Vec<PresentationPrimitive<Asset>>,
}

impl<Asset> Presentation<Asset> {
    pub const fn new(clear: Color) -> Self {
        Self {
            clear,
            primitives: Vec::new(),
        }
    }

    pub fn push(&mut self, primitive: PresentationPrimitive<Asset>) {
        self.primitives.push(primitive);
    }

    pub fn translate_y_from(&mut self, first: usize, offset: f32) {
        for primitive in &mut self.primitives[first..] {
            primitive.translate_y(offset);
        }
    }

    #[must_use]
    pub fn with_interface_font(mut self, interface_font: InterfaceFont) -> Self {
        if interface_font == InterfaceFont::Fraunces {
            for primitive in &mut self.primitives {
                primitive.apply_interface_font();
            }
        }
        self
    }
}

impl<Asset> PresentationPrimitive<Asset> {
    fn apply_interface_font(&mut self) {
        let style = match self {
            Self::Text { style, .. } | Self::TextCaret { style, .. } => style,
            Self::PushClip { .. }
            | Self::PopClip
            | Self::FillRoundedRect { .. }
            | Self::StrokeRoundedRect { .. }
            | Self::Icon { .. } => return,
        };
        if style.family == FontFamily::Interface {
            style.family = FontFamily::Brand;
        }
    }

    fn translate_y(&mut self, offset: f32) {
        let bounds = match self {
            Self::PushClip { .. } | Self::PopClip => return,
            Self::FillRoundedRect { bounds, .. }
            | Self::StrokeRoundedRect { bounds, .. }
            | Self::Text { bounds, .. }
            | Self::TextCaret { bounds, .. }
            | Self::Icon { bounds, .. } => bounds,
        };
        bounds.top += offset;
        bounds.bottom += offset;
    }
}
