/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Builder and serializer for the RIDX binary index format.

use std::collections::{HashMap, HashSet};
use std::io::{self, Seek, Write};

use crate::format::*;

/// Raw definition to be added to the index builder.
#[derive(Debug, Clone)]
pub struct RawDefinition {
    pub def_id: u32,
    pub kind: DefKind,
    pub visibility: u8,
    pub is_macro_expansion: bool,
    pub name: String,
    pub path: String,
    pub signature: Option<String>,
    pub file: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

/// Raw reference to be added to the index builder.
#[derive(Debug, Clone)]
pub struct RawReference {
    pub file: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub target_def_id: Option<u32>,
    pub target_path: String,
    pub target_file: Option<String>,
    pub target_unit: Option<String>,
    pub target_line: u32,
    pub target_col: u32,
    pub kind: DefKind,
    pub is_write: bool,
    pub is_macro_expansion: bool,
}

/// Raw expression type to be added to the index builder.
#[derive(Debug, Clone)]
pub struct RawType {
    pub file: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub type_name: String,
}

/// In-memory builder for constructing and serializing `.ridx` files.
pub struct IndexWriter {
    crate_name: String,
    crate_version: String,

    /// Interned string bytes (null-terminated UTF-8 strings)
    string_pool: Vec<u8>,
    string_map: HashMap<String, u32>,

    /// Files table: maps file path string to file ID
    file_map: HashMap<String, u32>,
    file_offsets: Vec<u32>,

    definitions: Vec<RawDefinition>,
    def_owners: Vec<(u32, String)>,
    references: Vec<RawReference>,
    types: Vec<(RawType, Option<String>)>,
    derefs: Vec<(String, String)>,
    unit_key: Option<String>,
    generation: Option<String>,
    file_hashes: HashMap<String, u64>,
    reexport_indices: HashSet<u32>,
}

impl IndexWriter {
    pub fn new(crate_name: impl Into<String>, crate_version: impl Into<String>) -> Self {
        let mut writer = Self {
            crate_name: crate_name.into(),
            crate_version: crate_version.into(),
            string_pool: Vec::with_capacity(64 * 1024),
            string_map: HashMap::new(),
            file_map: HashMap::new(),
            file_offsets: Vec::new(),
            definitions: Vec::new(),
            def_owners: Vec::new(),
            references: Vec::new(),
            types: Vec::new(),
            derefs: Vec::new(),
            unit_key: None,
            generation: None,
            file_hashes: HashMap::new(),
            reexport_indices: HashSet::new(),
        };
        // Offset 0 is reserved for empty string
        writer.string_pool.push(0);
        writer.string_map.insert(String::new(), 0);
        writer
    }
}

impl Default for IndexWriter {
    fn default() -> Self {
        Self::new("", "")
    }
}

impl IndexWriter {
    /// Sets the compilation unit key for this index shard.
    pub fn set_unit_key(&mut self, unit_key: impl Into<String>) {
        self.unit_key = Some(unit_key.into());
    }

    /// Returns the compilation unit key if set.
    pub fn unit_key(&self) -> Option<&str> {
        self.unit_key.as_deref()
    }

    /// Sets the source snapshot generation digest for this index shard.
    pub fn set_generation(&mut self, generation: impl Into<String>) {
        self.generation = Some(generation.into());
    }

    /// Returns the source snapshot generation digest if set.
    pub fn generation(&self) -> Option<&str> {
        self.generation.as_deref()
    }

    /// Sets the source file provenance hashes for files indexed in this shard.
    pub fn set_file_hashes(&mut self, hashes: HashMap<String, u64>) {
        self.file_hashes = hashes;
    }

    /// Records or updates the source text provenance hash for a single indexed file.
    pub fn add_file_hash(&mut self, file: &str, hash: u64) {
        self.file_hashes.insert(file.to_string(), hash);
    }

