//! H.264 encoder. Today: OpenH264 (software, portable). Phase 1b adds NVENC /
//! AMF / Quick Sync behind the same type, with the low-latency settings from
//! docs/protocol-analysis.md and in-place bitrate changes.

pub struct Encoded {
    /// Annex B byte stream (start codes + NAL units).
    pub data: Vec<u8>,
    pub keyframe: bool,
}

#[cfg(feature = "openh264")]
mod imp {
    use super::Encoded;
    use openh264::encoder::{
        BitRate, Complexity, Encoder, EncoderConfig, FrameRate, FrameType, IntraFramePeriod, RateControlMode,
        UsageType,
    };
    use openh264::formats::YUVSlices;
    use openh264::OpenH264API;

    pub struct H264Encoder {
        enc: Encoder,
        width: usize,
        height: usize,
        fps: u32,
        bitrate_bps: u32,
    }

    fn build(fps: u32, bitrate_bps: u32) -> Result<Encoder, String> {
        let config = EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .bitrate(BitRate::from_bps(bitrate_bps))
            .max_frame_rate(FrameRate::from_hz(fps as f32))
            .rate_control_mode(RateControlMode::Bitrate)
            .skip_frames(false)
            .complexity(Complexity::Low)
            // Slices encoded in parallel; one frame in, one frame out.
            .num_threads(4)
            // No periodic keyframes: only the first one and on request (PLI).
            .intra_frame_period(IntraFramePeriod::from_num_frames(0));
        Encoder::with_api_config(OpenH264API::from_source(), config).map_err(|e| format!("openh264 init: {e}"))
    }

    impl H264Encoder {
        pub fn new(width: usize, height: usize, fps: u32, bitrate_bps: u32) -> Result<Self, String> {
            Ok(Self { enc: build(fps, bitrate_bps)?, width, height, fps, bitrate_bps })
        }

        pub fn name(&self) -> &'static str {
            "OpenH264 (software)"
        }

        pub fn bitrate(&self) -> u32 {
            self.bitrate_bps
        }

        /// OpenH264's safe API cannot change the rate in place, so the encoder
        /// is rebuilt; the next frame is a keyframe. The caller limits how often.
        pub fn set_bitrate(&mut self, bps: u32) -> Result<(), String> {
            self.enc = build(self.fps, bps)?;
            self.bitrate_bps = bps;
            Ok(())
        }

        pub fn encode(&mut self, i420: &[u8], force_keyframe: bool) -> Result<Encoded, String> {
            let (w, h) = (self.width, self.height);
            let ys = w * h;
            let cs = ys / 4;
            let yuv = YUVSlices::new(
                (&i420[..ys], &i420[ys..ys + cs], &i420[ys + cs..ys + 2 * cs]),
                (w, h),
                (w, w / 2, w / 2),
            );
            if force_keyframe {
                self.enc.force_intra_frame();
            }
            let bs = self.enc.encode(&yuv).map_err(|e| format!("encode: {e}"))?;
            let keyframe = matches!(bs.frame_type(), FrameType::IDR | FrameType::I);
            Ok(Encoded { data: bs.to_vec(), keyframe })
        }
    }
}

#[cfg(not(feature = "openh264"))]
mod imp {
    use super::Encoded;

    /// Built without an encoder (used to compile-check other targets).
    pub struct H264Encoder;

    impl H264Encoder {
        pub fn new(_w: usize, _h: usize, _fps: u32, _bps: u32) -> Result<Self, String> {
            Err("built without an H.264 encoder (enable the `openh264` feature)".into())
        }
        pub fn name(&self) -> &'static str {
            "none"
        }
        pub fn bitrate(&self) -> u32 {
            0
        }
        pub fn set_bitrate(&mut self, _bps: u32) -> Result<(), String> {
            Ok(())
        }
        pub fn encode(&mut self, _i420: &[u8], _k: bool) -> Result<Encoded, String> {
            Err("no encoder".into())
        }
    }
}

pub use imp::H264Encoder;

/// Bitrate controller: follows the congestion estimate with hysteresis, so a
/// rebuild (and its keyframe) happens only for real changes.
pub struct RateControl {
    pub max_bps: u32,
    pub min_bps: u32,
    last_change: Option<std::time::Instant>,
}

impl RateControl {
    pub fn new(min_bps: u32, max_bps: u32) -> Self {
        Self { max_bps, min_bps, last_change: None }
    }

    /// Target for a given estimate, or None to keep the current rate.
    pub fn decide(&mut self, current: u32, estimate: u64, now: std::time::Instant) -> Option<u32> {
        // Leave 15 % headroom for audio, FEC and resends.
        let target = ((estimate as f64 * 0.85) as u64).clamp(self.min_bps as u64, self.max_bps as u64) as u32;
        let ratio = target as f64 / current.max(1) as f64;
        let big = !(0.8..=1.25).contains(&ratio);
        // Drops act fast (congestion); rises wait, to avoid see-sawing.
        let wait = if ratio < 1.0 { 1.0 } else { 4.0 };
        let waited = self.last_change.map_or(true, |t| now.duration_since(t).as_secs_f64() >= wait);
        if big && waited {
            self.last_change = Some(now);
            Some(target)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn rate_control_hysteresis() {
        let mut rc = RateControl::new(1_000_000, 20_000_000);
        let t = Instant::now();
        assert_eq!(rc.decide(8_000_000, 8_500_000, t), None, "small change ignored");
        assert_eq!(rc.decide(8_000_000, 4_000_000, t), Some(3_400_000), "drop at once");
        assert_eq!(rc.decide(3_400_000, 30_000_000, t + Duration::from_secs(2)), None, "rise waits");
        assert_eq!(rc.decide(3_400_000, 30_000_000, t + Duration::from_secs(5)), Some(20_000_000), "capped at max");
        assert_eq!(rc.decide(20_000_000, 100, t + Duration::from_secs(9)), Some(1_000_000), "floored at min");
    }
}
