/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Path qualification for `Implement default members`: a name a default body uses but the impl's module does not
//! see (`fmt::Formatter`, `Unexpected::Bool`, `PhantomData`) is replaced by the path of what the index resolved it
//! to in the trait's file, as rust-analyzer's path transform does.

use super::action_parse::{is_ident_byte, mask_comments_and_strings};
use super::action_parse_decl::next_ident_token;
use ridx_format::{DefKind, WorkspaceIndex};
use std::collections::HashSet;
use std::path::Path;

/// Where a member body sits: the trait file's text and path, and the body's byte offset in it.
pub(crate) struct MemberOrigin<'a> {
    pub trait_src: &'a str,
    pub trait_file: &'a str,
    pub start: usize,
}

/// Public modules of `core`/`std` whose items keep their definition path (`core::fmt::Formatter`); other sysroot
/// definition paths go through private modules (`core::io::io_slice::IoSlice`).
const PUBLIC_STD_MODULES: &[&str] = &[
    "fmt", "marker", "ops", "cmp", "convert", "iter", "option", "result", "mem", "ptr", "str", "slice", "num", "hash", "any", "borrow", "clone", "default", "error", "future",
    "task", "pin", "cell", "time", "char",
];

/// The path to write for an item defined in crate `def_crate` with index path `path` (crate-relative for the crate's
/// own items), seen from crate `dest_crate`; `None` when no written path is known to reach it.
fn path_from(dest_crate: &str, def_crate: &str, path: &str) -> Option<String> {
    let (root, _) = path.split_once("::").unwrap_or((path, ""));
    let full = match root {
        "core" | "std" => {
            let segs: Vec<&str> = path.split("::").collect();
            return (segs.len() == 2 || (segs.len() == 3 && PUBLIC_STD_MODULES.contains(&segs[1]))).then(|| path.to_string());
        }
        "alloc" => return None,
        _ if root == def_crate || def_crate.is_empty() => path.to_string(),
        _ => format!("{def_crate}::{path}"),
    };
    match full.split_once("::") {
        Some((r, rest)) if r == dest_crate => Some(format!("crate::{rest}")),
        Some(_) => Some(full),
        None => None,
    }
}

