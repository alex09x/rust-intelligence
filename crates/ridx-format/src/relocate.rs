/*
 * rust-intelligence — Compiler-backed, mmap-indexed code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Moving the targets of a shard's references to where their definitions now are, without compiling the crate again.
//! A reference records the line and column of the definition it resolved to (`RefEntry::target_line`/`target_col`),
//! and references, rename and safe delete compare them with the definition's current position: when a crate is edited
//! without a change of what its dependents see, only these numbers in the dependents' shards are out of date.

use crate::format::RefEntry;
use crate::reader::IndexReader;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;

/// Where the definition a reference targets is now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Where it was: the reference stays as it is.
    Same,
    /// At this line and column (1-based, the column in characters as the driver records it).
    Moved(u32, u32),
    /// Cannot be told: the shard is not rewritten, the crate is to be compiled.
    Unknown,
}

/// Rewrites the target position of each reference of the shard at `path` as `place` says, given the reference's target
/// file (as recorded) and its target line and column; a reference with no target line is left as it is. The file is
/// replaced whole (written next to it, then renamed over it), so a server holding the old one mapped keeps reading it.
/// Returns how many references moved, or `None` when `place` answered [`Placement::Unknown`] for one: nothing is then
/// written.
pub fn relocate_reference_targets(path: &Path, mut place: impl FnMut(&str, u32, u32) -> Placement) -> io::Result<Option<usize>> {
    let reader = IndexReader::open(path)?;
    let refs_offset = reader.header().refs_offset as usize;
    let mut moves = Vec::new();
    // Many references share a target: each is placed once.
    let mut placed: HashMap<(u32, u32, u32), Placement> = HashMap::new();
    for (i, r) in reader.references().iter().enumerate() {
        let (line, col) = (r.target_line(), r.target_col());
        if line == 0 {
            continue;
        }
        let key = (r.target_file_offset(), line, col);
        let placement = *placed.entry(key).or_insert_with(|| place(reader.get_str(r.target_file_offset()), line, col));
        match placement {
            Placement::Same => {}
            Placement::Moved(l, c) if (l, c) == (line, col) => {}
            Placement::Moved(l, c) => moves.push((i, l, c)),
            Placement::Unknown => return Ok(None),
        }
    }
    drop(reader);
    if moves.is_empty() {
        return Ok(Some(0));
    }
    let mut bytes = fs::read(path)?;
    let size = std::mem::size_of::<RefEntry>();
    let (line_at, col_at) = (std::mem::offset_of!(RefEntry, target_line), std::mem::offset_of!(RefEntry, target_col));
    for &(i, line, col) in &moves {
        let entry = refs_offset + i * size;
        bytes[entry + line_at..entry + line_at + 4].copy_from_slice(&line.to_le_bytes());
        bytes[entry + col_at..entry + col_at + 4].copy_from_slice(&col.to_le_bytes());
    }
    let tmp = path.with_extension(format!("ridx.relocate.{}", std::process::id()));
    if let Err(e) = fs::write(&tmp, &bytes).and_then(|()| fs::rename(&tmp, path)) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(Some(moves.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::{IndexWriter, RawReference};

    fn reference(line: u32, target_file: &str, target_line: u32, target_col: u32) -> RawReference {
        RawReference {
            file: "src/lib.rs".into(),
            start_line: line,
            start_col: 5,
            end_line: line,
            end_col: 9,
            target_def_id: None,
            target_path: "a::f".into(),
            target_file: Some(target_file.into()),
            target_unit: Some("unit-a".into()),
            target_line,
            target_col,
            kind: crate::DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        }
    }

    fn shard(dir: &Path, refs: Vec<RawReference>) -> std::path::PathBuf {
        let mut w = IndexWriter::new("b", "0.1.0");
        for r in refs {
            w.add_reference(r);
        }
        let path = dir.join("b.ridx");
        w.write_to(&mut fs::File::create(&path).unwrap()).unwrap();
        path
    }

    #[test]
    fn test_reference_targets_move_and_the_rest_stays() {
        let dir = tempfile::tempdir().unwrap();
        let path =
            shard(dir.path(), vec![reference(1, "a/src/lib.rs", 3, 8), reference(2, "a/src/lib.rs", 3, 8), reference(3, "other.rs", 3, 8), reference(4, "a/src/lib.rs", 0, 0)]);
        let mut asked = Vec::new();
        let moved = relocate_reference_targets(&path, |file, line, col| {
            asked.push((file.to_string(), line, col));
            if file == "a/src/lib.rs" {
                Placement::Moved(13, 8)
            } else {
                Placement::Same
            }
        })
        .unwrap();
        assert_eq!(moved, Some(2), "both references of the moved definition");
        assert_eq!(asked, [("a/src/lib.rs".to_string(), 3, 8), ("other.rs".to_string(), 3, 8)], "each target placed once, none without a line");
        let reader = IndexReader::open(&path).unwrap();
        let targets: Vec<(u32, u32)> = reader.references().iter().map(|r| (r.target_line(), r.target_col())).collect();
        assert_eq!(targets, [(13, 8), (13, 8), (3, 8), (0, 0)]);
        assert_eq!(reader.references()[0].target_path_offset(), reader.references()[2].target_path_offset(), "paths, by which the target index hashes, stay");
    }

    #[test]
    fn test_an_unknown_target_leaves_the_shard_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = shard(dir.path(), vec![reference(1, "a/src/lib.rs", 3, 8), reference(2, "a/src/lib.rs", 9, 1)]);
        let before = fs::read(&path).unwrap();
        let moved = relocate_reference_targets(&path, |_, line, _| if line == 3 { Placement::Moved(4, 8) } else { Placement::Unknown }).unwrap();
        assert_eq!(moved, None);
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}
