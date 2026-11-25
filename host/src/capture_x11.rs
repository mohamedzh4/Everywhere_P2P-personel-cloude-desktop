//! Linux screen capture on X11 (Xorg sessions, and X apps under XWayland).
//! The picture comes from the root window; the mouse pointer, which X leaves
//! out, is read with XFixes and drawn in like on Windows.
//!
//! Wayland sessions need the desktop portal + PipeWire instead (later); there
//! the root window is black or shows only X apps, so we say so and stop.

use x11rb::connection::Connection;
use x11rb::protocol::xfixes::ConnectionExt as _;
use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat, Window};
use x11rb::rust_connection::RustConnection;

use super::{bgra_to_i420, Cursor, Px, RawFrame, Source};

pub struct Screen {
    conn: RustConnection,
    root: Window,
    src_w: usize,
    src_h: usize,
    width: usize,
    height: usize,
    xfixes: bool,
    cursor: Option<(u32, Cursor)>,
}

/// True when this session is Wayland (X capture would not show the desktop).
pub fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t.eq_ignore_ascii_case("wayland"))
}

impl Screen {
    pub fn new(max_w: usize, max_h: usize) -> Result<Self, String> {
        if is_wayland() {
            return Err("this is a Wayland session; screen sharing needs an Xorg session for now \
                        (on the login screen choose \"Ubuntu on Xorg\" or similar)"
                .into());
        }
        let (conn, n) = x11rb::connect(None).map_err(|e| format!("cannot reach the X server (DISPLAY): {e}"))?;
        let screen = &conn.setup().roots[n];
        let (root, src_w, src_h) = (screen.root, screen.width_in_pixels as usize, screen.height_in_pixels as usize);
        if screen.root_depth != 24 && screen.root_depth != 32 {
            return Err(format!("unsupported screen depth {}", screen.root_depth));
        }
        let xfixes = conn
            .xfixes_query_version(5, 0)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some();
        let scale = (max_w as f64 / src_w as f64).min(max_h as f64 / src_h as f64).min(1.0);
        let width = ((src_w as f64 * scale) as usize) & !1;
        let height = ((src_h as f64 * scale) as usize) & !1;
        crate::say!("screen {src_w}x{src_h}, streaming at {width}x{height}");
        Ok(Self { conn, root, src_w, src_h, width, height, xfixes, cursor: None })
    }

    /// The pointer and its top-left corner, refreshed only when it changes.
    fn pointer(&mut self) -> Option<(i32, i32)> {
        let img = self.conn.xfixes_get_cursor_image().ok()?.reply().ok()?;
        let fresh = !matches!(&self.cursor, Some((serial, _)) if *serial == img.cursor_serial);
        if fresh {
            let px = img
                .cursor_image
                .iter()
                .map(|&argb| {
                    let a = (argb >> 24) as u8;
                    if a == 0 {
                        return Px::Clear;
                    }
                    // X gives premultiplied ARGB; undo it for normal blending.
                    let un = |c: u32| (((c & 0xff) * 255 + a as u32 / 2) / a as u32).min(255) as u8;
                    Px::Rgba(un(argb >> 16), un(argb >> 8), un(argb), a)
                })
                .collect();
            let c = Cursor { w: img.width as usize, h: img.height as usize, px };
            self.cursor = Some((img.cursor_serial, c));
        }
        Some((img.x as i32 - img.xhot as i32, img.y as i32 - img.yhot as i32))
    }
}

impl Source for Screen {
    fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    fn capture(&mut self, frame: &mut RawFrame) -> Result<(), String> {
        let img = self
            .conn
            .get_image(ImageFormat::Z_PIXMAP, self.root, 0, 0, self.src_w as u16, self.src_h as u16, !0)
            .map_err(|e| format!("get_image: {e}"))?
            .reply()
            .map_err(|e| format!("get_image: {e}"))?;
        // 24/32-bit TrueColor on little-endian: B, G, R, X per pixel.
        let stride = self.src_w * 4;
        if img.data.len() < stride * self.src_h {
            return Err("short screen image".into());
        }
        bgra_to_i420(&img.data, self.src_w, self.src_h, stride, frame);
        if self.xfixes {
            if let Some((x, y)) = self.pointer() {
                if let Some((_, c)) = &self.cursor {
                    c.draw(frame, x, y, self.src_w, self.src_h);
                }
            }
        }
        Ok(())
    }
}
