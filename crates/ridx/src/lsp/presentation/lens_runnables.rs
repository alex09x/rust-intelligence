/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! rust-analyzer's runnable lenses (`▶︎ Run …` / `⚙︎ Debug`) for clients that run `rust-analyzer.runSingle` and
//! `rust-analyzer.debugSingle` themselves: `fn main` of a binary or example, `#[test]` functions, inline modules
//! holding tests, `#[bench]` functions and library items whose docs carry a Rust code block.

use super::lens_runnable_hints::RunnableHints;
use super::lens_targets::{file_target, FileTarget, TargetKind};
use crate::lsp::editing::action_parse::mask_comments_and_strings;
use crate::lsp::protocol::path_to_uri;
use serde_json::{json, Value};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq)]
enum RunKind {
    Main,
    Test,
    TestMod,
    Bench,
    Doctest,
}

struct Runnable {
    kind: RunKind,
    path: String,
    line0: u32,
    name_col16: u32,
    name_len16: u32,
    /// `(start line, start col, end line, end col)` when the lens spans the item rather than its name: a doctest
    /// lens covers the item from its first doc or attribute line to its end, as rust-analyzer places it.
    span: Option<(u32, u32, u32, u32)>,
}

/// Span of a documented item: from its first doc or attribute line (`attr_lines` lines above `line`) to its end.
fn doctest_span(lines: &[&str], masked_lines: &[&str], line: usize, attr_lines: usize) -> (u32, u32, u32, u32) {
    let first = line - attr_lines;
    let indent = lines[first].len() - lines[first].trim_start().len();
    let (end_line, end_col) = item_end(lines, masked_lines, line);
    (first as u32, lines[first][..indent].encode_utf16().count() as u32, end_line, end_col)
}

/// End of the item whose header is on `line`: just past its matching `}` or its `;`, in UTF-16 columns.
fn item_end(lines: &[&str], masked_lines: &[&str], line: usize) -> (u32, u32) {
    let mut depth = 0i32;
    for (l, ml) in masked_lines.iter().enumerate().skip(line) {
        for (b, ch) in ml.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return (l as u32, lines[l][..=b].encode_utf16().count() as u32);
                    }
                }
                ';' if depth == 0 => return (l as u32, lines[l][..=b].encode_utf16().count() as u32),
                _ => {}
            }
        }
    }
    let last = lines.len().saturating_sub(1);
    (last as u32, lines.get(last).map_or(0, |l| l.encode_utf16().count() as u32))
}

fn is_test_attr(attr: &str) -> bool {
    let inner = attr.trim().trim_start_matches("#[").trim_end_matches(']');
    let path = inner.split('(').next().unwrap_or(inner).trim();
    path == "test" || path.ends_with("::test")
}

fn doc_has_rust_block(doc_lines: &[&str]) -> bool {
    doc_lines.iter().any(|l| {
        let t = l.trim_start().trim_start_matches("///").trim_start_matches("//!").trim();
        t.strip_prefix("```").is_some_and(|lang| {
            lang.split(',')
                .all(|tag| matches!(tag.trim(), "" | "rust" | "should_panic" | "no_run" | "compile_fail" | "edition2015" | "edition2018" | "edition2021" | "edition2024"))
        })
    })
}

fn ident_after<'a>(masked_line: &'a str, keyword: &str) -> Option<(usize, &'a str)> {
    let mut search = 0;
    while let Some(found) = masked_line[search..].find(keyword) {
        let at = search + found;
        let before_ok = at == 0 || !masked_line.as_bytes()[at - 1].is_ascii_alphanumeric() && masked_line.as_bytes()[at - 1] != b'_';
        let rest = &masked_line[at + keyword.len()..];
        if before_ok && rest.starts_with(|c: char| c.is_whitespace()) {
            let trimmed = rest.trim_start();
            let start = masked_line.len() - trimmed.len();
            let len = trimmed.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(trimmed.len());
            if len > 0 {
                return Some((start, &masked_line[start..start + len]));
            }
        }
        search = at + keyword.len();
    }
    None
}

