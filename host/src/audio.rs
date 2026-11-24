//! System audio: whatever the PC is playing, captured with WASAPI loopback
//! (Windows), encoded as Opus (low delay, 10 ms frames, 48 kHz stereo).
//!
//! Off Windows, or built without the `audio` feature, there is no audio and
//! the stream is video only.

use std::sync::atomic::AtomicBool;
#[cfg(all(feature = "audio", windows))]
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::Arc;

pub struct AudioPacket {
    pub data: Vec<u8>,
    /// RTP time at 48 kHz.
    pub rtp_48k: u64,
}

#[cfg_attr(not(all(feature = "audio", windows)), allow(dead_code))]
pub const FRAME_SAMPLES: usize = 480; // 10 ms at 48 kHz

/// Start capturing. Returns a guard; dropping it (or setting `stop`) ends
/// capture. Err when no audio is available here.
#[cfg(all(feature = "audio", windows))]
pub fn start(tx: Sender<AudioPacket>, stop: Arc<AtomicBool>) -> Result<Box<dyn std::any::Any>, String> {
    use audiopus::coder::Encoder;
    use audiopus::{Application, Channels, SampleRate};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

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
    let enc = Encoder::new(SampleRate::Hz48000, Channels::Stereo, Application::LowDelay).map_err(|e| e.to_string())?;
    let mut pending: Vec<f32> = Vec::with_capacity(FRAME_SAMPLES * 4);
    let mut out = vec![0u8; 1500];
    let mut rtp = 0u64;
    // WASAPI loopback: an *input* stream opened on the *output* device.
    let stream = device
        .build_input_stream::<f32, _, _>(
            cfg.config(),
            move |data: &[f32], _| {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                // Keep the first two channels (stereo).
                for f in data.chunks_exact(channels) {
                    pending.push(f[0]);
                    pending.push(*f.get(1).unwrap_or(&f[0]));
                }
                while pending.len() >= FRAME_SAMPLES * 2 {
                    if let Ok(n) = enc.encode_float(&pending[..FRAME_SAMPLES * 2], &mut out) {
                        let _ = tx.send(AudioPacket { data: out[..n].to_vec(), rtp_48k: rtp });
                    }
                    pending.drain(..FRAME_SAMPLES * 2);
                    rtp += FRAME_SAMPLES as u64;
                }
            },
            |e| eprintln!("audio: {e}"),
            None,
        )
        .map_err(|e| format!("loopback capture: {e}"))?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(Box::new(stream))
}

#[cfg(not(all(feature = "audio", windows)))]
pub fn start(_tx: Sender<AudioPacket>, _stop: Arc<AtomicBool>) -> Result<Box<dyn std::any::Any>, String> {
    Err("system audio capture is Windows-only".into())
}
