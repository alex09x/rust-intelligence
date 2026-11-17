/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The file a module of a definition lives in: `mod foo;`'s `foo.rs` or `foo/mod.rs`, a crate's root, an
//! `extern crate x as y;`'s crate.

use ridx_format::{DefKind, WorkspaceIndex};

pub(super) fn resolve_backing_module_file(ws: &WorkspaceIndex, def: &ridx_format::ResolvedDefinition<'_>, decl_file: &std::path::Path) -> Option<std::path::PathBuf> {
    let mod_name = def.name;
    // A crate's root (`base` of `base::ONE`): its file, from the start, as rust-analyzer opens it (it went to the line
    // of the first item).
    // The root is the entry its crate keys at `""`, in whatever file `[lib] path` names. A `pub mod foo;` of crate `foo`
    // shares the root's name and path but not its span: it opens `foo.rs` below (review of 14e0f41f).
    // A dependency's root reached from another crate (`engine::…` of a renamed `base`) is recorded in that crate's
    // index: the root compared is the one of the crate the module names.
    let is_root_of = |r: &ridx_format::IndexReader<memmap2::Mmap>| {
        r.find_definition_by_path("").is_some_and(|root| {
            let s = root.span();
            s.start_line() == def.start_line && s.start_col() == def.start_col && r.get_file(s.file_id()) == def.file
        })
    };
    let crate_root = def.reader_idx.and_then(|i| ws.readers().get(i)).is_some_and(is_root_of) || ws.readers().iter().any(|r| r.crate_name() == def.name && is_root_of(r));
    if def.kind == DefKind::Mod && crate_root {
        return Some(decl_file.to_path_buf());
    }
    if def.kind == DefKind::Mod {
        let parent = decl_file.parent()?;
        let stem = decl_file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let mut dirs = Vec::new();
        if !stem.is_empty() && !matches!(stem, "lib" | "main" | "mod") {
            dirs.push(parent.join(stem));
        }
        dirs.push(parent.to_path_buf());
        for dir in &dirs {
            for cand in [dir.join(format!("{mod_name}.rs")), dir.join(mod_name).join("mod.rs")] {
                if cand != decl_file && cand.is_file() {
                    return Some(cand);
                }
            }
        }
        if let Some(reader) = def.reader_idx.and_then(|idx| ws.readers().get(idx)) {
            let raw_path = std::path::Path::new(def.file);
            if let Some(raw_parent) = raw_path.parent() {
                let raw_stem = raw_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                let mut raw_dirs = Vec::new();
                if !raw_stem.is_empty() && !matches!(raw_stem, "lib" | "main" | "mod") {
                    raw_dirs.push(raw_parent.join(raw_stem));
                }
                raw_dirs.push(raw_parent.to_path_buf());
                for dir in &raw_dirs {
                    for cand in [dir.join(format!("{mod_name}.rs")), dir.join(mod_name).join("mod.rs")] {
                        if let Some(cs) = cand.to_str() {
                            if cs != def.file && reader.find_file_id(cs).is_some() {
                                return Some(cand);
                            }
                        }
                    }
                    let sub_prefix = format!("{}/", dir.join(mod_name).display());
                    if (0..reader.files_count()).any(|fid| reader.get_file(fid).starts_with(&sub_prefix)) {
                        return Some(dir.join(mod_name).join("mod.rs"));
                    }
                }
                if def.file.starts_with("library/") && def.start_line == def.end_line {
                    return Some(raw_parent.join(format!("{mod_name}.rs")));
                }
            }
        }
    }
    if decl_file.is_file() && def.start_line > 0 {
        if let Ok(src) = std::fs::read_to_string(decl_file) {
            if let Some(ln) = src.lines().nth(def.start_line.saturating_sub(1) as usize) {
                if let Some((_, after)) = ln.split_once("extern crate ") {
                    if let Some((real_crate, _)) = after.split_once(" as ") {
                        let rc = real_crate.trim().trim_end_matches(';').trim();
                        if let Some(ed) = ws.find_definition_by_path(rc) {
                            if ed.kind == DefKind::Mod {
                                return Some(ed.resolved_path());
                            }
                        }
                        for r in ws.readers() {
                            if r.crate_name() == rc && r.files_count() > 0 {
                                let f0 = r.get_file(0);
                                if !f0.is_empty() {
                                    return Some(ws.resolve_path_for_crate(rc, f0));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}
