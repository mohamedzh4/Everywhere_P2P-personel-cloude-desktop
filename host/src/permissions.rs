//! What this PC must allow before it can be used from elsewhere, checked at
//! start (while the owner is sitting at it, not when they are away):
//!
//! - macOS: Screen Recording (picture and sound) and Accessibility (mouse and
//!   keyboard). Asked for at start; the window links to the right settings.
//! - Linux: an X11 session (Wayland is not supported yet) and `parec` for sound.
//! - Windows: nothing to allow.

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Check {
    /// Short name shown in the window.
    pub name: String,
    pub ok: bool,
    /// What it means, or what to do.
    pub detail: String,
    /// Settings page the window's button opens (see `open_settings`).
    pub fix: Option<String>,
}

#[cfg_attr(windows, allow(dead_code))]
fn check(name: &str, ok: bool, detail: &str, fix: Option<&str>) -> Check {
    Check { name: name.into(), ok, detail: detail.into(), fix: fix.map(Into::into) }
}

/// Check everything; on macOS `ask` also shows the system prompts (not when
/// EVERYWHERE_NO_PROMPT is set, for test runs from a terminal or an IDE).
pub fn check_all(ask: bool) -> Vec<Check> {
    let ask = ask && std::env::var_os("EVERYWHERE_NO_PROMPT").is_none();
    #[cfg(target_os = "macos")]
    return mac::check_all(ask);
    #[cfg(target_os = "linux")]
    return linux_checks();
    #[allow(unreachable_code)]
    {
        let _ = ask;
        Vec::new()
    }
}

#[cfg(target_os = "linux")]
fn linux_checks() -> Vec<Check> {
    let wayland = std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t.eq_ignore_ascii_case("wayland"));
    let x11 = std::env::var_os("DISPLAY").is_some();
    let parec = std::process::Command::new("parec").arg("--version").output().is_ok_and(|o| o.status.success());
    vec![
        if wayland {
            check(
                "Screen",
                false,
                "Wayland session: not supported yet. Log out and choose an Xorg session (e.g. \"Ubuntu on Xorg\").",
                None,
            )
        } else if x11 {
            check("Screen", true, "X11 session", None)
        } else {
            check("Screen", false, "No display found (DISPLAY is not set).", None)
        },
        if parec {
            check("Sound", true, "PulseAudio/PipeWire monitor", None)
        } else {
            check("Sound", false, "Install pulseaudio-utils (parec) for sound.", None)
        },
    ]
}

/// Open the settings page for a failed check.
pub fn open_settings(which: &str) {
    #[cfg(target_os = "macos")]
    {
        let anchor = match which {
            "screen" => "Privacy_ScreenCapture",
            "accessibility" => "Privacy_Accessibility",
            _ => return,
        };
        let url = format!("x-apple.systempreferences:com.apple.preference.security?{anchor}");
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = which;
}

#[cfg(target_os = "macos")]
mod mac {
    use super::{check, Check};
    use std::ffi::c_void;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
        static kAXTrustedCheckOptionPrompt: *const c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFBooleanTrue: *const c_void;
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
        fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *const *const c_void,
            values: *const *const c_void,
            count: isize,
            key_cb: *const c_void,
            value_cb: *const c_void,
        ) -> *const c_void;
        fn CFRelease(cf: *const c_void);
    }

    /// Accessibility, showing the system prompt when `ask`.
    fn accessibility(ask: bool) -> bool {
        unsafe {
            if !ask {
                return AXIsProcessTrusted();
            }
            let keys = [kAXTrustedCheckOptionPrompt];
            let values = [kCFBooleanTrue];
            let dict = CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &kCFTypeDictionaryKeyCallBacks as *const c_void,
                &kCFTypeDictionaryValueCallBacks as *const c_void,
            );
            let ok = AXIsProcessTrustedWithOptions(dict);
            if !dict.is_null() {
                CFRelease(dict);
            }
            ok
        }
    }

    pub fn check_all(ask: bool) -> Vec<Check> {
        let mut screen = unsafe { CGPreflightScreenCaptureAccess() };
        if !screen && ask {
            // Shows the system prompt and adds Everywhere to the list.
            screen = unsafe { CGRequestScreenCaptureAccess() };
        }
        let input = accessibility(ask && screen);
        vec![
            if screen {
                check("Screen & sound", true, "Allowed", None)
            } else {
                check(
                    "Screen & sound",
                    false,
                    "Not allowed yet: turn on Everywhere in Screen & System Audio Recording, then restart Everywhere.",
                    Some("screen"),
                )
            },
            if input {
                check("Mouse & keyboard", true, "Allowed", None)
            } else {
                check(
                    "Mouse & keyboard",
                    false,
                    "Not allowed yet: turn on Everywhere in Accessibility.",
                    Some("accessibility"),
                )
            },
        ]
    }
}
