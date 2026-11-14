/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Which modules rust-analyzer runs as a whole (`▶︎ Run Tests`): a module that declares a test function itself, or
//! more than one submodule that is such a module in turn (`has_test_function_or_multiple_test_submodules`). A file
//! whose only tests sit in one `mod tests` gets the lens on that module, not on the file.

use crate::lsp::editing::action_parse::mask_comments_and_strings;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The test modules of one file: whether the file's own module is one, and the 0-based lines of the inline
/// `mod name {` that are.
pub(super) struct TestModules {
    pub file: bool,
    pub inline: HashSet<u32>,
    /// The `mod name;` declarations whose file is a test module, run from a lens on the name: 0-based line, byte
    /// offset of the name on the line, and the module's path from the file's module (`a::name`).
    pub outline: Vec<(u32, usize, String)>,
}

struct Frame {
    line0: u32,
    depth: i32,
    name: String,
    direct: bool,
    children: usize,
}

pub(super) fn is_test_attr(attr: &str) -> bool {
    let inner = attr.trim().trim_start_matches("#[").trim_end_matches(']');
    let path = inner.split('(').next().unwrap_or(inner).trim();
    path == "test" || path.ends_with("::test")
}

/// The name of an item `keyword name` on a masked line, with its byte offset.
pub(super) fn ident_after<'a>(masked_line: &'a str, keyword: &str) -> Option<(usize, &'a str)> {
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

/// `mod name;` (any visibility) on a trimmed masked line.
fn declared_file_module(t: &str) -> Option<&str> {
    let t = match t.strip_prefix("pub") {
        Some(rest) if rest.starts_with('(') => rest[rest.find(')')? + 1..].trim_start(),
        Some(rest) if rest.starts_with(' ') => rest.trim_start(),
        _ => t,
    };
    let name = t.strip_prefix("mod ")?.strip_suffix(';')?.trim();
    (!name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')).then_some(name)
}

/// The directory a file's `mod name;` children live in: beside a `mod.rs`/`lib.rs`/`main.rs`, else in a directory
/// named after the file.
fn module_dir(file: &Path) -> PathBuf {
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let parent = file.parent().unwrap_or(Path::new(""));
    if matches!(stem, "mod" | "lib" | "main") {
        parent.to_path_buf()
    } else {
        parent.join(stem)
    }
}

fn child_file_qualifies(dir: &Path, name: &str, budget: u32) -> bool {
    for cand in [dir.join(format!("{name}.rs")), dir.join(name).join("mod.rs")] {
        if let Ok(text) = std::fs::read_to_string(&cand) {
            return test_modules(&cand, &text, &HashSet::new(), budget, true).file;
        }
    }
    false
}

/// The test modules of `file` holding `text`. `extra_tests` are 0-based lines of tests the index knows a macro
/// generated there; `budget` bounds how many levels of `mod name;` files are read; without `read_attrs` only those
/// index tests count, not the test attributes the text spells (a file outside Cargo's layout).
pub(super) fn test_modules(file: &Path, text: &str, extra_tests: &HashSet<u32>, budget: u32, read_attrs: bool) -> TestModules {
    let masked = mask_comments_and_strings(text);
    let lines: Vec<&str> = text.lines().collect();
    let base = module_dir(file);
    let (mut file_direct, mut file_children) = (false, 0usize);
    let mut frames: Vec<Frame> = Vec::new();
    let mut inline = HashSet::new();
    let mut outline = Vec::new();
    // Open fn, impl and trait bodies: what they hold is no declaration of the module.
    let mut bodies: Vec<i32> = Vec::new();
    let mut depth = 0i32;
    for (i, ml) in masked.lines().enumerate() {
        let t = ml.trim_start();
        let is_fn = ident_after(ml, "fn").is_some();
        let test_attr = || lines[..i].iter().rev().map(|l| l.trim()).take_while(|l| l.starts_with("#[") || l.starts_with("//")).any(|a| a.starts_with("#[") && is_test_attr(a));
        if bodies.is_empty() && ((read_attrs && is_fn && test_attr()) || extra_tests.contains(&(i as u32))) {
            match frames.last_mut() {
                Some(f) => f.direct = true,
                None => file_direct = true,
            }
        }
        if let Some(name) = declared_file_module(t).filter(|_| bodies.is_empty() && budget > 0) {
            let dir = frames.iter().fold(base.clone(), |d, f| d.join(&f.name));
            if child_file_qualifies(&dir, name, budget - 1) {
                match frames.last_mut() {
                    Some(f) => f.children += 1,
                    None => file_children += 1,
                }
                if let Some((at, _)) = ident_after(ml, "mod") {
                    let path = frames.iter().map(|f| f.name.as_str()).chain([name]).collect::<Vec<_>>().join("::");
                    outline.push((i as u32, at, path));
                }
            }
        }
        let opens_mod = ident_after(ml, "mod").filter(|_| ml.contains('{') && bodies.is_empty());
        let opens_body = opens_mod.is_none() && ml.contains('{') && (is_fn || t.starts_with("impl") || t.starts_with("unsafe impl") || ident_after(ml, "trait").is_some());
        // A line that closes what it opens (`fn t() {}`, `mod empty {}`) leaves no body or module open.
        let before = depth;
        for ch in ml.chars() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    while let Some(f) = frames.pop_if(|f| f.depth > depth) {
                        if f.direct || f.children > 1 {
                            inline.insert(f.line0);
                            match frames.last_mut() {
                                Some(p) => p.children += 1,
                                None => file_children += 1,
                            }
                        }
                    }
                    while bodies.pop_if(|d| *d > depth).is_some() {}
                }
                _ => {}
            }
        }
        if depth > before {
            if let Some((_, name)) = opens_mod {
                frames.push(Frame { line0: i as u32, depth, name: name.to_string(), direct: false, children: 0 });
            } else if opens_body {
                bodies.push(depth);
            }
        }
    }
    TestModules { file: file_direct || file_children > 1, inline, outline }
}

