/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

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
        Self { file_id, start_line, start_col, end_line, end_col }
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
            (lines * 200 + (s.end_col as u64)).saturating_sub((s.start_col as u64).min(199))
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
pub const FLAG_REEXPORT: u8 = 1 << 1;

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
    #[inline]
    pub fn is_reexport(&self) -> bool {
        (self.flags & FLAG_REEXPORT) != 0
    }
}
