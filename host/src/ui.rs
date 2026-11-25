//! The app window: the status page in a native window, so double-clicking
//! Everywhere.exe opens a normal app instead of a terminal. Windows uses
//! WebView2 (part of Windows 10 and 11), macOS uses WKWebView. Where no
//! window can be made, the page opens in the default browser instead.

/// Open `url` in the default browser. Only http(s) links are passed on.
pub fn open_in_browser(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://127.0.0.1")) {
        return;
    }
    #[cfg(windows)]
    let r = std::process::Command::new("explorer").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(e) = r {
        crate::say!("could not open the browser: {e}");
    }
}

/// The logo as raw 64x64 RGBA (assets/make-icons.sh), for the title bar
/// and taskbar. The Mac app takes its icon from the bundle instead.
#[cfg(any(windows, target_os = "macos"))]
const ICON_64: &[u8] = include_bytes!("../../assets/icons/window-64.rgba");

/// Show the window; returns when it is closed. Err if no window can be
/// made (then the caller keeps running without one).
#[cfg(any(windows, target_os = "macos"))]
pub fn run_window(port: u16) -> Result<(), String> {
    use tao::dpi::LogicalSize;
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoop};
    use tao::window::WindowBuilder;
    use wry::{NewWindowResponse, WebContext, WebViewBuilder};

    let home = format!("http://127.0.0.1:{port}/");
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Everywhere")
        .with_inner_size(LogicalSize::new(440.0, 760.0))
        .with_min_inner_size(LogicalSize::new(360.0, 520.0))
        .with_window_icon(tao::window::Icon::from_rgba(ICON_64.to_vec(), 64, 64).ok())
        .build(&event_loop)
        .map_err(|e| format!("window: {e}"))?;
    // WebView2 keeps its cache here instead of next to the exe.
    let mut ctx = WebContext::new(Some(crate::config::path().with_file_name("webview")));
    let allowed = home.clone();
    let webview = WebViewBuilder::new_with_web_context(&mut ctx)
        .with_url(format!("{home}?app=1"))
        .with_clipboard(true)
        // The window only ever shows the local page; links go to the browser.
        .with_navigation_handler(move |url| url.starts_with(&allowed))
        .with_new_window_req_handler(|url, _| {
            open_in_browser(&url);
            NewWindowResponse::Deny
        })
        .build(&window)
        .map_err(|e| format!("web view: {e}"))?;
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        let _keep = &webview;
        if let Event::WindowEvent { event: WindowEvent::CloseRequested, .. } = event {
            // Closing the window stops Everywhere (the page says so).
            std::process::exit(0);
        }
    })
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn run_window(_port: u16) -> Result<(), String> {
    Err("no app window on this system".into())
}
