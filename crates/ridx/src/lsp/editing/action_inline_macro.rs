/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `Inline macro`: the innermost `macro_rules!` call around the cursor replaced by its one-step expansion, where
//! the expansion means what the call meant: names a hygienic expansion binds must not catch the call's own names,
//! an expression argument spliced into an `expr` fragment must not change precedence, the macros the expansion calls
//! must be in scope, and `$crate` must name a crate the file links.

use super::action_inline_callers_macros::{macros_in_scope, STD_MACROS};
use super::action_inline_callers_qualify::tokens;
use super::action_inline_macro_text::{is_atomic, rejoin_punctuation, tidy_spacing, top_level_pieces};
use super::action_parse::{is_ident_byte, line_starts_of, mask_comments_and_strings};
use super::action_parse_enclosing::find_enclosing_fn_at_cursor;
use super::action_parse_items::{contains_word, skip_balanced};
use super::action_struct_conv_index::byte_to_line_col;
use super::action_struct_conv_outside::enclosing_open;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_syntax::name_res::mbe_parse::MacroRulesDef;

pub const KIND_INLINE_MACRO: &str = "refactor.inline.macro";

/// The `Inline macro` action at the cursor of an empty `range`, with its edit.
pub fn inline_macro_action(doc: &str, uri: &str, range: &serde_json::Value, doc_version: Option<i32>, ws: &WorkspaceIndex) -> Option<serde_json::Value> {
    if range["start"] != range["end"] {
        return None;
    }
    let (line0, col) = (range["start"]["line"].as_u64()? as u32, range["start"]["character"].as_u64()? as u32);
    let edit = inline_macro_edit(doc, uri, line0, col, doc_version, ws)?;
    Some(serde_json::json!({
        "title": "Inline macro",
        "kind": "refactor.inline",
        "data": { "uri": uri, "version": doc_version, "kind": KIND_INLINE_MACRO, "cursor": { "line": line0, "character": col } },
        "edit": edit
    }))
}

/// The workspace edit inlining the macro call at the cursor `(line0, col)` (UTF-16).
pub fn inline_macro_edit(doc: &str, uri: &str, line0: u32, col: u32, doc_version: Option<i32>, ws: &WorkspaceIndex) -> Option<serde_json::Value> {
    let starts = line_starts_of(doc);
    let cursor = super::action_parse::utf16_pos_to_byte(doc, &starts, line0, col)?;
    let (lo, hi, text) = plan_inline_macro(doc, &crate::lsp::protocol::uri_to_path(uri), cursor, ws)?;
    super::action_struct_conv_helpers::to_lsp_edits(doc, &starts, vec![(lo, hi, text)]).map(|edits| {
        let mut obj = serde_json::json!({ "changes": { uri: edits.clone() } });
        if let Some(v) = doc_version {
            obj["documentChanges"] = serde_json::json!([{ "textDocument": { "uri": uri, "version": v }, "edits": edits }]);
        }
        obj
    })
}

/// The replacement of `doc[start..end]` that inlines the macro call around `cursor`.
pub fn plan_inline_macro(doc: &str, file: &str, cursor: usize, ws: &WorkspaceIndex) -> Option<(usize, usize, String)> {
    let masked = mask_comments_and_strings(doc);
    let starts = line_starts_of(doc);
    let call = macro_call_at(&masked, cursor)?;
    let name = &masked[call.name.0..call.name.1];
    if STD_MACROS.contains(&name) || name == "macro_rules" {
        return None;
    }
    let (def_text, def_crate) = macro_definition(doc, &starts, file, call.name.0, name, ws)?;
    let def = MacroRulesDef::parse_body(&def_text)?;
    let inner = &doc[call.open + 1..call.close - 1];
    let prefix = crate_prefix(ws, file, &masked, &masked[call.path_start..call.name.0], def_crate.as_deref())?;
    let expansion = tidy_spacing(&rejoin_punctuation(&def.expand(inner, &prefix)?));
    let expansion = expansion.trim();
    if expansion.is_empty() || !keeps_meaning(&def_text, inner, expansion, doc) {
        return None;
    }
    place(doc, &masked, &starts, &call, expansion)
}

struct MacroCall {
    path_start: usize,
    name: (usize, usize),
    open: usize,
    /// One past the closing delimiter.
    close: usize,
}

