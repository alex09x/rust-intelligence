/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Balanced delimiter and token parsing helpers for `textDocument/typeDefinition`.

use super::type_definition::is_primitive_or_builtin;
use super::type_definition_lookup::format_location;
use crate::lsp::protocol::PositionConverter;
use ridx_format::WorkspaceIndex;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub(super) fn push_component(comp: &str, out: &mut Vec<String>) {
    let c = comp.trim();
    if !c.is_empty() && !is_primitive_or_builtin(c) && !c.contains(['(', ')', '[', ']', '{', '}', ' ']) && !out.iter().any(|x| x == c) {
        out.push(c.to_string());
    }
}

pub(super) fn is_fn_type(s: &str) -> bool {
    s.starts_with("fn(") || s.starts_with("unsafe fn(") || s.starts_with("extern ")
}

pub(super) fn strip_leading_for_binder(mut s: &str) -> &str {
    while let Some(rest) = s.strip_prefix("for<") {
        if let Some(gt) = find_matching_angle(s, 3) {
            s = s[gt + 1..].trim_start();
        } else {
            let _ = rest;
            break;
        }
    }
    s
}

pub(super) fn peel_all_modifiers(mut ty: &str) -> &str {
    loop {
        ty = ty.trim();
        if let Some(rest) = ty.strip_prefix('&') {
            let mut inner = rest.trim_start();
            if inner.starts_with('\'') {
                let Some(idx) = inner.find(|c: char| c.is_whitespace() || c == '&') else {
                    break;
                };
                inner = inner[idx..].trim_start();
            }
            if let Some(after_mut) = inner.strip_prefix("mut") {
                if after_mut.starts_with(char::is_whitespace) || after_mut.starts_with(['&', '[', '(']) {
                    inner = after_mut.trim_start();
                }
            }
            ty = inner;
            continue;
        }
        if let Some(rest) = ["*const ", "*mut ", "dyn ", "impl "].iter().find_map(|p| ty.strip_prefix(p)) {
            ty = rest;
            continue;
        }
        break;
    }
    ty
}

pub(super) fn find_top_level_char(s: &str, target: char) -> Option<usize> {
    let (mut angle, mut paren, mut bracket, mut brace) = (0i32, 0i32, 0i32, 0i32);
    let bytes = s.as_bytes();
    for (i, ch) in s.char_indices() {
        if angle == 0 && paren == 0 && bracket == 0 && brace == 0 && ch == target {
            return Some(i);
        }
        match ch {
            '<' => angle += 1,
            '>' if i > 0 && bytes[i - 1] == b'-' => {}
            '>' => angle = (angle - 1).max(0),
            '(' => paren += 1,
            ')' => paren = (paren - 1).max(0),
            '[' => bracket += 1,
            ']' => bracket = (bracket - 1).max(0),
            '{' => brace += 1,
            '}' => brace = (brace - 1).max(0),
            _ => {}
        }
    }
    None
}

pub(super) fn find_matching_angle(s: &str, open_byte_idx: usize) -> Option<usize> {
    let mut depth = 0i32;
    let bytes = s.as_bytes();
    for (off, ch) in s[open_byte_idx..].char_indices() {
        let idx = open_byte_idx + off;
        match ch {
            '<' => depth += 1,
            '>' if idx > 0 && bytes[idx - 1] == b'-' => {}
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

pub(super) fn find_matching_delim(s: &str, open_byte_idx: usize, open_ch: char, close_ch: char) -> Option<usize> {
    let mut depth = 0i32;
    for (off, ch) in s[open_byte_idx..].char_indices() {
        if ch == open_ch {
            depth += 1;
        } else if ch == close_ch {
            depth -= 1;
            if depth == 0 {
                return Some(open_byte_idx + off);
            }
        }
    }
    None
}

pub(super) fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut angle, mut paren, mut bracket, mut brace) = (0i32, 0i32, 0i32, 0i32);
    let mut start = 0;
    let bytes = s.as_bytes();
    for (i, ch) in s.char_indices() {
        if angle == 0 && paren == 0 && bracket == 0 && brace == 0 && ch == sep {
            parts.push(s[start..i].trim());
            start = i + ch.len_utf8();
            continue;
        }
        match ch {
            '<' => angle += 1,
            '>' if i > 0 && bytes[i - 1] == b'-' => {}
            '>' => angle = (angle - 1).max(0),
            '(' => paren += 1,
            ')' => paren = (paren - 1).max(0),
            '[' => bracket += 1,
            ']' => bracket = (bracket - 1).max(0),
            '{' => brace += 1,
            '}' => brace = (brace - 1).max(0),
            _ => {}
        }
    }
    let tail = s[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts
}

pub(super) fn split_top_level_keyword<'a>(s: &'a str, kw: &str) -> Option<(&'a str, &'a str)> {
    let (mut angle, mut paren, mut bracket, mut brace) = (0i32, 0i32, 0i32, 0i32);
    let bytes = s.as_bytes();
    for (i, ch) in s.char_indices() {
        if angle == 0 && paren == 0 && bracket == 0 && brace == 0 && s[i..].starts_with(kw) {
            return Some((s[..i].trim(), s[i + kw.len()..].trim()));
        }
        match ch {
            '<' => angle += 1,
            '>' if i > 0 && bytes[i - 1] == b'-' => {}
            '>' => angle = (angle - 1).max(0),
            '(' => paren += 1,
            ')' => paren = (paren - 1).max(0),
            '[' => bracket += 1,
            ']' => bracket = (bracket - 1).max(0),
            '{' => brace += 1,
            '}' => brace = (brace - 1).max(0),
            _ => {}
        }
    }
    None
}

pub(super) fn following_angle_bracket_cols(line_str: &str, col_1based: u32) -> Option<(u32, u32, &str)> {
    let chars: Vec<(usize, char)> = line_str.char_indices().collect();
    let idx = (col_1based as usize).checked_sub(1).filter(|&i| i < chars.len())?;
    let is_id = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut end = idx;
    while end < chars.len() && is_id(chars[end].1) {
        end += 1;
    }
    if end + 2 < chars.len() && chars[end].1 == ':' && chars[end + 1].1 == ':' && chars[end + 2].1 == '<' {
        end += 2;
    }
    if end >= chars.len() || chars[end].1 != '<' {
        return None;
    }
    let open_byte = chars[end].0;
    let close_byte = find_matching_angle(line_str, open_byte)?;
    let close_char_idx = chars.iter().position(|&(b, _)| b == close_byte)?;
    Some(((end as u32) + 1, (close_char_idx as u32) + 2, &line_str[open_byte + 1..close_byte]))
}

pub(crate) fn sysroot_rust_src() -> Option<&'static Path> {
    static RUST_SRC: OnceLock<Option<PathBuf>> = OnceLock::new();
    RUST_SRC
        .get_or_init(|| {
            if let Some(env_p) = std::env::var_os("RUST_SRC_PATH").map(PathBuf::from) {
                if env_p.is_dir() {
                    return Some(env_p);
                }
            }
            let out = std::process::Command::new("rustc").arg("--print").arg("sysroot").output().ok()?;
            let sysroot = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let p = PathBuf::from(sysroot).join("lib/rustlib/src/rust");
            p.is_dir().then_some(p)
        })
        .as_deref()
}

