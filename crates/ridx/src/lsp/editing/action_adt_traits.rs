/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compile-safe `Generate single field From`, `Generate Default from new`, and `Add explicit enum discriminants`.

use super::action_adt::{collect_unsized_type_params, format_adt_impl_parts, format_cfg_lines, is_potentially_unsized, strip_field_vis};
use super::action_crate_checks::{crate_already_implements_conversion_for, doc_already_implements_trait_for};
use super::action_default_scope::has_top_level_macro_call;
use super::action_generics_helpers::{clean_param_ident, contains_inferred_underscore};
use super::action_generics_sub::{parse_generics_list, split_top_level_colon, split_top_level_commas, strip_leading_attrs_keep_cfg, GenKind};
use super::action_parse::{byte_to_utf16_pos, is_ident_byte, mask_comments_and_strings, AdtFieldsKind, EnclosingContext, ItemKindInfo, ParsedCursorItem};
use super::action_parse_decl::{collapse_ws, next_ident_token};
use super::action_parse_items::{contains_word, skip_balanced};
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};

pub fn build_single_field_from(doc_text: &str, masked_doc: &str, starts: &[usize], file_path: &str, item: &ParsedCursorItem) -> Option<(u32, u32, String)> {
    if !matches!(item.enclosing, EnclosingContext::None) {
        return None;
    }
    let ItemKindInfo::Adt { adt_kw: "struct", generics_src, where_src, fields_kind, end_byte, .. } = &item.kind else {
        return None;
    };
    if contains_word(&mask_comments_and_strings(generics_src), "cfg") {
        return None;
    }
    let outer_attrs = &doc_text[item.first_attr_byte..item.decl_start_byte];
    if contains_word(outer_attrs, "From") {
        return None;
    }

    let unsized_params = collect_unsized_type_params(generics_src, where_src);
    let (param_name, fty, ctor_expr) = match fields_kind {
        AdtFieldsKind::Named(raw) => {
            let mf = mask_comments_and_strings(raw);
            if contains_word(&mf, "cfg") || contains_word(&mf, "cfg_attr") || mf.contains('$') {
                return None;
            }
            let entries: Vec<String> = split_top_level_commas(raw).into_iter().filter(|e| !e.trim().is_empty()).collect();
            let mut fields = Vec::new();
            for e in &entries {
                let (_, rest) = strip_leading_attrs_keep_cfg(e);
                let clean = strip_field_vis(&rest);
                let (fn_raw, Some(ft_raw)) = split_top_level_colon(clean) else {
                    return None;
                };
                fields.push((clean_param_ident(fn_raw.trim())?, collapse_ws(ft_raw)));
            }
            // One field takes the value; any other must be a `PhantomData`, built as such.
            let (data, phantoms): (Vec<_>, Vec<_>) = fields.into_iter().partition(|(_, ty)| phantom_value(ty).is_none());
            let [(fname, fty)] = <[_; 1]>::try_from(data).ok()?;
            let rest: Vec<String> = phantoms.iter().map(|(n, ty)| format!("{n}: {}", phantom_value(ty).unwrap_or_default())).collect();
            let ctor = if rest.is_empty() { format!("Self {{ {fname} }}") } else { format!("Self {{ {fname}, {} }}", rest.join(", ")) };
            (fname, fty, ctor)
        }
        AdtFieldsKind::Tuple(raw) => {
            let mf = mask_comments_and_strings(raw);
            if contains_word(&mf, "cfg") || contains_word(&mf, "cfg_attr") || mf.contains('$') {
                return None;
            }
            let entries: Vec<String> = split_top_level_commas(raw).into_iter().filter(|e| !e.trim().is_empty()).collect();
            let tys: Vec<String> = entries.iter().map(|e| collapse_ws(strip_field_vis(&strip_leading_attrs_keep_cfg(e).1))).collect();
            let data: Vec<usize> = (0..tys.len()).filter(|&i| phantom_value(&tys[i]).is_none()).collect();
            let [at] = <[usize; 1]>::try_from(data).ok()?;
            let args: Vec<String> = tys.iter().enumerate().map(|(i, ty)| if i == at { "value".to_string() } else { phantom_value(ty).unwrap_or_default() }).collect();
            ("value".to_string(), tys[at].clone(), format!("Self({})", args.join(", ")))
        }
        _ => return None,
    };

    let fty_masked = mask_comments_and_strings(&fty);
    if fty.is_empty()
        || contains_inferred_underscore(&fty)
        || contains_word(&fty_masked, "Self")
        || names_own_type(&fty_masked, &item.name)
        || is_potentially_unsized(&fty, &unsized_params)
    {
        return None;
    }
    // Coherence guard (`E0119` against `impl<T> From<T> for T` or unnormalized `<T as Trait>::Assoc` projections):
    let decls = parse_generics_list(generics_src);
    // A bare parameter (`struct W<C>(C)`) is fine: `W<C>` is never `C`, so `From<C> for W<C>` meets no blanket impl.
    if decls.iter().any(|g| g.kind == GenKind::Type && (fty.contains(&format!("{}::", g.name)) || fty.contains(&format!("<{} as", g.name)))) {
        return None;
    }
    if crate_already_implements_conversion_for(masked_doc, file_path, &item.name, &fty) {
        return None;
    }

    let (impl_gens, type_args, where_clause) = format_adt_impl_parts(generics_src, where_src)?;
    let indent = &item.indent;
    let cfg_prefix = format_cfg_lines(&item.cfg_attrs, indent);
    let new_text = format!(
        "\n\n{cfg_prefix}{indent}impl{impl_gens} From<{fty}> for {}{type_args}{where_clause} {{\n{indent}    fn from({param_name}: {fty}) -> Self {{\n{indent}        {ctor_expr}\n{indent}    }}\n{indent}}}",
        item.raw_name
    );
    let (line0, col_u16) = byte_to_utf16_pos(doc_text, starts, *end_byte);
    Some((line0, col_u16, new_text))
}