/// A path that reaches `full` (`krate::m1::m2::Item`) from another crate, read from the defining crate's sources rooted
/// at `root_file`: the crate root's `pub use` re-export of `Item` when it has one, else `full` itself when every
/// module on the way is declared `pub mod`; `None` when neither can be shown.
fn public_path(root_file: &Path, full: &str) -> Option<String> {
    let segs: Vec<&str> = full.split("::").collect();
    let (krate, item) = (*segs.first()?, *segs.last()?);
    if segs.len() <= 2 {
        return Some(full.to_string());
    }
    let declares = |text: &str, kw: &str, name: &str| {
        let masked = mask_comments_and_strings(text);
        masked.match_indices(kw).any(|(p, _)| {
            let rest = &masked[p + kw.len()..];
            let b = masked.as_bytes();
            (p == 0 || !is_ident_byte(b[p - 1]))
                && (kw != "pub mod " || rest.starts_with(name) && !rest[name.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_'))
                && (kw != "pub use " || rest.split(';').next().is_some_and(|u| u.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == name)))
        })
    };
    let root_text = std::fs::read_to_string(root_file).ok()?;
    if declares(&root_text, "pub use ", item) {
        return Some(format!("{krate}::{item}"));
    }
    let (mut dir, mut text) = (root_file.parent()?.to_path_buf(), root_text);
    for m in &segs[1..segs.len() - 1] {
        if !declares(&text, "pub mod ", m) {
            return None;
        }
        let (file, sub) = [(dir.join(format!("{m}.rs")), dir.join(m)), (dir.join(m).join("mod.rs"), dir.join(m))].into_iter().find(|(f, _)| f.is_file())?;
        text = std::fs::read_to_string(&file).ok()?;
        dir = sub;
    }
    Some(full.to_string())
}

/// Rewrites `src` (a member taken from `origin`) so names outside `visible` resolve as they did in the trait's file.
/// Returns the new text and the crate roots it introduced (`core`, other crates), which the safety check must accept.
pub(crate) fn qualify_member_paths(src: &str, origin: &MemberOrigin<'_>, ws: &WorkspaceIndex, dest_crate: &str, visible: &dyn Fn(&str) -> bool) -> (String, HashSet<String>) {
    let masked = mask_comments_and_strings(src);
    let b = masked.as_bytes();
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut roots = HashSet::new();
    let mut i = 0usize;
    while i < b.len() {
        if !(b[i].is_ascii_alphabetic() || b[i] == b'_') || (i > 0 && (is_ident_byte(b[i - 1]) || b[i - 1] == b'\'')) {
            i += 1;
            continue;
        }
        let Some((tok, end)) = next_ident_token(&masked, i) else {
            i += 1;
            continue;
        };
        let before = masked[..i].trim_end();
        let after = masked[end..].trim_start();
        let is_path_head = after.starts_with("::");
        let candidate = !before.ends_with("::")
            && !before.ends_with('.')
            && !after.starts_with('!')
            // `name: value` / `name: Type` introduces a field or binding, not a path.
            && !(after.starts_with(':') && !is_path_head)
            && (is_path_head || tok.starts_with(|c: char| c.is_ascii_uppercase()))
            && !visible(tok);
        if candidate {
            let abs = origin.start + i;
            let line_start = origin.trait_src[..abs].rfind('\n').map_or(0, |p| p + 1);
            let line = origin.trait_src[..line_start].matches('\n').count() as u32 + 1;
            let col = origin.trait_src[line_start..abs].chars().count() as u32 + 1;
            if let Some(refs) = ws.find_direct_references_at(origin.trait_file, line, col) {
                let kind_ok = !matches!(refs.target_kind, DefKind::Local | DefKind::TyParam | DefKind::Field | DefKind::Use | DefKind::Macro);
                let named = refs.target_path.rsplit("::").next() == Some(tok.strip_prefix("r#").unwrap_or(tok));
                let def_reader = refs.target_reader_idx.and_then(|i| ws.readers().get(i));
                let def_crate = def_reader.map(|r| r.crate_name().replace('-', "_")).unwrap_or_default();
                // Another crate's item must be reached through public modules or a root re-export.
                let path = path_from(dest_crate, &def_crate, refs.target_path).and_then(|p| {
                    let root = p.split("::").next().unwrap_or_default();
                    if matches!(root, "crate" | "core" | "std") {
                        return Some(p);
                    }
                    let root_file = def_reader.map(|r| r.get_file(0)).filter(|f| !f.is_empty())?;
                    let root_file = refs.target_resolved_path().ancestors().find(|a| a.join(root_file).is_file()).map(|a| a.join(root_file)).unwrap_or_else(|| root_file.into());
                    public_path(&root_file, &p)
                });
                if kind_ok && named {
                    if let Some(p) = path {
                        roots.insert(p.split("::").next().unwrap_or_default().to_string());
                        edits.push((i, end, p));
                    }
                }
            }
        }
        i = end;
    }
    let mut out = src.to_string();
    for (lo, hi, text) in edits.into_iter().rev() {
        out.replace_range(lo..hi, &text);
    }
    (out, roots)
}

/// Names that start a path in `src` (an impl header): `E: de::Error` makes `E` and `de` visible, not `Error`.
pub(crate) fn path_heads(src: &str) -> HashSet<String> {
    let masked = mask_comments_and_strings(src);
    let b = masked.as_bytes();
    let mut out = HashSet::new();
    let mut i = 0;
    while i < b.len() {
        if (b[i].is_ascii_alphabetic() || b[i] == b'_') && (i == 0 || !is_ident_byte(b[i - 1])) {
            if let Some((tok, end)) = next_ident_token(&masked, i) {
                if !masked[..i].trim_end().ends_with("::") {
                    out.insert(tok.to_string());
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Whether the member body names a prelude item that `scope` (the impl module's own items and imports) shadows.
pub(crate) fn shadows_prelude_name(body_masked: &str, scope: &HashSet<String>) -> bool {
    const PRELUDE: &[&str] =
        &["Result", "Option", "Some", "None", "Ok", "Err", "Box", "Vec", "String", "ToString", "ToOwned", "Default", "Iterator", "IntoIterator", "From", "Into"];
    PRELUDE.iter().any(|p| scope.contains(*p) && super::action_parse_items::contains_word(body_masked, p))
}

/// Whether `cfg_attr(predicate, attrs…)` (the attribute's inner text) applies only `doc(…)` attributes.
pub(crate) fn cfg_attr_only_documents(inner_masked: &str) -> bool {
    let Some(args) = inner_masked.trim().strip_prefix("cfg_attr").map(str::trim).and_then(|a| a.strip_prefix('(')).and_then(|a| a.strip_suffix(')')) else {
        return false;
    };
    let parts = super::action_generics_sub::split_top_level_commas(args);
    parts.len() >= 2 && parts[1..].iter().all(|p| p.trim().starts_with("doc") && p.trim()[3..].trim_start().starts_with('('))
}

#[cfg(test)]
mod tests {
    use super::{cfg_attr_only_documents, path_from, path_heads, public_path, shadows_prelude_name};

    #[test]
    fn test_public_path_through_pub_modules_or_root_reexport() {
        let dir = std::env::temp_dir().join(format!("ridx_pubpath_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src/de")).unwrap();
        std::fs::write(dir.join("src/lib.rs"), "mod sink;\npub mod de;\npub use crate::sink::{Sink, SinkError};\n").unwrap();
        std::fs::write(dir.join("src/sink.rs"), "pub struct Sink;\npub struct SinkError;\npub struct Hidden;\n").unwrap();
        std::fs::write(dir.join("src/de/mod.rs"), "pub struct Unexpected;\n").unwrap();
        let root = dir.join("src/lib.rs");
        assert_eq!(public_path(&root, "searcher::sink::SinkError").as_deref(), Some("searcher::SinkError"), "root re-export");
        assert_eq!(public_path(&root, "searcher::de::Unexpected").as_deref(), Some("searcher::de::Unexpected"), "pub module chain");
        assert_eq!(public_path(&root, "searcher::sink::Hidden"), None, "private module, no re-export");
        let _ = std::fs::remove_dir_all(&dir);
    }
    use std::collections::HashSet;

    #[test]
    fn test_paths_written_from_the_impl_crate() {
        assert_eq!(path_from("serde_core", "serde_core", "serde_core::de::Unexpected").as_deref(), Some("crate::de::Unexpected"));
        assert_eq!(path_from("serde", "serde_core", "serde_core::de::Unexpected").as_deref(), Some("serde_core::de::Unexpected"));
        // The index names a crate's own items crate-relative.
        assert_eq!(path_from("tokio", "tokio", "runtime::task::Notified").as_deref(), Some("crate::runtime::task::Notified"));
        assert_eq!(path_from("tokio_util", "tokio", "io::AsyncWrite").as_deref(), Some("tokio::io::AsyncWrite"));
        assert_eq!(path_from("serde", "core", "core::fmt").as_deref(), Some("core::fmt"));
        assert_eq!(path_from("serde", "core", "core::marker::PhantomData").as_deref(), Some("core::marker::PhantomData"));
        assert_eq!(path_from("tokio", "core", "core::io::io_slice::IoSlice"), None);
        assert_eq!(path_from("serde", "alloc", "alloc::string::String"), None);
    }

    #[test]
    fn test_prelude_shadowing_and_doc_only_cfg_attr() {
        let scope: HashSet<String> = ["Result".to_string()].into_iter().collect();
        assert!(shadows_prelude_name("fn f() -> Result<(), E> { Ok(()) }", &scope));
        assert!(!shadows_prelude_name("fn f() -> Option<u8> { None }", &scope));
        assert!(cfg_attr_only_documents("cfg_attr(docsrs, doc(cfg(feature = \"std\")))"));
        assert!(!cfg_attr_only_documents("cfg_attr(feature = \"x\", inline)"));
        let heads = path_heads("impl<'de, E> Deserializer<'de> for Content<E> where E: de::Error");
        assert!(heads.contains("de") && heads.contains("E") && heads.contains("Deserializer") && !heads.contains("Error"), "{heads:?}");
    }
}
