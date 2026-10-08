/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Binary format specifications for the RIDX (.ridx) index file.
//!
//! All integer values are stored in little-endian format.
//! Structures are designed for zero-copy memory-mapped reading with natural alignment.

pub const MAGIC: [u8; 4] = *b"RIDX";
pub const CURRENT_VERSION: u32 = 2;
pub const CURRENT_FORMAT_VERSION: u32 = 2;
pub const EXTRACTOR_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Returns the 64-bit fingerprint of the active `ridx-driver` binary and runtime environment.
///
/// Precedence:
/// 1. `$RIDX_DRIVER_HASH`: explicit hex or string override (for tests and deterministic builds).
/// 2. `$RIDX_DRIVER_BIN`: path to driver binary if provided; hashes binary contents.
/// 3. Current executable binary: hashes `/proc/self/exe` or `current_exe()`.
/// 4. Fallback: hash of `CARGO_PKG_VERSION` + format version.
pub fn get_driver_hash() -> u64 {
    if let Ok(override_val) = std::env::var("RIDX_DRIVER_HASH") {
        let trimmed = override_val.trim();
        if let Ok(val) = u64::from_str_radix(trimmed.trim_start_matches("0x"), 16) {
            return val;
        }
        return fnv1a_64(trimmed.as_bytes());
    }
    if let Ok(driver_bin) = std::env::var("RIDX_DRIVER_BIN") {
        let p = std::path::Path::new(driver_bin.trim());
        if p.is_file() {
            if let Ok(bytes) = std::fs::read(p) {
                return fnv1a_64(&bytes);
            }
        }
    }
    if let Ok(exe_path) = std::env::current_exe() {
        if exe_path.is_file() {
            if let Ok(bytes) = std::fs::read(&exe_path) {
                return fnv1a_64(&bytes);
            }
        }
    }
    let fallback = format!("{}:{}", CURRENT_FORMAT_VERSION, EXTRACTOR_VERSION);
    fnv1a_64(fallback.as_bytes())
}

/// Resolves the global RIDX shared cache directory for external crate dependencies.
/// Uses `$RIDX_CACHE_DIR` if set, or platform default:
/// - Unix: `$XDG_CACHE_HOME/ridx/v{CURRENT_VERSION}` or `$HOME/.cache/ridx/v{CURRENT_VERSION}`
/// - Windows: `%LOCALAPPDATA%/ridx/v{CURRENT_VERSION}` or `$USERPROFILE/.cache/ridx/v{CURRENT_VERSION}`
pub fn resolve_cache_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("RIDX_CACHE_DIR") {
        let p = std::path::PathBuf::from(dir.trim());
        if !p.as_os_str().is_empty() {
            return p;
        }
    }
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        let p = std::path::PathBuf::from(xdg.trim());
        if !p.as_os_str().is_empty() {
            return p.join("ridx").join(format!("v{}", CURRENT_VERSION));
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let p = std::path::PathBuf::from(home.trim());
        if !p.as_os_str().is_empty() {
            return p
                .join(".cache")
                .join("ridx")
                .join(format!("v{}", CURRENT_VERSION));
        }
    }
    #[cfg(windows)]
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let p = std::path::PathBuf::from(local_app_data.trim());
        if !p.as_os_str().is_empty() {
            return p.join("ridx").join(format!("v{}", CURRENT_VERSION));
        }
    }
    std::path::PathBuf::from(".ridx_cache").join(format!("v{}", CURRENT_VERSION))
}

/// Returns the path to an object shard inside the global cache for a given unit key.
pub fn cache_object_path(cache_dir: &std::path::Path, unit_key: &str) -> std::path::PathBuf {
    let prefix = if unit_key.len() >= 2 {
        &unit_key[..2]
    } else {
        "00"
    };
    cache_dir
        .join("objects")
        .join(prefix)
        .join(format!("{}.ridx", unit_key))
}

/// Joins a base root directory with a relative path, resolving any overlapping path prefix.
///
/// For instance, if `root` is `/workspace/crates/app` and `rel` is `crates/app/src/lib.rs`,
/// naive joining yields `/workspace/crates/app/crates/app/src/lib.rs`.
/// This helper strips the overlapping `crates/app` prefix to yield `/workspace/crates/app/src/lib.rs`.
pub fn join_resolving_overlap(root: &std::path::Path, rel: &std::path::Path) -> std::path::PathBuf {
    if rel.is_absolute() {
        return rel.to_path_buf();
    }
    let direct = root.join(rel);
    if direct.exists() {
        return direct;
    }
    let rel_comps: Vec<_> = rel.components().collect();
    for i in (1..rel_comps.len()).rev() {
        let prefix: std::path::PathBuf = rel_comps[..i].iter().collect();
        if root.ends_with(&prefix) {
            let stripped: std::path::PathBuf = rel_comps[i..].iter().collect();
            let cand = root.join(stripped);
            return cand;
        }
    }
    direct
}