    /// Intern a string into the pool, returning its byte offset.
    /// Offset 0 is always reserved for the empty string (and used as a sentinel for 'no owner').
    pub fn intern_string(&mut self, s: &str) -> u32 {
        if self.string_pool.is_empty() {
            self.string_pool.push(0);
            self.string_map.insert(String::new(), 0);
        }
        if let Some(&offset) = self.string_map.get(s) {
            return offset;
        }
        let offset = self.string_pool.len() as u32;
        self.string_pool.extend_from_slice(s.as_bytes());
        self.string_pool.push(0); // Null terminator
        self.string_map.insert(s.to_string(), offset);
        offset
    }

    /// Get or assign a numeric ID for a file path.
    pub fn get_or_insert_file(&mut self, path: &str) -> u32 {
        if let Some(&id) = self.file_map.get(path) {
            return id;
        }
        let id = self.file_offsets.len() as u32;
        let offset = self.intern_string(path);
        self.file_offsets.push(offset);
        self.file_map.insert(path.to_string(), id);
        id
    }

    pub fn add_definition(&mut self, def: RawDefinition) {
        let idx = self.definitions.len() as u32;
        if let Some((parent, _)) = def.path.rsplit_once("::") {
            if !parent.is_empty() {
                self.def_owners.push((idx, parent.to_string()));
            }
        }
        self.definitions.push(def);
    }

    pub fn add_definition_with_owners(&mut self, def: RawDefinition, owners: Vec<String>) {
        let idx = self.definitions.len() as u32;
        for owner in owners {
            if !owner.is_empty() {
                self.def_owners.push((idx, owner));
            }
        }
        // Also add module parent if not present
        if let Some((parent, _)) = def.path.rsplit_once("::") {
            if !parent.is_empty() {
                self.def_owners.push((idx, parent.to_string()));
            }
        }
        self.definitions.push(def);
    }

    /// Adds a definition marked with FLAG_REEXPORT and associates it with owners.
    pub fn add_reexport_with_owners(&mut self, def: RawDefinition, owners: Vec<String>) {
        let idx = self.definitions.len() as u32;
        self.reexport_indices.insert(idx);
        self.add_definition_with_owners(def, owners);
    }

    pub fn add_def_owner(&mut self, def_id: u32, owner: String) {
        if let Some(idx) = self.definitions.iter().position(|d| d.def_id == def_id) {
            self.def_owners.push((idx as u32, owner));
        }
    }

    pub fn add_def_owner_by_index(&mut self, idx: u32, owner: String) {
        debug_assert!((idx as usize) < self.definitions.len(), "def index {idx} out of bounds");
        if (idx as usize) < self.definitions.len() && !owner.is_empty() {
            self.def_owners.push((idx, owner));
        }
    }

    pub fn add_reference(&mut self, r: RawReference) {
        self.references.push(r);
    }

    pub fn add_type(&mut self, t: RawType) {
        self.types.push((t, None));
    }

    pub fn add_type_with_owner(&mut self, t: RawType, owner_path: Option<String>) {
        self.types.push((t, owner_path));
    }

    pub fn add_deref(&mut self, source_type: impl Into<String>, target_type: impl Into<String>) {
        self.derefs.push((source_type.into(), target_type.into()));
    }

    /// Total count of definitions queued.
    pub fn definition_count(&self) -> usize {
        self.definitions.len()
    }

    /// Total count of references queued.
    pub fn reference_count(&self) -> usize {
        self.references.len()
    }

    /// Total count of types queued.
    pub fn type_count(&self) -> usize {
        self.types.len()
    }

    /// Total count of deref mappings queued.
    pub fn deref_count(&self) -> usize {
        self.derefs.len()
    }

