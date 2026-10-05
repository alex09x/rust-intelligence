//! Binary format specifications for the RIDX (.ridx) index file.
//!
//! All integer values are stored in little-endian format.
//! Structures are designed for zero-copy memory-mapped reading with natural alignment.

pub const MAGIC: [u8; 4] = *b"RIDX";
pub const CURRENT_VERSION: u32 = 1;

/// Fixed-size header at offset 0 of any .ridx file.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Magic signature ("RIDX")
    pub magic: [u8; 4],
    /// Format version (currently 1)
    pub version: u32,
    /// Offset in string pool for crate name
    pub crate_name_offset: u32,
    /// Offset in string pool for crate version
    pub crate_version_offset: u32,

    /// Offset of the string intern pool
    pub string_pool_offset: u64,
    /// Length in bytes of the string intern pool
    pub string_pool_len: u64,

    /// Offset of the file path table (array of u32 string pool offsets)
    pub files_offset: u64,
    pub files_count: u32,
    pub _pad0: u32,

    /// Offset of the definitions table (array of DefEntry)
    pub defs_offset: u64,
    pub defs_count: u32,
    pub _pad1: u32,

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

    /// Returns true if this span contains the given (line, col) position in the specified file.
    #[inline]
    pub fn contains(&self, file_id: u32, line: u32, col: u32) -> bool {
        if self.file_id != file_id {
            return false;
        }
        if line < self.start_line || line > self.end_line {
            return false;
        }
        if line == self.start_line && col < self.start_col {
            return false;
        }
        if line == self.end_line && col >= self.end_col {
            return false;
        }
        true
    }

    /// Approximate character length metric used for picking the innermost span when nested.
    #[inline]
    pub fn approximate_size(&self) -> u64 {
        let lines = (self.end_line.saturating_sub(self.start_line)) as u64;
        if lines == 0 {
            (self.end_col.saturating_sub(self.start_col)) as u64
        } else {
            lines * 80 + (self.end_col as u64)
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
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DefEntry {
    /// Crate-unique identifier for this definition
    pub def_id: u32,
    /// Kind of definition (DefKind)
    pub kind: u8,
    /// Visibility (0 = Public, 1 = Crate, 2 = Restricted, 3 = Private)
    pub visibility: u8,
    pub _pad: [u8; 2],
    /// Short identifier name offset in string pool (e.g. "enter")
    pub name_offset: u32,
    /// Canonical full path offset in string pool (e.g. "stdx::panic_context::enter")
    pub path_offset: u32,
    /// Signature or type string offset in string pool (e.g. "fn(String) -> PanicContext"), or 0
    pub sig_offset: u32,
    /// Source span of the definition
    pub span: Span,
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
    /// Target definition line (1-based), or 0
    pub target_line: u32,
    /// Target definition col (1-based), or 0
    pub target_col: u32,
    /// Kind of symbol being referenced
    pub kind: u8,
    /// Usage flags: 1 = write/mutable, 0 = read/call
    pub is_write: u8,
    pub _pad: [u8; 2],
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
