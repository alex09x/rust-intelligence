/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Syntax and token scanners for `macro_rules!` bodies, `proc-macro` crates, and unindexed Cargo targets.

use crate::format::DefKind;
use crate::workspace::cursor_path::is_ident_byte;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// A non-`self.` dot method call site (`.foo(` or `.foo::<`) inside a `macro_rules!` definition.
#[derive(Debug, Clone)]
pub struct MacroDotCall {
    pub method: String,
    pub line_1based: u32,
    pub col_1based: u32,
    pub is_turbofish: bool,
}

/// Pre-parsed summary of a single `macro_rules! name { ... }` block.
#[derive(Debug, Clone)]
pub struct ParsedMacroBlock {
    pub macro_name: String,
    pub file_id: u32,
    pub file_path: String,
    pub start_line_1based: u32,
    pub body: String,
    pub has_impl_kw: bool,
    pub glob_modules: Vec<String>,
    pub idents: HashSet<String>,
    pub dot_calls: Vec<MacroDotCall>,
}

/// Pre-loaded source file of a workspace `proc-macro` target.
#[derive(Debug, Clone)]
pub struct ProcMacroSourceFile {
    pub reader_idx: Option<usize>,
    pub file_id: Option<u32>,
    pub crate_name: String,
    pub file_path: String,
    pub content: String,
}

/// Pre-parsed imports, local declarations, and identifier set of an unindexed Cargo target `.rs` file.
#[derive(Debug, Clone)]
pub struct ParsedUnindexedTarget {
    pub path: PathBuf,
    pub pkg_crate: String,
    pub dep_crates: HashSet<String>,
    pub content: String,
    pub idents: HashSet<String>,
    pub local_defs: HashSet<(DefKind, String)>,
    pub explicit_imports: HashMap<String, Vec<String>>,
    pub anon_trait_imports: Vec<String>,
    pub glob_imports: Vec<String>,
}

pub fn has_self_receiver(sig: &str) -> bool {
    let args = sig.split("->").next().unwrap_or(sig);
    let inside = args.find('(').and_then(|s| args[s + 1..].find(')').map(|e| &args[s + 1..s + 1 + e])).unwrap_or(args);
    inside.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').any(|tok| tok == "self")
}

pub(crate) fn strip_line_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let (mut in_str, mut i) = (false, 0);
    while i < bytes.len() {
        if in_str {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            in_str = bytes[i] != b'"';
        } else if bytes[i] == b'"' {
            in_str = true;
        } else if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/') {
            return &line[..i];
        }
        i += 1;
    }
    line
}

fn collect_code_idents_and_strings(content: &str, out_idents: &mut HashSet<String>, mut out_str_idents: Option<&mut HashSet<String>>) {
    for raw_line in content.lines() {
        let line = strip_line_comment(raw_line);
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'"' {
                let (start, mut escaped) = (i + 1, false);
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        escaped = true;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                if !escaped {
                    let lit = &line[start..i.min(bytes.len())];
                    if !lit.is_empty() && (lit.as_bytes()[0].is_ascii_alphabetic() || lit.as_bytes()[0] == b'_') && lit.bytes().all(is_ident_byte) {
                        if let Some(ref mut s) = out_str_idents {
                            s.insert(lit.to_string());
                        }
                    }
                }
                i += 1;
            } else if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
                let start = i;
                while i < bytes.len() && is_ident_byte(bytes[i]) {
                    i += 1;
                }
                out_idents.insert(line[start..i].to_string());
            } else {
                i += 1;
            }
        }
    }
}