/// Computes a 64-bit FNV-1a hash over source bytes after stripping UTF-8 BOM and normalizing CRLF to LF,
/// exactly matching rustc's internal SourceFile.src normalization.
pub fn normalized_source_hash(bytes: &[u8]) -> u64 {
    let b = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    };
    let mut normalized = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\r' && i + 1 < b.len() && b[i + 1] == b'\n' {
            i += 1;
            continue;
        }
        normalized.push(b[i]);
        i += 1;
    }
    fnv1a_64(&normalized)
}

/// Compile-time layout guarantees to ensure unambiguous on-disk structure sizes.
const _: () = {
    assert!(std::mem::size_of::<Header>() == 128);
    assert!(std::mem::align_of::<Header>() == 8);
    assert!(std::mem::size_of::<Span>() == 20);
    assert!(std::mem::align_of::<Span>() == 4);
    assert!(std::mem::size_of::<DefEntry>() == 40);
    assert!(std::mem::align_of::<DefEntry>() == 4);
    assert!(std::mem::size_of::<RefEntry>() == 48);
    assert!(std::mem::align_of::<RefEntry>() == 4);
    assert!(std::mem::size_of::<TypeEntry>() == 28);
    assert!(std::mem::align_of::<TypeEntry>() == 4);
    assert!(std::mem::size_of::<HashIndexEntry>() == 16);
    assert!(std::mem::align_of::<HashIndexEntry>() == 8);
    assert!(std::mem::size_of::<FileEntry>() == 16);
    assert!(std::mem::align_of::<FileEntry>() == 8);
};

/// Computes an aggregate 64-bit digest over a map of file paths to their 64-bit source hashes.
pub fn compute_source_snapshot_digest(file_hashes: &std::collections::HashMap<String, u64>) -> u64 {
    let mut entries: Vec<(&String, &u64)> = file_hashes.iter().collect();
    entries.sort_by_key(|(k, _)| *k);
    let mut buf = Vec::new();
    for (k, h) in entries {
        buf.extend_from_slice(k.as_bytes());
        buf.extend_from_slice(&h.to_le_bytes());
    }
    fnv1a_64(&buf)
}

/// Merges embedded file hashes into a file_hashes map, ensuring embedded hashes are authoritative
/// across both package-relative and absolute/canonical aliases under `source_root`, and dropping
/// any conflicting aliases.
pub fn merge_embedded_hashes_authoritative(
    file_hashes: &mut std::collections::HashMap<String, u64>,
    embedded: &std::collections::HashMap<String, u64>,
    source_root: Option<&std::path::Path>,
) {
    // 1. Drop any conflicting aliases in file_hashes
    file_hashes.retain(|k, &mut v| {
        let p = std::path::Path::new(k);
        let rel_match = if let Some(root) = source_root {
            p.strip_prefix(root)
                .ok()
                .map(|r| r.to_string_lossy().to_string())
        } else {
            None
        };
        let target_rel = rel_match.as_deref().unwrap_or(k);
        if let Some(&emb_h) = embedded.get(target_rel).or_else(|| embedded.get(k)) {
            v == emb_h
        } else {
            true
        }
    });

    // 2. Authoritatively insert embedded relative paths and their absolute aliases under source_root
    for (rel_path, &h) in embedded {
        file_hashes.insert(rel_path.clone(), h);
        if let Some(root) = source_root {
            let abs = root.join(rel_path);
            file_hashes.insert(abs.to_string_lossy().to_string(), h);
            if let Ok(canon) = abs.canonicalize() {
                file_hashes.insert(canon.to_string_lossy().to_string(), h);
            }
        }
    }
}

/// File table entry: string pool offset for file path, and 64-bit source text provenance hash.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileEntry {
    pub path_offset: u32,
    pub _pad: u32,
    pub source_hash: u64,
}

