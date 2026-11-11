/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A function body inlined into another file of its crate: the names it uses (types, modules, functions) are written
//! as the paths the index resolved them to in the function's own file (`crate::wtr::stdout_buffered_line`), and every
//! item, field and method it reaches must be visible from there (`pub` or `pub(crate)`); a trait's method also needs
//! the trait in scope at the call site.

use super::action_default_safety::is_rust_kw_or_prelude;
use super::action_parse::{is_ident_byte, mask_comments_and_strings};
use super::action_parse_decl::next_ident_token;
use super::action_parse_items::contains_word;
use ridx_format::{DefKind, ItemVis, WorkspaceIndex};
use std::collections::{HashMap, HashSet};

/// A name in `masked` at `[i, end)` that is written as a path when moved: a type (`Name`), a path head (`m::`), or a
/// called function (`f(`), not after `.`/`::` and not a macro.
fn is_candidate(masked: &str, i: usize, end: usize, tok: &str) -> bool {
    let before = masked[..i].trim_end();
    let after = masked[end..].trim_start();
    !before.ends_with("::")
        && !before.ends_with('.')
        && !after.starts_with('!')
        && !(after.starts_with(':') && !after.starts_with("::"))
        && (after.starts_with("::") || after.starts_with('(') || tok.starts_with(|c: char| c.is_ascii_uppercase()))
}

