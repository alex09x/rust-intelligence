/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Notes: short texts the compiler knows about a source span that hover shows as they are (format version 4).

use super::Span;

/// The generic arguments of a call, at the callee's name: `` `T` = `String`, `A` = `Global` ``.
pub const NOTE_CALL_ARGS: u32 = 1;
/// The memory layout of a type, field, variant or binding, at its name: `size = 8, align = 0x8, no Drop`.
pub const NOTE_LAYOUT: u32 = 2;

/// A note record: the text a span carries, by kind.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NoteEntry {
    /// Source span the note belongs to (a name).
    pub span: Span,
    /// Offset in the string pool of the note's text.
    pub text_offset: u32,
    /// `NOTE_CALL_ARGS` or `NOTE_LAYOUT`.
    pub kind: u32,
}

impl NoteEntry {
    #[inline]
    pub fn new(span: Span, text_offset: u32, kind: u32) -> Self {
        Self { span, text_offset, kind }
    }

    #[inline]
    pub fn to_le(self) -> Self {
        Self { span: self.span.to_le(), text_offset: self.text_offset.to_le(), kind: self.kind.to_le() }
    }

    #[inline]
    pub fn span(&self) -> Span {
        self.span.from_le()
    }

    #[inline]
    pub fn text_offset(&self) -> u32 {
        u32::from_le(self.text_offset)
    }

    #[inline]
    pub fn kind(&self) -> u32 {
        u32::from_le(self.kind)
    }
}
