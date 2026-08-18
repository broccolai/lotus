use lotus_ui::theme::Color;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush;

pub(super) const fn d2d(color: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: color.red,
        g: color.green,
        b: color.blue,
        a: color.alpha,
    }
}

pub(super) fn set(brush: &ID2D1SolidColorBrush, color: Color) {
    let color = d2d(color);
    unsafe { brush.SetColor(&raw const color) };
}
