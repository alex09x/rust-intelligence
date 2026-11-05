/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Hover in rust-analyzer's layout, built from the item's source and the index: the path of the module or type
//! that holds the item, its declaration, then its documentation with intra-doc links resolved. A trait impl method
//! without docs of its own shows the trait method's docs.

use super::hover_decl::{header_end, line_start, render_adt, render_field, render_variant, short_type, skip_trivia};
use super::hover_doc;
use super::hover_generics::{generic_impl_header, with_impl_bounds, with_where_layout};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, ResolvedDefinition, WorkspaceIndex};

/// Crates whose documentation lives on doc.rust-lang.org.
const STD_CRATES: [&str; 5] = ["std", "core", "alloc", "proc_macro", "test"];

/// The file defining `d` on disk; a standard library file (`library/…`) is found in the toolchain's `rust-src`.
pub(crate) fn def_file(d: &ResolvedDefinition<'_>) -> std::path::PathBuf {
    let path = d.resolved_path();
    if !path.exists() && path.starts_with("library/") {
        if let Some(root) = crate::lsp::navigation::type_definition_parse::sysroot_rust_src() {
            return root.join(&path);
        }
    }
    path
}

/// Source text of the file that defines `d`: the editor buffer when open, the file on disk otherwise.
pub(crate) fn def_source(d: &ResolvedDefinition<'_>, overlay_mgr: Option<&OverlayManager>) -> Option<String> {
    let path = def_file(d);
    let path = path.to_str()?;
    if let Some(ov) = overlay_mgr.and_then(|m| m.get(path)) {
        return Some(ov.text.clone());
    }
    std::fs::read_to_string(path).ok()
}

/// `path` without generic arguments: `std::vec::Vec<T>::new` → `std::vec::Vec::new`.
fn without_generics(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut depth = 0;
    for c in path.chars() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// The crate-qualified form of an index path of crate `crate_name`.
fn qualified(path: &str, crate_name: &str) -> String {
    let path = without_generics(path);
    if crate_name.is_empty() || path == crate_name || path.starts_with(&format!("{crate_name}::")) || STD_CRATES.iter().any(|c| path.starts_with(&format!("{c}::"))) {
        path
    } else {
        format!("{crate_name}::{path}")
    }
}

/// For `<Type as Trait>::item`: the type and trait paths and the item name.
fn split_qualified_self(path: &str) -> Option<(&str, &str, &str)> {
    let inner = path.strip_prefix('<')?;
    let (head, item) = inner.rsplit_once(">::")?;
    let (ty, tr) = head.split_once(" as ")?;
    Some((ty, tr, item))
}

fn crate_of<'a>(d: &ResolvedDefinition<'_>, ws: &'a WorkspaceIndex) -> (&'a str, &'a str) {
    d.reader_idx.and_then(|i| ws.readers().get(i)).map_or(("", ""), |r| (r.crate_name(), r.crate_version()))
}

/// The full path of `d`, the `<impl …>` segments and generic arguments dropped; for a trait impl item, the
/// implementing type's path followed by the item name.
fn full_path(d: &ResolvedDefinition<'_>, ws: &WorkspaceIndex) -> String {
    let (cname, _) = crate_of(d, ws);
    let p = match split_qualified_self(d.path) {
        Some((ty, _, item)) => format!("{}::{item}", qualified(ty, cname)),
        None => qualified(d.path, cname),
    };
    p.split("::").filter(|s| !s.is_empty() && !s.starts_with('{') && !s.contains(' ')).collect::<Vec<_>>().join("::")
}

/// The module or type line rust-analyzer shows above the declaration.
fn container_line(d: &ResolvedDefinition<'_>, ws: &WorkspaceIndex) -> Option<String> {
    if d.kind == DefKind::Local {
        return None;
    }
    let full = full_path(d, ws);
    let (container, _) = full.rsplit_once("::")?;
    Some(container.to_string())
}

