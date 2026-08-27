//! Override-redirect freeze-frame overlay. Own X connection; never touches
//! GPUI's window (a second Vulkan swapchain Xid-faults; EWMH on the main
//! XID races GPUI's event loop).

use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use image::RgbaImage;
use x11rb::connection::Connection;
use x11rb::image::{BitsPerPixel, ColorComponent, Image, ImageOrder, PixelLayout, ScanlinePad};
use x11rb::protocol::xproto::{
    ConnectionExt, CreateGCAux, CreateWindowAux, EventMask, Gcontext, GrabMode, GrabStatus,
    Rectangle, Screen, Window, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::{CURRENT_TIME, NONE};

use crate::capture::{self, DesktopShot};
use crate::identity::APP_SLUG;

const XK_ESCAPE: u32 = 0xff1b;
const ACCENT: (u8, u8, u8) = (0x25, 0x63, 0xeb);
const GRAB_TRIES: u32 = 15;
const GRAB_GAP: Duration = Duration::from_millis(20);
/// Safety net if the compositor never delivers a button/key event.
const OVERLAY_TIMEOUT: Duration = Duration::from_secs(300);

pub fn select_region(shot: &DesktopShot) -> Result<Option<RgbaImage>> {
    eprintln!(
        "{APP_SLUG}: x11 snip {}x{} origin {},{}",
        shot.image.width(),
        shot.image.height(),
        shot.origin_x,
        shot.origin_y
    );
    let (conn, screen_num) = x11rb::connect(None).context("x11 overlay connect")?;
    let screen = &conn.setup().roots[screen_num];
    run(&conn, screen, shot)
}

fn run<C: Connection>(conn: &C, screen: &Screen, shot: &DesktopShot) -> Result<Option<RgbaImage>> {
    // CreateWindow x/y are i16; width/height are u16. Refuse canvases that
    // cannot be placed as a single override-redirect window.
    let width = u16::try_from(shot.image.width()).context("shot too wide for X11")?;
    let height = u16::try_from(shot.image.height()).context("shot too tall for X11")?;
    let x = i16::try_from(shot.origin_x).context("shot origin x does not fit i16")?;
    let y = i16::try_from(shot.origin_y).context("shot origin y does not fit i16")?;

    let visual = visual_layout(screen)?;
    let bright = rgba_to_native(&shot.image, visual, conn.setup())?;
    let dim = rgba_to_native(&capture::dim_copy(&shot.image), visual, conn.setup())?;

    let win = conn.generate_id()?;
    let pix_bright = conn.generate_id()?;
    let pix_dim = conn.generate_id()?;
    let gc = conn.generate_id()?;
    let gc_rect = conn.generate_id()?;

    conn.create_gc(gc, screen.root, &CreateGCAux::new().graphics_exposures(0))?;
    conn.create_pixmap(screen.root_depth, pix_bright, screen.root, width, height)?;
    conn.create_pixmap(screen.root_depth, pix_dim, screen.root, width, height)?;
    bright.put(conn, pix_bright, gc, 0, 0)?;
    dim.put(conn, pix_dim, gc, 0, 0)?;

    let accent = visual.encode((
        u16::from(ACCENT.0) * 257,
        u16::from(ACCENT.1) * 257,
        u16::from(ACCENT.2) * 257,
    ));
    conn.create_gc(
        gc_rect,
        screen.root,
        &CreateGCAux::new()
            .graphics_exposures(0)
            .foreground(accent)
            .line_width(2),
    )?;

    conn.create_window(
        screen.root_depth,
        win,
        screen.root,
        x,
        y,
        width,
        height,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new()
            .override_redirect(1)
            .event_mask(
                EventMask::EXPOSURE
                    | EventMask::BUTTON_PRESS
                    | EventMask::BUTTON_RELEASE
                    | EventMask::POINTER_MOTION
                    | EventMask::KEY_PRESS,
            )
            .background_pixmap(pix_dim),
    )?;

    let mut session = Session {
        conn,
        win,
        pix_bright,
        pix_dim,
        gc,
        gc_rect,
        grabbed: false,
    };
    conn.map_window(win)?;
    conn.configure_window(
        win,
        &x11rb::protocol::xproto::ConfigureWindowAux::new()
            .stack_mode(x11rb::protocol::xproto::StackMode::ABOVE),
    )?;
    conn.flush()?;

    session.grab()?;
    let escape = escape_keycodes(conn)?;
    let crop = session.event_loop(width, height, &shot.image, &escape)?;
    Ok(crop)
}

struct Session<'a, C: Connection> {
    conn: &'a C,
    win: Window,
    pix_bright: u32,
    pix_dim: u32,
    gc: Gcontext,
    gc_rect: Gcontext,
    grabbed: bool,
}

