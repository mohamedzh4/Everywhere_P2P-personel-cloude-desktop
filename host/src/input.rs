//! Input from the browser: decode the data-channel messages (layout in
//! web/js/input.js) and inject them into this PC.
//!
//! Every system gets physical key positions (not characters), so games and
//! shortcuts work whatever the keyboard layout:
//! - Windows: SendInput with hardware scan codes.
//! - macOS: CGEvent with virtual key codes (needs the Accessibility
//!   permission: System Settings > Privacy & Security > Accessibility).
//! - Linux (X11): XTest with X key codes (evdev + 8).
//!
//! Limits: SendInput cannot reach windows of programs running as
//! administrator, the lock screen or UAC prompts unless the host runs as a
//! service on the secure desktop (phase 6).

use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    Move { dx: i16, dy: i16 },
    Abs { x: u16, y: u16 },
    Wheel { dx: i16, dy: i16 },
    Key { down: bool, code: String },
    Button { down: bool, button: u8 },
    ReleaseAll,
}

pub fn decode(d: &[u8]) -> Option<InputEvent> {
    let i16at = |o: usize| d.get(o..o + 2).map(|b| i16::from_le_bytes([b[0], b[1]]));
    let u16at = |o: usize| d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    Some(match *d.first()? {
        1 => InputEvent::Move { dx: i16at(1)?, dy: i16at(3)? },
        2 => InputEvent::Abs { x: u16at(1)?, y: u16at(3)? },
        3 => InputEvent::Wheel { dx: i16at(1)?, dy: i16at(3)? },
        10 => {
            let len = *d.get(2)? as usize;
            let code = std::str::from_utf8(d.get(3..3 + len)?).ok()?.to_string();
            InputEvent::Key { down: *d.get(1)? != 0, code }
        }
        11 => InputEvent::Button { down: *d.get(1)? != 0, button: *d.get(2)? },
        12 => InputEvent::ReleaseAll,
        _ => return None,
    })
}

/// PC/AT set-1 scan code for a KeyboardEvent.code, and whether it is an
/// "extended" (E0-prefixed) key.
#[cfg_attr(not(any(windows, target_os = "linux", test)), allow(dead_code))]
pub fn scan_code(code: &str) -> Option<(u16, bool)> {
    let s = |v: u16| Some((v, false));
    let e = |v: u16| Some((v, true));
    match code {
        "Escape" => s(0x01),
        "Digit1" => s(0x02), "Digit2" => s(0x03), "Digit3" => s(0x04), "Digit4" => s(0x05),
        "Digit5" => s(0x06), "Digit6" => s(0x07), "Digit7" => s(0x08), "Digit8" => s(0x09),
        "Digit9" => s(0x0A), "Digit0" => s(0x0B), "Minus" => s(0x0C), "Equal" => s(0x0D),
        "Backspace" => s(0x0E), "Tab" => s(0x0F),
        "KeyQ" => s(0x10), "KeyW" => s(0x11), "KeyE" => s(0x12), "KeyR" => s(0x13), "KeyT" => s(0x14),
        "KeyY" => s(0x15), "KeyU" => s(0x16), "KeyI" => s(0x17), "KeyO" => s(0x18), "KeyP" => s(0x19),
        "BracketLeft" => s(0x1A), "BracketRight" => s(0x1B), "Enter" => s(0x1C),
        "ControlLeft" => s(0x1D),
        "KeyA" => s(0x1E), "KeyS" => s(0x1F), "KeyD" => s(0x20), "KeyF" => s(0x21), "KeyG" => s(0x22),
        "KeyH" => s(0x23), "KeyJ" => s(0x24), "KeyK" => s(0x25), "KeyL" => s(0x26),
        "Semicolon" => s(0x27), "Quote" => s(0x28), "Backquote" => s(0x29), "ShiftLeft" => s(0x2A),
        "Backslash" => s(0x2B),
        "KeyZ" => s(0x2C), "KeyX" => s(0x2D), "KeyC" => s(0x2E), "KeyV" => s(0x2F), "KeyB" => s(0x30),
        "KeyN" => s(0x31), "KeyM" => s(0x32), "Comma" => s(0x33), "Period" => s(0x34), "Slash" => s(0x35),
        "ShiftRight" => s(0x36), "NumpadMultiply" => s(0x37), "AltLeft" => s(0x38), "Space" => s(0x39),
        "CapsLock" => s(0x3A),
        "F1" => s(0x3B), "F2" => s(0x3C), "F3" => s(0x3D), "F4" => s(0x3E), "F5" => s(0x3F),
        "F6" => s(0x40), "F7" => s(0x41), "F8" => s(0x42), "F9" => s(0x43), "F10" => s(0x44),
        "NumLock" => s(0x45), "ScrollLock" => s(0x46),
        "Numpad7" => s(0x47), "Numpad8" => s(0x48), "Numpad9" => s(0x49), "NumpadSubtract" => s(0x4A),
        "Numpad4" => s(0x4B), "Numpad5" => s(0x4C), "Numpad6" => s(0x4D), "NumpadAdd" => s(0x4E),
        "Numpad1" => s(0x4F), "Numpad2" => s(0x50), "Numpad3" => s(0x51), "Numpad0" => s(0x52),
        "NumpadDecimal" => s(0x53), "IntlBackslash" => s(0x56), "F11" => s(0x57), "F12" => s(0x58),
        "IntlRo" => s(0x73), "IntlYen" => s(0x7D),
        "NumpadEnter" => e(0x1C), "ControlRight" => e(0x1D), "NumpadDivide" => e(0x35),
        "PrintScreen" => e(0x37), "AltRight" => e(0x38), "Pause" => s(0x45),
        "Home" => e(0x47), "ArrowUp" => e(0x48), "PageUp" => e(0x49), "ArrowLeft" => e(0x4B),
        "ArrowRight" => e(0x4D), "End" => e(0x4F), "ArrowDown" => e(0x50), "PageDown" => e(0x51),
        "Insert" => e(0x52), "Delete" => e(0x53), "MetaLeft" => e(0x5B), "MetaRight" => e(0x5C),
        "ContextMenu" => e(0x5D),
        _ => None,
    }
}