/// The declaration block for `d` from its source `src`; `None` when the source does not hold the expected item.
fn declaration(d: &ResolvedDefinition<'_>, src: &str, inferred: Option<&str>) -> Option<String> {
    let at = skip_trivia(src, line_start(src, d.start_line)?);
    let decl = match d.kind {
        DefKind::Struct | DefKind::Union => render_adt(src, at, false)?,
        DefKind::Enum => render_adt(src, at, true)?,
        DefKind::Variant | DefKind::VariantCtor => render_variant(src, at)?,
        DefKind::Field => render_field(src, at)?,
        DefKind::Fn | DefKind::AssocFn => {
            let (end, _) = header_end(src, at);
            let header = with_where_layout(&src[at..end]);
            match generic_impl_header(src, at) {
                Some((imp, bounds)) if d.kind == DefKind::AssocFn => format!("{imp}\n{}", with_impl_bounds(header, &bounds)),
                _ => header,
            }
        }
        DefKind::Trait | DefKind::TyAlias | DefKind::AssocTy | DefKind::Const | DefKind::Static | DefKind::AssocConst | DefKind::TraitAlias => {
            let (end, _) = header_end(src, at);
            with_where_layout(&src[at..end])
        }
        DefKind::Local => return local_binding(d, src, inferred?),
        _ => return None,
    };
    let names_it = decl.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == d.name.trim_start_matches("r#"));
    names_it.then_some(decl)
}

/// `let [mut] name: Type` for a `let` binding, `name: Type` for a parameter or pattern binding.
fn local_binding(d: &ResolvedDefinition<'_>, src: &str, ty: &str) -> Option<String> {
    let ls = line_start(src, d.start_line)?;
    let line = &src[ls..src[ls..].find('\n').map_or(src.len(), |e| ls + e)];
    binding_text(line, d.start_col, d.name, ty)
}

/// The hover for the binding named at 1-based char column `col` of `line` when the index has its type but no
/// definition record (the `let` itself): `None` unless the identifier there is a `let` binding.
pub(crate) fn let_binding_hover(line: &str, col: u32, ty: &str) -> Option<String> {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let at = (col.saturating_sub(1) as usize).min(chars.len().checked_sub(1)?);
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    if !is_ident(chars[at].1) {
        return None;
    }
    let start = chars[..at].iter().rposition(|&(_, c)| !is_ident(c)).map_or(0, |p| p + 1);
    let end = chars[at..].iter().position(|&(_, c)| !is_ident(c)).map_or(chars.len(), |p| at + p);
    let (b0, b1) = (chars[start].0, chars.get(end).map_or(line.len(), |&(b, _)| b));
    let text = binding_text(line, start as u32 + 1, &line[b0..b1], ty)?;
    text.starts_with("let ").then(|| format!("```rust\n{text}\n```"))
}

fn binding_text(line: &str, start_col: u32, name: &str, ty: &str) -> Option<String> {
    let col = line.char_indices().nth(start_col.saturating_sub(1) as usize).map_or(line.len(), |(i, _)| i);
    let d_name = name;
    let before = line[..col].trim_end();
    let (is_mut, before) = match before.strip_suffix("mut") {
        Some(rest) if rest.is_empty() || rest.ends_with([' ', '(', ',']) => (true, rest.trim_end()),
        _ => (false, before),
    };
    let is_let = before == "let" || before.ends_with(" let") || before.ends_with("\tlet");
    let mutability = if is_mut { "mut " } else { "" };
    let ty = short_type(ty);
    Some(if is_let { format!("let {mutability}{d_name}: {ty}") } else { format!("{mutability}{d_name}: {ty}") })
}