    /// Returns all unique file paths known to this writer.
    pub fn indexed_files(&self) -> Vec<String> {
        let mut set = std::collections::HashSet::new();
        for k in self.file_map.keys() {
            set.insert(k.clone());
        }
        for def in &self.definitions {
            set.insert(def.file.clone());
        }
        for r in &self.references {
            set.insert(r.file.clone());
        }
        for t in &self.types {
            set.insert(t.0.file.clone());
        }
        let mut list: Vec<String> = set.into_iter().collect();
        list.sort();
        list
    }

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
                span: Span::new(
                    file_id,
                    def.start_line,
                    def.start_col,
                    def.end_line,
                    def.end_col,
                ),
            };
            def_entries.push(entry);
        }

        // Build defs hash index
        for (i, def) in definitions.iter().enumerate() {
            let hash = fnv1a_64(def.path.as_bytes());
            def_hashes.push(HashIndexEntry {
                hash,
                index: i as u32,
                _pad: [0; 4],
            });
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
                deref_entries.push(DerefEntry::new(
                    source_type_hash,
                    source_type_offset,
                    target_type_offset,
                ));
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
                flags: if r.is_macro_expansion {
                    FLAG_MACRO_EXPANSION
                } else {
                    0
                },
                _pad: 0,
            };
            ref_entries.push(entry);
        }

        // Sort references by source span for binary search position lookup
        ref_entries.sort_by(|a, b| {
            (a.span.file_id, a.span.start_line, a.span.start_col).cmp(&(
                b.span.file_id,
                b.span.start_line,
                b.span.start_col,
            ))
        });

        // Build references target path hash index
        let mut ref_hashes = Vec::with_capacity(ref_entries.len());
        for (i, r) in ref_entries.iter().enumerate() {
            let target_path_bytes = self.get_interned_str(r.target_path_offset);
            let hash = fnv1a_64(target_path_bytes);
            ref_hashes.push(HashIndexEntry {
                hash,
                index: i as u32,
                _pad: [0; 4],
            });
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

            let entry = TypeEntry::new(
                Span::new(file_id, t.start_line, t.start_col, t.end_line, t.end_col),
                type_offset,
                owner_path_offset,
            );
            type_entries.push(entry);
        }

        // Sort types by source span
        type_entries.sort_by(|a, b| {
            (a.span.file_id, a.span.start_line, a.span.start_col).cmp(&(
                b.span.file_id,
                b.span.start_line,
                b.span.start_col,
            ))
        });

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
        let files_len =
            (self.file_offsets.len() * std::mem::size_of::<crate::format::FileEntry>()) as u64;
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
        let defs_by_owner_len =
            (owner_entries.len() * std::mem::size_of::<OwnerIndexEntry>()) as u64;
        current_offset = align_up(defs_by_owner_offset + defs_by_owner_len, 8);

        let derefs_offset = current_offset;
        let _derefs_len = (deref_entries.len() * std::mem::size_of::<DerefEntry>()) as u64;

        let header = Header {
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
        };

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

        // Write Definitions Table (in little-endian format)
        writer.seek(io::SeekFrom::Start(defs_offset))?;
        for entry in &def_entries {
            let entry_le = entry.to_le();
            writer.write_all(as_byte_slice(&entry_le))?;
        }

        // Write Definitions Hash Index (in little-endian format)
        writer.seek(io::SeekFrom::Start(defs_by_hash_offset))?;
        for entry in &def_hashes {
            let entry_le = entry.to_le();
            writer.write_all(as_byte_slice(&entry_le))?;
        }

        // Write References Table (in little-endian format)
        writer.seek(io::SeekFrom::Start(refs_offset))?;
        for entry in &ref_entries {
            let entry_le = entry.to_le();
            writer.write_all(as_byte_slice(&entry_le))?;
        }

        // Write References Target Hash Index (in little-endian format)
        writer.seek(io::SeekFrom::Start(refs_by_target_offset))?;
        for entry in &ref_hashes {
            let entry_le = entry.to_le();
            writer.write_all(as_byte_slice(&entry_le))?;
        }

        // Write Types Table (in little-endian format)
        writer.seek(io::SeekFrom::Start(types_offset))?;
        for entry in &type_entries {
            let entry_le = entry.to_le();
            writer.write_all(as_byte_slice(&entry_le))?;
        }

        // Write Definitions By Owner Table (in little-endian format)
        writer.seek(io::SeekFrom::Start(defs_by_owner_offset))?;
        for entry in &owner_entries {
            let entry_le = entry.to_le();
            writer.write_all(as_byte_slice(&entry_le))?;
        }

        // Write Derefs Table (in little-endian format)
        writer.seek(io::SeekFrom::Start(derefs_offset))?;
        for entry in &deref_entries {
            let entry_le = entry.to_le();
            writer.write_all(as_byte_slice(&entry_le))?;
        }

        writer.flush()?;
        Ok(())
    }

    fn get_interned_str(&self, offset: u32) -> &[u8] {
        let start = offset as usize;
        if start >= self.string_pool.len() {
            return b"";
        }
        let end = self.string_pool[start..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| start + p)
            .unwrap_or(self.string_pool.len());
        &self.string_pool[start..end]
    }
}

