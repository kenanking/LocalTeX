//! Black snip-overlay reticle. Native Win32 cursor — do not use GPUI's
//! Crosshair style (it can leak after destroy).

use anyhow::{anyhow, Result};
use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateCursor, DestroyCursor, GetSystemMetrics, LoadCursorW, SetCursor, HCURSOR, IDC_ARROW,
    IDC_CROSS, SM_CXCURSOR, SM_CYCURSOR,
};

pub struct OverlayCursor {
    pub handle: HCURSOR,
    owned: bool,
}

impl OverlayCursor {
    pub fn load(instance: HINSTANCE) -> Self {
        match make_black_crosshair(instance) {
            Ok(handle) => Self {
                handle,
                owned: true,
            },
            Err(_) => Self {
                handle: unsafe { LoadCursorW(None, IDC_CROSS) }.unwrap_or_default(),
                owned: false,
            },
        }
    }

    pub fn apply(&self) {
        unsafe { SetCursor(Some(self.handle)) };
    }
}

impl Drop for OverlayCursor {
    fn drop(&mut self) {
        unsafe { SetCursor(LoadCursorW(None, IDC_ARROW).ok()) };
        if self.owned {
            let _ = unsafe { DestroyCursor(self.handle) };
        }
    }
}

fn cursor_stride(width: i32) -> usize {
    (((width + 15) / 16) * 2) as usize
}

fn mask_set(plane: &mut [u8], x: i32, y: i32, width: i32, one: bool) {
    if x < 0 || y < 0 || x >= width {
        return;
    }
    let stride = cursor_stride(width);
    let i = y as usize * stride + x as usize / 8;
    if i >= plane.len() {
        return;
    }
    let bit = 7 - (x % 8);
    if one {
        plane[i] |= 1 << bit;
    } else {
        plane[i] &= !(1 << bit);
    }
}

#[cfg(test)]
fn mask_bit(plane: &[u8], x: i32, y: i32, width: i32) -> bool {
    let stride = cursor_stride(width);
    let i = y as usize * stride + x as usize / 8;
    let bit = 7 - (x % 8);
    (plane[i] >> bit) & 1 == 1
}

fn black_crosshair_planes(w: i32, h: i32) -> (Vec<u8>, Vec<u8>) {
    let bytes = cursor_stride(w) * h as usize;
    let mut and = vec![0xFFu8; bytes];
    let mut xor = vec![0u8; bytes];
    let cx = w / 2;
    let cy = h / 2;
    // Short 3px arms + 1px white halo (bigger than a 1px reticle, smaller
    // than IDC_CROSS spanning the whole cursor).
    let arm = 10i32;
    for y in 0..h {
        for x in 0..w {
            let dx = (x - cx).abs();
            let dy = (y - cy).abs();
            let black = (dx <= 1 && dy <= arm) || (dy <= 1 && dx <= arm);
            let halo = (dx <= 2 && dy <= arm + 1) || (dy <= 2 && dx <= arm + 1);
            if black {
                mask_set(&mut and, x, y, w, false);
                mask_set(&mut xor, x, y, w, false);
            } else if halo {
                mask_set(&mut and, x, y, w, false);
                mask_set(&mut xor, x, y, w, true);
            }
        }
    }
    (and, xor)
}

fn make_black_crosshair(instance: HINSTANCE) -> Result<HCURSOR> {
    let w = unsafe { GetSystemMetrics(SM_CXCURSOR) }.max(1);
    let h = unsafe { GetSystemMetrics(SM_CYCURSOR) }.max(1);
    let (and, xor) = black_crosshair_planes(w, h);
    unsafe {
        CreateCursor(
            Some(instance),
            w / 2,
            h / 2,
            w,
            h,
            and.as_ptr().cast(),
            xor.as_ptr().cast(),
        )
    }
    .map_err(|err| anyhow!(err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_crosshair_center_is_opaque_black() {
        let (and, xor) = black_crosshair_planes(32, 32);
        assert!(
            !mask_bit(&and, 16, 16, 32) && !mask_bit(&xor, 16, 16, 32),
            "center must be black (AND 0, XOR 0)"
        );
        assert!(mask_bit(&and, 0, 0, 32), "corner must stay transparent");
        assert!(
            !mask_bit(&and, 16, 8, 32),
            "arm should reach ~10px from center"
        );
        assert!(
            mask_bit(&and, 16, 2, 32),
            "arms should stay short (not a full-span cross)"
        );
    }
}