pub fn extract_macro_rules_blocks(file_id: u32, file_path: &str, content: &str) -> Vec<ParsedMacroBlock> {
    if !content.contains("macro_rules!") {
        return Vec::new();
    }
    let lines: Vec<&str> = content.lines().collect();
    let (mut blocks, mut idx) = (Vec::new(), 0usize);
    while idx < lines.len() {
        let code = strip_line_comment(lines[idx]);
        let Some(pos) = code.find("macro_rules!").filter(|&p| p == 0 || !is_ident_byte(code.as_bytes()[p - 1])) else {
            idx += 1;
            continue;
        };
        let after = code[pos + "macro_rules!".len()..].trim_start();
        let m_len = after.bytes().take_while(|&b| is_ident_byte(b)).count();
        let macro_name = if m_len > 0 { after[..m_len].to_string() } else { String::from("macro_rules") };

        let (mut depth, mut entered, mut end_idx) = (0i32, false, idx);
        while end_idx < lines.len() {
            let l_bytes = strip_line_comment(lines[end_idx]).as_bytes();
            let mut k = 0;
            while k < l_bytes.len() {
                if l_bytes[k] == b'"' {
                    k += 1;
                    while k < l_bytes.len() && l_bytes[k] != b'"' {
                        k += if l_bytes[k] == b'\\' { 2 } else { 1 };
                    }
                } else if matches!(l_bytes[k], b'{' | b'(' | b'[') {
                    depth += 1;
                    entered = true;
                } else if matches!(l_bytes[k], b'}' | b')' | b']') {
                    depth -= 1;
                }
                k += 1;
            }
            if entered && depth <= 0 {
                break;
            }
            end_idx += 1;
        }
        let end_idx = end_idx.min(lines.len().saturating_sub(1));
        let (mut idents, mut glob_modules, mut dot_calls) = (HashSet::new(), Vec::new(), Vec::new());
        for (off, &raw_ln) in lines[idx..=end_idx].iter().enumerate() {
            let (ln_1, c_ln) = ((idx + off + 1) as u32, strip_line_comment(raw_ln));
            collect_code_idents_and_strings(c_ln, &mut idents, None);
            if let Some(u_pos) = c_ln.find("use ").filter(|&p| p == 0 || !is_ident_byte(c_ln.as_bytes()[p - 1])) {
                if let Some(mod_path) = c_ln[u_pos + 4..].split(';').next().unwrap_or("").trim().strip_suffix("::*") {
                    let clean = mod_path.trim().trim_start_matches("$crate::").trim_start_matches("crate::").trim_start_matches("super::");
                    if !clean.is_empty() {
                        glob_modules.push(clean.to_string());
                    }
                }
            }
            let (b, mut k) = (c_ln.as_bytes(), 0usize);
            while k < b.len() {
                if b[k] == b'.' && (k == 0 || b[k - 1] != b'.') && b.get(k + 1).is_some_and(|ch| ch.is_ascii_alphabetic() || *ch == b'_') {
                    let before_dot = c_ln[..k].trim_end();
                    let is_self_dot = before_dot.ends_with("self") && (before_dot.len() == 4 || !is_ident_byte(before_dot.as_bytes()[before_dot.len() - 5]));
                    let m_start = k + 1;
                    let m_end = m_start + c_ln[m_start..].bytes().take_while(|&ch| is_ident_byte(ch)).count();
                    let after_m = c_ln[m_end..].trim_start();
                    if !is_self_dot && (after_m.starts_with('(') || after_m.starts_with("::<")) {
                        let col_1 = raw_ln[..m_start].chars().count() as u32 + 1;
                        dot_calls.push(MacroDotCall { method: c_ln[m_start..m_end].to_string(), line_1based: ln_1, col_1based: col_1, is_turbofish: after_m.starts_with("::<") });
                    }
                    k = m_end;
                } else {
                    k += 1;
                }
            }
        }
        let has_impl_kw = idents.contains("impl");
        let body = lines[idx..=end_idx].join("\n");
        blocks.push(ParsedMacroBlock {
            macro_name,
            file_id,
            file_path: file_path.to_string(),
            start_line_1based: (idx + 1) as u32,
            body,
            has_impl_kw,
            glob_modules,
            idents,
            dot_calls,
        });
        idx = end_idx + 1;
    }
    blocks
}

fn collect_rs_files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for p in entries.flatten().map(|e| e.path()) {
        if p.is_dir() {
            collect_rs_files_under(&p, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(p);
        }
    }
}

