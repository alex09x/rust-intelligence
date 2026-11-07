/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! On-disk `LangItemKind` catalog and `LangItemEntry` record for `.ridx` format v5 (1.2-G6).

use super::ty_prim::TyDefId;

/// Canonical compiler `#[lang = "..."]` item (and associated trait item) identifier.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LangItemKind {
    // Core markers & fundamental ADTs
    Sized = 0,
    Copy = 1,
    Clone = 2,
    Sync = 3,
    Freeze = 4,
    Unpin = 5,
    Drop = 6,
    Tuple = 7,
    FnPtrTrait = 8,
    Option = 9,
    OptionSome = 10,
    OptionNone = 11,
    Result = 12,
    ResultOk = 13,
    ResultErr = 14,
    ControlFlow = 15,
    ControlFlowContinue = 16,
    ControlFlowBreak = 17,
    Poll = 18,
    PollReady = 19,
    PollPending = 20,
    Pin = 21,
    OwnedBox = 22,
    String = 23,
    ManuallyDrop = 24,
    MaybeUninit = 25,
    UnsafeCell = 26,
    PhantomData = 27,

    // Range structs
    RangeFull = 28,
    Range = 29,
    RangeFrom = 30,
    RangeTo = 31,
    RangeInclusive = 32,
    RangeToInclusive = 33,

    // `?` operator (`Try` v2 — unstable in `core`, bypassed into index)
    Try = 34,
    TryOutput = 35,
    TryResidual = 36,
    TryBranch = 37,
    TryFromOutput = 38,
    FromResidual = 39,
    FromResidualFn = 40,
    Residual = 41,
    ResidualTryType = 42,

    // Sync & async callables (`Fn*`, `AsyncFn*` — `call*` unstable in `core`)
    Fn = 43,
    FnCall = 44,
    FnMut = 45,
    FnMutCallMut = 46,
    FnOnce = 47,
    FnOnceCallOnce = 48,
    FnOnceOutput = 49,
    AsyncFn = 50,
    AsyncFnCall = 51,
    AsyncFnMut = 52,
    AsyncFnMutCallMut = 53,
    AsyncFnOnce = 54,
    AsyncFnOnceCallOnce = 55,
    AsyncFnOnceOutput = 56,
    CallOnceFuture = 57,
    CallRefFuture = 58,

    // Autoderef, receiver & indexing
    Deref = 59,
    DerefTarget = 60,
    DerefFn = 61,
    DerefMut = 62,
    DerefMutFn = 63,
    Receiver = 64,
    ReceiverTarget = 65,
    Index = 66,
    IndexOutput = 67,
    IndexFn = 68,
    IndexMut = 69,
    IndexMutFn = 70,

    // Iteration & `async`/`.await`
    Iterator = 71,
    IteratorItem = 72,
    IteratorNext = 73,
    IntoIterator = 74,
    IntoIteratorItem = 75,
    IntoIteratorIntoIter = 76,
    IntoIteratorIntoIterFn = 77,
    Future = 78,
    FutureOutput = 79,
    FuturePoll = 80,
    IntoFuture = 81,
    IntoFutureOutput = 82,
    IntoFutureIntoFuture = 83,
    IntoFutureIntoFutureFn = 84,
    AsyncIterator = 85,
    AsyncIteratorPollNext = 86,

    // Operators & conversions
    Add = 87,
    Sub = 88,
    Mul = 89,
    Div = 90,
    Rem = 91,
    Neg = 92,
    Not = 93,
    BitXor = 94,
    BitAnd = 95,
    BitOr = 96,
    Shl = 97,
    Shr = 98,
    AddAssign = 99,
    SubAssign = 100,
    MulAssign = 101,
    DivAssign = 102,
    RemAssign = 103,
    BitXorAssign = 104,
    BitAndAssign = 105,
    BitOrAssign = 106,
    ShlAssign = 107,
    ShrAssign = 108,
    PartialEq = 109,
    PartialOrd = 110,
    From = 111,

    Other = u16::MAX,
}

impl LangItemKind {
    pub const KNOWN_MAX: u16 = 111;

    #[inline]
    pub const fn from_u16(v: u16) -> Self {
        if v <= Self::KNOWN_MAX {
            // SAFETY: `LangItemKind` is `#[repr(u16)]` with contiguous variants `0..=111`.
            unsafe { std::mem::transmute::<u16, Self>(v) }
        } else {
            Self::Other
        }
    }
}

/// Fixed-size 12-byte lang-item mapping row in `lang_items`, sorted by `kind`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LangItemEntry {
    pub kind: u16,
    pub _pad: u16,
    pub ty_def: u32,
    pub def_index: u32,
}

impl LangItemEntry {
    #[inline]
    pub const fn to_le(self) -> Self {
        Self { kind: self.kind.to_le(), _pad: 0, ty_def: self.ty_def.to_le(), def_index: self.def_index.to_le() }
    }

    #[inline]
    pub const fn from_le(self) -> Self {
        Self { kind: u16::from_le(self.kind), _pad: 0, ty_def: u32::from_le(self.ty_def), def_index: u32::from_le(self.def_index) }
    }

    #[inline]
    pub const fn kind_raw(&self) -> u16 {
        u16::from_le(self.kind)
    }

    #[inline]
    pub const fn lang_kind(&self) -> LangItemKind {
        LangItemKind::from_u16(self.kind_raw())
    }

    #[inline]
    pub const fn ty_def_id(&self) -> TyDefId {
        TyDefId(u32::from_le(self.ty_def))
    }

    #[inline]
    pub const fn def_index(&self) -> Option<u32> {
        let idx = u32::from_le(self.def_index);
        if idx == u32::MAX {
            None
        } else {
            Some(idx)
        }
    }
}