/// An inline module open at the current line.
struct ModFrame {
    name: String,
    /// Brace depth inside the module body; the module closes when the depth drops below it.
    depth: i32,
    line0: u32,
    name_col16: u32,
    name_len16: u32,
    has_tests: bool,
}

/// Self type of an `impl` header line (`impl<T> Trait for Ty<T> {` is `Ty`), the prefix of its methods' doctest paths.
fn impl_self_type(masked_line: &str) -> String {
    let header = masked_line.split('{').next().unwrap_or("").trim();
    let header = header.strip_prefix("unsafe").map_or(header, str::trim_start);
    let mut rest = header.strip_prefix("impl").unwrap_or(header).trim_start();
    if rest.starts_with('<') {
        let mut depth = 0;
        let end = rest.char_indices().find_map(|(i, c)| {
            depth += match c {
                '<' => 1,
                '>' => -1,
                _ => 0,
            };
            (depth == 0).then_some(i + 1)
        });
        rest = rest[end.unwrap_or(rest.len())..].trim_start();
    }
    let self_ty = rest.rsplit_once(" for ").map_or(rest, |(_, ty)| ty).trim();
    let path = self_ty.split(|c: char| c == '<' || c.is_whitespace()).next().unwrap_or("");
    path.rsplit("::").next().unwrap_or(path).trim_start_matches('&').to_string()
}

