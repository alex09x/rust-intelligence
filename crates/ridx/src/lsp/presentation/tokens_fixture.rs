/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Rust code inside fixture strings, as rust-analyzer highlights it: a raw string passed to a parameter whose name
//! starts with `ra_fixture` is Rust source (with `//-` meta lines and `$0` cursor markers), so its identifiers get
//! tokens like code does.

use super::tokens::RawToken;
use super::tokens_scan::scan_code_line;
use crate::lsp::editing::action_parse::mask_comments_and_strings;
use ridx_format::WorkspaceIndex;
use std::collections::HashMap;

/// Raw string literals of `text` (`r"…"`, `r#"…"#`, …): `(literal start, content start, content end)`.
fn raw_strings(text: &str, masked: &str) -> Vec<(usize, usize, usize)> {
    let (b, m) = (text.as_bytes(), masked.as_bytes());
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < b.len() {
        let starts = b[i] == b'r' && m[i] == b' ' && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_'));
        if !starts {
            i += 1;
            continue;
        }
        let hashes = b[i + 1..].iter().take_while(|&&c| c == b'#').count();
        if b.get(i + 1 + hashes) != Some(&b'"') {
            i += 1;
            continue;
        }
        let content = i + 2 + hashes;
        let close = format!("\"{}", "#".repeat(hashes));
        let Some(end) = text[content..].find(&close).map(|e| content + e) else { break };
        out.push((i, content, end));
        i = end + close.len();
    }
    out
}

/// The callee name position of the call whose argument list holds byte `at`: walk back to the unmatched `(` and take
/// the identifier before it.
fn callee_before(masked: &str, at: usize) -> Option<usize> {
    let b = masked.as_bytes();
    let mut depth = 0i32;
    let mut i = at;
    while i > 0 {
        i -= 1;
        match b[i] {
            b')' | b']' | b'}' => depth += 1,
            b'(' if depth == 0 => {
                let before = masked[..i].trim_end();
                let start = before.trim_end_matches(|c: char| c.is_ascii_alphanumeric() || c == '_').len();
                return (start < before.len()).then_some(start);
            }
            b'(' | b'[' | b'{' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            b';' if depth == 0 => return None,
            _ => {}
        }
    }
    None
}

/// Whether the function the index resolves at `(line, col)` of `file` takes a `ra_fixture…` parameter.
fn takes_fixture(ws: &WorkspaceIndex, file: &str, line: u32, col: u32, sources: &mut HashMap<std::path::PathBuf, Option<String>>) -> bool {
    let Some(refs) = ws.find_direct_references_at(file, line, col) else { return false };
    let path = refs.target_resolved_path();
    let Some(src) = sources.entry(path.clone()).or_insert_with(|| std::fs::read_to_string(&path).ok()).as_deref() else { return false };
    let start: usize = src.split_inclusive('\n').take(refs.target_line.saturating_sub(1) as usize).map(str::len).sum();
    let header = &src[start.min(src.len())..];
    let header = &header[..header.find(['{', ';']).unwrap_or(header.len())];
    header.contains("ra_fixture")
}

/// Tokens for the Rust code of every fixture string in `text`.
pub(super) fn scan_fixtures(file: &str, text: &str, ws: &WorkspaceIndex, tokens: &mut Vec<RawToken>) {
    if !text.contains("r#\"") && !text.contains("r\"") {
        return;
    }
    let masked = mask_comments_and_strings(text);
    let mut line_starts = vec![0usize];
    line_starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    let mut verdicts: HashMap<String, bool> = HashMap::new();
    let mut sources = HashMap::new();
    for (lit, content, end) in raw_strings(text, &masked) {
        let Some(callee) = callee_before(&masked, lit) else { continue };
        let name_end = callee + masked[callee..].find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(0);
        let line = line_starts.partition_point(|&s| s <= callee) - 1;
        let col = text[line_starts[line]..callee].chars().count() as u32 + 1;
        let fixture = *verdicts.entry(text[callee..name_end].to_string()).or_insert_with(|| takes_fixture(ws, file, line as u32 + 1, col, &mut sources));
        if !fixture {
            continue;
        }
        let mut in_block_comment = false;
        let first = line_starts.partition_point(|&s| s <= content) - 1;
        let last = line_starts.partition_point(|&s| s <= end) - 1;
        for l in first..=last {
            let ls = line_starts[l];
            let le = line_starts.get(l + 1).map_or(text.len(), |&n| n - 1).min(if l == last { end } else { usize::MAX });
            let from = if l == first { content - ls } else { 0 };
            let line_text = &text[ls..le.max(ls + from)];
            if line_text[from..].trim_start().starts_with("//-") {
                continue;
            }
            scan_code_line(line_text, line_text, from, l as u32, &mut in_block_comment, true, tokens);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{callee_before, raw_strings};
    use crate::lsp::editing::action_parse::mask_comments_and_strings;

    #[test]
    fn test_finds_raw_string_arguments_and_their_callee() {
        let text = "fn t() {\n    check(\n        r#\"\nfn foo() {}\n\"#,\n        expect![[\"x\"]],\n    );\n    let s = r\"plain\";\n}\n";
        let masked = mask_comments_and_strings(text);
        let lits = raw_strings(text, &masked);
        assert_eq!(lits.len(), 2, "{lits:?}");
        let (lit, content, end) = lits[0];
        assert_eq!(&text[content..end], "\nfn foo() {}\n");
        assert_eq!(callee_before(&masked, lit).map(|c| &text[c..c + 5]), Some("check"));
        assert_eq!(callee_before(&masked, lits[1].0), None, "a let binding is no call argument");
    }
}
