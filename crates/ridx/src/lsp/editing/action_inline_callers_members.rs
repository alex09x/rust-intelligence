/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Whether the fields and methods an inlined body reaches stay visible in the file it moves to, from the index's
//! definition and field tables.

use super::action_default_safety::is_rust_kw_or_prelude;
use super::action_inline_callers_qualify::{line_col, reachable, tokens};
use super::action_inline_callers_vis::{inside, member_owner, restricted_scope};
use super::action_parse::mask_comments_and_strings;
use super::action_parse_items::contains_word;
use ridx_format::{DefKind, ItemVis, WorkspaceIndex};

/// The visibility of field `path` (`krate::m::S::name` or `…::S::0`) from the index's field table, which records it
/// (a definition's own visibility does not for fields).
fn field_vis(reader: &ridx_format::IndexReader<memmap2::Mmap>, path: &str) -> Option<ItemVis> {
    let (owner, name) = path.rsplit_once("::")?;
    let owner_def = reader.find_definition_by_path(owner)?;
    let idx = reader.definitions().iter().position(|d| std::ptr::eq(d, owner_def))?;
    reader.find_field(idx as u32, name).map(|f| f.vis)
}

/// The names of the items a function body declares (`const`, `static`, `fn`, `struct`, `enum`, `union`, `type`,
/// `trait`, `mod`), which are local to it and have no path outside.
pub(crate) fn body_item_names(masked: &str) -> Vec<String> {
    const KW: &[&str] = &["const", "static", "fn", "struct", "enum", "union", "type", "trait", "mod"];
    let toks = tokens(masked);
    toks.windows(2)
        .filter(|w| KW.contains(&&masked[w[0].0..w[0].1]) && masked[w[0].1..w[1].0].trim().is_empty())
        .map(|w| masked[w[1].0..w[1].1].to_string())
        .filter(|n| n != "mut")
        .collect()
}

/// Whether the module that declares the private module `module` (on the way to `def`) re-exports `def`'s item to the
/// crate or wider (`mod any_value; pub(crate) use self::any_value::AnyValue;`). The declaring file is found among the
/// ancestors of `def`'s file: `dir/mod.rs`, `dir.rs` or `dir/lib.rs` with `mod module`.
pub(crate) fn reexported_above(ws: &WorkspaceIndex, def: &ridx_format::ResolvedDefinition<'_>, module: &str) -> bool {
    let Some(reader) = def.reader_idx.and_then(|r| ws.readers().get(r)) else { return false };
    let item = def.path.rsplit("::").next().unwrap_or(def.path);
    let file = ws.resolve_path_for_crate(reader.crate_name(), def.file);
    let declares_mod =
        |t: &str| t.match_indices("mod ").any(|(p, _)| t[p + 4..].trim_start().starts_with(module) && t[p + 4..].trim_start()[module.len()..].starts_with([';', ' ', '{']));
    for dir in file.ancestors().skip(1) {
        for cand in [dir.join("mod.rs"), dir.with_extension("rs"), dir.join("lib.rs")] {
            let Ok(text) = std::fs::read_to_string(&cand) else { continue };
            let masked = mask_comments_and_strings(&text);
            if !declares_mod(&masked) {
                continue;
            }
            return masked.match_indices("use ").any(|(p, _)| {
                let head = masked[..p].trim_end();
                let public = head.ends_with("pub") || head.ends_with("pub(crate)");
                let stmt = masked[p + 4..].split(';').next().unwrap_or("");
                public && contains_word(stmt, module) && contains_word(stmt, item)
            });
        }
    }
    false
}

/// Whether the masked text `dest` brings trait `tr` into scope: a `use` naming it, or a glob of a `prelude`. A word
/// `Context` elsewhere (a struct of that name) does not.
fn trait_imported(dest: &str, tr: &str) -> bool {
    dest.match_indices("use ").any(|(p, _)| {
        let stmt = dest[p + 4..].split(';').next().unwrap_or("");
        (p == 0 || !dest.as_bytes()[p - 1].is_ascii_alphanumeric()) && (contains_word(stmt, tr) || stmt.contains("prelude::*"))
    })
}

/// The visibility of each field of the struct at `path`, from the index's field table.
pub(crate) fn struct_field_vis(reader: &ridx_format::IndexReader<memmap2::Mmap>, path: &str) -> Option<Vec<ItemVis>> {
    let def = reader.find_definition_by_path(path)?;
    let idx = reader.definitions().iter().position(|d| std::ptr::eq(d, def))?;
    Some(reader.fields_for_owner(idx as u32).map(|f| f.vis).collect())
}

