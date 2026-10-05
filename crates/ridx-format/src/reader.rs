//! Fast zero-copy memory-mapped reader for .ridx files.
#![allow(clippy::manual_is_multiple_of)]

use std::fs::File;
use std::io;
use std::path::Path;
use memmap2::Mmap;

use crate::format::*;

/// Representation of a found definition target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDefinition<'a> {
    pub name: &'a str,
    pub path: &'a str,
    pub kind: DefKind,
    pub signature: Option<&'a str>,
    pub file: &'a str,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

/// Zero-copy reader over a memory-mapped or buffered `.ridx` index.
pub struct IndexReader<B> {
    buffer: B,
    header: Header,
}

impl IndexReader<Mmap> {
    /// Opens and memory-maps a `.ridx` file from disk.
    pub fn open(path: impl AsRef<Path>) -> io::Result<IndexReader<Mmap>> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        Self::from_buffer(mmap)
    }
}

impl<B: AsRef<[u8]>> IndexReader<B> {
    /// Wraps any byte buffer as an IndexReader.
    pub fn from_buffer(buffer: B) -> io::Result<Self> {
        let bytes = buffer.as_ref();
        if (bytes.as_ptr() as usize) % 8 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Buffer base pointer must be 8-byte aligned for zero-copy RIDX reading",
            ));
        }
        if bytes.len() < std::mem::size_of::<Header>() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "File too small for RIDX header",
            ));
        }

        // Safely decode header into aligned memory without unaligned pointer casting
        let mut header_buf = [0u8; std::mem::size_of::<Header>()];
        header_buf.copy_from_slice(&bytes[..std::mem::size_of::<Header>()]);
        let header: Header = unsafe { std::mem::transmute(header_buf) };

        if header.magic != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid RIDX magic: {:?}", header.magic),
            ));
        }
        if header.version != CURRENT_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unsupported RIDX version: {}", header.version),
            ));
        }

        validate_sections(bytes.len(), &header)?;

        Ok(Self { buffer, header })
    }

    #[inline]
    fn bytes(&self) -> &[u8] {
        self.buffer.as_ref()
    }

    #[inline]
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Crate name stored in the index.
    pub fn crate_name(&self) -> &str {
        self.get_str(self.header.crate_name_offset)
    }

    /// Crate version stored in the index.
    pub fn crate_version(&self) -> &str {
        self.get_str(self.header.crate_version_offset)
    }

    /// Retrieves an interned null-terminated string at the specified offset.
    #[inline]
    pub fn get_str(&self, offset: u32) -> &str {
        let pool_start = self.header.string_pool_offset as usize;
        let pool_len = self.header.string_pool_len as usize;
        let off = offset as usize;
        if off >= pool_len {
            return "";
        }
        let bytes = self.bytes();
        let start = pool_start + off;
        let max_end = (pool_start + pool_len).min(bytes.len());
        if start >= max_end {
            return "";
        }
        let end = bytes[start..max_end]
            .iter()
            .position(|&b| b == 0)
            .map(|p| start + p)
            .unwrap_or(max_end);

        std::str::from_utf8(&bytes[start..end]).unwrap_or("")
    }

    /// Returns the file path for a given file_id.
    pub fn get_file(&self, file_id: u32) -> &str {
        if file_id >= self.header.files_count {
            return "";
        }
        let offset_in_file_table =
            (self.header.files_offset as usize) + (file_id as usize) * std::mem::size_of::<u32>();
        let bytes = self.bytes();
        if offset_in_file_table + 4 > bytes.len() {
            return "";
        }
        let str_offset = u32::from_le_bytes(
            bytes[offset_in_file_table..offset_in_file_table + 4]
                .try_into()
                .unwrap(),
        );
        self.get_str(str_offset)
    }

    /// Look up the file_id for a given path substring or exact match.
    pub fn find_file_id(&self, file_path: &str) -> Option<u32> {
        for id in 0..self.header.files_count {
            let f = self.get_file(id);
            if f == file_path || f.ends_with(file_path) || file_path.ends_with(f) {
                return Some(id);
            }
        }
        None
    }

    /// Slice of all definitions in the index.
    #[inline]
    pub fn definitions(&self) -> &[DefEntry] {
        let header = self.header();
        let start = header.defs_offset as usize;
        let count = header.defs_count as usize;
        let len = count * std::mem::size_of::<DefEntry>();
        let bytes = self.bytes();
        if start + len > bytes.len() {
            return &[];
        }
        unsafe {
            std::slice::from_raw_parts(bytes[start..].as_ptr() as *const DefEntry, count)
        }
    }

    #[inline]
    fn defs_by_hash(&self) -> &[HashIndexEntry] {
        let header = self.header();
        let start = header.defs_by_hash_offset as usize;
        let count = header.defs_by_hash_count as usize;
        let len = count * std::mem::size_of::<HashIndexEntry>();
        let bytes = self.bytes();
        if start + len > bytes.len() {
            return &[];
        }
        unsafe {
            std::slice::from_raw_parts(bytes[start..].as_ptr() as *const HashIndexEntry, count)
        }
    }

    /// Slice of all references in the index (sorted by source span).
    #[inline]
    pub fn references(&self) -> &[RefEntry] {
        let header = self.header();
        let start = header.refs_offset as usize;
        let count = header.refs_count as usize;
        let len = count * std::mem::size_of::<RefEntry>();
        let bytes = self.bytes();
        if start + len > bytes.len() {
            return &[];
        }
        unsafe {
            std::slice::from_raw_parts(bytes[start..].as_ptr() as *const RefEntry, count)
        }
    }

    #[inline]
    fn refs_by_target(&self) -> &[HashIndexEntry] {
        let header = self.header();
        let start = header.refs_by_target_offset as usize;
        let count = header.refs_by_target_count as usize;
        let len = count * std::mem::size_of::<HashIndexEntry>();
        let bytes = self.bytes();
        if start + len > bytes.len() {
            return &[];
        }
        unsafe {
            std::slice::from_raw_parts(bytes[start..].as_ptr() as *const HashIndexEntry, count)
        }
    }

    /// Slice of all expression types in the index (sorted by source span).
    #[inline]
    pub fn types(&self) -> &[TypeEntry] {
        let header = self.header();
        let start = header.types_offset as usize;
        let count = header.types_count as usize;
        let len = count * std::mem::size_of::<TypeEntry>();
        let bytes = self.bytes();
        if start + len > bytes.len() {
            return &[];
        }
        unsafe {
            std::slice::from_raw_parts(bytes[start..].as_ptr() as *const TypeEntry, count)
        }
    }

    /// Find a definition by its canonical path (e.g. `stdx::panic_context::enter`).
    pub fn find_definition_by_path(&self, path: &str) -> Option<&DefEntry> {
        let hash = fnv1a_64(path.as_bytes());
        let defs_hash = self.defs_by_hash();
        let defs = self.definitions();

        let partition = defs_hash.binary_search_by_key(&hash, |e| e.hash);
        if let Ok(idx) = partition {
            // Check matching hash range
            let mut i = idx;
            while i > 0 && defs_hash[i - 1].hash == hash {
                i -= 1;
            }
            while i < defs_hash.len() && defs_hash[i].hash == hash {
                let def = &defs[defs_hash[i].index as usize];
                if self.get_str(def.path_offset) == path {
                    return Some(def);
                }
                i += 1;
            }
        }
        None
    }

    /// Go to definition at a given cursor position `(file, line, col)`.
    pub fn find_definition_at(
        &self,
        file: &str,
        line: u32,
        col: u32,
    ) -> Option<ResolvedDefinition<'_>> {
        let file_id = self.find_file_id(file)?;

        // 1. Check references: is the cursor on a symbol reference?
        let refs = self.references();
        for r in refs {
            if r.span.file_id == file_id && r.span.contains(file_id, line, col) {
                // If it resolves to a local def_id:
                if r.target_def_id != u32::MAX {
                    let defs = self.definitions();
                    if let Some(def) = defs.iter().find(|d| d.def_id == r.target_def_id) {
                        return Some(ResolvedDefinition {
                            name: self.get_str(def.name_offset),
                            path: self.get_str(def.path_offset),
                            kind: DefKind::from(def.kind),
                            signature: if def.sig_offset != 0 {
                                Some(self.get_str(def.sig_offset))
                            } else {
                                None
                            },
                            file: self.get_file(def.span.file_id),
                            start_line: def.span.start_line,
                            start_col: def.span.start_col,
                            end_line: def.span.end_line,
                            end_col: def.span.end_col,
                        });
                    }
                }
                // Otherwise return target path and location
                let target_path = self.get_str(r.target_path_offset);
                let target_file = self.get_str(r.target_file_offset);
                let target_name = target_path.rsplit("::").next().unwrap_or(target_path);

                return Some(ResolvedDefinition {
                    name: target_name,
                    path: target_path,
                    kind: DefKind::from(r.kind),
                    signature: None,
                    file: if !target_file.is_empty() {
                        target_file
                    } else {
                        "external"
                    },
                    start_line: r.target_line,
                    start_col: r.target_col,
                    end_line: r.target_line,
                    end_col: r.target_col + (target_name.len() as u32),
                });
            }
        }

        // 2. Check if cursor is directly on a definition span
        for def in self.definitions() {
            if def.span.file_id == file_id && def.span.contains(file_id, line, col) {
                return Some(ResolvedDefinition {
                    name: self.get_str(def.name_offset),
                    path: self.get_str(def.path_offset),
                    kind: DefKind::from(def.kind),
                    signature: if def.sig_offset != 0 {
                        Some(self.get_str(def.sig_offset))
                    } else {
                        None
                    },
                    file: self.get_file(def.span.file_id),
                    start_line: def.span.start_line,
                    start_col: def.span.start_col,
                    end_line: def.span.end_line,
                    end_col: def.span.end_col,
                });
            }
        }

        None
    }

    /// Find all references to a given symbol path (e.g. `thread::Builder::allow_leak`).
    pub fn find_references(&self, target_path: &str) -> Vec<&RefEntry> {
        let hash = fnv1a_64(target_path.as_bytes());
        let ref_hashes = self.refs_by_target();
        let refs = self.references();
        let mut results = Vec::new();

        if let Ok(idx) = ref_hashes.binary_search_by_key(&hash, |e| e.hash) {
            let mut i = idx;
            while i > 0 && ref_hashes[i - 1].hash == hash {
                i -= 1;
            }
            while i < ref_hashes.len() && ref_hashes[i].hash == hash {
                let r = &refs[ref_hashes[i].index as usize];
                if self.get_str(r.target_path_offset) == target_path {
                    results.push(r);
                }
                i += 1;
            }
        }
        results
    }

    /// Retrieves the inferred expression type at a cursor position `(file, line, col)`.
    /// When nested expressions overlap, selects the tightest (innermost) span.
    pub fn type_at(&self, file: &str, line: u32, col: u32) -> Option<&str> {
        let file_id = self.find_file_id(file)?;
        let mut best: Option<(&TypeEntry, u64)> = None;

        for entry in self.types() {
            if entry.span.file_id == file_id && entry.span.contains(file_id, line, col) {
                let size = entry.span.approximate_size();
                match best {
                    None => best = Some((entry, size)),
                    Some((_, best_size)) if size < best_size => best = Some((entry, size)),
                    _ => {}
                }
            }
        }

        best.map(|(e, _)| self.get_str(e.type_offset))
    }

    /// Fast symbol search across definitions matching a query string.
    pub fn search_symbols(&self, query: &str) -> Vec<&DefEntry> {
        let mut matches = Vec::new();
        let q = query.to_lowercase();
        for def in self.definitions() {
            let name = self.get_str(def.name_offset);
            let path = self.get_str(def.path_offset);
            if name.to_lowercase().contains(&q) || path.to_lowercase().contains(&q) {
                matches.push(def);
            }
        }
        matches
    }
}

