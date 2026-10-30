/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The two steps shared by every reference lookup of a reader: the check of the definition a
//! reference without a target unit points at, and the recording of a reference once.

use super::*;
use crate::format::RefEntry;
use memmap2::Mmap;
use std::collections::HashSet;

/// The definition a lookup targets: its file as the index spells it, its full path, its line.
pub(super) type DefCoords = (String, String, u32);

/// Optional target definition metadata used to filter out path collisions (such as field vs
/// method of the same name, block-local duplicate paths, or outer macro-callsite spans).
#[derive(Default)]
pub(super) struct ReferenceTarget<'a> {
    pub kind: Option<DefKind>,
    pub name: Option<&'a str>,
    pub coords: Option<DefCoords>,
    pub disambiguate_line: bool,
}

type SeenKey = (PathBuf, u32, u32, u32, u32);

pub(super) fn is_raw_ident_keyword(name: &str) -> bool {
    matches!(
        name,
        "as" | "async"
            | "await"
            | "box"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "gen"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "macro"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "try"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "yield"
            | "abstract"
            | "become"
            | "do"
            | "final"
            | "override"
            | "priv"
            | "typeof"
            | "unsized"
            | "virtual"
    )
}

/// Checks whether `r` matches the target definition kind, token width, and definition line.
pub(super) fn matches_reference_target(reader: &IndexReader<Mmap>, r: &RefEntry, target: &ReferenceTarget<'_>) -> bool {
    let r_kind = DefKind::from(r.kind);
    if let Some(t_kind) = target.kind {
        if r_kind != DefKind::Other {
            if t_kind == DefKind::Field && r_kind != DefKind::Field {
                return false;
            }
            if t_kind != DefKind::Field && r_kind == DefKind::Field {
                return false;
            }
        }
    }

    if r_kind != DefKind::Macro && target.kind != Some(DefKind::Macro) {
        let span = r.span();
        if span.end_line() != span.start_line() {
            return false;
        }
        let name = target.name.unwrap_or_else(|| {
            let p = reader.get_str(r.target_path_offset());
            p.rsplit("::").next().unwrap_or(p)
        });
        let name_chars = name.chars().count();
        let width = span.end_col().saturating_sub(span.start_col()) as usize;
        if r.is_macro_expansion() && r_kind != DefKind::Macro && target.kind != Some(DefKind::Macro) {
            if width != name_chars {
                return false;
            }
        } else if !r.is_macro_expansion() && width == name_chars + 2 && !is_raw_ident_keyword(name) {
            return false;
        }
    }

    if target.kind == Some(DefKind::Local) || target.disambiguate_line {
        if let Some(ref coords) = target.coords {
            if !points_at_definition(reader, r, coords) {
                return false;
            }
        }
    }

    true
}

/// Whether the reference `r` of `reader`, which records no target unit, points at the
/// definition `coords`: its target line and target file agree where it records them.
pub(super) fn points_at_definition(reader: &IndexReader<Mmap>, r: &RefEntry, coords: &DefCoords) -> bool {
    let (def_file, def_full, def_line) = coords;
    if r.target_line() != 0 && r.target_line() != *def_line {
        return false;
    }
    if r.target_file_offset() != 0 {
        let ref_file = reader.get_str(r.target_file_offset());
        if !ref_file.is_empty() && !def_file.is_empty() {
            let r_p = Path::new(ref_file);
            if r_p != Path::new(def_file) && r_p != Path::new(def_full) && !ref_file.ends_with(def_file.as_str()) && !def_file.ends_with(ref_file) {
                return false;
            }
        }
    }
    true
}

/// Records the reference `r` of `reader` in `out`, unless a reference over the same span of the
/// same file was recorded before.
pub(super) fn push_reference<'a>(
    reader: &'a IndexReader<Mmap>,
    r: &RefEntry,
    (reader_crate, reader_unit, source_root): (&'a str, Option<&'a str>, Option<&'a Path>),
    seen: &mut HashSet<SeenKey>,
    out: &mut Vec<WorkspaceReference<'a>>,
) {
    let span = r.span();
    let file = reader.get_file(span.file_id());
    let resolved_path = {
        let p = Path::new(file);
        if p.is_absolute() {
            p.to_path_buf()
        } else if let Some(root) = source_root {
            crate::format::join_resolving_overlap(root, p)
        } else {
            p.to_path_buf()
        }
    };
    let key = (resolved_path, span.start_line(), span.start_col(), span.end_line(), span.end_col());
    if !seen.insert(key) {
        return;
    }
    out.push(WorkspaceReference {
        crate_name: reader_crate,
        file,
        span,
        kind: DefKind::from(r.kind),
        is_write: r.is_write != 0,
        is_macro_expansion: r.is_macro_expansion(),
        source_root,
        unit_key: reader_unit,
    });
}
