/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Substitution, lifetime materialization, and generic parameter helpers for `fn` type aliases.

use super::action_parse::{is_ident_byte, mask_comments_and_strings};
use super::action_parse_decl::collapse_ws;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenKind {
    Lifetime,
    Type,
    Const,
}

#[derive(Debug, Clone)]
pub struct GenericDecl {
    pub name: String,
    pub kind: GenKind,
    pub head_no_bounds: String,
    pub inline_bounds: Vec<String>,
}

pub fn split_top_level_commas(src: &str) -> Vec<String> {
    let masked = mask_comments_and_strings(src);
    let mb = masked.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let (mut paren, mut bracket, mut brace, mut angle) = (0i32, 0i32, 0i32, 0i32);
    for i in 0..mb.len() {
        let prev = if i > 0 { mb[i - 1] } else { 0 };
        match mb[i] {
            b'(' => paren += 1,
            b')' => paren = (paren - 1).max(0),
            b'[' => bracket += 1,
            b']' => bracket = (bracket - 1).max(0),
            b'{' => brace += 1,
            b'}' => brace = (brace - 1).max(0),
            b'<' if paren == 0 && bracket == 0 && brace == 0 => angle += 1,
            b'>' if paren == 0 && bracket == 0 && brace == 0 && prev != b'-' && prev != b'=' => angle = (angle - 1).max(0),
            b',' if paren == 0 && bracket == 0 && brace == 0 && angle == 0 => {
                let piece = src[start..i].trim();
                if !piece.is_empty() {
                    out.push(piece.to_string());
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    let tail = src[start..].trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

pub fn strip_leading_attrs_keep_cfg(param: &str) -> (String, String) {
    let masked = mask_comments_and_strings(param);
    let mb = masked.as_bytes();
    let sb = param.as_bytes();
    let mut i = 0usize;
    let mut cfgs = Vec::new();
    loop {
        while i < mb.len() && mb[i].is_ascii_whitespace() {
            i += 1;
        }
        if mb.get(i).copied() == Some(b'#') {
            let mut k = i + 1;
            while k < mb.len() && mb[k].is_ascii_whitespace() {
                k += 1;
            }
            if mb.get(k).copied() == Some(b'[') {
                if let Some(end_b) = super::action_parse_items::skip_balanced(mb, k, b'[', b']') {
                    let inner = param[k + 1..end_b - 1].trim();
                    if inner.starts_with("cfg(") {
                        cfgs.push(collapse_ws(&param[i..end_b]));
                    }
                    i = end_b;
                    continue;
                }
            }
        }
        break;
    }
    let cfg_prefix = if cfgs.is_empty() { String::new() } else { format!("{} ", cfgs.join(" ")) };
    let rest = if i <= sb.len() { param[i..].trim().to_string() } else { String::new() };
    (cfg_prefix, rest)
}

pub fn parse_generics_list(generics_src: &str) -> Vec<GenericDecl> {
    let mut out = Vec::new();
    for raw in split_top_level_commas(generics_src) {
        let (_, clean) = strip_leading_attrs_keep_cfg(&raw);
        let no_default = strip_top_level_eq_default(&clean);
        let trimmed = no_default.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('\'') {
            let (lt, bounds) = split_top_level_colon(trimmed);
            let name = lt.trim().to_string();
            let inline_bounds = bounds.map(|b| vec![collapse_ws(b)]).unwrap_or_default();
            out.push(GenericDecl { name: name.clone(), kind: GenKind::Lifetime, head_no_bounds: name, inline_bounds });
        } else if let Some(rest) = trimmed.strip_prefix("const ") {
            let rest = rest.trim_start();
            if let (cname, Some(cty)) = split_top_level_colon(rest) {
                let name = cname.trim().strip_prefix("r#").unwrap_or(cname.trim()).to_string();
                out.push(GenericDecl { name, kind: GenKind::Const, head_no_bounds: format!("const {}: {}", cname.trim(), collapse_ws(cty)), inline_bounds: Vec::new() });
            }
        } else {
            let (tname, bounds) = split_top_level_colon(trimmed);
            let raw_t = tname.trim();
            let name = raw_t.strip_prefix("r#").unwrap_or(raw_t).to_string();
            if name.is_empty() {
                continue;
            }
            let inline_bounds = bounds.map(|b| split_top_level_plus(b).into_iter().map(|s| collapse_ws(&s)).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
            out.push(GenericDecl { name, kind: GenKind::Type, head_no_bounds: raw_t.to_string(), inline_bounds });
        }
    }
    out
}

pub fn split_top_level_colon(s: &str) -> (&str, Option<&str>) {
    let b = s.as_bytes();
    let (mut paren, mut bracket, mut brace, mut angle) = (0i32, 0i32, 0i32, 0i32);
    let mut i = 0;
    while i < b.len() {
        let prev = if i > 0 { b[i - 1] } else { 0 };
        match b[i] {
            b'(' => paren += 1,
            b')' => paren = (paren - 1).max(0),
            b'[' => bracket += 1,
            b']' => bracket = (bracket - 1).max(0),
            b'{' => brace += 1,
            b'}' => brace = (brace - 1).max(0),
            b'<' if paren == 0 && bracket == 0 && brace == 0 => angle += 1,
            b'>' if paren == 0 && bracket == 0 && brace == 0 && prev != b'-' && prev != b'=' => angle = (angle - 1).max(0),
            b':' if paren == 0 && bracket == 0 && brace == 0 && angle == 0 => {
                let next_colon = b.get(i + 1).copied() == Some(b':');
                let prev_colon = i > 0 && b[i - 1] == b':';
                if !next_colon && !prev_colon {
                    return (&s[..i], Some(s[i + 1..].trim()));
                }
            }
            _ => {}
        }
        i += 1;
    }
    (s, None)
}

fn strip_top_level_eq_default(s: &str) -> &str {
    let b = s.as_bytes();
    let (mut paren, mut bracket, mut brace, mut angle) = (0i32, 0i32, 0i32, 0i32);
    for i in 0..b.len() {
        let prev = if i > 0 { b[i - 1] } else { 0 };
        match b[i] {
            b'(' => paren += 1,
            b')' => paren = (paren - 1).max(0),
            b'[' => bracket += 1,
            b']' => bracket = (bracket - 1).max(0),
            b'{' => brace += 1,
            b'}' => brace = (brace - 1).max(0),
            b'<' if paren == 0 && bracket == 0 && brace == 0 => angle += 1,
            b'>' if paren == 0 && bracket == 0 && brace == 0 && prev != b'-' && prev != b'=' => angle = (angle - 1).max(0),
            b'=' if paren == 0 && bracket == 0 && brace == 0 && angle == 0 && b.get(i + 1).copied() != Some(b'>') && prev != b'=' && prev != b'!' => {
                return s[..i].trim();
            }
            _ => {}
        }
    }
    s
}

fn split_top_level_plus(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let (mut paren, mut bracket, mut brace, mut angle) = (0i32, 0i32, 0i32, 0i32);
    for i in 0..b.len() {
        let prev = if i > 0 { b[i - 1] } else { 0 };
        match b[i] {
            b'(' => paren += 1,
            b')' => paren = (paren - 1).max(0),
            b'[' => bracket += 1,
            b']' => bracket = (bracket - 1).max(0),
            b'{' => brace += 1,
            b'}' => brace = (brace - 1).max(0),
            b'<' if paren == 0 && bracket == 0 && brace == 0 => angle += 1,
            b'>' if paren == 0 && bracket == 0 && brace == 0 && prev != b'-' && prev != b'=' => angle = (angle - 1).max(0),
            b'+' if paren == 0 && bracket == 0 && brace == 0 && angle == 0 => {
                out.push(s[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(s[start..].trim().to_string());
    out
}

pub fn materialize_elided_lifetimes(ty_str: &str, counter: &mut usize, fresh: &mut Vec<GenericDecl>) -> String {
    let b = ty_str.as_bytes();
    let mut out = String::with_capacity(ty_str.len() + 16);
    let mut i = 0usize;
    while i < b.len() {
        if i + 2 <= b.len() && &b[i..i + 2] == b"'_" && (i + 2 == b.len() || !is_ident_byte(b[i + 2])) {
            let lt = format!("'ridx_l{}", *counter);
            *counter += 1;
            fresh.push(GenericDecl { name: lt.clone(), kind: GenKind::Lifetime, head_no_bounds: lt.clone(), inline_bounds: Vec::new() });
            out.push_str(&lt);
            i += 2;
        } else if b[i] == b'&' {
            let mut k = i + 1;
            while k < b.len() && b[k].is_ascii_whitespace() {
                k += 1;
            }
            if b.get(k).copied() == Some(b'\'') {
                out.push('&');
                i += 1;
            } else {
                let lt = format!("'ridx_l{}", *counter);
                *counter += 1;
                fresh.push(GenericDecl { name: lt.clone(), kind: GenKind::Lifetime, head_no_bounds: lt.clone(), inline_bounds: Vec::new() });
                out.push('&');
                out.push_str(&lt);
                out.push(' ');
                i = k;
            }
        } else {
            let ch = ty_str[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

pub fn collect_tokens_and_lifetimes(s: &str) -> HashSet<String> {
    let masked = mask_comments_and_strings(s);
    let b = masked.as_bytes();
    let mut set = HashSet::new();
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == b'\'' && i + 1 < b.len() && (b[i + 1].is_ascii_alphabetic() || b[i + 1] == b'_') {
            let start = i;
            i += 2;
            while i < b.len() && is_ident_byte(b[i]) {
                i += 1;
            }
            set.insert(masked[start..i].to_string());
        } else if b[i] == b'r' && i + 2 < b.len() && b[i + 1] == b'#' && (b[i + 2].is_ascii_alphabetic() || b[i + 2] == b'_') {
            let start = i + 2;
            i += 3;
            while i < b.len() && is_ident_byte(b[i]) {
                i += 1;
            }
            set.insert(masked[start..i].to_string());
        } else if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            let start = i;
            i += 1;
            while i < b.len() && is_ident_byte(b[i]) {
                i += 1;
            }
            set.insert(masked[start..i].to_string());
        } else {
            i += 1;
        }
    }
    set
}

pub fn replace_self_tokens<F>(input: &str, self_sub: &str, mut assoc_resolver: F) -> Option<String>
where
    F: FnMut(&str) -> Option<String>,
{
    let b = input.as_bytes();
    let mut out = String::with_capacity(input.len() + 32);
    let mut i = 0usize;
    while i < b.len() {
        if i + 4 <= b.len() && &b[i..i + 4] == b"Self" {
            let prev_ok = i == 0 || (!is_ident_byte(b[i - 1]) && b[i - 1] != b'#');
            let next_ok = i + 4 == b.len() || !is_ident_byte(b[i + 4]);
            if prev_ok && next_ok {
                let mut k = i + 4;
                while k < b.len() && b[k].is_ascii_whitespace() {
                    k += 1;
                }
                if k + 2 <= b.len() && &b[k..k + 2] == b"::" {
                    let mut m = k + 2;
                    while m < b.len() && b[m].is_ascii_whitespace() {
                        m += 1;
                    }
                    let assoc_start = m;
                    while m < b.len() && is_ident_byte(b[m]) {
                        m += 1;
                    }
                    if m > assoc_start {
                        let assoc = &input[assoc_start..m];
                        let rep = assoc_resolver(assoc)?;
                        out.push_str(&rep);
                        i = m;
                        continue;
                    }
                    return None;
                }
                let prev_amp = out.trim_end().ends_with('&') || out.trim_end().ends_with("mut");
                if prev_amp && (self_sub.starts_with("dyn ") || self_sub.contains(" + ")) {
                    out.push('(');
                    out.push_str(self_sub);
                    out.push(')');
                } else {
                    out.push_str(self_sub);
                }
                i += 4;
                continue;
            }
        }
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    Some(out)
}