#[cfg(test)]
mod tests {
    use super::test_modules;
    use std::collections::HashSet;
    use std::path::Path;

    #[test]
    fn a_module_runs_as_a_whole_with_its_own_test_or_several_test_submodules() {
        let none = HashSet::new();
        let at = Path::new("/nonexistent/src/lib.rs");
        let scan = |text: &str, extra: &HashSet<u32>| test_modules(at, text, extra, 0, true);
        let one_mod = "pub fn f() {}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() {}\n}\n";
        let m = scan(one_mod, &none);
        assert!(!m.file && m.inline.contains(&2), "one test submodule: only it runs");
        let direct = "#[test]\nfn t() {}\nmod tests {\n    #[test]\n    fn u() {}\n}\n";
        assert!(scan(direct, &none).file, "a test of the file's own");
        let two = "mod a {\n    #[test]\n    fn t() {}\n}\nmod b {\n    #[tokio::test]\n    async fn u() {}\n}\n";
        assert!(scan(two, &none).file, "two test submodules");
        let nested = "mod outer {\n    mod inner {\n        #[test]\n        fn t() {}\n    }\n}\n";
        let m = scan(nested, &none);
        assert!(!m.file && m.inline.contains(&1) && !m.inline.contains(&0), "outer holds a single test submodule");
        let in_impl = "struct S;\nimpl S {\n    #[test]\n    fn t() {}\n}\n";
        assert!(!scan(in_impl, &none).file, "a method is no module declaration");
        let generated = "macro_rules! case { ($n:ident) => {}; }\ncase!(empty);\n";
        assert!(scan(generated, &HashSet::from([1])).file, "a test the index knows a macro generated");
        assert!(!test_modules(at, direct, &none, 0, false).file, "outside Cargo's layout the text's attributes do not count");
    }
}
