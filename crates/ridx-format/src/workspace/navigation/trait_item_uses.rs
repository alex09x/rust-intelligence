/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Uses of a trait item through its implementations: the compiler resolves `v.into_iter()` on a `Vec` to
//! `<Vec<T> as IntoIterator>::into_iter`, and rust-analyzer counts such a use as a use of the trait item
//! (`IntoIterator::into_iter`) when references to the trait item are asked for.

use super::*;
use crate::format::RefEntry;
use std::collections::HashSet;

/// `(trait, item)` of a qualified path `<T as Trait<..>>::item` or of an impl item path `module::<impl Trait<..> for T>::item`,
/// the trait without generic arguments.
pub(super) fn qualified_trait_item(path: &str) -> Option<(&str, &str)> {
    if !path.starts_with('<') {
        let at = path.find("::<impl ")?;
        return qualified_trait_item(&path[at + 2..]);
    }
    let (rest, sep) = match path.strip_prefix("<impl ") {
        Some(r) => (r, " for "),
        None => (&path[1..], " as "),
    };
    let mut as_at = (sep == " for ").then_some(0);
    let mut trait_end = None;
    let (mut depth, mut close) = (1usize, None);
    for (i, c) in rest.char_indices() {
        match c {
            '<' => depth += 1,
            '>' if i > 0 && rest.as_bytes()[i - 1] == b'-' => {}
            '>' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            ' ' if depth == 1 && sep == " as " && as_at.is_none() && rest[i..].starts_with(" as ") => as_at = Some(i + 4),
            ' ' if depth == 1 && sep == " for " && trait_end.is_none() && rest[i..].starts_with(" for ") => trait_end = Some(i),
            _ => {}
        }
    }
    let (as_at, close) = (as_at?, close?);
    let item = rest[close + 1..].strip_prefix("::")?;
    if item.contains("::") || item.is_empty() {
        return None;
    }
    let tr = &rest[as_at..if sep == " for " { trait_end? } else { close }];
    Some((tr.find('<').map_or(tr, |g| &tr[..g]).trim(), item))
}

impl WorkspaceIndex {
    /// Adds to `out_refs` the uses of trait item `def` that the compiler resolved to an implementation of it.
    pub(super) fn expand_trait_item_impl_uses<'a>(&'a self, def: &ResolvedDefinition<'a>, out_refs: &mut Vec<WorkspaceReference<'a>>) {
        if !matches!(def.kind, DefKind::AssocFn | DefKind::AssocConst | DefKind::AssocTy) || def.path.starts_with('<') {
            return;
        }
        let (Some(t_idx), Some((parent, item))) = (def.reader_idx, def.path.rsplit_once("::")) else { return };
        let tr = &self.readers[t_idx];
        let cname = tr.crate_name();
        let local = parent.strip_prefix(&format!("{cname}::")).unwrap_or(parent);
        let global = format!("{cname}::{local}");
        if ![parent, local, global.as_str()].iter().any(|&p| super::decl_lookup::reader_has_def_kind_at_path(tr, p, DefKind::Trait)) {
            return;
        }
        // Code a macro generates (a derive's `Clone::clone(&self.x)`) uses the item where no name of it is written.
        out_refs.retain(|r| !r.is_macro_expansion);
        let mut traits: Vec<String> = vec![parent.to_string(), global.clone()];
        traits.extend(super::std_map::std_path_candidates(&global));
        let mut seen: HashSet<(&str, u32, u32)> = out_refs.iter().map(|r| (r.file, r.span.start_line(), r.span.start_col())).collect();
        for (i, reader) in self.readers.iter().enumerate() {
            if self.is_reader_external(i) {
                continue;
            }
            let rc = reader.crate_name();
            let Some(cell) = self.unit_match.qualified_item_targets.get(i) else { continue };
            let Some(targets) = cell.get_or_init(|| reader.qualified_item_targets()).get(item) else { continue };
            let found: Vec<&RefEntry> = targets
                .iter()
                .map(|&off| reader.get_str(off))
                .filter(|p| qualified_trait_item(p).is_some_and(|(t, it)| it == item && (traits.iter().any(|x| x == t) || (rc == cname && t == local))))
                .flat_map(|p| reader.find_references(p))
                .collect();
            let source_root = self.source_roots.get(i).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref());
            let unit_key = self.unit_keys.get(i).and_then(|u| u.as_deref());
            for r in found {
                let span = r.span();
                let file = reader.get_file(span.file_id());
                if !r.is_macro_expansion() && seen.insert((file, span.start_line(), span.start_col())) {
                    out_refs.push(WorkspaceReference {
                        crate_name: rc,
                        file,
                        span,
                        kind: DefKind::from(r.kind),
                        is_write: r.is_write != 0,
                        is_macro_expansion: r.is_macro_expansion(),
                        source_root,
                        unit_key,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::qualified_trait_item;

    #[test]
    fn splits_the_trait_and_item_of_a_qualified_path() {
        assert_eq!(qualified_trait_item("<alloc::vec::Vec<T, A> as core::iter::IntoIterator>::into_iter"), Some(("core::iter::IntoIterator", "into_iter")));
        assert_eq!(qualified_trait_item("<X as core::convert::From<<Y as Z>::A>>::from"), Some(("core::convert::From", "from")));
        assert_eq!(qualified_trait_item("<fn() -> u8 as Tr>::f"), Some(("Tr", "f")));
        assert_eq!(qualified_trait_item("<X as Tr>::Assoc::f"), None);
        assert_eq!(qualified_trait_item("core::clone::Clone::clone"), None);
        assert_eq!(qualified_trait_item("ser::impls::<impl ser::Serialize for alloc::vec::Vec<T>>::serialize"), Some(("ser::Serialize", "serialize")));
        assert_eq!(qualified_trait_item("de::<impl de::Visitor<'de> for X>::visit_u8"), Some(("de::Visitor", "visit_u8")));
        assert_eq!(qualified_trait_item("m::<impl m::Foo>::new"), None);
    }
}
