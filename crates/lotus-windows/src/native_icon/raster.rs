use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::NonNull;

use lotus_core::window::TrackedWindowKey;
use lotus_ui::icon::RasterIcon;
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, HBITMAP, HDC, HGDIOBJ, SelectObject,
};
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::UI::Shell::{
    SHDefExtractIconW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_PIDL,
    SHGetFileInfoW, SHParseDisplayName,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CopyIcon, DI_NORMAL, DestroyIcon, DrawIconEx, GCLP_HICON, GCLP_HICONSM,
    GET_CLASS_LONG_INDEX, GetClassLongPtrW, HICON, ICON_BIG, ICON_SMALL, ICON_SMALL2,
    SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_GETICON,
};
use windows::core::{Error, PCWSTR};

use super::{CacheKey, NativeIconError};

const BYTES_PER_PIXEL: u32 = 4;

pub(super) fn extract_icon(
    path: &std::path::Path,
    icon_index: i32,
    key: &CacheKey,
) -> Result<Option<RasterIcon>, NativeIconError> {
    let wide_path = wide_path(path)?;
    let Some(icon) = load_shell_icon(path, &wide_path, icon_index, key.size) else {
        return Ok(None);
    };

    rasterize_icon(
        icon.get(),
        format!("native:{}@{}px", key.normalized_path, key.size),
        key.size,
    )
    .map(Some)
}

pub(super) fn copy_window_icon(window: TrackedWindowKey) -> Option<OwnedIcon> {
    crate::window_tracker::with_live_tracked_window(window, |hwnd| {
        let icon = window_icon(hwnd, usize::try_from(ICON_SMALL2).ok()?)
            .or_else(|| window_icon(hwnd, usize::try_from(ICON_SMALL).ok()?))
            .or_else(|| window_icon(hwnd, usize::try_from(ICON_BIG).ok()?))
            .or_else(|| class_icon(hwnd, GCLP_HICONSM))
            .or_else(|| class_icon(hwnd, GCLP_HICON));
        icon.and_then(copy_icon)
    })
    .flatten()
}

pub(super) fn rasterize_icon(
    icon: HICON,
    identity: String,
    size: u32,
) -> Result<RasterIcon, NativeIconError> {
    let black = render_icon(icon, size, 0)?;
    let white = render_icon(icon, size, u8::MAX)?;
    let pixels = compose_premultiplied_bgra(&black, &white);
    RasterIcon::new(identity, size, size, pixels).map_err(NativeIconError::from)
}

fn window_icon(hwnd: windows::Win32::Foundation::HWND, kind: usize) -> Option<HICON> {
    let mut result = usize::default();
    let sent = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_GETICON,
            windows::Win32::Foundation::WPARAM(kind),
            windows::Win32::Foundation::LPARAM(0),
            SMTO_ABORTIFHUNG,
            100,
            Some(&raw mut result),
        )
    };
    (sent.0 != 0).then_some(())?;
    (!result.eq(&0)).then(|| HICON(std::ptr::with_exposed_provenance_mut(result)))
}

fn class_icon(
    hwnd: windows::Win32::Foundation::HWND,
    index: GET_CLASS_LONG_INDEX,
) -> Option<HICON> {
    let icon = unsafe { GetClassLongPtrW(hwnd, index) };
    (icon != 0).then(|| HICON(std::ptr::with_exposed_provenance_mut(icon)))
}

fn copy_icon(icon: HICON) -> Option<OwnedIcon> {
    let icon = unsafe { CopyIcon(icon) }.ok()?;
    (!icon.0.is_null()).then_some(OwnedIcon(icon))
}

