/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The `use` item a module imports a name through, and whether two files belong to one Cargo package, for the
//! trait lookup of `Implement default members`.

use super::action_generics_sub::split_top_level_commas;
use super::action_parse::is_ident_byte;
use std::path::Path;

/// What a `use` item imports a local name from: the original name, the crate it starts at (none for a path in
/// the same crate), the path after that crate, and whether it starts at `crate`/`super`/`self`.
pub(super) struct UseImportHit {
    pub(super) orig_name: String,
    pub(super) root_crate: Option<String>,
    pub(super) suffix: Option<String>,
    pub(super) is_local_rel: bool,
}

pub(super) fn split_root_and_suffix(p: &str) -> (Option<String>, String) {
    if let Some(r) = p.strip_prefix("crate::").or_else(|| p.strip_prefix("super::")).or_else(|| p.strip_prefix("self::")) {
        let clean = r.trim_start_matches("super::").trim_start_matches("self::");
        return (None, clean.to_string());
    }
    if let Some((first, rest)) = p.split_once("::") {
        let f = first.trim();
        if !f.is_empty() && !matches!(f, "std" | "core" | "alloc") {
            return (Some(f.to_string()), rest.trim().to_string());
        }
    }
    (None, p.to_string())
}

fn cargo_manifest_dir(p: &Path) -> Option<&Path> {
    let mut cur = p.parent();
    while let Some(dir) = cur {
        if dir.join("Cargo.toml").is_file() {
            return Some(dir);
        }
        cur = dir.parent();
    }
    None
}

pub(super) fn same_cargo_manifest_dir(a: &Path, b: &Path) -> bool {
    match (cargo_manifest_dir(a), cargo_manifest_dir(b)) {
        (Some(da), Some(db)) => da == db,
        _ => false,
    }
}

pub(super) fn find_imported_path_for(masked_mod: &str, ident: &str) -> Option<UseImportHit> {
    let mb = masked_mod.as_bytes();
    let mut depth = 0i32;
    let mut i = 0usize;
    while i + 4 < mb.len() {
        match mb[i] {
            b'{' => depth += 1,
            b'}' => depth = (depth - 1).max(0),
            _ if depth == 0 && &mb[i..i + 3] == b"use" && (i == 0 || !is_ident_byte(mb[i - 1])) && mb[i + 3].is_ascii_whitespace() => {
                let mut semi = i + 3;
                let mut b_d = 0i32;
                while semi < mb.len() {
                    match mb[semi] {
                        b'{' => b_d += 1,
                        b'}' => b_d = (b_d - 1).max(0),
                        b';' if b_d == 0 => break,
                        _ => {}
                    }
                    semi += 1;
                }
                if semi < mb.len() {
                    if let Some(full) = match_use_tree("", &masked_mod[i + 3..semi], ident) {
                        let clean = full.trim_start_matches("::");
                        let is_local_rel = clean.starts_with("crate::") || clean.starts_with("super::") || clean.starts_with("self::");
                        let (root_crate, sfx) = split_root_and_suffix(clean);
                        let orig_raw = sfx.rsplit("::").next().unwrap_or(&sfx).trim();
                        let orig_name = orig_raw.strip_prefix("r#").unwrap_or(orig_raw).to_string();
                        return Some(UseImportHit { orig_name, root_crate, suffix: Some(sfx), is_local_rel });
                    }
                    i = semi + 1;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn match_use_tree(prefix: &str, tree: &str, target_local: &str) -> Option<String> {
    for part in split_top_level_commas(tree) {
        let t = part.trim();
        if t.is_empty() || t.ends_with('*') {
            continue;
        }
        if let Some(open_b) = t.find('{') {
            if let Some(close_b) = t.rfind('}') {
                let sub_pfx = join_use_prefix(prefix, t[..open_b].trim().trim_end_matches("::"));
                if let Some(hit) = match_use_tree(&sub_pfx, &t[open_b + 1..close_b], target_local) {
                    return Some(hit);
                }
                continue;
            }
        }
        if let Some((orig, alias)) = t.rsplit_once(" as ") {
            let a_raw = alias.trim();
            let a = a_raw.strip_prefix("r#").unwrap_or(a_raw);
            if a == target_local {
                let o = orig.trim();
                return Some(if o == "self" { prefix.to_string() } else { join_use_prefix(prefix, o) });
            }
            continue;
        }
        let raw_leaf = t.rsplit("::").next().unwrap_or(t).trim();
        let leaf = raw_leaf.strip_prefix("r#").unwrap_or(raw_leaf);
        if leaf == "self" {
            let p_leaf = prefix.rsplit("::").next().unwrap_or(prefix).trim();
            if p_leaf.strip_prefix("r#").unwrap_or(p_leaf) == target_local {
                return Some(prefix.to_string());
            }
        } else if leaf == target_local {
            return Some(join_use_prefix(prefix, t));
        }
    }
    None
}

fn join_use_prefix(prefix: &str, rhs: &str) -> String {
    let p = prefix.trim().trim_end_matches("::");
    let r = rhs.trim().trim_start_matches("::");
    if p.is_empty() {
        r.to_string()
    } else if r.is_empty() {
        p.to_string()
    } else {
        format!("{p}::{r}")
    }
}
