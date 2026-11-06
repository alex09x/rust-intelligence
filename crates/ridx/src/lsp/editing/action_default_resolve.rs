/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Same-module and `WorkspaceIndex`-backed trait definition lookup for `Implement default members`.

use super::action_default_scope::enclosing_module_span;
use super::action_generics_sub::{parse_generics_list, split_top_level_commas, GenericDecl};
use super::action_parse::{byte_to_line0, is_ident_byte, line_starts_of, mask_comments_and_strings};
use super::action_parse_decl::{extract_angle_generics, find_header_terminator, next_ident_token};
use super::action_parse_items::skip_balanced;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use std::path::{Path, PathBuf};

pub struct ResolvedTraitSource {
    pub src: String,
    pub masked: String,
    pub same_module: bool,
    pub same_crate: bool,
    pub generics: Vec<GenericDecl>,
    pub body_open: usize,
    pub body_close: usize,
    /// The trait's path in the index, when the index resolved it: its compiled members are `<def_path>::<name>`.
    pub def_path: Option<String>,
    /// The file holding the trait, when the index resolved it.
    pub file: Option<String>,
}

/// The default of each generic parameter of `trait name<..>` declared last in `header` (`None` for a parameter
/// without one): `trait PartialOrd<Rhs: ?Sized = Self>` gives `[Some("Self")]`.
pub fn trait_generic_defaults(header: &str, name: &str) -> Vec<Option<String>> {
    let Some(at) = header.rfind(&format!("trait {name}")) else { return Vec::new() };
    let rest = header[at + 6 + name.len()..].trim_start();
    if !rest.starts_with('<') {
        return Vec::new();
    }
    let mut depth = 0i32;
    let mut close = None;
    for (i, c) in rest.char_indices() {
        match c {
            '<' => depth += 1,
            '>' if rest[..i].ends_with('-') || rest[..i].ends_with('=') => {}
            '>' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(close) = close else { return Vec::new() };
    split_top_level_commas(&rest[1..close])
        .iter()
        .map(|p| {
            let b = p.as_bytes();
            let mut d = 0i32;
            for (i, &c) in b.iter().enumerate() {
                match c {
                    b'<' | b'(' | b'[' => d += 1,
                    b'>' | b')' | b']' => d -= 1,
                    b'=' if d == 0 && b.get(i + 1) != Some(&b'>') => return Some(p[i + 1..].trim().to_string()),
                    _ => {}
                }
            }
            None
        })
        .collect()
}

pub fn split_trait_path_and_args(trait_ty: &str) -> Option<(&str, Vec<String>)> {
    let masked = mask_comments_and_strings(trait_ty);
    let mb = masked.as_bytes();
    let mut angle_open = None;
    let mut angle = 0i32;
    for i in 0..mb.len() {
        let prev = if i > 0 { mb[i - 1] } else { 0 };
        if mb[i] == b'<' {
            if angle == 0 {
                angle_open = Some(i);
            }
            angle += 1;
        } else if mb[i] == b'>' && prev != b'-' && prev != b'=' {
            angle -= 1;
            if angle == 0 && i + 1 < mb.len() && !mb[i + 1..].iter().all(|c| c.is_ascii_whitespace()) {
                return None;
            }
        }
    }
    if let Some(ao) = angle_open {
        let end_gt = trait_ty.rfind('>')?;
        let args = split_top_level_commas(&trait_ty[ao + 1..end_gt]);
        Some((trait_ty[..ao].trim(), args))
    } else {
        Some((trait_ty.trim(), Vec::new()))
    }
}

/// The trait an impl header names, as the index resolved it: the last word-bounded `simple` before the impl body
/// that the index maps to a trait definition.
#[allow(clippy::too_many_arguments)]
fn trait_from_index(
    doc_text: &str,
    masked_doc: &str,
    current_file: &str,
    impl_body_open: usize,
    simple: &str,
    ws: &WorkspaceIndex,
    overlay_mgr: &OverlayManager,
    impl_mod: (usize, usize),
) -> Option<ResolvedTraitSource> {
    let header_start = masked_doc[..impl_body_open].rfind("impl")?;
    let b = masked_doc.as_bytes();
    let at = masked_doc[header_start..impl_body_open]
        .match_indices(simple)
        .map(|(p, _)| header_start + p)
        .find(|&p| !is_ident_byte(b[p - 1]) && b.get(p + simple.len()).is_none_or(|c| !is_ident_byte(*c)))?;
    let line_start = doc_text[..at].rfind('\n').map_or(0, |p| p + 1);
    let line = doc_text[..line_start].matches('\n').count() as u32 + 1;
    let col = doc_text[line_start..at].chars().count() as u32 + 1;
    let refs = ws.find_direct_references_at(current_file, line, col)?;
    // A derive macro shares its trait's name and file (`PartialOrd`); in an impl header the name is the trait.
    let via_macro = refs.target_kind == DefKind::Macro;
    if refs.target_kind != DefKind::Trait && !via_macro {
        return None;
    }
    let mut target = refs.target_resolved_path();
    // A standard library trait is recorded as `library/…`: its source is the toolchain's `rust-src`.
    if !target.exists() {
        let s = target.to_string_lossy().into_owned();
        if let Some(at) = s.find("library/").filter(|&at| at == 0 || s[..at].ends_with('/')) {
            target = crate::lsp::navigation::type_definition_parse::sysroot_rust_src()?.join(&s[at..]);
        }
    }
    let path_str = target.to_str()?;
    let trait_src = if let Some(ov) = overlay_mgr.get(path_str) { ov.text.clone() } else { std::fs::read_to_string(&target).ok()? };
    let trait_masked = mask_comments_and_strings(&trait_src);
    let (generics, bo, bc) =
        find_trait_in_range(&trait_src, &trait_masked, 0, trait_masked.len(), simple_or_target(simple, refs.target_symbol), (!via_macro).then_some(refs.target_line))?;
    let same_module = path_str == current_file && enclosing_module_span(&trait_masked, bo) == impl_mod;
    let same_crate = same_module || same_cargo_manifest_dir(Path::new(current_file), &target);
    Some(ResolvedTraitSource {
        src: trait_src,
        masked: trait_masked,
        same_module,
        same_crate,
        generics,
        body_open: bo,
        body_close: bc,
        def_path: Some(refs.target_path.to_string()),
        file: Some(path_str.to_string()),
    })
}

/// The trait's own name: an impl may name it through a renaming import.
fn simple_or_target<'a>(simple: &'a str, target: &'a str) -> &'a str {
    if target.is_empty() {
        simple
    } else {
        target
    }
}

#[allow(clippy::too_many_arguments)]
pub fn resolve_trait_definition(
    doc_text: &str,
    masked_doc: &str,
    current_file: &str,
    impl_body_open: usize,
    trait_path: &str,
    simple_clean: &str,
    ws: &WorkspaceIndex,
    overlay_mgr: &OverlayManager,
) -> Option<ResolvedTraitSource> {
    let impl_mod = enclosing_module_span(masked_doc, impl_body_open);
    if !trait_path.contains("::") || trait_path.starts_with("self::") {
        if let Some((generics, bo, bc)) = find_trait_in_range(doc_text, masked_doc, impl_mod.0, impl_mod.1, simple_clean, None) {
            return Some(ResolvedTraitSource {
                src: doc_text.to_string(),
                masked: masked_doc.to_string(),
                same_module: true,
                same_crate: true,
                generics,
                body_open: bo,
                body_close: bc,
                def_path: None,
                file: None,
            });
        }
    }
    if impl_mod != (0, masked_doc.len()) && (!trait_path.contains("::") || trait_path.starts_with("super::") || trait_path.starts_with("crate::")) {
        if let Some((generics, bo, bc)) = find_trait_in_range(doc_text, masked_doc, 0, masked_doc.len(), simple_clean, None) {
            let same_module = enclosing_module_span(masked_doc, bo) == impl_mod;
            return Some(ResolvedTraitSource {
                src: doc_text.to_string(),
                masked: masked_doc.to_string(),
                same_module,
                same_crate: true,
                generics,
                body_open: bo,
                body_close: bc,
                def_path: None,
                file: None,
            });
        }
    }

    // The index resolved the trait name in the impl header: follow it, re-exports included.
    if let Some(found) = trait_from_index(doc_text, masked_doc, current_file, impl_body_open, simple_clean, ws, overlay_mgr, impl_mod) {
        return Some(found);
    }

    let trimmed_tp = trait_path.trim_start_matches("::");
    let is_local_rel = trimmed_tp.starts_with("crate::") || trimmed_tp.starts_with("super::") || trimmed_tp.starts_with("self::");
    let (explicit_crate, suffix_hint) = split_root_and_suffix(trimmed_tp);
    let use_import = if !trimmed_tp.contains("::") { find_imported_path_for(&masked_doc[impl_mod.0..impl_mod.1], simple_clean) } else { None };
    let target_ident = use_import.as_ref().map(|u| u.orig_name.as_str()).unwrap_or(simple_clean);
    let eff_crate = explicit_crate.or_else(|| use_import.as_ref().and_then(|u| u.root_crate.clone()));
    let eff_suffix = if trimmed_tp.contains("::") { Some(suffix_hint) } else { use_import.as_ref().and_then(|u| u.suffix.clone()) };
    let force_same_crate = is_local_rel || use_import.as_ref().is_some_and(|u| u.is_local_rel);
    let cur_path = Path::new(current_file);

    let mut cands: Vec<(PathBuf, u32, bool, String)> = Vec::new();
    let local_syms = ws.exact_local_symbols_by_name(target_ident);
    let syms = if local_syms.iter().any(|s| s.kind == DefKind::Trait && !s.is_macro_expansion) { local_syms } else { ws.exact_symbols_by_name(target_ident) };
    for s in syms {
        if s.kind != DefKind::Trait || s.is_macro_expansion || s.name.strip_prefix("r#").unwrap_or(s.name) != target_ident {
            continue;
        }
        if let Some(ref sfx) = eff_suffix {
            if sfx.contains("::") && !s.path.ends_with(sfx.as_str()) {
                continue;
            }
        }
        let rp = s.resolved_path();
        let is_ext = ws.is_external_target(s.file, &rp, s.unit_key);
        if !cands.iter().any(|c| c.0 == rp && c.1 == s.span.start_line()) {
            cands.push((rp, s.span.start_line(), is_ext, s.crate_name.replace('-', "_")));
        }
    }

    if cands.iter().any(|c| !c.2) {
        cands.retain(|c| !c.2);
    }
    if force_same_crate {
        cands.retain(|c| same_cargo_manifest_dir(cur_path, &c.0));
    } else if let Some(ref cr) = eff_crate {
        let cr_norm = cr.replace('-', "_");
        if cands.iter().any(|c| c.3 == cr_norm) {
            cands.retain(|c| c.3 == cr_norm);
        }
    }
    if cands.len() > 1 {
        let local_only: Vec<_> = cands.iter().filter(|c| same_cargo_manifest_dir(cur_path, &c.0)).cloned().collect();
        if local_only.len() == 1 {
            cands = local_only;
        }
    }
    if cands.len() != 1 {
        return None;
    }
    let (target_path, line1, _, _) = &cands[0];
    let path_str = target_path.to_str()?;
    let trait_src = if let Some(ov) = overlay_mgr.get(path_str) { ov.text.clone() } else { std::fs::read_to_string(Path::new(path_str)).ok()? };
    let trait_masked = mask_comments_and_strings(&trait_src);
    let (generics, bo, bc) = find_trait_in_range(&trait_src, &trait_masked, 0, trait_masked.len(), target_ident, Some(*line1))?;
    let same_module = path_str == current_file && enclosing_module_span(&trait_masked, bo) == impl_mod;
    let same_crate = same_module || same_cargo_manifest_dir(cur_path, target_path);
    Some(ResolvedTraitSource { src: trait_src, masked: trait_masked, same_module, same_crate, generics, body_open: bo, body_close: bc, def_path: None, file: None })
}

struct UseImportHit {
    orig_name: String,
    root_crate: Option<String>,
    suffix: Option<String>,
    is_local_rel: bool,
}

fn split_root_and_suffix(p: &str) -> (Option<String>, String) {
    if let Some(r) = p.strip_prefix("crate::").or_else(|| p.strip_prefix("super::")).or_else(|| p.strip_prefix("self::")) {
        let clean = r.trim_start_matches("super::").trim_start_matches("self::");
        return (None, clean.to_string());
    }
    if let Some((first, rest)) = p.split_once("::") {
        let f = first.trim();
        if !f.is_empty() && !matches!(f, "std" | "core" | "alloc") {
            return (Some(f.to_string()), rest.trim().to_string());
        }
    }
    (None, p.to_string())
}

fn cargo_manifest_dir(p: &Path) -> Option<&Path> {
    let mut cur = p.parent();
    while let Some(dir) = cur {
        if dir.join("Cargo.toml").is_file() {
            return Some(dir);
        }
        cur = dir.parent();
    }
    None
}

fn same_cargo_manifest_dir(a: &Path, b: &Path) -> bool {
    match (cargo_manifest_dir(a), cargo_manifest_dir(b)) {
        (Some(da), Some(db)) => da == db,
        _ => false,
    }
}

fn find_imported_path_for(masked_mod: &str, ident: &str) -> Option<UseImportHit> {
    let mb = masked_mod.as_bytes();
    let mut depth = 0i32;
    let mut i = 0usize;
    while i + 4 < mb.len() {
        match mb[i] {
            b'{' => depth += 1,
            b'}' => depth = (depth - 1).max(0),
            _ if depth == 0 && &mb[i..i + 3] == b"use" && (i == 0 || !is_ident_byte(mb[i - 1])) && mb[i + 3].is_ascii_whitespace() => {
                let mut semi = i + 3;
                let mut b_d = 0i32;
                while semi < mb.len() {
                    match mb[semi] {
                        b'{' => b_d += 1,
                        b'}' => b_d = (b_d - 1).max(0),
                        b';' if b_d == 0 => break,
                        _ => {}
                    }
                    semi += 1;
                }
                if semi < mb.len() {
                    if let Some(full) = match_use_tree("", &masked_mod[i + 3..semi], ident) {
                        let clean = full.trim_start_matches("::");
                        let is_local_rel = clean.starts_with("crate::") || clean.starts_with("super::") || clean.starts_with("self::");
                        let (root_crate, sfx) = split_root_and_suffix(clean);
                        let orig_raw = sfx.rsplit("::").next().unwrap_or(&sfx).trim();
                        let orig_name = orig_raw.strip_prefix("r#").unwrap_or(orig_raw).to_string();
                        return Some(UseImportHit { orig_name, root_crate, suffix: Some(sfx), is_local_rel });
                    }
                    i = semi + 1;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn match_use_tree(prefix: &str, tree: &str, target_local: &str) -> Option<String> {
    for part in split_top_level_commas(tree) {
        let t = part.trim();
        if t.is_empty() || t.ends_with('*') {
            continue;
        }
        if let Some(open_b) = t.find('{') {
            if let Some(close_b) = t.rfind('}') {
                let sub_pfx = join_use_prefix(prefix, t[..open_b].trim().trim_end_matches("::"));
                if let Some(hit) = match_use_tree(&sub_pfx, &t[open_b + 1..close_b], target_local) {
                    return Some(hit);
                }
                continue;
            }
        }
        if let Some((orig, alias)) = t.rsplit_once(" as ") {
            let a_raw = alias.trim();
            let a = a_raw.strip_prefix("r#").unwrap_or(a_raw);
            if a == target_local {
                let o = orig.trim();
                return Some(if o == "self" { prefix.to_string() } else { join_use_prefix(prefix, o) });
            }
            continue;
        }
        let raw_leaf = t.rsplit("::").next().unwrap_or(t).trim();
        let leaf = raw_leaf.strip_prefix("r#").unwrap_or(raw_leaf);
        if leaf == "self" {
            let p_leaf = prefix.rsplit("::").next().unwrap_or(prefix).trim();
            if p_leaf.strip_prefix("r#").unwrap_or(p_leaf) == target_local {
                return Some(prefix.to_string());
            }
        } else if leaf == target_local {
            return Some(join_use_prefix(prefix, t));
        }
    }
    None
}

fn join_use_prefix(prefix: &str, rhs: &str) -> String {
    let p = prefix.trim().trim_end_matches("::");
    let r = rhs.trim().trim_start_matches("::");
    if p.is_empty() {
        r.to_string()
    } else if r.is_empty() {
        p.to_string()
    } else {
        format!("{p}::{r}")
    }
}

fn find_trait_in_range(src: &str, masked: &str, lo: usize, hi: usize, target_name: &str, line1_hint: Option<u32>) -> Option<(Vec<GenericDecl>, usize, usize)> {
    let mb = masked.as_bytes();
    let starts = line_starts_of(src);
    let mut matches_found = Vec::new();
    let mut brace_depth = 0i32;
    let mut i = lo;
    while i + 5 < hi {
        match mb[i] {
            b'{' => {
                brace_depth += 1;
                i += 1;
                continue;
            }
            b'}' => {
                brace_depth = (brace_depth - 1).max(0);
                i += 1;
                continue;
            }
            _ => {}
        }
        if (line1_hint.is_some() || brace_depth == 0) && &mb[i..i + 5] == b"trait" && (i == 0 || !is_ident_byte(mb[i - 1])) && !is_ident_byte(mb[i + 5]) {
            if let Some((raw_n, after_n)) = next_ident_token(src, i + 5) {
                if raw_n.strip_prefix("r#").unwrap_or(raw_n) == target_name {
                    let (gen_s, after_g) = extract_angle_generics(src, masked, after_n);
                    if let Some(term) = find_header_terminator(mb, after_g) {
                        if term < hi && mb[term] == b'{' {
                            if let Some(after_b) = skip_balanced(mb, term, b'{', b'}') {
                                let ln1 = byte_to_line0(&starts, i) + 1;
                                matches_found.push((ln1, parse_generics_list(&gen_s), term, after_b - 1));
                            }
                        }
                    }
                }
            }
        }
        i += 1;
    }
    if let Some(hint) = line1_hint {
        matches_found.into_iter().min_by_key(|m| m.0.abs_diff(hint)).filter(|m| m.0.abs_diff(hint) <= 5).map(|m| (m.1, m.2, m.3))
    } else if matches_found.len() == 1 {
        let m = matches_found.into_iter().next()?;
        Some((m.1, m.2, m.3))
    } else {
        None
    }
}
