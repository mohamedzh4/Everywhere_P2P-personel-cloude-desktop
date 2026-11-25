//! System audio: whatever the PC is playing, encoded as Opus (low delay,
//! 10 ms frames, 48 kHz stereo).
//!
//! - Windows: WASAPI loopback (cpal).
//! - macOS 13+: ScreenCaptureKit audio (Screen Recording permission).
//! - Linux: the default output's "monitor" source, read with `parec`
//!   (PulseAudio, or PipeWire with pipewire-pulse, which desktops ship).
//!
//! Built without the `audio` feature, the stream is video only.

use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::sync::Arc;

pub struct AudioPacket {
    pub data: Vec<u8>,
    /// RTP time at 48 kHz.
    pub rtp_48k: u64,
}

#[cfg_attr(not(feature = "audio"), allow(dead_code))]
pub const FRAME_SAMPLES: usize = 480; // 10 ms at 48 kHz

/// Turns interleaved stereo f32 at 48 kHz into Opus packets.
#[cfg(feature = "audio")]
struct Packer {
    enc: audiopus::coder::Encoder,
    pending: Vec<f32>,
    out: Vec<u8>,
    rtp: u64,
    tx: Sender<AudioPacket>,
}

#[cfg(feature = "audio")]
impl Packer {
    fn new(tx: Sender<AudioPacket>) -> Result<Self, String> {
        use audiopus::{Application, Channels, SampleRate};
        let enc = audiopus::coder::Encoder::new(SampleRate::Hz48000, Channels::Stereo, Application::LowDelay)
            .map_err(|e| e.to_string())?;
        Ok(Self { enc, pending: Vec::with_capacity(FRAME_SAMPLES * 4), out: vec![0; 1500], rtp: 0, tx })
    }

    /// Returns false once the session is gone.
    fn push(&mut self, stereo: impl IntoIterator<Item = f32>) -> bool {
        self.pending.extend(stereo);
        while self.pending.len() >= FRAME_SAMPLES * 2 {
            if let Ok(n) = self.enc.encode_float(&self.pending[..FRAME_SAMPLES * 2], &mut self.out) {
                if self.tx.send(AudioPacket { data: self.out[..n].to_vec(), rtp_48k: self.rtp }).is_err() {
                    return false;
                }
            }
            self.pending.drain(..FRAME_SAMPLES * 2);
            self.rtp += FRAME_SAMPLES as u64;
        }
        true
    }
}

/// Start capturing. Returns a guard; dropping it (or setting `stop`) ends
/// capture. Err when no audio is available here.
pub fn start(tx: Sender<AudioPacket>, stop: Arc<AtomicBool>) -> Result<Box<dyn std::any::Any>, String> {
    #[cfg(all(feature = "audio", windows))]
    return windows_loopback(tx, stop);
    #[cfg(all(feature = "audio", target_os = "macos"))]
    return mac::start(tx, stop);
    #[cfg(all(feature = "audio", target_os = "linux"))]
    return linux_monitor(tx, stop);
    #[allow(unreachable_code)]
    {
        let _ = (tx, stop);
        Err("no system audio capture in this build".into())
    }
}

#[cfg(all(feature = "audio", windows))]
fn windows_loopback(tx: Sender<AudioPacket>, stop: Arc<AtomicBool>) -> Result<Box<dyn std::any::Any>, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::atomic::Ordering;

    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no audio output device")?;
    let cfg = device.default_output_config().map_err(|e| e.to_string())?;
    let rate = cfg.sample_rate();
    let channels = cfg.channels() as usize;
    if rate != 48_000 {
        return Err(format!("output runs at {rate} Hz; set it to 48000 Hz in Windows sound settings for audio"));
    }
    if cfg.sample_format() != cpal::SampleFormat::F32 {
        return Err(format!("unsupported sample format {:?}", cfg.sample_format()));
    }
    let mut packer = Packer::new(tx)?;
    // WASAPI loopback: an *input* stream opened on the *output* device.
    let stream = device
        .build_input_stream::<f32, _, _>(
            cfg.config(),
            move |data: &[f32], _| {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                // Keep the first two channels (stereo).
                packer.push(data.chunks_exact(channels).flat_map(|f| [f[0], *f.get(1).unwrap_or(&f[0])]));
            },
            |e| crate::say!("audio: {e}"),
            None,
        )
        .map_err(|e| format!("loopback capture: {e}"))?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(Box::new(stream))
}

