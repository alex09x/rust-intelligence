/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The `use` declarations of one module level and the names they bring in, parsed as use trees: `a::Foo as Bar`
//! brings in `Bar`, not `Foo`; `a::{self, b::C}` brings in `a` and `C`; `a::*` is a glob. A level is the file's own,
//! or the body of an inline module (`mod tests { … }`), where its own `use`s count and the file's do not.

use crate::lsp::editing::action_parse::mask_comments_and_strings;

/// One path a use tree imports: the name it is known by (`None` for a glob or `as _`), and its full path.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Leaf {
    pub(crate) path: String,
    pub(crate) name: Option<String>,
    pub(crate) glob: bool,
}

fn tokens(tree: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = tree.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c.is_alphanumeric() || c == '_' || c == '#' {
            let mut end = i + c.len_utf8();
            while let Some(&(j, d)) = chars.peek() {
                if !(d.is_alphanumeric() || d == '_' || d == '#') {
                    break;
                }
                end = j + d.len_utf8();
                chars.next();
            }
            out.push(tree[i..end].trim_start_matches("r#").to_string());
        } else if c == ':' && chars.peek().is_some_and(|&(_, d)| d == ':') {
            chars.next();
            out.push("::".to_string());
        } else if matches!(c, '{' | '}' | ',' | '*') {
            out.push(c.to_string());
        }
    }
    out
}

fn parse(t: &[String], i: &mut usize, prefix: &[String], out: &mut Vec<Leaf>) {
    let mut segs = prefix.to_vec();
    if t.get(*i).is_some_and(|x| x == "::") {
        *i += 1; // `::std::…`
    }
    while let Some(tok) = t.get(*i) {
        match tok.as_str() {
            "{" => {
                *i += 1;
                while *i < t.len() && t[*i] != "}" {
                    parse(t, i, &segs, out);
                    if t.get(*i).is_some_and(|x| x == ",") {
                        *i += 1;
                    }
                }
                *i += 1;
                return;
            }
            "*" => {
                *i += 1;
                out.push(Leaf { path: segs.join("::"), name: None, glob: true });
                return;
            }
            "}" | "," => return,
            word => {
                segs.push(word.to_string());
                *i += 1;
                if t.get(*i).is_some_and(|x| x == "::") {
                    *i += 1;
                    continue;
                }
                if word == "self" {
                    segs.pop();
                }
                let path = segs.join("::");
                let name = if t.get(*i).is_some_and(|x| x == "as") {
                    *i += 2;
                    t.get(*i - 1).filter(|a| a.as_str() != "_").cloned()
                } else {
                    segs.last().cloned()
                };
                out.push(Leaf { path, name, glob: false });
                return;
            }
        }
    }
}

/// The paths the use tree `tree` (the text between `use` and `;`) imports.
pub(crate) fn use_leaves(tree: &str) -> Vec<Leaf> {
    let t = tokens(tree);
    let (mut out, mut i) = (Vec::new(), 0);
    while i < t.len() {
        let before = i;
        parse(&t, &mut i, &[], &mut out);
        if i == before {
            i += 1;
        }
    }
    out
}

/// The byte range of the module level at `cursor` in `doc`: the body of the innermost inline module around it, else
/// the whole file.
pub(crate) fn level_at(doc: &str, cursor: usize) -> (usize, usize) {
    super::completion_import::inline_mod_ranges(doc).into_iter().filter(|&(s, e)| s < cursor && cursor <= e).min_by_key(|&(s, e)| e - s).map_or((0, doc.len()), |(s, e)| (s + 1, e))
}

/// The `use` declarations of the level `doc[start..end]`, in order: the byte where each ends (after its `;`) and its
/// tree. Comments, strings and nested braces (items, inner modules) are not that level.
pub(crate) fn level_uses(doc: &str, (start, end): (usize, usize)) -> Vec<(usize, String)> {
    let masked = mask_comments_and_strings(doc);
    let b = masked.as_bytes();
    let (mut out, mut depth, mut p) = (Vec::new(), 0i32, start);
    while p < end.min(b.len()) {
        match b[p] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            b'u' if depth == 0
                && masked[p..].starts_with("use")
                && (p == 0 || !(b[p - 1].is_ascii_alphanumeric() || b[p - 1] == b'_'))
                && b.get(p + 3).is_some_and(u8::is_ascii_whitespace) =>
            {
                let Some(semi) = masked[p..end].find(';') else { break };
                // The masked text: a comment inside a multi-line `use { … }` names nothing.
                out.push((p + semi + 1, masked[p + 3..p + semi].to_string()));
                p += semi + 1;
                continue;
            }
            _ => {}
        }
        p += 1;
    }
    out
}

/// Whether the `use`s of `level` bring a name `name` into it, by whatever path.
pub(crate) fn level_imports(doc: &str, level: (usize, usize), name: &str) -> bool {
    level_uses(doc, level).iter().any(|(_, tree)| use_leaves(tree).iter().any(|l| l.name.as_deref() == Some(name)))
}

/// Whether the level imports everything of its parent (`use super::*;`).
pub(crate) fn level_globs_super(doc: &str, level: (usize, usize)) -> bool {
    level_uses(doc, level).iter().any(|(_, tree)| use_leaves(tree).iter().any(|l| l.glob && l.path == "super"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(tree: &str) -> Vec<Option<String>> {
        use_leaves(tree).into_iter().map(|l| l.name).collect()
    }

    #[test]
    fn test_use_trees() {
        assert_eq!(names(" a::Foo as Bar"), [Some("Bar".into())], "an alias brings in the alias only");
        assert_eq!(names(" std::{fmt, collections::{BTreeMap as Tree, VecDeque}}"), [Some("fmt".into()), Some("Tree".into()), Some("VecDeque".into())]);
        assert_eq!(names(" a::{self, b::C}"), [Some("a".into()), Some("C".into())]);
        assert_eq!(names(" a::Trait as _"), [None]);
        let glob = use_leaves(" super::*");
        assert_eq!(glob, [Leaf { path: "super".into(), name: None, glob: true }]);
        assert_eq!(names(" ::std::fmt::Write"), [Some("Write".into())]);
    }

    #[test]
    fn test_level_imports() {
        let doc = "#[cfg(unix)]\nuse a::Unix;\npub(super) use a::Up;\n// use a::Commented;\nfn f() { use a::InFn; }\nmod tests {\n    use super::*;\n    use crate::x::Flat;\n}\n";
        let file = (0, doc.len());
        assert!(level_imports(doc, file, "Unix"), "after an attribute");
        assert!(level_imports(doc, file, "Up"), "pub(super)");
        assert!(!level_imports(doc, file, "Commented"));
        assert!(!level_imports(doc, file, "InFn"), "a function's use");
        assert!(!level_imports(doc, file, "Flat"), "mod tests' use");
        let tests = level_at(doc, doc.find("crate::x").unwrap());
        assert!(level_imports(doc, tests, "Flat"));
        assert!(level_globs_super(doc, tests));
        assert!(!level_imports(doc, tests, "Unix"));
        let commented = "use a::{\n    B,\n    // Foo was here\n    C,\n};\n";
        assert!(level_imports(commented, (0, commented.len()), "C"));
        assert!(!level_imports(commented, (0, commented.len()), "Foo"), "a comment in a use tree names nothing");
    }
}