impl FileEntry {
    #[inline]
    pub fn new(path_offset: u32, source_hash: u64) -> Self {
        Self {
            path_offset,
            _pad: 0,
            source_hash,
        }
    }

    #[inline]
    pub fn path_offset(&self) -> u32 {
        u32::from_le(self.path_offset)
    }

    #[inline]
    pub fn source_hash(&self) -> u64 {
        u64::from_le(self.source_hash)
    }

    #[inline]
    pub fn to_le(self) -> Self {
        Self {
            path_offset: self.path_offset.to_le(),
            _pad: 0,
            source_hash: self.source_hash.to_le(),
        }
    }
}

/// Fixed-size header at offset 0 of any .ridx file.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Magic signature ("RIDX")
    pub magic: [u8; 4],
    /// Format version (currently 2)
    pub version: u32,
    /// Offset in string pool for crate name
    pub crate_name_offset: u32,
    /// Offset in string pool for crate version
    pub crate_version_offset: u32,

    /// Offset of the string intern pool
    pub string_pool_offset: u64,
    /// Length in bytes of the string intern pool
    pub string_pool_len: u64,

    /// Offset of the file path table (array of FileEntry)
    pub files_offset: u64,
    pub files_count: u32,
    /// Offset in string pool for compilation unit key (or 0)
    pub unit_key_offset: u32,

    /// Offset of the definitions table (array of DefEntry)
    pub defs_offset: u64,
    pub defs_count: u32,
    /// Offset in string pool for source snapshot generation (or 0)
    pub generation_offset: u32,

    /// Offset of the definitions hash index (array of HashIndexEntry sorted by hash)
    pub defs_by_hash_offset: u64,
    pub defs_by_hash_count: u32,
    pub _pad2: u32,

    /// Offset of the references table (array of RefEntry sorted by span)
    pub refs_offset: u64,
    pub refs_count: u32,
    pub _pad3: u32,

    /// Offset of the references-by-target hash index (array of HashIndexEntry sorted by target hash)
    pub refs_by_target_offset: u64,
    pub refs_by_target_count: u32,
    pub _pad4: u32,

    /// Offset of the types table (array of TypeEntry sorted by span)
    pub types_offset: u64,
    pub types_count: u32,
    pub _pad5: u32,
}

impl Header {
    #[inline]
    pub fn to_le(self) -> Self {
        Self {
            magic: self.magic,
            version: self.version.to_le(),
            crate_name_offset: self.crate_name_offset.to_le(),
            crate_version_offset: self.crate_version_offset.to_le(),
            string_pool_offset: self.string_pool_offset.to_le(),
            string_pool_len: self.string_pool_len.to_le(),
            files_offset: self.files_offset.to_le(),
            files_count: self.files_count.to_le(),
            unit_key_offset: self.unit_key_offset.to_le(),
            defs_offset: self.defs_offset.to_le(),
            defs_count: self.defs_count.to_le(),
            generation_offset: self.generation_offset.to_le(),
            defs_by_hash_offset: self.defs_by_hash_offset.to_le(),
            defs_by_hash_count: self.defs_by_hash_count.to_le(),
            _pad2: 0,
            refs_offset: self.refs_offset.to_le(),
            refs_count: self.refs_count.to_le(),
            _pad3: 0,
            refs_by_target_offset: self.refs_by_target_offset.to_le(),
            refs_by_target_count: self.refs_by_target_count.to_le(),
            _pad4: 0,
            types_offset: self.types_offset.to_le(),
            types_count: self.types_count.to_le(),
            _pad5: 0,
        }
    }

    #[inline]
    pub fn from_le(self) -> Self {
        Self {
            magic: self.magic,
            version: u32::from_le(self.version),
            crate_name_offset: u32::from_le(self.crate_name_offset),
            crate_version_offset: u32::from_le(self.crate_version_offset),
            string_pool_offset: u64::from_le(self.string_pool_offset),
            string_pool_len: u64::from_le(self.string_pool_len),
            files_offset: u64::from_le(self.files_offset),
            files_count: u32::from_le(self.files_count),
            unit_key_offset: u32::from_le(self.unit_key_offset),
            defs_offset: u64::from_le(self.defs_offset),
            defs_count: u32::from_le(self.defs_count),
            generation_offset: u32::from_le(self.generation_offset),
            defs_by_hash_offset: u64::from_le(self.defs_by_hash_offset),
            defs_by_hash_count: u32::from_le(self.defs_by_hash_count),
            _pad2: 0,
            refs_offset: u64::from_le(self.refs_offset),
            refs_count: u32::from_le(self.refs_count),
            _pad3: 0,
            refs_by_target_offset: u64::from_le(self.refs_by_target_offset),
            refs_by_target_count: u32::from_le(self.refs_by_target_count),
            _pad4: 0,
            types_offset: u64::from_le(self.types_offset),
            types_count: u32::from_le(self.types_count),
            _pad5: 0,
        }
    }

