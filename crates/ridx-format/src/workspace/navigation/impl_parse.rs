/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Path parsing and generic parameter helpers for `DefKind::Impl` paths.

use super::strip_crate_prefix;
use crate::format::{fnv1a_64, DefKind};
use crate::reader::IndexReader;

pub(crate) fn has_fn_at_path(reader: &IndexReader<memmap2::Mmap>, path: &str) -> bool {
    let hash = fnv1a_64(path.as_bytes());
    let defs_hash = reader.defs_by_hash();
    let defs = reader.definitions();
    let Ok(idx) = defs_hash.binary_search_by_key(&hash, |e| e.hash()) else { return false };
    let mut i = idx;
    while i > 0 && defs_hash[i - 1].hash() == hash {
        i -= 1;
    }
    while i < defs_hash.len() && defs_hash[i].hash() == hash {
        let d = &defs[defs_hash[i].index() as usize];
        if reader.get_str(d.path_offset()) == path && matches!(DefKind::from(d.kind), DefKind::Fn | DefKind::AssocFn | DefKind::Closure) {
            return true;
        }
        i += 1;
    }
    false
}

pub(crate) fn is_impl_inside_fn(reader: &IndexReader<memmap2::Mmap>, raw_impl: &str, self_ty: &str) -> bool {
    let st_base = strip_tail_generics(self_ty);
    if st_base.contains("::<impl ") || st_base.starts_with('<') || st_base.contains("::{closure#") || raw_impl.contains("::{closure#") {
        return true;
    }
    let r_crate = reader.crate_name();
    let st_local = strip_crate_prefix(st_base, r_crate).unwrap_or(st_base);
    if let Some((parent, _)) = st_local.rsplit_once("::") {
        let pc = parent.strip_suffix("::_").unwrap_or(parent);
        if has_fn_at_path(reader, pc) || has_fn_at_path(reader, &format!("{r_crate}::{pc}")) {
            return true;
        }
    }
    if let Some(open_idx) = matching_angle_open_for_end(raw_impl) {
        if open_idx >= 2 && raw_impl[open_idx..].starts_with("<impl ") {
            let prefix = raw_impl[..open_idx - 2].strip_suffix("::_").unwrap_or(&raw_impl[..open_idx - 2]);
            if prefix.contains("::<impl ") {
                return true;
            }
            let prefix_local = strip_crate_prefix(prefix, r_crate).unwrap_or(prefix);
            if has_fn_at_path(reader, prefix_local) || has_fn_at_path(reader, prefix) {
                return true;
            }
        }
    }
    false
}

pub(crate) fn parse_impl_def_path(raw: &str) -> (Option<&str>, &str) {
    let raw = raw.trim();
    if let Some(open_idx) = matching_angle_open_for_end(raw) {
        if open_idx == 0 {
            let inner = &raw[1..raw.len() - 1];
            if let Some(as_pos) = find_top_level_keyword(inner, " as ") {
                let self_ty = inner[..as_pos].trim();
                let trait_path = inner[as_pos + 4..].trim();
                return (Some(trait_path), self_ty);
            }
            return (None, inner.trim());
        }
        if raw[open_idx..].starts_with("<impl ") {
            let mut inner = raw[open_idx + 6..raw.len() - 1].trim();
            if inner.starts_with('<') {
                if let Some(close_idx) = matching_angle_close(inner) {
                    if inner.as_bytes().get(close_idx + 1).is_some_and(|b| b.is_ascii_whitespace()) {
                        inner = inner[close_idx + 1..].trim();
                    }
                }
            }
            if let Some(for_pos) = find_top_level_keyword(inner, " for ") {
                let trait_path = inner[..for_pos].trim();
                let self_ty = inner[for_pos + 5..].trim();
                return (Some(trait_path), self_ty);
            }
            return (None, inner);
        }
    }
    (None, raw)
}

