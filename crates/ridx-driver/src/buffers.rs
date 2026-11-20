/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiling the text of an editor's unsaved buffers instead of the files on disk. `RIDX_BUFFERS` names a JSON
//! object `{ "<source path>": "<file holding the buffer's text>" }`; the compiler then reads each such source from
//! its buffer, and every other file from disk. Nothing on disk is written, so the index of the session describes
//! the code being edited while the saved files and their index stay as they are.

use std::collections::HashMap;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use rustc_span::source_map::{FileLoader, RealFileLoader};

pub(crate) const ENV: &str = "RIDX_BUFFERS";

pub(crate) struct BufferLoader {
    /// Source path, canonical, to the file holding its buffer's text.
    buffers: HashMap<PathBuf, PathBuf>,
}

impl BufferLoader {
    /// The loader the buffer list `list` (the file `RIDX_BUFFERS` names) describes, if readable.
    pub(crate) fn from_list(list: &Path) -> Option<Self> {
        let map: HashMap<String, String> = serde_json::from_slice(&std::fs::read(list).ok()?).ok()?;
        Some(Self::new(map.into_iter().map(|(src, text)| (PathBuf::from(src), PathBuf::from(text)))))
    }

    pub(crate) fn new(buffers: impl IntoIterator<Item = (PathBuf, PathBuf)>) -> Self {
        Self { buffers: buffers.into_iter().map(|(src, text)| (canonical(&src), text)).collect() }
    }

    fn buffer_of(&self, path: &Path) -> Option<&Path> {
        if self.buffers.is_empty() {
            return None;
        }
        self.buffers.get(&canonical(path)).map(PathBuf::as_path)
    }
}

/// One spelling for a path, whether rustc asks for it relative to its working directory or absolute, and whether the
/// file exists or is only a buffer (review of ca0c1ec7: `src/new.rs`, typed but never saved, missed its absolute key).
fn canonical(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().map(|d| d.join(path)).unwrap_or_else(|_| path.to_path_buf()) };
    if let Ok(c) = absolute.canonicalize() {
        return c;
    }
    // Not on disk: component by component, canonical (symbolic links resolved) as long as the path exists, then without
    // `.` and with `..` taking the component before (review of 11310378: `missing/../new.rs` lost its `..`).
    let mut out = PathBuf::new();
    let mut on_disk = true;
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => {
                out.push(other);
                if on_disk {
                    match out.canonicalize() {
                        Ok(c) => out = c,
                        Err(_) => on_disk = false,
                    }
                }
            }
        }
    }
    out
}

impl FileLoader for BufferLoader {
    fn file_exists(&self, path: &Path) -> bool {
        self.buffer_of(path).is_some() || RealFileLoader.file_exists(path)
    }

    fn read_file(&self, path: &Path) -> io::Result<String> {
        match self.buffer_of(path) {
            Some(text) => std::fs::read_to_string(text),
            None => RealFileLoader.read_file(path),
        }
    }

    fn read_binary_file(&self, path: &Path) -> io::Result<Arc<[u8]>> {
        match self.buffer_of(path) {
            Some(text) => std::fs::read(text).map(Into::into),
            None => RealFileLoader.read_binary_file(path),
        }
    }

    fn current_directory(&self) -> io::Result<PathBuf> {
        RealFileLoader.current_directory()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a_buffer_not_on_disk_is_found_by_a_relative_path() {
        let dir = std::env::current_dir().unwrap().join(format!("ridx_buffers_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let text = dir.join("text.rs");
        std::fs::write(&text, "pub fn new() {}\n").unwrap();
        let loader = BufferLoader::new([(dir.join("src/new.rs"), text)]);
        let relative = PathBuf::from(dir.file_name().unwrap()).join("src/./new.rs");
        assert!(loader.file_exists(&relative), "a typed, unsaved file asked for relative to the working directory");
        assert_eq!(loader.read_file(&relative).unwrap(), "pub fn new() {}\n");
        assert!(!loader.file_exists(&dir.join("src/other.rs")));
        assert!(loader.file_exists(&dir.join("missing/../src/new.rs")), "a `..` after a directory that does not exist");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