pub(super) fn lookup_unstable_sysroot_type(ws: &WorkspaceIndex, target_path: &str, conv: &mut PositionConverter) -> Option<Value> {
    if !ws.has_sysroot() || !matches!(target_path, "alloc::alloc::Global" | "std::alloc::Global") {
        return None;
    }
    let rel = "library/alloc/src/alloc.rs";
    let disk_path = sysroot_rust_src()?.join(rel);
    let disk_str = disk_path.to_str()?;
    for ln in 1..=120 {
        let line_txt = conv.get_line(disk_str, ln)?;
        if let Some(byte_idx) = line_txt.find("pub struct Global;") {
            let col = line_txt[..byte_idx].chars().count() as u32 + 12;
            return Some(format_location(Path::new(rel), rel, (ln, col, ln, col + 6), conv));
        }
    }
    None
}

pub(super) fn extract_rmeta_default_generics(resolved_file: &Path, s_line: u32, conv: &mut PositionConverter) -> Vec<String> {
    let disk_buf;
    let read_path: &Path = if !resolved_file.exists() && resolved_file.starts_with("library/") {
        let Some(root) = sysroot_rust_src() else {
            return Vec::new();
        };
        disk_buf = root.join(resolved_file);
        &disk_buf
    } else {
        resolved_file
    };
    let Some(path_str) = read_path.to_str() else {
        return Vec::new();
    };
    let mut header = String::new();
    for ln in s_line..=(s_line + 6) {
        let Some(l_txt) = conv.get_line(path_str, ln) else {
            break;
        };
        header.push_str(l_txt);
        header.push(' ');
        if l_txt.contains('{') || l_txt.contains(';') {
            break;
        }
    }
    let mut out = Vec::new();
    if header.contains("= RandomState") {
        out.push("std::hash::random::RandomState".to_string());
    }
    if header.contains("= Global") {
        out.push("alloc::alloc::Global".to_string());
    }
    out
}

pub(super) fn find_macro_template_trait_loc(resolved_file: &Path, raw_file: &str, def_name: &str, call_line: u32, conv: &mut PositionConverter) -> Option<Value> {
    if def_name.is_empty() {
        return None;
    }
    let path_str = resolved_file.to_str().unwrap_or(raw_file);
    let call_txt = conv.get_line(path_str, call_line)?;
    if !call_txt.contains("!(") {
        return None;
    }
    let pat = format!("pub trait {def_name}");
    for ln in 1..call_line {
        let Some(line_txt) = conv.get_line(path_str, ln) else {
            break;
        };
        if let Some(byte_idx) = line_txt.find(&pat) {
            let after = &line_txt[byte_idx + pat.len()..];
            if after.is_empty() || after.starts_with(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
                let col = line_txt[..byte_idx].chars().count() as u32 + 11;
                let end_col = col + def_name.chars().count() as u32;
                return Some(format_location(resolved_file, raw_file, (ln, col, ln, end_col), conv));
            }
        }
    }
    None
}
