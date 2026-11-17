/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Definitions the compiler's index cannot give, found in the file's syntax tree: the construct a control-flow
//! keyword belongs to (`return` → its function, `break` → its loop), a label or lifetime, a `mod foo;` declaration's
//! file, and a macro call's `macro_rules!` (the index records no reference at a call site: it is expanded away).
//! Before these, a keyword asked the index, which answered with what `for` desugars to (`IntoIterator::into_iter`,
//! `Some`) for any spot in a loop's body.

use ridx_syntax::{parse, SyntaxKind as K, SyntaxTree, TokenKind as T};
use std::ops::Range;
use std::path::{Path, PathBuf};

/// Where a definition found from the syntax lies.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// A byte range of the same file.
    Here(Range<usize>),
    /// A whole file (a module's).
    File(PathBuf),
    /// A macro named so, not defined in this file before the call: the index's `macro_rules!` of that name, preferably
    /// the one a `use` of the file at these byte offsets imports.
    Macro { name: String, use_sites: Vec<usize> },
    /// Item `member` of what the name at byte `anchor` of this file resolves to in the index (a trait's item, from its
    /// implementation's).
    Member { anchor: usize, member: String, kinds: &'static [ridx_format::DefKind] },
    /// The token is one the syntax answers for, with nothing to go to: the index must not answer instead.
    Nothing,
}

/// The name a path type is known by: its last segment before generic arguments (`Foo` of `crate::m::Foo<T>`).
fn type_name(tree: &SyntaxTree, ty: ridx_syntax::NodeId) -> Option<Range<usize>> {
    tree.tokens_of(ty).iter().take_while(|t| t.kind != T::Lt).filter(|t| matches!(t.kind, T::Ident | T::RawIdent)).last().map(|t| t.range())
}

/// The type children of an `impl` header: `[trait, self type]` with `for`, else `[self type]`.
fn impl_types(tree: &SyntaxTree, imp: ridx_syntax::NodeId) -> Vec<ridx_syntax::NodeId> {
    tree.children(imp).filter(|&c| !matches!(tree.kind(c), K::GenericParamList | K::WhereClause | K::ItemList | K::Attr | K::Visibility)).collect()
}

/// `Foo { x }` or `Foo { x } = …`: for a field written as its own binding, the name the record's path ends with and
/// the field's name.
pub(crate) fn shorthand(text: &str, offset: usize) -> Option<(usize, String)> {
    let tree = parse(text);
    let i = token_index(&tree, offset)?;
    let tok = tree.tokens[i];
    if !matches!(tok.kind, T::Ident | T::RawIdent) {
        return None;
    }
    let node = tree.covering(tok.start as usize);
    let field = std::iter::once(node).chain(tree.ancestors(node)).find(|&a| matches!(tree.kind(a), K::RecordExprField | K::RecordPatField))?;
    if tree.own_tokens(field).any(|(_, t)| t.kind == T::Colon) || tree.descendants(field).any(|d| tree.kind(d) == K::IdentPat && tree.text(d).contains(':')) {
        return None;
    }
    let record = tree.ancestors(field).find(|&a| matches!(tree.kind(a), K::RecordExpr | K::RecordPat))?;
    let path = tree.child(record, K::Path)?;
    Some((type_name(&tree, path)?.start, tok.text(text).trim_start_matches("r#").to_string()))
}

/// The token a request at byte `offset` is about: the one under it, or the name or keyword ending right at it (a caret
/// just past `spam` in `foo.spam;`), as rust-analyzer picks the token left of a caret.
pub(crate) fn token_index(tree: &SyntaxTree, offset: usize) -> Option<usize> {
    let toks = &tree.tokens;
    let at = toks.partition_point(|t| (t.end as usize) <= offset);
    let wordy = |k: T| matches!(k, T::Ident | T::RawIdent | T::Lifetime) || is_keyword(k);
    let under = toks.get(at).filter(|t| (t.start as usize) <= offset);
    let before = at.checked_sub(1).filter(|&i| toks[i].end as usize == offset);
    match (under, before) {
        (Some(t), Some(b)) if !wordy(t.kind) && wordy(toks[b].kind) => Some(b),
        (Some(t), _) if !t.is_trivia() => Some(at),
        // Past the last token before a space or the line's end (`=>| {}`).
        (_, Some(b)) if !toks[b].is_trivia() => Some(b),
        _ => None,
    }
}