fn collect_runnables(text: &str, target: &FileTarget, hints: &RunnableHints) -> Vec<Runnable> {
    let masked = mask_comments_and_strings(text);
    let lines: Vec<&str> = text.lines().collect();
    let masked_lines: Vec<&str> = masked.lines().collect();
    let mut out = Vec::new();
    let mut mods: Vec<ModFrame> = Vec::new();
    let mut depth = 0i32;
    // Open `impl` blocks: brace depth inside the block and its self type.
    let mut impls: Vec<(i32, String)> = Vec::new();
    let path_of = |mods: &[ModFrame], name: &str| {
        let mut segs: Vec<&str> = target.module_prefix.iter().map(String::as_str).collect();
        segs.extend(mods.iter().map(|m| m.name.as_str()));
        segs.push(name);
        segs.join("::")
    };
    for (i, ml) in masked_lines.iter().enumerate() {
        let attrs: Vec<&str> = lines[..i].iter().rev().map(|l| l.trim()).take_while(|l| l.starts_with("#[") || l.starts_with("///") || l.starts_with("//")).collect();
        let col16 = |byte: usize| lines[i][..byte].encode_utf16().count() as u32;
        let in_impl = !impls.is_empty();
        if let Some((at, name)) = ident_after(ml, "fn").filter(|_| target.layout_known) {
            let len16 = name.encode_utf16().count() as u32;
            let kind = if attrs.iter().any(|a| a.starts_with("#[") && is_test_attr(a)) {
                Some(RunKind::Test)
            } else if attrs.iter().any(|a| a.starts_with("#[bench")) {
                Some(RunKind::Bench)
            } else if name == "main" && depth == 0 && target.is_root && matches!(target.kind, TargetKind::Bin(_) | TargetKind::Example(_)) {
                Some(RunKind::Main)
            } else if target.kind == TargetKind::Lib && doc_has_rust_block(&attrs) {
                Some(RunKind::Doctest)
            } else {
                None
            };
            if let Some(kind) = kind {
                if kind == RunKind::Test {
                    mods.iter_mut().for_each(|m| m.has_tests = true);
                }
                let span = (kind == RunKind::Doctest).then(|| doctest_span(&lines, &masked_lines, i, attrs.len()));
                // A method's doctest is named after its impl's self type: `doctest Ty::method`.
                let item = match impls.last() {
                    Some((_, ty)) if kind == RunKind::Doctest && !ty.is_empty() => format!("{ty}::{name}"),
                    _ => name.to_string(),
                };
                out.push(Runnable { kind, path: path_of(&mods, &item), line0: i as u32, name_col16: col16(at), name_len16: len16, span });
            }
        } else if target.layout_known && !in_impl && target.kind == TargetKind::Lib && doc_has_rust_block(&attrs) {
            for kw in ["struct", "enum", "trait", "union", "type", "const", "static", "macro_rules!"] {
                if let Some((at, name)) = ident_after(ml, kw.trim_end_matches('!')) {
                    let path = path_of(&mods, name);
                    let span = Some(doctest_span(&lines, &masked_lines, i, attrs.len()));
                    out.push(Runnable { kind: RunKind::Doctest, path, line0: i as u32, name_col16: col16(at), name_len16: name.encode_utf16().count() as u32, span });
                    break;
                }
            }
        }
        // Tests and `main` the index knows on this line but the text does not spell, as macros generate them.
        let line0 = i as u32;
        for site in hints.tests.get(&line0).into_iter().flatten() {
            if out.iter().any(|r| r.line0 == line0 && r.name_col16 == site.col16) {
                continue;
            }
            mods.iter_mut().for_each(|m| m.has_tests = true);
            out.push(Runnable { kind: RunKind::Test, path: path_of(&mods, &site.name), line0, name_col16: site.col16, name_len16: site.len16, span: None });
        }
        if let Some(site) = hints.mains.get(&line0) {
            let runnable_target = matches!(target.kind, TargetKind::Bin(_) | TargetKind::Example(_) | TargetKind::Bench(_));
            if depth == 0 && target.is_root && runnable_target && !out.iter().any(|r| r.kind == RunKind::Main) {
                out.push(Runnable { kind: RunKind::Main, path: site.name.clone(), line0, name_col16: site.col16, name_len16: site.len16, span: None });
            }
        }
        let opens_mod = ident_after(ml, "mod").filter(|_| ml.contains('{'));
        let opens_impl = ml.trim_start().starts_with("impl") || ml.trim_start().starts_with("unsafe impl");
        for ch in ml.chars() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    while let Some(m) = mods.pop_if(|m| m.depth > depth) {
                        if m.has_tests {
                            let path = path_of(&mods, &m.name);
                            out.push(Runnable { kind: RunKind::TestMod, path, line0: m.line0, name_col16: m.name_col16, name_len16: m.name_len16, span: None });
                        }
                    }
                    while impls.pop_if(|(d, _)| *d > depth).is_some() {}
                }
                _ => {}
            }
        }
        if let Some((at, name)) = opens_mod {
            mods.push(ModFrame { name: name.to_string(), depth, line0: i as u32, name_col16: col16(at), name_len16: name.encode_utf16().count() as u32, has_tests: false });
        } else if opens_impl && ml.contains('{') {
            impls.push((depth, impl_self_type(ml)));
        }
    }
    out.sort_by_key(|r| (r.line0, r.name_col16));
    out
}

