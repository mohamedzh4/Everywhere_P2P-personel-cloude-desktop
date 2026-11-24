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

/// Pick a source by name. "screen" falls back to the pattern where screen
/// capture is not built (non-Windows) or fails to start.
pub fn open(name: &str, width: usize, height: usize) -> Box<dyn Source> {
    if name == "screen" {
        #[cfg(windows)]
        match dxgi::Screen::new(width, height) {
            Ok(s) => return Box::new(s),
            Err(e) => eprintln!("screen capture unavailable ({e}); using the test pattern"),
        }
        #[cfg(not(windows))]
        eprintln!("screen capture is Windows-only for now; using the test pattern");
    }
    Box::new(TestPattern::new(width, height))
}

/// BGRA (as captured on Windows) to I420, BT.709 limited range, with a
/// nearest-sample resize from `sw`x`sh` to the frame size.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
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

    use super::{bgra_to_i420, RawFrame, Source};
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
            println!("screen {src_w}x{src_h}, streaming at {w}x{h}");
            Ok(Self { width: w, height: h, device, ctx, dup: Some(dup), staging: None, src_w, src_h, have_frame: false })
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
            self.have_frame = true;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