fn is_keyword(k: T) -> bool {
    (k as u8) >= (T::As as u8) && (k as u8) <= (T::Underscore as u8)
}

/// The definition at byte `offset` of `text` (the file `file`) that the syntax tree gives, if the token there is one
/// the syntax answers for.
pub(crate) fn definition(file: &Path, text: &str, offset: usize) -> Option<Target> {
    let tree = parse(text);
    let i = token_index(&tree, offset)?;
    let tok = tree.tokens[i];
    let node = tree.covering(tok.start as usize);
    let up = || std::iter::once(node).chain(tree.ancestors(node));
    let first_tok = |id| tree.tokens_of(id).iter().find(|t| !t.is_trivia()).map(|t| t.range());
    let kw_of = |id, kind: T| tree.tokens_of(id).iter().find(|t| t.kind == kind).map(|t| t.range());
    let next_sig = || tree.tokens[i + 1..].iter().find(|t| !t.is_trivia()).copied();
    match tok.kind {
        T::Return | T::Question => {
            let f = up().find(|&a| matches!(tree.kind(a), K::Fn | K::ClosureExpr))?;
            Some(match tree.kind(f) {
                K::Fn => kw_of(f, T::Fn).map_or(Target::Nothing, Target::Here),
                _ => first_tok(f).map_or(Target::Nothing, Target::Here),
            })
        }
        T::Break | T::Continue => match next_sig().filter(|t| t.kind == T::Lifetime) {
            Some(label) => Some(label_target(&tree, node, label.text(text), true).unwrap_or(Target::Nothing)),
            None => {
                let l = up().take_while(|&a| !matches!(tree.kind(a), K::Fn | K::ClosureExpr)).find(|&a| matches!(tree.kind(a), K::LoopExpr | K::WhileExpr | K::ForExpr))?;
                Some(loop_keyword(&tree, l).map_or(Target::Nothing, Target::Here))
            }
        },
        T::Lifetime => {
            let name = tok.text(text);
            let prev = tree.tokens[..i].iter().rev().find(|t| !t.is_trivia()).map(|t| t.kind);
            if matches!(prev, Some(T::Break | T::Continue)) {
                return Some(label_target(&tree, node, name, false).unwrap_or(Target::Nothing));
            }
            if up().any(|a| matches!(tree.kind(a), K::Label | K::LifetimeParam)) {
                return Some(Target::Here(tok.range()));
            }
            // A lifetime used in a signature: the innermost generic parameter of that name.
            let params = up().filter_map(|a| tree.child(a, K::GenericParamList)).find_map(|gl| {
                tree.children(gl)
                    .filter(|&p| tree.kind(p) == K::LifetimeParam)
                    .find_map(|p| tree.tokens_of(p).iter().find(|t| t.kind == T::Lifetime && t.text(text) == name).map(|t| t.range()))
            });
            Some(params.map_or(Target::Nothing, Target::Here))
        }
        T::For | T::While | T::Loop | T::Fn | T::Match => Some(Target::Here(tok.range())),
        T::If => {
            // `else if` belongs to the chain's first `if`.
            let mut top = up().find(|&a| tree.kind(a) == K::IfExpr)?;
            while let Some(p) = tree.parent(top).filter(|&p| tree.kind(p) == K::IfExpr) {
                top = p;
            }
            Some(kw_of(top, T::If).map_or(Target::Nothing, Target::Here))
        }
        // A match arm's `=>` (lexed `=` `>`), from either half.
        T::Eq if next_sig().is_some_and(|t| t.kind == T::Gt && t.start == tok.end) && up().any(|a| tree.kind(a) == K::MatchArm) => {
            Some(Target::Here(tok.start as usize..tok.end as usize + 1))
        }
        T::Gt if i > 0 && tree.tokens[i - 1].kind == T::Eq && tree.tokens[i - 1].end == tok.start && up().any(|a| tree.kind(a) == K::MatchArm) => {
            Some(Target::Here(tree.tokens[i - 1].start as usize..tok.end as usize))
        }
        T::SelfKw if up().any(|a| tree.kind(a) == K::SelfParam) => Some(Target::Here(tok.range())),
        // `Self`: the type its `impl` names (rust-analyzer's), or the trait it is used in.
        T::SelfType => {
            let owner = up().find(|&a| matches!(tree.kind(a), K::Impl | K::Trait | K::Struct | K::Enum | K::Union))?;
            let r = match tree.kind(owner) {
                K::Impl => impl_types(&tree, owner).last().and_then(|&t| type_name(&tree, t)),
                _ => tree.child(owner, K::Name).map(|n| tree.range(n)),
            };
            Some(r.map_or(Target::Nothing, Target::Here))
        }
        T::Ident | T::RawIdent => ident_target(file, text, &tree, i, node),
        k if is_keyword(k) && !matches!(k, T::SelfKw | T::SelfType | T::Crate | T::Super) => Some(Target::Nothing),
        _ => None,
    }
}

