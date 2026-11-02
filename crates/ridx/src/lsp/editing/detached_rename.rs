/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Single-file and trait-linked rename fallback for standalone unindexed files
//! that are not part of the precompiled `.ridx` shard set.

use super::ident::{extract_ident_at_col, find_declaration_ident_in_lines};
use crate::lsp::protocol::{path_to_uri, PositionConverter};
use crate::overlay::{find_document_matching_identifiers, DocumentOverlay, OverlayManager};
use ridx_format::{DefKind, RenameRefusal, WorkspaceIndex};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

pub(super) enum DetachedPrepareOutcome {
    DelegatedTraitItem(String, u32, u32),
    LocalPlaceholder { range: Value, placeholder: String },
}

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
                let (mut depth, mut split_at) = (0i32, None);
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

fn resolve_delegated_trait_pos(ws: &WorkspaceIndex, lines: &[&str], decl_line: u32, kind: DefKind, name: &str) -> Option<(String, u32, u32)> {
    if !matches!(kind, DefKind::Fn | DefKind::AssocFn) {
        return None;
    }
    let trait_short = find_enclosing_trait_impl_short_name(lines, decl_line)?;
    let suffix = format!("{trait_short}::{name}");
    let sym = ws.find_first_symbol(name, |s| s.name == name && matches!(s.kind, DefKind::AssocFn | DefKind::AssocTy | DefKind::AssocConst) && s.path.ends_with(&suffix))?;
    let sym_path = ws.resolve_path_for_crate(sym.crate_name, sym.file);
    sym_path.to_str().map(|f| (f.to_string(), sym.span.start_line(), sym.span.start_col()))
}

