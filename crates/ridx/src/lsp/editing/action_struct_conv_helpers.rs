/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Field parsing, provenance, and constructor rewrite helpers for `action_struct_conv`.

use super::action_adt::strip_field_vis;
use super::action_generics_helpers::clean_param_ident;
use super::action_generics_sub::{split_top_level_colon, split_top_level_commas};
use super::action_parse::{byte_to_utf16_pos, is_ident_byte, mask_comments_and_strings};
use super::action_parse_decl::{collapse_ws, extract_angle_generics, find_header_terminator, next_ident_token, split_top_level_where};
use super::action_parse_items::{contains_word, skip_balanced};
use super::action_struct_conv_fields::{merge_continuations, split_leading_comments};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub(crate) fn is_isolated_leaf_module(mod_masked: &str, struct_name: &str) -> bool {
    if contains_word(mod_masked, "include") {
        return false;
    }
    let mb = mod_masked.as_bytes();
    let mut i = 0usize;
    while i < mb.len() {
        if mb[i] == b'!' && mb.get(i + 1).copied() != Some(b'=') {
            let mut k = i + 1;
            while k < mb.len() && mb[k].is_ascii_whitespace() {
                k += 1;
            }
            let end_m = match mb.get(k).copied() {
                Some(b'(') => skip_balanced(mb, k, b'(', b')'),
                Some(b'[') => skip_balanced(mb, k, b'[', b']'),
                Some(b'{') => skip_balanced(mb, k, b'{', b'}'),
                _ => None,
            };
            if let Some(em) = end_m {
                if contains_word(&mod_masked[k..em], struct_name) {
                    return false;
                }
                i = em;
                continue;
            }
        }
        if i + 3 < mb.len() && &mb[i..i + 3] == b"mod" && (i == 0 || !is_ident_byte(mb[i - 1])) && !is_ident_byte(mb[i + 3]) {
            if let Some((_, after_n)) = next_ident_token(mod_masked, i + 3) {
                if mod_masked[after_n..].trim_start().starts_with(';') {
                    return false;
                }
            }
        }
        i += 1;
    }
    true
}

/// Named fields as `(visibility, name, type, leading comments)`. Commas and colons are found in the masked text, so
/// comments between fields are kept (as the next field's leading comments) instead of being read as code.
pub(crate) fn parse_named_fields_clean(raw: &str) -> Option<Vec<(String, String, String, String)>> {
    let mf = mask_comments_and_strings(raw);
    if mf.contains('$') || mf.len() != raw.len() {
        return None;
    }
    let mb = mf.as_bytes();
    let mut out = Vec::new();
    // Leading whitespace and outer attributes (`#[serde(skip)]`) of an entry; they travel with the field.
    let skip_attrs = |m: &str| -> Option<usize> {
        let b = m.as_bytes();
        let mut p = m.len() - m.trim_start().len();
        while p < b.len() && b[p] == b'#' {
            let open = p + 1 + (m[p + 1..].len() - m[p + 1..].trim_start().len());
            p = skip_balanced(b, open, b'[', b']')?;
            p += m[p..].len() - m[p..].trim_start().len();
        }
        Some(p)
    };
    let (mut depth, mut start) = (0i32, 0usize);
    for i in 0..=mb.len() {
        let c = mb.get(i).copied();
        match c {
            Some(b'<' | b'(' | b'[' | b'{') => depth += 1,
            Some(b'>') if i > 0 && mb[i - 1] == b'-' => {}
            Some(b'>' | b')' | b']' | b'}') => depth -= 1,
            _ => {}
        }
        if c.is_some() && (c != Some(b',') || depth != 0) {
            continue;
        }
        let (m, r) = (&mf[start..i], &raw[start..i]);
        start = i + 1;
        if m.trim().is_empty() {
            // A comment after the last field has no field to travel with.
            if !r.trim().is_empty() {
                return None;
            }
            continue;
        }
        let lead = skip_attrs(m)?;
        let leading = r[..lead].trim().to_string();
        let t = r[lead..].trim_end();
        let rest = strip_field_vis(t);
        let vis = collapse_ws(t[..t.len() - rest.len()].trim());
        let (fn_raw, Some(ft_raw)) = split_top_level_colon(rest) else { return None };
        let fname = clean_param_ident(fn_raw.trim())?;
        let fty = collapse_ws(ft_raw);
        if fty.is_empty() {
            return None;
        }
        out.push((vis, fname, fty, leading));
    }
    Some(out)
}

pub(crate) fn parse_tuple_fields_clean(raw: &str) -> Option<Vec<(String, String)>> {
    let mf = mask_comments_and_strings(raw);
    if mf.contains('#') || mf.contains('$') {
        return None;
    }
    let mut out = Vec::new();
    for entry in split_top_level_commas(raw) {
        let t = entry.trim();
        if t.is_empty() {
            continue;
        }
        let rest = strip_field_vis(t);
        let vis = collapse_ws(t[..t.len() - rest.len()].trim());
        let fty = collapse_ws(rest);
        if fty.is_empty() {
            return None;
        }
        out.push((vis, fty));
    }
    Some(out)
}

