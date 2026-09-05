//! Per-monitor Win32 freeze-frame overlay. Not a GPUI window (a second
//! Vulkan swapchain is forbidden). Recreated every snip, so plugging a
//! display in or out does not need a process restart — Mathpix/Qt restarts
//! because QScreen geometry is cached in device-independent pixels.
//! Hide-before-WGC lives in `win.rs`; the reticle is `win_cursor.rs`.

use std::mem::size_of;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use image::RgbaImage;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush, DeleteDC,
    DeleteObject, EndPaint, FrameRect, GetDC, IntersectClipRect, InvalidateRect, ReleaseDC,
    RestoreDC, SaveDC, ScreenToClient, SelectObject, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, PAINTSTRUCT, SRCCOPY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, ReleaseCapture, SetCapture, UnregisterHotKey, MOD_NOREPEAT, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
    GetWindowLongPtrW, LoadCursorW, PeekMessageW, RegisterClassW, SetCursor, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, GWLP_USERDATA, HCURSOR,
    HWND_TOPMOST, IDC_ARROW, MSG, PM_REMOVE, SWP_SHOWWINDOW, SW_SHOW, WM_DESTROY, WM_DISPLAYCHANGE,
    WM_ERASEBKGND, WM_HOTKEY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE,
    WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONUP, WM_SETCURSOR, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};

use super::win;
use super::win_cursor::OverlayCursor;
use crate::capture::{self, DesktopShot};
use crate::identity::APP_SLUG;

const ACCENT: (u8, u8, u8) = (0x25, 0x63, 0xeb);
const CLASS_NAME: PCWSTR = w!("LocalTeXSnip");
const OVERLAY_TIMEOUT: Duration = Duration::from_secs(300);
const ESCAPE_HOTKEY_ID: i32 = 0x4C54;

struct Dib {
    info: BITMAPINFO,
    bits: Vec<u8>,
    width: i32,
    height: i32,
}

struct Overlay {
    hwnd: HWND,
    origin_x: i32,
    origin_y: i32,
    dim: Dib,
    bright: Dib,
    back_hdc: HDC,
    back_bmp: HBITMAP,
    back_old: HGDIOBJ,
}

impl Drop for Overlay {
    fn drop(&mut self) {
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            let _ = SelectObject(self.back_hdc, self.back_old);
            let _ = DeleteObject(self.back_bmp.into());
            let _ = DeleteDC(self.back_hdc);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

struct Session {
    image: *const RgbaImage,
    shot_origin_x: i32,
    shot_origin_y: i32,
    overlays: Vec<Overlay>,
    anchor: Option<(i32, i32)>,
    last: Option<(i32, i32, i32, i32)>,
    done: Option<Option<RgbaImage>>,
    cursor: HCURSOR,
}

pub fn select_region(shot: &DesktopShot) -> Result<Option<RgbaImage>> {
    // Dedicated overlay thread: CreateWindow + PeekMessage + DestroyWindow +
    // ReleaseCapture, then exit. Grab stays on the GPUI background pool.
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .name("localtex-snip".into())
            .spawn_scoped(s, || run_overlay(shot))
            .map_err(|err| anyhow!("snip overlay thread: {err}"))?
            .join()
            .unwrap_or_else(|_| Err(anyhow!("snip overlay thread panicked")))
    })
}