    #[inline]
    pub fn unit_key_offset(&self) -> u32 {
        u32::from_le(self.unit_key_offset)
    }

    #[inline]
    pub fn generation_offset(&self) -> u32 {
        u32::from_le(self.generation_offset)
    }
}

/// Source location span: file ID and line:column ranges (1-based, inclusive start, exclusive end).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub file_id: u32,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

impl Span {
    #[inline]
    pub fn new(file_id: u32, start_line: u32, start_col: u32, end_line: u32, end_col: u32) -> Self {
        Self {
            file_id,
            start_line,
            start_col,
            end_line,
            end_col,
        }
    }

    #[inline]
    pub fn to_le(self) -> Self {
        Self {
            file_id: self.file_id.to_le(),
            start_line: self.start_line.to_le(),
            start_col: self.start_col.to_le(),
            end_line: self.end_line.to_le(),
            end_col: self.end_col.to_le(),
        }
    }

    #[inline]
    pub fn from_le(self) -> Self {
        Self {
            file_id: u32::from_le(self.file_id),
            start_line: u32::from_le(self.start_line),
            start_col: u32::from_le(self.start_col),
            end_line: u32::from_le(self.end_line),
            end_col: u32::from_le(self.end_col),
        }
    }

    #[inline]
    pub fn file_id(&self) -> u32 {
        u32::from_le(self.file_id)
    }
    #[inline]
    pub fn start_line(&self) -> u32 {
        u32::from_le(self.start_line)
    }
    #[inline]
    pub fn start_col(&self) -> u32 {
        u32::from_le(self.start_col)
    }
    #[inline]
    pub fn end_line(&self) -> u32 {
        u32::from_le(self.end_line)
    }
    #[inline]
    pub fn end_col(&self) -> u32 {
        u32::from_le(self.end_col)
    }

    /// Returns true if this span contains the given (line, col) position in the specified file.
    #[inline]
    pub fn contains(&self, file_id: u32, line: u32, col: u32) -> bool {
        let s = self.from_le();
        if s.file_id != file_id {
            return false;
        }
        if line < s.start_line || line > s.end_line {
            return false;
        }
        if line == s.start_line && col < s.start_col {
            return false;
        }
        if line == s.end_line && col >= s.end_col {
            return false;
        }
        true
    }

    /// Approximate character length metric used for picking the innermost span when nested.
    #[inline]
    pub fn approximate_size(&self) -> u64 {
        let s = self.from_le();
        let lines = (s.end_line.saturating_sub(s.start_line)) as u64;
        if lines == 0 {
            (s.end_col.saturating_sub(s.start_col)) as u64
        } else {
            lines * 80 + (s.end_col as u64)
        }
    }
}

/// Category of definition, matching common Rust HIR DefKind.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DefKind {
    Mod = 0,
    Struct = 1,
    Union = 2,
    Enum = 3,
    Variant = 4,
    Trait = 5,
    TyAlias = 6,
    ForeignTy = 7,
    TraitAlias = 8,
    AssocTy = 9,
    TyParam = 10,
    Fn = 11,
    Const = 12,
    ConstParam = 13,
    Static = 14,
    StructCtor = 15,
    VariantCtor = 16,
    AssocFn = 17,
    AssocConst = 18,
    Macro = 19,
    ExternCrate = 20,
    Use = 21,
    ForeignMod = 22,
    AnonConst = 23,
    InlineConst = 24,
    OpaqueTy = 25,
    Field = 26,
    LifetimeParam = 27,
    GlobalAsm = 28,
    Impl = 29,
    Closure = 30,
    Local = 31,
    #[default]
    Other = 255,
}

