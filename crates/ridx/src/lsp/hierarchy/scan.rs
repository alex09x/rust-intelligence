/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Source text scanning helpers for call and type hierarchies.

use ridx_format::{DefKind, WorkspaceIndex, WorkspaceSymbol};
use std::path::Path;

pub(crate) fn is_call_site(file: &str, span_end_line: u32, span_end_col: u32) -> bool {
    let content = std::fs::read_to_string(file).ok();
    let Some(ref text) = content else {
        return false;
    };
    let lines: Vec<&str> = text.lines().collect();
    let l_idx = span_end_line.saturating_sub(1) as usize;
    if l_idx >= lines.len() {
        return false;
    }
    let line_chars: Vec<char> = lines[l_idx].chars().collect();
    let char_offset = span_end_col.saturating_sub(1) as usize;

    let check_from = |offset: usize| -> bool {
        let mut rest: String = line_chars.iter().skip(offset).collect();
        if rest.trim().is_empty() {
            for next_l in lines.iter().skip(l_idx + 1).take(3) {
                rest.push('\n');
                rest.push_str(next_l);
                if !next_l.trim().is_empty() {
                    break;
                }
            }
        }
        let trimmed = rest.trim_start();
        if trimmed.starts_with('(') {
            return true;
        }
        if trimmed.starts_with("::<") {
            if let Some(pos) = trimmed.find('(') {
                let between = &trimmed[3..pos];
                if between.contains('>') {
                    return true;
                }
            }
        }
        false
    };

    if check_from(char_offset) {
        return true;
    }
    if char_offset < line_chars.len()
        && (line_chars[char_offset].is_alphanumeric() || line_chars[char_offset] == '_')
        && check_from(char_offset + 1)
    {
        return true;
    }
    false
}

pub(crate) fn find_function_body_end_line(lines: &[&str], start_line_1based: u32) -> u32 {
    let start_idx = start_line_1based.saturating_sub(1) as usize;
    let mut depth = 0i32;
    let mut seen_open = false;
    for (idx, line) in lines.iter().enumerate().skip(start_idx) {
        for ch in line.chars() {
            if !seen_open && ch == ';' {
                return (idx + 1) as u32;
            }
            if ch == '{' {
                depth += 1;
                seen_open = true;
            } else if ch == '}' {
                depth -= 1;
                if seen_open && depth <= 0 {
                    return (idx + 1) as u32;
                }
            }
        }
    }
    lines.len() as u32
}

pub(crate) fn find_enclosing_function<'a>(
    ws: &'a WorkspaceIndex,
    file: &str,
    call_line: u32,
    file_lines: &[&str],
) -> Option<WorkspaceSymbol<'a>> {
    let file_path = Path::new(file);
    let mut candidates: Vec<_> = ws
        .search_symbols("")
        .into_iter()
        .filter(|d| {
            matches!(d.kind, DefKind::Fn | DefKind::AssocFn)
                && (d.resolved_path() == file_path || d.file == file)
                && d.span.start_line() <= call_line
        })
        .collect();

    candidates.sort_by_key(|d| std::cmp::Reverse(d.span.start_line()));

    for cand in candidates {
        let body_end = find_function_body_end_line(file_lines, cand.span.start_line());
        if call_line <= body_end {
            return Some(cand);
        }
    }
    None
}

pub(crate) fn strip_leading_generics(s: &str) -> &str {
    let trimmed = s.trim_start();
    if !trimmed.starts_with('<') {
        return trimmed;
    }
    let mut depth = 0;
    for (i, c) in trimmed.char_indices() {
        if c == '<' {
            depth += 1;
        } else if c == '>' {
            depth -= 1;
            if depth == 0 {
                return trimmed[i + 1..].trim_start();
            }
        }
    }
    trimmed
}

pub(crate) fn parse_impl_line(line: &str) -> (Option<String>, Option<String>) {
    let s = line.trim();
    let after_impl = match s.strip_prefix("impl") {
        Some(rest) => rest.trim_start(),
        None => return (None, None),
    };
    let without_generics = strip_leading_generics(after_impl);
    if let Some((trait_part, target_part)) = without_generics.split_once(" for ") {
        let trait_name = trait_part
            .split_whitespace()
            .last()
            .and_then(|p| p.split('<').next())
            .map(|p| p.to_string());
        let target_name = target_part
            .split_whitespace()
            .next()
            .and_then(|p| p.split('<').next())
            .map(|p| p.trim_matches('{').to_string());
        (trait_name, target_name)
    } else {
        (None, None)
    }
}

pub(crate) fn get_impl_line(lines: &[&str], line_idx: usize) -> Option<String> {
    if line_idx >= lines.len() {
        return None;
    }
    if lines[line_idx].trim_start().starts_with("impl") {
        let mut full = lines[line_idx].to_string();
        for next_line in lines.iter().skip(line_idx + 1).take(4) {
            if full.contains('{') || full.contains(';') {
                break;
            }
            full.push(' ');
            full.push_str(next_line.trim());
        }
        return Some(full);
    }
    for back in 1..=4.min(line_idx) {
        let prev_idx = line_idx - back;
        if lines[prev_idx].trim_start().starts_with("impl") {
            let mut full = lines[prev_idx].to_string();
            for l in lines.iter().skip(prev_idx + 1).take(5) {
                full.push(' ');
                full.push_str(l.trim());
                if full.contains('{') || full.contains(';') {
                    break;
                }
            }
            return Some(full);
        }
    }
    None
}