/// `{ a: x, b }` → `(x, b)` in field order. A rest pattern (`{ a, .. }`) fills the other fields with `_`; a struct
/// update from a plain path (`{ a: x, ..base }`) fills them with `base.N`, which moves the same fields in the same
/// order. Any other base would be evaluated once per field, so it is refused.
pub(crate) fn convert_brace_init_to_tuple(inner_src: &str, _inner_masked: &str, expected: usize, field_idx: &HashMap<&str, usize>) -> Option<String> {
    let entries = merge_continuations(split_top_level_commas(inner_src));
    let mut rest: Option<String> = None;
    let mut slots: Vec<Option<String>> = vec![None; expected];
    for (n, entry) in entries.iter().enumerate() {
        let (comments, t) = split_leading_comments(entry.trim());
        if t.is_empty() {
            continue;
        }
        if let Some(base) = t.strip_prefix("..") {
            let base = base.trim();
            let plain = base.split('.').all(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'));
            if entries[n + 1..].iter().any(|e| !e.trim().is_empty()) || !(base.is_empty() || plain) {
                return None;
            }
            rest = Some(if base.is_empty() { "_".to_string() } else { format!("{base}.") });
            continue;
        }
        let (lhs, rhs_opt) = split_top_level_colon(t);
        let (fname_clean, val_str) = if let Some(rhs) = rhs_opt {
            let f = lhs.trim().strip_prefix("r#").unwrap_or(lhs.trim());
            (f, rhs.trim().to_string())
        } else {
            let tok = lhs.split_whitespace().last()?;
            let f = tok.strip_prefix("r#").unwrap_or(tok);
            (f, lhs.trim().to_string())
        };
        let &idx = field_idx.get(fname_clean)?;
        if slots[idx].is_some() {
            return None;
        }
        // A trailing line comment would swallow the `,` or `)` that follows the value; a line break ends it.
        let eol = if val_str.rsplit('\n').next().is_some_and(|l| l.contains("//")) { "\n" } else { "" };
        slots[idx] = Some(format!("{comments}{val_str}{eol}"));
    }
    let ordered: Option<Vec<String>> = slots
        .into_iter()
        .enumerate()
        .map(|(i, s)| match (s, &rest) {
            (Some(v), _) => Some(v),
            (None, Some(r)) if r == "_" => Some("_".to_string()),
            (None, Some(base)) => Some(format!("{base}{i}")),
            (None, None) => None,
        })
        .collect();
    Some(format!("({})", ordered?.join(", ")))
}

pub(crate) fn convert_tuple_init_to_brace(inner_src: &str, inner_masked: &str, expected: usize) -> Option<String> {
    if inner_masked.contains("..") {
        return None;
    }
    let parts: Vec<String> = split_top_level_commas(inner_src).into_iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
    if parts.len() != expected {
        return None;
    }
    let named: Vec<String> = parts.into_iter().enumerate().map(|(i, e)| format!("field{}: {e}", i + 1)).collect();
    Some(format!(" {{ {} }}", named.join(", ")))
}

pub(crate) fn skip_optional_turbofish(src: &str, masked: &str, pos: usize) -> (bool, usize) {
    let trimmed = masked[pos..].trim_start();
    let ws = masked[pos..].len() - trimmed.len();
    if trimmed.starts_with("::") {
        let p2 = pos + ws + 2;
        if masked[p2..].trim_start().starts_with('<') {
            let (_, after_g) = extract_angle_generics(src, masked, p2);
            return (true, after_g);
        }
    }
    (false, pos)
}

pub(crate) fn is_brace_struct_literal_or_pat(masked: &str, floor: usize, tok_start: usize, brace_byte: usize, impl_spans: &[(usize, usize)]) -> bool {
    if impl_spans.iter().any(|&(bo, _)| bo == brace_byte) {
        return false;
    }
    let mb = masked.as_bytes();
    let (mut i, mut p, mut b) = (tok_start, 0i32, 0i32);
    let mut clause_lo = floor;
    while i > floor {
        i -= 1;
        match mb[i] {
            b')' => p += 1,
            b'(' => {
                if p == 0 {
                    clause_lo = i + 1;
                    break;
                }
                p -= 1;
            }
            b']' => b += 1,
            b'[' => {
                if b == 0 {
                    clause_lo = i + 1;
                    break;
                }
                b -= 1;
            }
            b'{' | b'}' | b';' if p == 0 && b == 0 => {
                clause_lo = i + 1;
                break;
            }
            _ => {}
        }
    }
    let mut clause = &masked[clause_lo..tok_start];
    if let Some(eq_idx) = rfind_top_level_assign_or_arrow(clause) {
        clause = &clause[eq_idx..];
    }
    let sub = clause.trim();
    if sub.ends_with("->") || (sub.ends_with(':') && !sub.ends_with("::")) {
        return false;
    }
    !["fn", "impl", "where", "trait", "struct", "enum", "union", "mod", "match", "for", "as"].iter().any(|kw| contains_word(sub, kw))
}

fn rfind_top_level_assign_or_arrow(slice: &str) -> Option<usize> {
    let b = slice.as_bytes();
    let (mut p, mut br, mut a) = (0i32, 0i32, 0i32);
    let mut last = None;
    for i in 0..b.len() {
        match b[i] {
            b'(' => p += 1,
            b')' => p = (p - 1).max(0),
            b'[' => br += 1,
            b']' => br = (br - 1).max(0),
            b'<' => a += 1,
            b'>' if i > 0 && (b[i - 1] == b'-' || b[i - 1] == b'=') => {}
            b'>' => a = (a - 1).max(0),
            b'=' if p == 0 && br == 0 && a == 0 => {
                if b.get(i + 1).copied() == Some(b'>') {
                    last = Some(i + 2);
                } else if b.get(i + 1).copied() != Some(b'=') && (i == 0 || !matches!(b[i - 1], b'=' | b'!' | b'<' | b'>')) {
                    last = Some(i + 1);
                }
            }
            _ => {}
        }
    }
    last
}

pub(crate) fn is_bare_value_ctor_fn_ref(masked: &str, floor: usize, start: usize, after: usize) -> bool {
    let prev = masked[floor..start].trim_end();
    let next = masked[after..].trim_start();
    if next.starts_with("::") || next.starts_with('<') || next.starts_with('{') || next.starts_with("where") {
        return false;
    }
    !(prev.ends_with(':') || prev.ends_with("->") || prev.ends_with('<') || prev.ends_with("as") || prev.ends_with("for") || prev.ends_with("impl"))
}

pub(crate) fn find_impl_body_spans_for(src: &str, masked: &str, lo: usize, hi: usize, struct_name: &str) -> Vec<(usize, usize)> {
    let mb = masked.as_bytes();
    let mut out = Vec::new();
    let mut i = lo;
    while i + 4 < hi {
        if &mb[i..i + 4] == b"impl" && (i == 0 || !is_ident_byte(mb[i - 1])) && !is_ident_byte(mb[i + 4]) {
            let (_, after_g) = extract_angle_generics(src, masked, i + 4);
            if let Some(bo) = find_header_terminator(mb, after_g) {
                if bo < hi && mb[bo] == b'{' {
                    if let Some(bc_after) = skip_balanced(mb, bo, b'{', b'}') {
                        let (hs, hm, _) = split_top_level_where(&src[after_g..bo], &masked[after_g..bo]);
                        let self_part = super::action_parse_decl::split_impl_trait_for(hs, hm).map(|(_, s)| s).unwrap_or(hs);
                        let leaf = self_part.split('<').next().unwrap_or(self_part).trim().rsplit("::").next().unwrap_or("").trim();
                        if leaf.strip_prefix("r#").unwrap_or(leaf) == struct_name {
                            out.push((bo, bc_after - 1));
                        }
                        i = bc_after;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    out
}

pub(crate) fn collect_typed_locals(mod_masked: &str, struct_name: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for line in mod_masked.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("let ") {
            let r = rest.trim_start_matches("mut ").trim();
            let v: String = r.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            if v.is_empty() {
                continue;
            }
            let after_v = r[v.len()..].trim_start();
            let rhs_or_ty = if let Some(after_c) = after_v.strip_prefix(':') {
                after_c.split('=').next().unwrap_or(after_c)
            } else if let Some(after_eq) = after_v.strip_prefix('=') {
                after_eq
            } else {
                continue;
            };
            let head = rhs_or_ty.trim().trim_start_matches('&').trim().trim_start_matches("mut ").trim();
            let head_ident: String = head.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            if head_ident == struct_name {
                out.insert(v);
            }
        }
    }
    out
}

pub(crate) fn receiver_is_proven_owner(masked: &str, floor: usize, dot_byte: usize, in_impl: bool, known_locals: &HashSet<String>) -> bool {
    let before = masked[floor..dot_byte].trim_end();
    let mut s = before.len();
    while s > 0 && is_ident_byte(before.as_bytes()[s - 1]) {
        s -= 1;
    }
    let recv = before[s..].strip_prefix("r#").unwrap_or(&before[s..]);
    // `a.self`/`m::x` are not the receiver itself; a field label (`Self { f: self.f }`) is.
    let lead = before[..s].trim_end();
    if lead.ends_with('.') || lead.ends_with("::") {
        return false;
    }
    (in_impl && recv == "self") || known_locals.contains(recv)
}

pub(crate) fn to_lsp_edits(doc_text: &str, starts: &[usize], mut raw_edits: Vec<(usize, usize, String)>) -> Option<Vec<Value>> {
    raw_edits.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    for w in raw_edits.windows(2) {
        if w[1].0 < w[0].1 {
            return None;
        }
    }
    Some(
        raw_edits
            .into_iter()
            .map(|(lo, hi, nt)| {
                let (sl, sc) = byte_to_utf16_pos(doc_text, starts, lo);
                let (el, ec) = byte_to_utf16_pos(doc_text, starts, hi);
                json!({
                    "range": { "start": { "line": sl, "character": sc }, "end": { "line": el, "character": ec } },
                    "newText": nt
                })
            })
            .collect(),
    )
}