impl DefKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            DefKind::Mod => "mod",
            DefKind::Struct => "struct",
            DefKind::Union => "union",
            DefKind::Enum => "enum",
            DefKind::Variant => "variant",
            DefKind::Trait => "trait",
            DefKind::TyAlias => "type",
            DefKind::ForeignTy => "type",
            DefKind::TraitAlias => "trait",
            DefKind::AssocTy => "type",
            DefKind::TyParam => "type",
            DefKind::Fn => "fn",
            DefKind::Const => "const",
            DefKind::ConstParam => "const",
            DefKind::Static => "static",
            DefKind::StructCtor => "struct",
            DefKind::VariantCtor => "variant",
            DefKind::AssocFn => "fn",
            DefKind::AssocConst => "const",
            DefKind::Macro => "macro",
            DefKind::ExternCrate => "extern crate",
            DefKind::Use => "use",
            DefKind::ForeignMod => "mod",
            DefKind::AnonConst => "const",
            DefKind::InlineConst => "const",
            DefKind::OpaqueTy => "type",
            DefKind::Field => "field",
            DefKind::LifetimeParam => "lifetime",
            DefKind::GlobalAsm => "asm",
            DefKind::Impl => "impl",
            DefKind::Closure => "closure",
            DefKind::Local => "let",
            DefKind::Other => "other",
        }
    }
}

impl From<u8> for DefKind {
    fn from(val: u8) -> Self {
        match val {
            0 => DefKind::Mod,
            1 => DefKind::Struct,
            2 => DefKind::Union,
            3 => DefKind::Enum,
            4 => DefKind::Variant,
            5 => DefKind::Trait,
            6 => DefKind::TyAlias,
            7 => DefKind::ForeignTy,
            8 => DefKind::TraitAlias,
            9 => DefKind::AssocTy,
            10 => DefKind::TyParam,
            11 => DefKind::Fn,
            12 => DefKind::Const,
            13 => DefKind::ConstParam,
            14 => DefKind::Static,
            15 => DefKind::StructCtor,
            16 => DefKind::VariantCtor,
            17 => DefKind::AssocFn,
            18 => DefKind::AssocConst,
            19 => DefKind::Macro,
            20 => DefKind::ExternCrate,
            21 => DefKind::Use,
            22 => DefKind::ForeignMod,
            23 => DefKind::AnonConst,
            24 => DefKind::InlineConst,
            25 => DefKind::OpaqueTy,
            26 => DefKind::Field,
            27 => DefKind::LifetimeParam,
            28 => DefKind::GlobalAsm,
            29 => DefKind::Impl,
            30 => DefKind::Closure,
            31 => DefKind::Local,
            _ => DefKind::Other,
        }
    }
}

/// A definition record in the index.
pub const FLAG_MACRO_EXPANSION: u8 = 1 << 0;

/// A definition record in the index.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DefEntry {
    /// Crate-unique identifier for this definition
    pub def_id: u32,
    /// Kind of definition (DefKind)
    pub kind: u8,
    /// Visibility (0 = Public, 1 = Crate, 2 = Restricted, 3 = Private)
    pub visibility: u8,
    /// Flags (bit 0: is_macro_expansion)
    pub flags: u8,
    pub _pad: u8,
    /// Short identifier name offset in string pool (e.g. "enter")
    pub name_offset: u32,
    /// Canonical full path offset in string pool (e.g. "stdx::panic_context::enter")
    pub path_offset: u32,
    /// Signature or type string offset in string pool (e.g. "fn(String) -> PanicContext"), or 0
    pub sig_offset: u32,
    /// Source span of the definition
    pub span: Span,
}

impl DefEntry {
    #[inline]
    pub fn to_le(self) -> Self {
        Self {
            def_id: self.def_id.to_le(),
            kind: self.kind,
            visibility: self.visibility,
            flags: self.flags,
            _pad: self._pad,
            name_offset: self.name_offset.to_le(),
            path_offset: self.path_offset.to_le(),
            sig_offset: self.sig_offset.to_le(),
            span: self.span.to_le(),
        }
    }

    #[inline]
    pub fn from_le(self) -> Self {
        Self {
            def_id: u32::from_le(self.def_id),
            kind: self.kind,
            visibility: self.visibility,
            flags: self.flags,
            _pad: self._pad,
            name_offset: u32::from_le(self.name_offset),
            path_offset: u32::from_le(self.path_offset),
            sig_offset: u32::from_le(self.sig_offset),
            span: self.span.from_le(),
        }
    }