pub(crate) fn matching_angle_open_for_end(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes[bytes.len() - 1] != b'>' {
        return None;
    }
    let mut depth = 0i32;
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        if bytes[i] == b'>' && (i == 0 || bytes[i - 1] != b'-') {
            depth += 1;
        } else if bytes[i] == b'<' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

pub(crate) fn matching_angle_close(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    for i in 0..bytes.len() {
        if bytes[i] == b'<' {
            depth += 1;
        } else if bytes[i] == b'>' && (i == 0 || bytes[i - 1] != b'-') {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

pub(crate) fn strip_tail_generics(s: &str) -> &str {
    let s = s.trim();
    let bytes = s.as_bytes();
    let mut angle = 0i32;
    let mut last_sep = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            angle += 1;
        } else if bytes[i] == b'>' && (i == 0 || bytes[i - 1] != b'-') {
            angle = (angle - 1).max(0);
        } else if angle == 0 && i + 1 < bytes.len() && bytes[i] == b':' && bytes[i + 1] == b':' {
            last_sep = i + 2;
            i += 2;
            continue;
        }
        i += 1;
    }
    let tail = &s[last_sep..];
    if let Some(rel_lt) = tail.find('<') {
        if rel_lt > 0 {
            return s[..last_sep + rel_lt].trim_end();
        }
    }
    s
}

pub(crate) fn has_multiple_generic_params(self_ty: &str) -> bool {
    let s = self_ty.trim();
    let Some(open_idx) = matching_angle_open_for_end(s) else { return false };
    if open_idx == 0 {
        return false;
    }
    let inner = &s[open_idx + 1..s.len() - 1];
    find_top_level_keyword(inner, ",").is_some()
}

pub(crate) fn find_top_level_keyword(s: &str, kw: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let kw_bytes = kw.as_bytes();
    if bytes.len() < kw_bytes.len() {
        return None;
    }
    let mut angle = 0i32;
    let mut paren = 0i32;
    for i in 0..=bytes.len() - kw_bytes.len() {
        match bytes[i] {
            b'<' => angle += 1,
            b'>' if i == 0 || bytes[i - 1] != b'-' => angle = (angle - 1).max(0),
            b'(' | b'[' => paren += 1,
            b')' | b']' => paren = (paren - 1).max(0),
            _ => {}
        }
        if angle == 0 && paren == 0 && &bytes[i..i + kw_bytes.len()] == kw_bytes {
            return Some(i);
        }
    }
    None
}

pub(crate) fn is_nominal_type_syntax(self_ty: &str) -> bool {
    let s = self_ty.trim_start();
    !s.is_empty() && !s.starts_with('&') && !s.starts_with('*') && !s.starts_with('(') && !s.starts_with('[')
}

pub(crate) fn is_builtin_derive_trait(trait_path: Option<&str>) -> bool {
    let Some(tp) = trait_path else { return false };
    let base = strip_tail_generics(tp);
    let short = base.rsplit("::").next().unwrap_or(base);
    matches!(short, "Clone" | "TrivialClone" | "Copy" | "Debug" | "Default" | "PartialEq" | "StructuralPartialEq" | "Eq" | "PartialOrd" | "Ord" | "Hash" | "Unpin")
        && (base == short
            || base.starts_with("core::")
            || base.starts_with("std::")
            || base.starts_with("fmt::")
            || base.starts_with("clone::")
            || base.starts_with("marker::")
            || base.starts_with("cmp::")
            || base.starts_with("hash::")
            || base.starts_with("default::"))
}

pub(crate) fn contains_ident_token(line: &str, ident: &str) -> bool {
    if ident.is_empty() {
        return false;
    }
    let bytes = line.as_bytes();
    let id_len = ident.len();
    let mut pos = 0;
    while let Some(rel) = line[pos..].find(ident) {
        let idx = pos + rel;
        let prev_ok = idx == 0 || !(bytes[idx - 1].is_ascii_alphanumeric() || bytes[idx - 1] == b'_');
        let next_idx = idx + id_len;
        let next_ok = next_idx >= bytes.len() || !(bytes[next_idx].is_ascii_alphanumeric() || bytes[next_idx] == b'_');
        if prev_ok && next_ok {
            return true;
        }
        pos = idx + 1;
    }
    false
}
