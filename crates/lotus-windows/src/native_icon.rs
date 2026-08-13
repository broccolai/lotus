use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;

use lotus_ui::icon::{RasterIcon, RasterIconError};
use thiserror::Error;
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
use windows::Win32::UI::WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx, HICON};
use windows::core::{Error, PCWSTR};

use super::launch::{
    resolve_executable, resolve_internet_shortcut_icon, resolve_shortcut_icon,
};
use crate::NativeError;

const MAX_ICON_SIZE: u32 = 1_024;
const BYTES_PER_PIXEL: u32 = 4;

#[derive(Debug, Error)]
pub enum NativeIconError {
    #[error("native icon paths must be nonempty and contain no null characters")]
    InvalidPath,
    #[error("native icon size must be between 1 and {MAX_ICON_SIZE} physical pixels")]
    InvalidSize,
    #[error("native icon raster dimensions exceed addressable memory")]
    RasterTooLarge,
    #[error(transparent)]
    InvalidRaster(#[from] RasterIconError),
    #[error(transparent)]
    Native(#[from] NativeError),
}

impl From<Error> for NativeIconError {
    fn from(error: Error) -> Self {
        Self::Native(error.into())
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CacheKey {
    normalized_path: String,
    icon_index: i32,
    size: u32,
}

#[derive(Default)]
pub struct NativeIconCache {
    icons: HashMap<CacheKey, Option<RasterIcon>>,
}

impl NativeIconCache {
    pub fn icon(
        &mut self,
        path: &Path,
        size: u32,
    ) -> Result<Option<RasterIcon>, NativeIconError> {
        validate_size(size)?;
        let source_path = sanitized_path(path)?;
        let normalized_path = normalize_path(&source_path)?;
        let extraction = icon_extraction_source(&source_path);
        let icon_index = extraction.as_ref().map_or(0, |(_, index)| *index);
        let key = CacheKey {
            normalized_path,
            icon_index,
            size,
        };

        if !self.icons.contains_key(&key) {
            let image = match extraction {
                Some((extraction_path, icon_index)) => {
                    extract_icon(&extraction_path, icon_index, &key)?
                }
                None => None,
            };
            self.icons.insert(key.clone(), image);
        }
        Ok(self.icons.get(&key).cloned().flatten())
    }
}

fn icon_extraction_source(source: &Path) -> Option<(PathBuf, i32)> {
    if has_extension(source, "lnk") {
        let target = resolve_executable(&source.to_string_lossy());
        let shortcut_icon = resolve_shortcut_icon(source);
        return select_shortcut_extraction(target, shortcut_icon);
    }
    if has_extension(source, "url") {
        return resolve_internet_shortcut_icon(source);
    }
    let resolved = resolve_executable(&source.to_string_lossy());
    select_extraction_path(source.to_owned(), resolved).map(|path| (path, 0))
}

fn select_shortcut_extraction(
    target: Option<PathBuf>,
    shortcut_icon: Option<(PathBuf, i32)>,
) -> Option<(PathBuf, i32)> {
    shortcut_icon.or_else(|| target.map(|path| (path, 0)))
}

fn select_extraction_path(source: PathBuf, resolved: Option<PathBuf>) -> Option<PathBuf> {
    resolved.or_else(|| (!has_extension(&source, "lnk")).then_some(source))
}

fn has_extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
}

fn extract_icon(
    path: &Path,
    icon_index: i32,
    key: &CacheKey,
) -> Result<Option<RasterIcon>, NativeIconError> {
    let wide_path = wide_path(path)?;
    let Some(icon) = load_shell_icon(path, &wide_path, icon_index, key.size) else {
        return Ok(None);
    };

    let black = render_icon(icon.get(), key.size, 0)?;
    let white = render_icon(icon.get(), key.size, u8::MAX)?;
    let pixels = compose_premultiplied_bgra(&black, &white);
    Ok(Some(RasterIcon::new(
        format!("native:{}@{}px", key.normalized_path, key.size),
        key.size,
        key.size,
        pixels,
    )?))
}

fn load_shell_icon(
    source: &Path,
    path: &[u16],
    icon_index: i32,
    size: u32,
) -> Option<OwnedIcon> {
    if source.to_string_lossy().starts_with("shell:")
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

fn normalize_path(path: &Path) -> Result<String, NativeIconError> {
    let path = path.to_string_lossy();
    let path = path.trim();
    if path.is_empty() || path.contains('\0') {
        return Err(NativeIconError::InvalidPath);
    }
    Ok(path.replace('/', "\\").to_lowercase())
}

fn sanitized_path(path: &Path) -> Result<PathBuf, NativeIconError> {
    let path = path.to_string_lossy();
    let path = path.trim();
    if path.is_empty() || path.contains('\0') {
        return Err(NativeIconError::InvalidPath);
    }
    Ok(PathBuf::from(path))
}

fn wide_path(path: &Path) -> Result<Vec<u16>, NativeIconError> {
    use std::os::windows::ffi::OsStrExt;

    let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if wide.is_empty() || wide.contains(&0) {
        return Err(NativeIconError::InvalidPath);
    }
    wide.push(0);
    Ok(wide)
}

fn validate_size(size: u32) -> Result<(), NativeIconError> {
    if size == 0 || size > MAX_ICON_SIZE {
        Err(NativeIconError::InvalidSize)
    } else {
        Ok(())
    }
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

struct OwnedIcon(HICON);

impl OwnedIcon {
    const fn get(&self) -> HICON {
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