/// The documentation page of `d`, as rust-analyzer links it.
fn doc_url(d: &ResolvedDefinition<'_>, ws: &WorkspaceIndex) -> Option<String> {
    let (cname, version) = crate_of(d, ws);
    let full = full_path(d, ws);
    let segs: Vec<&str> = full.split("::").collect();
    let krate = *segs.first()?;
    let base = if STD_CRATES.contains(&krate) {
        format!("https://doc.rust-lang.org/stable/{krate}/")
    } else {
        let version = if version.is_empty() || version == "0.0.0" { "latest" } else { version };
        format!("https://docs.rs/{}/{version}/{krate}/", if cname.is_empty() { krate } else { cname })
    };
    let page = |segs: &[&str], kind: DefKind| -> Option<String> {
        let (name, dirs) = segs.split_last()?;
        let dir = if dirs.len() > 1 { format!("{}/", dirs[1..].join("/")) } else { String::new() };
        let prefix = match kind {
            DefKind::Mod => return Some(format!("{base}{dir}{name}/index.html")),
            DefKind::Struct => "struct",
            DefKind::Enum => "enum",
            DefKind::Union => "union",
            DefKind::Trait => "trait",
            DefKind::TyAlias => "type",
            DefKind::Fn => "fn",
            DefKind::Const => "constant",
            DefKind::Static => "static",
            DefKind::Macro => "macro",
            _ => return None,
        };
        Some(format!("{base}{dir}{prefix}.{name}.html"))
    };
    let anchor = match d.kind {
        DefKind::AssocFn => Some("method"),
        DefKind::Field => Some("structfield"),
        DefKind::Variant | DefKind::VariantCtor => Some("variant"),
        DefKind::AssocConst => Some("associatedconstant"),
        DefKind::AssocTy => Some("associatedtype"),
        _ => None,
    };
    if d.kind == DefKind::Mod && segs.len() == 1 {
        return Some(format!("{base}index.html"));
    }
    match anchor {
        None => page(&segs, d.kind),
        Some(anchor) => {
            let (name, parent) = segs.split_last()?;
            let parent_def = ws.find_definition_by_path(&parent.join("::"));
            let parent_kind = parent_def.as_ref().map_or(DefKind::Struct, |p| p.kind);
            let anchor = if anchor == "method" && parent_kind == DefKind::Trait && is_required_method(d) { "tymethod" } else { anchor };
            Some(format!("{}#{anchor}.{name}", page(parent, parent_kind)?))
        }
    }
}

/// Whether the trait method `d` has no default body (its declaration ends with `;`).
fn is_required_method(d: &ResolvedDefinition<'_>) -> bool {
    let Some(src) = def_source(d, None) else { return false };
    let Some(at) = line_start(&src, d.start_line).map(|l| skip_trivia(&src, l)) else { return false };
    matches!(header_end(&src, at), (_, Some(b';')))
}

/// Resolves an intra-doc link written in the docs of `d` to a documentation URL.
fn resolve_link(d: &ResolvedDefinition<'_>, ws: &WorkspaceIndex, src: &str, target: &str) -> Option<String> {
    if let Some(anchor) = target.strip_prefix('#') {
        return Some(format!("{}#{anchor}", doc_url(d, ws)?.split('#').next()?));
    }
    if target.contains(".html") {
        return join_url(&doc_url(d, ws)?, target);
    }
    let link = hover_doc::link_target(target)?;
    if (link.wants_primitive || !link.path.contains("::")) && hover_doc::is_primitive(&link.path) {
        return Some(format!("https://doc.rust-lang.org/nightly/core/primitive.{}.html", link.path));
    }
    let path = link.path;
    let full = full_path(d, ws);
    let (cname, _) = crate_of(d, ws);
    let krate = full.split("::").next().unwrap_or(cname).to_string();
    let module = match d.kind {
        DefKind::Mod => full.clone(),
        _ => full.rsplit_once("::").map_or(krate.clone(), |(m, _)| m.to_string()),
    };
    let mut candidates = Vec::new();
    if let Some(rest) = path.strip_prefix("crate::") {
        candidates.push(format!("{krate}::{rest}"));
    } else if let Some(rest) = path.strip_prefix("self::") {
        candidates.push(format!("{module}::{rest}"));
    } else if let Some(rest) = path.strip_prefix("super::") {
        candidates.push(format!("{}::{rest}", module.rsplit_once("::").map_or(krate.as_str(), |(p, _)| p)));
    } else if let Some(rest) = path.strip_prefix("Self::") {
        candidates.push(format!("{module}::{rest}"));
    } else {
        let mut scope = module.as_str();
        loop {
            candidates.push(format!("{scope}::{path}"));
            match scope.rsplit_once("::") {
                Some((up, _)) => scope = up,
                None => break,
            }
        }
        candidates.push(path.clone());
        if let Some(imported) = imported_path(src, &path) {
            let imported = imported.strip_prefix("crate::").map_or(imported.clone(), |rest| format!("{krate}::{rest}"));
            candidates.insert(0, imported);
        }
    }
    let find = |c: &str| ws.find_definition_by_path(c).or_else(|| ws.find_definition_by_path(c.strip_prefix(&format!("{cname}::")).unwrap_or(c)));
    candidates.iter().find_map(|c| {
        let found = find(c)?;
        if link.wants_macro && found.kind != DefKind::Macro {
            return None;
        }
        doc_url(&found, ws)
    })
}

