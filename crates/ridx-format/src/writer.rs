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

use crate::format::*;

mod encode;

#[cfg(test)]
use self::encode::align_up;
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

/// Raw note to be added to the index builder: a text about a span (see `NOTE_CALL_ARGS`, `NOTE_LAYOUT`).
#[derive(Debug, Clone)]
pub struct RawNote {
    pub file: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub kind: u32,
    pub text: String,
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
    notes: Vec<RawNote>,
    ty_interner: crate::ty_arena::TyInterner,
    fn_sigs: Vec<FnSigEntry>,
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
            notes: Vec::new(),
            ty_interner: crate::ty_arena::TyInterner::new(),
            fn_sigs: Vec::new(),
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
        Self::intern_in_pool(&mut self.string_pool, &mut self.string_map, s)
    }

    pub(crate) fn intern_in_pool(string_pool: &mut Vec<u8>, string_map: &mut HashMap<String, u32>, s: &str) -> u32 {
        if string_pool.is_empty() {
            string_pool.push(0);
            string_map.insert(String::new(), 0);
        }
        if let Some(&offset) = string_map.get(s) {
            return offset;
        }
        let offset = string_pool.len() as u32;
        string_pool.extend_from_slice(s.as_bytes());
        string_pool.push(0); // Null terminator
        string_map.insert(s.to_string(), offset);
        offset
    }

    /// Interns a local or external definition handle inside this shard's `TyArena`.
    pub fn intern_ty_def(&mut self, def: crate::ty_arena::RawTyDef<'_>) -> TyDefId {
        let pool = &mut self.string_pool;
        let map = &mut self.string_map;
        self.ty_interner.intern_def(def, |s| Self::intern_in_pool(pool, map, s))
    }

    /// Interns a structured, `Infer`-free type inside this shard's `TyArena`.
    pub fn intern_ty(&mut self, ty: crate::ty_arena::RawTy<'_>) -> TyId {
        let pool = &mut self.string_pool;
        let map = &mut self.string_map;
        self.ty_interner.intern_ty(ty, |s| Self::intern_in_pool(pool, map, s))
    }

    /// Total count of deduplicated `TyEntry` nodes in `TyArena`.
    pub fn ty_entry_count(&self) -> usize {
        self.ty_interner.entries().len()
    }

    /// Read-only access to the `TyInterner` accumulated on this writer.
    pub fn ty_interner(&self) -> &crate::ty_arena::TyInterner {
        &self.ty_interner
    }

    /// Records a structured function/method signature for `def_index`.
    pub fn add_fn_sig(&mut self, sig: crate::ty_arena::RawFnSig<'_>) {
        let (inputs_start, inputs_len) = self.ty_interner.intern_ty_list(sig.inputs);
        self.fn_sigs.push(FnSigEntry { def_index: sig.def_index, inputs_start, inputs_len, flags: sig.flags, abi: sig.abi as u8, output: sig.output.0 });
    }

    /// Total count of structured function signatures queued.
    pub fn fn_sig_count(&self) -> usize {
        self.fn_sigs.len()
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

    pub fn add_definition(&mut self, def: RawDefinition) -> u32 {
        let idx = self.definitions.len() as u32;
        if let Some((parent, _)) = def.path.rsplit_once("::") {
            if !parent.is_empty() {
                self.def_owners.push((idx, parent.to_string()));
            }
        }
        self.definitions.push(def);
        idx
    }

    pub fn add_definition_with_owners(&mut self, def: RawDefinition, owners: Vec<String>) -> u32 {
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
        idx
    }

    /// Adds a definition marked with FLAG_REEXPORT and associates it with owners.
    pub fn add_reexport_with_owners(&mut self, def: RawDefinition, owners: Vec<String>) -> u32 {
        let idx = self.definitions.len() as u32;
        self.reexport_indices.insert(idx);
        self.add_definition_with_owners(def, owners)
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

    pub fn add_note(&mut self, note: RawNote) {
        self.notes.push(note);
    }

    /// Total count of notes queued.
    pub fn note_count(&self) -> usize {
        self.notes.len()
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
}

#[cfg(test)]
mod tests;
