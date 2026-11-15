/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The documentation a resolved completion item shows: the doc comment of its declaration, which the list builders
//! marked on the item, in rust-analyzer's markdown (code blocks tagged `rust`, a doctest's hidden `# ` lines dropped).

use serde_json::{json, Value};

/// Gives a completion item its declaration's doc comment as `documentation`, when it has one.
pub(crate) fn resolve_documentation(item: &mut Value) -> bool {
    let Some((file, line)) = super::completion_edits::item_decl(item) else { return false };
    let name = item["filterText"].as_str().or_else(|| item["label"].as_str()).unwrap_or_default();
    let name = name.rsplit('.').next().unwrap_or(name).to_string();
    let doc = std::fs::read_to_string(&file).ok().and_then(|text| {
        let line = super::fn_params::decl_line_near(&text, line, &name).unwrap_or(line);
        doc_comment_above(&text, line)
    });
    let Some(doc) = doc else { return false };
    item["documentation"] = json!({ "kind": "markdown", "value": doc });
    true
}

/// The `///` comment above the item declared on the 1-based `line` of `text`, past its attributes, as markdown.
pub(crate) fn doc_comment_above(text: &str, line: u32) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut i = (line as usize).checked_sub(1)?.min(lines.len());
    let mut docs: Vec<&str> = Vec::new();
    while i > 0 {
        i -= 1;
        let t = lines[i].trim();
        if let Some(d) = t.strip_prefix("///").filter(|_| !t.starts_with("////")) {
            docs.push(d.strip_prefix(' ').unwrap_or(d));
        } else if t.starts_with("#[") {
            continue;
        } else if t.ends_with(']') {
            // The last line of an attribute spread over lines: go on above its `#[`.
            match (i.saturating_sub(20)..i).rev().take_while(|&j| !lines[j].trim().is_empty() && !lines[j].trim().starts_with("///")).find(|&j| lines[j].trim().starts_with("#[")) {
                Some(j) => i = j,
                None => break,
            }
        } else {
            break;
        }
    }
    if docs.iter().all(|d| d.trim().is_empty()) {
        return None;
    }
    docs.reverse();
    Some(markdown(&docs))
}

/// Doc comment lines as rust-analyzer renders them: a code block without a language, or with doctest attributes
/// only, is Rust, and in a Rust block the lines hidden from the docs (`# `) are dropped.
fn markdown(lines: &[&str]) -> String {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut in_rust: Option<bool> = None;
    for &l in lines {
        let t = l.trim_start();
        if let Some(info) = t.strip_prefix("```") {
            if in_rust.is_some() {
                in_rust = None;
                out.push("```".to_string());
            } else {
                let rust =
                    info.split(',').map(str::trim).all(|a| a.is_empty() || matches!(a, "rust" | "ignore" | "no_run" | "should_panic" | "compile_fail") || a.starts_with("edition"));
                in_rust = Some(rust);
                out.push(if rust { "```rust".to_string() } else { l.to_string() });
            }
            continue;
        }
        if in_rust == Some(true) && (t == "#" || t.starts_with("# ")) {
            continue;
        }
        out.push(l.to_string());
    }
    out.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_doc_comments_above_declarations() {
        let src = "/// Adds two.\n///\n/// ```\n/// # use x::y;\n/// assert_eq!(add(1, 1), 2);\n/// ```\n#[inline]\n#[rustc_const_stable(\n    feature = \"x\",\n    since = \"1.0\",\n)]\npub fn add(a: u8, b: u8) -> u8 { a + b }\n\nstruct S {\n    /// First.\n    a: u8,\n    b: [u8; 2],\n    /// Third.\n    c: u8,\n}\n";
        assert_eq!(doc_comment_above(src, 12).as_deref(), Some("Adds two.\n\n```rust\nassert_eq!(add(1, 1), 2);\n```"));
        assert_eq!(doc_comment_above(src, 16).as_deref(), Some("First."));
        assert_eq!(doc_comment_above(src, 17), None, "a field without docs takes none from its neighbour");
        assert_eq!(doc_comment_above(src, 19).as_deref(), Some("Third."));
        assert_eq!(doc_comment_above("fn f() {}\n", 1), None);
        assert_eq!(doc_comment_above("/// ```text\n/// # not hidden\n/// ```\nfn f() {}\n", 4).as_deref(), Some("```text\n# not hidden\n```"));
    }
}
