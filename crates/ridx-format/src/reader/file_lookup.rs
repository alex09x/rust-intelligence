/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Looking a stored file up by its path: the one place where a request's spelling is matched
//! against the file table of a shard.

use std::path::Path;

use super::IndexReader;
use crate::paths::normalize_path;

impl<B: AsRef<[u8]>> IndexReader<B> {
    /// Looks up the file_id of the file stored as `file_path`: the same string, or the same path
    /// by components (`./src/lib.rs` and `src//lib.rs` name `src/lib.rs`), or a stored path whose
    /// `..` components resolve lexically to it: a file pulled in by `include!("../test_data/x.rs")`
    /// is stored as rustc spelled it, `crate/src/../test_data/x.rs`, and `crate/test_data/x.rs`
    /// names it. `..` on the query side is not resolved here: through a symlinked parent it is a
    /// filesystem question, which `WorkspaceIndex` settles before asking. A path that merely ends
    /// with a stored one, such as another crate's `src/lib.rs`, does not match. The stored form is
    /// the one the driver wrote: relative to the workspace root for workspace crates, relative to
    /// the package directory for dependencies, relative to the rust-src root for the sysroot;
    /// `WorkspaceIndex::target_file_for_reader` turns any input into it.
    pub fn find_file_id(&self, file_path: &str) -> Option<u32> {
        let wanted = Path::new(file_path.strip_prefix("./").unwrap_or(file_path));
        for id in 0..self.header.files_count {
            let f = self.get_file(id);
            if f == file_path || Path::new(f) == wanted {
                return Some(id);
            }
            if f.contains("..") && normalize_path(Path::new(f)) == wanted {
                return Some(id);
            }
        }
        None
    }
}
