// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Structured logging to a size-capped file.
//!
//! stdout and stderr are owned by someone else: in `pmp mcp` they carry the
//! JSON-RPC stream, and in the TUI stderr would paint over the alternate
//! screen. Logs therefore never go there. They are appended as one JSON object
//! per line to `data_dir/pmp/logs/pmp.log` (the name can be overridden from
//! `[logging]`), and the file rotates to `<name>.1` once it reaches
//! [`MAX_LOG_BYTES`], so a long session cannot fill the disk.
//!
//! [`init`] is called once from the composition root. It is deliberately
//! forgiving: a missing or unwritable logs directory leaves the process running
//! without file logs instead of aborting the TUI or the MCP server. `pmp
//! doctor` reports the same paths so the failure is visible.

use std::fs::OpenOptions;
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tracing::Subscriber;
use tracing_subscriber::fmt;

use crate::config::LoggingConfig;

/// Log file name used when `[logging] file` is unset.
pub const DEFAULT_LOG_FILE: &str = "pmp.log";

/// Size at which the current file rotates to `<name>.1`.
pub const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;

/// Where the log file lives, or `None` when file logging is disabled (an empty
/// `[logging] file` value). Relative names are resolved inside `logs_dir`.
pub fn destination(config: &LoggingConfig, logs_dir: &Path) -> Option<PathBuf> {
    let file = match config.file.as_deref() {
        None => DEFAULT_LOG_FILE,
        Some(file) => file.trim(),
    };
    if file.is_empty() {
        return None;
    }
    let path = Path::new(file);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        logs_dir.join(path)
    })
}

/// Builds the subscriber that writes JSON lines to the configured file, plus
/// the path it writes to. `None` when logging is disabled or the file cannot be
/// opened; callers must never fail because of logging.
pub fn build(
    config: &LoggingConfig,
    logs_dir: &Path,
) -> Option<(PathBuf, impl Subscriber + Send + Sync + 'static)> {
    let path = destination(config, logs_dir)?;
    let file = RotatingFile::open(&path, MAX_LOG_BYTES).ok()?;
    let writer = Arc::new(Mutex::new(file));
    let subscriber = fmt::Subscriber::builder()
        .json()
        .with_ansi(false)
        .with_max_level(config.level())
        .with_writer(move || BufferedWriter::new(Arc::clone(&writer)))
        .finish();
    Some((path, subscriber))
}

/// Installs the global logger, once, from the composition root.
///
/// The subscriber never writes to stderr: on failure the process simply keeps
/// running without file logs, and `pmp doctor` reports why.
pub fn init(config: &LoggingConfig, logs_dir: &Path) {
    let Some((path, subscriber)) = build(config, logs_dir) else {
        return;
    };
    // Another subscriber may already be installed (a test harness, an embedding
    // application); logging must not abort startup in that case either.
    if tracing::subscriber::set_global_default(subscriber).is_ok() {
        tracing::debug!(path = %path.display(), level = %config.level(), "logging to file");
    }
}

/// A size-capped append-only log file with one rotated backup.
struct RotatingFile {
    path: PathBuf,
    file: std::fs::File,
    written: u64,
    max_bytes: u64,
}

impl RotatingFile {
    fn open(path: &Path, max_bytes: u64) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let written = file.metadata()?.len();
        Ok(Self {
            path: path.to_path_buf(),
            file,
            written,
            max_bytes: max_bytes.max(1),
        })
    }

    /// Moves the current file to `<name>.1`, replacing the previous backup.
    /// If the rename fails (permissions, another mount), the file is truncated
    /// in place so the size limit still holds.
    fn rotate(&mut self) -> io::Result<()> {
        self.file.flush()?;
        let name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(DEFAULT_LOG_FILE);
        let backup = self.path.with_file_name(format!("{name}.1"));
        let _ = std::fs::remove_file(&backup);
        match std::fs::rename(&self.path, &backup) {
            Ok(()) => {
                self.file = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&self.path)?;
            }
            Err(_) => {
                self.file.set_len(0)?;
                self.file.seek(SeekFrom::Start(0))?;
            }
        }
        self.written = 0;
        Ok(())
    }
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.written > 0 && self.written + buf.len() as u64 > self.max_bytes {
            self.rotate()?;
        }
        let written = self.file.write(buf)?;
        self.written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        // Diagnostics are read after a crash, so every record is flushed.
        self.file.flush()
    }
}

/// One event's writer: `tracing` formats an event through several small
/// writes, so they are buffered and appended under a single lock when the
/// writer is dropped. Without this, two threads could interleave their writes
/// and break the one-JSON-object-per-line contract.
struct BufferedWriter {
    shared: Arc<Mutex<RotatingFile>>,
    buffer: Vec<u8>,
}

impl BufferedWriter {
    fn new(shared: Arc<Mutex<RotatingFile>>) -> Self {
        Self {
            shared,
            buffer: Vec::new(),
        }
    }
}

