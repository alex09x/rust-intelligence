/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Where a restricted item (`pub(super)`, `pub(in crate::a)`, `pub(self)`) is visible. The index records only that
//! an item is restricted; the scope is read from its declaration, at the span the index gives.

use ridx_format::{ResolvedDefinition, WorkspaceIndex};

/// The crate-relative module a restricted item is visible in (`""` for the whole crate), from the `pub(…)` before its
/// name; `owner` is the module the item is declared in (for a field or method, the module of its type). `None` when
/// the declaration cannot be read or names a scope other than `crate`, `self`, `super` or `in crate::…`.
pub(crate) fn restricted_scope(ws: &WorkspaceIndex, def: &ResolvedDefinition<'_>, owner: &str) -> Option<String> {
    let reader = def.reader_idx.and_then(|i| ws.readers().get(i))?;
    let path = ws.resolve_path_for_crate(reader.crate_name(), def.file);
    let text = std::fs::read_to_string(path).ok()?;
    let line = text.lines().nth((def.start_line as usize).checked_sub(1)?)?;
    let col = line.char_indices().nth((def.start_col as usize).checked_sub(1)?).map_or(line.len(), |(b, _)| b);
    scope_of(&line[..col], owner)
}

/// Whether a body naming `super::…` means the same module at every call site: the function sits at the top of its
/// file (no inline `mod` around it), and every call sits at the top of a file of the same crate whose module has the
/// same parent (a sibling, or the function's own module). `sites` are the call positions by absolute file.
pub(crate) fn super_means_the_same(ws: &WorkspaceIndex, file: &str, fn_in_inline_mod: bool, sites: &std::collections::BTreeMap<std::path::PathBuf, Vec<(u32, u32)>>) -> bool {
    let unit = |f: &str| ws.reader_index_for_file(f).map(|i| ws.readers()[i].crate_name().to_string());
    let parent = |f: &str| ws.module_of_file(f).map(|m| m.rsplit_once("::").map_or(String::new(), |(p, _)| p.to_string()));
    let (Some(home_unit), Some(home_parent)) = (unit(file), parent(file)) else { return false };
    // The crate root has no parent module for `super` to name.
    if fn_in_inline_mod || ws.module_of_file(file).is_some_and(|m| m.is_empty()) {
        return false;
    }
    sites.iter().all(|(path, spans)| {
        let Some(p) = path.to_str() else { return false };
        let Ok(text) = std::fs::read_to_string(path) else { return false };
        let masked = super::action_parse::mask_comments_and_strings(&text);
        let starts = super::action_parse::line_starts_of(&text);
        unit(p).as_ref() == Some(&home_unit)
            && parent(p).as_ref() == Some(&home_parent)
            && spans.iter().all(|&(l, c)| {
                super::action_inline_callers::line_col_to_byte(&text, &starts, l, c)
                    .is_some_and(|b| super::action_inline_callers_transform::find_enclosing_mod_range(&masked, b).is_none())
            })
    })
}

/// The scope of the last `pub(…)` in `prefix` (the declaration's text before the name), seen from module `owner`.
pub(crate) fn scope_of(prefix: &str, owner: &str) -> Option<String> {
    let open = prefix.rfind("pub(")? + 4;
    let inner = prefix[open..].split(')').next()?.trim();
    let parent = |m: &str| m.rsplit_once("::").map_or(String::new(), |(p, _)| p.to_string());
    match inner {
        "crate" => Some(String::new()),
        "self" => Some(owner.to_string()),
        "super" if !owner.is_empty() => Some(parent(owner)),
        _ => {
            let target = inner.strip_prefix("in ")?.trim();
            match target.strip_prefix("crate") {
                Some("") => Some(String::new()),
                Some(rest) => rest.strip_prefix("::").map(str::to_string),
                None if target == "super" && !owner.is_empty() => Some(parent(owner)),
                None if target == "self" => Some(owner.to_string()),
                None => None,
            }
        }
    }
}

/// Whether crate-relative module `dest` is `scope` or lies inside it (`""` is the whole crate).
pub(crate) fn inside(dest: &str, scope: &str) -> bool {
    scope.is_empty() || dest == scope || dest.starts_with(&format!("{scope}::"))
}

/// The module a member (`m::Type::name`, `m::Type::<T>::name`, crate-relative) is declared in: `m`, or `""` at the
/// crate root.
pub(crate) fn member_owner(rel_path: &str) -> String {
    // A member of an impl block (`m::<impl a::b::T>::name`) belongs to the module holding the impl, whose path may
    // name the type with `::` of its own.
    if let Some(p) = rel_path.find("::<impl ") {
        return rel_path[..p].to_string();
    }
    if rel_path.starts_with("<impl ") {
        return String::new();
    }
    let segs: Vec<&str> = rel_path.split("::").filter(|s| !s.starts_with('<')).collect();
    segs[..segs.len().saturating_sub(2)].join("::")
}

#[cfg(test)]
mod tests {
    use super::{inside, member_owner, scope_of};

    #[test]
    fn restricted_scopes_are_read_from_the_declaration() {
        assert_eq!(scope_of("    pub(super) ", "runtime::task::core").as_deref(), Some("runtime::task"));
        assert_eq!(scope_of("pub(in crate::runtime) fn ", "runtime::task").as_deref(), Some("runtime"));
        assert_eq!(scope_of("pub(crate) ", "a").as_deref(), Some(""));
        assert_eq!(scope_of("pub(self) ", "a::b").as_deref(), Some("a::b"));
        assert_eq!(scope_of("pub(in super::x) ", "a"), None);
        assert!(inside("runtime::task::harness", "runtime::task") && !inside("runtime::taskx", "runtime::task") && inside("x", ""));
        assert_eq!(member_owner("runtime::task::core::Header::scheduled_at"), "runtime::task::core");
        assert_eq!(member_owner("Header::f"), "");
        assert_eq!(member_owner("query::state::QueryState::<D, F>::iter_mut"), "query::state");
        assert_eq!(member_owner("runtime::task::harness::<impl runtime::task::raw::RawTask>::drop_reference"), "runtime::task::harness");
    }
}
