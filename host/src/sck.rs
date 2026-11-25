//! A small wrapper over Apple's ScreenCaptureKit (macOS 12.3+), called
//! directly through the objc2 bindings: no Swift toolchain needed to build.
//! Used for the screen (capture_mac.rs) and the system sound (audio.rs).

use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamOutput, SCStreamOutputType,
};

pub use objc2_screen_capture_kit::SCStreamOutputType as Kind;

type Callback = Box<dyn Fn(&CMSampleBuffer, SCStreamOutputType) + Send + Sync>;

pub struct OutputIvars {
    callback: Callback,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; Output has no Drop.
    #[unsafe(super(NSObject))]
    #[name = "EverywhereStreamOutput"]
    #[ivars = OutputIvars]
    struct Output;

    unsafe impl NSObjectProtocol for Output {}

    unsafe impl SCStreamOutput for Output {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn did_output(&self, _stream: &SCStream, sample: &CMSampleBuffer, kind: SCStreamOutputType) {
            (self.ivars().callback)(sample, kind);
        }
    }
);

impl Output {
    fn new(callback: Callback) -> Retained<Self> {
        let this = Self::alloc().set_ivars(OutputIvars { callback });
        unsafe { msg_send![super(this), init] }
    }
}

/// A running capture; stops when dropped.
pub struct Stream {
    stream: Retained<SCStream>,
    _output: Retained<Output>,
    _queue: DispatchRetained<DispatchQueue>,
}

// The ObjC objects are thread-safe to stop from another thread.
unsafe impl Send for Stream {}

impl Drop for Stream {
    fn drop(&mut self) {
        unsafe { self.stream.stopCaptureWithCompletionHandler(None) };
    }
}

struct SendPtr<T>(Option<Retained<T>>);
unsafe impl<T> Send for SendPtr<T> {}

fn describe(err: *mut NSError) -> String {
    match unsafe { err.as_ref() } {
        Some(e) => e.localizedDescription().to_string(),
        None => "unknown error".into(),
    }
}

const NEED_PERMISSION: &str =
    "allow Everywhere in System Settings > Privacy & Security > Screen & System Audio Recording, then restart it";

/// The main display and its size in points.
pub fn main_display() -> Result<(Retained<SCDisplay>, usize, usize), String> {
    let (tx, rx) = mpsc::channel::<Result<SendPtr<SCShareableContent>, String>>();
    let block = RcBlock::new(move |content: *mut SCShareableContent, err: *mut NSError| {
        let r = match unsafe { Retained::retain(content) } {
            Some(c) => Ok(SendPtr(Some(c))),
            None => Err(describe(err)),
        };
        let _ = tx.send(r);
    });
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&block) };
    let content = rx
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| format!("no answer from ScreenCaptureKit; {NEED_PERMISSION}"))?
        .map_err(|e| format!("{e}; {NEED_PERMISSION}"))?
        .0
        .ok_or("no shareable content")?;
    let displays = unsafe { content.displays() };
    let display = displays.firstObject().ok_or("no display found")?;
    let (w, h) = unsafe { (display.width() as usize, display.height() as usize) };
    Ok((display, w, h))
}

pub struct Config {
    pub width: usize,
    pub height: usize,
    pub fps: u32,
    pub audio: bool,
}

/// Start capturing `display`; `callback` runs on a private queue for every
/// screen frame (and audio buffer when `audio`).
pub fn start(
    display: &SCDisplay,
    cfg: Config,
    callback: impl Fn(&CMSampleBuffer, SCStreamOutputType) + Send + Sync + 'static,
) -> Result<Stream, String> {
    unsafe {
        let filter = SCContentFilter::initWithDisplay_excludingWindows(
            SCContentFilter::alloc(),
            display,
            &NSArray::new(),
        );
        let config = SCStreamConfiguration::new();
        config.setWidth(cfg.width);
        config.setHeight(cfg.height);
        config.setPixelFormat(objc2_core_video::kCVPixelFormatType_32BGRA);
        config.setShowsCursor(true);
        config.setQueueDepth(3);
        config.setMinimumFrameInterval(CMTime::new(1, cfg.fps.max(1) as i32));
        if cfg.audio {
            config.setCapturesAudio(true);
            config.setExcludesCurrentProcessAudio(true);
            config.setSampleRate(48_000);
            config.setChannelCount(2);
        }
        let stream = SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), &filter, &config, None);
        let output = Output::new(Box::new(callback));
        let queue = DispatchQueue::new("everywhere.capture", DispatchQueueAttr::SERIAL);
        let proto = ProtocolObject::from_ref(&*output);
        stream
            .addStreamOutput_type_sampleHandlerQueue_error(proto, SCStreamOutputType::Screen, Some(&queue))
            .map_err(|e| e.localizedDescription().to_string())?;
        if cfg.audio {
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(proto, SCStreamOutputType::Audio, Some(&queue))
                .map_err(|e| e.localizedDescription().to_string())?;
        }
        let (tx, rx) = mpsc::channel::<Option<String>>();
        let done = RcBlock::new(move |err: *mut NSError| {
            let _ = tx.send(if err.is_null() { None } else { Some(describe(err)) });
        });
        stream.startCaptureWithCompletionHandler(Some(&done));
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(None) => {}
            Ok(Some(e)) => return Err(format!("{e}; {NEED_PERMISSION}")),
            Err(_) => return Err(format!("capture did not start; {NEED_PERMISSION}")),
        }
        Ok(Stream { stream, _output: output, _queue: queue })
    }
}
