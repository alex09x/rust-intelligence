/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Cross-file caller processing for `Inline into all callers` (`refactor.inline`).

use super::action_inline_callers_helpers::{FnParamInfo, SelfKind};
use super::action_parse::{is_ident_byte, line_starts_of, mask_comments_and_strings};
use super::action_struct_conv_helpers::to_lsp_edits;
use ridx_format::WorkspaceIndex;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

const SAFE_PRELUDE_TYPES: &[&str] = &[
    "Option", "Result", "Some", "None", "Ok", "Err", "bool", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32", "f64", "str", "String",
    "Vec",
];

/// Checks whether a transformed function body is completely safe to inline into other files
/// without risking missing imports, private field accesses, or undeclared local functions.
pub(crate) fn is_safe_for_cross_file_inline(body: &str, masked_doc: &str) -> bool {
    let b = body.trim();
    if b.is_empty() || b.contains("super::") || b.contains("self::") || b.contains('\'') {
        return false;
    }
    // Reject tuple struct indexing like .0, .1
    let bytes = b.as_bytes();
    for (i, &byte) in bytes.iter().enumerate() {
        if byte == b'.' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
            return false;
        }
    }
    // Reject struct instantiation like `Foo { ... }`
    for (i, &byte) in bytes.iter().enumerate() {
        if byte.is_ascii_uppercase() && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_') {
            let mut j = i;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            let word = &b[i..j];
            let after = b[j..].trim_start();
            if after.starts_with('{') {
                return false;
            }
            if !SAFE_PRELUDE_TYPES.contains(&word) {
                return false;
            }
        }
    }
    // Reject bare function calls `foo(...)` unless preceded by `.` or in standard constructors
    let mut idx = 0;
    while let Some(open_paren) = b[idx..].find('(') {
        let abs_paren = idx + open_paren;
        let pre = b[..abs_paren].trim_end();
        let mut start = pre.len();
        while start > 0 {
            let prev_ch = pre.as_bytes()[start - 1];
            if prev_ch.is_ascii_alphanumeric() || prev_ch == b'_' {
                start -= 1;
            } else {
                break;
            }
        }
        if start < pre.len() {
            let fn_name = &pre[start..];
            let before_fn = pre[..start].trim_end();
            if !before_fn.ends_with('.') && !SAFE_PRELUDE_TYPES.contains(&fn_name) {
                return false;
            }
        }
        idx = abs_paren + 1;
    }
    // Reject bare field accesses `this.foo` unless followed by `(` (method call)
    let mut dot_idx = 0;
    while let Some(dot_pos) = b[dot_idx..].find('.') {
        let abs_dot = dot_idx + dot_pos;
        let after_dot = b[abs_dot + 1..].trim_start();
        let mut end = 0;
        while end < after_dot.len() {
            let ch = after_dot.as_bytes()[end];
            if ch.is_ascii_alphanumeric() || ch == b'_' {
                end += 1;
            } else {
                break;
            }
        }
        if end > 0 {
            let method_name = &after_dot[..end];
            let next_tail = after_dot[end..].trim_start();
            if !next_tail.starts_with('(') {
                return false;
            }
            if has_private_fn_def(method_name, masked_doc) {
                return false;
            }
        }
        dot_idx = abs_dot + 1;
    }
    true
}