pub fn build_default_from_new(doc_text: &str, masked_doc: &str, starts: &[usize], item: &ParsedCursorItem, ws: &WorkspaceIndex) -> Option<(u32, u32, String)> {
    if item.name != "new" || !item.cfg_attrs.is_empty() {
        return None;
    }
    let ItemKindInfo::Fn { is_unsafe: false, extern_abi: None, generics_src: fn_gens, params_src, ret_src, where_src: fn_where, has_body: true, body_range: Some((bo, _)) } =
        &item.kind
    else {
        return None;
    };
    if !fn_gens.trim().is_empty() || !fn_where.trim().is_empty() || !params_src.trim().is_empty() {
        return None;
    }
    if contains_word(&masked_doc[item.decl_start_byte..*bo], "async") {
        return None;
    }
    let EnclosingContext::InherentImpl { generics_src, self_ty, where_src, cfg_attrs, indent, body_close_byte, .. } = &item.enclosing else {
        return None;
    };
    let ret = ret_src.as_deref()?.trim();
    if ret != "Self" && ret != self_ty.trim() {
        return None;
    }

    let base_raw = self_ty.split('<').next().unwrap_or(self_ty).trim().rsplit("::").next().unwrap_or("").trim();
    let base_name = base_raw.strip_prefix("r#").unwrap_or(base_raw);
    if base_name.is_empty() || !base_name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    if doc_already_implements_or_derives_default(doc_text, masked_doc, base_name, ws) {
        return None;
    }

    let (impl_gens, _, where_clause) = format_adt_impl_parts(generics_src, where_src)?;
    let cfg_prefix = format_cfg_lines(cfg_attrs, indent);
    let new_text = format!(
        "\n\n{cfg_prefix}{indent}impl{impl_gens} Default for {self_ty}{where_clause} {{\n{indent}    fn default() -> Self {{\n{indent}        Self::new()\n{indent}    }}\n{indent}}}"
    );
    let (line0, col_u16) = byte_to_utf16_pos(doc_text, starts, *body_close_byte + 1);
    Some((line0, col_u16, new_text))
}

