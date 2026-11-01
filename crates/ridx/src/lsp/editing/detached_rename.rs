/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Single-file and trait-linked rename fallback for standalone Cargo targets (`examples/`, `build.rs`)
//! that are not part of the precompiled `.ridx` shard set.

use super::ident::find_declaration_ident_in_lines;
use crate::lsp::protocol::{path_to_uri, PositionConverter};
use crate::overlay::{find_document_matching_identifiers, DocumentOverlay, OverlayManager};
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

fn is_standalone_cargo_target(ws: &WorkspaceIndex, file: &str) -> bool {
    !ws.is_external_path(Path::new(file)) && ws.resolved_reader_file_ids(file).is_empty() && file.ends_with(".rs")
}

fn find_enclosing_trait_impl_short_name(lines: &[&str], decl_line_1based: u32) -> Option<String> {
    let start_idx = decl_line_1based.saturating_sub(1) as usize;
    for idx in (start_idx.saturating_sub(15)..=start_idx).rev() {
        let line = lines.get(idx)?.trim();
        if idx < start_idx && line.starts_with('}') {
            break;
        }
        if let Some(rest) = line.strip_prefix("impl") {
            let (before_for, _) = rest.split_once(" for ")?;
            let mut tr = before_for.trim();
            if tr.starts_with('<') {
                let mut depth = 0i32;
                let mut split_at = None;
                for (b_idx, ch) in tr.char_indices() {
                    if ch == '<' {
                        depth += 1;
                    } else if ch == '>' {
                        depth -= 1;
                        if depth == 0 {
                            split_at = Some(b_idx + 1);
                            break;
                        }
                    }
                }
                if let Some(pos) = split_at {
                    tr = tr[pos..].trim();
                }
            }
            let base = tr.split('<').next().unwrap_or(tr).trim();
            let short = base.rsplit("::").next().unwrap_or(base).trim();
            if !short.is_empty() {
                return Some(short.to_string());
            }
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub(super) fn collect_detached_rename_edits(
    ws: &WorkspaceIndex,
    file: &str,
    line_1based: u32,
    col_1based: u32,
    new_name: &str,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
    changes_map: &mut HashMap<String, Vec<Value>>,
) -> Option<(String, u32, u32)> {
    if !is_standalone_cargo_target(ws, file) {
        return None;
    }
    let overlay_opt = overlay_mgr.get(file);
    let text = if let Some(ov) = overlay_opt { ov.text.clone() } else { std::fs::read_to_string(file).ok()? };

    let ov = DocumentOverlay::parse_detached(text);
    let target_def = ov.find_definition_at(line_1based, col_1based)?;
    let lines: Vec<&str> = ov.text.lines().collect();
    let (decl_line, decl_col) = find_declaration_ident_in_lines(&lines, target_def.start_line, &target_def.name, target_def.kind)?;

    let mut delegated_trait_pos = None;
    if matches!(target_def.kind, DefKind::Fn | DefKind::AssocFn) {
        if let Some(trait_short) = find_enclosing_trait_impl_short_name(&lines, decl_line) {
            let suffix = format!("{trait_short}::{}", target_def.name);
            if let Some(sym) = ws.find_first_symbol(&target_def.name, |s| {
                s.name == target_def.name && matches!(s.kind, DefKind::AssocFn | DefKind::AssocTy | DefKind::AssocConst) && s.path.ends_with(&suffix)
            }) {
                let sym_path = ws.resolve_path_for_crate(sym.crate_name, sym.file);
                if let Some(sym_file) = sym_path.to_str() {
                    delegated_trait_pos = Some((sym_file.to_string(), sym.span.start_line(), sym.span.start_col()));
                }
            }
        }
    }

    let enclosing_mod = ov
        .document_symbols()
        .iter()
        .filter(|s| s.kind == DefKind::Mod && target_def.start_line >= s.start_line && target_def.end_line <= s.end_line)
        .min_by_key(|s| s.end_line.saturating_sub(s.start_line));

    let doc_uri = path_to_uri(file);
    let name_len = target_def.name.chars().count() as u32;

    for (l, c) in find_document_matching_identifiers(&ov.text, &target_def.name) {
        let is_decl = l == decl_line && c == decl_col;
        let matches_target = if is_decl {
            true
        } else if let Some(m) = enclosing_mod {
            if l >= m.start_line && l <= m.end_line {
                ov.find_definition_at(l, c).is_some_and(|d| d.start_line == target_def.start_line && d.start_col == target_def.start_col)
                    || ov.get_path_prefix_at(l, c).as_deref() == Some("self")
            } else {
                ov.get_path_prefix_at(l, c).as_deref().is_some_and(|p| p == m.name || p.ends_with(&format!("::{}", m.name)))
            }
        } else {
            let resolved_same = ov.find_definition_at(l, c).is_some_and(|d| d.start_line == target_def.start_line && d.start_col == target_def.start_col);
            let type_qual = matches!(target_def.kind, DefKind::Struct | DefKind::Enum | DefKind::Trait | DefKind::TyAlias) && ov.get_path_prefix_at(l, c).is_none();
            let self_assoc = ov.get_path_prefix_at(l, c).as_deref() == Some("Self");
            resolved_same || type_qual || self_assoc
        };

        if matches_target {
            let sc = overlay_opt.map(|o| o.char_col_to_utf16_col(l, c)).unwrap_or_else(|| conv.char_col_to_utf16_col(file, l, c));
            let ec = overlay_opt.map(|o| o.char_col_to_utf16_col(l, c + name_len)).unwrap_or_else(|| conv.char_col_to_utf16_col(file, l, c + name_len));
            let sl = l.saturating_sub(1);
            let edit = json!({
                "range": {
                    "start": { "line": sl, "character": sc },
                    "end": { "line": sl, "character": ec }
                },
                "newText": new_name
            });
            let edits = changes_map.entry(doc_uri.clone()).or_default();
            if !edits.iter().any(|e| e["range"] == edit["range"]) {
                edits.push(edit);
            }
        }
    }

    delegated_trait_pos
}
