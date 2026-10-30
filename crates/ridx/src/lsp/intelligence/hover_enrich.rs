/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Enriched hover extraction: full source declaration headers, leading `///` and
//! `#[cfg_attr(..., doc = "...")]` documentation, symbol provenance, and direct-reference
//! blast radius.

use crate::lsp::protocol::PositionConverter;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, ResolvedDefinition, WorkspaceIndex};

fn strip_line_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_str = false;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' && (i == 0 || bytes[i - 1] != b'\\') {
            in_str = !in_str;
        } else if !in_str && i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            return &line[..i];
        }
        i += 1;
    }
    line
}

fn truncate_at_body_delimiter(joined: &str) -> &str {
    let bytes = joined.as_bytes();
    let mut angle_depth = 0i32;
    let mut paren_depth = 0i32;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'>' => {
                i += 2;
                continue;
            }
            b'<' => angle_depth += 1,
            b'>' if angle_depth > 0 => angle_depth -= 1,
            b'(' => paren_depth += 1,
            b')' if paren_depth > 0 => paren_depth -= 1,
            b'{' | b';' if angle_depth == 0 && paren_depth == 0 => {
                return joined[..i].trim_end();
            }
            _ => {}
        }
        i += 1;
    }
    joined.trim_end()
}

fn matches_expected_decl_head(header: &str, kind_kw: &str, name: &str) -> bool {
    let mut words = header.split_whitespace().peekable();
    while let Some(&w) = words.peek() {
        if w == "pub"
            || w.starts_with("pub(")
            || w.ends_with(')')
            || matches!(w, "unsafe" | "async" | "const" | "auto" | "default")
        {
            words.next();
        } else {
            break;
        }
    }
    let Some(kw) = words.next() else {
        return false;
    };
    if kw != kind_kw {
        return false;
    }
    let Some(ident_tok) = words.next() else {
        return false;
    };
    let clean_ident = ident_tok
        .split(['<', '(', ':', '{', ';', '='])
        .next()
        .unwrap_or("")
        .trim_start_matches("r#");
    clean_ident == name.trim_start_matches("r#")
}

#[inline]
fn get_source_line<'a>(
    file_str: &str,
    line_no: u32,
    overlay_mgr: Option<&'a OverlayManager>,
    conv: &'a mut PositionConverter,
) -> Option<&'a str> {
    if let Some(ov_mgr) = overlay_mgr {
        if let Some(ov) = ov_mgr.get(file_str) {
            let idx = line_no.saturating_sub(1) as usize;
            if idx < ov.line_starts.len() && !ov.text.is_empty() {
                let start = ov.line_starts[idx];
                let end = ov
                    .line_starts
                    .get(idx + 1)
                    .copied()
                    .unwrap_or(ov.text.len());
                return Some(ov.text[start..end].trim_end_matches(['\r', '\n']));
            }
            return None;
        }
    }
    conv.get_line(file_str, line_no)
}

