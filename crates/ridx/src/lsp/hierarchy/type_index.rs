/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compact name-indexed definition table for struct, enum, and trait resolution.

use ridx_format::{fnv1a_64, DefKind, WorkspaceIndex, WorkspaceSymbol};

pub const KIND_STRUCT: u8 = 1 << 0;
pub const KIND_ENUM: u8 = 1 << 1;
pub const KIND_TRAIT: u8 = 1 << 2;

#[derive(Clone, Copy, Debug)]
pub struct TypeDefEntry {
    pub hash: u64,
    pub reader_idx: u16,
    pub def_idx: u32,
    pub kind: u8,
}

impl TypeDefEntry {
    #[inline]
    pub fn mask(&self) -> u8 {
        match DefKind::from(self.kind) {
            DefKind::Struct => KIND_STRUCT,
            DefKind::Enum => KIND_ENUM,
            DefKind::Trait => KIND_TRAIT,
            _ => 0,
        }
    }
}

pub fn build_type_index(ws: &WorkspaceIndex) -> Vec<TypeDefEntry> {
    let mut entries = Vec::new();
    for (reader_idx, reader) in ws.readers().iter().enumerate() {
        for (def_idx, def) in reader.definitions().iter().enumerate() {
            let k = DefKind::from(def.kind);
            if matches!(k, DefKind::Struct | DefKind::Enum | DefKind::Trait) {
                let name = reader.get_str(def.name_offset());
                let hash = fnv1a_64(name.as_bytes());
                entries.push(TypeDefEntry {
                    hash,
                    reader_idx: reader_idx as u16,
                    def_idx: def_idx as u32,
                    kind: def.kind,
                });
            }
        }
    }
    entries.sort_unstable_by_key(|e| (e.hash, e.reader_idx, e.def_idx));
    entries
}

pub fn find_in_type_index<'a>(
    entries: &[TypeDefEntry],
    ws: &'a WorkspaceIndex,
    query: &str,
    expected_mask: u8,
) -> Option<WorkspaceSymbol<'a>> {
    if query.contains("::") {
        return None;
    }
    let hash = fnv1a_64(query.as_bytes());
    let idx = entries.binary_search_by_key(&hash, |e| e.hash).ok()?;

    let mut start = idx;
    while start > 0 && entries[start - 1].hash == hash {
        start -= 1;
    }

    for entry in &entries[start..] {
        if entry.hash != hash {
            break;
        }
        if (entry.mask() & expected_mask) == 0 {
            continue;
        }
        let reader = &ws.readers()[entry.reader_idx as usize];
        let def = &reader.definitions()[entry.def_idx as usize];
        let name = reader.get_str(def.name_offset());
        if name != query {
            continue;
        }
        let crate_name = reader.crate_name();
        let path = reader.get_str(def.path_offset());
        let span = def.span();
        let file = reader.get_file(span.file_id());
        let signature = if def.sig_offset() != 0 {
            Some(reader.get_str(def.sig_offset()))
        } else {
            None
        };
        let source_root = ws
            .source_root_at(entry.reader_idx as usize)
            .or_else(|| ws.workspace_root());
        let unit_key = {
            let uk = reader.unit_key();
            if uk.is_empty() {
                None
            } else {
                Some(uk)
            }
        };

        return Some(WorkspaceSymbol {
            crate_name,
            name,
            path,
            kind: DefKind::from(def.kind),
            file,
            span,
            signature,
            is_macro_expansion: def.is_macro_expansion(),
            source_root,
            unit_key,
        });
    }
    None
}