fn run_overlay(shot: &DesktopShot) -> Result<Option<RgbaImage>> {
    eprintln!(
        "{APP_SLUG}: win snip {}x{} origin {},{}",
        shot.image.width(),
        shot.image.height(),
        shot.origin_x,
        shot.origin_y
    );

    let rects = overlay_rects(shot);
    for r in &rects {
        eprintln!(
            "{APP_SLUG}: win snip monitor {}x{} at {},{}",
            r.2, r.3, r.0, r.1
        );
    }

    unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    win::reset_pointer_state();

    register_class()?;

    let instance = unsafe { GetModuleHandleW(None) }.map_err(|err| anyhow!(err))?;
    let overlay_cursor = OverlayCursor::load(instance.into());
    let mut session = Session {
        image: &shot.image,
        shot_origin_x: shot.origin_x,
        shot_origin_y: shot.origin_y,
        overlays: Vec::new(),
        anchor: None,
        last: None,
        done: None,
        cursor: overlay_cursor.handle,
    };
    for &(x, y, w, h) in &rects {
        let (dim, bright) =
            rgba_to_dib_pair(&shot.image, (shot.origin_x, shot.origin_y), (x, y, w, h))?;
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                CLASS_NAME,
                w!("LocalTeX Overlay"),
                WS_POPUP,
                x,
                y,
                w,
                h,
                None,
                None,
                Some(HINSTANCE(instance.0)),
                None,
            )
        }
        .map_err(|err| anyhow!("create snip overlay: {err}"))?;
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, &mut session as *mut Session as isize);
        }
        let (back_hdc, back_bmp, back_old) = match create_back_buffer(hwnd, dim.width, dim.height) {
            Ok(back) => back,
            Err(err) => {
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    let _ = DestroyWindow(hwnd);
                }
                return Err(err);
            }
        };
        session.overlays.push(Overlay {
            hwnd,
            origin_x: x,
            origin_y: y,
            dim,
            bright,
            back_hdc,
            back_bmp,
            back_old,
        });
    }
    if session.overlays.is_empty() {
        return Err(anyhow!("no snip overlay window"));
    }
    let escape_registered =
        unsafe { RegisterHotKey(None, ESCAPE_HOTKEY_ID, MOD_NOREPEAT, u32::from(VK_ESCAPE.0)) }
            .is_ok();
    if !escape_registered {
        eprintln!("{APP_SLUG}: register snip Escape hotkey failed");
    }
    for overlay in &session.overlays {
        unsafe {
            SetWindowPos(
                overlay.hwnd,
                Some(HWND_TOPMOST),
                overlay.origin_x,
                overlay.origin_y,
                overlay.dim.width,
                overlay.dim.height,
                SWP_SHOWWINDOW,
            )
            .ok();
            let _ = ShowWindow(overlay.hwnd, SW_SHOW);
        }
    }
    unsafe {
        let hwnd = session.overlays[0].hwnd;
        let _ = SetForegroundWindow(hwnd);
        overlay_cursor.apply();
    }

    let deadline = Instant::now() + OVERLAY_TIMEOUT;
    while session.done.is_none() {
        if Instant::now() >= deadline {
            eprintln!("{APP_SLUG}: win snip timed out");
            cancel_snip(&mut session);
            break;
        }
        let mut msg = MSG::default();
        let had = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) };
        if had.as_bool() {
            if is_escape_hotkey(msg.message, msg.wParam) {
                cancel_snip(&mut session);
                continue;
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        } else {
            std::thread::sleep(Duration::from_millis(8));
        }
    }

    if escape_registered {
        let _ = unsafe { UnregisterHotKey(None, ESCAPE_HOTKEY_ID) };
    }
    let crop = session.done.take().flatten();
    session.overlays.clear();
    win::reset_pointer_state();
    Ok(crop)
}

fn is_escape_hotkey(message: u32, wparam: WPARAM) -> bool {
    message == WM_HOTKEY && wparam.0 as i32 == ESCAPE_HOTKEY_ID
}

fn overlay_rects(shot: &DesktopShot) -> Vec<(i32, i32, i32, i32)> {
    let mut rects = Vec::new();
    for &(x, y, w, h) in &shot.monitors {
        if let Some(rect) = clip_monitor_to_shot(x, y, w, h, shot) {
            rects.push(rect);
        }
    }
    if rects.is_empty() {
        let w = i32::try_from(shot.image.width()).unwrap_or(0);
        let h = i32::try_from(shot.image.height()).unwrap_or(0);
        if w > 0 && h > 0 {
            rects.push((shot.origin_x, shot.origin_y, w, h));
        }
    }
    rects
}

fn clip_monitor_to_shot(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    shot: &DesktopShot,
) -> Option<(i32, i32, i32, i32)> {
    let shot_w = i32::try_from(shot.image.width()).ok()?;
    let shot_h = i32::try_from(shot.image.height()).ok()?;
    let (ix0, iy0, ix1, iy1) = intersect(
        (x, y, x.saturating_add(w), y.saturating_add(h)),
        (
            shot.origin_x,
            shot.origin_y,
            shot.origin_x.saturating_add(shot_w),
            shot.origin_y.saturating_add(shot_h),
        ),
    )?;
    Some((ix0, iy0, ix1 - ix0, iy1 - iy0))
}