/// Injects events and remembers what is held, so focus loss or a dropped
/// connection never leaves a key stuck down on the PC.
pub struct Injector {
    keys_down: HashSet<String>,
    buttons_down: HashSet<u8>,
    pub injected: u64,
}

impl Default for Injector {
    fn default() -> Self {
        Self::new()
    }
}

impl Injector {
    pub fn new() -> Self {
        Self { keys_down: HashSet::new(), buttons_down: HashSet::new(), injected: 0 }
    }

    pub fn handle(&mut self, ev: InputEvent) {
        match &ev {
            InputEvent::Key { down: true, code } => {
                self.keys_down.insert(code.clone());
            }
            InputEvent::Key { down: false, code } => {
                self.keys_down.remove(code);
            }
            InputEvent::Button { down: true, button } => {
                self.buttons_down.insert(*button);
            }
            InputEvent::Button { down: false, button } => {
                self.buttons_down.remove(button);
            }
            InputEvent::ReleaseAll => {
                self.release_all();
                return;
            }
            _ => {}
        }
        self.inject(&ev);
    }

    pub fn release_all(&mut self) {
        for code in std::mem::take(&mut self.keys_down) {
            self.inject(&InputEvent::Key { down: false, code });
        }
        for button in std::mem::take(&mut self.buttons_down) {
            self.inject(&InputEvent::Button { down: false, button });
        }
    }

    fn inject(&mut self, ev: &InputEvent) {
        self.injected += 1;
        platform::inject(ev);
    }
}

impl Drop for Injector {
    fn drop(&mut self) {
        self.release_all();
    }
}

#[cfg(all(windows, not(test)))]
mod platform {
    use super::{scan_code, InputEvent};
    use windows::Win32::UI::Input::KeyboardAndMouse::*;

