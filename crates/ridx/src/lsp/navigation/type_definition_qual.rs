/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Qualified-path, `Self::Assoc`, and macro/raw-string filters for `textDocument/typeDefinition`.

use super::type_definition::{
    is_primitive_or_builtin, is_type_def_kind, peel_type_modifiers, strip_generics, FileReaderCtx,
};
use super::type_definition_lookup::lookup_type_by_path;
use crate::lsp::protocol::PositionConverter;
use ridx_format::{DefKind, DefKind::*, IndexReader, RefEntry, WorkspaceIndex};
use serde_json::Value;

/// Single-line lexical check for `use` declarations (`use ...`, `pub use ...`, `pub(...) use ...`).
///
/// Limitation: inspects only the queried line (`PositionConverter::get_line`) to avoid full-file
/// text scanning; multiline `use a::{\n B,\n};` continuation lines are resolved only if they do
/// not match an enclosing expression `TypeEntry`.
pub(super) fn is_use_statement_line(line_str: &str) -> bool {
    let trimmed = line_str.trim_start();
    if trimmed.starts_with("use ") || trimmed.starts_with("pub use ") {
        return true;
    }
    if let Some(after_pub) = trimmed.strip_prefix("pub(") {
        if let Some((_, rest)) = after_pub.split_once(')') {
            return rest.trim_start().starts_with("use ");
        }
    }
    false
}