fn intersect(
    (ax0, ay0, ax1, ay1): (i32, i32, i32, i32),
    (bx0, by0, bx1, by1): (i32, i32, i32, i32),
) -> Option<(i32, i32, i32, i32)> {
    let x0 = ax0.max(bx0);
    let y0 = ay0.max(by0);
    let x1 = ax1.min(bx1);
    let y1 = ay1.min(by1);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some((x0, y0, x1, y1))
}

fn register_class() -> Result<()> {
    let instance = unsafe { GetModuleHandleW(None) }.map_err(|err| anyhow!(err))?;
    let class = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: instance.into(),
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
        lpszClassName: CLASS_NAME,
        ..Default::default()
    };
    let atom = unsafe { RegisterClassW(&class) };
    if atom == 0 {
        let err = windows::core::Error::from_win32();
        // Already registered in this process (second snip).
        if err.code().0 & 0xFFFF == 1410 {
            return Ok(());
        }
        return Err(anyhow!("RegisterClassW: {err}"));
    }
    Ok(())
}

fn rgba_to_dib_pair(
    img: &RgbaImage,
    (shot_ox, shot_oy): (i32, i32),
    (x, y, w, h): (i32, i32, i32, i32),
) -> Result<(Dib, Dib)> {
    let src_x = u32::try_from(x - shot_ox).map_err(|_| anyhow!("slice x"))?;
    let src_y = u32::try_from(y - shot_oy).map_err(|_| anyhow!("slice y"))?;
    if w <= 0 || h <= 0 {
        return Err(anyhow!("empty overlay slice"));
    }
    let width = w;
    let height = h;
    let src_w = img.width();
    let src_h = img.height();
    if src_x.saturating_add(w as u32) > src_w || src_y.saturating_add(h as u32) > src_h {
        return Err(anyhow!("slice outside freeze-frame"));
    }
    let stride = (width as usize) * 4;
    let src_stride = src_w as usize * 4;
    let raw = img.as_raw();
    let mut dim_bits = vec![0u8; stride * height as usize];
    let mut bright_bits = vec![0u8; stride * height as usize];
    for row in 0..height as usize {
        let src_row = (src_y as usize + row) * src_stride + src_x as usize * 4;
        let dst_row = (height as usize - 1 - row) * stride;
        for col in 0..width as usize {
            let s = src_row + col * 4;
            let o = dst_row + col * 4;
            let r = raw[s];
            let g = raw[s + 1];
            let b = raw[s + 2];
            let d = capture::dim_luma(r, g, b);
            dim_bits[o] = d;
            dim_bits[o + 1] = d;
            dim_bits[o + 2] = d;
            dim_bits[o + 3] = 255;
            bright_bits[o] = b;
            bright_bits[o + 1] = g;
            bright_bits[o + 2] = r;
            bright_bits[o + 3] = 255;
        }
    }
    Ok((
        dib_from_bits(dim_bits, width, height),
        dib_from_bits(bright_bits, width, height),
    ))
}

fn dib_from_bits(bits: Vec<u8>, width: i32, height: i32) -> Dib {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: bits.len() as u32,
            ..Default::default()
        },
        ..Default::default()
    };
    Dib {
        info,
        bits,
        width,
        height,
    }
}

fn create_back_buffer(hwnd: HWND, width: i32, height: i32) -> Result<(HDC, HBITMAP, HGDIOBJ)> {
    unsafe {
        let screen = GetDC(Some(hwnd));
        if screen.is_invalid() {
            return Err(anyhow!("GetDC for snip back buffer"));
        }
        let hdc = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, width, height);
        ReleaseDC(Some(hwnd), screen);
        if hdc.is_invalid() || bmp.is_invalid() {
            if !hdc.is_invalid() {
                let _ = DeleteDC(hdc);
            }
            if !bmp.is_invalid() {
                let _ = DeleteObject(bmp.into());
            }
            return Err(anyhow!("create snip back buffer"));
        }
        let old = SelectObject(hdc, bmp.into());
        Ok((hdc, bmp, old))
    }
}

fn overlay_for(session: &Session, hwnd: HWND) -> Option<&Overlay> {
    session.overlays.iter().find(|o| o.hwnd == hwnd)
}