/// The innermost macro call (`path::name!(..)`, `name! { .. }`) whose name or arguments hold byte `at`.
fn macro_call_at(masked: &str, at: usize) -> Option<MacroCall> {
    let b = masked.as_bytes();
    let call_after_name = |name_end: usize| -> Option<MacroCall> {
        let rest = masked[name_end..].strip_prefix('!')?;
        let open = masked.len() - rest.trim_start().len();
        let close_ch = match *b.get(open)? {
            b'(' => b')',
            b'[' => b']',
            b'{' => b'}',
            _ => return None,
        };
        let close = skip_balanced(b, open, b[open], close_ch)?;
        let name_start = masked[..name_end].trim_end_matches(|c: char| c.is_alphanumeric() || c == '_').len();
        let path_start = masked[..name_start].trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == ':').len();
        // An attribute's or a `macro_rules!` definition's name is no call.
        let before = masked[..path_start].trim_end();
        (name_start < name_end && !before.ends_with('#') && !before.ends_with("#!")).then_some(MacroCall { path_start, name: (name_start, name_end), open, close })
    };
    // The cursor on the name itself.
    let mut e = at;
    while e < b.len() && is_ident_byte(b[e]) {
        e += 1;
    }
    if e > at || (at > 0 && is_ident_byte(b[at - 1])) {
        if let Some(c) = call_after_name(e) {
            return Some(c);
        }
    }
    // Inside the arguments: the enclosing delimiters, innermost first.
    let mut pos = at;
    while let Some(open) = enclosing_open(masked, pos) {
        let head = masked[..open].trim_end();
        if let Some(name_end) = head.strip_suffix('!').map(str::len) {
            if let Some(c) = call_after_name(name_end).filter(|c| c.open == open) {
                return Some(c);
            }
        }
        pos = open;
    }
    None
}

/// The `macro_rules!` body (with its braces) the call's name resolves to, and the defining crate when it is not the
/// file's own.
fn macro_definition(doc: &str, starts: &[usize], file: &str, name_at: usize, name: &str, ws: &WorkspaceIndex) -> Option<(String, Option<String>)> {
    let (line, col) = byte_to_line_col(doc, starts, name_at);
    let own = ws.reader_indices_for_file(file).first().map(|&i| ws.readers()[i].crate_name().to_string());
    // The index's resolution of the name, else its one `macro_rules!` of that name (the file's crate's first).
    let (path, target_line) = match ws.find_direct_references_at(file, line, col).filter(|r| r.target_kind == DefKind::Macro) {
        Some(refs) => (refs.target_resolved_path(), refs.target_line),
        None => {
            let defs: Vec<_> = ws.exact_symbols_by_name(name).into_iter().filter(|s| s.kind == DefKind::Macro && !s.is_macro_expansion).collect();
            let local: Vec<_> = defs.iter().filter(|s| own.as_deref() == Some(s.crate_name)).collect();
            let pick = if local.is_empty() { defs.iter().collect::<Vec<_>>() } else { local };
            // One definition seen through several units is one place.
            let mut places: Vec<(std::path::PathBuf, u32)> = pick
                .iter()
                .map(|s| {
                    let p = s.resolved_path();
                    (std::fs::canonicalize(&p).unwrap_or(p), s.span.start_line())
                })
                .collect();
            places.sort();
            places.dedup();
            let [only] = places.as_slice() else { return None };
            only.clone()
        }
    };
    let text = std::fs::read_to_string(&path).ok()?;
    if path.to_string_lossy().contains("/library/") {
        return None;
    }
    let masked = mask_comments_and_strings(&text);
    let line_lo = line_starts_of(&text).get((target_line as usize).saturating_sub(1)).copied()?;
    // The `macro_rules! name` nearest to the line the index gives (a file may define several macros close together).
    let named = |at: usize| {
        let rest = masked[at + 12..].trim_start();
        rest.strip_prefix(name).filter(|r| !r.starts_with(|c: char| c.is_alphanumeric() || c == '_'))
    };
    let at = masked.match_indices("macro_rules!").map(|(p, _)| p).filter(|&p| named(p).is_some()).min_by_key(|&p| p.abs_diff(line_lo))?;
    let after = named(at)?;
    let open = masked.len() - after.trim_start().len();
    let b = masked.as_bytes();
    let close_ch = match *b.get(open)? {
        b'(' => b')',
        b'[' => b']',
        b'{' => b'}',
        _ => return None,
    };
    let close = skip_balanced(b, open, b[open], close_ch)?;
    let own = ws.reader_indices_for_file(file).first().map(|&i| ws.readers()[i].crate_name().replace('-', "_"));
    let def_crate = ws.reader_indices_for_file(&path.to_string_lossy()).first().map(|&i| ws.readers()[i].crate_name().replace('-', "_"));
    let foreign = def_crate.filter(|d| own.as_deref() != Some(d.as_str()));
    Some((text[open..close].to_string(), foreign))
}

