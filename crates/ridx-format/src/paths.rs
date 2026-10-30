/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Path spellings shared by the shard reader and the workspace layer: the lexical form of a path
//! as rustc, cargo and the driver may spell it, without touching the filesystem.

use std::path::{Path, PathBuf};

/// Normalizes a path by removing redundant `.` and resolving `..` components lexically. A `..`
/// after a normal component removes it; a leading `..` of a relative path is kept, a `..` right
/// after the root of an absolute path is dropped. Symlinks are not consulted: callers that need
/// filesystem identity resolve the path first.
pub fn normalize_path(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut components = Vec::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if let Some(Component::Normal(_)) = components.last() {
                    components.pop();
                } else if !components.iter().any(|comp| matches!(comp, Component::RootDir | Component::Prefix(_))) {
                    components.push(c);
                }
            }
            _ => components.push(c),
        }
    }
    components.into_iter().collect()
}