/// The `for`, `while` or `loop` keyword of loop `l`.
fn loop_keyword(tree: &SyntaxTree, l: ridx_syntax::NodeId) -> Option<Range<usize>> {
    tree.own_tokens(l).map(|(_, t)| t).find(|t| matches!(t.kind, T::For | T::While | T::Loop)).map(|t| t.range())
}

/// The construct label `name` names, from inside it: from `break`/`continue` (`keyword`) its loop's keyword, from the
/// label itself the label; a labelled block has only its label. A macro's body is a token tree: the label is looked
/// up by its text among the enclosing labels.
fn label_target(tree: &SyntaxTree, node: ridx_syntax::NodeId, name: &str, keyword: bool) -> Option<Target> {
    let src = tree.src;
    std::iter::once(node).chain(tree.ancestors(node)).take_while(|&a| !matches!(tree.kind(a), K::Fn | K::ClosureExpr)).find_map(|a| {
        let label = tree.child(a, K::Label)?;
        let lt = tree.tokens_of(label).iter().find(|t| t.kind == T::Lifetime && t.text(src) == name)?;
        Some(Target::Here(loop_keyword(tree, a).filter(|_| keyword).unwrap_or(lt.range())))
    })
}

/// A name the syntax resolves: a `mod foo;`'s file, a macro call's `macro_rules!`.
fn ident_target(file: &Path, text: &str, tree: &SyntaxTree, i: usize, node: ridx_syntax::NodeId) -> Option<Target> {
    let tok = tree.tokens[i];
    let name = tok.text(text).trim_start_matches("r#");
    let next = tree.tokens[i + 1..].iter().find(|t| !t.is_trivia()).copied();
    // `mod foo;`: the file of the module.
    if tree.kind(node) == K::Name && tree.parent(node).is_some_and(|m| tree.kind(m) == K::Module && tree.child(m, K::ItemList).is_none()) {
        return module_file(file, name).map(Target::File);
    }
    // The name of an item of `impl Trait for T`: the trait's declaration of it, as rust-analyzer goes.
    if tree.kind(node) == K::Name {
        let item = tree.parent(node).filter(|&p| matches!(tree.kind(p), K::Fn | K::Const | K::TypeAlias));
        let imp = item.and_then(|p| tree.parent(p)).filter(|&l| tree.kind(l) == K::ItemList).and_then(|l| tree.parent(l)).filter(|&i| tree.kind(i) == K::Impl);
        if let Some(imp) = imp.filter(|&i| tree.own_tokens(i).any(|(_, t)| t.kind == T::For)) {
            use ridx_format::DefKind as D;
            let anchor = impl_types(tree, imp).first().and_then(|&t| type_name(tree, t))?;
            // A trait may hold `type A` and `const A`: the item of the same kind.
            let kinds: &'static [D] = match item.map(|p| tree.kind(p)) {
                Some(K::Fn) => &[D::AssocFn, D::Fn],
                Some(K::Const) => &[D::AssocConst, D::Const],
                _ => &[D::AssocTy, D::TyAlias],
            };
            return Some(Target::Member { anchor: anchor.start, member: name.to_string(), kinds });
        }
    }
    let in_macro_path = std::iter::once(node)
        .chain(tree.ancestors(node))
        .take_while(|&a| matches!(tree.kind(a), K::Path | K::PathSegment | K::Name | K::MacroCall))
        .any(|a| tree.kind(a) == K::MacroCall);
    let in_macro_use = std::iter::once(node).chain(tree.ancestors(node)).any(|a| tree.kind(a) == K::Attr && tree.text(a).contains("macro_use"));
    if !(next.is_some_and(|t| t.kind == T::Bang) && in_macro_path || in_macro_use) {
        return None;
    }
    // A `macro_rules!` of the file: the nearest one above the call (textual scope), else any.
    let defs: Vec<Range<usize>> = tree
        .descendants(tree.root())
        .filter(|&d| tree.kind(d) == K::MacroRules)
        .filter_map(|d| tree.child(d, K::Name))
        .filter(|&n| tree.text(n) == name)
        .map(|n| tree.range(n))
        .collect();
    // A path (`m::mac!`) names a macro elsewhere; a bare name, one in textual scope first.
    let qualified = tree.covering_of(tok.start as usize, K::MacroCall).and_then(|c| tree.child(c, K::Path)).is_some_and(|p| tree.text(p).contains("::"));
    if !in_macro_use && !qualified {
        if let Some(r) = defs.iter().rev().find(|r| r.start < tok.start as usize).or(defs.first()) {
            return Some(Target::Here(r.clone()));
        }
    }
    let use_sites = tree
        .descendants(tree.root())
        .filter(|&d| tree.kind(d) == K::Use)
        .flat_map(|u| {
            tree.tokens_of(u)
                .iter()
                .filter(|t| matches!(t.kind, T::Ident | T::RawIdent) && t.text(text).trim_start_matches("r#") == name)
                .map(|t| t.start as usize)
                .collect::<Vec<_>>()
        })
        .collect();
    Some(Target::Macro { name: name.to_string(), use_sites })
}

