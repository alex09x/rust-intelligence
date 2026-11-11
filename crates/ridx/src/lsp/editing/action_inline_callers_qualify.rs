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
use super::action_inline_callers_members::{members_reachable, struct_field_vis};
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
pub(crate) fn line_col(src: &str, at: usize) -> (u32, u32) {
    let line_start = src[..at].rfind('\n').map_or(0, |p| p + 1);
    (src[..line_start].matches('\n').count() as u32 + 1, src[line_start..at].chars().count() as u32 + 1)
}

/// Whether an item of `vis` defined in crate `def_crate` is reachable from another module of `dest_crate`.
pub(crate) fn reachable(vis: ItemVis, def_crate: &str, dest_crate: &str) -> bool {
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
/// body or parameter type that names one is refused when the paths are applied. With `external` the destination is
/// another crate that depends on `crate_name`, where its items are written `krate::…` and must be public.
#[allow(clippy::too_many_arguments)]
pub(crate) fn cross_file_paths(
    body: &str,
    fn_src: &str,
    fn_file: &str,
    start: usize,
    ws: &WorkspaceIndex,
    crate_name: &str,
    locals: &HashSet<String>,
    external: bool,
) -> Option<CrossFilePaths> {
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
        } else if def_crate == crate_name && external {
            // The function's own crate seen from a crate that depends on it: public modules or a root re-export, and
            // a struct built here public in every field.
            let builds = masked[e..].trim_start().starts_with(['(', '{']);
            if builds && def.kind == DefKind::Struct && !struct_field_vis(reader, def.path)?.iter().all(|v| *v == ItemVis::Public) {
                return None;
            }
            super::action_default_qualify::path_for(ws, &def, "", true).filter(|p| !p.starts_with("crate::"))?
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
            // Another crate's item, which the function's crate (the callers' too) depends on: through its public
            // modules or a root re-export.
            super::action_default_qualify::path_for(ws, &def, crate_name, true).filter(|p| !p.starts_with("crate::"))?
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
    /// Written for files of other crates that depend on the function's (`krate::…` paths, public items only).
    pub external: bool,
}

impl CrossFileBody {
    /// Whether the body may go into `dest`: a file of the function's crate (or, for an external body, of another
    /// crate), where every member it reaches is visible.
    pub(crate) fn fits(&self, ws: &WorkspaceIndex, dest: &std::path::Path, raw_body: &str, fn_src: &str, fn_file: &str, start: usize) -> bool {
        // One compilation unit holds both files (an integration test is another crate, where `crate::` is the test).
        let dest_file = dest.to_str().unwrap_or_default();
        let units: Vec<usize> = ws.resolved_reader_file_ids(fn_file).into_iter().map(|p| p.0).collect();
        let dest_units = ws.resolved_reader_file_ids(dest_file);
        let same_crate = dest_units.iter().any(|p| units.contains(&p.0));
        // Members are judged from the destination's crate: another crate sees only public ones.
        let dest_crate = if self.external { dest_units.first().map(|p| ws.readers()[p.0].crate_name().replace('-', "_")) } else { Some(self.crate_name.clone()) };
        let Some(dest_crate) = dest_crate.filter(|_| same_crate != self.external) else { return false };
        self.within.as_deref().is_none_or(|w| self.dest_inside(ws, dest_file, w))
            && std::fs::read_to_string(dest)
                .is_ok_and(|t| super::action_inline_callers_macros::macros_in_scope(raw_body, &t) && members_reachable(raw_body, fn_src, fn_file, start, ws, &dest_crate, &t))
    }

    /// Whether the module of `dest_file` is the crate-relative module `w` or lies inside it.
    fn dest_inside(&self, ws: &WorkspaceIndex, dest_file: &str, w: &str) -> bool {
        let Some(module) = ws.module_path_for_file(dest_file) else { return false };
        let rel = module.split_once("::").map_or("", |(_, r)| r);
        rel == w || rel.starts_with(&format!("{w}::"))
    }
}

/// The bodies to inline into other files: one for files of the function's crate, one for crates depending on it.
pub(crate) struct CrossBodies {
    pub local: Option<CrossFileBody>,
    pub external: Option<CrossFileBody>,
}

impl CrossBodies {
    /// The body that fits `dest`, if any (see [`CrossFileBody::fits`]).
    pub(crate) fn for_dest(&self, ws: &WorkspaceIndex, dest: &std::path::Path, raw_body: &str, fn_src: &str, fn_file: &str, start: usize) -> Option<&CrossFileBody> {
        [&self.local, &self.external].into_iter().flatten().find(|cb| cb.fits(ws, dest, raw_body, fn_src, fn_file, start))
    }
}

/// [`cross_file_body`] for files of the function's crate and for crates depending on it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn cross_bodies(
    doc: &str,
    file: &str,
    name: &str,
    name_byte: usize,
    body_close: usize,
    params_src: &str,
    ws: &WorkspaceIndex,
    body: &str,
    params: &[super::action_inline_callers_params::FnParamInfo],
) -> CrossBodies {
    let at = |external| cross_file_body(doc, file, name, name_byte, body_close, params_src, ws, body, params, external);
    CrossBodies { local: at(false), external: at(true) }
}

/// Prepares the inlining of the function declared in `doc` (`file`) from its name at `name_byte` to its body's close
/// at `body_close`, whose raw body starts at `body_open + 1`, into other files of its crate (or, `external`, of crates
/// depending on it, where a body naming `crate`, `self::` or `super::` means something else and is refused).
#[allow(clippy::too_many_arguments)]
fn cross_file_body(
    doc: &str,
    file: &str,
    name: &str,
    name_byte: usize,
    body_close: usize,
    params_src: &str,
    ws: &WorkspaceIndex,
    body: &str,
    params: &[super::action_inline_callers_params::FnParamInfo],
    external: bool,
) -> Option<CrossFileBody> {
    let crate_name = ws.reader_indices_for_file(file).first().map(|&i| ws.readers()[i].crate_name().replace('-', "_"))?;
    let sig_and_body = &doc[name_byte..body_close];
    let body_masked = mask_comments_and_strings(body);
    if external && (contains_word(&body_masked, "crate") || contains_word(&body_masked, "super") || body_masked.contains("self::")) {
        return None;
    }
    let mut locals = super::action_default_safety::collect_local_bindings(&mask_comments_and_strings(sig_and_body), &mask_comments_and_strings(params_src));
    locals.extend([name.to_string(), "ret".to_string()]);
    let paths = cross_file_paths(sig_and_body, doc, file, name_byte, ws, &crate_name, &locals, external)?;
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
    // Another crate sees no private item, wherever it is.
    if external && paths.within.is_some() {
        return None;
    }
    Some(CrossFileBody { body, params, crate_name, within: paths.within, external })
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
