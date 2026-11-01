/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `Self::Assoc`, `T::Assoc`, variant reference, and re-export prefix resolution helpers.

use super::cursor_path::{is_ident_byte, strip_generics_and_refs};
use super::WorkspaceIndex;
use crate::format::DefKind;
use crate::reader::ResolvedDefinition;

fn collect_param_bounds_in_line(code: &str, pat: &str, bounds: &mut Vec<String>) {
    for (idx, _) in code.match_indices(pat) {
        if idx > 0 && is_ident_byte(code.as_bytes()[idx - 1]) {
            continue;
        }
        let after = &code[idx + pat.len()..];
        if after.starts_with(':') {
            continue;
        }
        let stop = after.find(['>', ',', '{', '=', ';', ')']).unwrap_or(after.len());
        for part in after[..stop].split('+') {
            let b = strip_generics_and_refs(part.trim().trim_start_matches('?'));
            if !b.is_empty() && !b.starts_with('\'') && !matches!(b, "Sized" | "Send" | "Sync" | "Unpin" | "Copy" | "Clone") && !bounds.iter().any(|x| x == b) {
                bounds.push(b.to_string());
            }
        }
    }
}

fn strip_item_modifiers(mut s: &str) -> &str {
    s = s.trim_start();
    if let Some(rest) = s.strip_prefix("pub") {
        let mut r = rest.trim_start();
        if r.starts_with('(') {
            if let Some(close) = r.find(')') {
                r = r[close + 1..].trim_start();
            }
        }
        s = r;
    }
    for kw in ["default ", "const ", "async ", "unsafe "] {
        if let Some(r) = s.strip_prefix(kw) {
            s = r.trim_start();
        }
    }
    s
}

pub(super) fn scan_type_param_bounds(lines: &[&str], line_idx: usize, param: &str) -> Vec<String> {
    let mut bounds = Vec::new();
    if line_idx >= lines.len() {
        return bounds;
    }
    let pat = format!("{param}:");
    let end = (line_idx + 15).min(lines.len().saturating_sub(1));
    for &ln in &lines[line_idx..=end] {
        let code = ln.split("//").next().unwrap_or(ln);
        collect_param_bounds_in_line(code, &pat, &mut bounds);
        if code.contains(['{', ';', '}']) {
            break;
        }
    }
    let cur_code = lines[line_idx].split("//").next().unwrap_or(lines[line_idx]);
    let cur_item = strip_item_modifiers(cur_code);
    if cur_item.starts_with("fn ") && !bounds.is_empty() {
        return bounds;
    }
    let mut in_header = !cur_code.contains(['{', ';', '}']) && !(cur_item.starts_with("fn ") || cur_item.starts_with("impl") || cur_item.starts_with("trait "));
    let mut brace_depth = 0i32;
    let mut min_depth = 0i32;
    let start = line_idx.saturating_sub(250);
    for idx_ln in (start..line_idx).rev() {
        let code = lines[idx_ln].split("//").next().unwrap_or(lines[idx_ln]);
        for ch in code.chars().rev() {
            match ch {
                '}' => {
                    brace_depth += 1;
                    in_header = false;
                }
                ';' if brace_depth == min_depth => {
                    in_header = false;
                }
                '{' => {
                    brace_depth -= 1;
                    if brace_depth < min_depth {
                        min_depth = brace_depth;
                        in_header = true;
                    }
                }
                _ => {}
            }
        }
        if in_header {
            collect_param_bounds_in_line(code, &pat, &mut bounds);
            let item_s = strip_item_modifiers(code);
            if item_s.starts_with("fn ") {
                in_header = false;
                if !bounds.is_empty() {
                    break;
                }
            } else if item_s.starts_with("impl") || item_s.starts_with("trait ") {
                in_header = false;
            }
        }
    }
    bounds
}