/// The file of `mod name;` declared in `file`: `name.rs` or `name/mod.rs` beside a crate root or `mod.rs`, else in
/// the directory named after `file`.
pub(crate) fn module_file(file: &Path, name: &str) -> Option<PathBuf> {
    let dir = file.parent()?;
    let stem = file.file_stem()?.to_str()?;
    let base = if matches!(stem, "mod" | "lib" | "main") { dir.to_path_buf() } else { dir.join(stem) };
    [base.join(format!("{name}.rs")), base.join(name).join("mod.rs")].into_iter().find(|p| p.is_file())
}

/// The definition response for LSP position (`line`, `character`) of `file` (`text`, else read from disk) when the
/// syntax answers for the token there: `Some(null)` for a keyword with nothing to go to, `None` when the index and
/// the other steps decide.
pub(crate) fn answer(
    file: &str,
    text: Option<&str>,
    line: u32,
    character: u32,
    ws: Option<&ridx_format::WorkspaceIndex>,
    conv: &mut crate::lsp::protocol::PositionConverter,
) -> Option<serde_json::Value> {
    use crate::lsp::protocol::path_to_uri;
    use serde_json::{json, Value};
    let disk;
    let text = match text {
        Some(t) => t,
        None => {
            disk = std::fs::read_to_string(file).ok()?;
            &disk
        }
    };
    let offset = byte_offset(text, line, character)?;
    let loc = |uri: String, s: (u32, u32), e: (u32, u32)| json!({ "uri": uri, "range": { "start": { "line": s.0, "character": s.1 }, "end": { "line": e.0, "character": e.1 } } });
    match definition(Path::new(file), text, offset)? {
        Target::Here(r) => Some(loc(path_to_uri(file), position(text, r.start), position(text, r.end))),
        Target::File(p) => Some(loc(path_to_uri(&p.to_string_lossy()), (0, 0), (0, 0))),
        Target::Nothing => Some(Value::Null),
        Target::Member { anchor, member, kinds } => member_location(file, text, anchor, &member, kinds, ws?, conv),
        Target::Macro { name, use_sites } => {
            let ws = ws?;
            let mut at = |path: &str, sl: u32, sc: u32, el: u32, ec: u32| {
                let (s, e) = (conv.char_col_to_utf16_col(path, sl, sc), conv.char_col_to_utf16_col(path, el, ec));
                loc(path_to_uri(path), (sl.saturating_sub(1), s), (el.saturating_sub(1), e))
            };
            // The macro a `use` of the file imports, as the index resolved that `use`.
            for site in use_sites {
                let l = position(text, site).0;
                let char_col = text[..site].rsplit('\n').next().map_or(0, |s| s.chars().count()) as u32 + 1;
                if let Some(def) = ws.find_definition_at(file, l + 1, char_col).filter(|d| d.kind == ridx_format::DefKind::Macro && d.name == name) {
                    let p = super::type_definition_parse::on_disk(def.resolved_path());
                    let ps = p.to_string_lossy().into_owned();
                    return Some(at(&ps, def.start_line, def.start_col, def.end_line, def.end_col));
                }
            }
            // Else a `macro_rules!` of that name the index holds: the workspace's own before a dependency's.
            let mut syms: Vec<_> = ws.search_symbols(&name).into_iter().filter(|s| s.kind == ridx_format::DefKind::Macro && s.name == name).collect();
            syms.sort_by_key(|s| !ws.workspace_root().is_some_and(|r| symbol_file(s).starts_with(r)));
            let s = syms.first()?;
            let p = symbol_file(s);
            let ps = p.to_string_lossy().into_owned();
            Some(at(&ps, s.span.start_line(), s.span.start_col(), s.span.end_line(), s.span.end_col()))
        }
    }
}

