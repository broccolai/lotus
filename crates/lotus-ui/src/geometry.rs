use euclid::{Point2D, Rect, Size2D};

pub enum PhysicalPixel {}

pub type PhysicalPoint = Point2D<i32, PhysicalPixel>;
pub type PhysicalUnsignedPoint = Point2D<u32, PhysicalPixel>;
pub type PhysicalRect = Rect<u32, PhysicalPixel>;
pub type PhysicalSize = Size2D<u32, PhysicalPixel>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NonZeroPhysicalSize(PhysicalSize);

impl NonZeroPhysicalSize {
    pub const fn new(width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            None
        } else {
            Some(Self(PhysicalSize::new(width, height)))
        }
    }

    pub const fn width(self) -> u32 {
        self.0.width
    }

    pub const fn height(self) -> u32 {
        self.0.height
    }

    pub const fn get(self) -> PhysicalSize {
        self.0
    }
}

pub const fn physical_rect(x: u32, y: u32, width: u32, height: u32) -> PhysicalRect {
    PhysicalRect::new(PhysicalUnsignedPoint::new(x, y), Size2D::new(width, height))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DpiScale(u32);

impl DpiScale {
    const DEFAULT: u32 = 96;

    pub const fn new(dpi: u32) -> Option<Self> {
        if dpi == 0 {
            None
        } else {
            Some(Self(dpi))
        }
    }

    pub const fn from_system(dpi: u32) -> Self {
        Self(if dpi == 0 {
            Self::DEFAULT
        } else {
            dpi
        })
    }

    pub const fn dpi(self) -> u32 {
        self.0
    }

    pub fn physical(self, dips: u32) -> u32 {
        u32::try_from((u64::from(dips) * u64::from(self.0) + 48) / 96).unwrap_or(u32::MAX)
    }

    pub fn physical_i32(self, dips: u32) -> i32 {
        i32::try_from(self.physical(dips)).unwrap_or(i32::MAX)
    }
}
