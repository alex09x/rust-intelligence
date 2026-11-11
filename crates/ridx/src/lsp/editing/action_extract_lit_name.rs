/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Parameter-name inference from enclosing function/method calls for `Extract into variable/constant/static`.

use super::action_generics_sub::split_top_level_commas;
use super::action_parse::is_ident_byte;
use super::action_parse_decl::{extract_angle_generics, next_ident_token};
use super::action_parse_items::skip_balanced;
use ridx_format::{DefKind, WorkspaceIndex};

/// The name rust-analyzer gives a variable holding the string literal `lit` passed at `lit_start`: the literal's own
/// text when it is an identifier of at most 32 bytes, else the name of the parameter it is passed to.
pub(crate) fn suggest_literal_var_name(
    doc_text: &str,
    masked_doc: &str,
    stack: &[(u8, usize)],
    lit_start: usize,
    lit_src: &str,
    file: &str,
    ws: &WorkspaceIndex,
) -> Option<String> {
    if let Some(inner) = lit_src.strip_prefix(['b', 'c']).unwrap_or(lit_src).strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        let is_ident = inner.starts_with(|c: char| c.is_alphabetic() || c == '_') && inner.chars().all(|c| c.is_alphanumeric() || c == '_') && inner != "_";
        if is_ident && inner.len() <= 32 && !is_rust_keyword(inner) {
            return Some(inner.to_string());
        }
    }
    infer_callee_param_name(doc_text, masked_doc, stack, lit_start, file, ws).filter(|n| is_informative_name(n))
}

/// rust-analyzer's filter of names that say nothing (`new`, `str`, `from_…`).
fn is_informative_name(name: &str) -> bool {
    const USELESS: &[&str] = &["new", "default", "option", "some", "none", "ok", "err", "str", "string", "from", "into"];
    !USELESS.contains(&name) && !["from_", "with_", "into_"].iter().any(|p| name.starts_with(p))
}

pub(crate) fn infer_callee_param_name(doc_text: &str, masked_doc: &str, stack: &[(u8, usize)], lit_start: usize, file: &str, ws: &WorkspaceIndex) -> Option<String> {
    let &(_, open_paren) = stack.iter().rev().find(|&&(ch, _)| ch == b'(')?;
    let before = masked_doc[..open_paren].trim_end();
    let callee_end = if before.ends_with('>') { before.rfind("::<")? } else { before.len() };
    let head = before[..callee_end].trim_end();
    let mut id_start = head.len();
    while id_start > 0 && is_ident_byte(head.as_bytes()[id_start - 1]) {
        id_start -= 1;
    }
    let raw_callee = head[id_start..].strip_prefix("r#").unwrap_or(&head[id_start..]);
    if raw_callee.is_empty() || matches!(raw_callee, "if" | "while" | "match" | "return" | "Some" | "Ok" | "Err") {
        return None;
    }
    let is_method = head[..id_start].trim_end().ends_with('.');
    let arg_idx = count_top_level_commas(&masked_doc[open_paren + 1..lit_start]);

    // The function the compiler resolved the call to, when the index knows it.
    let line_start = doc_text[..id_start].rfind('\n').map_or(0, |p| p + 1);
    let (line, col) = (doc_text[..line_start].matches('\n').count() as u32 + 1, doc_text[line_start..id_start].chars().count() as u32 + 1);
    if let Some(Some(def)) = ws.resolve_definitions_at(file, &[(line, col)]).pop() {
        if matches!(def.kind, DefKind::Fn | DefKind::AssocFn) && def.name.strip_prefix("r#").unwrap_or(def.name) == raw_callee {
            // The index's signature is the function's type, without parameter names: they are read from its source.
            let def_file = std::path::Path::new(def.file);
            let path = match def.source_root {
                Some(root) if !def_file.is_absolute() => ridx_format::join_resolving_overlap(root, def_file),
                _ => def_file.to_path_buf(),
            };
            if let Some(name) = std::fs::read_to_string(&path).ok().and_then(|src| param_name_at_def(&src, def.start_line, raw_callee, arg_idx, is_method)) {
                return Some(name);
            }
        }
    }

    if let Some(pname) = find_fn_param_in_doc(doc_text, masked_doc, raw_callee, arg_idx, is_method) {
        return Some(pname);
    }

    let mut found_param: Option<String> = None;
    for s in ws.exact_symbols_by_name(raw_callee) {
        if !matches!(s.kind, DefKind::Fn | DefKind::AssocFn) || s.name.strip_prefix("r#").unwrap_or(s.name) != raw_callee {
            continue;
        }
        if let Some(sig) = s.signature {
            if let Some(pn) = extract_nth_param_from_sig(sig, arg_idx, is_method) {
                if let Some(ref prev) = found_param {
                    if prev != &pn {
                        return None;
                    }
                } else {
                    found_param = Some(pn);
                }
            }
        }
    }
    found_param
}