#[inline]
fn align_up(offset: u64, align: u64) -> u64 {
    (offset + align - 1) & !(align - 1)
}

#[inline]
fn as_byte_slice<T: Sized>(val: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts((val as *const T) as *const u8, std::mem::size_of::<T>()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::IndexReader;

    #[test]
    fn test_align_up_and_helpers() {
        assert_eq!(align_up(0, 8), 0);
        assert_eq!(align_up(1, 8), 8);
        assert_eq!(align_up(7, 8), 8);
        assert_eq!(align_up(8, 8), 8);
        assert_eq!(align_up(9, 8), 16);

        let writer = IndexWriter::new("test", "0.1.0");
        assert_eq!(writer.get_interned_str(99999), b"");
    }

    #[test]
    fn test_writer_reader_roundtrip() {
        let mut writer = IndexWriter::new("roundtrip_crate", "1.2.3");
        writer.set_unit_key("unit-xyz");
        writer.set_generation("gen-abc");
        writer.add_file_hash("src/lib.rs", 123456789);

        writer.get_or_insert_file("src/lib.rs");
        writer.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 1,
            is_macro_expansion: false,
            name: "my_fn".to_string(),
            path: "roundtrip_crate::my_fn".to_string(),
            signature: Some("pub fn my_fn()".to_string()),
            file: "src/lib.rs".to_string(),
            start_line: 10,
            start_col: 1,
            end_line: 12,
            end_col: 2,
        });

        writer.add_reference(RawReference {
            file: "src/lib.rs".to_string(),
            start_line: 20,
            start_col: 5,
            end_line: 20,
            end_col: 10,
            target_def_id: Some(1),
            target_path: "roundtrip_crate::my_fn".to_string(),
            target_file: Some("src/lib.rs".to_string()),
            target_unit: Some("unit-xyz".to_string()),
            target_line: 10,
            target_col: 1,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });

        writer.add_type(RawType {
            file: "src/lib.rs".to_string(),
            start_line: 20,
            start_col: 5,
            end_line: 20,
            end_col: 10,
            type_name: "()".to_string(),
        });

        assert_eq!(writer.unit_key(), Some("unit-xyz"));
        assert_eq!(writer.generation(), Some("gen-abc"));
        writer.get_or_insert_file("extra/file.rs");
        assert!(writer
            .indexed_files()
            .contains(&"extra/file.rs".to_string()));

        // Test macro expansion definition (line 249)
        writer.add_definition(RawDefinition {
            def_id: 2,
            kind: DefKind::Macro,
            visibility: 0,
            is_macro_expansion: true,
            name: "macro_def".to_string(),
            path: "roundtrip_crate::macro_def".to_string(),
            signature: None,
            file: "src/lib.rs".to_string(),
            start_line: 20,
            start_col: 1,
            end_line: 20,
            end_col: 10,
        });

        let mut buf = std::io::Cursor::new(Vec::new());
        writer.write_to(&mut buf).unwrap();

        let reader = IndexReader::from_buffer(buf.into_inner()).unwrap();
        assert_eq!(reader.crate_name(), "roundtrip_crate");
        assert_eq!(reader.crate_version(), "1.2.3");
        assert_eq!(reader.unit_key(), "unit-xyz");
        assert_eq!(reader.generation(), "gen-abc");
        assert_eq!(reader.file_source_hash(0), 123456789);

        let defs = reader.definitions();
        assert_eq!(defs.len(), 2);
        assert_eq!(reader.get_str(defs[0].name_offset()), "my_fn");
        assert!(defs[1].is_macro_expansion());

        let refs = reader.references();
        assert_eq!(refs.len(), 1);

        let types = reader.types();
        assert_eq!(types.len(), 1);
        assert_eq!(reader.get_str(types[0].type_offset()), "()");
    }
}