fn to_canvas(session: &Session, hwnd: HWND, client_x: i32, client_y: i32) -> Option<(i32, i32)> {
    let overlay = overlay_for(session, hwnd)?;
    Some((
        overlay.origin_x - session.shot_origin_x + client_x,
        overlay.origin_y - session.shot_origin_y + client_y,
    ))
}

fn cursor_canvas(session: &Session, hwnd: HWND) -> Option<(i32, i32)> {
    let mut pt = POINT::default();
    if unsafe { GetCursorPos(&mut pt) }.is_err() {
        return None;
    }
    let _ = unsafe { ScreenToClient(hwnd, &mut pt) };
    to_canvas(session, hwnd, pt.x, pt.y)
}

fn invalidate_selection(
    session: &Session,
    old: Option<(i32, i32, i32, i32)>,
    new: Option<(i32, i32, i32, i32)>,
) {
    for overlay in &session.overlays {
        let mut dirty: Option<RECT> = None;
        for rect in [old, new].into_iter().flatten() {
            if let Some(client) = selection_client_rect(session, overlay, rect) {
                dirty = Some(match dirty {
                    Some(prev) => union_rect(prev, client),
                    None => client,
                });
            }
        }
        if let Some(rc) = dirty {
            unsafe {
                let _ = InvalidateRect(Some(overlay.hwnd), Some(&rc), false);
            }
        }
    }
}

fn selection_client_rect(
    session: &Session,
    overlay: &Overlay,
    rect: (i32, i32, i32, i32),
) -> Option<RECT> {
    let src_x = overlay.origin_x - session.shot_origin_x;
    let src_y = overlay.origin_y - session.shot_origin_y;
    let (ix0, iy0, ix1, iy1) = intersect(
        rect,
        (
            src_x,
            src_y,
            src_x.saturating_add(overlay.dim.width),
            src_y.saturating_add(overlay.dim.height),
        ),
    )?;
    Some(RECT {
        left: (ix0 - src_x - 1).max(0),
        top: (iy0 - src_y - 1).max(0),
        right: (ix1 - src_x + 1).min(overlay.dim.width),
        bottom: (iy1 - src_y + 1).min(overlay.dim.height),
    })
}

fn union_rect(a: RECT, b: RECT) -> RECT {
    RECT {
        left: a.left.min(b.left),
        top: a.top.min(b.top),
        right: a.right.max(b.right),
        bottom: a.bottom.max(b.bottom),
    }
}

fn finish_drag(session: &mut Session, bx: i32, by: i32) {
    let Some((ax, ay)) = session.anchor.take() else {
        session.done = Some(None);
        return;
    };
    // Safety: `image` lives for the whole `select_region` call.
    let image = unsafe { &*session.image };
    session.done = Some(capture::crop_selection(image, ax, ay, bx, by));
}