fn validate_sections(buf_len: usize, header: &Header) -> io::Result<()> {
    // 1. String pool alignment and bounds
    if (header.string_pool_offset as usize) % 8 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "String pool offset is not 8-byte aligned",
        ));
    }
    let pool_start = header.string_pool_offset as usize;
    let pool_len = header.string_pool_len as usize;
    let pool_end = pool_start.checked_add(pool_len).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "String pool length overflow")
    })?;
    if pool_end > buf_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "String pool extends beyond file bounds",
        ));
    }

    if (header.crate_name_offset as usize) >= pool_len
        || (header.crate_version_offset as usize) >= pool_len
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Crate metadata string offsets exceed string pool bounds",
        ));
    }

    // 2. Files table
    check_section(
        buf_len,
        header.files_offset,
        header.files_count,
        std::mem::size_of::<u32>(),
        4,
        "Files table",
    )?;

    // 3. Definitions table
    check_section(
        buf_len,
        header.defs_offset,
        header.defs_count,
        std::mem::size_of::<DefEntry>(),
        4,
        "Definitions table",
    )?;

    // 4. Definitions hash index
    check_section(
        buf_len,
        header.defs_by_hash_offset,
        header.defs_by_hash_count,
        std::mem::size_of::<HashIndexEntry>(),
        8,
        "Definitions hash index",
    )?;

    // 5. References table
    check_section(
        buf_len,
        header.refs_offset,
        header.refs_count,
        std::mem::size_of::<RefEntry>(),
        4,
        "References table",
    )?;

    // 6. References target hash index
    check_section(
        buf_len,
        header.refs_by_target_offset,
        header.refs_by_target_count,
        std::mem::size_of::<HashIndexEntry>(),
        8,
        "References target hash index",
    )?;

    // 7. Types table
    check_section(
        buf_len,
        header.types_offset,
        header.types_count,
        std::mem::size_of::<TypeEntry>(),
        4,
        "Types table",
    )?;

    Ok(())
}

fn check_section(
    buf_len: usize,
    offset: u64,
    count: u32,
    elem_size: usize,
    required_align: usize,
    section_name: &str,
) -> io::Result<()> {
    let off = offset as usize;
    if off % required_align != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{section_name} offset {offset} is not aligned to {required_align} bytes"),
        ));
    }
    let total_bytes = (count as usize).checked_mul(elem_size).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{section_name} size calculation overflow"),
        )
    })?;
    let end = off.checked_add(total_bytes).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{section_name} end offset overflow"),
        )
    })?;
    if end > buf_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{section_name} extends beyond file bounds ({end} > {buf_len})"),
        ));
    }
    Ok(())
}
