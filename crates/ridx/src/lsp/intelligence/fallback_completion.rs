/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Streaming, low-latency fallback completion for dot- and colon-triggered queries.

use super::super::protocol::lsp_completion_kind;
use super::owner_completion::{signature_takes_receiver, OwnerCompletionContext};
use crate::overlay::DocumentOverlay;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};
use std::collections::HashSet;

/// Normalizes raw type strings (stripping leading `&`, `&mut `, and generic parameters `<...>`).
pub fn normalize_type(ty: &str) -> &str {
    let t = ty
        .trim()
        .strip_prefix("&mut ")
        .or_else(|| ty.trim().strip_prefix('&'))
        .unwrap_or(ty)
        .trim();
    t.split('<').next().unwrap_or(t).trim()
}

/// Executes fast fallback completion for dot- or colon-triggered queries.
///
/// Returns `(items, isIncomplete)`.
pub fn try_fallback_completion(
    ws: &WorkspaceIndex,
    ctx: &OwnerCompletionContext<'_>,
    overlay_opt: Option<&DocumentOverlay>,
) -> (Vec<Value>, bool) {
    let mut items = Vec::new();
    let mut seen_names = HashSet::new();
    let mut is_capped = false;

    let (target_prefix, target_suffix, target_for_needle, target_crate) = if ctx.is_dot {
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
        let rtype = ws
            .type_at(ctx.file, ctx.line_1based, rcol)
            .map(|t| t.to_string())
            .or_else(|| {
                ws.find_definition_at(ctx.file, ctx.line_1based, rcol)
                    .and_then(|d| d.signature.map(|s| s.to_string()))
            })
            .or_else(|| {
                overlay_opt.and_then(|ov| {
                    ov.symbols
                        .iter()
                        .find(|s| s.name == rname)
                        .and_then(|s| s.signature.clone())
                })
            });
        let norm = rtype.as_deref().map(normalize_type).unwrap_or("");
        (
            format!("{norm}::"),
            format!("::{norm}::"),
            norm.to_string(),
            None,
        )
    } else {
        let qual: String = ctx.trimmed_before[..ctx.trimmed_before.len() - 2]
            .trim_end()
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let def_opt = ws.resolve_qualifier(ctx.file, &qual);
        let target = def_opt.as_ref().map(|d| d.path).unwrap_or(&qual);
        let first_seg = qual.split("::").next().unwrap_or(&qual);
        let target_crate = if ws.has_crate(first_seg) {
            Some(first_seg.to_string())
        } else {
            None
        };
        (
            format!("{target}::"),
            format!("::{target}::"),
            target.to_string(),
            target_crate,
        )
    };

    // Determine targeted needle:
    // If prefix is typed, search for prefix.
    // If querying an exact crate root (e.g. derive_more::), needle is empty (match all in that crate).
    // Otherwise if target is known, search for target's short segment to avoid scanning unrelated symbols.
    let needle = if !ctx.prefix.is_empty() {
        ctx.prefix
    } else if let Some(ref cname) = target_crate {
        if target_for_needle == *cname {
            ""
        } else {
            target_for_needle
                .rsplit("::")
                .next()
                .unwrap_or(&target_for_needle)
        }
    } else if !target_for_needle.is_empty()
        && target_for_needle != "crate"
        && target_for_needle != "super"
        && target_for_needle != "self"
        && target_for_needle != "Self"
    {
        target_for_needle
            .rsplit("::")
            .next()
            .unwrap_or(&target_for_needle)
    } else {
        ""
    };

    let needle_lower = needle.to_ascii_lowercase();

    let mut collector = |reader: &ridx_format::IndexReader<memmap2::Mmap>,
                         def: &ridx_format::DefEntry|
     -> bool {
        let kind = DefKind::from(def.kind);
        if ctx.is_dot
            && (!matches!(
                kind,
                DefKind::AssocFn | DefKind::Fn | DefKind::Field
            ) || (def.sig_offset() != 0
                && !signature_takes_receiver(reader.get_str(def.sig_offset()))))
        {
            return true;
        }

        let name = reader.get_str(def.name_offset());
        let path = reader.get_str(def.path_offset());
        let crate_name = reader.crate_name();

        if target_prefix != "::" {
            let full_path = format!("{crate_name}::{path}");
            if !full_path.starts_with(&target_prefix)
                && !full_path.contains(&target_suffix)
                && !path.starts_with(&target_prefix)
            {
                return true;
            }
        }

        if !ctx.prefix.is_empty() && !name.starts_with(ctx.prefix) {
            return true;
        }

        if seen_names.insert(name.to_string()) {
            let detail = if def.sig_offset() != 0 {
                format!("{} (in {})", reader.get_str(def.sig_offset()), crate_name)
            } else {
                format!("{crate_name}::{path}")
            };
            items.push(json!({
                "label": name,
                "kind": lsp_completion_kind(kind),
                "detail": detail
            }));
            if items.len() >= 100 {
                is_capped = true;
                return false;
            }
        }
        true
    };

    if let Some(ref cname) = target_crate {
        ws.for_each_symbol_in_crate(cname, needle_lower.as_bytes(), &mut collector);
    } else {
        let preferred_crate = ws.crate_name_for_file(ctx.file);
        ws.for_each_symbol_ascii(needle_lower.as_bytes(), preferred_crate, &mut collector);
    }

    (items, is_capped)
}
