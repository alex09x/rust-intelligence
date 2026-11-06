/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Trait scope resolution, definition categorization, and inherent-first ordering.

use ridx_format::workspace::is_prelude_trait;
use ridx_format::DefKind;
use std::collections::HashSet;

/// Categorization of candidate completion items for scope filtering and priority ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DefScopeCategory {
    Field = 0,
    Inherent = 1,
    PreludeTrait = 2,
    InScopeTrait = 3,
    OutOfScopeTrait = 4,
}

/// Finds the byte offset of `pattern` within `s` at angle bracket depth zero.
pub fn find_depth_zero(s: &str, pattern: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let pat_bytes = pattern.as_bytes();
    let mut depth = 0usize;
    for i in 0..bytes.len() {
        if bytes[i] == b'<' {
            depth += 1;
        } else if bytes[i] == b'>' {
            depth = depth.saturating_sub(1);
        } else if depth == 0 && bytes[i..].starts_with(pat_bytes) {
            return Some(i);
        }
    }
    None
}

/// Matches the outermost `<` corresponding to the trailing `>` in `header`.
pub fn find_matching_start_angle(header: &str) -> Option<usize> {
    let bytes = header.as_bytes();
    let mut depth = 0usize;
    for (i, &b) in bytes.iter().enumerate().rev() {
        if b == b'>' {
            depth += 1;
        } else if b == b'<' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// Extracts (trait_ref, self_type) from a trait method definition path if it is an impl item.
/// E.g.
/// - `<core::option::Option<T> as core::fmt::Debug>::fmt` -> `("core::fmt::Debug", "core::option::Option<T>")`
/// - `impls::_::<impl reflect::Reflect for core::option::Option<T>>::into_any` -> `("reflect::Reflect", "core::option::Option<T>")`
pub fn extract_trait_and_self_from_def_path(path: &str) -> Option<(&str, &str)> {
    let last_colon = path.rfind("::")?;
    let header = &path[..last_colon];
    if !header.ends_with('>') {
        return None;
    }

    let start_angle = find_matching_start_angle(header)?;
    let inner = &header[start_angle + 1..header.len() - 1];

    // Case 1: `<impl TraitRef for SelfType>`
    if let Some(impl_pos) = find_depth_zero(inner, "impl ") {
        let after_impl = &inner[impl_pos + 5..];
        if let Some(for_pos) = find_depth_zero(after_impl, " for ") {
            let trait_part = after_impl[..for_pos].trim();
            let self_part = after_impl[for_pos + 5..].trim();
            if !trait_part.is_empty() && !self_part.is_empty() {
                return Some((trait_part, self_part));
            }
        }
    }

    // Case 2: `<SelfType as TraitRef>`
    if let Some(as_pos) = find_depth_zero(inner, " as ") {
        let self_part = inner[..as_pos].trim();
        let trait_part = inner[as_pos + 4..].trim();
        if !trait_part.is_empty() && !self_part.is_empty() {
            return Some((trait_part, self_part));
        }
    }

    None
}

/// Extracts the underlying type name from an inherent implementation path header.
/// E.g.
/// - `"relationship::relationship_query::<impl system::query::Query<'w, 's, D, F>>"` -> `"system::query::Query<'w, 's, D, F>"`
/// - `"<alloc::boxed::Box<dyn core::any::Any>>"` -> `"alloc::boxed::Box<dyn core::any::Any>"`
/// - `"<[T]>"` -> `"[T]"`
/// - `"bevy_ecs::world::World"` -> `"bevy_ecs::world::World"`
pub fn extract_parent_type(parent: &str) -> &str {
    let p = parent.trim();
    if p.ends_with('>') {
        if let Some(start_angle) = find_matching_start_angle(p) {
            let inner = p[start_angle + 1..p.len() - 1].trim();
            if find_depth_zero(inner, " as ").is_some() {
                return p;
            }
            if let Some(after_impl) = inner.strip_prefix("impl ") {
                if find_depth_zero(after_impl, " for ").is_none() {
                    return after_impl.trim();
                }
            } else if p.starts_with('<') {
                return inner;
            }
        }
    }
    p
}

/// Extracts the trait path from a definition path if this is a trait implementation item.
pub fn extract_trait_from_def_path(path: &str) -> Option<&str> {
    extract_trait_and_self_from_def_path(path).map(|(tr, _)| tr)
}

/// Strips generic parameters from a trait path, returning the unparameterized trait path.
/// E.g. `"core::convert::AsRef<[u8]>"` -> `"core::convert::AsRef"`.
pub fn base_trait_path(trait_path: &str) -> &str {
    if let Some(pos) = trait_path.find('<') {
        trait_path[..pos].trim()
    } else {
        trait_path
    }
}

/// Strips generic parameters from a type path, returning the unparameterized base path.
/// E.g. `"core::option::Option<T>"` -> `"core::option::Option"`.
pub fn base_type_path(type_path: &str) -> &str {
    let s = type_path.trim();
    let s = if s.starts_with('<') && s.ends_with('>') && s.len() >= 2 { &s[1..s.len() - 1] } else { s };
    if let Some(pos) = s.find('<') {
        s[..pos].trim_end_matches(':').trim()
    } else {
        s
    }
}

fn strip_ref_and_brackets(ty: &str) -> &str {
    let mut s = ty.trim();
    if s.starts_with('<') && s.ends_with('>') && s.len() >= 2 {
        s = s[1..s.len() - 1].trim();
    }
    if let Some(rest) = s.strip_prefix('&') {
        s = rest.trim();
        if s.starts_with('\'') {
            if let Some(pos) = s.find(|c: char| c.is_whitespace() || c == '[') {
                s = s[pos..].trim();
            }
        }
    }
    if let Some(rest) = s.strip_prefix("mut") {
        if rest.starts_with(' ') || rest.starts_with('\t') {
            s = rest.trim();
        }
    }
    s
}

pub fn is_slice_type(ty: &str) -> bool {
    let s = strip_ref_and_brackets(ty);
    s == "slice" || (s.starts_with('[') && s.ends_with(']') && !s.contains(';'))
}

pub fn is_array_type(ty: &str) -> bool {
    let s = strip_ref_and_brackets(ty);
    s == "array" || (s.starts_with('[') && s.ends_with(']') && s.contains(';'))
}

pub fn is_pointer_type(ty: &str) -> bool {
    let s = strip_ref_and_brackets(ty);
    s == "pointer" || s.starts_with("*const ") || s.starts_with("*mut ")
}

/// Returns true if the type name in a method's path matches the target owner.
///
/// Note on short-name collisions (P3): short-name matching (e.g. "Box" matching
/// "alloc::boxed::Box" or "Query" matching "bevy_ecs::system::query::Query") is
/// intentionally used to accommodate un-normalized or aliased type paths across
/// external crate boundaries. Exact canonical paths take precedence first.
pub fn type_matches_owner(self_ty: &str, target_owner: &str) -> bool {
    if is_slice_type(self_ty) && is_slice_type(target_owner) {
        return true;
    }
    if is_array_type(self_ty) && is_array_type(target_owner) {
        return true;
    }
    if is_pointer_type(self_ty) && is_pointer_type(target_owner) {
        return true;
    }
    let s_base = base_type_path(self_ty);
    let t_base = base_type_path(target_owner);
    if s_base == t_base {
        return true;
    }
    let s_short = s_base.rsplit("::").next().unwrap_or(s_base);
    let t_short = t_base.rsplit("::").next().unwrap_or(t_base);
    s_short == t_short
}

/// Returns the unqualified short name of a trait path.
/// E.g. `"core::convert::AsRef"` -> `"AsRef"`.
pub fn trait_short_name(trait_path: &str) -> &str {
    let base = base_trait_path(trait_path);
    base.rsplit("::").next().unwrap_or(base)
}

/// Returns true if the type is known not to implement `Deref`, avoiding expensive
/// autoderef graph traversal across workspace readers.
pub fn is_non_deref_type(ty: &str) -> bool {
    let b = base_type_path(ty);
    matches!(
        b,
        "usize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "isize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "f32"
            | "f64"
            | "bool"
            | "char"
            | "str"
            | "slice"
            | "core::option::Option"
            | "std::option::Option"
            | "Option"
            | "core::result::Result"
            | "std::result::Result"
            | "Result"
            | "core::cmp::Ordering"
            | "std::cmp::Ordering"
            | "Ordering"
            | "core::ops::Bound"
            | "std::ops::Bound"
            | "Bound"
    ) || is_slice_type(ty)
        || is_array_type(ty)
        || is_pointer_type(ty)
}

/// Checks if a trait belongs to the standard library prelude. `Debug`, `Display` and `Deref` are not in it: their
/// methods complete only where the file imports them.
pub fn is_in_prelude(trait_path: &str) -> bool {
    let base = base_trait_path(trait_path);
    is_prelude_trait(base) || is_prelude_trait(trait_path) || (base.contains("::") && (is_prelude_trait(&format!("core::{base}")) || is_prelude_trait(&format!("alloc::{base}"))))
}

/// Checks if a function signature takes a receiver argument (`self`, `&self`, `&'a Self`, etc.).
pub fn signature_takes_receiver(sig: &str) -> bool {
    let Some(i) = sig.find('(') else {
        return false;
    };
    let mut p = sig[i + 1..].trim_start();
    if let Some(rest) = p.strip_prefix('&') {
        p = rest.trim_start();
        // Skip optional lifetime like 'a, '_, 'static
        if p.starts_with('\'') {
            if let Some(sp) = p.find(|c: char| c.is_whitespace() || c == ',' || c == ')') {
                p = p[sp..].trim_start();
            }
        }
    }
    if let Some(rest) = p.strip_prefix("mut") {
        if rest.starts_with(' ') || rest.starts_with('\t') {
            p = rest.trim_start();
        }
    }
    let is_lowercase_self = p.starts_with("self");
    let is_capital_self = p.starts_with("Self");
    let is_self = is_lowercase_self || is_capital_self;
    is_self && (p.len() == 4 || p[4..].starts_with(',') || p[4..].starts_with(')') || p[4..].starts_with(':') || p[4..].starts_with(' ') || p[4..].starts_with('<'))
}

pub use super::super::scope_cache::collect_file_referenced_traits;

/// Categorizes a candidate definition by checking whether it is inherent or trait-provided,
/// and if trait-provided, whether that trait is in scope.
///
/// Implements zero-lookup ordering:
/// 1. Inherent if matches target_owner (no lookup).
/// 2. PreludeTrait if in prelude (no lookup).
/// 3. InScopeTrait if in file-referenced traits (no lookup).
/// 4. Inherent if matches current_owner from autoderef (no lookup).
/// 5. OutOfScopeTrait otherwise.
pub fn categorize_definition(def_kind: DefKind, def_path: &str, current_owner: &str, target_owner: &str, file_traits: &HashSet<String>) -> DefScopeCategory {
    if def_kind == DefKind::Field {
        return DefScopeCategory::Field;
    }

    // 1. Check if the definition's path is an explicit trait implementation:
    // `<SelfType as Trait>::method` or `<impl Trait for SelfType>::method`
    if let Some((trait_ref, self_ty)) = extract_trait_and_self_from_def_path(def_path) {
        if !type_matches_owner(self_ty, target_owner) && !type_matches_owner(self_ty, current_owner) {
            return DefScopeCategory::OutOfScopeTrait;
        }
        if is_in_prelude(trait_ref) {
            return DefScopeCategory::PreludeTrait;
        }
        let base = base_trait_path(trait_ref);
        let short = trait_short_name(trait_ref);
        if file_traits.contains(base) || file_traits.contains(trait_ref) || file_traits.contains(short) {
            return DefScopeCategory::InScopeTrait;
        }
        return DefScopeCategory::OutOfScopeTrait;
    }

    // 2. Check if def.path is of the form `Parent::method`
    if let Some(last_colon) = def_path.rfind("::") {
        let parent = &def_path[..last_colon];
        let parent_type = extract_parent_type(parent);

        // 1. Inherent method matching target owner (e.g. `Query::iter_ancestors`, `Box::downcast`)
        if type_matches_owner(parent_type, target_owner) {
            return DefScopeCategory::Inherent;
        }

        // 2. Standard prelude trait method (e.g. `Debug::fmt`, `Clone::clone`, `Deref::deref`)
        if is_in_prelude(parent_type) {
            return DefScopeCategory::PreludeTrait;
        }

        // 3. Trait method in scope via file imports or globs
        let base = base_trait_path(parent_type);
        let short = trait_short_name(parent_type);
        if file_traits.contains(base) || file_traits.contains(parent_type) || file_traits.contains(short) {
            return DefScopeCategory::InScopeTrait;
        }

        // 4. Inherent method on deref target type (e.g. `PathBuf -> Path`)
        if current_owner != target_owner && type_matches_owner(parent_type, current_owner) {
            return DefScopeCategory::Inherent;
        }

        return DefScopeCategory::OutOfScopeTrait;
    }

    DefScopeCategory::Inherent
}
