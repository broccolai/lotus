use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};

use crate::NativeError;

pub fn enable_per_monitor_v2() -> Result<(), NativeError> {
    // SAFETY: Lotus calls this once before creating windows or initializing UI APIs.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
        .map_err(Into::into)
}