/// Identifier tokens of `masked`, as `(start, end)`.
pub(crate) fn tokens(masked: &str) -> Vec<(usize, usize)> {
    let b = masked.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        if (b[i].is_ascii_alphabetic() || b[i] == b'_') && (i == 0 || !is_ident_byte(b[i - 1]) && b[i - 1] != b'\'') {
            if let Some((_, end)) = next_ident_token(masked, i) {
                out.push((i, end));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// `(line, col)` (1-based) of byte `at` in `src`.
fn line_col(src: &str, at: usize) -> (u32, u32) {
    let line_start = src[..at].rfind('\n').map_or(0, |p| p + 1);
    (src[..line_start].matches('\n').count() as u32 + 1, src[line_start..at].chars().count() as u32 + 1)
}

/// Whether an item of `vis` defined in crate `def_crate` is reachable from another module of `dest_crate`.
fn reachable(vis: ItemVis, def_crate: &str, dest_crate: &str) -> bool {
    vis == ItemVis::Public || (vis == ItemVis::Crate && def_crate == dest_crate)
}

/// Narrows `within` (the crate-relative module a destination must lie in) to `module`: the deeper of two nested modules,
/// `None` when they are apart.
fn narrow(within: &mut Option<String>, module: String) -> Option<()> {
    let inside = |inner: &str, outer: &str| inner == outer || inner.starts_with(&format!("{outer}::"));
    match within.as_deref() {
        Some(w) if inside(w, &module) => {}
        Some(w) if !inside(&module, w) => return None,
        _ => *within = Some(module),
    }
    Some(())
}

/// The paths to write for the names of `body` (at byte `start` of `fn_src`, the text of `fn_file`) in another file of
/// `crate_name`, keyed by name, and the module a destination must lie in for private items on those paths to be
/// visible; `None` when a name resolves to something unreachable or to two items. Generic parameters get no path: a
/// body or parameter type that names one is refused when the paths are applied.
pub(crate) fn cross_file_paths(body: &str, fn_src: &str, fn_file: &str, start: usize, ws: &WorkspaceIndex, crate_name: &str, locals: &HashSet<String>) -> Option<CrossFilePaths> {
    let mut within: Option<String> = None;
    let mut ty_params: HashSet<String> = HashSet::new();
    let masked = mask_comments_and_strings(body);
    let pairs = ws.resolved_reader_file_ids(fn_file);
    let names: Vec<(usize, usize)> = tokens(&masked)
        .into_iter()
        .filter(|&(i, e)| {
            let tok = &masked[i..e];
            is_candidate(&masked, i, e, tok) && !is_rust_kw_or_prelude(tok) && !locals.contains(tok) && !matches!(tok, "self" | "Self" | "crate" | "this")
        })
        .collect();
    let positions: Vec<(u32, u32)> = names.iter().map(|&(i, _)| line_col(fn_src, start + i)).collect();
    let mut out: HashMap<String, String> = HashMap::new();
    for (&(i, e), def) in names.iter().zip(ws.resolve_definitions_in(&pairs, fn_file, &positions)) {
        let tok = &masked[i..e];
        let def = def?;
        if def.kind == DefKind::TyParam {
            ty_params.insert(tok.to_string());
            continue;
        }
        if def.path.rsplit("::").next() != Some(tok) || matches!(def.kind, DefKind::Local | DefKind::Field | DefKind::Use | DefKind::Macro) {
            return None;
        }
        let reader = def.reader_idx.and_then(|r| ws.readers().get(r))?;
        let def_crate = reader.crate_name().replace('-', "_");
        let root = def.path.split("::").next().unwrap_or("");
        let path = if matches!(root, "core" | "alloc" | "std") {
            // A standard item is written through `std` where it has a public path; private module paths are refused.
            let segs: Vec<&str> = def.path.split("::").collect();
            (segs.len() <= 3).then(|| format!("std::{}", segs[1..].join("::")))?
        } else if def_crate == crate_name {
            // A private item of the crate root is visible in the whole crate; one deeper only in its own module and
            // the modules inside it, so every segment past the first is `pub` or `pub(crate)`, or private with the
            // destination inside its parent.
            let rel = def.path.strip_prefix(&format!("{def_crate}::")).unwrap_or(def.path);
            let segs: Vec<&str> = rel.split("::").collect();
            for k in 1..segs.len() {
                let prefix = segs[..=k].join("::");
                let vis = reader.find_definition_by_path(&format!("{def_crate}::{prefix}")).or_else(|| reader.find_definition_by_path(&prefix)).map(|d| d.item_vis());
                match vis {
                    Some(v) if reachable(v, &def_crate, crate_name) => {}
                    Some(ItemVis::Private) => narrow(&mut within, segs[..k].join("::"))?,
                    _ => return None,
                }
            }
            // A struct built here (`Name(..)`, `Name { .. }`) needs each field visible too: a private one only inside
            // the struct's module.
            let builds = masked[e..].trim_start().starts_with(['(', '{']);
            if builds && def.kind == DefKind::Struct {
                let parent = rel.rsplit_once("::").map(|(p, _)| p);
                for vis in struct_field_vis(reader, def.path)? {
                    match (vis, parent) {
                        (v, _) if reachable(v, &def_crate, crate_name) => {}
                        (ItemVis::Private, None) => {}
                        (ItemVis::Private, Some(p)) => narrow(&mut within, p.to_string())?,
                        _ => return None,
                    }
                }
            }
            format!("crate::{rel}")
        } else {
            return None;
        };
        if out.insert(tok.to_string(), path.clone()).is_some_and(|p| p != path) {
            return None;
        }
    }
    Some(CrossFilePaths { paths: out, within, ty_params })
}

/// What [`cross_file_paths`] found: each name's path, the crate-relative module a destination must lie in, and the
/// generic parameters named.
pub(crate) struct CrossFilePaths {
    pub paths: HashMap<String, String>,
    pub within: Option<String>,
    pub ty_params: HashSet<String>,
}

/// A parameter's type `t` for a binding in another file: its names written as paths and each generic parameter as `_`
/// (`&F` → `&_`, which keeps the reference the argument coerces to); `None` when a name has no path or a generic
/// parameter is projected (`F::Output`).
fn binding_type(t: &str, p: &CrossFilePaths, locals: &HashSet<String>) -> Option<String> {
    let masked = mask_comments_and_strings(t);
    let mut out = t.to_string();
    for (i, e) in tokens(&masked).into_iter().rev() {
        if p.ty_params.contains(&masked[i..e]) {
            if masked[e..].trim_start().starts_with("::") {
                return None;
            }
            out.replace_range(i..e, "_");
        }
    }
    apply_paths(&out, &p.paths, locals)
}

/// The visibility of field `path` (`krate::m::S::name` or `…::S::0`) from the index's field table, which records it
/// (a definition's own visibility does not for fields).
fn field_vis(reader: &ridx_format::IndexReader<memmap2::Mmap>, path: &str) -> Option<ItemVis> {
    let (owner, name) = path.rsplit_once("::")?;
    let owner_def = reader.find_definition_by_path(owner)?;
    let idx = reader.definitions().iter().position(|d| std::ptr::eq(d, owner_def))?;
    reader.find_field(idx as u32, name).map(|f| f.vis)
}

/// The visibility of each field of the struct at `path`, from the index's field table.
fn struct_field_vis(reader: &ridx_format::IndexReader<memmap2::Mmap>, path: &str) -> Option<Vec<ItemVis>> {
    let def = reader.find_definition_by_path(path)?;
    let idx = reader.definitions().iter().position(|d| std::ptr::eq(d, def))?;
    Some(reader.fields_for_owner(idx as u32).map(|f| f.vis).collect())
}

/// Whether every `.name` and `.N` of `body` (at byte `start` of `fn_src`) reaches a field or method visible from
/// another file of `crate_name` whose text is `dest`: a trait's method needs the trait in the prelude or named in `dest`.
pub(crate) fn members_reachable(body: &str, fn_src: &str, fn_file: &str, start: usize, ws: &WorkspaceIndex, crate_name: &str, dest: &str) -> bool {
    let masked = mask_comments_and_strings(body);
    let pairs = ws.resolved_reader_file_ids(fn_file);
    let mb = masked.as_bytes();
    let mut members: Vec<(usize, usize)> = tokens(&masked).into_iter().filter(|&(i, _)| masked[..i].trim_end().ends_with('.') && !masked[..i].trim_end().ends_with("..")).collect();
    // Tuple fields (`.0`), which are no identifiers.
    for (p, _) in masked.match_indices('.') {
        if mb.get(p + 1).is_some_and(|c| c.is_ascii_digit()) && (p == 0 || !mb[p - 1].is_ascii_digit()) && mb.get(p.wrapping_sub(1)) != Some(&b'.') {
            let end = p + 1 + masked[p + 1..].bytes().take_while(|c| c.is_ascii_digit()).count();
            members.push((p + 1, end));
        }
    }
    let positions: Vec<(u32, u32)> = members.iter().map(|&(i, _)| line_col(fn_src, start + i)).collect();
    let dest_masked = mask_comments_and_strings(dest);
    members.iter().zip(ws.resolve_definitions_in(&pairs, fn_file, &positions)).all(|(&(i, e), def)| {
        let tok = &masked[i..e];
        // `.await` and tuple fields (`.0`) are no names to resolve.
        if tok == "await" {
            return true;
        }
        let Some(def) = def else { return false };
        let Some(reader) = def.reader_idx.and_then(|r| ws.readers().get(r)) else { return false };
        let def_crate = reader.crate_name().replace('-', "_");
        let vis = reader.find_definition_by_path(def.path).map(|d| d.item_vis()).unwrap_or(ItemVis::Private);
        match def.kind {
            DefKind::Field => field_vis(reader, def.path).is_some_and(|v| reachable(v, &def_crate, crate_name)),
            DefKind::AssocFn | DefKind::Fn => {
                let Some((parent, _)) = def.path.rsplit_once("::") else { return false };
                match reader.find_definition_by_path(parent).map(|d| DefKind::from(d.kind)) {
                    Some(DefKind::Trait) => {
                        let tr = parent.rsplit("::").next().unwrap_or(parent);
                        is_rust_kw_or_prelude(tr) || contains_word(&dest_masked, tr)
                    }
                    _ => reachable(vis, &def_crate, crate_name) || matches!(def_crate.as_str(), "core" | "alloc" | "std"),
                }
            }
            _ => false,
        }
    })
}

/// `body` with each name of `paths` written as its path; `None` when a name to move is left without one.
pub(crate) fn apply_paths(body: &str, paths: &HashMap<String, String>, locals: &HashSet<String>) -> Option<String> {
    let masked = mask_comments_and_strings(body);
    let mut out = body.to_string();
    for (i, e) in tokens(&masked).into_iter().rev() {
        let tok = &masked[i..e];
        if !is_candidate(&masked, i, e, tok) || is_rust_kw_or_prelude(tok) || locals.contains(tok) || matches!(tok, "self" | "Self" | "crate" | "this" | "ret") {
            continue;
        }
        out.replace_range(i..e, paths.get(tok)?);
    }
    Some(out)
}

/// The function inlined into another file of its crate: its transformed `body` and its `params` with the names they
/// use written as paths, and the crate's name; `None` when a name cannot be written so.
pub(crate) struct CrossFileBody {
    pub body: String,
    pub params: Vec<super::action_inline_callers_params::FnParamInfo>,
    pub crate_name: String,
    /// The crate-relative module the destination must lie in, for the private items the paths name.
    pub within: Option<String>,
}

impl CrossFileBody {
    /// Whether the body may go into `dest` (a file of the same crate, where every member it reaches is visible).
    pub(crate) fn fits(&self, ws: &WorkspaceIndex, dest: &std::path::Path, raw_body: &str, fn_src: &str, fn_file: &str, start: usize) -> bool {
        // One compilation unit holds both files (an integration test is another crate, where `crate::` is the test).
        let dest_file = dest.to_str().unwrap_or_default();
        let units: Vec<usize> = ws.resolved_reader_file_ids(fn_file).into_iter().map(|p| p.0).collect();
        let same_crate = ws.resolved_reader_file_ids(dest_file).iter().any(|p| units.contains(&p.0));
        same_crate
            && self.within.as_deref().is_none_or(|w| self.dest_inside(ws, dest_file, w))
            && std::fs::read_to_string(dest)
                .is_ok_and(|t| super::action_inline_callers_macros::macros_in_scope(raw_body, &t) && members_reachable(raw_body, fn_src, fn_file, start, ws, &self.crate_name, &t))
    }

    /// Whether the module of `dest_file` is the crate-relative module `w` or lies inside it.
    fn dest_inside(&self, ws: &WorkspaceIndex, dest_file: &str, w: &str) -> bool {
        let Some(module) = ws.module_path_for_file(dest_file) else { return false };
        let rel = module.split_once("::").map_or("", |(_, r)| r);
        rel == w || rel.starts_with(&format!("{w}::"))
    }
}

/// Prepares the inlining of the function declared in `doc` (`file`) from its name at `name_byte` to its body's close
/// at `body_close`, whose raw body starts at `body_open + 1`, into other files of its crate.
#[allow(clippy::too_many_arguments)]
pub(crate) fn cross_file_body(
    doc: &str,
    file: &str,
    name: &str,
    name_byte: usize,
    body_close: usize,
    params_src: &str,
    ws: &WorkspaceIndex,
    body: &str,
    params: &[super::action_inline_callers_params::FnParamInfo],
) -> Option<CrossFileBody> {
    let crate_name = ws.reader_indices_for_file(file).first().map(|&i| ws.readers()[i].crate_name().replace('-', "_"))?;
    let sig_and_body = &doc[name_byte..body_close];
    let mut locals = super::action_default_safety::collect_local_bindings(&mask_comments_and_strings(sig_and_body), &mask_comments_and_strings(params_src));
    locals.extend([name.to_string(), "ret".to_string()]);
    let paths = cross_file_paths(sig_and_body, doc, file, name_byte, ws, &crate_name, &locals)?;
    let body = apply_paths(body, &paths.paths, &locals)?;
    // A parameter's type, written on a binding at the call site: `Some(None)` when it has none.
    let qualify_ty = |t: &Option<String>| -> Option<Option<String>> {
        match t {
            None => Some(None),
            Some(t) => binding_type(t, &paths, &locals).map(Some),
        }
    };
    let params = params
        .iter()
        .map(|p| Some(super::action_inline_callers_params::FnParamInfo { ty: qualify_ty(&p.ty)?, decl_ty: qualify_ty(&p.decl_ty)?, ..p.clone() }))
        .collect::<Option<Vec<_>>>()?;
    Some(CrossFileBody { body, params, crate_name, within: paths.within })
}

#[cfg(test)]
mod tests {
    use super::{apply_paths, binding_type, is_candidate, narrow, CrossFilePaths};
    use std::collections::{HashMap, HashSet};

    #[test]
    fn writes_moved_names_as_paths() {
        let body = "let s = Style::new(); helper(s.len(), io::stdout()); vec.push(x); Ok::<u8, Error>(1)";
        let at = |w: &str| body.find(w).unwrap();
        assert!(is_candidate(body, at("Style"), at("Style") + 5, "Style"));
        assert!(is_candidate(body, at("helper"), at("helper") + 6, "helper"));
        assert!(!is_candidate(body, at("push"), at("push") + 4, "push"), "a method");
        let paths: HashMap<String, String> = [("Style", "crate::fmt::Style"), ("helper", "crate::util::helper"), ("io", "std::io"), ("Error", "crate::Error")]
            .into_iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let locals: HashSet<String> = ["s", "vec", "x"].into_iter().map(String::from).collect();
        let out = apply_paths(body, &paths, &locals).unwrap();
        assert_eq!(out, "let s = crate::fmt::Style::new(); crate::util::helper(s.len(), std::io::stdout()); vec.push(x); Ok::<u8, crate::Error>(1)");
        assert!(apply_paths("Unknown::new()", &paths, &locals).is_none(), "a name without a path");
    }

    #[test]
    fn generic_parameters_bind_as_placeholders_and_private_items_narrow_the_destination() {
        let p = CrossFilePaths { paths: [("Style".to_string(), "crate::fmt::Style".to_string())].into(), within: None, ty_params: ["F".to_string(), "T".to_string()].into() };
        let none = HashSet::new();
        assert_eq!(binding_type("&mut F", &p, &none).as_deref(), Some("&mut _"));
        assert_eq!(binding_type("Vec<(T, Style)>", &p, &none).as_deref(), Some("Vec<(_, crate::fmt::Style)>"));
        assert!(binding_type("F::Output", &p, &none).is_none(), "a projection has no placeholder");
        let mut within = None;
        assert!(narrow(&mut within, "sync".into()).is_some() && narrow(&mut within, "sync::notify".into()).is_some());
        assert_eq!(within.as_deref(), Some("sync::notify"));
        assert!(narrow(&mut within, "sync".into()).is_some(), "an outer module is implied");
        assert!(narrow(&mut within, "runtime".into()).is_none(), "no module lies in both");
    }
}
