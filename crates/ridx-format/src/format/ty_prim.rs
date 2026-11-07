/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Scalar identifiers, primitive widths, calling conventions, and generic argument words for `TyArena`.

/// 32-bit interned type identifier within a shard's `TyArena`.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TyId(pub u32);

impl TyId {
    pub const NONE: Self = Self(u32::MAX);

    #[inline]
    pub const fn is_none(self) -> bool {
        self.0 == u32::MAX
    }

    #[inline]
    pub const fn is_some(self) -> bool {
        self.0 != u32::MAX
    }

    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// 32-bit interned type-level definition reference within a shard's `ty_defs` table.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TyDefId(pub u32);

impl TyDefId {
    pub const NONE: Self = Self(u32::MAX);

    #[inline]
    pub const fn is_none(self) -> bool {
        self.0 == u32::MAX
    }

    #[inline]
    pub const fn is_some(self) -> bool {
        self.0 != u32::MAX
    }

    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Reference or raw-pointer mutability.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TyMutability {
    #[default]
    Shared = 0,
    Mut = 1,
}

impl TyMutability {
    #[inline]
    pub const fn from_u8(v: u8) -> Self {
        if v == 1 {
            Self::Mut
        } else {
            Self::Shared
        }
    }
}

/// Signed integer width.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum IntTyWidth {
    #[default]
    Isize = 0,
    I8 = 1,
    I16 = 2,
    I32 = 3,
    I64 = 4,
    I128 = 5,
}

impl IntTyWidth {
    #[inline]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::I8,
            2 => Self::I16,
            3 => Self::I32,
            4 => Self::I64,
            5 => Self::I128,
            _ => Self::Isize,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Isize => "isize",
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::I128 => "i128",
        }
    }
}

/// Unsigned integer width.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum UintTyWidth {
    #[default]
    Usize = 0,
    U8 = 1,
    U16 = 2,
    U32 = 3,
    U64 = 4,
    U128 = 5,
}

impl UintTyWidth {
    #[inline]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::U8,
            2 => Self::U16,
            3 => Self::U32,
            4 => Self::U64,
            5 => Self::U128,
            _ => Self::Usize,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Usize => "usize",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::U128 => "u128",
        }
    }
}

/// Floating-point width.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FloatTyWidth {
    F16 = 0,
    #[default]
    F32 = 1,
    F64 = 2,
    F128 = 3,
}

impl FloatTyWidth {
    #[inline]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::F16,
            1 => Self::F32,
            3 => Self::F128,
            _ => Self::F64,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::F16 => "f16",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::F128 => "f128",
        }
    }
}

/// Function pointer ABI category.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FnAbiKind {
    #[default]
    Rust = 0,
    C = 1,
    System = 2,
    RustCall = 3,
    Other = 4,
}

impl FnAbiKind {
    #[inline]
    pub const fn from_u16(v: u16) -> Self {
        match v {
            0 => Self::Rust,
            1 => Self::C,
            2 => Self::System,
            3 => Self::RustCall,
            _ => Self::Other,
        }
    }
}

pub const FN_FLAG_UNSAFE: u8 = 1 << 0;
pub const FN_FLAG_VARIADIC: u8 = 1 << 1;
pub const BOUND_FLAG_MAYBE: u32 = 1 << 0;

/// Sentinel lifetime values encoded in `Ref` / `Dyn` / `GenericArgWord::from_lifetime`.
pub const LIFETIME_ELIDED: u32 = 0;
pub const LIFETIME_STATIC: u32 = 0x3fff_ffff;

/// Const generic / array length representation encoded inside a single `u32` word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConstArg {
    /// Inline non-negative integer constant (`0..=0x1fff_ffff`), e.g. `[u8; 32]`.
    Value(u32),
    /// Generic const parameter index (`const N: usize` -> `Param(idx)`).
    Param(u32),
    /// Interned `string_pool` offset for general constant expressions/literals.
    ExprOffset(u32),
}

/// Packed 32-bit generic argument word stored in `ty_args`.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct GenericArgWord(pub u32);

impl GenericArgWord {
    pub const TAG_MASK: u32 = 0xc000_0000;
    pub const PAYLOAD_MASK: u32 = 0x3fff_ffff;
    pub const TAG_TYPE: u32 = 0x0000_0000;
    pub const TAG_CONST_PARAM: u32 = 0x4000_0000;
    pub const TAG_CONST_VAL: u32 = 0x8000_0000;
    pub const TAG_LIFETIME: u32 = 0xc000_0000;
    const CONST_EXPR_BIT: u32 = 0x2000_0000;
    const CONST_SUB_MASK: u32 = 0x1fff_ffff;

    #[inline]
    pub const fn from_type(ty: TyId) -> Self {
        debug_assert!(ty.0 <= Self::PAYLOAD_MASK);
        Self(Self::TAG_TYPE | (ty.0 & Self::PAYLOAD_MASK))
    }

    #[inline]
    pub const fn from_const(c: ConstArg) -> Self {
        match c {
            ConstArg::Param(idx) => Self(Self::TAG_CONST_PARAM | (idx & Self::PAYLOAD_MASK)),
            ConstArg::Value(v) => Self(Self::TAG_CONST_VAL | (v & Self::CONST_SUB_MASK)),
            ConstArg::ExprOffset(off) => Self(Self::TAG_CONST_VAL | Self::CONST_EXPR_BIT | (off & Self::CONST_SUB_MASK)),
        }
    }

    #[inline]
    pub const fn from_lifetime(lifetime_code: u32) -> Self {
        Self(Self::TAG_LIFETIME | (lifetime_code & Self::PAYLOAD_MASK))
    }

    #[inline]
    pub const fn decode_const(raw: u32) -> ConstArg {
        let w = Self(raw);
        match w.0 & Self::TAG_MASK {
            Self::TAG_CONST_PARAM => ConstArg::Param(w.0 & Self::PAYLOAD_MASK),
            _ => {
                if (w.0 & Self::CONST_EXPR_BIT) != 0 {
                    ConstArg::ExprOffset(w.0 & Self::CONST_SUB_MASK)
                } else {
                    ConstArg::Value(w.0 & Self::CONST_SUB_MASK)
                }
            }
        }
    }

    #[inline]
    pub const fn tag_bits(self) -> u32 {
        self.0 & Self::TAG_MASK
    }

    #[inline]
    pub const fn payload(self) -> u32 {
        self.0 & Self::PAYLOAD_MASK
    }
}
