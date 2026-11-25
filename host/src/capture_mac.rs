//! macOS screen capture with ScreenCaptureKit (see sck.rs). The system
//! scales the picture to the stream size and draws the mouse pointer into it.
//!
//! Needs the Screen Recording permission. The first time, macOS asks; after
//! allowing it, Everywhere must be restarted.

use std::sync::{Arc, Mutex};

use objc2_core_video::{
    CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight, CVPixelBufferGetWidth,
    CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
};

use super::{bgra_to_i420, RawFrame, Source};
use crate::sck;

/// The newest frame from the capture queue.
#[derive(Default)]
struct Latest {
    bgra: Vec<u8>,
    width: usize,
    height: usize,
    stride: usize,
    seq: u64,
}

pub struct Screen {
    width: usize,
    height: usize,
    latest: Arc<Mutex<Latest>>,
    seen: u64,
    _stream: sck::Stream,
}

/// Fit the display (in points) into `max_w`x`max_h`, keeping the aspect
/// ratio. Retina screens have twice the pixels of their points, so up to 2x
/// the point size is real detail, not upscaling.
pub fn fit(points_w: usize, points_h: usize, max_w: usize, max_h: usize) -> (usize, usize) {
    let (pw, ph) = (points_w.max(1) as f64 * 2.0, points_h.max(1) as f64 * 2.0);
    let scale = (max_w as f64 / pw).min(max_h as f64 / ph).min(1.0);
    (((pw * scale) as usize) & !1, ((ph * scale) as usize) & !1)
}

impl Screen {
    pub fn new(max_w: usize, max_h: usize, fps: u32) -> Result<Self, String> {
        let (display, pw, ph) = sck::main_display()?;
        let (w, h) = fit(pw, ph, max_w, max_h);
        let latest = Arc::new(Mutex::new(Latest::default()));
        let sink = latest.clone();
        let stream = sck::start(&display, sck::Config { width: w, height: h, fps, audio: false }, move |sample, kind| {
            if kind != sck::Kind::Screen {
                return;
            }
            // Idle frames (nothing changed) carry no picture.
            let Some(pb) = (unsafe { sample.image_buffer() }) else { return };
            unsafe {
                if CVPixelBufferLockBaseAddress(&pb, CVPixelBufferLockFlags::ReadOnly) != 0 {
                    return;
                }
                let base = CVPixelBufferGetBaseAddress(&pb) as *const u8;
                let (width, height, stride) =
                    (CVPixelBufferGetWidth(&pb), CVPixelBufferGetHeight(&pb), CVPixelBufferGetBytesPerRow(&pb));
                if !base.is_null() {
                    let src = std::slice::from_raw_parts(base, stride * height);
                    if let Ok(mut l) = sink.lock() {
                        l.bgra.clear();
                        l.bgra.extend_from_slice(src);
                        (l.width, l.height, l.stride) = (width, height, stride);
                        l.seq += 1;
                    }
                }
                CVPixelBufferUnlockBaseAddress(&pb, CVPixelBufferLockFlags::ReadOnly);
            }
        })?;
        crate::say!("screen {pw}x{ph} points, streaming at {w}x{h}");
        Ok(Self { width: w, height: h, latest, seen: 0, _stream: stream })
    }
}

impl Source for Screen {
    fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    fn capture(&mut self, frame: &mut RawFrame) -> Result<(), String> {
        let l = self.latest.lock().map_err(|_| "capture lock")?;
        // Nothing new on screen: the last converted frame stays.
        if l.seq == self.seen || l.width == 0 || l.bgra.len() < l.stride * l.height {
            return Ok(());
        }
        self.seen = l.seq;
        bgra_to_i420(&l.bgra, l.width, l.height, l.stride, frame);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fits_retina_displays() {
        // MacBook Air 13" (1470x956 points, 2940x1912 pixels) into 1080p.
        assert_eq!(super::fit(1470, 956, 1920, 1080), (1660, 1080));
        // A 1080p display at 1x (960x540 "points" when scaled) stays 1920x1080.
        assert_eq!(super::fit(960, 540, 1920, 1080), (1920, 1080));
    }
}