/// Checks the associated functions `body` calls by path (`MatchesError::unwrap(..)`) against the destination: public
/// from another crate (`external`, or one the function's crate depends on), `pub(crate)` within the function's crate,
/// private only inside the module of the type's impl, which narrows `within`. `None` when one cannot be reached.
#[allow(clippy::too_many_arguments)]
pub(crate) fn assoc_calls_reachable(
    body: &str,
    fn_src: &str,
    fn_file: &str,
    start: usize,
    ws: &WorkspaceIndex,
    crate_name: &str,
    external: bool,
    within: &mut Option<String>,
) -> Option<()> {
    let masked = mask_comments_and_strings(body);
    let calls: Vec<(usize, usize)> =
        tokens(&masked).into_iter().filter(|&(i, e)| masked[..i].ends_with("::") && (masked[e..].trim_start().starts_with('(') || masked[e..].starts_with("::<"))).collect();
    let positions: Vec<(u32, u32)> = calls.iter().map(|&(i, _)| line_col(fn_src, start + i)).collect();
    for def in ws.resolve_definitions_in(&ws.resolved_reader_file_ids(fn_file), fn_file, &positions).into_iter().flatten() {
        if def.kind != DefKind::AssocFn {
            continue;
        }
        let reader = def.reader_idx.and_then(|r| ws.readers().get(r))?;
        let def_crate = reader.crate_name().replace('-', "_");
        if matches!(def_crate.as_str(), "core" | "alloc" | "std") {
            continue;
        }
        match reader.find_definition_by_path(def.path).map(|d| d.item_vis())? {
            ItemVis::Public => {}
            ItemVis::Crate if !external && def_crate == crate_name => {}
            ItemVis::Private if !external && def_crate == crate_name => {
                // `m::Type::f`: visible in `m` and the modules inside it.
                let owner = member_owner(def.path.strip_prefix(&format!("{def_crate}::")).unwrap_or(def.path));
                if !owner.is_empty() {
                    super::action_inline_callers_qualify::narrow(within, owner)?;
                }
            }
            // `pub(super) fn f`: visible in the scope its declaration names.
            ItemVis::Restricted if !external && def_crate == crate_name => {
                let owner = member_owner(def.path.strip_prefix(&format!("{def_crate}::")).unwrap_or(def.path));
                let scope = restricted_scope(ws, &def, &owner)?;
                if !scope.is_empty() {
                    super::action_inline_callers_qualify::narrow(within, scope)?;
                }
            }
            _ => return None,
        }
    }
    Some(())
}

/// Whether every `.name` and `.N` of `body` (at byte `start` of `fn_src`) reaches a field or method visible from
/// another file of `crate_name` whose text is `dest` and whose crate-relative module is `dest_module`: a private or
/// restricted member only inside its scope, a trait's method with the trait in the prelude or named in `dest`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn members_reachable(body: &str, fn_src: &str, fn_file: &str, start: usize, ws: &WorkspaceIndex, crate_name: &str, dest: &str, dest_module: Option<&str>) -> bool {
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
        let owner = member_owner(def.path.strip_prefix(&format!("{def_crate}::")).unwrap_or(def.path));
        let seen = |v: ItemVis| match v {
            _ if reachable(v, &def_crate, crate_name) => true,
            ItemVis::Private if def_crate == crate_name => dest_module.is_some_and(|d| inside(d, &owner)),
            ItemVis::Restricted if def_crate == crate_name => dest_module.zip(restricted_scope(ws, &def, &owner)).is_some_and(|(d, s)| inside(d, &s)),
            _ => false,
        };
        match def.kind {
            DefKind::Field => field_vis(reader, def.path).is_some_and(seen),
            DefKind::AssocFn | DefKind::Fn => {
                let Some((parent, _)) = def.path.rsplit_once("::") else { return false };
                // A trait impl's method (`<Flatten<I> as core::iter::Iterator>::next`) is the trait's: in scope with it.
                // Also written `context::<impl Context<T, E> for Result<T, E>>::context`.
                let impl_for = parent.rfind("<impl ").and_then(|p| parent[p + 6..].split_once(" for ")).map(|(t, _)| t);
                if let Some(tr) = parent.strip_suffix('>').and_then(|p| p.rsplit_once(" as ").map(|(_, t)| t)).or(impl_for).map(|t| t.split('<').next().unwrap_or(t)) {
                    let tr = tr.rsplit("::").next().unwrap_or(tr);
                    return is_rust_kw_or_prelude(tr) || trait_imported(&dest_masked, tr);
                }
                match reader.find_definition_by_path(parent).map(|d| DefKind::from(d.kind)) {
                    Some(DefKind::Trait) => {
                        let tr = parent.rsplit("::").next().unwrap_or(parent);
                        is_rust_kw_or_prelude(tr) || trait_imported(&dest_masked, tr)
                    }
                    _ => seen(vis) || matches!(def_crate.as_str(), "core" | "alloc" | "std"),
                }
            }
            _ => false,
        }
    })
}
