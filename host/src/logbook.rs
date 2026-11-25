//! Everything the host has to say: printed to the console (when there is
//! one), kept in memory for the app window, and appended to
//! `everywhere.log` next to the settings file, so a user can send it.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::Mutex;

const KEEP: usize = 300;
static LINES: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
static FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// Start writing to the log file (a fresh file each run).
pub fn open_file() {
    let path = crate::config::path().with_file_name("everywhere.log");
    if let Ok(f) = std::fs::File::create(path) {
        *FILE.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }
}

pub fn line(msg: String) {
    println!("{msg}");
    if let Some(f) = FILE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = writeln!(f, "{msg}");
    }
    let mut l = LINES.lock().unwrap_or_else(|e| e.into_inner());
    l.push_back(msg);
    while l.len() > KEEP {
        l.pop_front();
    }
}

/// The most recent lines, oldest first.
pub fn recent() -> Vec<String> {
    LINES.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect()
}

/// `say!("...", args)`: like `println!`, but also into the window and file.
#[macro_export]
macro_rules! say {
    ($($t:tt)*) => { $crate::logbook::line(format!($($t)*)) };
}