    #[inline]
    pub fn def_id(&self) -> u32 {
        u32::from_le(self.def_id)
    }
    #[inline]
    pub fn name_offset(&self) -> u32 {
        u32::from_le(self.name_offset)
    }
    #[inline]
    pub fn path_offset(&self) -> u32 {
        u32::from_le(self.path_offset)
    }
    #[inline]
    pub fn sig_offset(&self) -> u32 {
        u32::from_le(self.sig_offset)
    }
    #[inline]
    pub fn span(&self) -> Span {
        self.span.from_le()
    }
    #[inline]
    pub fn is_macro_expansion(&self) -> bool {
        (self.flags & FLAG_MACRO_EXPANSION) != 0
    }
}

/// A reference/usage record pointing to a target definition or symbol.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RefEntry {
    /// Source span of this reference usage
    pub span: Span,
    /// Target definition ID if resolved within the same crate, or u32::MAX for external
    pub target_def_id: u32,
    /// Target canonical path offset in string pool (e.g. "std::convert::Into")
    pub target_path_offset: u32,
    /// Target source file path offset in string pool, or 0 if external/unknown
    pub target_file_offset: u32,
    /// Target compilation unit key offset in string pool, or 0 if unknown
    pub target_unit_offset: u32,
    /// Target definition line (1-based), or 0
    pub target_line: u32,
    /// Target definition col (1-based), or 0
    pub target_col: u32,
    /// Kind of symbol being referenced
    pub kind: u8,
    /// Usage flags: 1 = write/mutable, 0 = read/call
    pub is_write: u8,
    /// Flags (bit 0: is_macro_expansion)
    pub flags: u8,
    pub _pad: u8,
}

impl RefEntry {
    #[inline]
    pub fn to_le(self) -> Self {
        Self {
            span: self.span.to_le(),
            target_def_id: self.target_def_id.to_le(),
            target_path_offset: self.target_path_offset.to_le(),
            target_file_offset: self.target_file_offset.to_le(),
            target_unit_offset: self.target_unit_offset.to_le(),
            target_line: self.target_line.to_le(),
            target_col: self.target_col.to_le(),
            kind: self.kind,
            is_write: self.is_write,
            flags: self.flags,
            _pad: self._pad,
        }
    }

    #[inline]
    pub fn from_le(self) -> Self {
        Self {
            span: self.span.from_le(),
            target_def_id: u32::from_le(self.target_def_id),
            target_path_offset: u32::from_le(self.target_path_offset),
            target_file_offset: u32::from_le(self.target_file_offset),
            target_unit_offset: u32::from_le(self.target_unit_offset),
            target_line: u32::from_le(self.target_line),
            target_col: u32::from_le(self.target_col),
            kind: self.kind,
            is_write: self.is_write,
            flags: self.flags,
            _pad: self._pad,
        }
    }

    #[inline]
    pub fn span(&self) -> Span {
        self.span.from_le()
    }
    #[inline]
    pub fn target_def_id(&self) -> u32 {
        u32::from_le(self.target_def_id)
    }
    #[inline]
    pub fn target_path_offset(&self) -> u32 {
        u32::from_le(self.target_path_offset)
    }
    #[inline]
    pub fn target_file_offset(&self) -> u32 {
        u32::from_le(self.target_file_offset)
    }
    #[inline]
    pub fn target_unit_offset(&self) -> u32 {
        u32::from_le(self.target_unit_offset)
    }
    #[inline]
    pub fn target_line(&self) -> u32 {
        u32::from_le(self.target_line)
    }
    #[inline]
    pub fn target_col(&self) -> u32 {
        u32::from_le(self.target_col)
    }
    #[inline]
    pub fn is_macro_expansion(&self) -> bool {
        (self.flags & FLAG_MACRO_EXPANSION) != 0
    }
}

/// Inferred expression type record.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TypeEntry {
    /// Source span of the expression
    pub span: Span,
    /// Offset in string pool of the inferred type name (e.g. "Option<JoinHandle<T>>")
    pub type_offset: u32,
    pub _pad: [u8; 4],
}

impl TypeEntry {
    #[inline]
    pub fn to_le(self) -> Self {
        Self {
            span: self.span.to_le(),
            type_offset: self.type_offset.to_le(),
            _pad: self._pad,
        }
    }

    #[inline]
    pub fn from_le(self) -> Self {
        Self {
            span: self.span.from_le(),
            type_offset: u32::from_le(self.type_offset),
            _pad: self._pad,
        }
    }

    #[inline]
    pub fn span(&self) -> Span {
        self.span.from_le()
    }
    #[inline]
    pub fn type_offset(&self) -> u32 {
        u32::from_le(self.type_offset)
    }
}