pub fn build_enum_discriminants(doc_text: &str, masked_doc: &str, starts: &[usize], item: &ParsedCursorItem) -> Option<Vec<Value>> {
    let ItemKindInfo::Adt { adt_kw: "enum", open_delim_byte, end_byte, .. } = &item.kind else {
        return None;
    };
    if *open_delim_byte + 1 >= *end_byte || doc_text.as_bytes().get(*open_delim_byte).copied() != Some(b'{') {
        return None;
    }
    let body_src = &doc_text[*open_delim_byte + 1..*end_byte - 1];
    let body_masked = &masked_doc[*open_delim_byte + 1..*end_byte - 1];
    if has_top_level_macro_call(body_masked) || body_masked.contains('$') {
        return None;
    }
    let outer_attrs = &doc_text[item.first_attr_byte..item.decl_start_byte];
    let has_int_repr = has_explicit_int_repr(outer_attrs);

    let mut edits = Vec::new();
    let mut next_val: i128 = 0;
    let mb = body_masked.as_bytes();
    let mut seg_start = 0usize;
    let (mut p, mut b, mut c) = (0i32, 0i32, 0i32);
    let mut spans = Vec::new();
    for (idx, &ch) in mb.iter().enumerate() {
        match ch {
            b'(' => p += 1,
            b')' => p = (p - 1).max(0),
            b'[' => b += 1,
            b']' => b = (b - 1).max(0),
            b'{' => c += 1,
            b'}' => c = (c - 1).max(0),
            b',' if p == 0 && b == 0 && c == 0 => {
                spans.push((seg_start, idx));
                seg_start = idx + 1;
            }
            _ => {}
        }
    }
    spans.push((seg_start, mb.len()));

    for (lo, hi) in spans {
        let seg_m = &body_masked[lo..hi];
        if seg_m.trim().is_empty() {
            continue;
        }
        if contains_word(seg_m, "cfg") || contains_word(seg_m, "cfg_attr") {
            return None;
        }
        let mut cur = lo;
        while cur < hi {
            while cur < hi && mb[cur].is_ascii_whitespace() {
                cur += 1;
            }
            if mb.get(cur).copied() == Some(b'#') {
                let mut k = cur + 1;
                while k < hi && mb[k].is_ascii_whitespace() {
                    k += 1;
                }
                cur = skip_balanced(mb, k, b'[', b']')?;
            } else {
                break;
            }
        }
        if cur >= hi {
            continue;
        }
        let (_, name_end) = next_ident_token(body_src, cur)?;
        let mut after_var = name_end;
        let mut q = after_var;
        while q < hi && mb[q].is_ascii_whitespace() {
            q += 1;
        }
        if q < hi && (mb[q] == b'(' || mb[q] == b'{') {
            if !has_int_repr {
                return None;
            }
            after_var = if mb[q] == b'(' { skip_balanced(mb, q, b'(', b')')? } else { skip_balanced(mb, q, b'{', b'}')? };
            q = after_var;
            while q < hi && mb[q].is_ascii_whitespace() {
                q += 1;
            }
        }
        if q < hi && mb[q] == b'=' {
            let rhs = body_src[q + 1..hi].trim();
            next_val = parse_int_discriminant(rhs)?.checked_add(1)?;
        } else if q == hi {
            let abs_byte = *open_delim_byte + 1 + after_var;
            let (ln, col) = byte_to_utf16_pos(doc_text, starts, abs_byte);
            edits.push(json!({
                "range": { "start": { "line": ln, "character": col }, "end": { "line": ln, "character": col } },
                "newText": format!(" = {next_val}")
            }));
            next_val = next_val.checked_add(1)?;
        } else {
            return None;
        }
    }

    (!edits.is_empty()).then_some(edits)
}

fn has_explicit_int_repr(attrs: &str) -> bool {
    const INTS: &[&str] = &["u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize"];
    contains_word(attrs, "repr") && INTS.iter().any(|t| contains_word(attrs, t))
}

