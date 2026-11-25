//! Frame sources: the test pattern (anywhere) and the real screen (Windows,
//! DXGI Desktop Duplication).

/// One raw frame in I420 (Y plane, then U, then V, tightly packed).
pub struct RawFrame {
    pub width: usize,
    pub height: usize,
    pub i420: Vec<u8>,
}

impl RawFrame {
    pub fn new(width: usize, height: usize) -> Self {
        Self { width, height, i420: vec![0; width * height * 3 / 2] }
    }

    pub fn planes_mut(&mut self) -> (&mut [u8], &mut [u8], &mut [u8]) {
        let ys = self.width * self.height;
        let cs = ys / 4;
        let (y, rest) = self.i420.split_at_mut(ys);
        let (u, v) = rest.split_at_mut(cs);
        (y, u, v)
    }
}

pub trait Source: Send {
    fn size(&self) -> (usize, usize);
    /// Fill `frame` with the next picture. Called at the stream frame rate.
    fn capture(&mut self, frame: &mut RawFrame) -> Result<(), String>;
}

/// Pick a source by name. "screen" falls back to the test pattern when the
/// screen cannot be captured (no permission, Wayland, no display).
pub fn open(name: &str, width: usize, height: usize, fps: u32) -> Box<dyn Source> {
    let _ = fps;
    if name == "screen" {
        #[cfg(windows)]
        let screen = dxgi::Screen::new(width, height).map(|s| Box::new(s) as Box<dyn Source>);
        #[cfg(target_os = "macos")]
        let screen = mac::Screen::new(width, height, fps).map(|s| Box::new(s) as Box<dyn Source>);
        #[cfg(target_os = "linux")]
        let screen = x11::Screen::new(width, height).map(|s| Box::new(s) as Box<dyn Source>);
        #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
        let screen: Result<Box<dyn Source>, String> = Err("no screen capture on this system".into());
        match screen {
            Ok(s) => return s,
            Err(e) => crate::say!("screen capture unavailable: {e}. Showing a test pattern instead."),
        }
    }
    Box::new(TestPattern::new(width, height))
}

#[cfg(target_os = "macos")]
#[path = "capture_mac.rs"]
mod mac;
#[cfg(target_os = "linux")]
#[path = "capture_x11.rs"]
mod x11;

/// BGRA (as captured on every system) to I420, BT.709 limited range, with a
/// nearest-sample resize from `sw`x`sh` to the frame size.
pub fn bgra_to_i420(src: &[u8], sw: usize, sh: usize, pitch: usize, frame: &mut RawFrame) {
    let (w, h) = (frame.width, frame.height);
    let (y, u, v) = frame.planes_mut();
    let sx = |x: usize| x * sw / w;
    let sy = |yy: usize| yy * sh / h;
    let px = |x: usize, yy: usize| {
        let o = sy(yy) * pitch + sx(x) * 4;
        (src[o + 2] as i32, src[o + 1] as i32, src[o] as i32) // r, g, b
    };
    for row in 0..h {
        for col in 0..w {
            let (r, g, b) = px(col, row);
            y[row * w + col] = ((47 * r + 157 * g + 16 * b + 128) / 256 + 16) as u8;
        }
    }
    let cw = w / 2;
    for row in 0..h / 2 {
        for col in 0..cw {
            let (r, g, b) = px(col * 2, row * 2);
            u[row * cw + col] = ((-26 * r - 87 * g + 112 * b + 128) / 256 + 128).clamp(0, 255) as u8;
            v[row * cw + col] = ((112 * r - 102 * g - 10 * b + 128) / 256 + 128).clamp(0, 255) as u8;
        }
    }
}