impl<C: Connection> Drop for Session<'_, C> {
    fn drop(&mut self) {
        if self.grabbed {
            let _ = self.conn.ungrab_pointer(CURRENT_TIME);
            let _ = self.conn.ungrab_keyboard(CURRENT_TIME);
            self.grabbed = false;
        }
        let _ = self.conn.unmap_window(self.win);
        let _ = self.conn.destroy_window(self.win);
        let _ = self.conn.free_pixmap(self.pix_bright);
        let _ = self.conn.free_pixmap(self.pix_dim);
        let _ = self.conn.free_gc(self.gc);
        let _ = self.conn.free_gc(self.gc_rect);
        let _ = self.conn.flush();
    }
}

impl<C: Connection> Session<'_, C> {
    fn grab(&mut self) -> Result<()> {
        // Pointer-grab mask cannot include keyboard events (X11 BadValue 2).
        let mask = EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION;
        for i in 0..GRAB_TRIES {
            let pointer = self
                .conn
                .grab_pointer(
                    false,
                    self.win,
                    mask,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                    NONE,
                    NONE,
                    CURRENT_TIME,
                )?
                .reply()?;
            if pointer.status == GrabStatus::SUCCESS {
                // Pointer is held; mark grabbed so Drop ungrabs if keyboard reply fails.
                self.grabbed = true;
                let keyboard = self
                    .conn
                    .grab_keyboard(
                        false,
                        self.win,
                        CURRENT_TIME,
                        GrabMode::ASYNC,
                        GrabMode::ASYNC,
                    )?
                    .reply()?;
                if keyboard.status == GrabStatus::SUCCESS {
                    return Ok(());
                }
                let _ = self.conn.ungrab_pointer(CURRENT_TIME);
                let _ = self.conn.ungrab_keyboard(CURRENT_TIME);
                self.grabbed = false;
            }
            if i + 1 < GRAB_TRIES {
                std::thread::sleep(GRAB_GAP);
            }
        }
        Err(anyhow!("could not grab pointer/keyboard for snip"))
    }

    fn event_loop(
        &self,
        width: u16,
        height: u16,
        image: &RgbaImage,
        escape: &[u8],
    ) -> Result<Option<RgbaImage>> {
        let mut anchor: Option<(i16, i16)> = None;
        let mut last_sel: Option<Rectangle> = None;
        self.paint_full_dim(width, height)?;
        self.conn.flush()?;
        let deadline = Instant::now() + OVERLAY_TIMEOUT;

        loop {
            if Instant::now() >= deadline {
                eprintln!("{APP_SLUG}: x11 snip timed out");
                return Ok(None);
            }
            let Some(event) = self.conn.poll_for_event()? else {
                std::thread::sleep(Duration::from_millis(16));
                continue;
            };
            match event {
                Event::Expose(_) => {
                    self.paint_full_dim(width, height)?;
                    if let Some(sel) = last_sel {
                        self.paint_sel(sel)?;
                    }
                    self.conn.flush()?;
                }
                Event::ButtonPress(ev) => {
                    if ev.detail == 3 {
                        return Ok(None);
                    }
                    if ev.detail == 1 {
                        anchor = Some((ev.event_x, ev.event_y));
                    }
                }
                Event::MotionNotify(ev) => {
                    if let Some(a) = anchor {
                        let sel = rect_from_drag(a, (ev.event_x, ev.event_y), width, height);
                        self.restripe(last_sel, sel, width, height)?;
                        last_sel = sel;
                        self.conn.flush()?;
                    }
                }
                Event::ButtonRelease(ev) => {
                    if ev.detail != 1 {
                        continue;
                    }
                    let Some(a) = anchor.take() else {
                        return Ok(None);
                    };
                    return Ok(capture::crop_selection(
                        image,
                        i32::from(a.0),
                        i32::from(a.1),
                        i32::from(ev.event_x),
                        i32::from(ev.event_y),
                    ));
                }
                Event::KeyPress(ev) => {
                    if escape.contains(&ev.detail) {
                        return Ok(None);
                    }
                }
                Event::Error(err) => {
                    eprintln!("{APP_SLUG}: x11 snip error: {err:?}");
                }
                _ => {}
            }
        }
    }

    fn paint_full_dim(&self, width: u16, height: u16) -> Result<()> {
        self.conn
            .copy_area(self.pix_dim, self.win, self.gc, 0, 0, 0, 0, width, height)?;
        Ok(())
    }

    fn restripe(
        &self,
        prev: Option<Rectangle>,
        next: Option<Rectangle>,
        width: u16,
        height: u16,
    ) -> Result<()> {
        if let Some(prev) = prev {
            let r = inflate(prev, width, height);
            self.conn.copy_area(
                self.pix_dim,
                self.win,
                self.gc,
                r.x,
                r.y,
                r.x,
                r.y,
                r.width,
                r.height,
            )?;
        }
        if let Some(sel) = next {
            self.paint_sel(sel)?;
        }
        Ok(())
    }

    fn paint_sel(&self, sel: Rectangle) -> Result<()> {
        if sel.width == 0 || sel.height == 0 {
            return Ok(());
        }
        self.conn.copy_area(
            self.pix_bright,
            self.win,
            self.gc,
            sel.x,
            sel.y,
            sel.x,
            sel.y,
            sel.width,
            sel.height,
        )?;
        self.conn.poly_rectangle(self.win, self.gc_rect, &[sel])?;
        Ok(())
    }
}

