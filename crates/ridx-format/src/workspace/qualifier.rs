/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Scoped qualifier resolution: resolves path qualifiers (e.g. `range::`, `crate::foo::`)
//! respecting Rust's lexical and module visibility hierarchy:
//! 1. File scope: local `mod`, local items, and local `use` declarations in `file`.
//! 2. Crate scope: file's enclosing module and crate root (`crate::<qual>`).
//! 3. Prelude: standard library prelude 2021/2024 types and traits.
//! 4. External dependencies: crates in dependencies.
//! 5. Sysroot: standard library (`core`, `alloc`, `std`) if explicitly prefixed or requested.
//! 6. Fallback: single unique workspace match.

use super::prelude::prelude_candidate_paths;
use super::WorkspaceIndex;
use crate::format::{DefEntry, DefKind};
use crate::reader::ResolvedDefinition;

impl WorkspaceIndex {
    fn resolve_file_item<'a>(
        &'a self,
        idx: usize,
        def: &'a DefEntry,
        rest_opt: Option<&str>,
    ) -> Option<ResolvedDefinition<'a>> {
        let kind = DefKind::from(def.kind);
        if kind == DefKind::Use {
            let resolved = self.make_resolved_definition(idx, def);
            let target_opt = resolved
                .signature
                .and_then(|sig| self.find_definition_by_path(sig))
                .or_else(|| {
                    self.find_definition_at(resolved.file, resolved.start_line, resolved.start_col)
                });
            if let Some(target) = target_opt {
                if target.kind == DefKind::Use
                    || target.kind == DefKind::Local
                    || (target.file == resolved.file
                        && target.start_line == resolved.start_line
                        && target.start_col == resolved.start_col)
                {
                    return None;
                }
                if let Some(rest) = rest_opt {
                    return self.find_definition_by_path(&format!("{}::{rest}", target.path));
                } else {
                    return Some(target);
                }
            }
            return None;
        } else if matches!(
            kind,
            DefKind::Mod
                | DefKind::Struct
                | DefKind::Enum
                | DefKind::Union
                | DefKind::Trait
                | DefKind::TyAlias
        ) {
            let local_def = self.make_resolved_definition(idx, def);
            if let Some(rest) = rest_opt {
                return self.find_definition_by_path(&format!("{}::{rest}", local_def.path));
            } else {
                return Some(local_def);
            }
        }
        None
    }

    /// Resolves a qualifier path in the lexical and module scope of `file`:
    /// 1. File scope: local `mod`, local items, and local `use` declarations in `file`.
    /// 2. Crate scope: module path of the file and crate root.
    /// 3. Prelude: standard library prelude 2021/2024 types and traits.
    /// 4. Dependencies: external crate prelude / dependencies.
    /// 5. Sysroot: standard library (`core`, `alloc`, `std`) if explicitly requested or unshadowed.
    /// 6. Fallback: single unique workspace candidate.
    pub fn resolve_qualifier<'a>(
        &'a self,
        file: &str,
        qual: &str,
    ) -> Option<ResolvedDefinition<'a>> {
        if qual.is_empty() {
            return None;
        }

        if qual == "crate" {
            let cname = self.crate_name_for_file(file)?;
            return self.find_definition_by_path(cname);
        }

        if let Some(rest) = qual.strip_prefix("crate::") {
            let cname = self.crate_name_for_file(file)?;
            return self.find_definition_by_path(&format!("{cname}::{rest}"));
        }

        // Handle `super::` qualifiers by navigating up from the file's module path
        if qual.starts_with("super::") {
            let mod_path = self.module_path_for_file(file)?;
            let mut parts: Vec<&str> = mod_path.split("::").collect();
            let mut cur = qual;
            while let Some(rest) = cur.strip_prefix("super::") {
                if parts.len() > 1 {
                    parts.pop();
                }
                cur = rest;
            }
            let base = parts.join("::");
            let target = if cur.is_empty() {
                base
            } else {
                format!("{base}::{cur}")
            };
            return self.find_definition_by_path(&target);
        }

        let qual_trimmed = if let Some(rest) = qual.strip_prefix("self::") {
            rest
        } else {
            qual
        };

        let mut segs = qual_trimmed.split("::");
        let first_seg = segs.next().unwrap_or(qual_trimmed);
        let rest_opt = if qual_trimmed.len() > first_seg.len() + 2 {
            Some(&qual_trimmed[first_seg.len() + 2..])
        } else {
            None
        };

        let reader_indices = self.reader_indices_for_file(file);
        let crate_idx = reader_indices.first().copied();
        let mod_path_opt = self.module_path_for_file(file);

        let is_sysroot = matches!(first_seg, "core" | "alloc" | "std");
        let is_dep_crate = !is_sysroot && self.has_crate(first_seg);

        // 1. File scope: check definitions and imports declared directly in `file` across matching readers
        for &idx in &reader_indices {
            let reader = &self.readers[idx];
            let target_file = self.target_file_for_reader(idx, file);
            let target_str = target_file.as_deref().unwrap_or(file);
            if let Some(fid) = reader.find_file_id(target_str) {
                // Fast path: check if local item exists with path `<mod_path>::<first_seg>`
                if !is_sysroot && !is_dep_crate {
                    if let Some(ref mod_path) = mod_path_opt {
                        let fast_target = format!("{mod_path}::{first_seg}");
                        if let Some(def) = reader.find_definition_by_path(&fast_target) {
                            let span = def.span();
                            if span.file_id == fid {
                                if let Some(res) = self.resolve_file_item(idx, def, rest_opt) {
                                    return Some(res);
                                }
                            }
                        }
                    }
                }

                // General file scan for local items / imports with matching name
                for def in reader.definitions() {
                    let span = def.span();
                    if span.file_id == fid && reader.get_str(def.name_offset()) == first_seg {
                        if (is_sysroot || is_dep_crate) && DefKind::from(def.kind) == DefKind::Use {
                            continue;
                        }
                        if let Some(res) = self.resolve_file_item(idx, def, rest_opt) {
                            return Some(res);
                        }
                    }
                }

                // Glob imports declared in `file`: `use X::*` (and `use super::*`)
                // Probing `<X>::<first_seg>` via O(log N) path lookup
                if !is_sysroot && !is_dep_crate {
                    let cname = reader.crate_name();
                    for def in reader.definitions() {
                        let span = def.span();
                        if span.file_id == fid && DefKind::from(def.kind) == DefKind::Use {
                            let name = reader.get_str(def.name_offset());
                            let path = reader.get_str(def.path_offset());
                            let is_glob = name == "*"
                                || name == "{{glob}}"
                                || name.starts_with("{{use}}")
                                || path.ends_with("::*");
                            if is_glob {
                                let raw_target = if def.sig_offset() != 0 {
                                    let sig = reader.get_str(def.sig_offset());
                                    sig.strip_suffix("::*").unwrap_or(sig).to_string()
                                } else {
                                    let resolved = self.make_resolved_definition(idx, def);
                                    if let Some(target) = self.find_definition_at(
                                        resolved.file,
                                        resolved.start_line,
                                        resolved.start_col,
                                    ) {
                                        target.path.to_string()
                                    } else if let Some(stripped) = path.strip_suffix("::*") {
                                        stripped.to_string()
                                    } else {
                                        String::new()
                                    }
                                };

                                let glob_target = if raw_target == "super"
                                    || raw_target.starts_with("super::")
                                {
                                    if let Some(ref mod_path) = mod_path_opt {
                                        let mut parts: Vec<&str> = mod_path.split("::").collect();
                                        let mut cur = raw_target.as_str();
                                        while let Some(rest) = cur.strip_prefix("super::") {
                                            if parts.len() > 1 {
                                                parts.pop();
                                            }
                                            cur = rest;
                                        }
                                        if cur == "super" && parts.len() > 1 {
                                            parts.pop();
                                            parts.join("::")
                                        } else if cur != "super" {
                                            format!("{}::{cur}", parts.join("::"))
                                        } else {
                                            parts.join("::")
                                        }
                                    } else {
                                        raw_target
                                    }
                                } else if let Some(rest) = raw_target.strip_prefix("crate::") {
                                    format!("{cname}::{rest}")
                                } else {
                                    raw_target
                                };

                                if !glob_target.is_empty() {
                                    let probe = format!("{glob_target}::{first_seg}");
                                    if let Some(mut found) = self.find_definition_by_path(&probe) {
                                        if found.kind == DefKind::Use {
                                            let target = found
                                                .signature
                                                .and_then(|sig| self.find_definition_by_path(sig))
                                                .or_else(|| {
                                                    self.find_definition_at(
                                                        found.file,
                                                        found.start_line,
                                                        found.start_col,
                                                    )
                                                });
                                            if let Some(target) = target {
                                                if target.kind != DefKind::Local
                                                    && target.kind != DefKind::Use
                                                {
                                                    found = target;
                                                } else {
                                                    continue;
                                                }
                                            } else {
                                                continue;
                                            }
                                        }
                                        if let Some(rest) = rest_opt {
                                            if let Some(sub) = self.find_definition_by_path(
                                                &format!("{}::{rest}", found.path),
                                            ) {
                                                return Some(sub);
                                            }
                                        } else {
                                            return Some(found);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // 2. Crate scope: check enclosing module (<module_path>::<qual>) and crate root (<crate>::<qual>)
        if let Some(idx) = crate_idx {
            let cname = self.readers[idx].crate_name();

            if !is_sysroot && !is_dep_crate {
                // Try enclosing module
                if let Some(ref mod_path) = mod_path_opt {
                    if let Some(def) =
                        self.find_definition_by_path(&format!("{mod_path}::{qual_trimmed}"))
                    {
                        if def.kind != DefKind::Use {
                            return Some(def);
                        }
                    }
                }

                // Try crate root
                if let Some(def) = self.find_definition_by_path(&format!("{cname}::{qual_trimmed}"))
                {
                    if def.kind != DefKind::Use {
                        return Some(def);
                    }
                }
            }
        }

        // 3. Prelude: standard library prelude 2021/2024 types and traits
        for candidate in prelude_candidate_paths(first_seg) {
            if let Some(cand) = self.find_definition_by_path(candidate) {
                if let Some(rest) = rest_opt {
                    if let Some(sub) =
                        self.find_definition_by_path(&format!("{}::{rest}", cand.path))
                    {
                        return Some(sub);
                    }
                } else {
                    return Some(cand);
                }
            }
        }

        // 4. External dependencies: check if `first_seg` is a loaded dependency crate (not sysroot)
        if self.has_crate(first_seg)
            && first_seg != "core"
            && first_seg != "alloc"
            && first_seg != "std"
        {
            let rest = rest_opt?;
            if let Some(def) = self.find_definition_by_path(&format!("{first_seg}::{rest}")) {
                return Some(def);
            }
        }

        // 5. Sysroot: check standard library if explicitly requested or unshadowed
        if qual_trimmed.starts_with("core::")
            || qual_trimmed.starts_with("alloc::")
            || qual_trimmed.starts_with("std::")
            || matches!(first_seg, "core" | "alloc" | "std")
        {
            if let Some(def) = self.find_definition_by_path(qual_trimmed) {
                return Some(def);
            }
        }

        // 6. Fallback: single unique match across workspace readers (reject ambiguous matches)
        let mut matched: Option<ResolvedDefinition<'a>> = None;
        for (i, reader) in self.readers.iter().enumerate() {
            if !qual_trimmed.contains("::")
                && Some(i) != crate_idx
                && reader.crate_name() != qual_trimmed
            {
                continue;
            }
            let prefix = format!("{}::", reader.crate_name());
            let sub = qual_trimmed.strip_prefix(&prefix).unwrap_or(qual_trimmed);
            if let Some(def) = reader
                .find_definition_by_path(qual_trimmed)
                .or_else(|| reader.find_definition_by_path(sub))
            {
                if DefKind::from(def.kind) == DefKind::Use {
                    continue;
                }
                if matched.is_some() {
                    return None;
                }
                matched = Some(self.make_resolved_definition(i, def));
            }
        }
        matched
    }
}
