/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compile-safe `Generate impl` and `Generate new` builders for nominal ADTs.

use super::action_crate_checks::crate_denies_missing_docs;
use super::action_generics_helpers::{clean_param_ident, contains_inferred_underscore};
use super::action_generics_sub::{collect_tokens_and_lifetimes, parse_generics_list, split_top_level_colon, split_top_level_commas, strip_leading_attrs_keep_cfg, GenKind};
use super::action_parse::{byte_to_utf16_pos, is_ident_byte, mask_comments_and_strings, AdtFieldsKind, EnclosingContext, ItemKindInfo, ParsedCursorItem};
use super::action_parse_decl::{collapse_ws, extract_angle_generics, next_ident_token, split_top_level_where};
use super::action_parse_items::{contains_word, skip_balanced};
use super::action_suggest_name::{name_of_type, NameGenerator};
use ridx_format::{DefKind, WorkspaceIndex};
use std::collections::HashSet;
use std::path::Path;

/// rust-analyzer's `Generate trait impl`: an `impl _ for Type {}` template after the type, the trait left for the
/// user to name (an editor with snippets places the cursor on the `_`).
pub fn build_generate_trait_impl(doc_text: &str, starts: &[usize], item: &ParsedCursorItem) -> Option<(u32, u32, String)> {
    let (line0, col_u16, text) = build_generate_impl(doc_text, starts, item)?;
    let at = text.find("impl")? + "impl".len();
    let mut generics_end = at;
    if text[at..].starts_with('<') {
        let mut depth = 0i32;
        for (i, c) in text[at..].char_indices() {
            match c {
                '<' => depth += 1,
                '>' if text[..at + i].ends_with('-') => {}
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        generics_end = at + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    Some((line0, col_u16, format!("{} _ for{}", &text[..generics_end], &text[generics_end..])))
}

pub fn build_generate_impl(doc_text: &str, starts: &[usize], item: &ParsedCursorItem) -> Option<(u32, u32, String)> {
    if !matches!(item.enclosing, EnclosingContext::None) {
        return None;
    }
    let ItemKindInfo::Adt { generics_src, where_src, end_byte, .. } = &item.kind else {
        return None;
    };
    if contains_word(&mask_comments_and_strings(generics_src), "cfg") {
        return None;
    }
    let (impl_gens, type_args, where_clause) = format_adt_impl_parts(generics_src, where_src)?;
    let indent = &item.indent;
    let cfg_prefix = format_cfg_lines(&item.cfg_attrs, indent);
    let new_text = format!("\n\n{cfg_prefix}{indent}impl{impl_gens} {}{type_args}{where_clause} {{}}", item.raw_name);
    let (line0, col_u16) = byte_to_utf16_pos(doc_text, starts, *end_byte);
    Some((line0, col_u16, new_text))
}

pub fn build_generate_new(doc_text: &str, masked_doc: &str, starts: &[usize], file_path: &str, item: &ParsedCursorItem, ws: &WorkspaceIndex) -> Option<(u32, u32, String)> {
    if !matches!(item.enclosing, EnclosingContext::None) {
        return None;
    }
    let ItemKindInfo::Adt { adt_kw: "struct", generics_src, where_src, fields_kind, end_byte, .. } = &item.kind else {
        return None;
    };
    if matches!(fields_kind, AdtFieldsKind::Unit | AdtFieldsKind::EnumOrUnion) {
        return None;
    }
    if contains_word(&mask_comments_and_strings(generics_src), "cfg") {
        return None;
    }
    if struct_already_has_new(doc_text, masked_doc, file_path, &item.name, ws) {
        return None;
    }
    // A `new` of a struct that is not public is not used yet: where dead code is denied it breaks the build.
    if item.vis_prefix.trim() != "pub" && super::action_crate_checks::crate_denies_dead_code(masked_doc, file_path) {
        return None;
    }

    let (impl_gens, type_args, where_clause) = format_adt_impl_parts(generics_src, where_src)?;
    let unsized_params = collect_unsized_type_params(generics_src, where_src);
    let type_params: HashSet<String> = parse_generics_list(generics_src).into_iter().map(|g| g.name).collect();
    let (params_joined, ctor_expr) = build_struct_new_signature_and_body(fields_kind, &unsized_params, &type_params)?;

    let indent = &item.indent;
    let vis = &item.vis_prefix;
    let cfg_prefix = format_cfg_lines(&item.cfg_attrs, indent);
    let doc_line =
        if vis.starts_with("pub") && crate_denies_missing_docs(masked_doc, file_path) { format!("{indent}    /// Creates a new [`{}`].\n", item.name) } else { String::new() };
    let new_text = format!(
        "\n\n{cfg_prefix}{indent}impl{impl_gens} {}{type_args}{where_clause} {{\n{doc_line}{indent}    {vis}fn new({params_joined}) -> Self {{\n{indent}        {ctor_expr}\n{indent}    }}\n{indent}}}",
        item.raw_name
    );
    let (line0, col_u16) = byte_to_utf16_pos(doc_text, starts, *end_byte);
    Some((line0, col_u16, new_text))
}

pub(crate) fn format_cfg_lines(cfg_attrs: &[String], indent: &str) -> String {
    if cfg_attrs.is_empty() {
        String::new()
    } else {
        let mut s = String::new();
        for c in cfg_attrs {
            s.push_str(indent);
            s.push_str(c);
            s.push('\n');
        }
        s
    }
}

pub(crate) fn format_adt_impl_parts(generics_src: &str, where_src: &str) -> Option<(String, String, String)> {
    let decls = parse_generics_list(generics_src);
    let mut impl_parts = Vec::with_capacity(decls.len());
    let mut arg_parts = Vec::with_capacity(decls.len());
    for d in &decls {
        if d.inline_bounds.is_empty() {
            impl_parts.push(d.head_no_bounds.clone());
        } else {
            impl_parts.push(format!("{}: {}", d.head_no_bounds, d.inline_bounds.join(" + ")));
        }
        match d.kind {
            GenKind::Lifetime => arg_parts.push(d.name.clone()),
            GenKind::Type => arg_parts.push(d.head_no_bounds.clone()),
            GenKind::Const => arg_parts.push(d.name.clone()),
        }
    }
    let impl_gens = if impl_parts.is_empty() { String::new() } else { format!("<{}>", impl_parts.join(", ")) };
    let type_args = if arg_parts.is_empty() { String::new() } else { format!("<{}>", arg_parts.join(", ")) };
    let w_trim = where_src.trim().trim_end_matches(',').trim();
    let where_clause = if w_trim.is_empty() { String::new() } else { format!(" where {w_trim}") };
    Some((impl_gens, type_args, where_clause))
}

fn build_struct_new_signature_and_body(fields_kind: &AdtFieldsKind, unsized_params: &HashSet<String>, type_params: &HashSet<String>) -> Option<(String, String)> {
    match fields_kind {
        AdtFieldsKind::Named(raw_fields) => {
            let masked_f = mask_comments_and_strings(raw_fields);
            if contains_word(&masked_f, "cfg") || contains_word(&masked_f, "cfg_attr") || masked_f.contains('$') {
                return None;
            }
            let mut params = Vec::new();
            let mut inits = Vec::new();
            for entry in split_top_level_commas(raw_fields) {
                let (_, rest) = strip_leading_attrs_keep_cfg(&entry);
                let clean = strip_field_vis(&rest);
                if clean.is_empty() {
                    continue;
                }
                let (fname_raw, Some(fty_raw)) = split_top_level_colon(clean) else {
                    return None;
                };
                let fname = clean_param_ident(fname_raw.trim())?;
                let fty = collapse_ws(fty_raw);
                if fty.is_empty() || contains_inferred_underscore(&fty) || is_potentially_unsized(&fty, unsized_params) {
                    return None;
                }
                params.push(format!("{fname}: {fty}"));
                inits.push(fname);
            }
            let body = if inits.is_empty() { "Self {}".to_string() } else { format!("Self {{ {} }}", inits.join(", ")) };
            Some((params.join(", "), body))
        }
        AdtFieldsKind::Tuple(raw_fields) => {
            let masked_f = mask_comments_and_strings(raw_fields);
            if contains_word(&masked_f, "cfg") || contains_word(&masked_f, "cfg_attr") || masked_f.contains('$') {
                return None;
            }
            let mut params = Vec::new();
            let mut args = Vec::new();
            // Parameters are named after their types (`Arc<Notify>` → `notify`), else `_0`, `_1`, as rust-analyzer does.
            let mut names = NameGenerator::default();
            for (idx, entry) in split_top_level_commas(raw_fields).into_iter().enumerate() {
                let (_, rest) = strip_leading_attrs_keep_cfg(&entry);
                let fty = collapse_ws(strip_field_vis(&rest));
                if fty.is_empty() || contains_inferred_underscore(&fty) || is_potentially_unsized(&fty, unsized_params) {
                    return None;
                }
                let pname = match name_of_type(&fty, type_params) {
                    Some(n) => names.suggest_name(&n),
                    None => names.suggest_name(&format!("_{idx}")),
                };
                params.push(format!("{pname}: {fty}"));
                args.push(pname);
            }
            Some((params.join(", "), format!("Self({})", args.join(", "))))
        }
        _ => None,
    }
}

pub(crate) fn strip_field_vis(s: &str) -> &str {
    let t = s.trim();
    if let Some(after_pub) = t.strip_prefix("pub") {
        let b = after_pub.as_bytes();
        if b.is_empty() {
            return "";
        }
        if b[0].is_ascii_whitespace() {
            return after_pub.trim_start();
        }
        if b[0] == b'(' {
            if let Some(end_p) = skip_balanced(b, 0, b'(', b')') {
                return after_pub[end_p..].trim_start();
            }
        }
    }
    t
}

pub(crate) fn collect_unsized_type_params(generics_src: &str, where_src: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for g in parse_generics_list(generics_src) {
        if g.inline_bounds.iter().any(|b| b.contains("?Sized")) {
            out.insert(g.name);
        }
    }
    for pred in split_top_level_commas(where_src) {
        let (lhs, Some(rhs)) = split_top_level_colon(&pred) else { continue };
        if rhs.contains("?Sized") {
            let t = lhs.trim().strip_prefix("r#").unwrap_or(lhs.trim());
            out.insert(t.to_string());
        }
    }
    out
}

pub(crate) fn is_potentially_unsized(fty: &str, unsized_params: &HashSet<String>) -> bool {
    let t = fty.trim();
    if t.starts_with('&') || t.starts_with("*const ") || t.starts_with("*mut ") {
        return false;
    }
    if t == "str" || t.starts_with("dyn ") || t.starts_with("(dyn ") {
        return true;
    }
    if t.starts_with('[') && t.ends_with(']') {
        let inner = &t[1..t.len() - 1];
        let masked = mask_comments_and_strings(inner);
        let mb = masked.as_bytes();
        let (mut p, mut b, mut a, mut has_semi) = (0i32, 0i32, 0i32, false);
        for &ch in mb {
            match ch {
                b'(' => p += 1,
                b')' => p = (p - 1).max(0),
                b'[' => b += 1,
                b']' => b = (b - 1).max(0),
                b'<' if p == 0 && b == 0 => a += 1,
                b'>' if p == 0 && b == 0 => a = (a - 1).max(0),
                b';' if p == 0 && b == 0 && a == 0 => has_semi = true,
                _ => {}
            }
        }
        if !has_semi {
            return true;
        }
    }
    if !unsized_params.is_empty() {
        let toks = collect_tokens_and_lifetimes(t);
        if toks.iter().any(|tk| unsized_params.contains(tk)) {
            let safe_ptr = t.starts_with("Box<")
                || t.starts_with("std::boxed::Box<")
                || t.starts_with("Arc<")
                || t.starts_with("Rc<")
                || t.starts_with("PhantomData<")
                || t.starts_with("core::marker::PhantomData<")
                || t.starts_with("std::marker::PhantomData<")
                || t.starts_with("NonNull<");
            if !safe_ptr {
                return true;
            }
        }
    }
    false
}

fn struct_already_has_new(doc_text: &str, masked_doc: &str, file_path: &str, struct_name: &str, ws: &WorkspaceIndex) -> bool {
    if doc_defines_inherent_new_or_macro(doc_text, masked_doc, struct_name) {
        return true;
    }
    let req_path = Path::new(file_path);
    // This struct's own `new` (its path, with or without the crate's name), not that of a same-named struct elsewhere.
    let mut news = HashSet::new();
    for s in ws.exact_local_symbols_by_name(struct_name) {
        if s.kind == DefKind::Struct && s.name.strip_prefix("r#").unwrap_or(s.name) == struct_name && s.resolved_path() == req_path {
            if ws.find_definition_by_path(&format!("{}::new", s.path)).is_some() {
                return true;
            }
            news.insert((s.crate_name.to_string(), format!("{}::new", s.path)));
            if let Some(rel) = s.path.strip_prefix(&format!("{}::", s.crate_name)) {
                news.insert((s.crate_name.to_string(), format!("{rel}::new")));
            }
        }
    }
    !news.is_empty() && ws.exact_local_symbols_by_name("new").iter().any(|sym| sym.name == "new" && news.contains(&(sym.crate_name.to_string(), sym.path.to_string())))
}

fn doc_defines_inherent_new_or_macro(doc_text: &str, masked_doc: &str, struct_name: &str) -> bool {
    let mb = masked_doc.as_bytes();
    let mut brace_depth = 0i32;
    let mut i = 0usize;
    while i < mb.len() {
        if brace_depth == 0 && mb[i] == b'!' && mb.get(i + 1).copied() != Some(b'=') {
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
                if contains_word(&masked_doc[k..em], struct_name) {
                    return true;
                }
                i = em;
                continue;
            }
        }
        if i + 4 < mb.len() && &mb[i..i + 4] == b"impl" && (i == 0 || !is_ident_byte(mb[i - 1])) && !is_ident_byte(mb[i + 4]) {
            let (_, after_g) = extract_angle_generics(doc_text, masked_doc, i + 4);
            if let Some(bo) = super::action_parse_decl::find_header_terminator(mb, after_g) {
                if mb[bo] == b'{' {
                    if let Some(bc_after) = skip_balanced(mb, bo, b'{', b'}') {
                        let (head_s, head_m, _) = split_top_level_where(&doc_text[after_g..bo], &masked_doc[after_g..bo]);
                        if super::action_parse_decl::split_impl_trait_for(head_s, head_m).is_none() {
                            let base_ty = head_s.split('<').next().unwrap_or(head_s).trim().rsplit("::").next().unwrap_or("").trim();
                            if base_ty.strip_prefix("r#").unwrap_or(base_ty) == struct_name {
                                let body_m = &masked_doc[bo + 1..bc_after - 1];
                                if super::action_default_scope::has_top_level_macro_call(body_m) || impl_body_has_fn_new(body_m) {
                                    return true;
                                }
                            }
                        }
                        i = bc_after;
                        continue;
                    }
                }
            }
        }
        match mb[i] {
            b'{' => brace_depth += 1,
            b'}' => brace_depth = (brace_depth - 1).max(0),
            _ => {}
        }
        i += 1;
    }
    false
}

fn impl_body_has_fn_new(body_masked: &str) -> bool {
    let b = body_masked.as_bytes();
    let (mut br, mut pa, mut bk) = (0i32, 0i32, 0i32);
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'{' => br += 1,
            b'}' => br = (br - 1).max(0),
            b'(' => pa += 1,
            b')' => pa = (pa - 1).max(0),
            b'[' => bk += 1,
            b']' => bk = (bk - 1).max(0),
            _ if br == 0
                && pa == 0
                && bk == 0
                && i + 2 <= b.len()
                && &b[i..i + 2] == b"fn"
                && (i == 0 || !is_ident_byte(b[i - 1]))
                && (i + 2 == b.len() || !is_ident_byte(b[i + 2])) =>
            {
                if let Some((n, _)) = next_ident_token(body_masked, i + 2) {
                    if n.strip_prefix("r#").unwrap_or(n) == "new" {
                        return true;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}