fn load_shell_icon(
    source: &std::path::Path,
    path: &[u16],
    icon_index: i32,
    size: u32,
) -> Option<OwnedIcon> {
    if super::is_shell_namespace_path(source)
        && let Some(icon) = load_namespace_icon(path)
    {
        return Some(icon);
    }
    let path = PCWSTR(path.as_ptr());
    let mut icon = HICON::default();
    // SAFETY: `path` is a null-terminated UTF-16 string, the output points to writable storage,
    // and the requested large-icon size fits SHDefExtractIconW's low 16-bit size field.
    let extracted =
        unsafe { SHDefExtractIconW(path, icon_index, 0, Some(&raw mut icon), None, size) };
    if !icon.0.is_null() {
        let icon = OwnedIcon(icon);
        if extracted.is_ok() {
            return Some(icon);
        }
    }

    let mut info = SHFILEINFOW::default();
    // SAFETY: All pointers reference initialized storage for this synchronous shell query.
    let result = unsafe {
        SHGetFileInfoW(
            path,
            FILE_FLAGS_AND_ATTRIBUTES::default(),
            Some(&raw mut info),
            u32_size::<SHFILEINFOW>(),
            SHGFI_ICON | SHGFI_LARGEICON,
        )
    };
    if info.hIcon.0.is_null() {
        None
    } else {
        let icon = OwnedIcon(info.hIcon);
        (result != 0).then_some(icon)
    }
}

fn load_namespace_icon(path: &[u16]) -> Option<OwnedIcon> {
    let mut item_id_list = std::ptr::null_mut();
    // SAFETY: `path` is null-terminated, the bind context is optional, and the
    // output receives a task-allocated absolute item ID list.
    unsafe {
        SHParseDisplayName(PCWSTR(path.as_ptr()), None, &raw mut item_id_list, 0, None)
    }
    .ok()?;
    let item_id_list = CoTaskMemItemIdList(item_id_list);
    let mut info = SHFILEINFOW::default();
    // SAFETY: SHGFI_PIDL documents that the first argument is an absolute PIDL;
    // the owned PIDL and writable result storage remain live for this call.
    let result = unsafe {
        SHGetFileInfoW(
            PCWSTR(item_id_list.0.cast()),
            FILE_FLAGS_AND_ATTRIBUTES::default(),
            Some(&raw mut info),
            u32_size::<SHFILEINFOW>(),
            SHGFI_PIDL | SHGFI_ICON | SHGFI_LARGEICON,
        )
    };
    if result == 0 || info.hIcon.0.is_null() {
        None
    } else {
        Some(OwnedIcon(info.hIcon))
    }
}

struct CoTaskMemItemIdList(*mut windows::Win32::UI::Shell::Common::ITEMIDLIST);

impl Drop for CoTaskMemItemIdList {
    fn drop(&mut self) {
        // SAFETY: This PIDL came from SHParseDisplayName and is released exactly
        // once with the documented task allocator.
        unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(self.0.cast())) };
    }
}

fn render_icon(icon: HICON, size: u32, background: u8) -> Result<Vec<u8>, NativeIconError> {
    let dimension = i32::try_from(size).map_err(|_| NativeIconError::InvalidSize)?;
    let byte_len = raster_byte_len(size)?;
    let dc = OwnedMemoryDc::create()?;
    let bitmap = OwnedDib::create(dc.get(), dimension, byte_len)?;
    let selection = SelectedBitmap::select(dc.get(), bitmap.get())?;

    // SAFETY: The top-down DIB exposes exactly `byte_len` writable bytes while its bitmap lives.
    let pixels =
        unsafe { std::slice::from_raw_parts_mut(bitmap.bits().as_ptr(), byte_len) };
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[background, background, background, u8::MAX]);
    }

    // SAFETY: The memory DC has the live DIB selected, the icon is owned and live, and dimensions
    // were validated for both GDI and the allocated pixel buffer.
    unsafe {
        DrawIconEx(
            dc.get(),
            0,
            0,
            icon,
            dimension,
            dimension,
            0,
            None,
            DI_NORMAL,
        )?;
    };
    let rendered = pixels.to_vec();
    drop(selection);
    Ok(rendered)
}

fn compose_premultiplied_bgra(black: &[u8], white: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(black.len());
    for (black, white) in black.chunks_exact(4).zip(white.chunks_exact(4)) {
        let mut differences = [
            white[0].saturating_sub(black[0]),
            white[1].saturating_sub(black[1]),
            white[2].saturating_sub(black[2]),
        ];
        differences.sort_unstable();
        let alpha = u8::MAX.saturating_sub(differences[1]);
        output.extend_from_slice(&[
            black[0].min(alpha),
            black[1].min(alpha),
            black[2].min(alpha),
            alpha,
        ]);
    }
    output
}

