/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Hover on code the index has not compiled yet (a line just typed or pasted, before the reindex): the name under
//! the cursor is resolved the way completion resolves it — a method through its receiver's inferred type, a path
//! from a crate root through the index, a bare name through the one item of that name in the file's crate — and
//! shown as the index's hover for that definition.

use super::hover::{format_hover_full, HoverCursor};
use crate::lsp::protocol::PositionConverter;
use crate::overlay::{DocumentOverlay, OverlayManager};
use ridx_format::{DefKind, ResolvedDefinition, WorkspaceHover, WorkspaceIndex};
use serde_json::Value;

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The hover of the name at the 1-based `line`/`col` of the dirty buffer `overlay` of `file`, from its definition.
pub(super) fn hover_unindexed(
    ws: &WorkspaceIndex,
    file: &str,
    overlay: &DocumentOverlay,
    line: u32,
    col: u32,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Option<Value> {
    let text: Vec<char> = overlay.text.lines().nth(line.saturating_sub(1) as usize)?.chars().collect();
    let at = (col.saturating_sub(1) as usize).min(text.len());
    let mut s = at;
    while s > 0 && is_ident(text[s - 1]) {
        s -= 1;
    }
    let mut e = at;
    while e < text.len() && is_ident(text[e]) {
        e += 1;
    }
    if s == e || text[s].is_ascii_digit() {
        return None;
    }
    let name: String = text[s..e].iter().collect();
    let def = if s > 0 && text[s - 1] == '.' && (s < 2 || text[s - 2] != '.') {
        method_def(ws, file, overlay, line, e as u32 + 1, &name, overlay_mgr)
    } else {
        path_def(ws, file, &text[..s], &name)
    }?;
    let hover = WorkspaceHover { definition: Some(def), inferred_type: None };
    Some(format_hover_full(&hover, Some(ws), Some(overlay_mgr), Some(conv), Some(file), HoverCursor::default()))
}

/// The method `name` of the receiver before the `.`, from the receiver's inferred type.
fn method_def<'a>(
    ws: &'a WorkspaceIndex,
    file: &str,
    overlay: &DocumentOverlay,
    line: u32,
    end_col: u32,
    name: &str,
    overlay_mgr: &OverlayManager,
) -> Option<ResolvedDefinition<'a>> {
    let off = overlay.line_char_col_to_byte(line, end_col) as u32;
    let db = crate::lsp::navigation::live_db_for(ws);
    let mut db = db.lock().unwrap_or_else(|e| e.into_inner());
    let cand = db.dot_completion(ws, file, off, Some(overlay_mgr)).cands.into_iter().find(|c| c.label == name)?;
    let loc = cand.def_location?;
    ws.find_definition_at(loc.file.to_str()?, loc.line, loc.col).filter(|d| d.name.rsplit("::").next() == Some(name))
}

/// The item a path ending in `name` names: `std::fs::metadata` and `crate::a::b` through the index, a bare name
/// through the one item so named in the file's crate.
fn path_def<'a>(ws: &'a WorkspaceIndex, file: &str, before: &[char], name: &str) -> Option<ResolvedDefinition<'a>> {
    let mut segs: Vec<String> = Vec::new();
    let mut i = before.len();
    while i >= 2 && before[i - 1] == ':' && before[i - 2] == ':' {
        let mut j = i - 2;
        while j > 0 && is_ident(before[j - 1]) {
            j -= 1;
        }
        if j == i - 2 {
            return None; // generics or a leading `::`
        }
        segs.push(before[j..i - 2].iter().collect());
        i = j;
    }
    segs.reverse();
    if segs.is_empty() {
        let krate = ws.crate_name_for_file(file);
        let mut found = ws.exact_symbols_by_name(name).into_iter().filter(|s| !matches!(s.kind, DefKind::Local | DefKind::Field) && krate.is_some_and(|k| s.crate_name == k));
        let only = found.next()?;
        return found.next().is_none().then(|| ws.find_definition_by_path(only.path)).flatten();
    }
    let path = format!("{}::{name}", segs.join("::"));
    match segs[0].as_str() {
        "std" | "core" | "alloc" => ws.find_definition_by_path(&path),
        "crate" => ws.find_definition_by_path(path.trim_start_matches("crate::")),
        _ => None,
    }
}