/// Filters out cursor positions inside line comments, single-line raw strings, or macro/attribute
/// expansion call-site spans (Class (b): `enclosing_macro_or_attr`).
///
/// When rustc expands attribute or declarative macros, `resolve_span_coords` maps expanded HIR
/// references via `source_callsite()` to the entire macro invocation span:
/// - Attribute/test/snapshot macros (`#[test]`, `#[tokio::test]`, `expect![[...]]`, `tokio::select!`)
///   emit synthetic `Struct`/`Enum`/`Variant` references (`test::types::TestDescAndFn`, `expect_test::Expect`).
/// - Built-in assertion/panic macros (`assert!`, `assert_eq!`, `panic!`, `unreachable!`) and
///   attribute-generating macros (`define_label!` emitting `#[must_use]`) emit `DefKind::Fn`
///   references to `core::panicking::{panic, panic_fmt}` or `core::hint::must_use` across the callsite.
///
/// When the innermost `RefEntry` at the cursor is not a single identifier token and matches one of
/// these expansion targets, `typeDefinition` returns `null` rather than the synthetic macro helper.
pub(super) fn is_enclosing_macro_or_raw_string(
    ctx: &[FileReaderCtx<'_>],
    line_str: &str,
    col_1based: u32,
) -> bool {
    if line_str.trim_start().starts_with("//") || is_inside_raw_string(line_str, col_1based) {
        return true;
    }
    for c in ctx {
        let Some(r) = c.inner_ref else {
            continue;
        };
        if is_single_token_ref(c.reader, r, line_str) {
            return false;
        }
        let kind = DefKind::from(r.kind);
        let target_path = c.reader.get_str(r.target_path_offset());
        let target_name = target_path.rsplit("::").next().unwrap_or(target_path);
        if is_type_def_kind(kind)
            || matches!(kind, Variant | VariantCtor)
            || matches!(target_name, "must_use" | "panic" | "panic_fmt")
        {
            return true;
        }
    }
    false
}

/// Checks whether `r` covers a single identifier token (either matching `target_name.len()` or the
/// `Self` keyword when `target_path` was resolved to the concrete implementing type).
pub(super) fn is_single_token_ref<B: AsRef<[u8]>>(
    reader: &IndexReader<B>,
    r: &RefEntry,
    line_str: &str,
) -> bool {
    let span = r.span();
    if span.start_line() != span.end_line() {
        return false;
    }
    let target_path = reader.get_str(r.target_path_offset());
    let target_name = target_path.rsplit("::").next().unwrap_or(target_path);
    let width = span.end_col().saturating_sub(span.start_col()) as usize;
    width == target_name.len()
        || token_slice(line_str, span.start_col(), span.end_col()) == Some("Self")
}

fn token_slice(line_str: &str, start_col_1based: u32, end_col_1based: u32) -> Option<&str> {
    let start_idx = (start_col_1based as usize).checked_sub(1)?;
    let end_idx = (end_col_1based as usize).checked_sub(1)?;
    if start_idx >= end_idx {
        return None;
    }
    let mut chars = line_str.char_indices();
    let (start_byte, _) = chars.nth(start_idx)?;
    let end_byte = chars
        .nth(end_idx - start_idx - 1)
        .map_or(line_str.len(), |(b, _)| b);
    line_str.get(start_byte..end_byte)
}

pub(super) fn should_prefer_def_at(
    ctx: &[FileReaderCtx<'_>],
    (line, col): (u32, u32),
    line_str: &str,
) -> bool {
    let qual = extract_qualified_path(line_str, col).filter(|(p, _, _)| {
        p.contains("::")
            && p.rsplit("::")
                .next()
                .is_some_and(|last| last.starts_with(|c: char| c.is_ascii_uppercase()))
    });
    for c in ctx {
        let Some(entry) = c.inner_type else {
            continue;
        };
        let span = entry.span();
        if span.start_line() != span.end_line() {
            return true;
        }
        if let Some((_, p_sc, p_ec)) = qual {
            if span.start_line() < line
                || span.end_line() > line
                || span.start_col() < p_sc
                || span.end_col() > p_ec
            {
                return true;
            }
        }
    }
    false
}

pub(super) fn resolve_qualified_path_fallback(
    ws: &WorkspaceIndex,
    file_readers: &[usize],
    ctx: &[FileReaderCtx<'_>],
    (file, (line, col), line_str): (&str, (u32, u32), &str),
    type_entry: Option<(&str, Option<&str>)>,
    conv: &mut PositionConverter,
) -> Option<Value> {
    if let Some((ty, own)) = type_entry {
        if strip_generics(peel_type_modifiers(ty)) == "str"
            || own.is_some_and(|o| strip_generics(peel_type_modifiers(o)) == "str")
        {
            return None;
        }
    }
    let (qual_path, p_sc, p_ec) = extract_qualified_path(line_str, col)?;
    if !qual_path.contains("::") {
        return None;
    }
    let last_seg = qual_path.rsplit("::").next().unwrap_or("");
    if !last_seg.starts_with(|c: char| c.is_ascii_uppercase()) {
        return None;
    }

    for c in ctx {
        let refs = c.reader.references();
        let lo = refs.partition_point(|r| {
            let s = r.span();
            (s.file_id(), s.start_line(), s.start_col()) < (c.file_id, line, p_sc)
        });
        for r in &refs[lo..] {
            let span = r.span();
            if (span.file_id(), span.start_line()) != (c.file_id, line) || span.start_col() > p_ec {
                break;
            }
            if span.end_line() != line || span.end_col() > p_ec {
                continue;
            }
            let target_path = c.reader.get_str(r.target_path_offset());
            let target_name = target_path.rsplit("::").next().unwrap_or(target_path);
            if (span.end_col().saturating_sub(span.start_col()) as usize) != target_name.len() {
                continue;
            }
            let kind = DefKind::from(r.kind);
            if matches!(kind, Struct | StructCtor | Enum | Union | Trait) {
                if let Some(loc) =
                    lookup_type_by_path(ws, file_readers, file, target_path, true, conv)
                {
                    return Some(loc);
                }
            } else if matches!(kind, Variant | VariantCtor) {
                if let Some((enum_path, _)) = target_path.rsplit_once("::") {
                    if let Some(loc) =
                        lookup_type_by_path(ws, file_readers, file, enum_path, true, conv)
                    {
                        return Some(loc);
                    }
                }
            }
        }
    }

    if qual_path.starts_with("Self::") {
        let same_line_ty = ctx
            .iter()
            .any(|c| c.inner_type.is_some_and(|e| e.span().start_line() == line));
        if !same_line_ty {
            return resolve_self_type_in_impl(ws, file_readers, ctx, file, line, conv);
        }
    }
    None
}

fn resolve_self_type_in_impl(
    ws: &WorkspaceIndex,
    file_readers: &[usize],
    ctx: &[FileReaderCtx<'_>],
    file: &str,
    line: u32,
    conv: &mut PositionConverter,
) -> Option<Value> {
    for c in ctx {
        let Some(def) = ws.find_enclosing_impl_or_trait_def(c.idx, c.file_id, line) else {
            continue;
        };
        let def_path = c.reader.get_str(def.path_offset());
        let def_kind = DefKind::from(def.kind);
        if def_kind == Trait {
            if let Some(loc) = lookup_type_by_path(ws, file_readers, file, def_path, true, conv) {
                return Some(loc);
            }
            continue;
        }
        let owner_part = match def_kind {
            AssocFn | AssocTy | AssocConst => {
                def_path.rsplit_once("::").map_or(def_path, |(p, _)| p)
            }
            _ => def_path,
        };
        let cands: Vec<&str> = owner_part
            .strip_prefix('<')
            .and_then(|s| s.strip_suffix('>'))
            .map_or_else(|| vec![owner_part], |inner| inner.split(" as ").collect());
        for cand_ty in cands {
            let base = strip_generics(peel_type_modifiers(cand_ty));
            if !base.is_empty() && !is_primitive_or_builtin(base) {
                if let Some(loc) = lookup_type_by_path(ws, file_readers, file, base, true, conv) {
                    return Some(loc);
                }
            }
        }
    }
    None
}

fn extract_qualified_path(line_str: &str, col_1based: u32) -> Option<(&str, u32, u32)> {
    let chars: Vec<(usize, char)> = line_str.char_indices().collect();
    let idx = (col_1based as usize)
        .checked_sub(1)
        .filter(|&i| i < chars.len())?;
    let is_id = |c: char| c.is_ascii_alphanumeric() || c == '_';
    if !is_id(chars[idx].1) && chars[idx].1 != ':' {
        return None;
    }
    let mut start = idx;
    while start > 0 {
        if is_id(chars[start - 1].1) {
            start -= 1;
        } else if start >= 2 && chars[start - 1].1 == ':' && chars[start - 2].1 == ':' {
            start -= 2;
        } else {
            break;
        }
    }
    let mut end = idx + 1;
    while end < chars.len() {
        if is_id(chars[end].1) {
            end += 1;
        } else if end + 1 < chars.len() && chars[end].1 == ':' && chars[end + 1].1 == ':' {
            end += 2;
        } else {
            break;
        }
    }
    let (start_byte, end_byte) = (
        chars[start].0,
        chars.get(end).map_or(line_str.len(), |&(b, _)| b),
    );
    Some((
        &line_str[start_byte..end_byte],
        (start as u32) + 1,
        (end as u32) + 1,
    ))
}

/// Single-line lexical check for raw string literals (`r"..."`, `r#"..."#`).
///
/// Limitation: inspects only the queried line (`PositionConverter::get_line`); multiline raw
/// string bodies without a `TypeEntry` or with an enclosing `TypeEntry` are not lexically parsed across lines.
fn is_inside_raw_string(line_str: &str, col_1based: u32) -> bool {
    let bytes = line_str.as_bytes();
    let is_id = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'r' && (i == 0 || !is_id(bytes[i - 1])) {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] == b'#' {
                j += 1;
            }
            let hashes = j - (i + 1);
            if j < bytes.len() && bytes[j] == b'"' {
                let start_col = line_str[..i].chars().count() as u32 + 1;
                let mut k = j + 1;
                let mut end_byte = bytes.len();
                while k < bytes.len() {
                    if bytes[k] == b'"'
                        && k + hashes < bytes.len()
                        && bytes[k + 1..k + 1 + hashes].iter().all(|&b| b == b'#')
                    {
                        end_byte = k + 1 + hashes;
                        break;
                    }
                    k += 1;
                }
                let end_col = line_str[..end_byte].chars().count() as u32 + 1;
                if col_1based >= start_col && col_1based <= end_col {
                    return true;
                }
                i = end_byte;
                continue;
            }
        }
        i += 1;
    }
    false
}
