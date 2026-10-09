/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Self:: qualifier resolution to enclosing impl and trait owners.

pub mod scan;
pub mod types;

pub use scan::*;
pub use types::*;

use ridx_format::{DefKind, WorkspaceIndex};
use std::collections::HashSet;

fn collect_type_owner(
    ws: &WorkspaceIndex,
    file: &str,
    type_name: &str,
    owners: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    if type_name.is_empty() {
        return;
    }
    if let Some(def) = ws.resolve_qualifier(file, type_name) {
        if seen.insert(def.path.to_string()) {
            owners.push(def.path.to_string());
        }
    }
    if seen.insert(type_name.to_string()) {
        owners.push(type_name.to_string());
    }
}

fn collect_trait_and_supertraits(
    ws: &WorkspaceIndex,
    file: &str,
    trait_name: &str,
    owners: &mut Vec<String>,
    seen: &mut HashSet<String>,
    depth: usize,
) {
    if depth > 4 || trait_name.is_empty() {
        return;
    }
    let def_opt = ws.resolve_qualifier(file, trait_name);
    let resolved_path = def_opt.as_ref().map(|d| d.path.to_string());
    let target = resolved_path.as_deref().unwrap_or(trait_name);

    if !seen.insert(target.to_string()) {
        return;
    }
    owners.push(target.to_string());
    if target != trait_name && seen.insert(trait_name.to_string()) {
        owners.push(trait_name.to_string());
    }

    if let Some(def) = def_opt {
        if def.kind == DefKind::Trait {
            if let Ok(src) = std::fs::read_to_string(def.file) {
                let start_line = def.start_line as usize;
                let header: String = src
                    .lines()
                    .skip(start_line.saturating_sub(1))
                    .take(10)
                    .take_while(|l| !l.contains('{'))
                    .collect::<Vec<_>>()
                    .join(" ");
                if let Some(colon_idx) = header.find(':') {
                    let after_colon = &header[colon_idx + 1..];
                    let super_chunk = after_colon.split("where").next().unwrap_or(after_colon);
                    for part in super_chunk.split('+') {
                        let st_clean = clean_trait_name(part);
                        if !st_clean.is_empty() {
                            collect_trait_and_supertraits(
                                ws,
                                def.file,
                                &st_clean,
                                owners,
                                seen,
                                depth + 1,
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Resolves owner candidates for `Self::` completion within the lexical scope of `file:line`.
pub fn resolve_self_owners(
    ws: &WorkspaceIndex,
    file: &str,
    line_1based: u32,
    line_slice: &str,
    overlay_text: Option<&str>,
) -> Option<Vec<String>> {
    let text = if let Some(t) = overlay_text {
        t.to_string()
    } else {
        std::fs::read_to_string(file).ok()?
    };

    let scope = find_enclosing_scope(&text, line_1based, line_slice)?;
    let mut owners = Vec::new();
    let mut seen = HashSet::new();

    match scope {
        EnclosingScope::Impl {
            trait_name,
            type_name,
        } => {
            if let Some(tname) = trait_name {
                collect_trait_and_supertraits(ws, file, &tname, &mut owners, &mut seen, 0);
            }
            collect_type_owner(ws, file, &type_name, &mut owners, &mut seen);
        }
        EnclosingScope::Trait {
            trait_name,
            supertraits,
        } => {
            collect_trait_and_supertraits(ws, file, &trait_name, &mut owners, &mut seen, 0);
            for st in supertraits {
                collect_trait_and_supertraits(ws, file, &st, &mut owners, &mut seen, 0);
            }
        }
        EnclosingScope::Other => return None,
    }

    if owners.is_empty() {
        None
    } else {
        Some(owners)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_impl_headers() {
        let h1 = "impl<I: Iterator<Item: EntityEquivalent>> Iterator for UniqueEntityIter<I>";
        assert_eq!(
            parse_header(h1),
            EnclosingScope::Impl {
                trait_name: Some("Iterator".to_string()),
                type_name: "UniqueEntityIter".to_string(),
            }
        );

        let h2 = "impl<K: EntityEquivalent + Hash, V> Slice<K, V>";
        assert_eq!(
            parse_header(h2),
            EnclosingScope::Impl {
                trait_name: None,
                type_name: "Slice".to_string(),
            }
        );

        let h3 = "unsafe impl<D: ReadOnlyQueryData + 'static, F: QueryFilter + 'static> QueryData for NestedQuery<D, F>";
        assert_eq!(
            parse_header(h3),
            EnclosingScope::Impl {
                trait_name: Some("QueryData".to_string()),
                type_name: "NestedQuery".to_string(),
            }
        );

        let h4 = "impl<T: Component> ContiguousQueryData for Ref<'_, T>";
        assert_eq!(
            parse_header(h4),
            EnclosingScope::Impl {
                trait_name: Some("ContiguousQueryData".to_string()),
                type_name: "Ref".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_trait_headers() {
        let h1 = "pub trait ContiguousQueryData: ArchetypeQueryData + IterQueryData";
        assert_eq!(
            parse_header(h1),
            EnclosingScope::Trait {
                trait_name: "ContiguousQueryData".to_string(),
                supertraits: vec!["ArchetypeQueryData".to_string(), "IterQueryData".to_string()],
            }
        );

        let h2 = "pub trait EntitySet";
        assert_eq!(
            parse_header(h2),
            EnclosingScope::Trait {
                trait_name: "EntitySet".to_string(),
                supertraits: vec![],
            }
        );
    }

    #[test]
    fn test_find_enclosing_scope_nested() {
        let code = r#"
impl MyStruct {
    pub fn do_something(&self) {
        unsafe {
            Self::
        }
    }
}
"#;
        let scope = find_enclosing_scope(code, 5, "            Self::");
        assert_eq!(
            scope,
            Some(EnclosingScope::Impl {
                trait_name: None,
                type_name: "MyStruct".to_string(),
            })
        );
    }

    #[test]
    fn test_find_enclosing_scope_signature_before_brace() {
        let code = r#"
impl<I: Iterator> Iterator for UniqueEntityIter<I> {
    type Item = I::Item;

    fn reduce<F>(self, f: F) -> Option<Self::
"#;
        let scope = find_enclosing_scope(code, 5, "    fn reduce<F>(self, f: F) -> Option<Self::");
        assert_eq!(
            scope,
            Some(EnclosingScope::Impl {
                trait_name: Some("Iterator".to_string()),
                type_name: "UniqueEntityIter".to_string(),
            })
        );
    }

    #[test]
    fn test_find_enclosing_scope_multiline_impl() {
        let code = r#"
unsafe impl<D: ReadOnlyQueryData + 'static, F: QueryFilter + 'static> QueryData
    for NestedQuery<D, F>
{
    unsafe fn fetch<'w, 's>(
        state: &'s Self::
"#;
        let scope = find_enclosing_scope(code, 6, "        state: &'s Self::");
        assert_eq!(
            scope,
            Some(EnclosingScope::Impl {
                trait_name: Some("QueryData".to_string()),
                type_name: "NestedQuery".to_string(),
            })
        );
    }

    #[test]
    fn test_find_enclosing_scope_trait_body() {
        let code = r#"
pub trait EntitySet {
    type Item;
    fn get(&self) -> Option<Self::
}
"#;
        let scope = find_enclosing_scope(code, 4, "    fn get(&self) -> Option<Self::");
        assert_eq!(
            scope,
            Some(EnclosingScope::Trait {
                trait_name: "EntitySet".to_string(),
                supertraits: vec![],
            })
        );
    }
}