/// The mouse pointer as drawn on the PC. Screen capture leaves it out of the
/// picture, so it is drawn into each frame before encoding.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
#[derive(Clone, Debug, PartialEq)]
pub struct Cursor {
    pub w: usize,
    pub h: usize,
    pub px: Vec<Px>,
}

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Px {
    Clear,
    /// r, g, b, alpha (0-255)
    Rgba(u8, u8, u8, u8),
    /// Inverts what is underneath (the I-beam text cursor does this).
    Invert,
}

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
impl Cursor {
    /// 1 bit per pixel: an AND mask (first half) above an XOR mask.
    pub fn monochrome(buf: &[u8], w: usize, h2: usize, pitch: usize) -> Option<Self> {
        let h = h2 / 2;
        let bit = |row: usize, col: usize| buf.get(row * pitch + col / 8).map(|b| (b >> (7 - col % 8)) & 1);
        let mut px = Vec::with_capacity(w * h);
        for row in 0..h {
            for col in 0..w {
                px.push(match (bit(row, col)?, bit(row + h, col)?) {
                    (0, 0) => Px::Rgba(0, 0, 0, 255),
                    (0, _) => Px::Rgba(255, 255, 255, 255),
                    (_, 0) => Px::Clear,
                    _ => Px::Invert,
                });
            }
        }
        Some(Self { w, h, px })
    }

    /// 32-bit BGRA. `masked`: alpha 0 means opaque colour, alpha 255 means
    /// XOR with the screen (black = no change); otherwise ordinary alpha.
    pub fn color(buf: &[u8], w: usize, h: usize, pitch: usize, masked: bool) -> Option<Self> {
        let mut px = Vec::with_capacity(w * h);
        for row in 0..h {
            for col in 0..w {
                let o = row * pitch + col * 4;
                let [b, g, r, a]: [u8; 4] = buf.get(o..o + 4)?.try_into().ok()?;
                px.push(match (masked, a) {
                    (true, 0) => Px::Rgba(r, g, b, 255),
                    (true, _) if (r, g, b) == (0, 0, 0) => Px::Clear,
                    (true, _) => Px::Invert,
                    (false, 0) => Px::Clear,
                    (false, a) => Px::Rgba(r, g, b, a),
                });
            }
        }
        Some(Self { w, h, px })
    }

    /// Draw with its top-left at (`x`, `y`) in screen pixels of a
    /// `sw`x`sh` screen, scaled to the frame like the picture.
    pub fn draw(&self, frame: &mut RawFrame, x: i32, y: i32, sw: usize, sh: usize) {
        let (fw, fh) = (frame.width as i64, frame.height as i64);
        let (sw, sh) = (sw.max(1) as i64, sh.max(1) as i64);
        let (x, y) = (x as i64, y as i64);
        let fx0 = (x * fw).div_euclid(sw).max(0);
        let fy0 = (y * fh).div_euclid(sh).max(0);
        let fx1 = ((x + self.w as i64) * fw).div_euclid(sw).min(fw);
        let fy1 = ((y + self.h as i64) * fh).div_euclid(sh).min(fh);
        let (width, cw) = (frame.width, frame.width / 2);
        let (yp, up, vp) = frame.planes_mut();
        for fy in fy0..fy1 {
            let cy = fy * sh / fh - y;
            if cy < 0 || cy >= self.h as i64 {
                continue;
            }
            for fx in fx0..fx1 {
                let cx = fx * sw / fw - x;
                if cx < 0 || cx >= self.w as i64 {
                    continue;
                }
                let yi = fy as usize * width + fx as usize;
                let ci = (fy as usize / 2) * cw + fx as usize / 2;
                let chroma = fx % 2 == 0 && fy % 2 == 0;
                match self.px[cy as usize * self.w + cx as usize] {
                    Px::Clear => {}
                    Px::Invert => {
                        yp[yi] = 251u8.saturating_sub(yp[yi]).clamp(16, 235);
                        if chroma {
                            up[ci] = 255 - up[ci];
                            vp[ci] = 255 - vp[ci];
                        }
                    }
                    Px::Rgba(r, g, b, a) => {
                        let (r, g, b, a) = (r as i32, g as i32, b as i32, a as i32);
                        let mix = |old: u8, new: i32| ((old as i32 * (255 - a) + new * a) / 255).clamp(0, 255) as u8;
                        yp[yi] = mix(yp[yi], (47 * r + 157 * g + 16 * b + 128) / 256 + 16);
                        if chroma {
                            up[ci] = mix(up[ci], (-26 * r - 87 * g + 112 * b + 128) / 256 + 128);
                            vp[ci] = mix(vp[ci], (112 * r - 102 * g - 10 * b + 128) / 256 + 128);
                        }
                    }
                }
            }
        }
    }
}