#[cfg(all(feature = "audio", target_os = "macos"))]
mod mac {
    use super::{AudioPacket, Packer};
    use crate::sck;
    use objc2_core_audio_types::{AudioBuffer, AudioBufferList};
    use objc2_core_media::{CMBlockBuffer, CMSampleBuffer};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::Sender;
    use std::sync::{Arc, Mutex};

    /// The PCM in one audio sample buffer as interleaved stereo f32.
    /// ScreenCaptureKit delivers 32-bit float, usually one buffer per channel.
    fn stereo(sample: &CMSampleBuffer) -> Option<Vec<f32>> {
        // Room for an AudioBufferList with up to 4 buffers, 16-byte aligned.
        let mut storage = [0u128; 5];
        let list = storage.as_mut_ptr() as *mut AudioBufferList;
        let mut block: *mut CMBlockBuffer = std::ptr::null_mut();
        let status = unsafe {
            sample.audio_buffer_list_with_retained_block_buffer(
                std::ptr::null_mut(),
                list,
                std::mem::size_of_val(&storage),
                None,
                None,
                1, // kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment
                &mut block,
            )
        };
        // The block buffer owns the samples; release it when done.
        let _owner = std::ptr::NonNull::new(block).map(|p| unsafe { objc2_core_foundation::CFRetained::from_raw(p) });
        if status != 0 {
            return None;
        }
        let n = unsafe { (*list).mNumberBuffers as usize }.min(4);
        let first = unsafe { std::ptr::addr_of!((*list).mBuffers) as *const AudioBuffer };
        let chan = |i: usize| -> Vec<f32> {
            let b = unsafe { &*first.add(i) };
            if b.mData.is_null() {
                return Vec::new();
            }
            let len = b.mDataByteSize as usize / 4;
            unsafe { std::slice::from_raw_parts(b.mData as *const f32, len) }.to_vec()
        };
        let b0 = unsafe { &*first };
        Some(match (n, b0.mNumberChannels) {
            (0, _) => return None,
            (1, 1) => chan(0).into_iter().flat_map(|m| [m, m]).collect(),
            (1, c) => {
                let c = c.max(1) as usize;
                chan(0).chunks_exact(c).flat_map(|f| [f[0], *f.get(1).unwrap_or(&f[0])]).collect()
            }
            _ => chan(0).into_iter().zip(chan(1)).flat_map(|(l, r)| [l, r]).collect(),
        })
    }

    /// Its own small stream (the 2x2 picture is ignored), so audio starts and
    /// stops with the session independently of the video capture.
    pub fn start(tx: Sender<AudioPacket>, stop: Arc<AtomicBool>) -> Result<Box<dyn std::any::Any>, String> {
        let (display, _, _) = sck::main_display()?;
        let packer = Mutex::new(Packer::new(tx)?);
        let stream = sck::start(&display, sck::Config { width: 2, height: 2, fps: 1, audio: true }, move |sample, kind| {
            if kind != sck::Kind::Audio || stop.load(Ordering::Relaxed) {
                return;
            }
            if let (Some(pcm), Ok(mut p)) = (stereo(sample), packer.lock()) {
                p.push(pcm);
            }
        })?;
        Ok(Box::new(stream))
    }
}

#[cfg(all(feature = "audio", target_os = "linux"))]
fn linux_monitor(tx: Sender<AudioPacket>, stop: Arc<AtomicBool>) -> Result<Box<dyn std::any::Any>, String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::sync::atomic::Ordering;

    /// Ends `parec` when the session ends.
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let mut child = Command::new("parec")
        .args([
            "--device=@DEFAULT_MONITOR@",
            "--format=float32le",
            "--rate=48000",
            "--channels=2",
            "--latency-msec=10",
            "--raw",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start parec ({e}); install pulseaudio-utils for sound"))?;
    let mut out = child.stdout.take().ok_or("parec has no output")?;
    let mut packer = Packer::new(tx)?;
    std::thread::Builder::new()
        .name("audio".into())
        .spawn(move || {
            let mut buf = vec![0u8; FRAME_SAMPLES * 8];
            let mut carry: Vec<u8> = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                let Ok(n) = out.read(&mut buf) else { return };
                if n == 0 {
                    return;
                }
                carry.extend_from_slice(&buf[..n]);
                let whole = carry.len() / 4 * 4;
                let samples: Vec<f32> =
                    carry[..whole].chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
                carry.drain(..whole);
                if !packer.push(samples) {
                    return;
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(Box::new(Child(child)))
}