fn cancel_snip(session: &mut Session) {
    win::reset_pointer_state();
    session.anchor = None;
    session.done = Some(None);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let session = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Session;
    if session.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    let session = unsafe { &mut *session };
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_SETCURSOR => {
            unsafe { SetCursor(Some(session.cursor)) };
            LRESULT(1)
        }
        WM_PAINT => {
            paint(hwnd, session);
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(1),
        WM_LBUTTONDOWN => {
            if let Some(pt) = to_canvas(session, hwnd, lparam_x(lparam), lparam_y(lparam)) {
                session.anchor = Some(pt);
                session.last = None;
                unsafe { SetCapture(hwnd) };
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if session.anchor.is_some() {
                if let Some(pt) = cursor_canvas(session, hwnd)
                    .or_else(|| to_canvas(session, hwnd, lparam_x(lparam), lparam_y(lparam)))
                {
                    if let Some((ax, ay)) = session.anchor {
                        let next = norm_rect(ax, ay, pt.0, pt.1);
                        if session.last != Some(next) {
                            let prev = session.last;
                            session.last = Some(next);
                            invalidate_selection(session, prev, session.last);
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let _ = unsafe { ReleaseCapture() };
            // Snip-button mouse-up can land on the overlay. Without an
            // overlay LBUTTONDOWN that must not cancel the snip.
            if session.anchor.is_none() {
                return LRESULT(0);
            }
            let pt = cursor_canvas(session, hwnd)
                .or_else(|| to_canvas(session, hwnd, lparam_x(lparam), lparam_y(lparam)));
            if let Some((bx, by)) = pt {
                finish_drag(session, bx, by);
            } else {
                session.done = Some(None);
            }
            LRESULT(0)
        }
        WM_RBUTTONUP | WM_DISPLAYCHANGE => {
            cancel_snip(session);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 as u16 == VK_ESCAPE.0 {
                cancel_snip(session);
            }
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn lparam_x(lparam: LPARAM) -> i32 {
    (lparam.0 as i16) as i32
}

fn lparam_y(lparam: LPARAM) -> i32 {
    ((lparam.0 >> 16) as i16) as i32
}

fn norm_rect(ax: i32, ay: i32, bx: i32, by: i32) -> (i32, i32, i32, i32) {
    (ax.min(bx), ay.min(by), ax.max(bx), ay.max(by))
}

fn paint(hwnd: HWND, session: &Session) {
    let Some(overlay) = overlay_for(session, hwnd) else {
        return;
    };
    let mut ps = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
    if hdc.is_invalid() {
        return;
    }
    // Compose dim + bright + frame off-screen, then BitBlt once. Painting dim
    // onto the window DC first made the selection strobe dark on every move.
    compose(overlay, session, overlay.back_hdc, ps.rcPaint);
    let x = ps.rcPaint.left;
    let y = ps.rcPaint.top;
    let w = (ps.rcPaint.right - ps.rcPaint.left).max(0);
    let h = (ps.rcPaint.bottom - ps.rcPaint.top).max(0);
    if w > 0 && h > 0 {
        let _ = unsafe { BitBlt(hdc, x, y, w, h, Some(overlay.back_hdc), x, y, SRCCOPY) };
    }
    let _ = unsafe { EndPaint(hwnd, &ps) };
}

fn compose(overlay: &Overlay, session: &Session, hdc: HDC, clip: RECT) {
    if clip.right <= clip.left || clip.bottom <= clip.top {
        return;
    }
    let src_x = overlay.origin_x - session.shot_origin_x;
    let src_y = overlay.origin_y - session.shot_origin_y;
    let saved = unsafe { SaveDC(hdc) };
    if saved != 0 {
        unsafe { IntersectClipRect(hdc, clip.left, clip.top, clip.right, clip.bottom) };
    }
    blit_full(hdc, &overlay.dim);
    if let Some((x0, y0, x1, y1)) = session.last {
        if let Some((ix0, iy0, ix1, iy1)) = intersect(
            (x0, y0, x1, y1),
            (
                src_x,
                src_y,
                src_x.saturating_add(overlay.bright.width),
                src_y.saturating_add(overlay.bright.height),
            ),
        ) {
            let dx = ix0 - src_x;
            let dy = iy0 - src_y;
            let inner = unsafe { SaveDC(hdc) };
            if inner != 0 {
                unsafe { IntersectClipRect(hdc, dx, dy, ix1 - src_x, iy1 - src_y) };
                blit_full(hdc, &overlay.bright);
                let _ = unsafe { RestoreDC(hdc, inner) };
            }
            let accent = unsafe {
                CreateSolidBrush(windows::Win32::Foundation::COLORREF(
                    u32::from(ACCENT.2) | u32::from(ACCENT.1) << 8 | u32::from(ACCENT.0) << 16,
                ))
            };
            if !accent.is_invalid() {
                let rc = RECT {
                    left: dx,
                    top: dy,
                    right: ix1 - src_x,
                    bottom: iy1 - src_y,
                };
                unsafe { FrameRect(hdc, &rc, accent) };
                let _ = unsafe { DeleteObject(accent.into()) };
            }
        }
    }
    if saved != 0 {
        let _ = unsafe { RestoreDC(hdc, saved) };
    }
}

fn blit_full(hdc: HDC, dib: &Dib) {
    if dib.width <= 0 || dib.height <= 0 {
        return;
    }
    unsafe {
        StretchDIBits(
            hdc,
            0,
            0,
            dib.width,
            dib.height,
            0,
            0,
            dib.width,
            dib.height,
            Some(dib.bits.as_ptr().cast()),
            &dib.info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn shot(origin_x: i32, origin_y: i32, w: u32, h: u32) -> DesktopShot {
        DesktopShot {
            image: RgbaImage::from_pixel(w, h, Rgba([1, 2, 3, 255])),
            origin_x,
            origin_y,
            monitors: vec![(origin_x, origin_y, w as i32, h as i32)],
        }
    }

    #[test]
    fn clip_keeps_monitor_inside_virtual_desktop() {
        let shot = shot(0, 0, 20, 10);
        assert_eq!(
            clip_monitor_to_shot(0, 0, 10, 10, &shot),
            Some((0, 0, 10, 10))
        );
        assert_eq!(
            clip_monitor_to_shot(10, 0, 20, 10, &shot),
            Some((10, 0, 10, 10))
        );
        assert_eq!(clip_monitor_to_shot(50, 0, 10, 10, &shot), None);
    }

    #[test]
    fn clip_handles_negative_virtual_origin() {
        let shot = shot(-10, 0, 20, 8);
        assert_eq!(
            clip_monitor_to_shot(-10, 0, 10, 8, &shot),
            Some((-10, 0, 10, 8))
        );
        assert_eq!(
            clip_monitor_to_shot(0, 0, 10, 8, &shot),
            Some((0, 0, 10, 8))
        );
    }

    #[test]
    fn canvas_from_client_matches_shot_space() {
        let session_origin = (-1920, 0);
        let overlay_origin = (0, 0);
        let canvas_x = overlay_origin.0 - session_origin.0 + 10;
        let canvas_y = overlay_origin.1 - session_origin.1 + 20;
        assert_eq!((canvas_x, canvas_y), (1930, 20));
    }

    #[test]
    fn overlay_rects_stacked_monitors_keep_each_screen() {
        let mut img = RgbaImage::from_pixel(20, 18, Rgba([0, 0, 0, 255]));
        for y in 0..10 {
            for x in 0..20 {
                img.put_pixel(x, y, Rgba([10, 0, 0, 255]));
            }
        }
        for y in 10..18 {
            for x in 2..12 {
                img.put_pixel(x, y, Rgba([20, 0, 0, 255]));
            }
        }
        let shot = DesktopShot {
            image: img,
            origin_x: 0,
            origin_y: 0,
            monitors: vec![(0, 0, 20, 10), (2, 10, 10, 8)],
        };
        let primary = crate::imgutil::crop(&shot.image, 0, 0, 20, 10);
        let secondary = crate::imgutil::crop(&shot.image, 2, 10, 10, 8);
        assert_eq!(primary.get_pixel(0, 0).0, [10, 0, 0, 255]);
        assert_eq!(secondary.get_pixel(0, 0).0, [20, 0, 0, 255]);
        assert_eq!(overlay_rects(&shot), vec![(0, 0, 20, 10), (2, 10, 10, 8)]);
    }

    #[test]
    fn dib_pair_builds_dim_and_bright_pixels_together() {
        let img = RgbaImage::from_pixel(1, 1, Rgba([10, 20, 30, 40]));
        let (dim, bright) = rgba_to_dib_pair(&img, (0, 0), (0, 0, 1, 1)).unwrap();
        let d = capture::dim_luma(10, 20, 30);
        assert_eq!(dim.bits, [d, d, d, 255]);
        assert_eq!(bright.bits, [30, 20, 10, 255]);
    }

    #[test]
    fn dirty_union_covers_old_and_new_selection() {
        let old = RECT {
            left: 10,
            top: 10,
            right: 40,
            bottom: 40,
        };
        let new = RECT {
            left: 30,
            top: 20,
            right: 80,
            bottom: 50,
        };
        let u = union_rect(old, new);
        assert_eq!((u.left, u.top, u.right, u.bottom), (10, 10, 80, 50));
    }

    #[test]
    fn escape_hotkey_matches_only_its_registered_message() {
        assert!(is_escape_hotkey(
            WM_HOTKEY,
            WPARAM(ESCAPE_HOTKEY_ID as usize)
        ));
        assert!(!is_escape_hotkey(
            WM_HOTKEY,
            WPARAM((ESCAPE_HOTKEY_ID + 1) as usize)
        ));
        assert!(!is_escape_hotkey(
            WM_KEYDOWN,
            WPARAM(ESCAPE_HOTKEY_ID as usize)
        ));
    }
}
