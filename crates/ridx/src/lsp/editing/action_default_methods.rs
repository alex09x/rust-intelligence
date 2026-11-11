/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Method calls and field accesses in a default member body that resolve where the body is copied to (`Implement
//! default members`). The index says what each `.name` in the trait's file reaches: an inherent method or a field is
//! reachable wherever its type is, when it is public or of the same crate; a trait method only where its trait is in
//! scope.

use super::action_default_qualify::MemberOrigin;
use super::action_parse::{is_ident_byte, mask_comments_and_strings};
use super::action_parse_decl::next_ident_token;
use ridx_format::{DefKind, ItemVis, WorkspaceIndex};
use std::collections::HashSet;

/// The names after a `.` in `src` (a member body from `origin`), methods and fields, that resolve in crate
/// `dest_crate`, a trait's method counting only when `trait_in_scope` accepts the trait's name. A name met once
/// unresolvably is left out.
pub(crate) fn resolvable_method_calls(src: &str, origin: &MemberOrigin<'_>, ws: &WorkspaceIndex, dest_crate: &str, trait_in_scope: &dyn Fn(&str) -> bool) -> HashSet<String> {
    let masked = mask_comments_and_strings(src);
    let b = masked.as_bytes();
    let mut calls: Vec<(&str, (u32, u32))> = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        if !(b[i].is_ascii_alphabetic() || b[i] == b'_') || (i > 0 && is_ident_byte(b[i - 1])) {
            i += 1;
            continue;
        }
        let Some((tok, end)) = next_ident_token(&masked, i) else {
            i += 1;
            continue;
        };
        let before = masked[..i].trim_end();
        if before.ends_with('.') && !before.ends_with("..") {
            let abs = origin.start + i;
            let line_start = origin.trait_src[..abs].rfind('\n').map_or(0, |p| p + 1);
            let line = origin.trait_src[..line_start].matches('\n').count() as u32 + 1;
            let col = origin.trait_src[line_start..abs].chars().count() as u32 + 1;
            calls.push((tok, (line, col)));
        }
        i = end;
    }
    let positions: Vec<(u32, u32)> = calls.iter().map(|c| c.1).collect();
    let (mut ok, mut bad) = (HashSet::new(), HashSet::new());
    for ((name, _), def) in calls.iter().zip(ws.resolve_definitions_in(origin.file_ids, origin.trait_file, &positions)) {
        let resolves = def.is_some_and(|def| {
            if def.path.rsplit("::").next() != Some(name) {
                return false;
            }
            let Some(reader) = def.reader_idx.and_then(|i| ws.readers().get(i)) else { return false };
            let same_crate = reader.crate_name().replace('-', "_") == dest_crate;
            // A field: public, or of the destination crate.
            if def.kind == DefKind::Field {
                return same_crate || reader.find_definition_by_path(def.path).is_some_and(|d| d.item_vis() == ItemVis::Public);
            }
            if !matches!(def.kind, DefKind::AssocFn | DefKind::Fn) {
                return false;
            }
            let Some((parent, _)) = def.path.rsplit_once("::") else { return false };
            match reader.find_definition_by_path(parent).map(|d| DefKind::from(d.kind)) {
                Some(DefKind::Trait) => trait_in_scope(parent.rsplit("::").next().unwrap_or(parent)),
                // An inherent method of a named type: public, or of the destination crate.
                Some(DefKind::Struct | DefKind::Enum | DefKind::Union) => same_crate || reader.find_definition_by_path(def.path).is_some_and(|d| d.item_vis() == ItemVis::Public),
                _ => false,
            }
        });
        if resolves {
            ok.insert(name.to_string());
        } else {
            bad.insert(name.to_string());
        }
    }
    ok.retain(|n| !bad.contains(n));
    ok
}
