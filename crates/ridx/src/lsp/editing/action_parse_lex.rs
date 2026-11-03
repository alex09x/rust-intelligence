/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Balanced lexical scanners for declaration headers, generics, and `where` clauses.

use super::action_parse::is_ident_byte;

pub fn next_ident_token(s: &str, from: usize) -> Option<(&str, usize)> {
    let b = s.as_bytes();
    let mut i = from;
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= b.len() {
        return None;
    }
    let start = i;
    if i + 2 < b.len() && b[i] == b'r' && b[i + 1] == b'#' && (b[i + 2].is_ascii_alphabetic() || b[i + 2] == b'_') {
        i += 2;
    } else if !(b[i].is_ascii_alphabetic() || b[i] == b'_') {
        return None;
    }
    while i < b.len() && is_ident_byte(b[i]) {
        i += 1;
    }
    Some((&s[start..i], i))
}

pub fn extract_angle_generics(src: &str, masked: &str, from: usize) -> (String, usize) {
    let mb = masked.as_bytes();
    let mut i = from;
    while i < mb.len() && mb[i].is_ascii_whitespace() {
        i += 1;
    }
    if mb.get(i).copied() != Some(b'<') {
        return (String::new(), from);
    }
    let open = i;
    let (mut angle, mut paren, mut bracket, mut brace) = (0i32, 0i32, 0i32, 0i32);
    while i < mb.len() {
        let prev = if i > 0 { mb[i - 1] } else { 0 };
        match mb[i] {
            b'(' => paren += 1,
            b')' => paren = (paren - 1).max(0),
            b'[' => bracket += 1,
            b']' => bracket = (bracket - 1).max(0),
            b'{' => brace += 1,
            b'}' => brace = (brace - 1).max(0),
            b'<' if paren == 0 && bracket == 0 && brace == 0 => angle += 1,
            b'>' if paren == 0 && bracket == 0 && brace == 0 && prev != b'-' && prev != b'=' => {
                angle -= 1;
                if angle == 0 {
                    return (src[open + 1..i].trim().to_string(), i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    (String::new(), from)
}

pub fn find_header_terminator(mb: &[u8], from: usize) -> Option<usize> {
    let (mut paren, mut bracket, mut angle) = (0i32, 0i32, 0i32);
    let mut i = from;
    while i < mb.len() {
        let prev = if i > 0 { mb[i - 1] } else { 0 };
        match mb[i] {
            b'(' => paren += 1,
            b')' => paren = (paren - 1).max(0),
            b'[' => bracket += 1,
            b']' => bracket = (bracket - 1).max(0),
            b'<' if paren == 0 && bracket == 0 => angle += 1,
            b'>' if paren == 0 && bracket == 0 && prev != b'-' && prev != b'=' => angle = (angle - 1).max(0),
            b'{' | b';' if paren == 0 && bracket == 0 && angle == 0 => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

pub fn split_top_level_where<'a>(src_slice: &'a str, masked_slice: &'a str) -> (&'a str, &'a str, String) {
    let mb = masked_slice.as_bytes();
    let (mut paren, mut bracket, mut brace, mut angle) = (0i32, 0i32, 0i32, 0i32);
    let mut i = 0;
    while i < mb.len() {
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
            _ if paren == 0 && bracket == 0 && brace == 0 && angle == 0 && i + 5 <= mb.len() && &mb[i..i + 5] == b"where" => {
                let prev_ok = i == 0 || !is_ident_byte(mb[i - 1]);
                let next_ok = i + 5 == mb.len() || !is_ident_byte(mb[i + 5]);
                if prev_ok && next_ok {
                    let w = collapse_ws(src_slice[i + 5..].trim());
                    return (&src_slice[..i], &masked_slice[..i], w);
                }
            }
            _ => {}
        }
        i += 1;
    }
    (src_slice, masked_slice, String::new())
}

pub fn split_impl_trait_for<'a>(head_src: &'a str, head_masked: &'a str) -> Option<(&'a str, &'a str)> {
    let mb = head_masked.as_bytes();
    let (mut paren, mut bracket, mut brace, mut angle) = (0i32, 0i32, 0i32, 0i32);
    let mut i = 0;
    while i < mb.len() {
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
            _ if paren == 0 && bracket == 0 && brace == 0 && angle == 0 && i + 3 <= mb.len() && &mb[i..i + 3] == b"for" => {
                let prev_ok = i == 0 || !is_ident_byte(mb[i - 1]);
                let next_ok = i + 3 == mb.len() || !is_ident_byte(mb[i + 3]);
                if prev_ok && next_ok {
                    let mut k = i + 3;
                    while k < mb.len() && mb[k].is_ascii_whitespace() {
                        k += 1;
                    }
                    if mb.get(k).copied() != Some(b'<') {
                        return Some((&head_src[..i], &head_src[i + 3..]));
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

pub fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_ws = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                out.push(' ');
                prev_ws = true;
            }
        } else {
            out.push(ch);
            prev_ws = false;
        }
    }
    out.trim().to_string()
}

#[derive(Debug, Clone)]
pub struct ScannedStringLit {
    pub start_byte: usize,
    pub end_byte: usize,
    pub prefix: &'static str,
    pub is_raw: bool,
    pub hashes: usize,
    pub body_start: usize,
    pub body_end: usize,
}

pub fn find_string_lit_at_cursor(src: &str, cursor_byte: usize) -> Option<ScannedStringLit> {
    let b = src.as_bytes();
    let n = b.len();
    let mut i = 0usize;
    while i < n {
        if i + 1 < n && b[i] == b'/' && b[i + 1] == b'/' {
            while i < n && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < n && b[i] == b'/' && b[i + 1] == b'*' {
            let mut depth = 1usize;
            i += 2;
            while i < n && depth > 0 {
                if i + 1 < n && b[i] == b'/' && b[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if i + 1 < n && b[i] == b'*' && b[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if b[i] == b'\'' {
            if i + 2 < n && b[i + 1] != b'\\' && b[i + 2] == b'\'' {
                i += 3;
                continue;
            }
            if i + 1 < n && b[i + 1] == b'\\' {
                i += 2;
                while i < n && b[i] != b'\'' && b[i] != b'\n' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                if i < n && b[i] == b'\'' {
                    i += 1;
                }
                continue;
            }
            i += 1;
            continue;
        }
        let prev_ident = i > 0 && is_ident_byte(b[i - 1]);
        if !prev_ident {
            if let Some(lit) = try_scan_string_at(b, i) {
                if cursor_byte >= lit.start_byte && cursor_byte <= lit.end_byte {
                    if lit.end_byte < n && is_ident_byte(b[lit.end_byte]) {
                        return None;
                    }
                    return Some(lit);
                }
                i = lit.end_byte;
                continue;
            }
        }
        i += 1;
    }
    None
}

fn try_scan_string_at(b: &[u8], start: usize) -> Option<ScannedStringLit> {
    let n = b.len();
    let (prefix, p) = if start + 2 < n && matches!(b[start], b'b' | b'c') && b[start + 1] == b'r' && (b[start + 2] == b'"' || b[start + 2] == b'#') {
        (if b[start] == b'b' { "b" } else { "c" }, start + 1)
    } else if start + 1 < n && b[start] == b'r' && (b[start + 1] == b'"' || b[start + 1] == b'#') {
        ("", start)
    } else if start + 1 < n && matches!(b[start], b'b' | b'c') && b[start + 1] == b'"' {
        (if b[start] == b'b' { "b" } else { "c" }, start + 1)
    } else if b[start] == b'"' {
        ("", start)
    } else {
        return None;
    };

    if b[p] == b'r' {
        let mut h_end = p + 1;
        while h_end < n && b[h_end] == b'#' {
            h_end += 1;
        }
        if h_end >= n || b[h_end] != b'"' {
            return None;
        }
        let hashes = h_end - (p + 1);
        let body_start = h_end + 1;
        let mut k = body_start;
        while k < n {
            if b[k] == b'"' && k + 1 + hashes <= n && b[k + 1..k + 1 + hashes].iter().all(|&c| c == b'#') {
                return Some(ScannedStringLit { start_byte: start, end_byte: k + 1 + hashes, prefix, is_raw: true, hashes, body_start, body_end: k });
            }
            k += 1;
        }
        return None;
    }

    let body_start = p + 1;
    let mut k = body_start;
    while k < n {
        if b[k] == b'\\' {
            k += 2;
            continue;
        }
        if b[k] == b'"' {
            return Some(ScannedStringLit { start_byte: start, end_byte: k + 1, prefix, is_raw: false, hashes: 0, body_start, body_end: k });
        }
        k += 1;
    }
    None
}

pub fn is_inside_attr_or_extern_abi(masked_doc: &str, lit_start: usize) -> bool {
    let mb = masked_doc.as_bytes();
    let mut b_depth = 0i32;
    for i in (0..lit_start.min(mb.len())).rev() {
        match mb[i] {
            b']' => b_depth += 1,
            b'[' => {
                if b_depth > 0 {
                    b_depth -= 1;
                } else {
                    let prev_non_ws = masked_doc[..i].trim_end();
                    if prev_non_ws.ends_with('#') || prev_non_ws.ends_with("#!") {
                        return true;
                    }
                    break;
                }
            }
            b';' | b'{' | b'}' if b_depth == 0 => break,
            _ => {}
        }
    }
    let before_trim = masked_doc[..lit_start].trim_end();
    before_trim.ends_with("extern") && (before_trim.len() == 6 || !is_ident_byte(before_trim.as_bytes()[before_trim.len() - 7]))
}
