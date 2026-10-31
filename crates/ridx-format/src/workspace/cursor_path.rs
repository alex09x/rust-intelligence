/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Cursor-position path segment, `Self::Assoc`, and `T::Assoc` resolution for `find_definition_at`.

use super::WorkspaceIndex;
use crate::format::join_resolving_overlap;
use crate::reader::ResolvedDefinition;
use std::path::Path;

pub(super) fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

pub(super) fn strip_generics_and_refs(s: &str) -> &str {
    let mut t = s.trim();
    while let Some(r) = t.strip_prefix("&mut ").or_else(|| t.strip_prefix('&')).or_else(|| t.strip_prefix("mut ")).or_else(|| t.strip_prefix("dyn ")) {
        t = r.trim_start();
    }
    if t.starts_with('\'') {
        if let Some((_, after)) = t.split_once(char::is_whitespace) {
            t = after.trim_start();
            if let Some(r) = t.strip_prefix("mut ") {
                t = r.trim_start();
            }
        }
    }
    let end = t.find(['<', '(', '[', ' ']).unwrap_or(t.len());
    t[..end].trim_end_matches("::")
}

fn extract_use_tree_prefix(lines: &[&str], line_idx: usize, p_start_byte: usize) -> Option<String> {
    let mut braces = 0i32;
    let mut prefixes = Vec::new();
    let min_line = line_idx.saturating_sub(25);
    for l in (min_line..=line_idx).rev() {
        let s = if l == line_idx { &lines[l][..p_start_byte.min(lines[l].len())] } else { lines[l] };
        let code = s.split("//").next().unwrap_or(s);
        let bytes = code.as_bytes();
        let mut i = bytes.len();
        while i > 0 {
            i -= 1;
            match bytes[i] {
                b'}' => braces += 1,
                b';' if braces == 0 => return None,
                b'{' => {
                    if braces > 0 {
                        braces -= 1;
                    } else {
                        let before = code[..i].trim_end();
                        if let Some(before_colons) = before.strip_suffix("::") {
                            let mut k = before_colons.len();
                            while k > 0 && (is_ident_byte(before_colons.as_bytes()[k - 1]) || before_colons.as_bytes()[k - 1] == b':') {
                                k -= 1;
                            }
                            let seg = before_colons[k..].trim_matches(':');
                            if !seg.is_empty() {
                                prefixes.push(seg.to_string());
                            }
                            i = k;
                        }
                    }
                }
                _ => {}
            }
        }
        let trimmed = code.trim_start();
        let is_use = trimmed.starts_with("use ") || trimmed.starts_with("pub use ") || (trimmed.starts_with("pub(") && trimmed.contains(") use "));
        if is_use {
            prefixes.reverse();
            return Some(prefixes.join("::"));
        }
    }
    None
}

enum LexState {
    Normal,
    BlockComment(u32),
    StringLit,
    RawString(usize),
}