/// The file on disk of a symbol the index holds. A symbol of the standard library is recorded relative to the
/// sysroot's sources (`library/core/src/macros/mod.rs`), which a symbol search joins to the workspace's root: in Zed
/// on BTCR `include!` went to `BTCR/library/core/…`, no file.
fn symbol_file(s: &ridx_format::workspace::WorkspaceSymbol) -> PathBuf {
    let relative = Path::new(s.file);
    let path = if relative.is_relative() && relative.starts_with("library/") { relative.to_path_buf() } else { s.resolved_path() };
    super::type_definition_parse::on_disk(path)
}

/// The location of `member` of what the name at byte `anchor` of `file` resolves to in the index: a field of a struct
/// or variant, an item of a trait.
pub(crate) fn member_location(
    file: &str,
    text: &str,
    anchor: usize,
    member: &str,
    kinds: &[ridx_format::DefKind],
    ws: &ridx_format::WorkspaceIndex,
    conv: &mut crate::lsp::protocol::PositionConverter,
) -> Option<serde_json::Value> {
    let line = position(text, anchor).0 + 1;
    let char_col = text[..anchor].rsplit('\n').next().map_or(0, |s| s.chars().count()) as u32 + 1;
    let owner = ws.find_definition_at(file, line, char_col)?;
    let full = format!("{}::{member}", owner.path);
    let same_path = |p: &str| p == full || p.ends_with(&format!("::{full}")) || full.ends_with(&format!("::{p}"));
    let sym = ws.search_symbols(member).into_iter().find(|s| s.name == member && kinds.contains(&s.kind) && same_path(s.path))?;
    let p = symbol_file(&sym);
    let ps = p.to_string_lossy().into_owned();
    let (sl, el) = (sym.span.start_line(), sym.span.end_line());
    let (sc, ec) = (conv.char_col_to_utf16_col(&ps, sl, sym.span.start_col()), conv.char_col_to_utf16_col(&ps, el, sym.span.end_col()));
    Some(
        serde_json::json!({ "uri": crate::lsp::protocol::path_to_uri(&ps), "range": { "start": { "line": sl.saturating_sub(1), "character": sc }, "end": { "line": el.saturating_sub(1), "character": ec } } }),
    )
}

