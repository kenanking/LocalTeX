//! Windows clipboard image read. GPUI 0.2 only recognizes registered PNG/GIF
//! and prefers CF_UNICODETEXT when both text and a bitmap are present; Win+Shift+S
//! / Paint / most apps put CF_DIB, CF_BITMAP, and maybe PNG. Collect candidates
//! (PNG magic first, then DIB, then HBITMAP) so a bogus preferred format can
//! fall through to decode.

use std::mem::size_of;
use std::time::Duration;

use windows::core::w;
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, GetDC, GetDIBits, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW,
};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

use crate::identity::APP_SLUG;

const CF_BITMAP: u32 = 2;
const CF_DIB: u32 = 8;
const CF_DIBV5: u32 = 17;

pub fn read() -> Vec<Vec<u8>> {
    with_clipboard(|| {
        let mut out = Vec::new();
        out.extend(registered_image_bytes());
        for format in [CF_DIB, CF_DIBV5] {
            if let Some(bytes) = clipboard_bytes(format) {
                out.push(bytes);
            }
        }
        if let Some(bytes) = dib_from_cf_bitmap() {
            out.push(bytes);
        }
        out
    })
    .unwrap_or_default()
}

fn registered_image_bytes() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for name in [w!("PNG"), w!("image/png"), w!("JFIF"), w!("GIF")] {
        let format = unsafe { RegisterClipboardFormatW(name) };
        if format == 0 {
            continue;
        }
        let Some(bytes) = clipboard_bytes(format) else {
            continue;
        };
        if is_png(&bytes) || is_jpeg(&bytes) || is_gif(&bytes) {
            out.push(bytes);
        }
    }
    out
}

fn clipboard_bytes(format: u32) -> Option<Vec<u8>> {
    if unsafe { IsClipboardFormatAvailable(format) }.is_err() {
        return None;
    }
    let handle = unsafe { GetClipboardData(format) }.ok()?;
    copy_hglobal(handle)
}

fn copy_hglobal(handle: HANDLE) -> Option<Vec<u8>> {
    let global = HGLOBAL(handle.0);
    let size = unsafe { GlobalSize(global) };
    if size == 0 {
        return None;
    }
    let ptr = unsafe { GlobalLock(global) };
    if ptr.is_null() {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), size) }.to_vec();
    let _ = unsafe { GlobalUnlock(global) };
    Some(bytes)
}

/// CF_BITMAP is an HBITMAP, not an HGLOBAL. Must convert before CloseClipboard.
fn dib_from_cf_bitmap() -> Option<Vec<u8>> {
    if unsafe { IsClipboardFormatAvailable(CF_BITMAP) }.is_err() {
        return None;
    }
    let handle = unsafe { GetClipboardData(CF_BITMAP) }.ok()?;
    let hbmp = HBITMAP(handle.0);
    if hbmp.is_invalid() {
        return None;
    }
    unsafe { hbitmap_to_dib32(hbmp) }
}

unsafe fn hbitmap_to_dib32(hbmp: HBITMAP) -> Option<Vec<u8>> {
    let screen = GetDC(None);
    if screen.is_invalid() {
        return None;
    }
    let hdc = CreateCompatibleDC(Some(screen));
    if hdc.is_invalid() {
        ReleaseDC(None, screen);
        return None;
    }
    let mut info = BITMAPINFO::default();
    info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    let header_ok = GetDIBits(hdc, hbmp, 0, 0, None, &mut info, DIB_RGB_COLORS) != 0;
    if !header_ok || info.bmiHeader.biWidth <= 0 || info.bmiHeader.biHeight == 0 {
        let _ = DeleteDC(hdc);
        ReleaseDC(None, screen);
        return None;
    }
    let width = info.bmiHeader.biWidth;
    let height = info.bmiHeader.biHeight.unsigned_abs();
    info.bmiHeader.biHeight = -(height as i32);
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB.0;
    info.bmiHeader.biSizeImage = 0;
    let mut bits = vec![0u8; (width as usize) * (height as usize) * 4];
    let rows = GetDIBits(
        hdc,
        hbmp,
        0,
        height,
        Some(bits.as_mut_ptr().cast()),
        &mut info,
        DIB_RGB_COLORS,
    );
    let _ = DeleteDC(hdc);
    ReleaseDC(None, screen);
    if rows == 0 {
        return None;
    }
    let mut out = vec![0u8; 40];
    out[0..4].copy_from_slice(&40u32.to_le_bytes());
    out[4..8].copy_from_slice(&width.to_le_bytes());
    out[8..12].copy_from_slice(&(-((height) as i32)).to_le_bytes());
    out[12..14].copy_from_slice(&1u16.to_le_bytes());
    out[14..16].copy_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&bits);
    Some(out)
}

fn with_clipboard<T>(f: impl FnOnce() -> T) -> Option<T> {
    for attempt in 0..5 {
        match unsafe { OpenClipboard(None) } {
            Ok(()) => {
                let out = f();
                if let Err(err) = unsafe { CloseClipboard() } {
                    eprintln!("{APP_SLUG}: CloseClipboard: {err}");
                }
                return Some(out);
            }
            Err(err) => {
                if attempt == 4 {
                    eprintln!("{APP_SLUG}: OpenClipboard: {err}");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    None
}

fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, b'P', b'N', b'G'])
}

fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF
}

fn is_gif(bytes: &[u8]) -> bool {
    bytes.starts_with(b"GIF8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_bytes() {
        assert!(is_png(&[0x89, b'P', b'N', b'G', 0x0D]));
        assert!(is_jpeg(&[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(is_gif(b"GIF89a"));
        assert!(!is_png(&[0, 1, 2, 3, 4]));
    }
}
