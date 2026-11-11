/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `Self` in `textDocument/typeDefinition`: the type of the impl or trait whose block holds the cursor.

use super::type_definition::{is_primitive_or_builtin, peel_type_modifiers, strip_generics, FileReaderCtx};
use super::type_definition_lookup::lookup_type_by_path;
use super::type_definition_walk::{push_unique_locs, resolve_path_with_alias_expansion};
use crate::lsp::protocol::PositionConverter;
use ridx_format::{DefKind, DefKind::*, WorkspaceIndex};
use serde_json::Value;

pub(super) fn resolve_self_only_in_impl(
    ws: &WorkspaceIndex,
    file_readers: &[usize],
    ctx: &[FileReaderCtx<'_>],
    file: &str,
    line: u32,
    locs: &mut Vec<Value>,
    conv: &mut PositionConverter,
) {
    resolve_self_into_locs(ws, file_readers, ctx, file, line, false, locs, conv);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_self_into_locs(
    ws: &WorkspaceIndex,
    file_readers: &[usize],
    ctx: &[FileReaderCtx<'_>],
    file: &str,
    line: u32,
    include_trait: bool,
    locs: &mut Vec<Value>,
    conv: &mut PositionConverter,
) {
    for c in ctx {
        // The nearest preceding impl item may be a derive's (on the struct itself) or close before `line`.
        let Some(def) = ws
            .find_enclosing_impl_or_trait_def(c.idx, c.file_id, line)
            .filter(|d| !d.is_macro_expansion() && super::type_definition_parse::still_open_at(conv, file, d.span().start_line(), line))
        else {
            continue;
        };
        let def_path = c.reader.get_str(def.path_offset());
        let def_kind = DefKind::from(def.kind);
        if def_kind == Trait {
            if let Some(loc) = lookup_type_by_path(ws, file_readers, file, def_path, true, conv) {
                push_unique_locs(locs, loc);
                return;
            }
            continue;
        }
        let owner_part = match def_kind {
            AssocFn | AssocTy | AssocConst => def_path.rsplit_once("::").map_or(def_path, |(p, _)| p),
            _ => def_path,
        };
        let cands: Vec<&str> = owner_part.strip_prefix('<').and_then(|s| s.strip_suffix('>')).map_or_else(
            || vec![owner_part],
            |inner| {
                let parts: Vec<&str> = inner.split(" as ").collect();
                if include_trait {
                    parts
                } else {
                    parts.into_iter().take(1).collect()
                }
            },
        );
        let before = locs.len();
        for cand_ty in cands {
            let base = strip_generics(peel_type_modifiers(cand_ty));
            if !base.is_empty() && !is_primitive_or_builtin(base) {
                resolve_path_with_alias_expansion(ws, file_readers, file, base, true, 0, locs, conv);
            }
        }
        if locs.len() > before {
            return;
        }
    }
}
