/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Structured LSP server logging for `--log-file`, `--no-log-buffering`, `-v`, and `-q`.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

enum LogSink {
    Unbuffered(File),
    Buffered(BufWriter<File>),
    Stderr,
}

impl LogSink {
    fn write_line(&mut self, line: &str, force_flush: bool) {
        match self {
            LogSink::Unbuffered(f) => {
                let _ = f.write_all(line.as_bytes());
                let _ = f.flush();
            }
            LogSink::Buffered(w) => {
                let _ = w.write_all(line.as_bytes());
                if force_flush {
                    let _ = w.flush();
                }
            }
            LogSink::Stderr => {
                let _ = std::io::stderr().write_all(line.as_bytes());
                if force_flush {
                    let _ = std::io::stderr().flush();
                }
            }
        }
    }

    fn flush(&mut self) {
        match self {
            LogSink::Unbuffered(f) => {
                let _ = f.flush();
            }
            LogSink::Buffered(w) => {
                let _ = w.flush();
            }
            LogSink::Stderr => {
                let _ = std::io::stderr().flush();
            }
        }
    }
}

struct ActiveLogger {
    sink: LogSink,
    no_buffering: bool,
    verbose: u8,
    quiet: bool,
}

static LOGGER: OnceLock<Mutex<Option<ActiveLogger>>> = OnceLock::new();

fn logger_slot() -> &'static Mutex<Option<ActiveLogger>> {
    LOGGER.get_or_init(|| Mutex::new(None))
}

/// Initializes the LSP logger when `--log-file`, `-v`, or `-q` is supplied on the CLI.
pub fn init_lsp_logger(log_file: Option<PathBuf>, no_buffering: bool, verbose: u8, quiet: bool) {
    let sink = if let Some(ref path) = log_file {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            let _ = std::fs::create_dir_all(parent);
        }
        match OpenOptions::new().create(true).append(true).open(path) {
            Ok(file) => {
                if no_buffering {
                    Some(LogSink::Unbuffered(file))
                } else {
                    Some(LogSink::Buffered(BufWriter::new(file)))
                }
            }
            Err(_) => None,
        }
    } else if verbose > 0 {
        Some(LogSink::Stderr)
    } else {
        None
    };

    let mut guard = logger_slot().lock().unwrap();
    if let Some(mut s) = sink {
        if !quiet {
            let line = format!("[ridx-lsp] INFO: logger initialized (verbose={verbose}, no_buffering={no_buffering})\n");
            s.write_line(&line, true);
        }
        *guard = Some(ActiveLogger { sink: s, no_buffering, verbose, quiet });
    } else {
        *guard = None;
    }
}

/// Logs a lifecycle event (`initialize`, `shutdown`, index reload) when not `--quiet`.
pub fn log_info(msg: &str) {
    let Some(slot) = LOGGER.get() else {
        return;
    };
    if let Ok(mut guard) = slot.lock() {
        if let Some(ref mut logger) = *guard {
            if !logger.quiet {
                let line = format!("[ridx-lsp] INFO: {msg}\n");
                logger.sink.write_line(&line, logger.no_buffering);
            }
        }
    }
}

/// Logs per-request details when `-v` / `--verbose` is enabled and not `--quiet`.
pub fn log_verbose(msg: &str) {
    let Some(slot) = LOGGER.get() else {
        return;
    };
    if let Ok(mut guard) = slot.lock() {
        if let Some(ref mut logger) = *guard {
            if !logger.quiet && logger.verbose > 0 {
                let line = format!("[ridx-lsp] DEBUG: {msg}\n");
                logger.sink.write_line(&line, logger.no_buffering);
            }
        }
    }
}

/// Logs an LSP error response or fatal server error (written even under `--quiet`).
pub fn log_error(msg: &str) {
    let Some(slot) = LOGGER.get() else {
        return;
    };
    if let Ok(mut guard) = slot.lock() {
        if let Some(ref mut logger) = *guard {
            let line = format!("[ridx-lsp] ERROR: {msg}\n");
            logger.sink.write_line(&line, true);
        }
    }
}

/// Flushes any buffered log output.
pub fn flush_logger() {
    let Some(slot) = LOGGER.get() else {
        return;
    };
    if let Ok(mut guard) = slot.lock() {
        if let Some(ref mut logger) = *guard {
            logger.sink.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lsp_logger_writes_to_file_and_respects_verbosity() {
        let dir = std::env::temp_dir().join(format!("ridx_logger_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let log_path = dir.join("nested/ridx-lsp.log");

        init_lsp_logger(Some(log_path.clone()), true, 1, false);
        log_info("workspace opened");
        log_verbose("request textDocument/hover (id=1) -> ok");
        log_error("request experimental/runnables (id=2) -> error -32601: Method not found");
        flush_logger();

        let text = std::fs::read_to_string(&log_path).expect("log file must exist");
        assert!(text.contains("logger initialized (verbose=1, no_buffering=true)"));
        assert!(text.contains("INFO: workspace opened"));
        assert!(text.contains("DEBUG: request textDocument/hover (id=1) -> ok"));
        assert!(text.contains("ERROR: request experimental/runnables"));

        // Quiet mode suppresses INFO/DEBUG but still records ERROR
        let quiet_path = dir.join("quiet.log");
        init_lsp_logger(Some(quiet_path.clone()), false, 1, true);
        log_info("suppressed info");
        log_verbose("suppressed debug");
        log_error("fatal error recorded");
        flush_logger();

        let quiet_text = std::fs::read_to_string(&quiet_path).expect("quiet log file must exist");
        assert!(!quiet_text.contains("suppressed"));
        assert!(quiet_text.contains("ERROR: fatal error recorded"));

        init_lsp_logger(None, false, 0, false);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