pub(super) fn scan_enclosing_impl_or_trait_headers(lines: &[&str], line_idx: usize) -> Vec<String> {
    let mut out = Vec::new();
    let start = line_idx.saturating_sub(250);
    for &ln in lines[start..=line_idx].iter().rev() {
        let code = ln.split("//").next().unwrap_or(ln).trim();
        let rest_opt = code
            .strip_prefix("unsafe ")
            .unwrap_or(code)
            .strip_prefix("impl")
            .or_else(|| code.strip_prefix("pub trait "))
            .or_else(|| code.strip_prefix("trait "))
            .or_else(|| code.strip_prefix("pub(crate) trait "));
        let Some(mut r) = rest_opt else {
            continue;
        };
        r = r.trim_start();
        if r.starts_with('<') {
            let mut depth = 0i32;
            let mut end_idx = 0;
            for (idx, ch) in r.char_indices() {
                match ch {
                    '<' => depth += 1,
                    '>' => {
                        depth -= 1;
                        if depth == 0 {
                            end_idx = idx + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if end_idx > 0 {
                r = r[end_idx..].trim_start();
            }
        }
        let header = r.split('{').next().unwrap_or(r).trim();
        let header = header.split(" where ").next().unwrap_or(header).trim();
        for part in header.split(" for ") {
            let ty = strip_generics_and_refs(part.trim().trim_start_matches('!'));
            if !ty.is_empty() && !out.iter().any(|x| x == ty) {
                out.push(ty.to_string());
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    out
}

impl WorkspaceIndex {
    pub(crate) fn resolve_self_assoc_at<'a>(&'a self, file: &str, line: u32, _col: u32, lines: &[&str], line_idx: usize, ident: &str) -> Option<ResolvedDefinition<'a>> {
        for i in self.candidate_reader_indices_for_file(file) {
            let reader = &self.readers[i];
            let Some(tf) = self.target_file_for_reader(i, file) else {
                continue;
            };
            let Some(fid) = reader.find_file_id(tf.as_ref()) else {
                continue;
            };
            let cname = reader.crate_name();
            if let Some(d) = self.find_enclosing_impl_or_trait_def(i, fid, line) {
                let d_kind = DefKind::from(d.kind);
                let d_path = reader.get_str(d.path_offset());
                let owner_opt = if d_kind == DefKind::Trait {
                    Some(d_path)
                } else if matches!(d_kind, DefKind::AssocFn | DefKind::AssocTy | DefKind::AssocConst) {
                    d_path.rsplit_once("::").map(|(p, _)| p)
                } else if d_kind == DefKind::Impl {
                    let slice = self.unit_match.defs_in_file_up_to(i, fid, d.span().end_line());
                    slice.iter().find_map(|&(_, _, idx)| {
                        let cd = reader.definitions().get(idx as usize)?;
                        if cd.span().start_line() >= d.span().start_line() && matches!(DefKind::from(cd.kind), DefKind::AssocFn | DefKind::AssocTy | DefKind::AssocConst) {
                            reader.get_str(cd.path_offset()).rsplit_once("::").map(|(p, _)| p)
                        } else {
                            None
                        }
                    })
                } else {
                    None
                };

                if let Some(owner) = owner_opt {
                    let is_qual_impl = owner.starts_with('<') || owner.contains("::<impl ") || owner.contains(" as ");
                    let cands: Vec<&str> = if is_qual_impl {
                        owner.find('<').and_then(|idx| owner[idx + 1..].strip_suffix('>')).map_or_else(
                            || vec![owner],
                            |inner| {
                                if let Some(imp) = inner.strip_prefix("impl ") {
                                    imp.split(" for ").collect()
                                } else {
                                    inner.split(" as ").collect::<Vec<_>>().into_iter().rev().collect()
                                }
                            },
                        )
                    } else {
                        vec![strip_generics_and_refs(owner)]
                    };
                    for raw_ty in cands {
                        let ty = strip_generics_and_refs(raw_ty);
                        if let Some(def) = self.resolve_owner_member(file, Some(i), cname, ty, ident) {
                            return Some(def);
                        }
                    }
                }
            }
            for header_ty in scan_enclosing_impl_or_trait_headers(lines, line_idx) {
                if let Some(def) = self.resolve_owner_member(file, Some(i), cname, &header_ty, ident) {
                    return Some(def);
                }
            }
        }
        None
    }

    pub(crate) fn find_variant_by_owner_suffix<'a>(&'a self, preferred_reader: Option<usize>, owner_name: &str, variant_name: &str) -> Option<ResolvedDefinition<'a>> {
        if owner_name.is_empty() || !variant_name.as_bytes().first().is_some_and(|b| b.is_ascii_uppercase()) {
            return None;
        }
        let exact = format!("{owner_name}::{variant_name}");
        let suffix = format!("::{owner_name}::{variant_name}");
        for require_line in [true, false] {
            for i in 0..self.readers.len() {
                let reader = &self.readers[i];
                for r in reader.find_references(&exact) {
                    if matches!(DefKind::from(r.kind), DefKind::Variant | DefKind::VariantCtor) && (!require_line || r.target_line() > 0) {
                        return Some(self.finalize_reader_resolution(i, reader.resolve_ref_definition(r)));
                    }
                }
            }
            if let Some(i) = preferred_reader {
                if let Some(reader) = self.readers.get(i).filter(|r| r.references().len() <= 100_000) {
                    for r in reader.references() {
                        if !matches!(DefKind::from(r.kind), DefKind::Variant | DefKind::VariantCtor) || (require_line && r.target_line() == 0) {
                            continue;
                        }
                        let tp = reader.get_str(r.target_path_offset());
                        if tp == exact || tp.ends_with(&suffix) {
                            return Some(self.finalize_reader_resolution(i, reader.resolve_ref_definition(r)));
                        }
                    }
                }
            }
        }
        None
    }

    fn resolve_owner_member<'a>(&'a self, file: &str, reader_idx: Option<usize>, cname: &str, ty: &str, ident: &str) -> Option<ResolvedDefinition<'a>> {
        if ty.is_empty() {
            return None;
        }
        let has_cname = !cname.is_empty() && !ty.starts_with(&format!("{cname}::"));
        let try_direct =
            || self.find_definition_by_path(&format!("{ty}::{ident}")).or_else(|| has_cname.then(|| self.find_definition_by_path(&format!("{cname}::{ty}::{ident}"))).flatten());
        if ty.contains("::") {
            if let Some(def) = try_direct() {
                return Some(def);
            }
        }
        if let Some(base) = self
            .resolve_qualifier(file, ty)
            .or_else(|| self.find_definition_by_path(ty))
            .or_else(|| has_cname.then(|| self.find_definition_by_path(&format!("{cname}::{ty}"))).flatten())
        {
            if let Some(def) = self.find_definition_by_path(&format!("{}::{ident}", base.path)) {
                return Some(def);
            }
            if let Some(def) = self.find_definitions_by_owner(base.path).into_iter().find(|d| d.name == ident) {
                return Some(def);
            }
            for sup in self.resolve_deref_chain(base.path, 4) {
                if let Some(def) = self.find_definition_by_path(&format!("{sup}::{ident}")) {
                    return Some(def);
                }
            }
            if let Some(def) = self.find_variant_by_owner_suffix(reader_idx.or(base.reader_idx), base.name, ident) {
                return Some(def);
            }
        }
        if !ty.contains("::") {
            if let Some(def) = try_direct() {
                return Some(def);
            }
        }
        let short = ty.rsplit("::").next().unwrap_or(ty);
        self.find_variant_by_owner_suffix(reader_idx, short, ident)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn resolve_type_relative_at<'a>(
        &'a self,
        file: &str,
        line: u32,
        col: u32,
        lines: &[&str],
        line_idx: usize,
        prefix: &str,
        ident: &str,
    ) -> Option<ResolvedDefinition<'a>> {
        let mut bounds = scan_type_param_bounds(lines, line_idx, prefix);
        for i in self.reader_indices_for_file(file) {
            let reader = &self.readers[i];
            let Some(tf) = self.target_file_for_reader(i, file) else {
                continue;
            };
            let Some(fid) = reader.find_file_id(tf.as_ref()) else {
                continue;
            };
            let defs = reader.definitions();
            let has_tp = self
                .unit_match
                .defs_in_file_up_to(i, fid, line)
                .iter()
                .any(|&(_, _, di)| defs.get(di as usize).is_some_and(|d| d.kind == DefKind::TyParam as u8 && reader.get_str(d.name_offset()) == prefix));
            if !has_tp {
                continue;
            }
            if let Some((tp_def, tp_idx)) = reader.find_type_param_in_scope(tf.as_ref(), prefix, line, col) {
                let tp_path = reader.get_str(tp_def.path_offset());
                for b in reader.find_owners_by_def_index(tp_idx) {
                    if b.starts_with("scope:") || b.contains('<') || tp_path.starts_with(&format!("{b}::")) {
                        continue;
                    }
                    let clean = strip_generics_and_refs(b);
                    if !clean.is_empty() && !bounds.iter().any(|x| x == clean) {
                        bounds.push(clean.to_string());
                    }
                }
                if tp_def.sig_offset() != 0 {
                    let sig = reader.get_str(tp_def.sig_offset());
                    let rhs = sig.split_once(':').map_or(sig, |(_, r)| r);
                    for part in rhs.split('+') {
                        let clean = strip_generics_and_refs(part.trim().trim_start_matches('?'));
                        if !clean.is_empty() && !bounds.iter().any(|x| x == clean) {
                            bounds.push(clean.to_string());
                        }
                    }
                }
            }
        }
        let pref = self.reader_index_for_file(file);
        let cname = self.crate_name_for_file(file).unwrap_or("");
        for b in &bounds {
            if let Some(def) = self.resolve_owner_member(file, pref, cname, b, ident) {
                return Some(def);
            }
        }
        if bounds.is_empty() && prefix.len() > 1 && prefix.as_bytes()[0].is_ascii_uppercase() && is_ident_byte(prefix.as_bytes()[0]) {
            if let Some(def) = self.resolve_owner_member(file, pref, cname, prefix, ident) {
                return Some(def);
            }
        }
        None
    }
}
