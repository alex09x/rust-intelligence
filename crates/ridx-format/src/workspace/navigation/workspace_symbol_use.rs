/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Imports among workspace symbols: whether one is public (the index records no visibility for `use` items, so the
//! source decides) and the kind of the item it imports.

use crate::format::DefKind;
use crate::workspace::WorkspaceIndex;

/// Kind of the item a `use` imports, through the path its signature records; `Use` when the index cannot tell.
pub(super) fn import_target_kind(ws: &WorkspaceIndex, reader: &crate::reader::IndexReader<memmap2::Mmap>, target: &str) -> DefKind {
    reader
        .find_definition_by_path(target)
        .map(|d| DefKind::from(d.kind))
        .or_else(|| ws.find_definition_by_path(target).map(|d| d.kind))
        .or_else(|| ws.find_definition_by_path(&format!("{}::{target}", reader.crate_name())).map(|d| d.kind))
        .unwrap_or(DefKind::Use)
}

/// Whether a private `use` (path `use_path`, importing `target`) takes an explicit visibility from the item it
/// imports, as rust-analyzer's visibility minimum does: the item is `pub(super)` in a direct child of the importing
/// module (`use self::state::Version;` with `mod state { pub(super) struct Version; }`), so both visibilities name
/// the same module and the item's explicit one is kept. Gives the item's file and name position when it lies in such
/// a child; `declared_pub_super` then reads its visibility from the source.
pub(super) fn child_item_of_private_import<'a>(reader: &'a crate::reader::IndexReader<memmap2::Mmap>, use_path: &str, target: &str) -> Option<(&'a str, u32, u32)> {
    let (use_module, _) = use_path.rsplit_once("::")?;
    let (item_module, _) = target.rsplit_once("::")?;
    if item_module.rsplit_once("::").map(|(parent, _)| parent) != Some(use_module) {
        return None;
    }
    let span = reader.find_definition_by_path(target)?.span();
    Some((reader.get_file(span.file_id()), span.start_line(), span.start_col()))
}

/// Whether the item whose name is at `(line_1b, col_1b)` of `src` is declared `pub(super)`.
pub(super) fn declared_pub_super(src: &str, line_1b: u32, col_1b: u32) -> bool {
    let Some(line) = src.lines().nth(line_1b.saturating_sub(1) as usize) else { return false };
    let head: String = line.chars().take(col_1b.saturating_sub(1) as usize).collect();
    head.trim_start().starts_with("pub(super)")
}

/// Whether the `use` statement holding the name at `(line_1b, col_1b)` is `pub` or `pub(…)`. The index records
/// no visibility for imports, so the source decides: walk back from the name to the statement's `use` keyword,
/// across enclosing `{…}` groups, and look at the token before it.
pub(super) fn use_is_pub(src: &str, line_1b: u32, col_1b: u32) -> bool {
    let Some(line_start) = src.split_inclusive('\n').take(line_1b.saturating_sub(1) as usize).map(str::len).try_fold(0usize, |a, l| a.checked_add(l)) else {
        return false;
    };
    let line = src[line_start..].split('\n').next().unwrap_or("");
    let at = line_start + line.char_indices().nth(col_1b.saturating_sub(1) as usize).map_or(line.len(), |(b, _)| b);
    let b = src.as_bytes();
    let is_id = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut depth = 0i32;
    let mut i = at.min(b.len());
    while i > 0 {
        i -= 1;
        match b[i] {
            b'}' => depth += 1,
            b'{' => depth = (depth - 1).max(0),
            b';' if depth == 0 => return false,
            b'e' if depth == 0 && i >= 2 && &b[i - 2..=i] == b"use" && (i < 3 || !is_id(b[i - 3])) && b.get(i + 1).is_some_and(|c| !is_id(*c)) => {
                let before = src[..i - 2].trim_end();
                let before = if before.ends_with(')') { before[..before.rfind('(').unwrap_or(0)].trim_end() } else { before };
                return before.ends_with("pub") && (before.len() == 3 || !is_id(before.as_bytes()[before.len() - 4]));
            }
            _ => {}
        }
    }
    false
}