/// Moving test pattern: colour bars, a sweeping bar, and a grid.
pub struct TestPattern {
    width: usize,
    height: usize,
    n: u64,
}

impl TestPattern {
    pub fn new(width: usize, height: usize) -> Self {
        assert!(width % 2 == 0 && height % 2 == 0);
        Self { width, height, n: 0 }
    }
}

impl Source for TestPattern {
    fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    fn capture(&mut self, frame: &mut RawFrame) -> Result<(), String> {
        let (w, h) = (self.width, self.height);
        let n = self.n as usize;
        self.n += 1;
        let (y, u, v) = frame.planes_mut();
        let bar_x = (n * 8) % w;
        for row in 0..h {
            let line = &mut y[row * w..(row + 1) * w];
            for (col, px) in line.iter_mut().enumerate() {
                let grid = col % 120 == 0 || row % 120 == 0;
                let bar = col >= bar_x && col < bar_x + 40;
                *px = if bar { 235 } else if grid { 180 } else { (16 + ((col + row + n * 2) % 200)) as u8 };
            }
        }
        let cw = w / 2;
        const U: [u8; 8] = [128, 16, 166, 54, 202, 90, 240, 128];
        const V: [u8; 8] = [128, 146, 16, 34, 222, 240, 110, 128];
        for row in 0..h / 2 {
            for col in 0..cw {
                let band = col * 8 / cw;
                u[row * cw + col] = U[band];
                v[row * cw + col] = V[band];
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
mod dxgi {
    //! DXGI Desktop Duplication of the primary screen. The GPU copies the
    //! desktop into a CPU-readable texture, which is converted to I420.
    //! Phase 1b hands the GPU texture straight to NVENC instead (zero copy).

    use super::{bgra_to_i420, Cursor, RawFrame, Source};
    use windows::core::Interface;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
    use windows::Win32::Graphics::Dxgi::*;

    pub struct Screen {
        width: usize,
        height: usize,
        device: ID3D11Device,
        ctx: ID3D11DeviceContext,
        dup: Option<IDXGIOutputDuplication>,
        staging: Option<ID3D11Texture2D>,
        src_w: usize,
        src_h: usize,
        have_frame: bool,
        cursor: Option<Cursor>,
        cursor_at: (i32, i32),
        cursor_visible: bool,
        shape_buf: Vec<u8>,
    }

    // COM objects are used from the encode thread only.
    unsafe impl Send for Screen {}

    fn duplicate(device: &ID3D11Device) -> windows::core::Result<(IDXGIOutputDuplication, usize, usize)> {
        unsafe {
            let dxgi: IDXGIDevice = device.cast()?;
            let adapter = dxgi.GetAdapter()?;
            let output = adapter.EnumOutputs(0)?;
            let output1: IDXGIOutput1 = output.cast()?;
            let dup = output1.DuplicateOutput(device)?;
            let desc = dup.GetDesc();
            Ok((dup, desc.ModeDesc.Width as usize, desc.ModeDesc.Height as usize))
        }
    }

    impl Screen {
        pub fn new(width: usize, height: usize) -> Result<Self, String> {
            let mut device = None;
            let mut ctx = None;
            unsafe {
                D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_HARDWARE,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut ctx),
                )
                .map_err(|e| format!("D3D11CreateDevice: {e}"))?;
            }
            let (device, ctx) = (device.ok_or("no D3D11 device")?, ctx.ok_or("no D3D11 context")?);
            let (dup, src_w, src_h) = duplicate(&device).map_err(|e| format!("DuplicateOutput: {e}"))?;
            // Never upscale; keep the aspect ratio of the screen.
            let scale = (width as f64 / src_w as f64).min(height as f64 / src_h as f64).min(1.0);
            let w = ((src_w as f64 * scale) as usize) & !1;
            let h = ((src_h as f64 * scale) as usize) & !1;
            crate::say!("screen {src_w}x{src_h}, streaming at {w}x{h}");
            Ok(Self { width: w, height: h, device, ctx, dup: Some(dup), staging: None, src_w, src_h, have_frame: false,
                cursor: None, cursor_at: (0, 0), cursor_visible: false, shape_buf: Vec::new() })
        }

        fn read_cursor_shape(&mut self, dup: &IDXGIOutputDuplication, size: u32) {
            self.shape_buf.resize(size as usize, 0);
            let mut needed = 0u32;
            let mut si = DXGI_OUTDUPL_POINTER_SHAPE_INFO::default();
            let ok = unsafe {
                dup.GetFramePointerShape(size, self.shape_buf.as_mut_ptr().cast(), &mut needed, &mut si)
            };
            if ok.is_err() {
                return;
            }
            let (w, h, pitch) = (si.Width as usize, si.Height as usize, si.Pitch as usize);
            let buf = &self.shape_buf[..];
            let shape = match si.Type as i32 {
                t if t == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MONOCHROME.0 => Cursor::monochrome(buf, w, h, pitch),
                t if t == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_COLOR.0 => Cursor::color(buf, w, h, pitch, false),
                t if t == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MASKED_COLOR.0 => Cursor::color(buf, w, h, pitch, true),
                _ => None,
            };
            if shape.is_some() {
                self.cursor = shape;
            }
        }

        fn staging_for(&mut self, tex: &ID3D11Texture2D) -> windows::core::Result<ID3D11Texture2D> {
            if let Some(s) = &self.staging {
                return Ok(s.clone());
            }
            let mut d = D3D11_TEXTURE2D_DESC::default();
            unsafe { tex.GetDesc(&mut d) };
            d.Usage = D3D11_USAGE_STAGING;
            d.BindFlags = 0;
            d.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            d.MiscFlags = 0;
            d.MipLevels = 1;
            d.ArraySize = 1;
            d.SampleDesc = DXGI_SAMPLE_DESC { Count: 1, Quality: 0 };
            let mut s = None;
            unsafe { self.device.CreateTexture2D(&d, None, Some(&mut s))? };
            let s = s.expect("texture created");
            self.staging = Some(s.clone());
            Ok(s)
        }
    }

    impl Source for Screen {
        fn size(&self) -> (usize, usize) {
            (self.width, self.height)
        }

        fn capture(&mut self, frame: &mut RawFrame) -> Result<(), String> {
            if self.dup.is_none() {
                // Lost after a mode change, lock screen or full-screen game switch.
                let (dup, w, h) = duplicate(&self.device).map_err(|e| format!("re-duplicate: {e}"))?;
                if (w, h) != (self.src_w, self.src_h) {
                    return Err(format!("screen size changed to {w}x{h}; restart the session"));
                }
                self.dup = Some(dup);
                self.staging = None;
            }
            let dup = self.dup.clone().expect("set above");
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut res: Option<IDXGIResource> = None;
            match unsafe { dup.AcquireNextFrame(0, &mut info, &mut res) } {
                Ok(()) => {}
                // Nothing changed on screen: the last converted frame stays.
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT && self.have_frame => return Ok(()),
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(()),
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                    self.dup = None;
                    return Ok(());
                }
                Err(e) => return Err(format!("AcquireNextFrame: {e}")),
            }
            if info.LastMouseUpdateTime != 0 {
                self.cursor_visible = info.PointerPosition.Visible.as_bool();
                self.cursor_at = (info.PointerPosition.Position.x, info.PointerPosition.Position.y);
            }
            if info.PointerShapeBufferSize > 0 {
                self.read_cursor_shape(&dup, info.PointerShapeBufferSize);
            }
            let result = (|| -> windows::core::Result<()> {
                let res = res.ok_or_else(|| windows::core::Error::from_hresult(windows::Win32::Foundation::E_POINTER))?;
                let tex: ID3D11Texture2D = res.cast()?;
                let staging = self.staging_for(&tex)?;
                unsafe {
                    self.ctx.CopyResource(&staging, &tex);
                    let mut m = D3D11_MAPPED_SUBRESOURCE::default();
                    self.ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut m))?;
                    let len = m.RowPitch as usize * self.src_h;
                    let src = std::slice::from_raw_parts(m.pData as *const u8, len);
                    bgra_to_i420(src, self.src_w, self.src_h, m.RowPitch as usize, frame);
                    self.ctx.Unmap(&staging, 0);
                }
                Ok(())
            })();
            unsafe {
                let _ = dup.ReleaseFrame();
            }
            result.map_err(|e| format!("copy frame: {e}"))?;
            if let (true, Some(c)) = (self.cursor_visible, &self.cursor) {
                c.draw(frame, self.cursor_at.0, self.cursor_at.1, self.src_w, self.src_h);
            }
            self.have_frame = true;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monochrome_cursor_masks() {
        // 8x1 pointer: AND row then XOR row.
        // cols 0-1: AND 0 XOR 0 = black; 2-3: AND 0 XOR 1 = white;
        // 4-5: AND 1 XOR 0 = clear; 6-7: AND 1 XOR 1 = invert.
        let c = Cursor::monochrome(&[0b0000_1111, 0b0011_0011], 8, 2, 1).unwrap();
        assert_eq!((c.w, c.h), (8, 1));
        assert_eq!(c.px[0], Px::Rgba(0, 0, 0, 255));
        assert_eq!(c.px[2], Px::Rgba(255, 255, 255, 255));
        assert_eq!(c.px[4], Px::Clear);
        assert_eq!(c.px[6], Px::Invert);
        assert!(Cursor::monochrome(&[0], 8, 2, 1).is_none(), "short buffer");
    }

    #[test]
    fn color_cursor_draws_scaled_and_clipped() {
        // 2x2 opaque white pointer (BGRA), drawn on a black 4x4 frame that
        // shows an 8x8 screen: it covers one frame pixel.
        let c = Cursor::color(&[255u8; 16], 2, 2, 8, false).unwrap();
        let mut f = RawFrame::new(4, 4);
        f.planes_mut().0.fill(16);
        c.draw(&mut f, 2, 2, 8, 8);
        let y = f.planes_mut().0.to_vec();
        assert_eq!(y[4 + 1], 235, "white at frame (1,1)");
        assert_eq!(y.iter().filter(|&&v| v == 235).count(), 1);
        // Half off the top-left edge: no panic, the visible part drawn.
        c.draw(&mut f, -1, -1, 8, 8);
        // Masked colour: alpha 255 + black = clear.
        let m = Cursor::color(&[0, 0, 0, 255], 1, 1, 4, true).unwrap();
        assert_eq!(m.px[0], Px::Clear);
    }

    #[test]
    fn bgra_white_black_and_scale() {
        let (sw, sh) = (8, 4);
        let mut src = vec![0u8; sw * sh * 4];
        for row in 0..sh {
            for col in 0..sw / 2 {
                src[(row * sw + col) * 4..(row * sw + col) * 4 + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        let mut f = RawFrame::new(4, 2); // half size
        bgra_to_i420(&src, sw, sh, sw * 4, &mut f);
        let (y, u, v) = f.planes_mut();
        assert_eq!((y[0], y[3]), (235, 16), "white left, black right");
        assert!((126..=130).contains(&u[0]) && (126..=130).contains(&v[0]), "grey has neutral chroma");
    }
}
