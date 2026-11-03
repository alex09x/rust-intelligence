/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Generic-parameter closure, associated-type lookup, and fresh-identifier helpers.

use super::action_generics_sub::{collect_tokens_and_lifetimes, split_top_level_colon, GenKind, GenericDecl};
use super::action_parse::{is_ident_byte, mask_comments_and_strings};
use super::action_parse_decl::{collapse_ws, extract_angle_generics, next_ident_token, split_top_level_where};
use super::action_parse_items::{contains_word, skip_balanced};
use std::collections::HashSet;

pub fn impl_body_defines_assoc_type(masked_body: &str, assoc: &str) -> bool {
    let b = masked_body.as_bytes();
    let (mut brace, mut paren, mut bracket) = (0i32, 0i32, 0i32);
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'{' => brace += 1,
            b'}' => brace = (brace - 1).max(0),
            b'(' => paren += 1,
            b')' => paren = (paren - 1).max(0),
            b'[' => bracket += 1,
            b']' => bracket = (bracket - 1).max(0),
            _ if brace == 0 && paren == 0 && bracket == 0 && i + 4 <= b.len() && &b[i..i + 4] == b"type" => {
                let prev_ok = i == 0 || !is_ident_byte(b[i - 1]);
                let next_ok = i + 4 == b.len() || !is_ident_byte(b[i + 4]);
                if prev_ok && next_ok {
                    if let Some((tname, _)) = next_ident_token(masked_body, i + 4) {
                        if tname.strip_prefix("r#").unwrap_or(tname) == assoc {
                            return true;
                        }
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

pub fn find_sibling_trait_defining_assoc(doc_text: &str, masked_doc: &str, target_self_ty: &str, assoc: &str) -> Option<String> {
    let mb = masked_doc.as_bytes();
    let target_norm = collapse_ws(target_self_ty);
    let mut i = 0usize;
    while i + 4 < mb.len() {
        if &mb[i..i + 4] == b"impl" && (i == 0 || !is_ident_byte(mb[i - 1])) && !is_ident_byte(mb[i + 4]) {
            let (_, after_gen) = extract_angle_generics(doc_text, masked_doc, i + 4);
            let mut k = after_gen;
            let (mut p, mut br, mut an) = (0i32, 0i32, 0i32);
            let mut brace_open = None;
            while k < mb.len() {
                let prev = if k > 0 { mb[k - 1] } else { 0 };
                match mb[k] {
                    b'(' => p += 1,
                    b')' => p = (p - 1).max(0),
                    b'[' => br += 1,
                    b']' => br = (br - 1).max(0),
                    b'<' if p == 0 && br == 0 => an += 1,
                    b'>' if p == 0 && br == 0 && prev != b'-' && prev != b'=' => an = (an - 1).max(0),
                    b';' if p == 0 && br == 0 && an == 0 => break,
                    b'{' if p == 0 && br == 0 && an == 0 => {
                        brace_open = Some(k);
                        break;
                    }
                    _ => {}
                }
                k += 1;
            }
            if let Some(bo) = brace_open {
                if let Some(bc_after) = skip_balanced(mb, bo, b'{', b'}') {
                    let (head_s, _, _) = split_top_level_where(&doc_text[after_gen..bo], &masked_doc[after_gen..bo]);
                    if let Some(for_idx) = head_s.find(" for ") {
                        let tr_s = collapse_ws(&head_s[..for_idx]);
                        let sf_s = collapse_ws(&head_s[for_idx + 5..]);
                        if sf_s == target_norm && impl_body_defines_assoc_type(&masked_doc[bo + 1..bc_after - 1], assoc) {
                            return Some(tr_s);
                        }
                    }
                    i = bc_after;
                    continue;
                }
            }
        }
        i += 1;
    }
    None
}

pub fn assemble_kept_generics_and_where(rhs: &str, candidates: &[GenericDecl], where_preds: &[String]) -> Option<(String, String)> {
    let all_names: HashSet<String> = candidates.iter().map(|c| c.name.clone()).collect();
    let rhs_tokens = collect_tokens_and_lifetimes(rhs);
    let mut kept: HashSet<String> = candidates.iter().filter(|c| rhs_tokens.contains(&c.name)).map(|c| c.name.clone()).collect();

    loop {
        let before = kept.len();
        for c in candidates {
            if c.kind == GenKind::Const && kept.contains(&c.name) {
                for tok in collect_tokens_and_lifetimes(&c.head_no_bounds) {
                    if all_names.contains(&tok) {
                        kept.insert(tok);
                    }
                }
            }
        }
        if kept.len() == before {
            break;
        }
    }

    let projected_params = find_projected_params(rhs, &kept);
    let mut lt_parts = Vec::new();
    let mut tc_parts = Vec::new();

    for c in candidates {
        if !kept.contains(&c.name) {
            continue;
        }
        let mut valid_bounds = Vec::new();
        for b in &c.inline_bounds {
            let b_toks = free_tokens_ignoring_hrtb(b);
            let mentions_unkept = b_toks.iter().any(|t| all_names.contains(t) && !kept.contains(t));
            if mentions_unkept {
                if projected_params.contains(&c.name) {
                    return None;
                }
            } else {
                valid_bounds.push(b.clone());
            }
        }
        let formatted = if valid_bounds.is_empty() { c.head_no_bounds.clone() } else { format!("{}: {}", c.head_no_bounds, valid_bounds.join(" + ")) };
        if c.kind == GenKind::Lifetime {
            lt_parts.push(formatted);
        } else {
            tc_parts.push(formatted);
        }
    }

    let mut kept_where = Vec::new();
    for pred in where_preds {
        let p_toks = free_tokens_ignoring_hrtb(pred);
        let cand_hits: Vec<&String> = p_toks.iter().filter(|t| all_names.contains(*t)).collect();
        if cand_hits.is_empty() {
            continue;
        }
        let mentions_unkept = cand_hits.iter().any(|t| !kept.contains(*t));
        if mentions_unkept {
            let (lhs, _) = split_top_level_colon(pred);
            if projected_params.contains(lhs.trim()) {
                return None;
            }
        } else {
            kept_where.push(pred.clone());
        }
    }

    lt_parts.extend(tc_parts);
    let gen_header = if lt_parts.is_empty() { String::new() } else { format!("<{}>", lt_parts.join(", ")) };
    let where_clause = if kept_where.is_empty() { String::new() } else { format!(" where {}", kept_where.join(", ")) };
    Some((gen_header, where_clause))
}

fn free_tokens_ignoring_hrtb(s: &str) -> HashSet<String> {
    let mut toks = collect_tokens_and_lifetimes(s);
    if let Some(rest) = s.trim_start().strip_prefix("for<") {
        if let Some(end_a) = rest.find('>') {
            for bound_lt in collect_tokens_and_lifetimes(&rest[..end_a]) {
                if bound_lt.starts_with('\'') {
                    toks.remove(&bound_lt);
                }
            }
        }
    }
    toks
}

fn find_projected_params(rhs: &str, kept: &HashSet<String>) -> HashSet<String> {
    let mut out = HashSet::new();
    for p in kept {
        if !p.starts_with('\'') && rhs.contains(&format!("{p}::")) {
            out.insert(p.clone());
        }
    }
    out
}

pub fn to_pascal_fn_alias(fn_name: &str) -> String {
    let mut out = String::new();
    for seg in fn_name.split('_').filter(|s| !s.is_empty()) {
        let mut chars = seg.chars();
        if let Some(first) = chars.next() {
            for up in first.to_uppercase() {
                out.push(up);
            }
            out.push_str(chars.as_str());
        }
    }
    if out.is_empty() {
        out.push_str("Generated");
    }
    out.push_str("Fn");
    out
}

pub fn unique_ident(base: &str, doc_text: &str) -> String {
    let toks = collect_tokens_and_lifetimes(doc_text);
    if !toks.contains(base) {
        return base.to_string();
    }
    let mut n = 2usize;
    loop {
        let cand = format!("{base}{n}");
        if !toks.contains(&cand) {
            return cand;
        }
        n += 1;
    }
}

pub fn unique_lifetime(base: &str, doc_text: &str) -> String {
    let toks = collect_tokens_and_lifetimes(doc_text);
    if !toks.contains(base) {
        return base.to_string();
    }
    let mut n = 2usize;
    loop {
        let cand = format!("{base}{n}");
        if !toks.contains(&cand) {
            return cand;
        }
        n += 1;
    }
}

pub fn desugar_param_impl_trait<F>(ty_raw: &str, doc_text: &str, impl_decls: &mut Vec<GenericDecl>, sub_self: &F) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    let mut cur = sub_self(ty_raw.trim())?;
    for _ in 0..8 {
        let masked = mask_comments_and_strings(&cur);
        let Some((impl_start, bound_start, bound_end)) = find_rightmost_impl_trait_span(&masked) else {
            return Some(cur);
        };
        let bound_str = cur[bound_start..bound_end].trim();
        if bound_str.is_empty() {
            return None;
        }
        let base = format!("ImplParam{}", impl_decls.len());
        let pname = unique_ident(&base, &format!("{doc_text} {cur}"));
        impl_decls.push(GenericDecl { name: pname.clone(), kind: GenKind::Type, head_no_bounds: pname.clone(), inline_bounds: vec![collapse_ws(bound_str)] });
        cur.replace_range(impl_start..bound_end, &pname);
    }
    if contains_word(&mask_comments_and_strings(&cur), "impl") {
        None
    } else {
        Some(cur)
    }
}

fn find_rightmost_impl_trait_span(masked: &str) -> Option<(usize, usize, usize)> {
    let b = masked.as_bytes();
    let mut last_kw: Option<usize> = None;
    let mut i = 0usize;
    while i + 4 <= b.len() {
        if &b[i..i + 4] == b"impl" {
            let prev_ok = i == 0 || (!is_ident_byte(b[i - 1]) && b[i - 1] != b'.' && b[i - 1] != b'#');
            let next_b = b.get(i + 4).copied().unwrap_or(0);
            if prev_ok && (next_b.is_ascii_whitespace() || next_b == b'(' || next_b == b'?') {
                last_kw = Some(i);
            }
        }
        i += 1;
    }
    let kw_pos = last_kw?;
    let mut k = kw_pos + 4;
    while k < b.len() && b[k].is_ascii_whitespace() {
        k += 1;
    }
    let (mut paren, mut bracket, mut brace, mut angle) = (0i32, 0i32, 0i32, 0i32);
    let mut end_pos = b.len();
    for pos in k..b.len() {
        let prev = if pos > 0 { b[pos - 1] } else { 0 };
        match b[pos] {
            b'(' => paren += 1,
            b')' => {
                if paren == 0 {
                    end_pos = pos;
                    break;
                }
                paren -= 1;
            }
            b'[' => bracket += 1,
            b']' => {
                if bracket == 0 {
                    end_pos = pos;
                    break;
                }
                bracket -= 1;
            }
            b'{' => brace += 1,
            b'}' => {
                if brace == 0 {
                    end_pos = pos;
                    break;
                }
                brace -= 1;
            }
            b'<' if paren == 0 && bracket == 0 && brace == 0 => angle += 1,
            b'>' if paren == 0 && bracket == 0 && brace == 0 && prev != b'-' && prev != b'=' => {
                if angle == 0 {
                    end_pos = pos;
                    break;
                }
                angle -= 1;
            }
            b',' | b';' | b'=' if paren == 0 && bracket == 0 && brace == 0 && angle == 0 => {
                end_pos = pos;
                break;
            }
            _ => {}
        }
    }
    Some((kw_pos, k, end_pos))
}