pub fn scan_proc_macro_dir(pm_crate: &str, src_dir: &Path, coupled: &mut HashMap<String, String>, out_files: &mut Vec<ProcMacroSourceFile>) {
    let mut rs_paths = Vec::new();
    collect_rs_files_under(src_dir, &mut rs_paths);
    rs_paths.sort();
    let (mut has_dot_interp, mut str_lits, mut dummy_idents) = (false, HashSet::new(), HashSet::new());

    for p in rs_paths {
        let Ok(content) = std::fs::read_to_string(&p) else { continue };
        collect_code_idents_and_strings(&content, &mut dummy_idents, Some(&mut str_lits));
        for raw_ln in content.lines() {
            let ln = strip_line_comment(raw_ln);
            for (i, _) in ln.match_indices('.') {
                let rest = ln[i + 1..].trim_start();
                if let Some(after_hash) = rest.strip_prefix('#') {
                    if after_hash.trim_start().bytes().next().is_some_and(|ch| ch.is_ascii_alphabetic() || ch == b'_') {
                        has_dot_interp = true;
                    }
                }
            }
        }
        out_files.push(ProcMacroSourceFile { reader_idx: None, file_id: None, crate_name: pm_crate.to_string(), file_path: p.to_string_lossy().to_string(), content });
    }
    if has_dot_interp {
        for lit in str_lits {
            coupled.entry(lit).or_insert_with(|| pm_crate.to_string());
        }
    }
}

fn expand_use_tree(prefix: &str, tree: &str, pkg_crate: &str, explicit: &mut HashMap<String, Vec<String>>, anon_traits: &mut Vec<String>, globs: &mut Vec<String>) {
    let t = tree.trim();
    if t.is_empty() {
        return;
    }
    if let (Some(brace_pos), Some(end_brace)) = (t.find('{'), t.rfind('}')) {
        let sub_prefix = t[..brace_pos].trim().trim_end_matches("::");
        let combined = match (prefix.is_empty(), sub_prefix.is_empty()) {
            (true, _) => sub_prefix.to_string(),
            (false, true) => prefix.to_string(),
            (false, false) => format!("{prefix}::{sub_prefix}"),
        };
        let inner = &t[brace_pos + 1..end_brace];
        let (mut depth, mut start) = (0i32, 0usize);
        for (idx, ch) in inner.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => depth -= 1,
                ',' if depth == 0 => {
                    expand_use_tree(&combined, &inner[start..idx], pkg_crate, explicit, anon_traits, globs);
                    start = idx + 1;
                }
                _ => {}
            }
        }
        expand_use_tree(&combined, &inner[start..], pkg_crate, explicit, anon_traits, globs);
        return;
    }
    let full = if t == "self" {
        prefix.to_string()
    } else if prefix.is_empty() {
        t.to_string()
    } else {
        format!("{prefix}::{t}")
    };
    let raw_norm = full.trim().trim_start_matches("::");
    let norm = raw_norm.strip_prefix("crate::").map_or_else(|| raw_norm.to_string(), |rest| format!("{pkg_crate}::{rest}"));
    if let Some(glob_mod) = norm.strip_suffix("::*").filter(|g| !g.is_empty()) {
        globs.push(glob_mod.to_string());
    } else if let Some((orig, alias)) = norm.split_once(" as ") {
        let (orig, alias) = (orig.trim(), alias.trim());
        if alias == "_" {
            anon_traits.push(orig.to_string());
        } else if !alias.is_empty() {
            explicit.entry(alias.to_string()).or_default().push(orig.to_string());
        }
    } else if let Some(short) = norm.rsplit("::").next().map(str::trim).filter(|s| !s.is_empty() && *s != "*") {
        explicit.entry(short.to_string()).or_default().push(norm);
    }
}