    fn mouse(dx: i32, dy: i32, data: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT { dx, dy, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: 0 },
            },
        }
    }

    fn send(inputs: &[INPUT]) {
        unsafe {
            SendInput(inputs, std::mem::size_of::<INPUT>() as i32);
        }
    }

    pub fn inject(ev: &InputEvent) {
        match ev {
            InputEvent::Move { dx, dy } => send(&[mouse(*dx as i32, *dy as i32, 0, MOUSEEVENTF_MOVE)]),
            InputEvent::Abs { x, y } => send(&[mouse(
                *x as i32,
                *y as i32,
                0,
                MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            )]),
            InputEvent::Wheel { dx, dy } => {
                if *dy != 0 {
                    send(&[mouse(0, 0, *dy as i32, MOUSEEVENTF_WHEEL)]);
                }
                if *dx != 0 {
                    send(&[mouse(0, 0, *dx as i32, MOUSEEVENTF_HWHEEL)]);
                }
            }
            InputEvent::Button { down, button } => {
                let (flags, data) = match (button, down) {
                    (0, true) => (MOUSEEVENTF_LEFTDOWN, 0),
                    (0, false) => (MOUSEEVENTF_LEFTUP, 0),
                    (1, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
                    (1, false) => (MOUSEEVENTF_MIDDLEUP, 0),
                    (2, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
                    (2, false) => (MOUSEEVENTF_RIGHTUP, 0),
                    (3, true) => (MOUSEEVENTF_XDOWN, 1),
                    (3, false) => (MOUSEEVENTF_XUP, 1),
                    (4, true) => (MOUSEEVENTF_XDOWN, 2),
                    (4, false) => (MOUSEEVENTF_XUP, 2),
                    _ => return,
                };
                send(&[mouse(0, 0, data, flags)]);
            }
            InputEvent::Key { down, code } => {
                let Some((scan, extended)) = scan_code(code) else { return };
                let mut flags = KEYEVENTF_SCANCODE;
                if extended {
                    flags |= KEYEVENTF_EXTENDEDKEY;
                }
                if !down {
                    flags |= KEYEVENTF_KEYUP;
                }
                let ki = KEYBDINPUT { wVk: VIRTUAL_KEY(0), wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 };
                send(&[INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki } }]);
            }
            InputEvent::ReleaseAll => {}
        }
    }
}

/// Linux evdev key code (X key code = evdev + 8). For most keys it equals the
/// set-1 scan code; the extended keys have their own numbers.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
pub fn evdev_code(code: &str) -> Option<u16> {
    let (scan, extended) = scan_code(code)?;
    if !extended {
        return Some(match scan {
            0x73 => 89,  // IntlRo
            0x7D => 124, // IntlYen
            0x45 if code == "Pause" => 119,
            s => s,
        });
    }
    Some(match code {
        "NumpadEnter" => 96, "ControlRight" => 97, "NumpadDivide" => 98, "PrintScreen" => 99,
        "AltRight" => 100, "Home" => 102, "ArrowUp" => 103, "PageUp" => 104, "ArrowLeft" => 105,
        "ArrowRight" => 106, "End" => 107, "ArrowDown" => 108, "PageDown" => 109, "Insert" => 110,
        "Delete" => 111, "MetaLeft" => 125, "MetaRight" => 126, "ContextMenu" => 127,
        _ => return None,
    })
}

/// macOS virtual key code (kVK_*) for a KeyboardEvent.code. The Windows key
/// maps to Command, so Win+C on a PC keyboard becomes Cmd+C on the Mac.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub fn mac_keycode(code: &str) -> Option<u16> {
    Some(match code {
        "KeyA" => 0x00, "KeyS" => 0x01, "KeyD" => 0x02, "KeyF" => 0x03, "KeyH" => 0x04, "KeyG" => 0x05,
        "KeyZ" => 0x06, "KeyX" => 0x07, "KeyC" => 0x08, "KeyV" => 0x09, "IntlBackslash" => 0x0A,
        "KeyB" => 0x0B, "KeyQ" => 0x0C, "KeyW" => 0x0D, "KeyE" => 0x0E, "KeyR" => 0x0F, "KeyY" => 0x10,
        "KeyT" => 0x11, "Digit1" => 0x12, "Digit2" => 0x13, "Digit3" => 0x14, "Digit4" => 0x15,
        "Digit6" => 0x16, "Digit5" => 0x17, "Equal" => 0x18, "Digit9" => 0x19, "Digit7" => 0x1A,
        "Minus" => 0x1B, "Digit8" => 0x1C, "Digit0" => 0x1D, "BracketRight" => 0x1E, "KeyO" => 0x1F,
        "KeyU" => 0x20, "BracketLeft" => 0x21, "KeyI" => 0x22, "KeyP" => 0x23, "Enter" => 0x24,
        "KeyL" => 0x25, "KeyJ" => 0x26, "Quote" => 0x27, "KeyK" => 0x28, "Semicolon" => 0x29,
        "Backslash" => 0x2A, "Comma" => 0x2B, "Slash" => 0x2C, "KeyN" => 0x2D, "KeyM" => 0x2E,
        "Period" => 0x2F, "Tab" => 0x30, "Space" => 0x31, "Backquote" => 0x32, "Backspace" => 0x33,
        "Escape" => 0x35, "MetaRight" => 0x36, "MetaLeft" => 0x37, "ShiftLeft" => 0x38,
        "CapsLock" => 0x39, "AltLeft" => 0x3A, "ControlLeft" => 0x3B, "ShiftRight" => 0x3C,
        "AltRight" => 0x3D, "ControlRight" => 0x3E, "NumpadDecimal" => 0x41, "NumpadMultiply" => 0x43,
        "NumpadAdd" => 0x45, "NumLock" => 0x47, "NumpadDivide" => 0x4B, "NumpadEnter" => 0x4C,
        "NumpadSubtract" => 0x4E, "NumpadEqual" => 0x51, "Numpad0" => 0x52, "Numpad1" => 0x53,
        "Numpad2" => 0x54, "Numpad3" => 0x55, "Numpad4" => 0x56, "Numpad5" => 0x57, "Numpad6" => 0x58,
        "Numpad7" => 0x59, "Numpad8" => 0x5B, "Numpad9" => 0x5C, "IntlYen" => 0x5D, "IntlRo" => 0x5E,
        "F5" => 0x60, "F6" => 0x61, "F7" => 0x62, "F3" => 0x63, "F8" => 0x64, "F9" => 0x65,
        "F11" => 0x67, "F13" => 0x69, "F14" => 0x6B, "F10" => 0x6D, "F12" => 0x6F, "F15" => 0x71,
        "Insert" => 0x72, "Home" => 0x73, "PageUp" => 0x74, "Delete" => 0x75, "F4" => 0x76,
        "End" => 0x77, "F2" => 0x78, "PageDown" => 0x79, "F1" => 0x7A, "ArrowLeft" => 0x7B,
        "ArrowRight" => 0x7C, "ArrowDown" => 0x7D, "ArrowUp" => 0x7E,
        _ => return None,
    })
}

