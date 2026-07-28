//! Time-stamped ring buffer for GUI events. Max 1000 entries (FIFO eviction).
//! `export_to_file` writes to a file in `directories::ProjectDirs`-resolved location.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_ENTRIES: usize = 1000;

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp_ms: u64,
    pub level: LogLevel,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Debug)]
pub struct LogRing {
    inner: Mutex<Vec<LogEntry>>,
}

impl Default for LogRing {
    fn default() -> Self {
        Self::new()
    }
}

impl LogRing {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Vec::with_capacity(MAX_ENTRIES)),
        }
    }

    pub fn push(&self, level: LogLevel, message: impl Into<String>) {
        let mut g = self.inner.lock().expect("LogRing poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        g.push(LogEntry {
            timestamp_ms: now,
            level,
            message: message.into(),
        });
        if g.len() > MAX_ENTRIES {
            let excess = g.len() - MAX_ENTRIES;
            g.drain(0..excess);
        }
    }

    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.inner.lock().expect("LogRing poisoned").clone()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().expect("LogRing poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub fn export_log_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("io", "smitea", "bee-gui")
        .map(|p| p.data_dir().join("log"))
}

pub fn export_path() -> PathBuf {
    export_log_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bee-gui.log")
}

pub fn export_to_file(entries: &[LogEntry]) -> std::io::Result<PathBuf> {
    let path = export_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut s = String::new();
    for e in entries {
        s.push_str(&format!("{:-13} {}\n", e.level.as_str(), e.message));
    }
    fs::write(&path, s)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_ring_buffer_eviction() {
        let ring = LogRing::new();
        for i in 0..(MAX_ENTRIES + 1) {
            ring.push(LogLevel::Info, format!("entry {}", i));
        }
        assert_eq!(ring.len(), MAX_ENTRIES);
        let snap = ring.snapshot();
        // The oldest entry should have been evicted, so the first remaining
        // entry is the second one we pushed.
        assert!(
            snap.first().unwrap().message.contains("1"),
            "oldest evicted: first message = {}",
            snap.first().unwrap().message
        );
    }

    #[test]
    fn log_export_writes_file() {
        let ring = LogRing::new();
        ring.push(LogLevel::Info, "hello world");
        let entries = ring.snapshot();
        let path = export_to_file(&entries).expect("export ok");
        assert!(path.exists(), "export file should exist at {:?}", path);
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("hello world"));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn log_level_as_str() {
        assert_eq!(LogLevel::Debug.as_str(), "DEBUG");
        assert_eq!(LogLevel::Info.as_str(), "INFO");
        assert_eq!(LogLevel::Warn.as_str(), "WARN");
        assert_eq!(LogLevel::Error.as_str(), "ERROR");
    }
}