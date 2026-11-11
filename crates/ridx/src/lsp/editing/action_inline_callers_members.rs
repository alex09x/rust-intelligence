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

/// The visibility of each field of the struct at `path`, from the index's field table.
pub(crate) fn struct_field_vis(reader: &ridx_format::IndexReader<memmap2::Mmap>, path: &str) -> Option<Vec<ItemVis>> {
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