/// The name of the `arg_idx`-th argument's parameter of the function `name` declared on line `line` (1-based) of `src`.
fn param_name_at_def(src: &str, line: u32, name: &str, arg_idx: usize, is_method: bool) -> Option<String> {
    let line_start = src.split_inclusive('\n').take(line.checked_sub(1)? as usize).map(str::len).sum::<usize>();
    let masked = super::action_parse::mask_comments_and_strings(src);
    let mb = masked.as_bytes();
    let at = line_start + masked[line_start..].find(name)?;
    let (_, after_g) = extract_angle_generics(src, &masked, at + name.len());
    let p = after_g + (masked[after_g..].len() - masked[after_g..].trim_start().len());
    let rp = skip_balanced(mb, p, b'(', b')')?;
    extract_nth_param_from_params(&src[p + 1..rp - 1], arg_idx, is_method)
}

fn count_top_level_commas(slice: &str) -> usize {
    let mut cnt = 0usize;
    let (mut p, mut b, mut c) = (0i32, 0i32, 0i32);
    for &ch in slice.as_bytes() {
        match ch {
            b'(' => p += 1,
            b')' => p = (p - 1).max(0),
            b'[' => b += 1,
            b']' => b = (b - 1).max(0),
            b'{' => c += 1,
            b'}' => c = (c - 1).max(0),
            b',' if p == 0 && b == 0 && c == 0 => cnt += 1,
            _ => {}
        }
    }
    cnt
}

fn find_fn_param_in_doc(doc_text: &str, masked_doc: &str, fn_name: &str, arg_idx: usize, is_method: bool) -> Option<String> {
    let mb = masked_doc.as_bytes();
    let mut i = 0usize;
    let mut hit: Option<String> = None;
    while i + 2 < mb.len() {
        if &mb[i..i + 2] == b"fn" && (i == 0 || !is_ident_byte(mb[i - 1])) && !is_ident_byte(mb[i + 2]) {
            if let Some((n, after_n)) = next_ident_token(doc_text, i + 2) {
                if n.strip_prefix("r#").unwrap_or(n) == fn_name {
                    let (_, after_g) = extract_angle_generics(doc_text, masked_doc, after_n);
                    let mut p = after_g;
                    while p < mb.len() && mb[p].is_ascii_whitespace() {
                        p += 1;
                    }
                    if let Some(rp) = skip_balanced(mb, p, b'(', b')') {
                        if let Some(pn) = extract_nth_param_from_params(&doc_text[p + 1..rp - 1], arg_idx, is_method) {
                            if hit.as_ref().is_some_and(|prev| prev != &pn) {
                                return None;
                            }
                            hit = Some(pn);
                        }
                    }
                }
            }
        }
        i += 1;
    }
    hit
}

fn extract_nth_param_from_sig(sig: &str, arg_idx: usize, is_method: bool) -> Option<String> {
    let lp = sig.find('(')?;
    let rp = skip_balanced(sig.as_bytes(), lp, b'(', b')')?;
    extract_nth_param_from_params(&sig[lp + 1..rp - 1], arg_idx, is_method)
}

fn extract_nth_param_from_params(params_src: &str, arg_idx: usize, is_method: bool) -> Option<String> {
    let parts = split_top_level_commas(params_src);
    let mut non_self = Vec::with_capacity(parts.len());
    for (idx, raw) in parts.into_iter().enumerate() {
        // A parameter's attributes (`#[rust_analyzer::rust_fixture] ra_fixture: &str`) are not part of its name.
        let mut t = raw.trim();
        while let Some(rest) = t.strip_prefix('#') {
            let rest = rest.trim_start();
            let Some(end) = skip_balanced(rest.as_bytes(), 0, b'[', b']') else { break };
            t = rest[end..].trim_start();
        }
        if t.is_empty() {
            continue;
        }
        let is_self = idx == 0 && (t == "self" || t == "mut self" || t.ends_with(" self") || t.starts_with("self:") || t.starts_with("mut self:"));
        if is_self {
            if !is_method {
                if arg_idx == 0 {
                    return None;
                }
                non_self.push(String::new());
            }
            continue;
        }
        non_self.push(t.to_string());
    }
    let target = non_self.get(arg_idx)?;
    if target.is_empty() {
        return None;
    }
    let lhs = target.split(':').next()?.trim().trim_start_matches("mut ").trim_start_matches("ref ").trim();
    let clean = lhs.strip_prefix("r#").unwrap_or(lhs).trim_matches('_');
    if clean.is_empty() || is_rust_keyword(clean) || !clean.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || clean.chars().next()?.is_ascii_digit() {
        return None;
    }
    Some(clean.to_string())
}

fn is_rust_keyword(s: &str) -> bool {
    matches!(
        s,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "gen"
            | "macro"
            | "override"
            | "priv"
            | "try"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
    )
}
