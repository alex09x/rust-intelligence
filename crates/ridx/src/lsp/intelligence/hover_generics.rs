/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Generics and `where` clauses in rust-analyzer's hover layout: parameter bounds moved out of the parameter list
//! into `where`, one bound per line, and the enclosing generic `impl` shown above a method with its bounds listed
//! under `// Bounds from impl:`.

use super::hover_decl::{collapse, skip_trivia, split_top, strip_comment};

/// `header` with its `where` clause, if any, moved to rust-analyzer's layout: `where` on its own line, one bound per
/// line, each with a trailing comma.
pub(crate) fn with_where_layout(header: &str) -> String {
    let collapsed = collapse(header);
    let (collapsed, mut bounds) = hoist_inline_bounds(&collapsed);
    let head = match top_level_where(&collapsed) {
        Some(at) => {
            bounds.extend(split_top(&collapsed[at + "where".len()..]).into_iter().map(|b| b.trim().to_string()));
            collapsed[..at].trim_end().to_string()
        }
        None => collapsed,
    };
    if bounds.is_empty() {
        return head;
    }
    let mut out = format!("{head}\nwhere");
    for bound in bounds {
        out.push_str(&format!("\n    {bound},"));
    }
    out
}

/// The item's generic parameter bounds moved out of its parameter list, as rust-analyzer prints them:
/// `fn f<I: Integer, A: Alloc = Global>` becomes `fn f<I, A = Global>` with bounds `I: Integer`, `A: Alloc`.
fn hoist_inline_bounds(header: &str) -> (String, Vec<String>) {
    let limit = header.find('(').unwrap_or(header.len());
    let after_name = |i: usize| i > 0 && (header.as_bytes()[i - 1].is_ascii_alphanumeric() || header.as_bytes()[i - 1] == b'_');
    let Some(open) = header[..limit].find('<').filter(|&i| after_name(i)) else {
        return (header.to_string(), Vec::new());
    };
    let Some(close) = generics_close(&header[open + 1..]).map(|c| open + 1 + c) else { return (header.to_string(), Vec::new()) };
    let mut params = Vec::new();
    let mut bounds = Vec::new();
    for param in split_top(&header[open + 1..close]) {
        let param = param.trim();
        if param.starts_with("const ") {
            params.push(param.to_string());
            continue;
        }
        let (decl, default) = match split_top_eq(param) {
            Some((d, v)) => (d.trim(), Some(v.trim())),
            None => (param, None),
        };
        let name = match decl.split_once(':') {
            Some((name, bound)) => {
                bounds.push(format!("{}: {}", name.trim(), bound.trim()));
                name.trim()
            }
            None => decl,
        };
        params.push(match default {
            Some(v) => format!("{name} = {v}"),
            None => name.to_string(),
        });
    }
    if bounds.is_empty() {
        return (header.to_string(), bounds);
    }
    (format!("{}<{}>{}", &header[..open], params.join(", "), &header[close + 1..]), bounds)
}

/// `param` split at a `=` outside brackets (a default), not a `=>`.
fn split_top_eq(param: &str) -> Option<(&str, &str)> {
    let b = param.as_bytes();
    let mut depth = 0i32;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'<' | b'(' | b'[' => depth += 1,
            b'>' if i > 0 && b[i - 1] == b'-' => {}
            b'>' | b')' | b']' => depth -= 1,
            b'=' if depth == 0 && b.get(i + 1) != Some(&b'>') => return Some((&param[..i], &param[i + 1..])),
            _ => {}
        }
    }
    None
}

/// Byte offset of a `where` keyword outside brackets and generics.
fn top_level_where(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0i32;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'(' | b'[' | b'<' => depth += 1,
            b')' | b']' => depth -= 1,
            b'>' if i > 0 && b[i - 1] != b'-' && b[i - 1] != b'=' => depth -= 1,
            b'w' if depth == 0 && s[i..].starts_with("where") && (i == 0 || b[i - 1] == b' ') && b.get(i + 5).is_none_or(|n| !n.is_ascii_alphanumeric() && *n != b'_') => {
                return Some(i);
            }
            _ => {}
        }
    }
    None
}

/// The `impl` header enclosing the item at `start` when it declares generics, as rust-analyzer shows it above a
/// method: parameter names only (`impl<T, A> Vec<T, A>`), with the bounds returned apart. `None` for a plain
/// `impl Type` or when no enclosing impl is found.
pub(crate) fn generic_impl_header(src: &str, start: usize) -> Option<(String, Vec<String>)> {
    let mut depth = 0i32;
    let mut open = None;
    let mut line_end = start;
    'lines: loop {
        let line_begin = src[..line_end].rfind('\n').map_or(0, |p| p + 1);
        let code = strip_comment(&src[line_begin..line_end]);
        for (i, c) in code.char_indices().rev() {
            match c {
                '}' => depth += 1,
                '{' if depth == 0 => {
                    open = Some(line_begin + i);
                    break 'lines;
                }
                '{' => depth -= 1,
                _ => {}
            }
        }
        if line_begin == 0 {
            break;
        }
        line_end = line_begin - 1;
    }
    let open = open?;
    let from = src[..open].rfind(['}', ';', '{']).map_or(0, |p| p + 1);
    let from = skip_trivia(src, from);
    let header = collapse(&src[from..open]);
    let header = header.trim_start_matches("unsafe ").trim_start_matches("const ").trim_start_matches("unsafe ");
    let rest = header.strip_prefix("impl<")?;
    let close = generics_close(rest)?;
    let (params, after) = (&rest[..close], rest[close + 1..].trim_start());
    let (target, where_bounds) = match top_level_where(after) {
        Some(w) => (after[..w].trim_end(), split_top(&after[w + "where".len()..]).into_iter().map(|b| b.trim().to_string()).collect()),
        None => (after, Vec::new()),
    };
    let mut names = Vec::new();
    let mut bounds = Vec::new();
    for param in split_top(params) {
        let param = param.trim();
        if param.starts_with("const ") {
            names.push(param.to_string());
            continue;
        }
        match param.split_once(':') {
            Some((name, bound)) => {
                names.push(name.trim().to_string());
                bounds.push(format!("{}: {}", name.trim(), bound.trim()));
            }
            None => names.push(param.to_string()),
        }
    }
    bounds.extend(where_bounds);
    Some((format!("impl<{}> {target}", names.join(", ")), bounds))
}

/// The `>` closing generics whose `<` precedes `text`.
fn generics_close(text: &str) -> Option<usize> {
    let b = text.as_bytes();
    let mut depth = 0i32;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'<' | b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b'>' if i > 0 && b[i - 1] == b'-' => {}
            b'>' if depth == 0 => return Some(i),
            b'>' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// A method header with its enclosing impl's bounds added to its `where` clause, as rust-analyzer shows them.
pub(crate) fn with_impl_bounds(method: String, impl_bounds: &[String]) -> String {
    if impl_bounds.is_empty() {
        return method;
    }
    let mut out = method;
    if !out.contains("\nwhere") {
        out.push_str("\nwhere");
    }
    out.push_str("\n    // Bounds from impl:");
    for b in impl_bounds {
        out.push_str(&format!("\n    {b},"));
    }
    out
}