/// Extracts `doc = "..."` string value from a `#[doc = "..."]` or `#[cfg_attr(..., doc = "...")]` attribute.
fn extract_doc_from_attr(attr: &str) -> Option<String> {
    let bytes = attr.as_bytes();
    let mut i = 0;
    while i + 3 < bytes.len() {
        if &bytes[i..i + 3] == b"doc" {
            let prev_ok =
                i == 0 || matches!(bytes[i - 1], b'[' | b'(' | b',' | b' ' | b'\t' | b'\n');
            if prev_ok {
                let mut j = i + 3;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'=' {
                    j += 1;
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j < bytes.len() && bytes[j] == b'"' {
                        j += 1;
                        let mut out = String::new();
                        let str_part = &attr[j..];
                        let mut chars = str_part.chars();
                        while let Some(ch) = chars.next() {
                            if ch == '\\' {
                                match chars.next() {
                                    Some('n') => out.push('\n'),
                                    Some('t') => out.push('\t'),
                                    Some(c) => out.push(c),
                                    None => break,
                                }
                            } else if ch == '"' {
                                let clean = out.strip_prefix(' ').unwrap_or(&out);
                                return Some(clean.to_string());
                            } else {
                                out.push(ch);
                            }
                        }
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// Extracts the full source declaration header (including visibility, generics, supertraits,
/// and `where` bounds) for type and trait declarations whose `.ridx` record omits `signature`.
pub(crate) fn extract_source_decl_header(
    d: &ResolvedDefinition<'_>,
    ws: Option<&WorkspaceIndex>,
    overlay_mgr: Option<&OverlayManager>,
    conv: &mut PositionConverter,
) -> Option<String> {
    if d.start_line == 0
        || d.is_macro_expansion
        || d.file.is_empty()
        || d.file == "external"
        || d.reader_idx
            .is_some_and(|idx| ws.is_some_and(|w| w.is_reader_external(idx)))
        || !matches!(
            d.kind,
            DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::Trait | DefKind::TyAlias
        )
    {
        return None;
    }
    let resolved = d.resolved_path();
    let file_str = resolved.to_str().unwrap_or(d.file);
    let mut parts = Vec::new();
    for line_no in d.start_line..=(d.start_line + 7) {
        let Some(raw) = get_source_line(file_str, line_no, overlay_mgr, conv) else {
            break;
        };
        let clean = strip_line_comment(raw).trim();
        if clean.is_empty() {
            continue;
        }
        if parts.is_empty() && clean.starts_with("#[") {
            continue;
        }
        parts.push(clean.to_string());
        if clean.contains('{') || clean.contains(';') {
            break;
        }
    }
    if parts.is_empty() {
        return None;
    }
    let joined = parts.join(" ");
    let collapsed = joined.split_whitespace().collect::<Vec<_>>().join(" ");
    let header = truncate_at_body_delimiter(&collapsed);
    if matches_expected_decl_head(header, d.kind.as_str(), d.name) {
        Some(header.to_string())
    } else {
        None
    }
}

/// Extracts leading `///` and `#[cfg_attr(..., doc = "...")]` / `#[doc = "..."]` comments
/// immediately above `d.start_line`, skipping intermediate single-line or multi-line attributes.
pub(crate) fn extract_source_doc_comment(
    d: &ResolvedDefinition<'_>,
    ws: Option<&WorkspaceIndex>,
    overlay_mgr: Option<&OverlayManager>,
    conv: &mut PositionConverter,
    cursor_file: Option<&str>,
) -> Option<String> {
    if d.start_line <= 1
        || d.is_macro_expansion
        || d.file.is_empty()
        || d.file == "external"
        || d.reader_idx
            .is_some_and(|idx| ws.is_some_and(|w| w.is_reader_external(idx)))
        || matches!(
            d.kind,
            DefKind::Local
                | DefKind::TyParam
                | DefKind::LifetimeParam
                | DefKind::ConstParam
                | DefKind::Use
                | DefKind::Mod
        )
    {
        return None;
    }
    let resolved = d.resolved_path();
    let file_str = resolved.to_str().unwrap_or(d.file);
    let file_ready = overlay_mgr.is_some_and(|om| om.get(file_str).is_some())
        || conv.cache.contains_key(file_str)
        || cursor_file == Some(file_str);
    if !file_ready {
        return None;
    }

    let mut doc_lines_rev: Vec<String> = Vec::new();
    let mut multi_attr_rev: Vec<String> = Vec::new();
    let mut attr_bracket_depth: i32 = 0;
    let mut line_no = d.start_line - 1;
    let min_line = d.start_line.saturating_sub(80).max(1);

    while line_no >= min_line {
        let Some(raw) = get_source_line(file_str, line_no, overlay_mgr, conv) else {
            break;
        };
        let trimmed = raw.trim();
        if attr_bracket_depth > 0 {
            let no_comment = strip_line_comment(trimmed);
            multi_attr_rev.push(no_comment.to_string());
            for b in no_comment.bytes() {
                if b == b']' {
                    attr_bracket_depth += 1;
                } else if b == b'[' {
                    attr_bracket_depth -= 1;
                }
            }
            if attr_bracket_depth <= 0 {
                attr_bracket_depth = 0;
                multi_attr_rev.reverse();
                let joined_attr = multi_attr_rev.join(" ");
                multi_attr_rev.clear();
                if let Some(attr_doc) = extract_doc_from_attr(&joined_attr) {
                    doc_lines_rev.push(attr_doc);
                }
            }
            if line_no == 1 {
                break;
            }
            line_no -= 1;
            continue;
        }
        if trimmed.starts_with("///") && !trimmed.starts_with("////") {
            let body = trimmed.strip_prefix("/// ").unwrap_or(&trimmed[3..]);
            doc_lines_rev.push(body.to_string());
            if doc_lines_rev.len() >= 64 {
                break;
            }
        } else if trimmed.starts_with("#[") && trimmed.ends_with(']') {
            if let Some(attr_doc) = extract_doc_from_attr(trimmed) {
                doc_lines_rev.push(attr_doc);
            }
        } else if trimmed.ends_with(']') && !trimmed.starts_with("//") {
            let no_comment = strip_line_comment(trimmed);
            multi_attr_rev.clear();
            multi_attr_rev.push(no_comment.to_string());
            for b in no_comment.bytes() {
                if b == b']' {
                    attr_bracket_depth += 1;
                } else if b == b'[' {
                    attr_bracket_depth -= 1;
                }
            }
        } else {
            break;
        }
        if line_no == 1 {
            break;
        }
        line_no -= 1;
    }

    if doc_lines_rev.is_empty() {
        return None;
    }
    doc_lines_rev.reverse();
    doc_lines_rev.truncate(48);
    let doc = doc_lines_rev.join("\n").trim().to_string();
    (!doc.is_empty()).then_some(doc)
}

/// Formats the Provenance and direct-reference Blast Radius markdown block for `d`.
pub(crate) fn format_provenance_and_blast_radius(
    d: &ResolvedDefinition<'_>,
    ws: &WorkspaceIndex,
) -> Option<String> {
    if d.path.is_empty() || d.kind == DefKind::Local {
        return None;
    }
    let (crate_label, is_ext) = if let Some(idx) = d.reader_idx {
        if let Some(reader) = ws.readers().get(idx) {
            let cname = reader.crate_name();
            let cver = reader.crate_version();
            let label = if !cver.is_empty() && cver != "0.0.0" {
                format!("{cname} v{cver}")
            } else {
                cname.to_string()
            };
            (label, ws.is_reader_external(idx))
        } else {
            (
                d.path.split("::").next().unwrap_or(d.path).to_string(),
                false,
            )
        }
    } else {
        (
            d.path.split("::").next().unwrap_or(d.path).to_string(),
            false,
        )
    };

    let loc_part = if !d.file.is_empty() && d.file != "external" && d.start_line > 0 {
        format!(" (`{}:{}`)", d.file, d.start_line)
    } else if is_ext {
        " (external)".to_string()
    } else {
        String::new()
    };

    let blast = ws.direct_blast_radius(d);
    let ref_word = if blast.direct_refs == 1 {
        "reference"
    } else {
        "references"
    };
    let file_word = if blast.files == 1 { "file" } else { "files" };
    let crate_word = if blast.crates == 1 { "crate" } else { "crates" };

    Some(format!(
        "**Provenance:** `{}` · crate `{crate_label}`{loc_part}  \n**Blast Radius:** {} direct {ref_word} across {} {file_word} in {} {crate_word}",
        d.path, blast.direct_refs, blast.files, blast.crates
    ))
}
