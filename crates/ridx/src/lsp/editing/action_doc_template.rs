/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Context-aware Rustdoc template generator (`/// .`, constructor/getter/setter summaries, `# Panics`, `# Errors`, `# Safety`).

use super::action_generics_sub::{split_top_level_colon, split_top_level_commas, strip_leading_attrs_keep_cfg};
use super::action_parse::{mask_comments_and_strings, EnclosingContext, ItemKindInfo, ParsedCursorItem};
use super::action_parse_items::contains_word;

pub fn build_doc_template(masked_doc: &str, item: &ParsedCursorItem) -> Option<(u32, u32, String)> {
    let ItemKindInfo::Fn { is_unsafe, params_src, ret_src, has_body, body_range, .. } = &item.kind else {
        return None;
    };
    // A trait impl's method takes documentation too (rust-analyzer offers it there as well).
    if item.has_doc {
        return None;
    }
    if !*has_body && !matches!(item.enclosing, EnclosingContext::TraitDecl { .. }) {
        return None;
    }

    let indent = &item.indent;
    let summary = build_summary_line(&item.name, params_src, ret_src.as_deref(), &item.enclosing);
    let mut lines = vec![format!("{indent}/// {summary}")];

    let can_panic = body_range.filter(|&(bo, bc)| bo + 1 < bc && bc <= masked_doc.len()).is_some_and(|(bo, bc)| body_masked_can_panic(&masked_doc[bo + 1..bc - 1]));
    if can_panic {
        lines.push(format!("{indent}///"));
        lines.push(format!("{indent}/// # Panics"));
        lines.push(format!("{indent}///"));
        lines.push(format!("{indent}/// Panics if ."));
    }

    if return_is_result(ret_src.as_deref()) {
        lines.push(format!("{indent}///"));
        lines.push(format!("{indent}/// # Errors"));
        lines.push(format!("{indent}///"));
        lines.push(format!("{indent}/// This function will return an error if ."));
    }

    if *is_unsafe {
        lines.push(format!("{indent}///"));
        lines.push(format!("{indent}/// # Safety"));
        lines.push(format!("{indent}///"));
        lines.push(format!("{indent}/// ."));
    }

    let mut out = lines.join("\n");
    out.push('\n');
    Some((item.attr_start_line0, 0, out))
}

fn build_summary_line(fn_name: &str, params_src: &str, ret_src: Option<&str>, enclosing: &EnclosingContext) -> String {
    let EnclosingContext::InherentImpl { self_ty, .. } = enclosing else {
        return ".".to_string();
    };
    let Some(self_doc) = format_doc_self_ty(self_ty) else {
        return ".".to_string();
    };
    let clean_params: Vec<String> = split_top_level_commas(params_src).into_iter().map(|p| strip_leading_attrs_keep_cfg(&p).1).filter(|p| !p.is_empty()).collect();

    if fn_name == "new" && ret_mentions_self_or_owner(ret_src, &self_doc) {
        return format!("Creates a new [`{self_doc}`].");
    }

    // rust-analyzer's introduction: a getter is any method taking `&self` or `&mut self` and nothing else (but
    // `as_…`, `to_…` and `get`), whatever it returns; a setter is any `set_…` method.
    let by_ref_self = clean_params.first().is_some_and(|p| is_ref_self_param(p));
    if by_ref_self && clean_params.len() == 1 && !fn_name.starts_with("as_") && !fn_name.starts_with("to_") && fn_name != "get" {
        let what = property_words(fn_name.trim_end_matches("_mut"));
        let ret = ret_src.map(str::trim).unwrap_or("");
        let reference = if ret.starts_with("&mut ") || (ret.starts_with("&'") && ret.split_whitespace().nth(1) == Some("mut")) {
            " a mutable reference to"
        } else if ret.starts_with('&') {
            " a reference to"
        } else {
            ""
        };
        return format!("Returns{reference} the {what} of this [`{self_doc}`].");
    }
    if let Some(prop) = fn_name.strip_prefix("set_") {
        return format!("Sets the {} of this [`{self_doc}`].", property_words(prop));
    }

    ".".to_string()
}

/// A method name as the words of its property (`max_len` → `max len`, `len` → `length`).
fn property_words(name: &str) -> String {
    let what = name.replace('_', " ");
    if what == "len" {
        "length".to_string()
    } else {
        what
    }
}

/// Whether the parameter is `&self`, `&mut self` or `&'a self`, or typed `self: &…`.
fn is_ref_self_param(param: &str) -> bool {
    match classify_self_param(param) {
        Some(_) => {
            let t = param.trim();
            t.starts_with('&') || split_top_level_colon(t).1.is_some_and(|ty| ty.trim().starts_with('&'))
        }
        None => false,
    }
}

fn classify_self_param(param: &str) -> Option<bool> {
    let t = param.trim();
    if t == "self" || t == "mut self" {
        return Some(t.starts_with("mut "));
    }
    if let Some(rest) = t.strip_prefix('&') {
        let mut r = rest.trim_start();
        if r.starts_with('\'') {
            r = r.split_once(char::is_whitespace)?.1.trim_start();
        }
        if r == "self" {
            return Some(false);
        }
        if r == "mut self" {
            return Some(true);
        }
    }
    let (lhs, rhs) = split_top_level_colon(t);
    if rhs.is_some() && matches!(lhs.trim(), "self" | "mut self") {
        return Some(lhs.trim().starts_with("mut "));
    }
    None
}

fn format_doc_self_ty(self_ty: &str) -> Option<String> {
    let t = self_ty.trim();
    if t.is_empty() || t.starts_with('&') || t.starts_with('(') || t.starts_with('[') {
        return None;
    }
    let Some(lt_pos) = t.find('<') else {
        let leaf = t.rsplit("::").next().unwrap_or(t).trim();
        let clean = leaf.strip_prefix("r#").unwrap_or(leaf);
        return (!clean.is_empty()).then(|| clean.to_string());
    };
    let gt_pos = t.rfind('>')?;
    let base_raw = t[..lt_pos].trim().rsplit("::").next().unwrap_or("").trim();
    let base = base_raw.strip_prefix("r#").unwrap_or(base_raw);
    if base.is_empty() {
        return None;
    }
    let kept: Vec<String> = split_top_level_commas(&t[lt_pos + 1..gt_pos]).into_iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty() && !a.starts_with('\'')).collect();
    if kept.is_empty() {
        Some(base.to_string())
    } else {
        Some(format!("{base}<{}>", kept.join(", ")))
    }
}

fn ret_mentions_self_or_owner(ret_src: Option<&str>, self_doc: &str) -> bool {
    let Some(r) = ret_src else {
        return false;
    };
    let base = self_doc.split('<').next().unwrap_or(self_doc).trim();
    let rm = mask_comments_and_strings(r);
    contains_word(&rm, "Self") || (!base.is_empty() && contains_word(&rm, base))
}

fn return_is_result(ret_src: Option<&str>) -> bool {
    let Some(r) = ret_src else {
        return false;
    };
    let base = r.split('<').next().unwrap_or(r).trim();
    base == "Result" || base.ends_with("::Result")
}

fn body_masked_can_panic(body_masked: &str) -> bool {
    contains_word(body_masked, "panic")
        || contains_word(body_masked, "assert")
        || contains_word(body_masked, "assert_eq")
        || contains_word(body_masked, "assert_ne")
        || contains_word(body_masked, "unreachable")
        || body_masked.contains(".unwrap(")
        || body_masked.contains(".expect(")
}