/// `found`, the definition answer at the request's position, with the field when the name there is a shorthand field
/// (`Foo { x }` names both the field and the binding, and rust-analyzer gives both; the binding came alone).
pub(crate) fn with_shorthand_field(
    found: serde_json::Value,
    msg: &serde_json::Value,
    ws: Option<&ridx_format::WorkspaceIndex>,
    overlay_mgr: &crate::overlay::OverlayManager,
    conv: &mut crate::lsp::protocol::PositionConverter,
) -> serde_json::Value {
    let file = crate::lsp::protocol::uri_to_path(msg["params"]["textDocument"]["uri"].as_str().unwrap_or(""));
    let (line, character) = (msg["params"]["position"]["line"].as_u64().unwrap_or(0) as u32, msg["params"]["position"]["character"].as_u64().unwrap_or(0) as u32);
    let field = ws.and_then(|ws| {
        let text = overlay_mgr.get(&file).map(|o| o.text.clone()).or_else(|| std::fs::read_to_string(&file).ok())?;
        let (anchor, name) = shorthand(&text, byte_offset(&text, line, character)?)?;
        member_location(&file, &text, anchor, &name, &[ridx_format::DefKind::Field], ws, conv)
    });
    match field {
        Some(f) if f != found && !found.is_null() => serde_json::json!([f, found]),
        Some(f) => f,
        None => found,
    }
}

/// The location of the binding a pattern introduces at LSP position (`line`, `character`) of `file` (`text`).
pub(crate) fn binding(file: &str, text: &str, line: u32, character: u32) -> Option<serde_json::Value> {
    let tree = parse(text);
    let i = token_index(&tree, byte_offset(text, line, character)?)?;
    let tok = tree.tokens[i];
    let node = tree.covering(tok.start as usize);
    let is_binding = matches!(tok.kind, T::Ident | T::RawIdent) && tree.kind(node) == K::Name && tree.parent(node).is_some_and(|p| tree.kind(p) == K::IdentPat);
    is_binding.then(|| {
        let (s, e) = (position(text, tok.start as usize), position(text, tok.end as usize));
        serde_json::json!({ "uri": crate::lsp::protocol::path_to_uri(file), "range": { "start": { "line": s.0, "character": s.1 }, "end": { "line": e.0, "character": e.1 } } })
    })
}

/// The byte offset of LSP position (`line`, UTF-16 `character`), both 0-based, in `text`.
pub(crate) fn byte_offset(text: &str, line: u32, character: u32) -> Option<usize> {
    let start = if line == 0 { 0 } else { text.match_indices('\n').nth(line as usize - 1)?.0 + 1 };
    let mut units = 0;
    for (i, c) in text[start..].char_indices() {
        if units >= character as usize || c == '\n' {
            return Some(start + i);
        }
        units += c.len_utf16();
    }
    Some(text.len())
}

/// The LSP position (0-based line, UTF-16 character) of byte `offset` of `text`.
pub(crate) fn position(text: &str, offset: usize) -> (u32, u32) {
    let before = &text[..offset.min(text.len())];
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    (before.matches('\n').count() as u32, before[line_start..].encode_utf16().count() as u32)
}

#[cfg(test)]
#[path = "syntactic_tests.rs"]
mod tests;