/// What `$crate` becomes in the file: `crate` for its own macros; another crate's name, written as the call's path
/// writes it (`bitflags::bitflags!`), as the file's crate links it, or as the file itself names it (`use
/// expect_test::expect;` in a test module, whose dev-dependencies the library unit's extern list leaves out).
fn crate_prefix(ws: &WorkspaceIndex, file: &str, masked_doc: &str, call_path: &str, def_crate: Option<&str>) -> Option<String> {
    let Some(def_crate) = def_crate else { return Some("crate".to_string()) };
    if let Some(head) = call_path.trim_end_matches("::").split("::").next().filter(|h| !h.is_empty() && *h != "crate" && *h != "self" && *h != "super") {
        return Some(format!("::{head}"));
    }
    let b = masked_doc.as_bytes();
    let named_root = masked_doc.match_indices(&format!("{def_crate}::")).any(|(p, _)| p == 0 || !(is_ident_byte(b[p - 1]) || b[p - 1] == b':'));
    (named_root || ws.extern_crate_names_for_file(file).is_none_or(|l| l.contains(&def_crate))).then(|| format!("::{def_crate}"))
}

/// Whether the expansion means what the call meant (see the module documentation).
fn keeps_meaning(def_text: &str, inner: &str, expansion: &str, doc: &str) -> bool {
    let inner_m = mask_comments_and_strings(inner);
    let exp_m = mask_comments_and_strings(expansion);
    let arg_names: std::collections::HashSet<&str> = tokens(&inner_m).into_iter().map(|(s, e)| &inner_m[s..e]).collect();
    // Only the names the macro's own template binds are hygienic; those the arguments bind are the caller's.
    if binders(&mask_comments_and_strings(def_text)).iter().any(|n| arg_names.contains(n.as_str())) {
        return false;
    }
    let has_expr_frag = def_text.match_indices('$').any(|(p, _)| {
        let rest = &def_text[p + 1..];
        let name_len = rest.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(rest.len());
        rest[name_len..].trim_start().strip_prefix(':').is_some_and(|f| f.trim_start().starts_with("expr"))
    });
    if has_expr_frag && top_level_pieces(&inner_m).iter().any(|p| !is_atomic(p)) {
        return false;
    }
    macros_in_scope(expansion, doc) || nested_macros_defined_in(&exp_m, &mask_comments_and_strings(doc))
}

/// The length of a `let` pattern in `rest` (after `let`): up to its `=`, `:` or `;`.
fn let_pattern_len(rest: &str) -> usize {
    let b = rest.as_bytes();
    let mut depth = 0i32;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'=' | b';' if depth <= 0 => return i,
            b':' if depth <= 0 && b.get(i + 1) != Some(&b':') && (i == 0 || b[i - 1] != b':') => return i,
            _ => {}
        }
    }
    b.len()
}

/// The names a macro template binds itself (not through a metavariable `$x`): `let x`, `let mut x`, `for x in`,
/// closure parameters, labels.
fn binders(m: &str) -> Vec<String> {
    let toks: Vec<(usize, usize)> = tokens(m).into_iter().filter(|&(s, _)| !m[..s].ends_with('$')).collect();
    let word = |i: usize| toks.get(i).map(|&(s, e)| &m[s..e]);
    let mut out = Vec::new();
    for i in 0..toks.len() {
        match word(i) {
            // The names a pattern binds (`let x`, `if let Some(&s)`, `let (a, mut b)`), not its constructors.
            Some("let") => {
                let pat_end = toks[i].1 + let_pattern_len(&m[toks[i].1..]);
                for &(s, e) in toks[i + 1..].iter().take_while(|&&(s, _)| s < pat_end) {
                    let name = &m[s..e];
                    let after = m[e..].trim_start();
                    let binds = name.starts_with(|c: char| c.is_lowercase() || c == '_') && !matches!(name, "mut" | "ref" | "_");
                    if binds && !m[..s].ends_with("::") && !after.starts_with(['(', '{', '!']) && !after.starts_with("::") && !m[toks[i].1..s].contains('$') {
                        out.push(name.to_string());
                    }
                }
            }
            Some("for") if word(i + 2) == Some("in") && !m[toks[i].1..toks[i + 1].0].contains('$') => out.push(word(i + 1).unwrap_or_default().to_string()),
            _ => {}
        }
    }
    for (p, _) in m.match_indices('|') {
        if let Some(q) = m[p + 1..].find('|') {
            let params = &m[p + 1..p + 1 + q];
            if !params.contains(['(', '{', ';', '$']) && !params.trim().is_empty() {
                out.extend(tokens(params).into_iter().map(|(s, e)| params[s..e].to_string()));
            }
        }
    }
    out.extend(m.match_indices('\'').filter_map(|(p, _)| {
        let rest = &m[p + 1..];
        let n = rest.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(rest.len());
        (n > 0 && rest[n..].trim_start().starts_with(':') && !rest[n..].trim_start().starts_with("::")).then(|| rest[..n].to_string())
    }));
    out.sort();
    out.dedup();
    out
}

