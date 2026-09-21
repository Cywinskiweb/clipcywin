//! Tiny file logger. Off unless a level is configured, so idle cost is zero.

use log::{Level, LevelFilter, Log, Metadata, Record};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

struct FileLogger {
    file: Mutex<Option<File>>,
    max_level: LevelFilter,
}

impl Log for FileLogger {
    fn enabled(&self, m: &Metadata) -> bool {
        m.level() <= self.max_level
    }
    fn log(&self, r: &Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let line = format!("{} [{}] {}: {}\n", ts, r.level(), r.target(), r.args());
        if let Ok(mut g) = self.file.lock() {
            if let Some(f) = g.as_mut() {
                let _ = f.write_all(line.as_bytes());
                if r.level() <= Level::Warn {
                    let _ = f.flush();
                }
            }
        }
        #[cfg(debug_assertions)]
        {
            eprint!("{line}");
        }
    }
    fn flush(&self) {
        if let Ok(mut g) = self.file.lock() {
            if let Some(f) = g.as_mut() {
                let _ = f.flush();
            }
        }
    }
}

pub fn init(path: &Path, level: LevelFilter) {
    // Rotate when > 1 MB.
    if let Ok(md) = std::fs::metadata(path) {
        if md.len() > 1_000_000 {
            let _ = std::fs::rename(path, path.with_extension("log.1"));
        }
    }
    let file = if level == LevelFilter::Off {
        None
    } else {
        OpenOptions::new().create(true).append(true).open(path).ok()
    };
    let logger = Box::new(FileLogger { file: Mutex::new(file), max_level: level });
    let _ = log::set_boxed_logger(logger);
    log::set_max_level(level);
}
