//! Input from the browser: decode the data-channel messages (layout in
//! web/js/input.js) and inject them into this PC.
//!
//! Windows: SendInput with hardware scan codes, so games see real key
//! positions whatever the keyboard layout. Other systems: events are counted
//! and dropped (the host is Windows; this keeps the rest testable).
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
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
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

#[cfg(windows)]
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

#[cfg(not(windows))]
mod platform {
    use super::InputEvent;
    /// No injection off Windows; events are only counted by the Injector.
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
}