/// Whether every macro the expansion calls by a bare name is defined (`macro_rules!`) in the document.
fn nested_macros_defined_in(exp_m: &str, doc_m: &str) -> bool {
    tokens(exp_m).into_iter().all(|(s, e)| {
        let n = &exp_m[s..e];
        let bang = exp_m[e..].starts_with('!') && !exp_m[e..].starts_with("!=");
        !bang || exp_m[..s].ends_with("::") || STD_MACROS.contains(&n) || doc_m.contains(&format!("macro_rules! {n}"))
    })
}

/// The edit putting `expansion` where the call is: spliced at an item or statement position (a trailing `;` dropped
/// outside a function body, where it would be an empty item; `let`s wrapped in a block inside one, so their names stay
/// the expansion's), wrapped as `({ .. })` in an expression.
fn place(doc: &str, masked: &str, starts: &[usize], call: &MacroCall, expansion: &str) -> Option<(usize, usize, String)> {
    let prev = masked[..call.path_start].trim_end();
    let next = masked[call.close..].trim_start();
    let at_statement = prev.is_empty() || prev.ends_with([';', '{', '}', ']']);
    let ends_statement = next.is_empty() || next.starts_with([';', '}']) || masked.as_bytes()[call.open] == b'{';
    let in_fn = find_enclosing_fn_at_cursor(doc, masked, starts, call.path_start).is_some_and(|f| {
        let super::action_parse::ItemKindInfo::Fn { body_range: Some((bo, bc)), .. } = f.kind else { return false };
        call.path_start > bo && call.path_start < bc
    });
    let exp_m = mask_comments_and_strings(expansion);
    if at_statement && ends_statement {
        if !in_fn {
            let semi = if next.starts_with(';') { masked.len() - next.len() + 1 } else { call.close };
            return Some((call.path_start, semi, expansion.to_string()));
        }
        let declares_items = ["fn", "struct", "enum", "union", "impl", "trait", "mod", "use", "static", "const", "type", "macro_rules"].iter().any(|k| contains_word(&exp_m, k));
        if contains_word(&exp_m, "let") {
            return (!declares_items).then(|| (call.path_start, call.close, format!("{{ {expansion} }}")));
        }
        return Some((call.path_start, call.close, expansion.to_string()));
    }
    let in_expression = prev.ends_with(['=', '(', ',', '[', '>']) || contains_word(prev.rsplit(|c: char| !c.is_alphanumeric()).next().unwrap_or(""), "return");
    (in_expression && in_fn).then(|| (call.path_start, call.close, format!("({{ {expansion} }})")))
}

#[cfg(test)]
mod tests {
    use super::{binders, macro_call_at};

    #[test]
    fn the_macro_call_around_the_cursor_and_its_arguments_are_read() {
        let s = "fn f() { cfg_rt! { pub fn a() { g!(x) } } }";
        let c = macro_call_at(s, s.find("x)").unwrap()).unwrap();
        assert_eq!(&s[c.name.0..c.name.1], "g");
        let c = macro_call_at(s, s.find("pub fn").unwrap()).unwrap();
        assert_eq!(&s[c.name.0..c.name.1], "cfg_rt");
        let q = "bitflags::bitflags! { struct F: u8 {} }";
        let c = macro_call_at(q, 12).unwrap();
        assert_eq!((&q[c.path_start..c.name.0], &q[c.name.0..c.name.1]), ("bitflags::", "bitflags"));
        assert_eq!(binders("{ let tmp = 1; for i in 0..n {} 'out: loop {} |a, b| a }"), ["a", "b", "i", "out", "tmp"]);
        assert!(binders("{ impl X for Y { fn f(&mut self) { let $v = 1; for $i in 0..n {} } } }").is_empty());
        assert_eq!(binders("{ let msg = if let Some(&s) = e.downcast_ref::<&str>() { s } else { x }; let (a, mut b): (u8, u8) = p; }"), ["a", "b", "msg", "s"]);
        let pp = "pin_project! {\n    #[derive(Debug)]\n    struct RunUntil<'a, F> {\n        local_set: &'a LocalSet,\n    }\n}";
        let masked = crate::lsp::editing::action_parse::mask_comments_and_strings(pp);
        let c = macro_call_at(&masked, pp.find("RunUntil").unwrap()).expect("call around a struct with lifetimes");
        assert_eq!(&pp[c.name.0..c.name.1], "pin_project");
    }
}
