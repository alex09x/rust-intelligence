/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use std::io::{self, Seek, Write};

use super::IndexWriter;
use crate::format::*;

impl IndexWriter {
    /// Serializes the entire index to any writer implementing `Write + Seek`.
    pub fn write_to<W: Write + Seek>(&mut self, writer: &mut W) -> io::Result<()> {
        let crate_name = self.crate_name.clone();
        let crate_version = self.crate_version.clone();
        let crate_name_offset = self.intern_string(&crate_name);
        let crate_version_offset = self.intern_string(&crate_version);
        let unit_key = self.unit_key.clone();
        let unit_key_offset = match &unit_key {
            Some(k) => self.intern_string(k),
            None => 0,
        };
        let generation = self.generation.clone();
        let generation_offset = match &generation {
            Some(g) => self.intern_string(g),
            None => 0,
        };

        let definitions = std::mem::take(&mut self.definitions);
        let def_owners_raw = std::mem::take(&mut self.def_owners);
        let references = std::mem::take(&mut self.references);
        let types = std::mem::take(&mut self.types);
        let derefs_raw = std::mem::take(&mut self.derefs);

        // Prepare definitions records and hash index
        let mut def_entries = Vec::with_capacity(definitions.len());
        let mut def_hashes = Vec::with_capacity(definitions.len());

        for (def_idx, def) in definitions.iter().enumerate() {
            let file_id = self.get_or_insert_file(&def.file);
            let name_offset = self.intern_string(&def.name);
            let path_offset = self.intern_string(&def.path);
            let sig_offset = match &def.signature {
                Some(s) => self.intern_string(s),
                None => 0,
            };

            let mut flags = 0u8;
            if def.is_macro_expansion {
                flags |= FLAG_MACRO_EXPANSION;
            }
            if self.reexport_indices.contains(&(def_idx as u32)) {
                flags |= FLAG_REEXPORT;
            }

            let entry = DefEntry {
                def_id: def.def_id,
                kind: def.kind as u8,
                visibility: def.visibility,
                flags,
                _pad: 0,
                name_offset,
                path_offset,
                sig_offset,
                span: Span::new(file_id, def.start_line, def.start_col, def.end_line, def.end_col),
            };
            def_entries.push(entry);
        }

        // Build defs hash index
        for (i, def) in definitions.iter().enumerate() {
            let hash = fnv1a_64(def.path.as_bytes());
            def_hashes.push(HashIndexEntry { hash, index: i as u32, _pad: [0; 4] });
        }
        def_hashes.sort_by_key(|e| e.hash);

        // Build definitions by owner index
        let mut owner_entries = Vec::with_capacity(def_owners_raw.len());
        let mut seen_owners = std::collections::HashSet::new();
        for (def_idx, owner) in &def_owners_raw {
            if (*def_idx as usize) >= definitions.len() || owner.is_empty() {
                continue;
            }
            if seen_owners.insert((*def_idx, owner.clone())) {
                let owner_path_offset = self.intern_string(owner);
                let hash = fnv1a_64(owner.as_bytes());
                owner_entries.push(OwnerIndexEntry::new(hash, owner_path_offset, *def_idx));
            }
        }
        owner_entries.sort_by_key(|e| e.hash);

        // Build derefs table
        let mut deref_entries = Vec::with_capacity(derefs_raw.len());
        let mut seen_derefs = std::collections::HashSet::new();
        for (src, tgt) in &derefs_raw {
            if src.is_empty() || tgt.is_empty() {
                continue;
            }
            if seen_derefs.insert((src.clone(), tgt.clone())) {
                let source_type_offset = self.intern_string(src);
                let target_type_offset = self.intern_string(tgt);
                let source_type_hash = fnv1a_64(src.as_bytes());
                deref_entries.push(DerefEntry::new(source_type_hash, source_type_offset, target_type_offset));
            }
        }
        deref_entries.sort_by_key(|e| e.source_type_hash);

        // Prepare references records
        let mut ref_entries = Vec::with_capacity(references.len());
        for r in &references {
            let file_id = self.get_or_insert_file(&r.file);
            let target_path_offset = self.intern_string(&r.target_path);
            let target_file_offset = match &r.target_file {
                Some(f) => self.intern_string(f),
                None => 0,
            };
            let target_unit_offset = match &r.target_unit {
                Some(u) => self.intern_string(u),
                None => 0,
            };

            let entry = RefEntry {
                span: Span::new(file_id, r.start_line, r.start_col, r.end_line, r.end_col),
                target_def_id: r.target_def_id.unwrap_or(u32::MAX),
                target_path_offset,
                target_file_offset,
                target_unit_offset,
                target_line: r.target_line,
                target_col: r.target_col,
                kind: r.kind as u8,
                is_write: if r.is_write { 1 } else { 0 },
                flags: if r.is_macro_expansion { FLAG_MACRO_EXPANSION } else { 0 },
                _pad: 0,
            };
            ref_entries.push(entry);
        }

        // Sort references by source span for binary search position lookup
        ref_entries.sort_by_key(|a| (a.span.file_id, a.span.start_line, a.span.start_col));

        // Build references target path hash index
        let mut ref_hashes = Vec::with_capacity(ref_entries.len());
        for (i, r) in ref_entries.iter().enumerate() {
            let target_path_bytes = self.get_interned_str(r.target_path_offset);
            let hash = fnv1a_64(target_path_bytes);
            ref_hashes.push(HashIndexEntry { hash, index: i as u32, _pad: [0; 4] });
        }
        ref_hashes.sort_by_key(|e| e.hash);

        // Prepare types records
        let mut type_entries = Vec::with_capacity(types.len());
        for (t, owner_opt) in &types {
            let file_id = self.get_or_insert_file(&t.file);
            let type_offset = self.intern_string(&t.type_name);
            let owner_path_offset = match owner_opt {
                Some(o) => self.intern_string(o),
                None => 0,
            };

            let entry = TypeEntry::new(Span::new(file_id, t.start_line, t.start_col, t.end_line, t.end_col), type_offset, owner_path_offset);
            type_entries.push(entry);
        }

        // Sort types by source span
        type_entries.sort_by_key(|a| (a.span.file_id, a.span.start_line, a.span.start_col));

        // Prepare notes, sorted by span
        let notes = std::mem::take(&mut self.notes);
        let mut note_entries = Vec::with_capacity(notes.len());
        for n in &notes {
            let file_id = self.get_or_insert_file(&n.file);
            let text_offset = self.intern_string(&n.text);
            note_entries.push(NoteEntry::new(Span::new(file_id, n.start_line, n.start_col, n.end_line, n.end_col), text_offset, n.kind));
        }
        note_entries.sort_by_key(|a| (a.span.file_id, a.span.start_line, a.span.start_col, a.kind));
        note_entries.dedup_by_key(|a| (a.span.file_id, a.span.start_line, a.span.start_col, a.kind));
        self.notes = notes;

        // Restore original collections on self
        self.definitions = definitions;
        self.def_owners = def_owners_raw;
        self.references = references;
        self.types = types;
        self.derefs = derefs_raw;

        // Layout the file sections with 8-byte alignment
        let header_size = std::mem::size_of::<Header>() as u64;
        let mut current_offset = align_up(header_size, 8);

        let string_pool_offset = current_offset;
        let string_pool_len = self.string_pool.len() as u64;
        current_offset = align_up(string_pool_offset + string_pool_len, 8);

        let files_offset = current_offset;
        let files_len = (self.file_offsets.len() * std::mem::size_of::<crate::format::FileEntry>()) as u64;
        current_offset = align_up(files_offset + files_len, 8);

        let defs_offset = current_offset;
        let defs_len = (def_entries.len() * std::mem::size_of::<DefEntry>()) as u64;
        current_offset = align_up(defs_offset + defs_len, 8);

        let defs_by_hash_offset = current_offset;
        let defs_by_hash_len = (def_hashes.len() * std::mem::size_of::<HashIndexEntry>()) as u64;
        current_offset = align_up(defs_by_hash_offset + defs_by_hash_len, 8);

        let refs_offset = current_offset;
        let refs_len = (ref_entries.len() * std::mem::size_of::<RefEntry>()) as u64;
        current_offset = align_up(refs_offset + refs_len, 8);

        let refs_by_target_offset = current_offset;
        let refs_by_target_len = (ref_hashes.len() * std::mem::size_of::<HashIndexEntry>()) as u64;
        current_offset = align_up(refs_by_target_offset + refs_by_target_len, 8);

        let types_offset = current_offset;
        let types_len = (type_entries.len() * std::mem::size_of::<TypeEntry>()) as u64;
        current_offset = align_up(types_offset + types_len, 8);

        let defs_by_owner_offset = current_offset;
        let defs_by_owner_len = (owner_entries.len() * std::mem::size_of::<OwnerIndexEntry>()) as u64;
        current_offset = align_up(defs_by_owner_offset + defs_by_owner_len, 8);

        let derefs_offset = current_offset;
        let derefs_len = (deref_entries.len() * std::mem::size_of::<DerefEntry>()) as u64;
        current_offset = align_up(derefs_offset + derefs_len, 8);

        let notes_offset = current_offset;
        let notes_len = (note_entries.len() * std::mem::size_of::<NoteEntry>()) as u64;
        current_offset = align_up(notes_offset + notes_len, 8);

        let typed_sections = self.prepare_typed_sections(current_offset);

        let mut header = Header {
            magic: MAGIC,
            version: CURRENT_VERSION,
            crate_name_offset,
            crate_version_offset,
            string_pool_offset,
            string_pool_len,
            files_offset,
            files_count: self.file_offsets.len() as u32,
            unit_key_offset,
            defs_offset,
            defs_count: def_entries.len() as u32,
            generation_offset,
            defs_by_hash_offset,
            defs_by_hash_count: def_hashes.len() as u32,
            _pad2: 0,
            refs_offset,
            refs_count: ref_entries.len() as u32,
            _pad3: 0,
            refs_by_target_offset,
            refs_by_target_count: ref_hashes.len() as u32,
            _pad4: 0,
            types_offset,
            types_count: type_entries.len() as u32,
            _pad5: 0,
            defs_by_owner_offset,
            defs_by_owner_count: owner_entries.len() as u32,
            _pad6: 0,
            derefs_offset,
            derefs_count: deref_entries.len() as u32,
            _pad7: 0,
            notes_offset,
            notes_count: note_entries.len() as u32,
            _pad8: 0,
            ..Header::default()
        };
        self.fill_typed_header(&mut header, &typed_sections);

        // Write Header in explicit little-endian format
        writer.seek(io::SeekFrom::Start(0))?;
        let header_le = header.to_le();
        writer.write_all(as_byte_slice(&header_le))?;

        // Write String Pool (raw bytes, endian-agnostic)
        writer.seek(io::SeekFrom::Start(string_pool_offset))?;
        writer.write_all(&self.string_pool)?;

        // Write File Table (array of FileEntry in little-endian format)
        writer.seek(io::SeekFrom::Start(files_offset))?;
        for &offset in &self.file_offsets {
            let f_path = self.get_interned_str(offset);
            let f_str = std::str::from_utf8(f_path).unwrap_or("");
            let hash = self.file_hashes.get(f_str).copied().unwrap_or(0);
            let entry = crate::format::FileEntry::new(offset, hash);
            writer.write_all(as_byte_slice(&entry.to_le()))?;
        }

        write_le_items(writer, defs_offset, &def_entries, DefEntry::to_le)?;
        write_le_items(writer, defs_by_hash_offset, &def_hashes, HashIndexEntry::to_le)?;
        write_le_items(writer, refs_offset, &ref_entries, RefEntry::to_le)?;
        write_le_items(writer, refs_by_target_offset, &ref_hashes, HashIndexEntry::to_le)?;
        write_le_items(writer, types_offset, &type_entries, TypeEntry::to_le)?;
        write_le_items(writer, defs_by_owner_offset, &owner_entries, OwnerIndexEntry::to_le)?;
        write_le_items(writer, derefs_offset, &deref_entries, DerefEntry::to_le)?;
        write_le_items(writer, notes_offset, &note_entries, NoteEntry::to_le)?;
        self.write_typed_sections(writer, &typed_sections)?;

        writer.flush()?;
        Ok(())
    }

    pub(super) fn get_interned_str(&self, offset: u32) -> &[u8] {
        let start = offset as usize;
        if start >= self.string_pool.len() {
            return b"";
        }
        let end = self.string_pool[start..].iter().position(|&b| b == 0).map(|p| start + p).unwrap_or(self.string_pool.len());
        &self.string_pool[start..end]
    }
}

pub(super) fn write_le_items<W: Write + Seek, T: Copy, F: Fn(T) -> T>(writer: &mut W, offset: u64, items: &[T], to_le: F) -> io::Result<()> {
    writer.seek(io::SeekFrom::Start(offset))?;
    for &item in items {
        let le = to_le(item);
        writer.write_all(as_byte_slice(&le))?;
    }
    Ok(())
}

#[inline]
pub(super) fn align_up(offset: u64, align: u64) -> u64 {
    (offset + align - 1) & !(align - 1)
}

#[inline]
fn as_byte_slice<T: Sized>(val: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts((val as *const T) as *const u8, std::mem::size_of::<T>()) }
}