fn inflate(sel: Rectangle, max_w: u16, max_h: u16) -> Rectangle {
    let pad = 3i16;
    let x = sel.x.saturating_sub(pad).max(0);
    let y = sel.y.saturating_sub(pad).max(0);
    let right = (i32::from(sel.x) + i32::from(sel.width) + i32::from(pad)).min(i32::from(max_w));
    let bottom = (i32::from(sel.y) + i32::from(sel.height) + i32::from(pad)).min(i32::from(max_h));
    Rectangle {
        x,
        y,
        width: (right - i32::from(x)).max(0) as u16,
        height: (bottom - i32::from(y)).max(0) as u16,
    }
}

fn rect_from_drag(a: (i16, i16), b: (i16, i16), max_w: u16, max_h: u16) -> Option<Rectangle> {
    let max_x = i32::from(max_w);
    let max_y = i32::from(max_h);
    let x0 = i32::from(a.0).min(i32::from(b.0)).clamp(0, max_x);
    let y0 = i32::from(a.1).min(i32::from(b.1)).clamp(0, max_y);
    let x1 = i32::from(a.0).max(i32::from(b.0)).clamp(0, max_x);
    let y1 = i32::from(a.1).max(i32::from(b.1)).clamp(0, max_y);
    let w = x1 - x0;
    let h = y1 - y0;
    if w <= 0 || h <= 0 {
        return None;
    }
    Some(Rectangle {
        x: x0 as i16,
        y: y0 as i16,
        width: w as u16,
        height: h as u16,
    })
}

fn visual_layout(screen: &Screen) -> Result<PixelLayout> {
    let info = screen
        .allowed_depths
        .iter()
        .find_map(|depth| {
            depth
                .visuals
                .iter()
                .find(|v| v.visual_id == screen.root_visual)
                .copied()
        })
        .ok_or_else(|| anyhow!("root visual not found"))?;
    PixelLayout::from_visual_type(info).context("root visual is not truecolor")
}

fn rgba_to_native(
    img: &RgbaImage,
    visual: PixelLayout,
    setup: &x11rb::protocol::xproto::Setup,
) -> Result<Image<'static>> {
    let w = u16::try_from(img.width())?;
    let h = u16::try_from(img.height())?;
    let rgb = PixelLayout::new(
        ColorComponent::new(8, 16)?,
        ColorComponent::new(8, 8)?,
        ColorComponent::new(8, 0)?,
    );
    let mut packed = Image::allocate(
        w,
        h,
        ScanlinePad::Pad32,
        24,
        BitsPerPixel::B32,
        ImageOrder::MsbFirst,
    );
    for (x, y, pixel) in img.enumerate_pixels() {
        let value =
            u32::from(pixel.0[0]) << 16 | u32::from(pixel.0[1]) << 8 | u32::from(pixel.0[2]);
        packed.put_pixel(x as u16, y as u16, value);
    }
    Ok(packed.reencode(rgb, visual, setup)?.into_owned())
}

fn escape_keycodes<C: Connection>(conn: &C) -> Result<Vec<u8>> {
    let setup = conn.setup();
    let min = setup.min_keycode;
    let count = setup.max_keycode.saturating_sub(min).saturating_add(1);
    let reply = conn.get_keyboard_mapping(min, count)?.reply()?;
    let per = reply.keysyms_per_keycode as usize;
    if per == 0 {
        return Ok(vec![9]);
    }
    let mut codes = Vec::new();
    for (i, chunk) in reply.keysyms.chunks(per).enumerate() {
        if chunk.contains(&XK_ESCAPE) {
            codes.push(min.saturating_add(i as u8));
        }
    }
    if codes.is_empty() {
        codes.push(9);
    }
    Ok(codes)
}

#[cfg(test)]
mod tests {
    #[test]
    fn overlay_does_not_touch_foreign_ewmh() {
        let src = include_str!("x11_snip.rs");
        let prod = src.split("#[cfg(test)]").next().expect("prod");
        assert!(
            !prod.contains("NET_ACTIVE_WINDOW"),
            "snip overlay must not send _NET_ACTIVE_WINDOW"
        );
        assert!(
            !prod.contains("NET_WM_STATE"),
            "snip overlay must not send EWMH to the main window"
        );
        let grab = prod
            .split("fn grab(")
            .nth(1)
            .expect("grab")
            .split("fn event_loop")
            .next()
            .expect("event_loop");
        assert!(
            !grab.contains("KEY_PRESS"),
            "GrabPointer event-mask cannot include KEY_PRESS (X11 BadValue)"
        );
        assert!(grab.contains("BUTTON_PRESS") && grab.contains("POINTER_MOTION"));
    }
}