/// Hash table entry for $O(\log N)$ or binary search lookup by string hash.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HashIndexEntry {
    /// 64-bit FNV-1a hash of the string (path or symbol)
    pub hash: u64,
    /// Index into the corresponding records table (defs or refs)
    pub index: u32,
    pub _pad: [u8; 4],
}

impl HashIndexEntry {
    #[inline]
    pub fn to_le(self) -> Self {
        Self {
            hash: self.hash.to_le(),
            index: self.index.to_le(),
            _pad: self._pad,
        }
    }

    #[inline]
    pub fn from_le(self) -> Self {
        Self {
            hash: u64::from_le(self.hash),
            index: u32::from_le(self.index),
            _pad: self._pad,
        }
    }

    #[inline]
    pub fn hash(&self) -> u64 {
        u64::from_le(self.hash)
    }
    #[inline]
    pub fn index(&self) -> u32 {
        u32::from_le(self.index)
    }
}

/// FNV-1a 64-bit string hash used across the RIDX format.
#[inline]
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    #[test]
    fn test_cache_helpers() {
        std::env::set_var("RIDX_CACHE_DIR", "/custom/cache");
        let dir = resolve_cache_dir();
        assert_eq!(dir, PathBuf::from("/custom/cache"));
        std::env::remove_var("RIDX_CACHE_DIR");

        let cache_base = Path::new("/var/cache/ridx");
        let p1 = cache_object_path(cache_base, "abcdef");
        assert_eq!(p1, cache_base.join("objects/ab/abcdef.ridx"));

        let p2 = cache_object_path(cache_base, "x");
        assert_eq!(p2, cache_base.join("objects/00/x.ridx"));
    }

    #[test]
    fn test_format_struct_accessors() {
        let fe = FileEntry {
            path_offset: 12,
            _pad: 0,
            source_hash: 12345,
        };
        assert_eq!(fe.path_offset(), 12);
        assert_eq!(fe.source_hash(), 12345);

        let hdr = Header {
            magic: MAGIC,
            version: CURRENT_VERSION,
            crate_name_offset: 0,
            crate_version_offset: 0,
            string_pool_offset: 0,
            string_pool_len: 0,
            files_offset: 0,
            files_count: 0,
            unit_key_offset: 42,
            defs_offset: 0,
            defs_count: 0,
            generation_offset: 84,
            defs_by_hash_offset: 0,
            defs_by_hash_count: 0,
            _pad2: 0,
            refs_offset: 0,
            refs_count: 0,
            _pad3: 0,
            refs_by_target_offset: 0,
            refs_by_target_count: 0,
            _pad4: 0,
            types_offset: 0,
            types_count: 0,
            _pad5: 0,
        };
        let hdr_le = hdr.to_le();
        let hdr_from = hdr_le.from_le();
        assert_eq!(hdr_from.unit_key_offset(), 42);
        assert_eq!(hdr_from.generation_offset(), 84);

        let def = DefEntry {
            def_id: 1,
            name_offset: 10,
            path_offset: 20,
            sig_offset: 30,
            span: Span::new(0, 1, 1, 2, 2),
            kind: 1,
            visibility: 0,
            flags: 1,
            _pad: 0,
        };
        let def_le = def.to_le();
        let def_from = def_le.from_le();
        assert_eq!(def.def_id, def_from.def_id);

        let r = RefEntry {
            span: Span::new(0, 1, 1, 2, 2),
            target_def_id: 1,
            target_path_offset: 10,
            target_file_offset: 100,
            target_unit_offset: 200,
            target_line: 15,
            target_col: 4,
            kind: 1,
            is_write: 1,
            flags: FLAG_MACRO_EXPANSION,
            _pad: 0,
        };
        let r_le = r.to_le();
        let r_from = r_le.from_le();
        assert_eq!(r_from.target_file_offset(), 100);
        assert_eq!(r_from.target_unit_offset(), 200);
        assert_eq!(r_from.target_line(), 15);
        assert_eq!(r_from.target_col(), 4);
        assert!(r_from.is_macro_expansion());

        let ty = TypeEntry {
            span: Span::new(0, 1, 1, 2, 2),
            type_offset: 50,
            _pad: [0; 4],
        };
        let ty_le = ty.to_le();
        let ty_from = ty_le.from_le();
        assert_eq!(ty.type_offset, ty_from.type_offset);

        let h_entry = HashIndexEntry {
            hash: 9999,
            index: 7,
            _pad: [0; 4],
        };
        let h_le = h_entry.to_le();
        let h_from = h_le.from_le();
        assert_eq!(h_from.hash(), 9999);
        assert_eq!(h_from.index(), 7);
    }

    #[test]
    fn test_merge_embedded_hashes_and_digest() {
        let mut map = HashMap::new();
        map.insert("src/lib.rs".to_string(), 12345u64);
        let digest = compute_source_snapshot_digest(&map);
        assert_ne!(digest, 0);

        let mut existing = HashMap::new();
        existing.insert("src/lib.rs".to_string(), 99999u64); // conflict
        existing.insert("src/other.rs".to_string(), 88888u64);

        let mut embedded = HashMap::new();
        embedded.insert("src/lib.rs".to_string(), 12345u64);

        merge_embedded_hashes_authoritative(&mut existing, &embedded, None);
        assert_eq!(existing.get("src/lib.rs"), Some(&12345u64));
        assert_eq!(existing.get("src/other.rs"), Some(&88888u64));
    }

    #[test]
    fn test_def_kind_all_variants_and_as_str() {
        for val in 0..=33 {
            let kind = DefKind::from(val);
            let s = kind.as_str();
            assert!(!s.is_empty());
        }
    }

    #[test]
    fn test_resolve_cache_dir_and_object_path() {
        let _lock = crate::workspace::ENV_LOCK.lock().unwrap();
        std::env::set_var("RIDX_CACHE_DIR", "/custom/ridx_cache");

        assert_eq!(
            resolve_cache_dir(),
            std::path::PathBuf::from("/custom/ridx_cache")
        );
        std::env::remove_var("RIDX_CACHE_DIR");

        std::env::set_var("XDG_CACHE_HOME", "/custom/xdg");
        let xdg_res = resolve_cache_dir();
        assert!(xdg_res.starts_with("/custom/xdg/ridx"));
        std::env::remove_var("XDG_CACHE_HOME");

        // Empty string variants and fallback (lines 30, 36, 45-46, 54)
        std::env::set_var("RIDX_CACHE_DIR", "   ");
        std::env::set_var("XDG_CACHE_HOME", "   ");
        std::env::set_var("HOME", "/custom/home");
        let home_res = resolve_cache_dir();
        assert!(home_res.starts_with("/custom/home/.cache/ridx"));

        std::env::set_var("HOME", "   ");
        let fallback_res = resolve_cache_dir();
        assert!(fallback_res.starts_with(".ridx_cache"));
        std::env::remove_var("RIDX_CACHE_DIR");
        std::env::remove_var("XDG_CACHE_HOME");
        std::env::remove_var("HOME");

        let obj1 = cache_object_path(Path::new("/cache"), "a");
        assert_eq!(obj1, std::path::PathBuf::from("/cache/objects/00/a.ridx"));

        let obj2 = cache_object_path(Path::new("/cache"), "12345");
        assert_eq!(
            obj2,
            std::path::PathBuf::from("/cache/objects/12/12345.ridx")
        );
    }

    #[test]
    fn test_join_resolving_overlap_variants() {
        let abs_rel = Path::new("/var/foo/bar.rs");
        assert_eq!(join_resolving_overlap(Path::new("/root"), abs_rel), abs_rel);

        let root = Path::new("/workspace/crates/app");
        let rel = Path::new("crates/app/src/lib.rs");
        let joined = join_resolving_overlap(root, rel);
        assert_eq!(joined, Path::new("/workspace/crates/app/src/lib.rs"));

        let rel_non = Path::new("other/file.rs");
        let joined_non = join_resolving_overlap(root, rel_non);
        assert_eq!(joined_non, Path::new("/workspace/crates/app/other/file.rs"));

        // Direct exists branch (line 81)
        let temp = std::env::temp_dir().join(format!("test_overlap_{}", std::process::id()));
        let _ = std::fs::create_dir_all(temp.join("src"));
        let test_file = temp.join("src/main.rs");
        let _ = std::fs::write(&test_file, b"fn main() {}");
        assert_eq!(
            join_resolving_overlap(&temp, Path::new("src/main.rs")),
            test_file
        );
        let _ = std::fs::remove_dir_all(&temp);

        // Span contains with mismatched file_id (line 418)
        let span = Span::new(1, 10, 1, 10, 5);
        assert!(!span.contains(999, 10, 2));
    }
}
