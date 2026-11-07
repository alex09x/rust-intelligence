/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! On-disk `#[repr(C)]` structures and discriminant tags for `TyArena` (`.ridx` format version 5).

/// Discriminant tag for `TyEntry` (`Infer`-free by construction).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TyTag {
    #[default]
    Unit = 0,
    Bool = 1,
    Char = 2,
    Int = 3,
    Uint = 4,
    Float = 5,
    Str = 6,
    Never = 7,
    Adt = 8,
    Ref = 9,
    Ptr = 10,
    Slice = 11,
    Array = 12,
    Tuple = 13,
    FnPtr = 14,
    FnDef = 15,
    Param = 16,
    Assoc = 17,
    Dyn = 18,
    ImplTrait = 19,
    Unknown = 255,
}

impl TyTag {
    #[inline]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Unit,
            1 => Self::Bool,
            2 => Self::Char,
            3 => Self::Int,
            4 => Self::Uint,
            5 => Self::Float,
            6 => Self::Str,
            7 => Self::Never,
            8 => Self::Adt,
            9 => Self::Ref,
            10 => Self::Ptr,
            11 => Self::Slice,
            12 => Self::Array,
            13 => Self::Tuple,
            14 => Self::FnPtr,
            15 => Self::FnDef,
            16 => Self::Param,
            17 => Self::Assoc,
            18 => Self::Dyn,
            19 => Self::ImplTrait,
            _ => Self::Unknown,
        }
    }
}

/// Interned definition reference for ADTs, traits, and function items in `TyArena` (16 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TyDefEntry {
    pub path_hash: u64,
    pub path_offset: u32,
    /// Bit 31 == 0 => `local_def_idx`; Bit 31 == 1 => external `unit_key` offset in `string_pool` (masked by `0x7fff_ffff`).
    pub loc_or_unit: u32,
}

impl TyDefEntry {
    pub const EXTERN_BIT: u32 = 0x8000_0000;

    #[inline]
    pub const fn local(path_hash: u64, path_offset: u32, local_def_idx: u32) -> Self {
        Self { path_hash, path_offset, loc_or_unit: local_def_idx & !Self::EXTERN_BIT }
    }

    #[inline]
    pub const fn external(path_hash: u64, path_offset: u32, unit_offset: u32) -> Self {
        Self { path_hash, path_offset, loc_or_unit: Self::EXTERN_BIT | (unit_offset & !Self::EXTERN_BIT) }
    }

    #[inline]
    pub const fn to_le(self) -> Self {
        Self { path_hash: self.path_hash.to_le(), path_offset: self.path_offset.to_le(), loc_or_unit: self.loc_or_unit.to_le() }
    }

    #[inline]
    pub const fn from_le(self) -> Self {
        Self { path_hash: u64::from_le(self.path_hash), path_offset: u32::from_le(self.path_offset), loc_or_unit: u32::from_le(self.loc_or_unit) }
    }

    #[inline]
    pub const fn path_hash(&self) -> u64 {
        u64::from_le(self.path_hash)
    }

    #[inline]
    pub const fn path_offset(&self) -> u32 {
        u32::from_le(self.path_offset)
    }

    #[inline]
    pub const fn local_def_idx(&self) -> Option<u32> {
        let v = u32::from_le(self.loc_or_unit);
        if (v & Self::EXTERN_BIT) == 0 {
            Some(v)
        } else {
            None
        }
    }

    #[inline]
    pub const fn extern_unit_offset(&self) -> Option<u32> {
        let v = u32::from_le(self.loc_or_unit);
        if (v & Self::EXTERN_BIT) != 0 {
            Some(v & !Self::EXTERN_BIT)
        } else {
            None
        }
    }
}

/// Trait bound entry used by `Dyn`, `ImplTrait`, and generic predicates (16 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TyBoundEntry {
    pub trait_def: u32,
    pub args_start: u32,
    pub args_len: u16,
    pub assoc_len: u16,
    pub flags: u32,
}

impl TyBoundEntry {
    #[inline]
    pub const fn to_le(self) -> Self {
        Self {
            trait_def: self.trait_def.to_le(),
            args_start: self.args_start.to_le(),
            args_len: self.args_len.to_le(),
            assoc_len: self.assoc_len.to_le(),
            flags: self.flags.to_le(),
        }
    }

    #[inline]
    pub const fn from_le(self) -> Self {
        Self {
            trait_def: u32::from_le(self.trait_def),
            args_start: u32::from_le(self.args_start),
            args_len: u16::from_le(self.args_len),
            assoc_len: u16::from_le(self.assoc_len),
            flags: u32::from_le(self.flags),
        }
    }
}

/// Fixed-size 16-byte interned type node in `ty_entries`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TyEntry {
    pub tag: u8,
    pub flags: u8,
    pub extra: u16,
    pub a: u32,
    pub b: u32,
    pub c: u32,
}

impl TyEntry {
    #[inline]
    pub const fn to_le(self) -> Self {
        Self { tag: self.tag, flags: self.flags, extra: self.extra.to_le(), a: self.a.to_le(), b: self.b.to_le(), c: self.c.to_le() }
    }

    #[inline]
    pub const fn from_le(self) -> Self {
        Self { tag: self.tag, flags: self.flags, extra: u16::from_le(self.extra), a: u32::from_le(self.a), b: u32::from_le(self.b), c: u32::from_le(self.c) }
    }
}

/// Structured function or method signature (`16` bytes, align `4`), sorted ascending by `def_index`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FnSigEntry {
    /// 0-based index into the `defs` section (`reader.definitions()[def_index as usize]`).
    pub def_index: u32,
    /// Start index of input parameter `TyId` words in `ty_args`.
    pub inputs_start: u32,
    /// Number of input parameters (including receiver when `FN_FLAG_HAS_SELF` is set).
    pub inputs_len: u16,
    /// Bitmask of `FN_FLAG_UNSAFE | FN_FLAG_VARIADIC | FN_FLAG_ASYNC | FN_FLAG_CONST | FN_FLAG_HAS_SELF`.
    pub flags: u8,
    /// ABI category (`FnAbiKind as u8`).
    pub abi: u8,
    /// Return type `TyId` in `ty_entries`.
    pub output: u32,
}

impl FnSigEntry {
    #[inline]
    pub const fn to_le(self) -> Self {
        Self {
            def_index: self.def_index.to_le(),
            inputs_start: self.inputs_start.to_le(),
            inputs_len: self.inputs_len.to_le(),
            flags: self.flags,
            abi: self.abi,
            output: self.output.to_le(),
        }
    }

    #[inline]
    pub const fn from_le(self) -> Self {
        Self {
            def_index: u32::from_le(self.def_index),
            inputs_start: u32::from_le(self.inputs_start),
            inputs_len: u16::from_le(self.inputs_len),
            flags: self.flags,
            abi: self.abi,
            output: u32::from_le(self.output),
        }
    }

    #[inline]
    pub const fn def_index(&self) -> u32 {
        u32::from_le(self.def_index)
    }

    #[inline]
    pub const fn inputs_start(&self) -> u32 {
        u32::from_le(self.inputs_start)
    }

    #[inline]
    pub const fn inputs_len(&self) -> u16 {
        u16::from_le(self.inputs_len)
    }

    #[inline]
    pub const fn output_ty(&self) -> super::ty_prim::TyId {
        super::ty_prim::TyId(u32::from_le(self.output))
    }

    #[inline]
    pub const fn abi_kind(&self) -> super::ty_prim::FnAbiKind {
        super::ty_prim::FnAbiKind::from_u8(self.abi)
    }
}