fn runnable_arg(r: &Runnable, target: &FileTarget, uri: &str) -> Value {
    let target_flag: Vec<String> = match &target.kind {
        TargetKind::Lib => vec!["--lib".into()],
        TargetKind::Bin(n) => vec!["--bin".into(), n.clone()],
        TargetKind::Test(n) => vec!["--test".into(), n.clone()],
        TargetKind::Bench(n) => vec!["--bench".into(), n.clone()],
        TargetKind::Example(n) => vec!["--example".into(), n.clone()],
    };
    let pkg = target.package.clone();
    let (label, cargo_args, exec_args, cwd): (String, Vec<String>, Vec<String>, &Path) = match r.kind {
        RunKind::Main => {
            let name = match &target.kind {
                TargetKind::Bin(n) | TargetKind::Example(n) | TargetKind::Bench(n) => n.clone(),
                _ => pkg.clone(),
            };
            // A bench target with its own `main` (`harness = false`, as criterion's) runs under `cargo bench`.
            let sub = if matches!(target.kind, TargetKind::Bench(_)) { "bench" } else { "run" };
            (format!("run {name}"), [vec![sub.into(), "--package".into(), pkg], target_flag].concat(), vec![], &target.workspace_root)
        }
        RunKind::Test => (
            format!("test {}", r.path),
            [vec!["test".into(), "--package".into(), pkg], target_flag].concat(),
            vec![r.path.clone(), "--exact".into(), "--nocapture".into()],
            &target.pkg_dir,
        ),
        RunKind::TestMod => {
            (format!("test-mod {}", r.path), [vec!["test".into(), "--package".into(), pkg], target_flag].concat(), vec![r.path.clone(), "--nocapture".into()], &target.pkg_dir)
        }
        RunKind::Bench => (
            format!("bench {}", r.path),
            [vec!["bench".into(), "--package".into(), pkg], target_flag].concat(),
            vec![r.path.clone(), "--exact".into(), "--nocapture".into()],
            &target.pkg_dir,
        ),
        RunKind::Doctest => {
            (format!("doctest {}", r.path), vec!["test".into(), "--doc".into(), "--package".into(), pkg], vec![r.path.clone(), "--nocapture".into()], &target.pkg_dir)
        }
    };
    let range = match r.span {
        Some((sl, sc, el, ec)) => json!({ "start": { "line": sl, "character": sc }, "end": { "line": el, "character": ec } }),
        None => json!({
            "start": { "line": r.line0, "character": r.name_col16 },
            "end": { "line": r.line0, "character": r.name_col16 + r.name_len16 }
        }),
    };
    json!({
        "label": label,
        "location": { "targetUri": uri, "targetRange": range, "targetSelectionRange": range },
        "kind": "cargo",
        "args": {
            "environment": {},
            "cwd": cwd.to_string_lossy(),
            "overrideCargo": null,
            "workspaceRoot": target.workspace_root.to_string_lossy(),
            "cargoArgs": cargo_args,
            "executableArgs": exec_args
        }
    })
}

/// Runnable lenses of `file` in rust-analyzer's shape, sorted by position, a `Run` lens before its `Debug` lens.
pub fn runnable_lenses(file: &str, text: &str, run: bool, debug: bool, hints: &RunnableHints) -> Vec<Value> {
    if !run && !debug {
        return Vec::new();
    }
    let Some(target) = file_target(Path::new(file)) else {
        return Vec::new();
    };
    let uri = path_to_uri(file);
    let mut out = Vec::new();
    let mut runnables = collect_runnables(text, &target, hints);
    // A file holding tests is itself a test module: rust-analyzer runs it from a lens spanning the whole file.
    let file_module = runnables.iter().any(|r| r.kind == RunKind::Test).then(|| {
        let line_count = text.lines().count() as u32;
        let (end_line, end_col) = match text.lines().last() {
            Some(last) if !text.ends_with('\n') => (line_count - 1, last.encode_utf16().count() as u32),
            _ => (line_count, 0),
        };
        let path = target.module_prefix.join("::");
        Runnable { kind: RunKind::TestMod, path, line0: 0, name_col16: 0, name_len16: 0, span: Some((0, 0, end_line, end_col)) }
    });
    runnables.splice(0..0, file_module);
    for r in runnables {
        let title = match r.kind {
            RunKind::Main => "▶︎ Run ",
            RunKind::Test => "▶︎ Run Test",
            RunKind::TestMod => "▶︎ Run Tests",
            RunKind::Bench => "▶︎ Run Bench",
            RunKind::Doctest => "▶︎ Run Doctest",
        };
        let arg = runnable_arg(&r, &target, &uri);
        let range = arg["location"]["targetSelectionRange"].clone();
        if run {
            out.push(json!({ "range": range, "command": { "title": title, "command": "rust-analyzer.runSingle", "arguments": [arg] } }));
        }
        if debug {
            out.push(json!({ "range": range, "command": { "title": "⚙︎ Debug", "command": "rust-analyzer.debugSingle", "arguments": [arg] } }));
        }
    }
    out
}

#[cfg(test)]
#[path = "lens_runnables_tests.rs"]
mod tests;