fn parse_int_discriminant(rhs: &str) -> Option<i128> {
    let clean: String = rhs.chars().filter(|&c| c != '_').collect();
    let t = clean.trim().trim_end_matches(|c: char| c.is_ascii_alphabetic());
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        i128::from_str_radix(hex, 16).ok()
    } else if let Some(bin) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
        i128::from_str_radix(bin, 2).ok()
    } else {
        t.parse::<i128>().ok()
    }
}

fn doc_already_implements_or_derives_default(doc_text: &str, masked_doc: &str, base_name: &str, ws: &WorkspaceIndex) -> bool {
    if doc_already_implements_trait_for(masked_doc, "Default", base_name) {
        return true;
    }
    // Check `#[derive(... Default ...)]` preceding `struct/enum <base_name>` in `doc_text`.
    let mb = masked_doc.as_bytes();
    let mut i = 0usize;
    while i + 6 < mb.len() {
        if (&mb[i..i + 6] == b"struct" || &mb[i..i + 4] == b"enum") && (i == 0 || !is_ident_byte(mb[i - 1])) {
            let kw_len = if &mb[i..i + 6] == b"struct" { 6 } else { 4 };
            if !is_ident_byte(mb[i + kw_len]) {
                if let Some((n, _)) = next_ident_token(doc_text, i + kw_len) {
                    if n.strip_prefix("r#").unwrap_or(n) == base_name {
                        let lookback = doc_text[..i].rfind('}').map(|p| p + 1).unwrap_or(0);
                        let attrs = &doc_text[lookback..i];
                        if contains_word(attrs, "derive") && contains_word(attrs, "Default") {
                            return true;
                        }
                    }
                }
            }
        }
        i += 1;
    }
    let sfx = format!("::{base_name}::default");
    let exact = format!("{base_name}::default");
    ws.exact_symbols_by_name("default").into_iter().any(|s| s.path == exact || s.path.ends_with(&sfx))
}

/// The value of a `PhantomData` field's type (`PhantomData<&'a ()>` → `PhantomData`, `core::marker::PhantomData<T>` →
/// `core::marker::PhantomData`) or of a unit field (`()`), as rust-analyzer builds them; `None` for any other type.
fn phantom_value(ty: &str) -> Option<String> {
    if ty.trim() == "()" {
        return Some("()".to_string());
    }
    let base = ty.split('<').next().unwrap_or(ty).trim();
    (base.rsplit("::").next() == Some("PhantomData")).then(|| base.to_string())
}

/// Whether the field type may name the struct itself: its name bare or under `crate`/`self`/`super`, not one reached
/// through another module or crate (`regex_automata::MatchKind` in `struct MatchKind`).
fn names_own_type(fty_masked: &str, name: &str) -> bool {
    let b = fty_masked.as_bytes();
    fty_masked.match_indices(name).any(|(p, _)| {
        if (p > 0 && is_ident_byte(b[p - 1])) || b.get(p + name.len()).is_some_and(|c| is_ident_byte(*c)) {
            return false;
        }
        let path = fty_masked[..p].rsplit(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':')).next().unwrap_or("");
        let root = path.split("::").next().unwrap_or("");
        path.is_empty() || matches!(root, "crate" | "self" | "super" | "Self")
    })
}

#[cfg(test)]
mod tests {
    use super::{names_own_type, phantom_value};

    #[test]
    fn a_field_type_from_another_module_is_another_type() {
        assert!(names_own_type("Box<MatchKind>", "MatchKind"));
        assert!(names_own_type("Option<crate::flags::MatchKind>", "MatchKind"));
        assert!(!names_own_type("regex_automata::MatchKind", "MatchKind"));
        assert!(!names_own_type("onepass::Config", "Config"));
        assert!(!names_own_type("MatchKinds", "MatchKind"));
    }

    #[test]
    fn a_phantom_field_is_built_by_its_path() {
        assert_eq!(phantom_value("PhantomData<&'a ()>").as_deref(), Some("PhantomData"));
        assert_eq!(phantom_value("core::marker::PhantomData<fn() -> T>").as_deref(), Some("core::marker::PhantomData"));
        assert_eq!(phantom_value("Vec<PhantomData<u8>>"), None);
        assert_eq!(phantom_value("()").as_deref(), Some("()"));
    }
}