/// The path a `use` item of `src` brings `link`'s first segment in with: `use core::ops::Index;` makes `Index::index`
/// `core::ops::Index::index`.
fn imported_path(src: &str, link: &str) -> Option<String> {
    let (first, rest) = link.split_once("::").map_or((link, ""), |(f, r)| (f, r));
    for stmt in src.split(';') {
        let stmt = stmt.trim_start();
        let Some(body) = stmt.strip_prefix("pub use ").or_else(|| stmt.strip_prefix("use ")).or_else(|| stmt.strip_prefix("pub(crate) use ")) else { continue };
        let body: String = body.split_whitespace().collect();
        let found = match body.split_once('{') {
            Some((prefix, group)) => group.trim_end_matches('}').split(',').find(|item| item.rsplit("::").next() == Some(first)).map(|item| format!("{prefix}{item}")),
            None => (body.rsplit("::").next() == Some(first)).then(|| body.clone()),
        };
        if let Some(found) = found {
            return Some(if rest.is_empty() { found } else { format!("{found}::{rest}") });
        }
    }
    None
}

/// `rel` (`../../std/string/struct.String.html`) resolved against the page `base`.
fn join_url(base: &str, rel: &str) -> Option<String> {
    let base = base.split('#').next()?;
    let (origin_and_path, _) = base.rsplit_once('/')?;
    let scheme_end = origin_and_path.find("://")? + 3;
    let host_end = origin_and_path[scheme_end..].find('/').map_or(origin_and_path.len(), |p| scheme_end + p);
    let mut segs: Vec<&str> = origin_and_path[host_end..].split('/').filter(|s| !s.is_empty()).collect();
    for part in rel.split('/') {
        match part {
            ".." => {
                segs.pop();
            }
            "." | "" => {}
            p => segs.push(p),
        }
    }
    Some(format!("{}/{}", &origin_and_path[..host_end], segs.join("/")))
}

/// Docs of `d`; for a trait impl item without its own, the docs of the trait's item.
fn docs_of(d: &ResolvedDefinition<'_>, ws: &WorkspaceIndex, overlay_mgr: Option<&OverlayManager>, conv: &mut crate::lsp::protocol::PositionConverter) -> Option<String> {
    if let Some(doc) = super::hover_enrich::extract_source_doc_comment(d, Some(ws), overlay_mgr, conv, None) {
        return Some(doc);
    }
    let (_, tr, item) = split_qualified_self(d.path)?;
    let (cname, _) = crate_of(d, ws);
    let trait_item =
        ws.find_definition_by_path(&format!("{}::{item}", without_generics(tr))).or_else(|| ws.find_definition_by_path(&format!("{}::{item}", qualified(tr, cname))))?;
    super::hover_enrich::extract_source_doc_comment(&trait_item, Some(ws), overlay_mgr, conv, None)
}