#[cfg(all(any(target_os = "macos", target_os = "linux"), not(test)))]
mod platform {
    //! macOS and Linux (X11) through enigo. Created on first use, so a host
    //! that nobody connects to never asks for the Accessibility permission.
    use super::InputEvent;
    use enigo::{Axis, Button, Coordinate, Direction, Enigo, Keyboard, Mouse, Settings};
    use std::cell::RefCell;

    struct State {
        enigo: Option<Enigo>,
        failed: bool,
        /// Wheel movement not yet sent (trackpads send small steps).
        wheel: (i32, i32),
    }

    thread_local! {
        static STATE: RefCell<State> = const { RefCell::new(State { enigo: None, failed: false, wheel: (0, 0) }) };
    }

    #[cfg(target_os = "macos")]
    fn key(code: &str) -> Option<u16> {
        super::mac_keycode(code)
    }
    #[cfg(target_os = "linux")]
    fn key(code: &str) -> Option<u16> {
        super::evdev_code(code).map(|c| c + 8)
    }

    pub fn inject(ev: &InputEvent) {
        STATE.with(|st| {
            let mut st = st.borrow_mut();
            if st.enigo.is_none() && !st.failed {
                match Enigo::new(&Settings::default()) {
                    Ok(e) => st.enigo = Some(e),
                    Err(e) => {
                        st.failed = true;
                        crate::say!(
                            "cannot control the mouse and keyboard: {e}. On a Mac, allow Everywhere in \
                             System Settings > Privacy & Security > Accessibility, then restart it"
                        );
                    }
                }
            }
            let State { enigo, wheel, .. } = &mut *st;
            let Some(en) = enigo.as_mut() else { return };
            let dir = |down: bool| if down { Direction::Press } else { Direction::Release };
            let _ = match ev {
                InputEvent::Move { dx, dy } => en.move_mouse(*dx as i32, *dy as i32, Coordinate::Rel),
                InputEvent::Abs { x, y } => {
                    let (w, h) = en.main_display().unwrap_or((1920, 1080));
                    let px = (*x as i64 * w as i64 / 65536) as i32;
                    let py = (*y as i64 * h as i64 / 65536) as i32;
                    en.move_mouse(px, py, Coordinate::Abs)
                }
                InputEvent::Wheel { dx, dy } => {
                    // Browser sends Windows units: 120 per notch, positive = away
                    // from the user. enigo counts notches, positive = down/right.
                    wheel.0 += *dx as i32;
                    wheel.1 += *dy as i32;
                    let (nx, ny) = (wheel.0 / 120, wheel.1 / 120);
                    wheel.0 -= nx * 120;
                    wheel.1 -= ny * 120;
                    if ny != 0 {
                        let _ = en.scroll(-ny, Axis::Vertical);
                    }
                    if nx != 0 {
                        let _ = en.scroll(nx, Axis::Horizontal);
                    }
                    Ok(())
                }
                InputEvent::Button { down, button } => {
                    let b = match button {
                        0 => Button::Left,
                        1 => Button::Middle,
                        2 => Button::Right,
                        3 => Button::Back,
                        4 => Button::Forward,
                        _ => return,
                    };
                    en.button(b, dir(*down))
                }
                InputEvent::Key { down, code } => match key(code) {
                    Some(k) => en.raw(k, dir(*down)),
                    None => Ok(()),
                },
                InputEvent::ReleaseAll => Ok(()),
            };
        });
    }
}

