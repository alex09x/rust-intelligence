/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The body `Inline into all callers` writes where the function's own names are not in scope: another file of its
//! crate, a module nested in its own, or a crate depending on it.

use super::action_inline_callers_members::members_reachable;
use super::action_inline_callers_qualify::{apply_paths, binding_type, cross_file_paths, line_col, narrow, CrossFilePaths};
use super::action_parse::mask_comments_and_strings;
use super::action_parse_items::contains_word;
use ridx_format::{DefKind, ItemVis, WorkspaceIndex};

/// The function inlined where its names are not in scope: its transformed `body` and its `params` with the names
/// they use written as paths, and the crate's name; `None` when a name cannot be written so.
pub(crate) struct CrossFileBody {
    pub body: String,
    pub params: Vec<super::action_inline_callers_params::FnParamInfo>,
    pub crate_name: String,
    /// The crate-relative module the destination must lie in, for the private items the paths name.
    pub within: Option<String>,
    /// Written for files of other crates that depend on the function's (`krate::…` paths, public items only).
    pub external: bool,
    /// The crates the written paths start with (not `crate`, `core` or `std`): the destination's unit must link them.
    pub roots: Vec<String>,
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
        // The destination's crate-relative module, against which private and restricted members are judged.
        let dest_module = ws.module_path_for_file(dest_file).map(|m| m.split_once("::").map_or(String::new(), |(_, r)| r.to_string()));
        let Ok(text) = std::fs::read_to_string(dest) else { return false };
        // Another crate's unit must link every crate the paths start with (an example linking `clap` does not see
        // `clap_builder`): by the index's extern list, or, where the index records none, by the destination naming
        // the crate itself (`use clap::…`, `clap::Arg`). Within the crate an unrecorded list is the function's own.
        let names_crate = |r: &str| {
            let masked = mask_comments_and_strings(&text);
            masked
                .match_indices(&format!("{r}::"))
                .any(|(p, _)| p == 0 || !(masked.as_bytes()[p - 1].is_ascii_alphanumeric() || masked.as_bytes()[p - 1] == b'_' || masked.as_bytes()[p - 1] == b':'))
        };
        let linked = match ws.extern_crate_names_for_file(dest_file) {
            Some(l) => self.roots.iter().all(|r| l.contains(&r.as_str())),
            None => !self.external || self.roots.iter().all(|r| names_crate(r)),
        };
        linked
            && self.within.as_deref().is_none_or(|w| self.dest_inside(ws, dest_file, w))
            && super::action_inline_callers_macros::macros_in_scope(raw_body, &text)
            && members_reachable(raw_body, fn_src, fn_file, start, ws, &dest_crate, &text, dest_module.as_deref())
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

/// The path of the impl's type, which the body names where it said `Self` (rewritten to the type's name): read from
/// the function's own definition (`m::Inner::from_static_ref` -> `crate::m::Inner`), visible as the type is.
/// `None` when the type is restricted or the definition cannot be read; nothing is added for a free function.
fn self_type_path(ws: &WorkspaceIndex, doc: &str, file: &str, name_byte: usize, crate_name: &str, body: &str, paths: &mut CrossFilePaths) -> Option<()> {
    let at = line_col(doc, name_byte);
    let Some(def) = ws.resolve_definitions_in(&ws.resolved_reader_file_ids(file), file, &[at]).into_iter().next().flatten() else { return Some(()) };
    let reader = def.reader_idx.and_then(|r| ws.readers().get(r))?;
    let segs: Vec<&str> = def.path.split("::").filter(|s| !s.starts_with('<')).collect();
    let rel: Vec<&str> = segs.iter().copied().skip_while(|s| *s == crate_name).collect();
    if rel.len() < 2 || def.kind != DefKind::AssocFn {
        return Some(());
    }
    let owner = &rel[..rel.len() - 1];
    let ty = *owner.last()?;
    // Only a body that names the type (where it said `Self`) needs its path.
    if paths.paths.contains_key(ty) || !ty.starts_with(|c: char| c.is_ascii_uppercase()) || !contains_word(&mask_comments_and_strings(body), ty) {
        return Some(());
    }
    // Every segment past the first is `pub`/`pub(crate)`, or private with the destination inside its parent.
    for k in 1..owner.len() {
        let prefix = owner[..=k].join("::");
        let vis = reader.find_definition_by_path(&prefix).or_else(|| reader.find_definition_by_path(&format!("{crate_name}::{prefix}"))).map(|d| d.item_vis())?;
        match vis {
            ItemVis::Public | ItemVis::Crate => {}
            ItemVis::Private => narrow(&mut paths.within, owner[..k].join("::"))?,
            ItemVis::Restricted => return None,
        }
    }
    // A body building the type (`Self { stream, f }`) needs its fields visible as well.
    let masked = mask_comments_and_strings(body);
    let builds =
        masked.match_indices(ty).any(|(i, _)| masked[i + ty.len()..].trim_start().starts_with(['{', '(']) && (i == 0 || !masked.as_bytes()[i - 1].is_ascii_alphanumeric()));
    if builds {
        for vis in super::action_inline_callers_members::struct_field_vis(reader, &owner.join("::")).unwrap_or_default() {
            match vis {
                ItemVis::Public | ItemVis::Crate => {}
                ItemVis::Private if owner.len() > 1 => narrow(&mut paths.within, owner[..owner.len() - 1].join("::"))?,
                ItemVis::Private => {}
                ItemVis::Restricted => return None,
            }
        }
    }
    paths.paths.insert(ty.to_string(), format!("crate::{}", owner.join("::")));
    Some(())
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
    // Items the body declares (`const SLOT_MASK: u64 = …;`) move with it and are named as they are.
    locals.extend(super::action_inline_callers_members::body_item_names(&mask_comments_and_strings(sig_and_body)));
    let mut paths = cross_file_paths(sig_and_body, doc, file, name_byte, ws, &crate_name, &locals, external)?;
    if !external {
        self_type_path(ws, doc, file, name_byte, &crate_name, body, &mut paths)?;
    }
    // The function's generic parameters stay as they are: each call's turbofish replaces them.
    let in_body: std::collections::HashSet<String> = locals.iter().chain(&paths.ty_params).cloned().collect();
    let body = apply_paths(body, &paths.paths, &in_body)?;
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
    let mut roots: Vec<String> = paths.paths.values().filter_map(|p| p.split("::").next()).filter(|r| !matches!(*r, "crate" | "core" | "std")).map(String::from).collect();
    roots.sort_unstable();
    roots.dedup();
    Some(CrossFileBody { body, params, crate_name, within: paths.within, external, roots })
}
