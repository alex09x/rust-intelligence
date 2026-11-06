/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Parameter-name hints before call arguments (`ty:`), with rust-analyzer's rules for leaving out the obvious
//! ones. Parameter names come from the callee's declaration in source (the standard library's from `rust-src`).

use super::{Hint, SourceMap};
use ridx_format::{DefKind, WorkspaceIndex};
use std::collections::HashMap;

pub fn parameter_hints(src: &SourceMap<'_>, ws: &WorkspaceIndex, file: &str, lines: (u32, u32)) -> Vec<Hint> {
    let mut out = Vec::new();
    let mut names_cache: HashMap<(String, u32), Option<Vec<Option<String>>>> = HashMap::new();
    let (Some(from), to) = (src.line_start(lines.0 as usize), src.line_start(lines.1 as usize + 1).unwrap_or(src.masked.len())) else { return out };
    let m = src.masked;
    let b = m.as_bytes();
    let in_macro = macro_argument_ranges(m);
    let mut i = from;
    while i < to {
        if b[i] != b'(' {
            i += 1;
            continue;
        }
        // rust-analyzer shows no parameter hints for calls inside a macro's arguments (`assert_eq!(f(x), y)`).
        if in_macro.iter().any(|&(s, e)| s < i && i < e) {
            i += 1;
            continue;
        }
        let open = i;
        i += 1;
        let Some((name_start, name_end, is_method)) = callee_before(m, open) else { continue };
        let name = &m[name_start..name_end];
        let Some(close) = matching_paren(b, open) else { continue };
        // Structure from the masked text, contents from the source (a string literal argument is blank in the mask).
        let args: Vec<(usize, usize)> = split_args(m, open + 1, close).into_iter().filter(|&(s, e)| !src.text[s..e].trim().is_empty()).collect();
        if args.is_empty() {
            continue;
        }
        let (l0, c0) = src.line_col(name_start);
        let Some(hover) = ws.hover_at(file, l0 + 1, c0 + 1) else { continue };
        let Some(def) = hover.definition else { continue };
        if !matches!(def.kind, DefKind::Fn | DefKind::AssocFn) || def.name.rsplit("::").next() != Some(name) {
            continue;
        }
        let key = (def.file.to_string(), def.start_line);
        let params = names_cache.entry(key).or_insert_with(|| param_names(&def)).clone();
        let Some(mut params) = params else { continue };
        if is_method && params.first().is_some_and(|p| p.as_deref() == Some("self")) {
            params.remove(0);
        }
        let n_params = params.len();
        for ((a_start, a_end), p) in args.iter().zip(params.iter()) {
            // No hint for the receiver passed as the first argument of a path call.
            let Some(p) = p.as_ref().filter(|p| *p != "self") else { continue };
            let arg = src.text[*a_start..*a_end].trim();
            if should_hide(p, arg, name, n_params) {
                continue;
            }
            let lead = src.text[*a_start..*a_end].len() - src.text[*a_start..*a_end].trim_start().len();
            let mut h = Hint::new(src, a_start + lead, format!("{p}:"), Some(2));
            h.padding_right = true;
            out.push(h);
        }
    }
    out
}