fn wide_path(path: &std::path::Path) -> Result<Vec<u16>, NativeIconError> {
    use std::os::windows::ffi::OsStrExt;

    let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if wide.is_empty() || wide.contains(&0) {
        return Err(NativeIconError::InvalidPath);
    }
    wide.push(0);
    Ok(wide)
}

fn raster_byte_len(size: u32) -> Result<usize, NativeIconError> {
    let pixels = size
        .checked_mul(size)
        .ok_or(NativeIconError::RasterTooLarge)?;
    usize::try_from(
        pixels
            .checked_mul(BYTES_PER_PIXEL)
            .ok_or(NativeIconError::RasterTooLarge)?,
    )
    .map_err(|_| NativeIconError::RasterTooLarge)
}

pub(super) struct OwnedIcon(HICON);

impl OwnedIcon {
    pub(super) const fn get(&self) -> HICON {
        self.0
    }
}

impl Drop for OwnedIcon {
    fn drop(&mut self) {
        // SAFETY: This guard owns an HICON returned by a shell extraction function.
        let _ = unsafe { DestroyIcon(self.0) };
    }
}

struct OwnedMemoryDc(HDC);

impl OwnedMemoryDc {
    fn create() -> Result<Self, NativeIconError> {
        // SAFETY: A null source DC creates a display-compatible memory DC.
        let dc = unsafe { CreateCompatibleDC(None) };
        if dc.0.is_null() {
            Err(Error::from_thread().into())
        } else {
            Ok(Self(dc))
        }
    }

    const fn get(&self) -> HDC {
        self.0
    }
}

impl Drop for OwnedMemoryDc {
    fn drop(&mut self) {
        // SAFETY: This guard owns a memory DC created by CreateCompatibleDC.
        let _ = unsafe { DeleteDC(self.0) };
    }
}

struct OwnedDib {
    bitmap: HBITMAP,
    bits: NonNull<u8>,
}

impl OwnedDib {
    fn create(dc: HDC, dimension: i32, byte_len: usize) -> Result<Self, NativeIconError> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: u32_size::<BITMAPINFOHEADER>(),
                biWidth: dimension,
                biHeight: -dimension,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: u32::try_from(byte_len)
                    .map_err(|_| NativeIconError::RasterTooLarge)?,
                ..BITMAPINFOHEADER::default()
            },
            ..BITMAPINFO::default()
        };
        let mut bits = std::ptr::null_mut::<c_void>();
        // SAFETY: `info` fully describes a 32-bit top-down DIB and `bits` is writable output.
        let bitmap = unsafe {
            CreateDIBSection(
                Some(dc),
                &raw const info,
                DIB_RGB_COLORS,
                &raw mut bits,
                None,
                0,
            )?
        };
        let bits = NonNull::new(bits.cast::<u8>()).ok_or_else(|| {
            Error::new(E_FAIL, "CreateDIBSection returned no pixel storage")
        })?;
        Ok(Self { bitmap, bits })
    }

    const fn get(&self) -> HBITMAP {
        self.bitmap
    }

    const fn bits(&self) -> NonNull<u8> {
        self.bits
    }
}

impl Drop for OwnedDib {
    fn drop(&mut self) {
        // SAFETY: This guard owns a DIB section that is restored out of its memory DC first.
        let _ = unsafe { DeleteObject(HGDIOBJ(self.bitmap.0)) };
    }
}

struct SelectedBitmap {
    dc: HDC,
    previous: HGDIOBJ,
}

impl SelectedBitmap {
    fn select(dc: HDC, bitmap: HBITMAP) -> Result<Self, NativeIconError> {
        // SAFETY: Both handles are live and the bitmap is not selected into another DC.
        let previous = unsafe { SelectObject(dc, HGDIOBJ(bitmap.0)) };
        if previous.0.is_null() || previous.0.addr() == usize::MAX {
            Err(Error::from_thread().into())
        } else {
            Ok(Self { dc, previous })
        }
    }
}

impl Drop for SelectedBitmap {
    fn drop(&mut self) {
        // SAFETY: Restores the exact GDI object returned by the successful selection call.
        let _ = unsafe { SelectObject(self.dc, self.previous) };
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "Win32 ABI structure sizes are fixed and far below u32::MAX"
)]
const fn u32_size<T>() -> u32 {
    size_of::<T>() as u32
}
