/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `w.0.name` → `w.0.N` is right only when `w.0` has no `.N` of its own: when it is the converted struct itself
//! (`BoolAttr(Attr<..>)`), or a standard pointer reaching it through `Deref` (`GroupInfo(Arc<GroupInfoInner>)`), whose
//! fields are private or named.

use super::action_struct_conv_index::byte_to_line_col;
use ridx_format::{DefKind, WorkspaceIndex};

/// Standard pointers whose fields are private or named: `.N` on one reaches its target through `Deref`.
const STD_POINTERS: &[&str] = &["Box", "Arc", "Rc", "Pin", "Cow", "Ref", "RefMut", "MutexGuard", "RwLockReadGuard", "RwLockWriteGuard"];

/// Where the tuple index of the receiver text `text` (ending right before a `.name`) starts: `w.0` → the `0`.
pub(crate) fn tuple_index_start(text: &str) -> Option<usize> {
    let t = text.trim_end();
    let digits = t.len() - t.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    (digits > 0 && t[..t.len() - digits].trim_end().ends_with('.')).then_some(t.len() - digits)
}

/// Whether the tuple field the index resolves at byte `at` of `doc` (its digit) has the type `struct_name`, maybe
/// behind references or a standard pointer.
pub(crate) fn tuple_field_reaches_struct(ws: &WorkspaceIndex, file: &str, doc: &str, starts: &[usize], at: usize, struct_name: &str) -> bool {
    let (line, col) = byte_to_line_col(doc, starts, at);
    let Some(refs) = ws.find_direct_references_at(file, line, col) else { return false };
    if refs.target_kind != DefKind::Field {
        return false;
    }
    let Ok(src) = std::fs::read_to_string(refs.target_resolved_path()) else { return false };
    let Some(decl) = src.lines().nth((refs.target_line as usize).saturating_sub(1)) else { return false };
    let decl: String = decl.chars().skip((refs.target_col as usize).saturating_sub(1)).collect();
    type_reaches(&decl, struct_name)
}

/// Whether the tuple field declaration starting `decl` (`pub Arc<Inner>,`) is `struct_name` or a standard pointer
/// to it.
fn type_reaches(decl: &str, struct_name: &str) -> bool {
    let mut t = decl.trim_start();
    if let Some(rest) = t.strip_prefix("pub") {
        let rest = rest.trim_start();
        t = match rest.strip_prefix('(') {
            Some(r) => r.split_once(')').map_or("", |(_, after)| after),
            None => rest,
        };
    }
    let mut t = t.trim_start();
    // References: `&'a mut Inner`.
    while let Some(rest) = t.strip_prefix('&') {
        let rest = rest.trim_start();
        let rest = if rest.starts_with('\'') { rest.trim_start_matches(|c: char| c == '\'' || c.is_alphanumeric() || c == '_').trim_start() } else { rest };
        t = rest.strip_prefix("mut ").unwrap_or(rest).trim_start();
    }
    let head_end = t.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':')).unwrap_or(t.len());
    let head = t[..head_end].rsplit("::").next().unwrap_or("");
    if head == struct_name {
        return true;
    }
    // A pointer whose first argument is the struct: `Arc<GroupInfoInner>`, `Box<Inner<T>>`.
    STD_POINTERS.contains(&head) && t[head_end..].strip_prefix('<').is_some_and(|args| type_reaches(args, struct_name))
}

#[cfg(test)]
mod tests {
    use super::{tuple_index_start, type_reaches};

    #[test]
    fn a_tuple_field_reaches_the_struct_directly_or_through_a_standard_pointer() {
        assert!(type_reaches("Arc<GroupInfoInner>);", "GroupInfoInner"));
        assert!(type_reaches("pub(crate) Attr<'c, ()>);", "Attr"));
        assert!(type_reaches("&'a mut Box<Inner<T>>,", "Inner"));
        assert!(!type_reaches("Wrapper<Inner>);", "Inner"));
        assert!(!type_reaches("Vec<Inner>);", "Inner"));
        assert_eq!(tuple_index_start("self.group_info.0"), Some(16));
        assert_eq!(tuple_index_start("self.group_info"), None);
    }
}