pub(super) fn try_prepare_detached_rename(
    ws: &WorkspaceIndex,
    file: &str,
    line_1based: u32,
    col_1based: u32,
    overlay_mgr: &OverlayManager,
    preloaded: Option<&str>,
    conv: &mut PositionConverter,
) -> Option<Result<DetachedPrepareOutcome, RenameRefusal>> {
    if !is_standalone_cargo_target(ws, file) {
        return None;
    }
    let overlay_opt = overlay_mgr.get(file);
    let text = if let Some(ov) = overlay_opt {
        ov.text.clone()
    } else if let Some(s) = preloaded {
        s.to_string()
    } else {
        std::fs::read_to_string(file).ok()?
    };
    let ov = DocumentOverlay::parse_detached(text);
    let target_def = ov.find_definition_at(line_1based, col_1based)?;
    let lines: Vec<&str> = ov.text.lines().collect();
    let (decl_line, _) = find_declaration_ident_in_lines(&lines, target_def.start_line, &target_def.name, target_def.kind)?;

    let enclosing_mod = ov
        .document_symbols()
        .iter()
        .filter(|s| s.kind == DefKind::Mod && target_def.start_line >= s.start_line && target_def.end_line <= s.end_line)
        .min_by_key(|s| s.end_line.saturating_sub(s.start_line));

    if matches!(target_def.kind, DefKind::Fn) && target_def.name == "main" && enclosing_mod.is_none() {
        if let Some((target_kind, target_name)) = ws.cargo_main_entrypoint(Path::new(file)) {
            return Some(Err(RenameRefusal::CargoMainEntrypoint { target_kind, target_name }));
        }
    }

    if let Some((tf, tl, tc)) = resolve_delegated_trait_pos(ws, &lines, decl_line, target_def.kind, &target_def.name) {
        return Some(Ok(DetachedPrepareOutcome::DelegatedTraitItem(tf, tl, tc)));
    }

    let cur_line = lines.get(line_1based.saturating_sub(1) as usize).copied().unwrap_or("");
    let placeholder = extract_ident_at_col(cur_line, col_1based).unwrap_or(target_def.name.as_str()).to_string();
    let ident_col = super::rename_special::extract_full_ident_around_col(cur_line, col_1based).map(|(c, _)| c).unwrap_or(col_1based);
    let name_len = placeholder.chars().count() as u32;
    let sc = overlay_opt.map(|o| o.char_col_to_utf16_col(line_1based, ident_col)).unwrap_or_else(|| conv.char_col_to_utf16_col(file, line_1based, ident_col));
    let ec = overlay_opt.map(|o| o.char_col_to_utf16_col(line_1based, ident_col + name_len)).unwrap_or_else(|| conv.char_col_to_utf16_col(file, line_1based, ident_col + name_len));
    let l0 = line_1based.saturating_sub(1);
    Some(Ok(DetachedPrepareOutcome::LocalPlaceholder { range: json!({ "start": { "line": l0, "character": sc }, "end": { "line": l0, "character": ec } }), placeholder }))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn collect_detached_rename_edits(
    ws: &WorkspaceIndex,
    file: &str,
    line_1based: u32,
    col_1based: u32,
    new_name: &str,
    overlay_mgr: &OverlayManager,
    preloaded: Option<&str>,
    conv: &mut PositionConverter,
    changes_map: &mut HashMap<String, Vec<Value>>,
) -> Result<Option<(String, u32, u32)>, RenameRefusal> {
    if !is_standalone_cargo_target(ws, file) {
        return Ok(None);
    }
    let overlay_opt = overlay_mgr.get(file);
    let text = if let Some(ov) = overlay_opt {
        ov.text.clone()
    } else if let Some(s) = preloaded {
        s.to_string()
    } else {
        let Ok(t) = std::fs::read_to_string(file) else { return Ok(None) };
        t
    };

    let ov = DocumentOverlay::parse_detached(text);
    let Some(target_def) = ov.find_definition_at(line_1based, col_1based) else { return Ok(None) };
    let lines: Vec<&str> = ov.text.lines().collect();
    let Some((decl_line, decl_col)) = find_declaration_ident_in_lines(&lines, target_def.start_line, &target_def.name, target_def.kind) else {
        return Ok(None);
    };

    let enclosing_mod = ov
        .document_symbols()
        .iter()
        .filter(|s| s.kind == DefKind::Mod && target_def.start_line >= s.start_line && target_def.end_line <= s.end_line)
        .min_by_key(|s| s.end_line.saturating_sub(s.start_line));

    if matches!(target_def.kind, DefKind::Fn) && target_def.name == "main" && enclosing_mod.is_none() {
        if let Some((target_kind, target_name)) = ws.cargo_main_entrypoint(Path::new(file)) {
            return Err(RenameRefusal::CargoMainEntrypoint { target_kind, target_name });
        }
    }

    let delegated_trait_pos = resolve_delegated_trait_pos(ws, &lines, decl_line, target_def.kind, &target_def.name);
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
                "range": { "start": { "line": sl, "character": sc }, "end": { "line": sl, "character": ec } },
                "newText": new_name
            });
            let edits = changes_map.entry(doc_uri.clone()).or_default();
            if !edits.iter().any(|e| e["range"] == edit["range"]) {
                edits.push(edit);
            }
        }
    }

    if enclosing_mod.is_none() {
        let p = Path::new(file);
        if let (Some(parent), Some(stem)) = (p.parent(), p.file_stem().and_then(|s| s.to_str())) {
            if !matches!(stem, "lib" | "main" | "mod") {
                let mod_pat = format!("{stem}::");
                for sib in ["lib.rs", "mod.rs"] {
                    let sib_path = parent.join(sib);
                    let Some(sib_str) = sib_path.to_str() else { continue };
                    let Ok(sib_text) = std::fs::read_to_string(&sib_path) else { continue };
                    let sib_uri = path_to_uri(sib_str);
                    for (idx, ln) in sib_text.lines().enumerate() {
                        if !ln.contains("use ") || !ln.contains(&mod_pat) {
                            continue;
                        }
                        let ln_1 = (idx + 1) as u32;
                        for (b_pos, _) in ln.match_indices(&target_def.name) {
                            let end_b = b_pos + target_def.name.len();
                            let prev_ok = b_pos == 0 || !super::ident::is_ident_continue(ln[..b_pos].chars().next_back().unwrap_or(' '));
                            let next_ok = end_b >= ln.len() || !super::ident::is_ident_continue(ln[end_b..].chars().next().unwrap_or(' '));
                            if prev_ok && next_ok {
                                let col_1 = ln[..b_pos].chars().count() as u32 + 1;
                                let sc = conv.char_col_to_utf16_col(sib_str, ln_1, col_1);
                                let ec = conv.char_col_to_utf16_col(sib_str, ln_1, col_1 + name_len);
                                let edit = json!({
                                    "range": { "start": { "line": ln_1 - 1, "character": sc }, "end": { "line": ln_1 - 1, "character": ec } },
                                    "newText": new_name
                                });
                                let edits = changes_map.entry(sib_uri.clone()).or_default();
                                if !edits.iter().any(|e| e["range"] == edit["range"]) {
                                    edits.push(edit);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(delegated_trait_pos)
}
