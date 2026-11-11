/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The crates workspace symbol search lists, as rust-analyzer does: one entry per crate root file, under the crate's
//! name, and another under the Cargo target's name when the file names it with dashes (`completion-derive`). Build
//! scripts are no crates to list, and the other units compiled from one root (a binary's test harness) are the same
//! crate.

use super::super::target_kind::TargetKind;
use super::super::WorkspaceIndex;
use crate::reader::IndexReader;

/// Marks a crate root entry listed under the crate's name.
pub(super) const CRATE_ROOT: u32 = u32::MAX;
/// Marks a crate root entry listed under its Cargo target's dashed name.
pub(super) const CRATE_TARGET_NAME: u32 = u32::MAX - 1;

/// Whether `def` marks a crate root entry.
pub(super) fn is_crate_entry(def: u32) -> bool {
    def >= CRATE_TARGET_NAME
}

/// The Cargo target name a root file gives when it names the target with dashes (`examples/completion-derive.rs`);
/// a `main.rs`/`lib.rs` takes its package's name, which the crate's name already spells.
pub(super) fn dashed_target_name(root_file: &str) -> Option<&str> {
    let stem = root_file.rsplit(['/', '\\']).next()?.strip_suffix(".rs")?;
    (stem.contains('-') && !matches!(stem, "main" | "lib" | "mod")).then_some(stem)
}

/// The name a crate entry `def` of `reader` is listed under.
pub(super) fn crate_entry_name(reader: &IndexReader<memmap2::Mmap>, def: u32) -> &str {
    if def == CRATE_TARGET_NAME {
        dashed_target_name(reader.get_file(0)).unwrap_or(reader.crate_name())
    } else {
        reader.crate_name()
    }
}

/// `(reader, entry)` for every crate to list.
pub(super) fn crate_entries(ws: &WorkspaceIndex) -> Vec<(u32, u32)> {
    // Per root file (resolved against the reader's source root), the unit with the shortest crate name.
    let mut by_root: std::collections::HashMap<std::path::PathBuf, usize> = std::collections::HashMap::new();
    for (i, reader) in ws.readers.iter().enumerate() {
        let (crate_name, root_file) = (reader.crate_name(), reader.get_file(0));
        if crate_name.is_empty() || root_file.is_empty() || ws.target_kind_at(i) == Some(TargetKind::CustomBuild) {
            continue;
        }
        // Without a source root a relative `src/lib.rs` names no one file: the crate's name keeps those apart.
        let root = ws.source_roots.get(i).and_then(|r| r.as_deref()).map_or_else(|| std::path::Path::new(crate_name).join(root_file), |r| r.join(root_file));
        let slot = by_root.entry(root).or_insert(i);
        if crate_name.len() < ws.readers[*slot].crate_name().len() {
            *slot = i;
        }
    }
    let mut readers: Vec<usize> = by_root.into_values().collect();
    readers.sort_unstable();
    let mut out = Vec::new();
    for i in readers {
        out.push((i as u32, CRATE_ROOT));
        let reader = &ws.readers[i];
        if dashed_target_name(reader.get_file(0)).is_some_and(|t| t.replace('-', "_") == reader.crate_name()) {
            out.push((i as u32, CRATE_TARGET_NAME));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::dashed_target_name;

    #[test]
    fn a_dashed_file_names_its_target() {
        assert_eq!(dashed_target_name("clap_complete/examples/completion-derive.rs"), Some("completion-derive"));
        assert_eq!(dashed_target_name("examples/completion.rs"), None);
        assert_eq!(dashed_target_name("tools/build-wasm-example/src/main.rs"), None, "a package's binary keeps the crate's name");
    }
}