/// The callee name right before the `(` at `open` (past a turbofish), and whether it is a method call; `None` for
/// declarations, keywords, macros and constructors.
fn callee_before(m: &str, open: usize) -> Option<(usize, usize, bool)> {
    let mut end = open;
    let before = m[..open].trim_end();
    end -= m[..open].len() - before.len();
    if before.ends_with('>') {
        let mut depth = 0i32;
        let bb = before.as_bytes();
        let mut k = bb.len();
        while k > 0 {
            k -= 1;
            match bb[k] {
                b'>' => depth += 1,
                b'<' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        let rest = before[..k].strip_suffix("::")?;
        end = rest.len();
    }
    let head = &m[..end];
    let start = head.rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map_or(0, |p| p + 1);
    let name = &m[start..end];
    if name.is_empty() || name.as_bytes()[0].is_ascii_digit() || name.as_bytes()[0].is_ascii_uppercase() {
        return None;
    }
    if matches!(
        name,
        "if" | "while" | "match" | "for" | "return" | "loop" | "in" | "fn" | "let" | "move" | "as" | "impl" | "where" | "else" | "break" | "continue" | "unsafe" | "await"
    ) {
        return None;
    }
    let prev = m[..start].trim_end();
    if prev.ends_with("fn") && prev[..prev.len() - 2].chars().last().is_none_or(|c| !c.is_alphanumeric()) {
        return None;
    }
    if m[end..open].contains('!') {
        return None;
    }
    Some((start, end, prev.ends_with('.')))
}

/// The byte ranges of expression-macro arguments (`name!(..)`, `name![..]`); a braced invocation wrapping items
/// (`cfg_rt! { .. }`) keeps its hints.
fn macro_argument_ranges(m: &str) -> Vec<(usize, usize)> {
    let b = m.as_bytes();
    let mut out = Vec::new();
    for (bang, _) in m.match_indices('!') {
        if bang == 0 || !(b[bang - 1].is_ascii_alphanumeric() || b[bang - 1] == b'_') {
            continue;
        }
        let rest = &m[bang + 1..];
        let open = bang + 1 + (rest.len() - rest.trim_start().len());
        if !matches!(b.get(open), Some(b'(' | b'[')) {
            continue;
        }
        let mut depth = 0i32;
        for (k, &c) in b.iter().enumerate().skip(open) {
            match c {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        out.push((open, k));
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    out
}

fn matching_paren(b: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (k, &c) in b.iter().enumerate().skip(open) {
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return (c == b')').then_some(k);
                }
            }
            _ => {}
        }
    }
    None
}

/// Byte ranges of the top-level, comma-separated arguments in `m[from..to]`.
fn split_args(m: &str, from: usize, to: usize) -> Vec<(usize, usize)> {
    let b = m.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = from;
    for k in from..to {
        match b[k] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'<' if k > from && (b[k - 1] == b':') => depth += 1,
            b'>' if depth > 0 && k > from && b[k - 1] != b'-' && b[k - 1] != b'=' => {}
            b',' if depth == 0 => {
                out.push((start, k));
                start = k + 1;
            }
            _ => {}
        }
    }
    if start < to {
        out.push((start, to));
    }
    out
}

/// Parameter names of the function `d` from its declaration: `None` for a pattern parameter; `self` for a receiver.
fn param_names(d: &ridx_format::ResolvedDefinition<'_>) -> Option<Vec<Option<String>>> {
    let text = std::fs::read_to_string(crate::lsp::intelligence::hover_render::def_file(d)).ok()?;
    let line_start = text.split_inclusive('\n').take(d.start_line.saturating_sub(1) as usize).map(str::len).sum::<usize>();
    let name = d.name.rsplit("::").next()?;
    let mut end = (line_start + 4000).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let window = text.get(line_start..end)?;
    let fn_at = window.find(&format!("fn {name}"))?;
    let after = &window[fn_at + 3 + name.len()..];
    let after = if after.starts_with('<') { skip_generics(after) } else { after };
    let open = after.find('(')?;
    let masked = crate::lsp::editing::action_parse::mask_comments_and_strings(&after[open..]);
    let close = matching_paren(masked.as_bytes(), 0)?;
    let params = split_args(&masked, 1, close);
    Some(
        params
            .into_iter()
            .filter(|&(s, e)| !after[open + s..open + e].trim().is_empty())
            .map(|(s, e)| {
                let p = after[open + s..open + e].trim();
                let p = p.trim_start_matches('#').trim();
                if p.trim_start_matches('&').trim_start_matches("mut ").trim_start().starts_with("self") || p.starts_with("&'") && p.contains("self") {
                    return Some("self".to_string());
                }
                let pat = p.split(':').next().unwrap_or(p).trim().trim_start_matches("mut ").trim();
                (pat.chars().all(|c| c.is_alphanumeric() || c == '_') && !pat.is_empty()).then(|| pat.to_string())
            })
            .collect(),
    )
}

fn skip_generics(text: &str) -> &str {
    let mut depth = 0i32;
    for (i, c) in text.char_indices() {
        match c {
            '<' => depth += 1,
            '>' if i > 0 && &text[i - 1..i] != "-" => {
                depth -= 1;
                if depth == 0 {
                    return &text[i + 1..];
                }
            }
            _ => {}
        }
    }
    text
}

/// rust-analyzer's rules for a parameter hint not worth showing.
fn should_hide(param: &str, arg: &str, fn_name: &str, n_params: usize) -> bool {
    let param = param.trim_start_matches('_');
    if param.is_empty() {
        return true;
    }
    if arg.starts_with('!') {
        return false;
    }
    let suffix_of_fn = n_params <= 1 && (fn_name == param || fn_name.strip_suffix(param).is_some_and(|p| p.ends_with('_')));
    let obvious = n_params == 1 && (param.len() == 1 || matches!(param, "predicate" | "value" | "pat" | "rhs" | "other"));
    suffix_of_fn || obvious || param.starts_with("ra_fixture") || argument_like_param(arg, param) || constructor_like_param(arg, param)
}

/// The names an argument expression goes by, as rust-analyzer collects them: the segments of a field/method chain
/// (`subs.iter().map(..).collect()` → `subs`, `iter`, `map`, `collect`), a path's last segment or a macro's name.
/// A call of a function (`f(x)`) or a literal has none.
fn argument_names(arg: &str) -> Vec<String> {
    let a = arg.trim();
    let a = a.split(" as ").next().unwrap_or(a).trim();
    let a = a.trim_start_matches(['&', '*', '-']).trim_start_matches("mut ").trim();
    // Split at top-level dots.
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, c) in a.char_indices() {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            '.' if depth == 0 => {
                parts.push(&a[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&a[start..]);
    let name_of = |p: &str| -> Option<String> {
        let p = p.trim();
        let head = p.split(['(', '[']).next().unwrap_or(p);
        let head = head.split("::<").next().unwrap_or(head);
        let head = head.strip_suffix('!').unwrap_or(head);
        let last = head.rsplit("::").next().unwrap_or(head);
        (!last.is_empty() && last.chars().all(|c| c.is_alphanumeric() || c == '_')).then(|| last.to_string())
    };
    if parts.len() == 1 {
        // A lone call of a function names nothing; a macro, path or field names itself.
        let p = parts[0].trim();
        if p.ends_with(')') && !p.split('(').next().unwrap_or("").ends_with('!') {
            return Vec::new();
        }
    }
    parts.into_iter().filter_map(name_of).collect()
}

fn argument_like_param(arg: &str, param: &str) -> bool {
    argument_names(arg).iter().any(|a| name_like_param(a, param) || name_like_param(param, a))
}

fn name_like_param(argument: &str, param: &str) -> bool {
    let argument = argument.trim_start_matches('_');
    if argument.len() >= param.len() && argument.is_char_boundary(param.len()) {
        let (prefix, rest) = argument.split_at(param.len());
        if prefix.eq_ignore_ascii_case(param) && (rest.is_empty() || rest.starts_with('_')) {
            return true;
        }
    }
    if let Some(at) = argument.len().checked_sub(param.len()) {
        if argument.is_char_boundary(at) {
            let (rest, suffix) = argument.split_at(at);
            if param.eq_ignore_ascii_case(suffix) && (rest.is_empty() || rest.ends_with('_')) {
                return true;
            }
        }
    }
    false
}

/// `Foo::new()`, `Foo { .. }` or `Foo(..)` passed for a parameter named `foo`.
fn constructor_like_param(arg: &str, param: &str) -> bool {
    let a = arg.trim().trim_start_matches('&');
    let head = a.split(['(', '{', ' ']).next().unwrap_or(a);
    let segs: Vec<&str> = head.split("::").collect();
    let ty = match segs.as_slice() {
        [.., ty, _method] if ty.chars().next().is_some_and(char::is_uppercase) => *ty,
        [.., ty] if ty.chars().next().is_some_and(char::is_uppercase) => *ty,
        _ => return false,
    };
    let ty = ty.split('<').next().unwrap_or(ty);
    to_snake(ty) == param
}

fn to_snake(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hiding_rules() {
        assert!(should_hide("name", "name", "f", 2));
        assert!(should_hide("name", "&self.name", "f", 2));
        assert!(should_hide("value", "x + 1", "set", 1));
        assert!(should_hide("x", "1", "f", 1));
        assert!(should_hide("index", "5", "set_index", 1));
        assert!(should_hide("config", "Config::default()", "f", 2));
        assert!(should_hide("ty", "&field.ty_of()", "f", 2));
        assert!(!should_hide("ty", "&field.kind()", "f", 2));
        assert!(!should_hide("len", "5", "f", 2));
        assert!(!should_hide("name", "!name", "f", 2));
    }

    #[test]
    fn test_callee_and_args() {
        let m = "foo(a, (b, c), d) x.bar::<T>(1) fn baz(y: u8) vec!(1) Some(2)";
        let open = m.find('(').unwrap();
        assert_eq!(callee_before(m, open).map(|(s, e, me)| (&m[s..e], me)), Some(("foo", false)));
        let args = split_args(m, open + 1, matching_paren(m.as_bytes(), open).unwrap());
        assert_eq!(args.iter().map(|&(s, e)| m[s..e].trim()).collect::<Vec<_>>(), ["a", "(b, c)", "d"]);
        let o2 = m.find("(1)").unwrap();
        assert_eq!(callee_before(m, o2).map(|(s, e, me)| (&m[s..e], me)), Some(("bar", true)));
        assert!(callee_before(m, m.find("(y").unwrap()).is_none());
        assert!(callee_before(m, m.find("!(").unwrap() + 1).is_none());
        assert!(callee_before(m, m.find("(2)").unwrap()).is_none());
    }
}
