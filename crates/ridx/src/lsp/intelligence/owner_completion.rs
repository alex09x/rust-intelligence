/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Fast owner-index and autoderef-based completion handler.

use super::super::protocol::lsp_completion_kind;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};
use std::collections::HashSet;

/// Returns true if the path belongs to standard/sysroot crates not yet indexed in `.ridx`.
pub(crate) fn is_sysroot_path(path: &str) -> bool {
    path.starts_with("core::")
        || path.starts_with("alloc::")
        || path.starts_with("std::")
        || path == "str"
        || path == "slice"
        || matches!(
            path,
            "bool"
                | "char"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
                | "f32"
                | "f64"
        )
}

/// Checks if a function signature takes a receiver argument (`self`, `&self`, `&mut self`).
pub(crate) fn signature_takes_receiver(sig: &str) -> bool {
    sig.find('(').is_some_and(|i| {
        let p = sig[i + 1..].trim_start();
        p.starts_with('&')
            || p.starts_with("self")
            || p.starts_with("Self")
            || p.starts_with("mut self")
    })
}

/// Context parameters for owner-based scoped completion.
pub struct OwnerCompletionContext<'a> {
    pub file: &'a str,
    pub line_1based: u32,
    pub line_slice: &'a str,
    pub prefix: &'a str,
    pub is_dot: bool,
    pub is_colon: bool,
    pub trimmed_before: &'a str,
}

/// Attempts high-speed owner-based completion for dot- (`receiver.`) or colon- (`path::`) trigger.
///
/// Returns `Some(items)` if the owner index resolved candidate items directly (sub-100 µs),
/// or `None` if fallback to workspace symbol scanning is required (e.g. sysroot types, unknown types).
pub fn try_owner_completion(
    ws: &WorkspaceIndex,
    ctx: &OwnerCompletionContext<'_>,
) -> Option<Vec<Value>> {
    if ctx.is_dot {
        let before_dot = ctx.trimmed_before[..ctx.trimmed_before.len() - 1].trim_end();
        let rname: String = before_dot
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let rstart = before_dot.len().saturating_sub(rname.len());
        let rcol = ctx.line_slice[..rstart].chars().count() as u32 + 1;
        let rend_col = ctx.line_slice[..before_dot.len()].chars().count() as u32;

        let type_entry = ws
            .type_entry_at(ctx.file, ctx.line_1based, rend_col)
            .or_else(|| ws.type_entry_at(ctx.file, ctx.line_1based, rcol));

        let Some((_ty_name, Some(owner))) = type_entry else {
            return None;
        };

        if is_sysroot_path(owner) {
            return None;
        }

        let mut owners = vec![owner.to_string()];
        for deref_target in ws.resolve_deref_chain(owner, 3) {
            owners.push(deref_target.to_string());
        }

        let mut items = Vec::new();
        let mut seen = HashSet::new();

        for o in &owners {
            for def in ws.find_definitions_by_owner(o) {
                if !matches!(
                    def.kind,
                    DefKind::AssocFn | DefKind::Fn | DefKind::Field
                ) {
                    continue;
                }
                if def
                    .signature
                    .as_ref()
                    .is_some_and(|sig| !signature_takes_receiver(sig))
                {
                    continue;
                }
                if !ctx.prefix.is_empty() && !def.name.starts_with(ctx.prefix) {
                    continue;
                }
                if seen.insert(def.name.to_string()) {
                    let detail = def
                        .signature
                        .map(|sig| format!("{sig} (in {})", def.path))
                        .unwrap_or_else(|| def.path.to_string());
                    items.push(json!({
                        "label": def.name,
                        "kind": lsp_completion_kind(def.kind),
                        "detail": detail
                    }));
                    if items.len() >= 100 {
                        break;
                    }
                }
            }
            if items.len() >= 100 {
                break;
            }
        }

        if items.is_empty() {
            None
        } else {
            Some(items)
        }
    } else if ctx.is_colon {
        let qual: String = ctx.trimmed_before[..ctx.trimmed_before.len() - 2]
            .trim_end()
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        if qual.is_empty() {
            return None;
        }

        let qual_path = if qual == "crate" {
            ws.crate_name_for_file(ctx.file)
                .map(|c| c.to_string())
                .unwrap_or_else(|| qual.clone())
        } else if let Some(rest) = qual.strip_prefix("crate::") {
            ws.crate_name_for_file(ctx.file)
                .map(|c| format!("{c}::{rest}"))
                .unwrap_or_else(|| qual.clone())
        } else {
            qual.clone()
        };

        let def_opt = ws.find_definition_by_path(&qual_path);
        let target_owner = def_opt
            .as_ref()
            .map(|d| d.path.to_string())
            .unwrap_or(qual_path);

        if is_sysroot_path(&target_owner) {
            return None;
        }

        let mut items = Vec::new();
        let mut seen = HashSet::new();

        for def in ws.find_definitions_by_owner(&target_owner) {
            if !ctx.prefix.is_empty() && !def.name.starts_with(ctx.prefix) {
                continue;
            }
            if seen.insert(def.name.to_string()) {
                let detail = def
                    .signature
                    .map(|sig| format!("{sig} (in {})", def.path))
                    .unwrap_or_else(|| def.path.to_string());
                items.push(json!({
                    "label": def.name,
                    "kind": lsp_completion_kind(def.kind),
                    "detail": detail
                }));
                if items.len() >= 100 {
                    break;
                }
            }
        }

        if items.is_empty() {
            None
        } else {
            Some(items)
        }
    } else {
        None
    }
}