fn has_private_fn_def(method: &str, masked_doc: &str) -> bool {
    let mb = masked_doc.as_bytes();
    let pat = format!("fn {method}");
    let pat_b = pat.as_bytes();
    let mut idx = 0;
    while let Some(pos) = masked_doc[idx..].find(&pat) {
        let abs = idx + pos;
        let prev_ok = abs == 0 || !is_ident_byte(mb[abs - 1]);
        let after_byte = abs + pat_b.len();
        let next_ok = after_byte < mb.len() && (mb[after_byte].is_ascii_whitespace() || mb[after_byte] == b'(' || mb[after_byte] == b'<');
        if prev_ok && next_ok {
            let pre = masked_doc[..abs].trim_end();
            let mut start = pre.len();
            while start > 0 && mb[start - 1] != b';' && mb[start - 1] != b'{' && mb[start - 1] != b'}' {
                start -= 1;
            }
            let vis_span = &pre[start..];
            if !vis_span.contains("pub") {
                return true;
            }
        }
        idx = abs + pat_b.len();
    }
    false
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn process_other_document_call_sites(
    abs_path: &Path,
    spans: &[(u32, u32)],
    fn_name: &str,
    self_kind: Option<SelfKind>,
    params: &[FnParamInfo],
    body_src: &str,
    body_has_question_mark: bool,
    effective_ret_ty: Option<&str>,
    file: &str,
    ws: Option<&WorkspaceIndex>,
    is_fully_pub: bool,
    other_file_edits: &mut BTreeMap<String, Vec<Value>>,
) -> bool {
    let Ok(other_text) = std::fs::read_to_string(abs_path) else {
        return true; // skipped
    };
    let other_starts = line_starts_of(&other_text);
    let other_masked = mask_comments_and_strings(&other_text);
    let (edits, skipped) = super::action_inline_callers::process_call_sites_for_document(
        &other_text,
        &other_masked,
        &other_starts,
        spans,
        fn_name,
        self_kind,
        params,
        body_src,
        body_has_question_mark,
        effective_ret_ty,
        None,
        Some(abs_path.to_str().unwrap_or(file)),
        ws,
        is_fully_pub,
    );
    if !edits.is_empty() {
        if let Some(lsp_edits) = to_lsp_edits(&other_text, &other_starts, edits) {
            let uri = super::super::protocol::path_to_uri(abs_path.to_str().unwrap_or(""));
            other_file_edits.insert(uri, lsp_edits);
        }
    }
    skipped
}

pub(crate) fn resolve_ref_to_absolute(ws: &WorkspaceIndex, doc_file: &str, r: &ridx_format::WorkspaceReference) -> std::path::PathBuf {
    let p = r.resolved_path();
    let doc_path = Path::new(doc_file);
    if p == doc_path || r.file == doc_file {
        return doc_path.to_path_buf();
    }
    let r_norm = r.file.strip_prefix('/').unwrap_or(r.file);
    if doc_file.ends_with(r_norm) {
        let prefix_len = doc_file.len() - r_norm.len();
        if prefix_len == 0 || doc_file.as_bytes()[prefix_len - 1] == b'/' {
            return doc_path.to_path_buf();
        }
    }
    if let Some(p_str) = p.to_str() {
        let p_norm = p_str.strip_prefix('/').unwrap_or(p_str);
        if doc_file.ends_with(p_norm) {
            let prefix_len = doc_file.len() - p_norm.len();
            if prefix_len == 0 || doc_file.as_bytes()[prefix_len - 1] == b'/' {
                return doc_path.to_path_buf();
            }
        }
    }
    if p.is_absolute() && p.exists() {
        return p;
    }
    if let Some(root) = ws.workspace_root() {
        let joined = root.join(&p);
        if joined.exists() {
            return joined;
        }
        let joined_rf = root.join(Path::new(r.file));
        if joined_rf.exists() {
            return joined_rf;
        }
    }
    if let Some(prefix) = doc_file.strip_suffix(r_norm) {
        let prefix_path = Path::new(prefix);
        let joined = prefix_path.join(&p);
        if joined.exists() {
            return joined;
        }
        let joined_rf = prefix_path.join(Path::new(r.file));
        if joined_rf.exists() {
            return joined_rf;
        }
    }
    p
}

pub(crate) fn is_same_doc(abs: &Path, doc_file: &str, canonical_file: Option<&Path>) -> bool {
    if abs == Path::new(doc_file) {
        return true;
    }
    if let Some(canon) = canonical_file {
        if let Ok(abs_canon) = std::fs::canonicalize(abs) {
            if abs_canon == canon {
                return true;
            }
        }
    }
    false
}