pub(crate) fn parse_file_imports(content: &str, pkg_crate: &str, pub_only: bool) -> (HashMap<String, Vec<String>>, Vec<String>) {
    let (mut explicit, mut anon, mut globs) = (HashMap::new(), Vec::new(), Vec::new());
    let lines: Vec<&str> = content.lines().map(strip_line_comment).collect();
    let mut idx = 0usize;
    while idx < lines.len() {
        let t = lines[idx].trim();
        let ok = if pub_only { t.starts_with("pub use ") } else { t.starts_with("use ") || t.starts_with("pub use ") || t.starts_with("pub(crate) use ") };
        if ok {
            let mut stmt = String::from(t);
            while !stmt.contains(';') && idx + 1 < lines.len() {
                idx += 1;
                stmt.push(' ');
                stmt.push_str(lines[idx].trim());
            }
            if let Some((_, after_use)) = stmt.split_once("use ") {
                expand_use_tree("", after_use.split(';').next().unwrap_or(after_use).trim(), pkg_crate, &mut explicit, &mut anon, &mut globs);
            }
        }
        idx += 1;
    }
    (explicit, globs)
}

pub fn parse_unindexed_rust_file(path: PathBuf, pkg_crate: String, dep_crates: HashSet<String>) -> Option<ParsedUnindexedTarget> {
    let content = std::fs::read_to_string(&path).ok()?;
    let mut idents = HashSet::new();
    collect_code_idents_and_strings(&content, &mut idents, None);

    let (mut local_defs, mut explicit_imports, mut anon_trait_imports, mut glob_imports) = (HashSet::new(), HashMap::new(), Vec::new(), Vec::new());
    let lines: Vec<&str> = content.lines().map(strip_line_comment).collect();
    let (mut idx, mut impl_brace_depth) = (0usize, 0i32);
    while idx < lines.len() {
        let trimmed = lines[idx].trim();
        if trimmed.starts_with("use ") || trimmed.starts_with("pub use ") || trimmed.starts_with("pub(crate) use ") {
            let mut stmt = String::from(trimmed);
            while !stmt.contains(';') && idx + 1 < lines.len() {
                idx += 1;
                stmt.push(' ');
                stmt.push_str(lines[idx].trim());
            }
            if let Some((_, after_use)) = stmt.split_once("use ") {
                expand_use_tree("", after_use.split(';').next().unwrap_or(after_use).trim(), &pkg_crate, &mut explicit_imports, &mut anon_trait_imports, &mut glob_imports);
            }
            idx += 1;
            continue;
        }
        let is_impl_hdr = trimmed.starts_with("impl ") || trimmed.starts_with("impl<") || trimmed.starts_with("unsafe impl");
        for (kw, kind) in [
            ("struct ", DefKind::Struct),
            ("enum ", DefKind::Enum),
            ("trait ", DefKind::Trait),
            ("union ", DefKind::Union),
            ("fn ", DefKind::Fn),
            ("mod ", DefKind::Mod),
            ("type ", DefKind::TyAlias),
        ] {
            if !(kind == DefKind::TyAlias && impl_brace_depth > 0) {
                if let Some(pos) = trimmed.find(kw).filter(|&p| p == 0 || !is_ident_byte(trimmed.as_bytes()[p - 1])) {
                    let rest = trimmed[pos + kw.len()..].trim_start();
                    let n_len = rest.bytes().take_while(|&b| is_ident_byte(b)).count();
                    if n_len > 0 && (impl_brace_depth == 0 || kind != DefKind::Fn) {
                        local_defs.insert((kind, rest[..n_len].to_string()));
                    }
                }
            }
        }
        let delta: i32 = lines[idx].bytes().map(|b| i32::from(b == b'{') - i32::from(b == b'}')).sum();
        if is_impl_hdr && delta > 0 {
            impl_brace_depth += delta;
        } else if impl_brace_depth > 0 {
            impl_brace_depth = (impl_brace_depth + delta).max(0);
        }
        idx += 1;
    }

    Some(ParsedUnindexedTarget { path, pkg_crate, dep_crates, content, idents, local_defs, explicit_imports, anon_trait_imports, glob_imports })
}
