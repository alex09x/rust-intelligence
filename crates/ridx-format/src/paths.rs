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

/// Canonicalizes `path`, falling back to canonicalizing its deepest existing ancestor and
/// appending any non-existent suffix components normalized via [`normalize_path`].
pub fn canonicalize_best_effort(path: &Path) -> PathBuf {
    if let Ok(canon) = std::fs::canonicalize(path) {
        return canon;
    }
    for ancestor in path.ancestors().skip(1) {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        if let Ok(canon_ancestor) = std::fs::canonicalize(ancestor) {
            if let Ok(tail) = path.strip_prefix(ancestor) {
                return normalize_path(&canon_ancestor.join(tail));
            }
        }
    }
    normalize_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_best_effort_resolves_symlink_prefix_for_missing_descendant() {
        let dir = std::env::temp_dir().join(format!("ridx_paths_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let real_sub = dir.join("real");
        std::fs::create_dir_all(&real_sub).unwrap();

        #[cfg(unix)]
        {
            let link_sub = dir.join("link");
            std::os::unix::fs::symlink(&real_sub, &link_sub).unwrap();

            let expected = std::fs::canonicalize(&real_sub).unwrap().join("missing/leaf.rs");
            let via_link = canonicalize_best_effort(&link_sub.join("missing/./sub/../leaf.rs"));
            assert_eq!(via_link, expected);

            // Relative path without existing prefix keeps lexical form.
            assert_eq!(canonicalize_best_effort(Path::new("no_such_dir/a/../b.rs")), PathBuf::from("no_such_dir/b.rs"));
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