pub(super) fn is_in_string_or_comment(lines: &[&str], target_line: usize, target_byte: usize) -> bool {
    if target_line >= lines.len() {
        return false;
    }
    let mut state = LexState::Normal;
    for (l, line) in lines.iter().enumerate().take(target_line + 1) {
        let b = line.as_bytes();
        let end = if l == target_line { target_byte.min(b.len()) } else { b.len() };
        let mut i = 0;
        while i < end {
            match state {
                LexState::BlockComment(ref mut depth) => {
                    if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
                        *depth += 1;
                        i += 2;
                    } else if i + 1 < b.len() && b[i] == b'*' && b[i + 1] == b'/' {
                        if l == target_line && i + 1 >= end {
                            return true;
                        }
                        *depth -= 1;
                        i += 2;
                        if *depth == 0 {
                            state = LexState::Normal;
                        }
                    } else {
                        i += 1;
                    }
                }
                LexState::StringLit => {
                    if b[i] == b'\\' {
                        i += 2;
                    } else if b[i] == b'"' {
                        state = LexState::Normal;
                        i += 1;
                    } else {
                        i += 1;
                    }
                }
                LexState::RawString(hashes) => {
                    if b[i] == b'"' && i + 1 + hashes <= b.len() && b[i + 1..i + 1 + hashes].iter().all(|&x| x == b'#') {
                        if l == target_line && i + hashes >= end {
                            return true;
                        }
                        state = LexState::Normal;
                        i += 1 + hashes;
                    } else {
                        i += 1;
                    }
                }
                LexState::Normal => {
                    if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'/' {
                        if l == target_line {
                            return true;
                        }
                        break;
                    } else if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
                        state = LexState::BlockComment(1);
                        i += 2;
                    } else if matches!(b[i], b'r' | b'b' | b'c') && (i == 0 || !(is_ident_byte(b[i - 1]) || b[i - 1] == b'\'' || b[i - 1] == b'"')) {
                        let raw_after = if b[i] == b'r' {
                            Some(i + 1)
                        } else if i + 1 < b.len() && b[i + 1] == b'r' {
                            Some(i + 2)
                        } else {
                            None
                        };
                        if let Some(r0) = raw_after {
                            let mut j = r0;
                            while j < b.len() && b[j] == b'#' {
                                j += 1;
                            }
                            if j < b.len() && b[j] == b'"' {
                                state = LexState::RawString(j - r0);
                                i = j + 1;
                                continue;
                            }
                        }
                        if matches!(b[i], b'b' | b'c') && i + 1 < b.len() && b[i + 1] == b'"' {
                            state = LexState::StringLit;
                            i += 2;
                        } else {
                            i += 1;
                        }
                    } else if b[i] == b'"' {
                        state = LexState::StringLit;
                        i += 1;
                    } else if b[i] == b'\'' {
                        if i + 2 < b.len() && b[i + 1] != b'\\' && b[i + 2] == b'\'' {
                            i += 3;
                        } else if i + 1 < b.len() && b[i + 1] == b'\\' {
                            let mut k = i + 2;
                            while k < b.len() && k <= i + 10 && b[k] != b'\'' {
                                k += 1;
                            }
                            i = if k < b.len() && b[k] == b'\'' { k + 1 } else { i + 1 };
                        } else {
                            i += 1;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
        }
    }
    !matches!(state, LexState::Normal)
}

impl WorkspaceIndex {
    pub(crate) fn read_source_file(&self, file: &str) -> Option<String> {
        let p = Path::new(file);
        if p.is_absolute() && p.is_file() {
            return std::fs::read_to_string(p).ok();
        }
        for root in [self.workspace_root.as_deref(), self.root_dir.as_deref()].into_iter().flatten() {
            let full = join_resolving_overlap(root, p);
            if full.is_file() {
                return std::fs::read_to_string(full).ok();
            }
        }
        for i in self.candidate_reader_indices_for_file(file) {
            if let Some(Some(root)) = self.source_roots.get(i) {
                let full = join_resolving_overlap(root, p);
                if full.is_file() {
                    return std::fs::read_to_string(full).ok();
                }
            }
        }
        if p.is_file() {
            return std::fs::read_to_string(p).ok();
        }
        None
    }

    pub(crate) fn cursor_ident_at(&self, file: &str, line: u32, col: u32) -> Option<(String, u32, u32)> {
        let content = self.read_source_file(file)?;
        let line_idx = (line as usize).checked_sub(1)?;
        let line_str = content.lines().nth(line_idx)?;
        let chars: Vec<(usize, char)> = line_str.char_indices().collect();
        if chars.is_empty() {
            return None;
        }
        let cidx = (col as usize).saturating_sub(1).min(chars.len() - 1);
        let bpos = chars[cidx].0;
        let bytes = line_str.as_bytes();
        let pos = if bpos < bytes.len() && is_ident_byte(bytes[bpos]) {
            bpos
        } else if bpos > 0 && is_ident_byte(bytes[bpos - 1]) {
            bpos - 1
        } else {
            return None;
        };
        let mut s = pos;
        while s > 0 && is_ident_byte(bytes[s - 1]) {
            s -= 1;
        }
        let mut e = pos + 1;
        while e < bytes.len() && is_ident_byte(bytes[e]) {
            e += 1;
        }
        if bytes[s].is_ascii_digit() {
            return None;
        }
        let sc = line_str[..s].chars().count() as u32 + 1;
        let ec = line_str[..e].chars().count() as u32 + 1;
        Some((line_str[s..e].to_string(), sc, ec))
    }

    pub(crate) fn resolve_cursor_path_at<'a>(&'a self, file: &str, line: u32, col: u32) -> Option<ResolvedDefinition<'a>> {
        let content = self.read_source_file(file)?;
        let line_idx = (line as usize).checked_sub(1)?;
        let lines: Vec<&str> = content.lines().take(line_idx + 16).collect();
        let line_str = *lines.get(line_idx)?;
        let chars: Vec<(usize, char)> = line_str.char_indices().collect();
        if chars.is_empty() {
            return None;
        }
        let cidx = (col as usize).saturating_sub(1).min(chars.len() - 1);
        let bpos = chars[cidx].0;
        let bytes = line_str.as_bytes();
        let pos = if bpos < bytes.len() && is_ident_byte(bytes[bpos]) {
            bpos
        } else if bpos > 0 && is_ident_byte(bytes[bpos - 1]) {
            bpos - 1
        } else {
            return None;
        };
        let mut id_start = pos;
        while id_start > 0 && is_ident_byte(bytes[id_start - 1]) {
            id_start -= 1;
        }
        let mut id_end = pos + 1;
        while id_end < bytes.len() && is_ident_byte(bytes[id_end]) {
            id_end += 1;
        }
        let ident = &line_str[id_start..id_end];
        if ident.as_bytes()[0].is_ascii_digit() {
            return None;
        }
        let mut p_start = id_start;
        while p_start >= 2 && &line_str[p_start - 2..p_start] == "::" {
            let seg_end = p_start - 2;
            let mut seg_start = seg_end;
            while seg_start > 0 && is_ident_byte(bytes[seg_start - 1]) {
                seg_start -= 1;
            }
            if seg_start == seg_end {
                break;
            }
            p_start = seg_start;
        }
        let prefix = if p_start < id_start { &line_str[p_start..id_start - 2] } else { "" };
        let has_trailing_colons = line_str[id_end..].starts_with("::");
        let use_outer = extract_use_tree_prefix(&lines, line_idx, p_start);
        if prefix.is_empty() && !has_trailing_colons && use_outer.is_none() {
            return None;
        }
        if is_in_string_or_comment(&lines, line_idx, id_start) {
            return None;
        }
        if prefix == "Self" {
            if let Some(def) = self.resolve_self_assoc_at(file, line, col, &lines, line_idx, ident) {
                return Some(def);
            }
        }
        if !prefix.is_empty() && !prefix.contains("::") && prefix != "Self" && prefix.as_bytes()[0].is_ascii_uppercase() {
            if let Some(def) = self.resolve_type_relative_at(file, line, col, &lines, line_idx, prefix, ident) {
                return Some(def);
            }
            if use_outer.is_none() && prefix.len() > 1 && is_ident_byte(prefix.as_bytes()[0]) {
                return None;
            }
        }
        let local_path = &line_str[p_start..id_end];
        let full_qual = match &use_outer {
            Some(outer) if !outer.is_empty() => format!("{outer}::{local_path}"),
            _ => local_path.to_string(),
        };
        if let Some(rest) = full_qual.strip_prefix("self::") {
            if let Some(inline_mod) = self.enclosing_inline_mod_path(file, line, col) {
                if let Some(def) = self.find_definition_by_path(&format!("{inline_mod}::{rest}")) {
                    return Some(def);
                }
            }
        }
        self.resolve_qualifier(file, &full_qual).or_else(|| self.find_definition_by_path(&full_qual)).or_else(|| {
            if !has_trailing_colons || !full_qual.contains("::") {
                return None;
            }
            let prefix_colon = format!("{full_qual}::");
            for (i, fid) in self.resolved_reader_file_ids(file) {
                let r = &self.readers[i];
                let refs = r.references();
                let lo = refs.partition_point(|re| re.span().file_id() < fid);
                let hi = refs.partition_point(|re| re.span().file_id() <= fid);
                for re in &refs[lo..hi] {
                    let tp = r.get_str(re.target_path_offset());
                    if tp.starts_with(&prefix_colon) {
                        let path_slice = &tp[..full_qual.len()];
                        let name_slice = path_slice.rsplit("::").next().unwrap_or(path_slice);
                        return Some(ResolvedDefinition {
                            name: name_slice,
                            path: path_slice,
                            kind: crate::format::DefKind::Mod,
                            signature: None,
                            file: "external",
                            start_line: 0,
                            start_col: 0,
                            end_line: 0,
                            end_col: 0,
                            is_macro_expansion: false,
                            is_reexport: false,
                            source_root: None,
                            unit_key: None,
                            reader_idx: Some(i),
                        });
                    }
                }
            }
            None
        })
    }
}
