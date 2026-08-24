use std::collections::HashMap;

use lotus_core::window::TrackedWindowKey;
use lotus_ui::geometry::PhysicalRect;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION,
    DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE, DwmRegisterThumbnail,
    DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
};

use crate::WindowHandle;

pub struct DwmThumbnailHost {
    destination: HWND,
    thumbnails: HashMap<TrackedWindowKey, DwmThumbnail>,
}

impl DwmThumbnailHost {
    pub fn new(destination: WindowHandle) -> Self {
        Self {
            destination: destination.raw(),
            thumbnails: HashMap::new(),
        }
    }

    pub fn reconcile(&mut self, previews: &[(TrackedWindowKey, PhysicalRect)]) {
        self.thumbnails.retain(|window, _| {
            previews.iter().any(|(candidate, _)| candidate == window)
                && crate::window_tracker::is_live_tracked_window(*window)
        });

        for (window, bounds) in previews {
            if !crate::window_tracker::is_live_tracked_window(*window) {
                continue;
            }
            let thumbnail = self.thumbnails.entry(*window).or_insert_with(|| {
                DwmThumbnail::register(self.destination, *window)
                    .unwrap_or_else(DwmThumbnail::unavailable)
            });
            thumbnail.update(*window, *bounds);
        }
    }

    pub fn clear(&mut self) {
        self.thumbnails.clear();
    }
}

struct DwmThumbnail {
    handle: Option<isize>,
}

impl DwmThumbnail {
    fn register(destination: HWND, source: TrackedWindowKey) -> Option<Self> {
        let handle =
            crate::window_tracker::with_live_tracked_window(source, |source| unsafe {
                DwmRegisterThumbnail(destination, source).ok()
            })??;
        Some(Self {
            handle: Some(handle),
        })
    }

    const fn unavailable() -> Self {
        Self { handle: None }
    }

    fn update(&self, source: TrackedWindowKey, bounds: PhysicalRect) {
        let Some(handle) = self.handle else {
            return;
        };
        let properties = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION
                | DWM_TNP_VISIBLE
                | DWM_TNP_OPACITY
                | DWM_TNP_SOURCECLIENTAREAONLY,
            rcDestination: RECT {
                left: i32::try_from(bounds.min_x()).unwrap_or(i32::MAX),
                top: i32::try_from(bounds.min_y()).unwrap_or(i32::MAX),
                right: i32::try_from(bounds.max_x()).unwrap_or(i32::MAX),
                bottom: i32::try_from(bounds.max_y()).unwrap_or(i32::MAX),
            },
            opacity: u8::MAX,
            fVisible: true.into(),
            fSourceClientAreaOnly: false.into(),
            ..DWM_THUMBNAIL_PROPERTIES::default()
        };
        let _ = crate::window_tracker::with_live_tracked_window(source, |_| unsafe {
            DwmUpdateThumbnailProperties(handle, &raw const properties)
        });
    }
}

impl Drop for DwmThumbnail {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = unsafe { DwmUnregisterThumbnail(handle) };
        }
    }
}