#[cfg(any(test, not(any(windows, target_os = "macos", target_os = "linux"))))]
mod platform {
    use super::InputEvent;
    /// Tests (and unknown systems): events are only counted, so running the
    /// tests never moves the real mouse.
    pub fn inject(_ev: &InputEvent) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_js_layout() {
        // Same bytes as web/test/run.mjs expects from the encoder.
        assert_eq!(decode(&hex::decode("01fdff2c01").unwrap()), Some(InputEvent::Move { dx: -3, dy: 300 }));
        assert_eq!(decode(&hex::decode("0a01044b657957").unwrap()), Some(InputEvent::Key { down: true, code: "KeyW".into() }));
        assert_eq!(decode(&hex::decode("0b0002").unwrap()), Some(InputEvent::Button { down: false, button: 2 }));
        assert_eq!(decode(&hex::decode("03000088ff").unwrap()), Some(InputEvent::Wheel { dx: 0, dy: -120 }));
        assert_eq!(decode(&[12]), Some(InputEvent::ReleaseAll));
    }

    #[test]
    fn rejects_short_or_unknown() {
        assert_eq!(decode(&[]), None);
        assert_eq!(decode(&[1, 0]), None);
        assert_eq!(decode(&[10, 1, 50, b'a']), None);
        assert_eq!(decode(&[99]), None);
    }

    #[test]
    fn release_all_clears_held_keys() {
        let mut inj = Injector::new();
        inj.handle(InputEvent::Key { down: true, code: "KeyW".into() });
        inj.handle(InputEvent::Button { down: true, button: 0 });
        inj.handle(InputEvent::ReleaseAll);
        assert!(inj.keys_down.is_empty() && inj.buttons_down.is_empty());
        assert_eq!(inj.injected, 4, "2 presses + 2 releases");
    }

    #[test]
    fn scan_codes_cover_gaming_keys() {
        for k in ["KeyW", "KeyA", "KeyS", "KeyD", "Space", "ShiftLeft", "ControlLeft", "Escape", "Tab", "ArrowUp", "F5"] {
            assert!(scan_code(k).is_some(), "{k}");
        }
        assert_eq!(scan_code("ArrowUp"), Some((0x48, true)));
    }

    #[test]
    fn linux_and_mac_key_codes() {
        // evdev: KEY_W 17, KEY_ESC 1, KEY_UP 103, KEY_RIGHTCTRL 97, KEY_102ND 86.
        assert_eq!(evdev_code("KeyW"), Some(17));
        assert_eq!(evdev_code("Escape"), Some(1));
        assert_eq!(evdev_code("ArrowUp"), Some(103));
        assert_eq!(evdev_code("ControlRight"), Some(97));
        assert_eq!(evdev_code("IntlBackslash"), Some(86));
        assert_eq!(evdev_code("Pause"), Some(119));
        // macOS: kVK_ANSI_W 0x0D, kVK_Escape 0x35, kVK_UpArrow 0x7E, Win key = Command.
        assert_eq!(mac_keycode("KeyW"), Some(0x0D));
        assert_eq!(mac_keycode("Escape"), Some(0x35));
        assert_eq!(mac_keycode("ArrowUp"), Some(0x7E));
        assert_eq!(mac_keycode("MetaLeft"), Some(0x37));
        for k in ["KeyW", "KeyA", "KeyS", "KeyD", "Space", "ShiftLeft", "ControlLeft", "Tab", "Enter", "F5"] {
            assert!(evdev_code(k).is_some() && mac_keycode(k).is_some(), "{k}");
        }
    }
}
