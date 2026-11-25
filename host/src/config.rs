//! Settings kept on this PC: the PC code, the password and stream options.
//!
//! Stored as JSON in the user's config folder, readable only by this user:
//!   Windows  %APPDATA%\Everywhere\config.json
//!   macOS    ~/Library/Application Support/Everywhere/config.json
//!   Linux    ~/.config/everywhere/config.json

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::pake::random_bytes;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 9 digits. An address, not a secret.
    pub pc_code: String,
    /// The secret. Checked by this PC only, through the PAKE.
    pub password: String,
    pub relays: Vec<String>,
    pub stun_servers: Vec<String>,
    pub width: usize,
    pub height: usize,
    pub fps: u32,
    pub max_bitrate_mbps: f64,
    /// Ask the router for a port per session (UPnP). Off: hole punching only.
    pub upnp: bool,
    /// Local status page, 127.0.0.1 only.
    pub status_port: u16,
    /// "screen" (default on Windows) or "pattern".
    pub source: String,
    /// The web client people open on the laptop (GitHub Pages of the repo).
    pub web_url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            pc_code: new_pc_code(),
            password: new_password(),
            relays: crate::nostr::DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect(),
            stun_servers: vec!["stun.l.google.com:19302".into(), "stun.cloudflare.com:3478".into()],
            width: 1920,
            height: 1080,
            fps: 60,
            max_bitrate_mbps: 20.0,
            upnp: true,
            status_port: 47800,
            source: "screen".into(),
            web_url: DEFAULT_WEB_URL.into(),
        }
    }
}

pub const DEFAULT_WEB_URL: &str = "https://mohamedzh4.github.io/Everywhere_P2P-personel-cloude-desktop/";

pub fn new_pc_code() -> String {
    let n = u64::from_le_bytes(random_bytes::<8>()) % 1_000_000_000;
    format!("{n:09}")
}

/// 12 characters from an alphabet without look-alikes (no 0/o, 1/l/i),
/// shown as xxxx-xxxx-xxxx. About 59 bits; online guessing is rate-limited.
pub fn new_password() -> String {
    const A: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut out = String::new();
    let mut i = 0;
    while i < 12 {
        let b = random_bytes::<1>()[0];
        // Rejection sampling keeps the choice uniform.
        if (b as usize) < 256 - 256 % A.len() {
            if i > 0 && i % 4 == 0 {
                out.push('-');
            }
            out.push(A[b as usize % A.len()] as char);
            i += 1;
        }
    }
    out
}

/// PC code as shown to people: 482-913-007.
pub fn pretty_code(code: &str) -> String {
    code.as_bytes().chunks(3).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join("-")
}

/// What the user types may have dashes or spaces.
#[cfg_attr(not(test), allow(dead_code))]
pub fn normalize_code(input: &str) -> String {
    input.chars().filter(|c| c.is_ascii_digit()).collect()
}

pub fn dir() -> PathBuf {
    if let Ok(d) = std::env::var("EVERYWHERE_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    #[cfg(windows)]
    {
        if let Ok(a) = std::env::var("APPDATA") {
            return PathBuf::from(a).join("Everywhere");
        }
    }
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."));
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Everywhere")
    } else {
        std::env::var("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|_| home.join(".config")).join("everywhere")
    }
}

pub fn path() -> PathBuf {
    dir().join("config.json")
}

pub fn load_or_create() -> Result<Config, String> {
    let p = path();
    match std::fs::read_to_string(&p) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("{}: {e}", p.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let c = Config::default();
            save(&c)?;
            Ok(c)
        }
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
}

pub fn save(c: &Config) -> Result<(), String> {
    let p = path();
    std::fs::create_dir_all(dir()).map_err(|e| format!("{}: {e}", dir().display()))?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(c).expect("serialize config")).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    // On Windows the file inherits %APPDATA%'s ACL, which is per-user already.
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_and_password_shapes() {
        for _ in 0..200 {
            let c = new_pc_code();
            assert_eq!(c.len(), 9);
            assert!(c.chars().all(|x| x.is_ascii_digit()));
            let p = new_password();
            assert_eq!(p.len(), 14);
            assert_eq!(p.matches('-').count(), 2);
            assert!(!p.contains(['0', 'o', '1', 'l', 'i']));
        }
        assert_eq!(pretty_code("482913007"), "482-913-007");
        assert_eq!(normalize_code(" 482-913 007 "), "482913007");
    }

    #[test]
    fn save_and_load_round_trip() {
        let d = std::env::temp_dir().join(format!("everywhere-test-{}", std::process::id()));
        std::env::set_var("EVERYWHERE_CONFIG_DIR", &d);
        let c = load_or_create().unwrap();
        let again = load_or_create().unwrap();
        assert_eq!(c.pc_code, again.pc_code);
        assert_eq!(c.password, again.password);
        let _ = std::fs::remove_dir_all(&d);
        std::env::remove_var("EVERYWHERE_CONFIG_DIR");
    }
}