/// The hover Markdown for `d` in rust-analyzer's layout; `None` when the declaration cannot be read from source.
pub(crate) fn render_hover(
    d: &ResolvedDefinition<'_>,
    inferred: Option<&str>,
    ws: &WorkspaceIndex,
    overlay_mgr: Option<&OverlayManager>,
    conv: &mut crate::lsp::protocol::PositionConverter,
) -> Option<String> {
    if d.is_macro_expansion || d.start_line == 0 || d.file.is_empty() || d.file == "external" {
        return None;
    }
    let src = def_source(d, overlay_mgr)?;
    let decl = declaration(d, &src, inferred)?;
    let mut blocks = Vec::new();
    if let Some(container) = container_line(d, ws) {
        blocks.push(format!("```rust\n{container}\n```"));
    }
    blocks.push(format!("```rust\n{decl}\n```"));
    let mut text = blocks.join("\n\n");
    if let Some(subst) = matches!(d.kind, DefKind::Fn | DefKind::AssocFn).then(|| associated_return(&decl, inferred)).flatten() {
        text.push_str("\n\n---\n\n");
        text.push_str(&subst);
    }
    if let Some(doc) = docs_of(d, ws, overlay_mgr, conv) {
        let rendered = hover_doc::render(&doc, &|t| resolve_link(d, ws, &src, t));
        text.push_str("\n\n---\n\n");
        text.push_str(&rendered);
    }
    Some(text)
}

/// For a method returning an associated type (`-> Self::Item`), what that type is at the call, in the place
/// rust-analyzer lists generic substitutions: `` `Self::Item` = `i32` ``.
fn associated_return(decl: &str, inferred: Option<&str>) -> Option<String> {
    let ty = inferred.filter(|t| !t.starts_with("fn(") && !t.starts_with("for<") && !t.contains('{'))?;
    let ret = decl.lines().find_map(|l| l.split_once(") -> ").map(|(_, r)| r.trim()))?;
    (ret.starts_with("Self::") && !ret.contains(['<', ' '])).then(|| format!("`{ret}` = `{}`", short_type(ty)))
}

#[cfg(test)]
mod tests {
    use super::{associated_return, imported_path, join_url, let_binding_hover};

    #[test]
    fn test_associated_return_type_is_named_at_the_call() {
        assert_eq!(associated_return("fn extract(&self) -> Self::Item", Some("i32")).as_deref(), Some("`Self::Item` = `i32`"));
        assert_eq!(associated_return("fn len(&self) -> usize", Some("usize")), None);
        assert_eq!(associated_return("fn extract(&self) -> Self::Item", Some("fn(&Container) -> i32 {Container::extract}")), None);
    }

    #[test]
    fn test_imports_and_relative_links_resolve() {
        let src = "use core::ops::Index;\nuse crate::raw::{RawVec, Cap};\nfn f() {}\n";
        assert_eq!(imported_path(src, "Index").as_deref(), Some("core::ops::Index"));
        assert_eq!(imported_path(src, "RawVec::new").as_deref(), Some("crate::raw::RawVec::new"));
        assert_eq!(imported_path(src, "Missing"), None);
        let base = "https://doc.rust-lang.org/stable/core/option/enum.Option.html#method.map";
        assert_eq!(join_url(base, "../../std/string/struct.String.html").as_deref(), Some("https://doc.rust-lang.org/stable/std/string/struct.String.html"));
    }

    #[test]
    fn test_let_binding_hover_names_the_binding_with_a_short_type() {
        let line = "    let mut v: Vec<String> = Vec::new();";
        assert_eq!(let_binding_hover(line, 13, "std::vec::Vec<std::string::String>").as_deref(), Some("```rust\nlet mut v: Vec<String>\n```"));
        assert_eq!(let_binding_hover("    let c = Circle::new(2.0);", 9, "shapes::Circle").as_deref(), Some("```rust\nlet c: Circle\n```"));
        assert_eq!(let_binding_hover("    outlet(c);", 12, "u8"), None, "no let binding");
    }
}
