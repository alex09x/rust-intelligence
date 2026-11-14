/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! References inside items a `#[cfg(…)]` switches off in every indexed build (`#[cfg(windows)] fn test_osstring()`
//! on Linux). The compiler drops them, so the index holds no reference there; rust-analyzer still lists the
//! occurrences it resolves. An occurrence of the name counts when the definition at it is the target.

use ridx_format::{LocationReferences, WorkspaceIndex};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// A switched-off item: its file, its first 1-based line and its original and masked lines.
pub(super) struct InactiveItem {
    path: PathBuf,
    first_line: u32,
    lines: Vec<String>,
    masked: Vec<String>,
}

type Items = Arc<Vec<InactiveItem>>;
static INACTIVE: OnceLock<Mutex<HashMap<super::references_gated::WsKey, Items>>> = OnceLock::new();

pub(crate) fn clear_inactive_cache() {
    if let Some(mut c) = INACTIVE.get().and_then(|m| m.lock().ok()) {
        c.clear();
    }
}

/// Byte index just past the bracket group opening at `open` (`[`, `(` or `{`), or `None` when unbalanced.
fn group_end(b: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, &c) in b.iter().enumerate().skip(open) {
        match c {
            b'[' | b'(' | b'{' => depth += 1,
            b']' | b')' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// Whether the item text at `at` is an import (`use …;`, `pub(crate) use …;`).
fn is_use_item(rest: &str) -> bool {
    let rest = rest.strip_prefix("pub").map_or(rest, |r| {
        let r = r.trim_start();
        if r.starts_with('(') {
            r.find(')').map_or(r, |p| &r[p + 1..])
        } else {
            r
        }
    });
    rest.trim_start().strip_prefix("use").is_some_and(|r| r.starts_with(|c: char| c.is_whitespace() || c == ':' || c == '{'))
}

/// Whether `rest` starts an item (`fn`, `pub struct`, `impl`, …), not a statement or expression a body gates.
fn is_item_head(rest: &str) -> bool {
    const KEYWORDS: [&str; 16] = ["pub", "fn", "struct", "enum", "union", "impl", "mod", "trait", "const", "static", "type", "unsafe", "async", "extern", "macro_rules", "use"];
    let word: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '!').collect();
    KEYWORDS.contains(&word.trim_end_matches('!'))
}

/// End byte of the item that starts at `at`, after its outer attributes: its `;` or its closing brace at depth 0.
/// An import is left out: its tree is resolved only in the builds that compile it.
fn item_end(b: &[u8], mut at: usize) -> Option<usize> {
    loop {
        while at < b.len() && b[at].is_ascii_whitespace() {
            at += 1;
        }
        if b.get(at) == Some(&b'#') && b.get(at + 1) == Some(&b'[') {
            at = group_end(b, at + 1)?;
        } else {
            break;
        }
    }
    let head = String::from_utf8_lossy(&b[at..(at + 64).min(b.len())]);
    if is_use_item(&head) || !is_item_head(&head) {
        return None;
    }
    let mut i = at;
    while i < b.len() {
        match b[i] {
            b';' => return Some(i + 1),
            b'{' => return group_end(b, i),
            b'(' | b'[' => i = group_end(b, i)?,
            b'}' => return None,
            _ => i += 1,
        }
    }
    None
}

/// The 0-based line ranges of the items `#[cfg(…)]` gates in `masked`, attributes included.
pub(super) fn cfg_gated_items(masked: &str) -> Vec<(usize, usize)> {
    let b = masked.as_bytes();
    let line_of = |off: usize| masked[..off].bytes().filter(|&c| c == b'\n').count();
    // Nested gates are listed too: a switched-off `fn` inside a `mod tests` some build compiles.
    let mut out = Vec::new();
    for (at, _) in masked.match_indices("#[cfg(") {
        let line_start = masked[..at].rfind('\n').map_or(0, |p| p + 1);
        if !masked[line_start..at].trim().is_empty() {
            continue;
        }
        let Some(attr_end) = group_end(b, at + 1) else { continue };
        let Some(end) = item_end(b, attr_end) else { continue };
        // Inside a macro template (`$name`) the tokens are not code yet.
        if masked[at..end].contains('$') {
            continue;
        }
        out.push((line_of(at), line_of(end.saturating_sub(1))));
    }
    out
}

/// The workspace's cfg-gated items in which no indexed build holds a reference.
fn inactive_items(ws: &WorkspaceIndex) -> Items {
    let key = super::references_gated::ws_cache_key(ws);
    let cache = INACTIVE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(v) = cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
        return v;
    }
    let mut ref_lines: HashMap<PathBuf, HashSet<u32>> = HashMap::new();
    for (i, reader) in ws.readers().iter().enumerate() {
        if ws.is_reader_external(i) {
            continue;
        }
        let Some(root) = ws.source_root_at(i) else { continue };
        let mut per_file: Vec<Vec<u32>> = vec![Vec::new(); reader.files_count() as usize];
        for r in reader.references() {
            if let Some(f) = per_file.get_mut(r.span.file_id as usize) {
                f.push(r.span.start_line());
            }
        }
        for (fid, lines) in per_file.into_iter().enumerate() {
            if lines.is_empty() {
                continue;
            }
            let path = ridx_format::join_resolving_overlap(root, std::path::Path::new(reader.get_file(fid as u32)));
            ref_lines.entry(path).or_default().extend(lines);
        }
    }
    let mut out = Vec::new();
    for (path, lines) in &ref_lines {
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        if !text.contains("#[cfg(") {
            continue;
        }
        let masked = crate::lsp::editing::action_parse::mask_comments_and_strings(&text);
        let (orig, mask): (Vec<&str>, Vec<&str>) = (text.lines().collect(), masked.lines().collect());
        for (s, e) in cfg_gated_items(&masked) {
            if (s..=e).any(|l| lines.contains(&(l as u32 + 1))) || e >= orig.len() {
                continue;
            }
            let lines = orig[s..=e].iter().map(|l| l.to_string()).collect();
            let masked = mask[s..=e].iter().map(|l| l.to_string()).collect();
            out.push(InactiveItem { path: path.clone(), first_line: s as u32 + 1, lines, masked });
        }
    }
    let v = Arc::new(out);
    if let Ok(mut c) = cache.lock() {
        super::references_gated::insert_bounded(&mut c, key, Arc::clone(&v));
    }
    v
}

/// References to the target of `refs` in switched-off items: `(file, 0-based line, utf16 start, utf16 end)`.
pub(super) fn inactive_item_refs(ws: &WorkspaceIndex, refs: &LocationReferences<'_>) -> Vec<(PathBuf, u32, u32, u32)> {
    let name = refs.target_symbol;
    // An associated item (`Command::new`) or a field resolves through types the build left out there; a local
    // belongs to one body.
    use ridx_format::DefKind as K;
    let assoc = matches!(refs.target_kind, K::AssocFn | K::AssocConst | K::AssocTy | K::Field | K::Local | K::TyParam | K::LifetimeParam);
    if assoc || name.is_empty() || refs.target_file.is_empty() || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
        return Vec::new();
    }
    let target = (refs.target_resolved_path(), refs.target_line);
    let is_id = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut out = Vec::new();
    for item in inactive_items(ws).iter() {
        let Some(file) = item.path.to_str() else { continue };
        for (k, scan) in item.masked.iter().enumerate() {
            let b = scan.as_bytes();
            for (at, _) in scan.match_indices(name) {
                let end = at + name.len();
                if (at > 0 && is_id(b[at - 1])) || (end < b.len() && is_id(b[end])) {
                    continue;
                }
                let line = item.first_line + k as u32;
                let Some(orig) = item.lines.get(k).and_then(|l| l.get(..at)) else { continue };
                let char_col = orig.chars().count() as u32 + 1;
                if ws.find_definition_at(file, line, char_col).is_some_and(|d| (d.resolved_path(), d.start_line) == target) {
                    let col = orig.encode_utf16().count() as u32;
                    out.push((item.path.clone(), line - 1, col, col + name.encode_utf16().count() as u32));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::cfg_gated_items;

    #[test]
    fn finds_the_lines_a_cfg_gates() {
        let src = "use a::b;\n#[cfg(windows)]\n#[test]\nfn f() {\n    g();\n}\nfn h() {}\n#[cfg(feature = \"x\")] use c::d;\n#[cfg(test)]\nmod t {\n    #[cfg(all(unix, not(test)))]\n    struct S(u8);\n}\n";
        assert_eq!(cfg_gated_items(src), vec![(1, 5), (8, 12), (10, 11)], "the gated `use` is left out");
        assert!(cfg_gated_items("#[cfg(unix)]\npub(crate) use a::{b, c};\n#[cfg(x)]\nfn f() { $name::g() }\n").is_empty());
        assert!(cfg_gated_items("fn f() {\n    #[cfg(x)]\n    if let Some(c) = g() { h(c) }\n    #[cfg(y)]\n    let z = 1;\n}\n").is_empty(), "statements are not items");
        assert!(cfg_gated_items("let x = cfg!(windows); #[cfg_attr(test, derive(Debug))] struct T;").is_empty());
    }
}