impl Write for BufferedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        // The event is not complete yet; the buffer is flushed on drop.
        Ok(())
    }
}

impl Drop for BufferedWriter {
    fn drop(&mut self) {
        if self.buffer.is_empty() {
            return;
        }
        let mut file = self
            .shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Logging must never panic: a full disk or a revoked permission just
        // loses the record.
        let _ = file.write_all(&self.buffer);
        let _ = file.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;

    fn sandbox() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        (dir, logs)
    }

    fn config(level: &str, file: Option<&str>) -> LoggingConfig {
        LoggingConfig {
            level: level.to_string(),
            file: file.map(str::to_string),
        }
    }

    #[test]
    fn destinations_default_to_the_logs_directory() {
        let (_dir, logs) = sandbox();
        assert_eq!(
            destination(&config("warn", None), &logs),
            Some(logs.join(DEFAULT_LOG_FILE))
        );
        assert_eq!(
            destination(&config("warn", Some("debug.log")), &logs),
            Some(logs.join("debug.log"))
        );
        assert_eq!(
            destination(&config("warn", Some("/tmp/pmp.log")), &logs),
            Some(PathBuf::from("/tmp/pmp.log"))
        );
    }

    #[test]
    fn an_empty_file_disables_logging() {
        let (_dir, logs) = sandbox();
        assert_eq!(destination(&config("warn", Some("  ")), &logs), None);
        assert!(build(&config("warn", Some("")), &logs).is_none());
    }

    #[test]
    fn records_are_json_lines_with_fields() {
        let (_dir, logs) = sandbox();
        let (path, subscriber) = build(&config("info", None), &logs).expect("logger builds");

        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(task_id = 7, "boom");
            tracing::info!("next");
        });

        let text = fs::read_to_string(&path).expect("log file exists");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "one JSON object per event: {text}");

        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["level"], "WARN");
        assert_eq!(first["fields"]["message"], "boom");
        assert_eq!(first["fields"]["task_id"], 7);
        assert!(first["timestamp"].is_string());
    }

    #[test]
    fn the_level_filters_lower_events() {
        let (_dir, logs) = sandbox();
        let (path, subscriber) = build(&config("error", None), &logs).expect("logger builds");

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("dropped");
            tracing::error!("kept");
        });

        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("dropped"), "{text}");
        assert!(text.contains("kept"), "{text}");
    }

    #[test]
    fn the_file_rotates_once_it_reaches_the_limit() {
        let (dir, _logs) = sandbox();
        let path = dir.path().join("pmp.log");
        let mut file = RotatingFile::open(&path, 100).expect("log file opens");

        for _ in 0..5 {
            file.write_all(&[b'x'; 60]).unwrap();
        }
        file.flush().unwrap();

        let backup = dir.path().join("pmp.log.1");
        assert!(backup.exists(), "a rotated backup should exist");
        assert!(fs::metadata(&path).unwrap().len() <= 100);
        assert!(fs::metadata(&backup).unwrap().len() <= 100);
    }

    #[test]
    fn opening_creates_the_logs_directory() {
        let (_dir, logs) = sandbox();
        assert!(!logs.exists());
        let (path, _subscriber) = build(&config("warn", None), &logs).expect("logger builds");
        assert!(path.parent().unwrap().is_dir());
        assert!(path.exists());
    }

    #[test]
    fn concurrent_events_never_interleave_lines() {
        let (_dir, logs) = sandbox();
        let path = destination(&config("info", None), &logs).unwrap();
        let workers: Vec<_> = (0..4)
            .map(|worker| {
                let logs = logs.clone();
                std::thread::spawn(move || {
                    // A subscriber per thread, all appending to the same file, is
                    // the worst case: no shared buffer can serialize them.
                    let (_path, subscriber) =
                        build(&config("info", None), &logs).expect("logger builds");
                    tracing::subscriber::with_default(subscriber, || {
                        for event in 0..25 {
                            tracing::info!(worker, event, "record");
                        }
                    });
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }

        let text = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 100, "one line per event: {text}");
        for line in lines {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["fields"]["message"], "record", "{line}");
        }
    }

    #[test]
    fn the_subscriber_can_be_built_twice_for_temporary_directories() {
        // Guards the testing strategy: `build` never installs the subscriber,
        // so two sandboxes can coexist in the same test process.
        let (first_dir, first_logs) = sandbox();
        let (second_dir, second_logs) = sandbox();
        let (first_path, first) = build(&config("warn", None), &first_logs).unwrap();
        let (second_path, second) = build(&config("warn", None), &second_logs).unwrap();

        tracing::subscriber::with_default(first, || tracing::warn!("first"));
        tracing::subscriber::with_default(second, || tracing::warn!("second"));

        assert!(fs::read_to_string(&first_path).unwrap().contains("first"));
        assert!(fs::read_to_string(&second_path).unwrap().contains("second"));
        assert!(first_dir.path().is_dir() && second_dir.path().is_dir());
    }
}
